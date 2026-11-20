//! Two-pass assembly of a symbolic script listing.
use cmvs_core::{
    MAX_PAYLOAD,
    format::ps3::{Container, Script, ScriptFormat},
    profile::{EngineProfile, Registry},
    vm::bytecode::{self, Instruction, InstructionKind},
};
use cmvs_tooling::{
    Result,
    project::{self, CallbackTarget, Field},
    table::{Encoding, unescape},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::BufRead,
};

struct Header {
    format: ScriptFormat,
    compressed: bool,
    encoding: Encoding,
    original_code: usize,
    original_data: usize,
}

fn header<'a>(
    line: &str,
    selected: Option<&'a EngineProfile>,
) -> Result<(Header, Option<&'a EngineProfile>)> {
    let fields: Vec<_> = line.split('\t').collect();
    let [
        project::FORMAT,
        magic,
        version,
        key,
        reserved,
        storage,
        id,
        encoding,
        code,
        data,
    ] = fields.as_slice()
    else {
        return Err("expected cmvs-script-v1 project header".into());
    };
    let container = match *magic {
        "PS2A" => Container::Ps2a,
        "PSS0" => Container::Pss0,
        _ => return Err("unknown script container".into()),
    };
    let profile = if *id == "-" {
        selected
    } else {
        let recorded = Registry::builtin()
            .profile(id)
            .ok_or_else(|| format!("unknown project profile {id}"))?;
        if selected.is_some_and(|p| p.id != recorded.id) {
            return Err("selected profile differs from project profile".into());
        }
        Some(recorded)
    };
    if profile.is_some_and(|p| p.is_cvns() != (container == Container::Pss0)) {
        return Err("profile family differs from project container".into());
    }
    let header = Header {
        format: ScriptFormat {
            container,
            version_raw: u32::from_str_radix(version, 16)?,
            key: u32::from_str_radix(key, 16)?,
            reserved: u32::from_str_radix(reserved, 16)?,
        },
        compressed: match *storage {
            "lzss" => true,
            "plain" => false,
            _ => return Err("expected lzss or plain storage".into()),
        },
        encoding: Encoding::parse(encoding)?,
        original_code: usize::from_str_radix(code, 16)?,
        original_data: usize::from_str_radix(data, 16)?,
    };
    if header.original_code > MAX_PAYLOAD || header.original_data > MAX_PAYLOAD {
        return Err("project segment exceeds payload limit".into());
    }
    Ok((header, profile))
}

type Symbols = BTreeMap<String, u32>;

fn define(symbols: &mut Symbols, name: &str, value: usize) -> Result<()> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(format!("invalid symbol {name:?}").into());
    }
    if symbols
        .insert(name.to_owned(), u32::try_from(value)?)
        .is_some()
    {
        return Err(format!("duplicate symbol {name}").into());
    }
    Ok(())
}

fn lookup(symbols: &Symbols, name: &str) -> Result<u32> {
    symbols
        .get(name)
        .copied()
        .ok_or_else(|| format!("undefined symbol {name}").into())
}

#[derive(Default)]
struct Project {
    entry: Option<String>,
    labels: Symbols,
    targets: Vec<String>,
    pcs: Symbols,
    starts: Vec<usize>,
    code: Vec<u8>,
    data: Vec<u8>,
    strings: Symbols,
    text: Vec<u8>,
    fixups: BTreeMap<usize, (Field, String)>,
    runtime_pcs: BTreeSet<usize>,
    moved: bool,
}

