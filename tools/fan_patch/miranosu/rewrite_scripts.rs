//! Offline script rewriting helper; invoked by convert_miranosu_patch.py.
use cmvs_core::{
    bytecode::{self, Expression, InstructionKind, Term},
    cpz::{ArchiveIndex, cpz3_checksum},
    ps3::Script,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs::{self, File, OpenOptions},
    io::{Cursor, Write},
    path::Path,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
type Translations = BTreeMap<u32, BTreeMap<u32, Vec<u8>>>;

fn word(data: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        data.get(at..at + 4)
            .ok_or("truncated patch field")?
            .try_into()?,
    ))
}
fn put(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn push(data: &mut Vec<u8>, value: u32) {
    data.extend(value.to_le_bytes());
}
fn new_file(path: &Path, data: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(data)?;
    Ok(())
}
fn parse_patch(data: &[u8]) -> Result<Translations> {
    if data.len() > cmvs_core::MAX_PAYLOAD {
        return Err("patch exceeds limit".into());
    }
    let mut blocks = BTreeMap::new();
    let mut at = 0;
    while at < data.len() {
        let count = word(data, at)?;
        let key = word(data, at + 4)?;
        at += 8;
        if count == 0 || count as usize > (data.len() - at) / 8 {
            return Err("invalid patch count".into());
        }
        let mut rows = BTreeMap::new();
        for _ in 0..count {
            let flagged = word(data, at)?;
            let size = word(data, at + 4)? as usize;
            if flagged & 0x8000_0000 == 0 {
                return Err("unsupported patch record flag".into());
            }
            let padded = size.checked_add(3).ok_or("patch size overflow")? & !3;
            let end = at
                .checked_add(8 + padded)
                .filter(|&n| n <= data.len())
                .ok_or("patch record exceeds input")?;
            let text = data[at + 8..at + 8 + size].to_vec();
            // The DLL ignores alignment bytes and writes the terminator itself.
            if text.contains(&0) {
                return Err("embedded patch text terminator".into());
            }
            if rows.insert(flagged & 0x7fff_ffff, text).is_some() {
                return Err("duplicate patch offset".into());
            }
            at = end;
        }
        if blocks.insert(key, rows).is_some() {
            return Err("duplicate patch key".into());
        }
    }
    Ok(blocks)
}
fn rewrite_expression(
    expr: &Expression,
    code: &mut [u8],
    offsets: &BTreeMap<u32, u32>,
) -> Result<usize> {
    let mut changed = 0;
    for term in &expr.terms {
        match term {
            Term::Scalar {
                pc,
                tag: 0x120,
                raw,
            } if expr.kind == 0x201 => {
                let &target = offsets.get(raw).ok_or_else(|| {
                    format!("literal at {pc:x} points outside string boundaries: {raw:x}")
                })?;
                put(code, pc + 2, target);
                changed += usize::from(*raw != target);
            }
            Term::Nested { expression, .. } => {
                changed += rewrite_expression(expression, code, offsets)?
            }
            Term::Indexed { indices, .. } => {
                for index in indices {
                    changed += rewrite_expression(index, code, offsets)?;
                }
            }
            _ => {}
        }
    }
    Ok(changed)
}
pub(crate) fn rewrite(raw: &[u8], rows: &BTreeMap<u32, Vec<u8>>) -> Result<(Vec<u8>, usize)> {
    let script = Script::decode(raw)?;
    script.strings()?; // Validate terminators, then include empty strings too.
    let mut start = 0;
    let strings: Vec<_> = script
        .text()
        .split_inclusive(|&b| b == 0)
        .map(|bytes| {
            let entry = cmvs_core::ps3::TextEntry {
                offset: start,
                bytes: &bytes[..bytes.len() - 1],
            };
            start += bytes.len();
            entry
        })
        .collect();
    let starts: BTreeSet<_> = strings.iter().map(|s| s.offset as u32).collect();
    if rows.keys().any(|k| !starts.contains(k)) {
        return Err("patch offset outside original string table".into());
    }
    let mut text = Vec::new();
    let mut offsets = BTreeMap::new();
    for string in strings {
        offsets.insert(string.offset as u32, text.len() as u32);
        let recoded;
        let value = if let Some(translated) = rows.get(&(string.offset as u32)) {
            translated.as_slice()
        } else {
            let (unicode, _, bad) = encoding_rs::SHIFT_JIS.decode(string.bytes);
            if bad {
                return Err("invalid untranslated Shift-JIS string".into());
            }
            let (encoded, _, bad) = encoding_rs::GBK.encode(&unicode);
            if bad {
                return Err("untranslated string cannot be represented in GBK".into());
            }
            recoded = encoded.into_owned();
            &recoded
        };
        if encoding_rs::GBK.decode(value).2 {
            return Err("invalid GBK patch string".into());
        }
        text.extend(value);
        text.push(0);
    }
    let mut code = script.code().to_vec();
    let instructions = bytecode::decode_layout(&code, script.header.entry_pc, &script.labels())?;
    let mut changed = 0;
    for insn in &instructions {
        if let InstructionKind::Expression(expr) = &insn.kind {
            changed += rewrite_expression(expr, &mut code, &offsets)?;
        }
    }
    // Code length, PCs, labels, branches and the mutable data segment are preserved.
    let after = bytecode::decode_layout(&code, script.header.entry_pc, &script.labels())?;
    if !instructions
        .iter()
        .zip(&after)
        .all(|(a, b)| (a.pc, a.end, a.opcode) == (b.pc, b.end, b.opcode))
    {
        return Err("instruction layout changed".into());
    }
    let mut payload = Vec::new();
    for label in script.labels() {
        push(&mut payload, label);
    }
    payload.extend(code);
    payload.extend(script.unknown());
    payload.extend(text);
    if payload.len() > cmvs_core::MAX_PAYLOAD {
        return Err("converted script exceeds limit".into());
    }
    let mut out = raw[..48].to_vec();
    put(
        &mut out,
        28,
        (payload.len()
            - script.header.label_count as usize * 4
            - script.header.code_size as usize
            - script.header.unknown_size as usize) as u32,
    );
    put(&mut out, 36, 0); // Ordinary uncompressed PS2A; retain its original key.
    put(&mut out, 40, payload.len() as u32);
    let key = script.header.key;
    let xor = ((key >> 3) + (key >> 24)) as u8;
    let shift = (key >> 20) % 5 + 1;
    out.extend(
        payload
            .iter()
            .map(|&b| (b.rotate_left(shift) ^ xor).wrapping_add(124)),
    );
    let verified = Script::decode(&out)?;
    if verified.payload() != payload {
        return Err("PS2A roundtrip differs".into());
    }
    Ok((out, changed))
}
fn encrypt(bytes: &mut [u8]) {
    const TABLE: [u32; 16] = [
        0x4d0d4a5e, 0xb3abf3e1, 0x3c37336d, 0x86c3f5f3, 0x7d4f9b89, 0x58d7de11, 0x6367778d,
        0xa5f34629, 0x067fa4b5, 0xed0ae742, 0xb19450cc, 0xe7204a5a, 0xd9af04f5, 0x5d3b687f,
        0xc1c7a6fd, 0xfc502289,
    ];
    let mut flag = 3;
    let (words, tail) = bytes.as_chunks_mut::<4>();
    for chunk in words {
        let value = u32::from_le_bytes(*chunk)
            .rotate_right(21)
            .wrapping_sub(0x6e58a5c2)
            ^ TABLE[flag];
        *chunk = value.to_le_bytes();
        flag = (flag + 1) & 15;
    }
    let length = tail.len();
    for (i, b) in tail.iter_mut().enumerate() {
        *b = b.wrapping_sub(0x52) ^ (TABLE[flag] >> (4 * (length - i))) as u8;
        flag = (flag + 1) & 15;
    }
}
pub(crate) fn pack(entries: &[(Vec<u8>, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut index = Vec::new();
    let mut data = Vec::new();
    let mut names = BTreeSet::new();
    for (name, bytes) in entries {
        if name.is_empty()
            || !name.is_ascii()
            || name.iter().any(|b| b"/\\:\0".contains(b))
            || !names.insert(name)
        {
            return Err("unsafe/duplicate archive basename".into());
        }
        let mut packed = bytes.clone();
        encrypt(&mut packed);
        for value in [
            25 + name.len() as u32,
            packed.len() as u32,
            data.len() as u32,
            0,
            cpz3_checksum(&packed),
            0xc7f5da63,
        ] {
            push(&mut index, value);
        }
        index.extend(name);
        index.push(0);
        data.extend(packed);
    }
    encrypt(&mut index);
    let mut out = b"CPZ3".to_vec();
    for value in [
        entries.len() as u32 ^ 0x5e9c4f37,
        index.len() as u32 ^ 0xf32aed17,
        cpz3_checksum(&index),
        0xdddddddd ^ 0x7bf4a539,
    ] {
        push(&mut out, value);
    }
    out.extend(index);
    out.extend(data);
    let mut reader = Cursor::new(&out);
    let decoded = ArchiveIndex::read(&mut reader)?;
    for (name, expected) in entries {
        if decoded.read_entry_verified(&mut reader, decoded.find(name)?)? != *expected {
            return Err("CPZ3 roundtrip differs".into());
        }
    }
    Ok(out)
}
fn run(game: &Path, patch: &Path, output: &Path) -> Result<()> {
    let mut blocks = parse_patch(&fs::read(patch)?)?;
    let mut archive = File::open(game.join("data/pack/script.cpz"))?;
    let index = ArchiveIndex::read(&mut archive)?;
    let mut entries = Vec::new();
    let mut report = String::from("name\tkey\tstrings\tremapped_references\tbytes\n");
    let mut keys = BTreeSet::new();
    for (entry_index, entry) in index.entries().iter().enumerate() {
        let raw = index.read_entry_verified(&mut archive, entry_index)?;
        let key = Script::decode(&raw)?.header.key;
        if !keys.insert(key) {
            return Err("duplicate original script key".into());
        }
        let rows = blocks.remove(&key).unwrap_or_default();
        let (converted, changed) = rewrite(&raw, &rows)
            .map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&entry.name)))?;
        report.push_str(&format!(
            "{}\t{key:08x}\t{}\t{changed}\t{}\n",
            std::str::from_utf8(&entry.name)?,
            rows.len(),
            converted.len()
        ));
        entries.push((entry.name.clone(), converted));
    }
    let raw = fs::read(game.join("data/pack/start.ps2"))?;
    let key = Script::decode(&raw)?.header.key;
    if !keys.insert(key) {
        return Err("duplicate startup key".into());
    }
    let rows = blocks.remove(&key).ok_or("startup missing from patch")?;
    let (startup, changed) = rewrite(&raw, &rows)?;
    report.push_str(&format!(
        "start.ps2\t{key:08x}\t{}\t{changed}\t{}\n",
        rows.len(),
        startup.len()
    ));
    if !blocks.is_empty() {
        return Err("unmatched patch script keys".into());
    }
    let packed = pack(&entries)?;
    new_file(&output.join("script.cpz"), &packed)?;
    new_file(&output.join("start.ps2"), &startup)?;
    new_file(&output.join("scripts.tsv"), report.as_bytes())?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "internal helper: <original-game> <decoded-mirai> <existing-empty-output>".into(),
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

    fn fixture_script() -> Vec<u8> {
        let mut code = Vec::new();
        for (kind, tag, value) in [
            (0x200u16, 0x100u16, 3u32),
            (0x201, 0x120, 3),
            (0x201, 0x120, 0),
        ] {
            code.extend(kind.to_le_bytes());
            code.extend(tag.to_le_bytes());
            push(&mut code, value);
            code.extend(0x20fu16.to_le_bytes());
        }
        code.extend(0x400u16.to_le_bytes());
        push(&mut code, 0);
        let mut payload = vec![0; 4];
        payload.extend(&code);
        payload.extend([1, 2, 32, 1]);
        payload.extend(b"\0A\0B\0");
        let mut out = b"PS2A".to_vec();
        for v in [
            48,
            1,
            0,
            1,
            code.len() as u32,
            4,
            5,
            0,
            0,
            payload.len() as u32,
            0,
        ] {
            push(&mut out, v);
        }
        out.extend(payload.iter().map(|&b| b.rotate_left(1).wrapping_add(124)));
        out
    }

    #[test]
    fn longer_text_relocates_only_string_literals_and_preserves_empty_data_labels_and_jumps()
    -> Result<()> {
        let raw = fixture_script();
        let before = Script::decode(&raw)?;
        let rows = BTreeMap::from([(1, vec![0xd6, 0xd0, 0xce, 0xc4])]);
        let (out, count) = rewrite(&raw, &rows)?;
        let after = Script::decode(&out)?;
        assert_eq!(count, 1);
        assert_eq!(after.text(), b"\0\xd6\xd0\xce\xc4\0B\0");
        assert_eq!(after.labels(), before.labels());
        assert_eq!(after.unknown(), before.unknown());
        assert_eq!(word(after.code(), 4)?, 3);
        assert_eq!(word(after.code(), 14)?, 6);
        assert_eq!(word(after.code(), 24)?, 0);
        let mut expected = before.code().to_vec();
        put(&mut expected, 14, 6);
        assert_eq!(after.code(), expected);
        assert!(rewrite(&raw, &BTreeMap::from([(2, b"bad offset".to_vec())])).is_err());
        Ok(())
    }

    #[test]
    fn records_skip_arbitrary_padding_but_reject_duplicate_offsets_and_truncation() -> Result<()> {
        let mut record = Vec::new();
        for v in [1, 0x12345678, 0x80000001, 1] {
            push(&mut record, v);
        }
        record.extend(b"Xabc");
        assert_eq!(parse_patch(&record)?[&0x12345678][&1], b"X");
        for length in 1..record.len() {
            assert!(parse_patch(&record[..length]).is_err());
        }
        let mut duplicate = record.clone();
        put(&mut duplicate, 0, 2);
        duplicate.extend(&record[8..]);
        assert!(parse_patch(&duplicate).is_err());
        let mut duplicate_key = record.clone();
        duplicate_key.extend(&record);
        assert!(parse_patch(&duplicate_key).is_err());
        put(&mut record, 8, 1);
        assert!(parse_patch(&record).is_err());
        Ok(())
    }

    #[test]
    fn cpz3_writer_uses_existing_strict_reader_and_rejects_ambiguous_names() -> Result<()> {
        let entries = vec![
            (b"first.ps2".to_vec(), fixture_script()),
            (b"second.ps2".to_vec(), vec![1, 2, 3, 4, 5, 6, 7]),
        ];
        let bytes = pack(&entries)?;
        let mut stream = Cursor::new(bytes);
        let index = ArchiveIndex::read(&mut stream)?;
        for (name, expected) in &entries {
            assert_eq!(
                &index.read_entry_verified(&mut stream, index.find(name)?)?,
                expected
            );
        }
        assert!(pack(&[entries[0].clone(), entries[0].clone()]).is_err());
        assert!(pack(&[(b"../escape".to_vec(), vec![])]).is_err());
        Ok(())
    }
}
