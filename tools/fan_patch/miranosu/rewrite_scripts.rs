//! Offline script rewriting helper; invoked by convert_miranosu_patch.py.
#[path = "../rewrite_scripts.rs"]
mod scripts;
use cmvs_core::{cpz::ArchiveIndex, ps3::Script};
use scripts::{OutputEncoding, pack, rewrite_encoded, validate_utf8};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs::{self, File, OpenOptions},
    io::Write,
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
#[cfg(test)]
fn put(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
#[cfg(test)]
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
            let text = &data[at + 8..at + 8 + size];
            // The DLL ignores alignment bytes and writes the terminator itself.
            if text.contains(&0) {
                return Err("embedded patch text terminator".into());
            }
            let (unicode, _, bad) = encoding_rs::GBK.decode(text);
            if bad {
                return Err("invalid GBK source patch string".into());
            }
            let text = unicode.as_bytes().to_vec();
            validate_utf8(&text)?;
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
fn run(game: &Path, patch: &Path, output: &Path, encoding: OutputEncoding) -> Result<()> {
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
        let (converted, changed) = rewrite_encoded(&raw, &rows, encoding, &[])
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
    let (startup, changed) = rewrite_encoded(&raw, &rows, encoding, &[])?;
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
    if args.len() != 4 {
        return Err(
            "internal helper: <original-game> <decoded-mirai> <empty-output> <encoding>".into(),
        );
    }
    run(
        Path::new(&args[0]),
        Path::new(&args[1]),
        Path::new(&args[2]),
        OutputEncoding::parse(&args[3])?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn source_gbk_is_transcoded_and_invalid_bytes_are_rejected() -> Result<()> {
        let mut record = Vec::new();
        for v in [1, 0x12345678, 0x80000001, 4] {
            push(&mut record, v);
        }
        record.extend([0xd6, 0xd0, 0xce, 0xc4]);
        assert_eq!(parse_patch(&record)?[&0x12345678][&1], "中文".as_bytes());
        record[16..20].fill(0xff);
        assert!(parse_patch(&record).is_err());
        Ok(())
    }
}