impl Project {
    fn instruction(&mut self, name: &str, words: &str) -> Result<()> {
        let pc = self.code.len();
        define(&mut self.pcs, name, pc)?;
        self.starts.push(pc);
        self.moved |= name
            .strip_prefix('p')
            .and_then(|s| usize::from_str_radix(s, 16).ok())
            != Some(pc);
        for token in words.split_ascii_whitespace() {
            let field = match token.as_bytes()[0] {
                b'@' => Some(Field::Pc),
                b'%' => Some(Field::Label32),
                b'&' => Some(Field::Label16),
                b'$' => Some(Field::Text),
                _ => None,
            };
            if let Some(field) = field {
                self.fixups
                    .insert(self.code.len(), (field, token[1..].to_owned()));
                let width = if field == Field::Label16 { 2 } else { 4 };
                self.code.resize(self.code.len() + width, 0);
            } else {
                if !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(format!("invalid hex operand {token}").into());
                }
                match token.len() {
                    4 => self
                        .code
                        .extend(u16::from_str_radix(token, 16)?.to_le_bytes()),
                    8 => self
                        .code
                        .extend(u32::from_str_radix(token, 16)?.to_le_bytes()),
                    _ => {
                        return Err(
                            format!("expected 4 or 8 hex digits or a symbol, got {token}").into(),
                        );
                    }
                }
            }
        }
        if self.code.len() == pc {
            return Err("empty instruction".into());
        }
        Ok(())
    }

    fn row(&mut self, line: &str, header: &Header) -> Result<()> {
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["entry", target] if self.entry.is_none() => self.entry = Some((*target).into()),
            ["label", name, target] => {
                define(&mut self.labels, name, self.targets.len())?;
                self.targets.push((*target).into());
            }
            ["data", _, hex] | ["append", hex] => {
                let bytes = hex.as_bytes().as_chunks::<2>();
                if !hex.bytes().all(|b| b.is_ascii_hexdigit()) || !bytes.1.is_empty() {
                    return Err("expected pairs of hex digits in data".into());
                }
                if fields[0] == "data" {
                    let pos = usize::from_str_radix(fields[1], 16)?;
                    if pos != self.data.len()
                        || pos >= header.original_data
                        || bytes.0.len() != (header.original_data - pos).min(project::DATA_ROW)
                    {
                        return Err("original data row must keep its offset and byte length".into());
                    }
                } else if self.data.len() < header.original_data {
                    return Err("append follows all original data rows".into());
                }
                for pair in bytes.0 {
                    self.data
                        .push(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?);
                }
            }
            ["text", name, escaped] => {
                define(&mut self.strings, name, self.text.len())?;
                self.text.extend(unescape(escaped, header.encoding)?);
            }
            ["code", name, words, rest @ ..] => {
                let pc = self.code.len();
                let mut rest = rest;
                if let Some(("runtime-pc", tail)) = rest.split_first().map(|(s, t)| (*s, t)) {
                    self.runtime_pcs.insert(pc);
                    rest = tail;
                }
                if !(rest.is_empty() || matches!(rest,[comment] if comment.starts_with('#'))) {
                    return Err("unknown instruction annotation".into());
                }
                self.instruction(name, words)?;
            }
            _ => return Err("invalid or repeated project directive".into()),
        }
        if self.code.len() + self.text.len() + self.data.len() + 4 * self.targets.len()
            > MAX_PAYLOAD
        {
            return Err("project exceeds script payload limit".into());
        }
        Ok(())
    }

    fn finish(mut self, h: &Header, profile: Option<&EngineProfile>) -> Result<(Script, bool)> {
        if self.data.len() < h.original_data {
            return Err("missing original data rows".into());
        }
        let entry = lookup(
            &self.pcs,
            self.entry.as_deref().ok_or("missing entry point")?,
        )?;
        let labels: Vec<_> = self
            .targets
            .iter()
            .map(|s| lookup(&self.pcs, s))
            .collect::<Result<_>>()?;
        for (&at, (kind, name)) in &self.fixups {
            let table = match kind {
                Field::Pc => &self.pcs,
                Field::Label16 | Field::Label32 => &self.labels,
                Field::Text => &self.strings,
                Field::Number | Field::Immediate => unreachable!(),
            };
            let value = lookup(table, name).map_err(|e| format!("code+{at:#x}: {e}"))?;
            if *kind == Field::Label16 {
                self.code[at..at + 2].copy_from_slice(&u16::try_from(value)?.to_le_bytes());
            } else {
                self.code[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        self.moved |= self.code.len() != h.original_code;
        let listing = bytecode::decode_layout(&self.code, entry, &labels)?;
        if !self
            .starts
            .iter()
            .copied()
            .eq(listing.iter().map(|ins| ins.pc))
        {
            return Err("each code row must contain exactly one complete instruction".into());
        }
        self.check_fields(&listing)?;
        self.relocate_callbacks(&listing, entry, &labels, profile, h.format.container)?;
        let script =
            Script::from_segments(h.format, entry, &labels, &self.code, &self.data, &self.text)?;
        let text = super::ScriptText::from_layout(&script, &listing)?;
        super::check_accumulators(&text, |key| text.entries()[key].len())?;
        Ok((script, self.moved))
    }

    fn check_fields(&self, listing: &[Instruction]) -> Result<()> {
        let mut recognized = 0;
        for ins in listing {
            let mut invalid = None;
            project::fields(ins, |at, field| {
                let supplied = self.fixups.get(&at).map(|(kind, _)| *kind);
                if supplied.is_some() {
                    recognized += 1;
                }
                let valid = match field {
                    Field::Number => supplied.is_none(),
                    Field::Immediate => matches!(supplied, None | Some(Field::Pc)),
                    _ => supplied == Some(field),
                };
                if !valid {
                    invalid = Some(at);
                }
            });
            if let Some(at) = invalid {
                return Err(format!("code+{at:#x}: wrong or missing symbolic operand").into());
            }
        }
        if recognized != self.fixups.len() {
            return Err("symbol used outside an instruction operand".into());
        }
        Ok(())
    }

    fn callback_pc(&self, field: usize, raw: u32) -> Option<u32> {
        if self.moved && !self.fixups.contains_key(&field) {
            self.pcs.get(&format!("p{raw:x}")).copied()
        } else {
            Some(raw)
        }
    }

    fn relocate_callbacks(
        &mut self,
        listing: &[Instruction],
        entry: u32,
        labels: &[u32],
        profile: Option<&EngineProfile>,
        container: Container,
    ) -> Result<()> {
        let entries = project::entry_points(listing, entry, labels, profile, |at, value| {
            self.callback_pc(at, value)
        });
        for (i, ins) in listing.iter().enumerate() {
            let InstructionKind::System { command } = ins.kind else {
                continue;
            };
            let Some(profile) = profile else {
                if self.moved && project::possible_callback(container, command) {
                    return Err(format!(
                        "code+{:#x}: select --profile to relocate callback arguments",
                        ins.pc
                    )
                    .into());
                }
                continue;
            };
            if project::runtime_command(profile, command) != Some(0x88) {
                continue;
            }
            let annotated = self.runtime_pcs.remove(&ins.pc);
            match project::callback_target(listing, i, &entries) {
                CallbackTarget::Literal { field, value } if value != u32::MAX => {
                    let target = self.callback_pc(field, value).ok_or_else(|| {
                        format!("code+{field:#x}: specify the callback target with @symbol")
                    })?;
                    if listing
                        .binary_search_by_key(&(target as usize), |ins| ins.pc)
                        .is_err()
                    {
                        return Err(format!(
                            "code+{field:#x}: invalid callback target {target:#x}"
                        )
                        .into());
                    }
                    self.code[field..field + 4].copy_from_slice(&target.to_le_bytes());
                }
                CallbackTarget::Dynamic if self.moved && !annotated => {
                    return Err(format!("code+{:#x}: dynamic callback target; relocate its producer and mark this command runtime-pc", ins.pc).into());
                }
                _ => {}
            }
        }
        if let Some(pc) = self.runtime_pcs.first() {
            return Err(
                format!("code+{pc:#x}: runtime-pc requires a profiled callback command").into(),
            );
        }
        Ok(())
    }
}

/// Assemble a complete project. The boolean reports code layout changes relevant to old saves.
pub fn assemble(
    input: impl BufRead,
    selected: Option<&EngineProfile>,
    compressed: Option<bool>,
) -> Result<(Vec<u8>, bool)> {
    let mut lines = input.lines();
    let (h, profile) = header(&lines.next().ok_or("empty project")??, selected)?;
    let mut project = Project::default();
    for (i, line) in lines.enumerate() {
        let line = line?;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        project
            .row(&line, &h)
            .map_err(|e| format!("project line {}: {e}", i + 2))?;
    }
    let (script, moved) = project.finish(&h, profile)?;
    let bytes = if compressed.unwrap_or(h.compressed) {
        script.encode_compressed()?
    } else {
        script.encode()?
    };
    Ok((bytes, moved))
}
