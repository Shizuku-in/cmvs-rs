//! UTF-8 conversion of the digest-pinned CK-GAL patch. GBK output bypasses this helper.
#[allow(dead_code)]
#[path = "../rewrite_scripts.rs"]
mod scripts;
use cmvs_core::{
    bytecode::{self, Expression, InstructionKind, Term},
    cpz::ArchiveIndex,
    ps3::Script,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn literal_positions(expr: &Expression, positions: &mut Vec<usize>) {
    for term in &expr.terms {
        match term {
            Term::Scalar { pc, tag: 0x120, .. } if expr.kind == 0x201 => positions.push(pc + 2),
            Term::Nested { expression, .. } => literal_positions(expression, positions),
            Term::Indexed { indices, .. } => {
                for expr in indices {
                    literal_positions(expr, positions);
                }
            }
            _ => {}
        }
    }
}

fn layout(code: &[u8], entry: u32, labels: &[u32]) -> Result<Vec<usize>> {
    let mut pc = 0;
    let mut starts = BTreeSet::new();
    let mut targets = vec![entry];
    targets.extend_from_slice(labels);
    let mut positions = Vec::new();
    while pc < code.len() {
        starts.insert(pc as u32);
        // The original snsy13 contains an empty integer expression. Preserve its
        // bytes; this scanner establishes boundaries, not evaluation semantics.
        if code.get(pc..pc + 4) == Some(&[0, 2, 15, 2]) {
            pc += 4;
            continue;
        }
        let insn = bytecode::decode_at(code, pc)?;
        match &insn.kind {
            InstructionKind::Expression(expr) => literal_positions(expr, &mut positions),
            InstructionKind::Jump { target }
            | InstructionKind::ConditionalJump { target }
            | InstructionKind::CompareJump { target, .. }
            | InstructionKind::LongJump { target, .. } => targets.push(*target),
            InstructionKind::Label { index } if *index as usize >= labels.len() => {
                return Err("label outside table".into());
            }
            _ => {}
        }
        pc = insn.end;
    }
    if targets.iter().any(|target| !starts.contains(target)) {
        return Err("branch or label is not an instruction boundary".into());
    }
    Ok(positions)
}

fn transcode(text: &[u8]) -> Result<(Vec<u8>, BTreeMap<u32, u32>)> {
    if text.last() != Some(&0) {
        return Err("unterminated script text".into());
    }
    let mut out = Vec::new();
    let mut offsets = BTreeMap::new();
    let mut at = 0;
    while at < text.len() {
        offsets.insert(at as u32, out.len() as u32);
        if text[at] == 0 {
            out.push(0);
            at += 1;
            continue;
        }
        // encoding_rs GBK includes the single-byte euro at 0x80.
        let len = if text[at] >= 0x81 { 2 } else { 1 };
        let bytes = text.get(at..at + len).ok_or("truncated GBK character")?;
        if bytes.contains(&0) {
            return Err("truncated GBK character before NUL".into());
        }
        let (unicode, _, bad) = encoding_rs::GBK.decode(bytes);
        if bad {
            return Err(format!("invalid GBK at text+0x{at:x}").into());
        }
        scripts::validate_utf8(unicode.as_bytes())?;
        out.extend_from_slice(unicode.as_bytes());
        at += len;
    }
    Ok((out, offsets))
}

fn recode(name: &[u8], original: &[u8], patched: &[u8]) -> Result<(Vec<u8>, usize)> {
    let original = Script::decode(original)?;
    let patch = Script::decode(patched)?;
    if original.code().len() != patch.code().len()
        || original.labels() != patch.labels()
        || original.header.entry_pc != patch.header.entry_pc
        || original.unknown() != patch.unknown()
    {
        return Err("patch changed original script structure".into());
    }
    let positions = layout(
        original.code(),
        original.header.entry_pc,
        &original.labels(),
    )?;
    let mut code = patch.code().to_vec();
    // CK-GAL's byte-pattern replacement also matched within scalar operands.
    // Restore only the two exact corruptions in the pinned patch, including the
    // high byte of the real literal in snas09; leave GBK output unchanged.
    let repair = match name {
        b"snas09.ps2" => Some((0x3a6c1, [0x02, 0xec, 0, 0], [0, 0x0f, 0x02, 0x30])),
        b"snma10.ps2" => Some((0x20e51, [0xd6, 0x2b, 0, 0], [0, 0x0f, 0x02, 0x30])),
        _ => None,
    };
    if let Some((at, before, after)) = repair {
        if code.get(at..at + 4) != Some(before.as_slice())
            || original.code().get(at..at + 4) != Some(after.as_slice())
        {
            return Err("unsupported patch code corruption".into());
        }
        code[at..at + 4].copy_from_slice(&after);
    }
    let mut structural = code.clone();
    for &at in &positions {
        structural[at..at + 4].copy_from_slice(&original.code()[at..at + 4]);
    }
    if structural != original.code() {
        return Err("unexpected patch change outside string literals".into());
    }
    let (text, offsets) = transcode(patch.text())?;
    for &at in &positions {
        let old = u32::from_le_bytes(code[at..at + 4].try_into()?);
        let target = offsets.get(&old).ok_or_else(|| {
            format!("literal at code+0x{at:x} references a non-character boundary: text+0x{old:x}")
        })?;
        code[at..at + 4].copy_from_slice(&target.to_le_bytes());
    }
    if layout(&code, patch.header.entry_pc, &patch.labels())? != positions {
        return Err("converted instruction layout changed".into());
    }
    Ok((
        scripts::rebuild_script(patched, &code, &text)?,
        positions.len(),
    ))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}
fn run(game: &Path, input: &Path, output: &Path) -> Result<()> {
    let mut patched_file = File::open(input.join("script.cpz"))?;
    let patched = ArchiveIndex::read(&mut patched_file)?;
    let mut original_file = File::open(game.join("data/pack/script.cpz"))?;
    let original = ArchiveIndex::read(&mut original_file)?;
    if patched.entries().len() != original.entries().len() {
        return Err("script entry count differs".into());
    }
    let mut entries = Vec::new();
    let mut report = String::from("name\tliteral_references\tbytes\n");
    for (i, entry) in patched.entries().iter().enumerate() {
        let raw = patched.read_entry_verified(&mut patched_file, i)?;
        let base = original.read_entry_verified(&mut original_file, original.find(&entry.name)?)?;
        let (converted, count) = recode(&entry.name, &base, &raw)
            .map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&entry.name)))?;
        report.push_str(&format!(
            "{}\t{count}\t{}\n",
            std::str::from_utf8(&entry.name)?,
            converted.len()
        ));
        entries.push((entry.name.clone(), converted));
    }
    let (startup, count) = recode(
        b"start.ps2",
        &fs::read(game.join("data/pack/start.ps2"))?,
        &fs::read(input.join("start.ps2"))?,
    )?;
    report.push_str(&format!("start.ps2\t{count}\t{}\n", startup.len()));
    write_new(&output.join("script.cpz"), &scripts::pack(&entries)?)?;
    write_new(&output.join("start.ps2"), &startup)?;
    write_new(&output.join("scripts.tsv"), report.as_bytes())?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "internal helper: <original-game> <normalized-gbk-patch> <empty-output>".into(),
        );
    }
    run(
        Path::new(&args[0]),
        Path::new(&args[1]),
        Path::new(&args[2]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(code: &[u8], text: &[u8]) -> Vec<u8> {
        let mut header = b"PS2A".to_vec();
        for value in [
            48,
            1,
            0,
            1,
            code.len() as u32,
            0,
            text.len() as u32,
            0,
            0,
            (4 + code.len() + text.len()) as u32,
            0,
        ] {
            header.extend(value.to_le_bytes());
        }
        let mut payload = vec![0; 4];
        payload.extend(code);
        payload.extend(text);
        header.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        header
    }
    fn literal(code: &mut Vec<u8>, offset: u32) {
        code.extend([1, 2, 0x20, 1]);
        code.extend(offset.to_le_bytes());
        code.extend([15, 2, 0x30, 4]);
    }
    #[test]
    fn utf8_relocates_suffixes_empty_strings_and_terminators() -> Result<()> {
        let text = b"\0\xd6\xd0\xce\xc4tail\0\0";
        let mut code = Vec::new();
        for at in [0, 1, 3, 5, 6, 9, 10] {
            literal(&mut code, at);
        }
        let raw = fixture(&code, text);
        let (out, count) = recode(b"test.ps2", &raw, &raw)?;
        let after = Script::decode(&out)?;
        assert_eq!(after.text(), "\0中文tail\0\0".as_bytes());
        assert_eq!(count, 7);
        for (i, expected) in [0, 1, 4, 7, 8, 11, 12].iter().enumerate() {
            assert_eq!(
                u32::from_le_bytes(after.code()[i * 12 + 4..i * 12 + 8].try_into()?),
                *expected
            );
        }
        assert_eq!(after.labels(), vec![0]);
        Ok(())
    }
    #[test]
    fn refuses_invalid_text_mid_character_references_and_unexpected_code_edits() {
        for text in [b"\xff\0".as_slice(), b"\x81\0", b"unterminated"] {
            assert!(transcode(text).is_err());
        }
        let mut code = Vec::new();
        literal(&mut code, 1);
        let raw = fixture(&code, b"\xd6\xd0\0");
        assert!(recode(b"test.ps2", &raw, &raw).is_err());
        code.clear();
        literal(&mut code, 0);
        let original = fixture(&code, b"text\0");
        code[10] = 0x16; // An unrelated opcode change is not a translation.
        let patch = fixture(&code, b"text\0");
        assert!(recode(b"test.ps2", &original, &patch).is_err());
    }
    #[test]
    fn preserves_original_empty_integer_expression() -> Result<()> {
        let raw = fixture(&[0, 2, 15, 2, 0x30, 4], b"\0");
        let (out, _) = recode(b"test.ps2", &raw, &raw)?;
        assert_eq!(Script::decode(&out)?.code(), Script::decode(&raw)?.code());
        Ok(())
    }
    #[test]
    fn known_repairs_require_exact_original_and_patch_bytes() -> Result<()> {
        for (name, at, bad) in [
            (b"snas09.ps2".as_slice(), 0x3a6c1, [2, 0xec, 0, 0]),
            (b"snma10.ps2", 0x20e51, [0xd6, 0x2b, 0, 0]),
        ] {
            // Valid two-byte instructions up to a scalar's final byte at `at`.
            let mut code = [0x30, 4].repeat((at - 7) / 2);
            code.extend([0, 2, 0, 1, 2, 0x20, 1, 0, 15, 2, 0x30, 4]);
            assert_eq!(code[at..at + 4], [0, 15, 2, 0x30]);
            let original = fixture(&code, b"\0");
            code[at..at + 4].copy_from_slice(&bad);
            let patched = fixture(&code, b"\0");
            let (out, _) = recode(name, &original, &patched)?;
            assert_eq!(
                Script::decode(&out)?.code(),
                Script::decode(&original)?.code()
            );
            code[at] ^= 1;
            assert!(recode(name, &original, &fixture(&code, b"\0")).is_err());
        }
        Ok(())
    }
}
