//! Export a standalone script project. Symbol names encode the original offsets.
use cmvs_core::{
    format::ps3::Script,
    profile::EngineProfile,
    vm::{
        bytecode::{self, InstructionKind},
        dispatch,
    },
};
use cmvs_tooling::{
    Result,
    project::{self, CallbackTarget, Field},
    table::{Encoding, escape},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

pub fn export_project(
    script: &Script,
    profile: Option<&EngineProfile>,
    encoding: Encoding,
    mut out: impl Write,
) -> Result<()> {
    let h = script.header();
    let labels = script.labels();
    let listing = bytecode::decode_layout(script.code(), h.entry_pc, &labels)?;
    let mut extra = BTreeMap::new();
    if let Some(profile) = profile {
        if profile.is_cvns() != (h.container == cmvs_core::format::ps3::Container::Pss0) {
            return Err("script container differs from project profile".into());
        }
        let entries =
            project::entry_points(&listing, h.entry_pc, &labels, Some(profile), |_, value| {
                Some(value)
            });
        for (i, ins) in listing.iter().enumerate() {
            if let InstructionKind::System { command } = ins.kind
                && project::runtime_command(profile, command) == Some(0x88)
                && let CallbackTarget::Literal { field, value } =
                    project::callback_target(&listing, i, &entries)
                && value != u32::MAX
            {
                if listing
                    .binary_search_by_key(&(value as usize), |ins| ins.pc)
                    .is_err()
                {
                    return Err(
                        format!("code+{:#x}: invalid callback PC {value:#x}", ins.pc).into(),
                    );
                }
                extra.insert(field, Field::Pc);
            }
        }
    }
    writeln!(
        out,
        "{}\t{}\t{:x}\t{:x}\t{:x}\t{}\t{}\t{}\t{:x}\t{:x}",
        project::FORMAT,
        std::str::from_utf8(h.container.magic())?,
        h.version_raw,
        h.key,
        h.reserved,
        if h.compressed_size == 0 {
            "plain"
        } else {
            "lzss"
        },
        profile.map_or("-", |p| &p.id),
        encoding.name(),
        h.code_size,
        h.unknown_size
    )?;
    writeln!(out, "entry\tp{:x}", h.entry_pc)?;
    for (i, target) in labels.iter().enumerate() {
        writeln!(out, "label\tl{i:x}\tp{target:x}")?;
    }
    for (i, chunk) in script.data().chunks(project::DATA_ROW).enumerate() {
        write!(out, "data\t{:x}\t", i * project::DATA_ROW)?;
        for b in chunk {
            write!(out, "{b:02x}")?;
        }
        writeln!(out)?;
    }
    let mut cuts = BTreeSet::from([0, script.text().len()]);
    for (i, byte) in script.text().iter().enumerate() {
        if *byte == 0 {
            cuts.insert(i + 1);
        }
    }
    let mut operands = Vec::new();
    for ins in &listing {
        operands.clear();
        project::fields(ins, |at, kind| operands.push((at, kind)));
        let mut fields = operands.iter().copied().peekable();
        let mut pc = ins.pc;
        write!(out, "code\tp{pc:x}\t")?;
        while pc < ins.end {
            if pc != ins.pc {
                write!(out, " ")?;
            }
            let bytes = &script.code()[pc..ins.end];
            if fields.peek().is_some_and(|&(at, _)| at == pc) {
                let (_, base) = fields.next().unwrap();
                let kind = *extra.get(&pc).unwrap_or(&base);
                if kind == Field::Label16 {
                    write!(out, "&l{:x}", u16::from_le_bytes(bytes[..2].try_into()?))?;
                    pc += 2;
                } else {
                    let value = u32::from_le_bytes(bytes[..4].try_into()?);
                    match kind {
                        Field::Pc => write!(out, "@p{value:x}")?,
                        Field::Label32 => {
                            if value as usize >= labels.len() {
                                return Err(
                                    format!("code+{pc:#x}: invalid label index {value}").into()
                                );
                            }
                            write!(out, "%l{value:x}")?;
                        }
                        Field::Text => {
                            if value as usize >= script.text().len() {
                                return Err(
                                    format!("code+{pc:#x}: invalid text offset {value}").into()
                                );
                            }
                            cuts.insert(value as usize);
                            write!(out, "$t{value:x}")?;
                        }
                        Field::Number | Field::Immediate => write!(out, "{value:08x}")?,
                        Field::Label16 => unreachable!(),
                    }
                    pc += 4;
                }
            } else {
                write!(out, "{:04x}", u16::from_le_bytes(bytes[..2].try_into()?))?;
                pc += 2;
            }
        }
        if let InstructionKind::System { command } = ins.kind
            && let Some(p) = profile
            && let Some(cmd) = project::runtime_command(p, command)
            && let Some(handler) = dispatch::entry(p.semantics(cmd), cmd)
        {
            write!(out, "\t# {}", handler.name)?;
        }
        writeln!(out)?;
    }
    let mut start = 0;
    for end in cuts.into_iter().skip(1) {
        let bytes = &script.text()[start..end];
        let (bytes, nul) = bytes
            .strip_suffix(&[0])
            .map_or((bytes, ""), |s| (s, "\\x00"));
        writeln!(out, "text\tt{start:x}\t{}{nul}", escape(bytes, encoding))?;
        start = end;
    }
    out.flush()?;
    Ok(())
}
