//! Shared encoding, string relocation, PS2A rebuilding and CPZ3 writing for offline converters.
use cmvs_core::{
    bytecode::{self, Expression, InstructionKind, Term},
    cpz::{ArchiveIndex, Cpz3Index},
    ps3::Script,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    io::Cursor,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Debug)]
pub(crate) enum OutputEncoding {
    Utf8,
    Gbk,
}
impl OutputEncoding {
    pub(crate) fn parse(value: &std::ffi::OsStr) -> Result<Self> {
        match value.to_str() {
            Some("utf-8" | "utf8") => Ok(Self::Utf8),
            Some("gbk") => Ok(Self::Gbk),
            _ => Err("encoding must be utf-8, utf8 or gbk".into()),
        }
    }
    fn encode(self, text: &str, substitutions: &[(char, &str)]) -> Result<Vec<u8>> {
        validate_utf8(text.as_bytes())?;
        if matches!(self, Self::Utf8) {
            return Ok(text.as_bytes().to_vec());
        }
        let mut text = text.to_owned();
        for &(from, to) in substitutions {
            text = text.replace(from, to);
        }
        let (bytes, _, bad) = encoding_rs::GBK.encode(&text);
        if bad {
            return Err(format!("text cannot be represented in GBK: {text}").into());
        }
        Ok(bytes.into_owned())
    }
}

/// Match the runtime's u16 glyph model and NUL-terminated string table.
pub(crate) fn validate_utf8(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes)?;
    if text.chars().any(|c| c == '\0' || c as u32 > 0xffff) {
        return Err("text contains NUL or a character outside the BMP".into());
    }
    Ok(())
}

#[cfg(test)]
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
#[cfg(test)]
pub(crate) fn rewrite(raw: &[u8], rows: &BTreeMap<u32, Vec<u8>>) -> Result<(Vec<u8>, usize)> {
    rewrite_encoded(raw, rows, OutputEncoding::Utf8, &[])
}
pub(crate) fn rewrite_encoded(
    raw: &[u8],
    rows: &BTreeMap<u32, Vec<u8>>,
    encoding: OutputEncoding,
    substitutions: &[(char, &str)],
) -> Result<(Vec<u8>, usize)> {
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
            recoded = unicode.as_bytes().to_vec();
            &recoded
        };
        validate_utf8(value)?;
        text.extend(encoding.encode(std::str::from_utf8(value)?, substitutions)?);
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
    Ok((rebuild_script(raw, &code, &text)?, changed))
}

pub(crate) fn rebuild_script(raw: &[u8], code: &[u8], text: &[u8]) -> Result<Vec<u8>> {
    let script = Script::decode(raw)?;
    if code.len() != script.code().len() {
        return Err("instruction segment size changed".into());
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
    Ok(out)
}
pub(crate) fn pack(entries: &[(Vec<u8>, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut names = BTreeSet::new();
    for (name, _) in entries {
        if name.is_empty()
            || !name.is_ascii()
            || name.iter().any(|b| b"/\\:\0".contains(b))
            || !names.insert(name)
        {
            return Err("unsafe/duplicate archive basename".into());
        }
    }
    let mut writer = Cursor::new(Vec::new());
    Cpz3Index::write(&mut writer, entries)?;
    let out = writer.into_inner();
    let mut reader = Cursor::new(&out);
    let decoded = ArchiveIndex::read(&mut reader)?;
    for (name, expected) in entries {
        if decoded.read_entry_verified(&mut reader, decoded.find(name)?)? != *expected {
            return Err("CPZ3 roundtrip differs".into());
        }
    }
    Ok(out)
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
        let rows = BTreeMap::from([(1, "中文♪・䇲".as_bytes().to_vec())]);
        let (out, count) = rewrite(&raw, &rows)?;
        let after = Script::decode(&out)?;
        assert_eq!(count, 1);
        assert_eq!(after.text(), "\0中文♪・䇲\0B\0".as_bytes());
        assert_eq!(after.labels(), before.labels());
        assert_eq!(after.unknown(), before.unknown());
        assert_eq!(word(after.code(), 4)?, 3);
        assert_eq!(word(after.code(), 14)?, 17);
        assert_eq!(word(after.code(), 24)?, 0);
        let mut expected = before.code().to_vec();
        put(&mut expected, 14, 17);
        assert_eq!(after.code(), expected);
        assert!(rewrite(&raw, &BTreeMap::from([(2, b"bad offset".to_vec())])).is_err());
        Ok(())
    }

    #[test]
    fn both_output_encodings_relocate_literals_and_apply_only_explicit_substitutions() -> Result<()>
    {
        let raw = fixture_script();
        let rows = BTreeMap::from([(1, "中文♪・䇲".as_bytes().to_vec())]);
        let substitutions = [('♪', "☆"), ('・', "·"), ('䇲', "筴")];
        let (gbk, _) = rewrite_encoded(&raw, &rows, OutputEncoding::Gbk, &substitutions)?;
        let gbk = Script::decode(&gbk)?;
        assert_eq!(encoding_rs::GBK.decode(gbk.text()).0, "\0中文☆·筴\0B\0");
        assert_eq!(word(gbk.code(), 14)?, 12);
        let (utf8, _) = rewrite_encoded(&raw, &rows, OutputEncoding::Utf8, &substitutions)?;
        let utf8 = Script::decode(&utf8)?;
        assert_eq!(utf8.text(), "\0中文♪・䇲\0B\0".as_bytes());
        assert_eq!(word(utf8.code(), 14)?, 17);
        assert!(rewrite_encoded(&raw, &rows, OutputEncoding::Gbk, &[]).is_err());
        assert!(OutputEncoding::parse(std::ffi::OsStr::new("utf16")).is_err());
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
    #[test]
    fn utf8_rows_reject_invalid_bytes_non_bmp_and_nul() {
        let raw = fixture_script();
        for bad in [vec![0xff], "😀".as_bytes().to_vec(), b"a\0b".to_vec()] {
            assert!(rewrite(&raw, &BTreeMap::from([(1, bad)])).is_err());
        }
    }
}
