//! Offline translation-table/BMP conversion; no patch DLL is executed.
#[path = "../pb3_writer.rs"]
mod images;
#[allow(dead_code)]
#[path = "../rewrite_scripts.rs"]
mod scripts;

use cmvs_core::{cpz::ArchiveIndex, pb3, ps3::Script};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
type StringIndex = BTreeMap<(u32, String), Vec<usize>>;

struct Record {
    offset: u32,
    original: String,
    translated: Vec<u8>,
}

fn encode_utf8(text: &str) -> Result<Vec<u8>> {
    scripts::validate_utf8(text.as_bytes())?;
    Ok(text.as_bytes().to_vec())
}

fn japanese(bytes: &[u8]) -> Result<String> {
    let (text, _, bad) = encoding_rs::SHIFT_JIS.decode(bytes);
    if bad {
        return Err("invalid original Shift-JIS string".into());
    }
    Ok(text.into_owned())
}

fn parse_trans(raw: &[u8]) -> Result<Vec<Record>> {
    if raw.len() > cmvs_core::MAX_PAYLOAD {
        return Err("translation table exceeds limit".into());
    }
    let mut records = Vec::new();
    for line in std::str::from_utf8(raw)?.lines() {
        let fields: Vec<_> = line.split('|').collect();
        if fields.len() != 3
            || fields[0].is_empty()
            || fields[1].is_empty()
            || fields[1].contains('\0')
            || !fields[0].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid translation record".into());
        }
        records.push(Record {
            offset: u32::from_str_radix(fields[0], 16)?,
            original: fields[1].to_owned(),
            translated: encode_utf8(fields[2])?,
        });
    }
    if records.is_empty() {
        return Err("empty translation table".into());
    }
    Ok(records)
}

fn assign_records(records: &[Record], index: &StringIndex) -> Result<Vec<usize>> {
    let candidates: Vec<_> = records
        .iter()
        .map(|r| {
            index
                .get(&(r.offset, r.original.clone()))
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    if let Some(i) = candidates.iter().position(Vec::is_empty) {
        return Err(format!("unmatched translation record {i}: {}", records[i].original).into());
    }
    let mut previous = vec![None; records.len()];
    let mut next = vec![None; records.len()];
    let mut anchor = None;
    for (i, c) in candidates.iter().enumerate() {
        previous[i] = anchor;
        if c.len() == 1 {
            anchor = Some(i);
        }
    }
    anchor = None;
    for (i, c) in candidates.iter().enumerate().rev() {
        next[i] = anchor;
        if c.len() == 1 {
            anchor = Some(i);
        }
    }
    candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if c.len() == 1 {
                return Ok(c[0]);
            }
            let before = previous[i].map(|p| candidates[p][0]);
            let after = next[i].map(|n| candidates[n][0]);
            // The export groups rows by script. Unique offset+source anchors identify
            // each group; shared prelude rows belong to the following script when
            // their offsets restart below the previous group's final unique row.
            let prev_match = before.filter(|p| c.contains(p));
            let next_match = after.filter(|n| c.contains(n));
            if let Some(p) = prev_match
                && prev_match == next_match
            {
                return Ok(p);
            }
            if let Some(n) = next_match
                && (prev_match.is_none()
                    || previous[i].is_some_and(|p| records[i].offset < records[p].offset))
            {
                return Ok(n);
            }
            prev_match.ok_or_else(|| format!("ambiguous translation record {i}").into())
        })
        .collect()
}

fn bmp_rgba(raw: &[u8]) -> Result<(Vec<u8>, u32, u32)> {
    fn word(raw: &[u8], at: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(
            raw.get(at..at + 4)
                .ok_or("truncated BMP header")?
                .try_into()?,
        ))
    }
    if raw.len() < 54
        || &raw[..2] != b"BM"
        || word(raw, 2)? as usize != raw.len()
        || word(raw, 10)? != 54
        || word(raw, 14)? != 40
        || raw[26..30] != [1, 0, 32, 0]
        || word(raw, 30)? != 0
    {
        return Err("expected uncompressed top-down BGRA32 BMP".into());
    }
    let width = word(raw, 18)? as i32;
    let height = word(raw, 22)? as i32;
    if width <= 0 || width > 65535 || !(-65535..0).contains(&height) {
        return Err("unsupported BMP dimensions or row order".into());
    }
    let (width, height) = (width as u32, (-height) as u32);
    let count = u64::from(width) * u64::from(height) * 4;
    if count > cmvs_core::MAX_PAYLOAD as u64
        || raw.len() as u64 != count + 56
        || raw[raw.len() - 2..] != [0, 0]
    {
        return Err("BMP pixels/trailer exceed bounds".into());
    }
    // DLL sub_10001650 copies bytes at +54 directly, including the alpha byte.
    let mut rgba = raw[54..raw.len() - 2].to_vec();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok((rgba, width, height))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

fn image_dimensions_match(name: &[u8], original: [u32; 2], replacement: [u32; 2]) -> bool {
    original == replacement
        // The 1.05 atlas widens the canvas without moving the shared sprites.
        || (name == b"dialog_chip.pb3" && original == [1024, 720] && replacement == [1280, 720])
}

fn run(game: &Path, stage: &Path, encoding: scripts::OutputEncoding) -> Result<()> {
    let mut file = File::open(game.join("data/pack/script.cpz"))?;
    let archive = ArchiveIndex::read(&mut file)?;
    let mut sources = Vec::new();
    for (i, entry) in archive.entries().iter().enumerate() {
        sources.push((
            std::str::from_utf8(&entry.name)?.to_owned(),
            archive.read_entry_verified(&mut file, i)?,
        ));
    }
    sources.push((
        "start.ps3".into(),
        fs::read(game.join("data/pack/start.ps3"))?,
    ));
    if sources.len() != 86 {
        return Err("unexpected original script count".into());
    }
    let mut index = StringIndex::new();
    for (i, (_, raw)) in sources.iter().enumerate() {
        for entry in Script::decode(raw)?.strings()? {
            index
                .entry((entry.offset as u32, japanese(entry.bytes)?))
                .or_default()
                .push(i);
        }
    }
    let mut records = parse_trans(&fs::read(stage.join("Project/trans.txt"))?)?;
    // One source row was edited in the patch export. Only this exact source
    // punctuation differs in the pinned Japanese game; keep its translation.
    let exported = "「Ｄ・Ｄ、きさまこのゲームやり込んでいるなッ！」";
    let installed = "「Ｄ・Ｄ、きさまこのゲームやり込んでいるなッ! 」";
    let mut adapted = 0;
    for r in &mut records {
        if r.offset == 0xc41a && r.original == exported {
            r.original = installed.to_owned();
            adapted += 1;
        }
    }
    if records.len() != 52362 || adapted != 1 {
        return Err("unexpected translation coverage".into());
    }
    let assigned = assign_records(&records, &index)?;
    let mut translations = vec![BTreeMap::new(); sources.len()];
    for (record, target) in records.iter().zip(assigned) {
        if translations[target]
            .insert(record.offset, record.translated.clone())
            .is_some()
        {
            return Err("duplicate translation within script".into());
        }
    }
    if translations.iter().filter(|r| !r.is_empty()).count() != 80 {
        return Err("unexpected translated script count".into());
    }
    let output = stage.join("pack");
    fs::create_dir(&output)?;
    let mut entries = BTreeMap::new();
    let mut report =
        String::from("source\tname\ttranslated_strings\tstrings\tremapped_references\tbytes\n");
    for (i, ((name, raw), translated)) in sources.iter().zip(translations).enumerate() {
        let external = i == sources.len() - 1;
        let location = if external { "external" } else { "script.cpz" };
        let script = Script::decode(raw)?;
        let mut rows = BTreeMap::new();
        for entry in script.strings()? {
            let value = match translated.get(&(entry.offset as u32)) {
                Some(text) => text.clone(),
                None => encode_utf8(&japanese(entry.bytes)?)?,
            };
            rows.insert(entry.offset as u32, value);
        }
        let (out, changed) = scripts::rewrite_encoded(
            raw,
            &rows,
            encoding,
            &[('♪', "☆"), ('・', "·"), ('䇲', "筴")],
        )
        .map_err(|e| format!("{name}: {e}"))?;
        report.push_str(&format!(
            "{location}\t{name}\t{}\t{}\t{changed}\t{}\n",
            translated.len(),
            rows.len(),
            out.len()
        ));
        // CPZ6 also contains a different start.ps3. Retain that archive entry.
        if external {
            write_new(&output.join(name), &out)?;
        } else {
            entries.insert(i, out);
        }
    }
    scripts::write_archive(
        &mut file,
        &archive,
        &output.join("script.cpz"),
        &entries,
        &[],
    )?;
    write_new(&stage.join("scripts.tsv"), report.as_bytes())?;
    println!("Converted 86 scripts / 52362 translation records");

    let mut pictures = BTreeMap::new();
    for item in fs::read_dir(stage.join("Project"))? {
        let path = item?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("invalid picture name")?;
        if let Some(name) = name.strip_suffix(".bmp") {
            pictures.insert(name.as_bytes().to_vec(), path);
        }
    }
    let mut seen = BTreeSet::new();
    let mut report = String::from("archive\tname\twidth\theight\toriginal_bpp\tbytes\n");
    for archive in [
        "balloon.cpz",
        "bg.cpz",
        "chip.cpz",
        "event.cpz",
        "ps.cpz",
        "stand.cpz",
        "up.cpz",
    ] {
        let mut file = File::open(game.join("data/pack").join(archive))?;
        let index = ArchiveIndex::read(&mut file)?;
        if !index
            .entries()
            .iter()
            .any(|e| pictures.contains_key(&e.name))
        {
            continue;
        }
        let mut entries = BTreeMap::new();
        for (i, entry) in index.entries().iter().enumerate() {
            if let Some(path) = pictures.get(&entry.name) {
                let original = index.read_entry_verified(&mut file, i)?;
                if !seen.insert(entry.name.clone()) {
                    return Err("ambiguous image basename".into());
                }
                let header = pb3::Header::parse(&original)?;
                let (rgba, width, height) = bmp_rgba(&fs::read(path)?)?;
                if !image_dimensions_match(
                    &entry.name,
                    [u32::from(header.width), u32::from(header.height)],
                    [width, height],
                ) {
                    return Err(
                        format!("BMP/original dimensions differ: {}", path.display()).into(),
                    );
                }
                let out = images::encode_rgba(&rgba, width, height)?;
                report.push_str(&format!(
                    "{archive}\t{}\t{width}\t{height}\t{}\t{}\n",
                    std::str::from_utf8(&entry.name)?,
                    header.bpp,
                    out.len()
                ));
                entries.insert(i, out);
            }
        }
        scripts::write_archive(&mut file, &index, &output.join(archive), &entries, &[])?;
        println!("Rebuilt {archive}");
    }
    if pictures.len() != 92 || seen.len() != pictures.len() {
        return Err("unmatched patch images".into());
    }
    write_new(&stage.join("images.tsv"), report.as_bytes())?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("internal helper: <original-game> <stage-directory> <encoding>".into());
    }
    run(
        Path::new(&args[0]),
        Path::new(&args[1]),
        scripts::OutputEncoding::parse(&args[2])?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_table_is_strict_and_preserves_symbols() -> Result<()> {
        let records = parse_trans("A|原文|中文♪䇲\r\n".as_bytes())?;
        assert_eq!(records[0].offset, 10);
        assert_eq!(String::from_utf8_lossy(&records[0].translated), "中文♪䇲");
        for bad in ["A|x", "A|x|y|z", "-1|x|y", "A||y", "A|x|😀", "A|x|a\0b", ""] {
            assert!(parse_trans(bad.as_bytes()).is_err());
        }
        assert!(parse_trans(&[0xff]).is_err());
        Ok(())
    }

    #[test]
    fn script_anchors_disambiguate_repeated_source_and_shared_preludes() -> Result<()> {
        let records = parse_trans(b"10|first|one\n20|shared|two\n50|end|three\n1|prelude|four\n10|second|five\n20|shared|six\n")?;
        let index = BTreeMap::from([
            ((0x10, "first".into()), vec![0]),
            ((0x20, "shared".into()), vec![0, 1]),
            ((0x50, "end".into()), vec![0]),
            ((1, "prelude".into()), vec![0, 1]),
            ((0x10, "second".into()), vec![1]),
        ]);
        assert_eq!(assign_records(&records, &index)?, [0, 0, 0, 1, 1, 1]);
        assert!(assign_records(&records, &BTreeMap::new()).is_err());
        let ambiguous = parse_trans(b"20|shared|x\n")?;
        assert!(assign_records(&ambiguous, &index).is_err());
        Ok(())
    }

    fn bmp() -> Vec<u8> {
        let mut b = vec![0; 72];
        b[..2].copy_from_slice(b"BM");
        for (at, value) in [
            (2, 72u32),
            (10, 54),
            (14, 40),
            (18, 2),
            (22, (-2i32) as u32),
        ] {
            b[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        b[26..30].copy_from_slice(&[1, 0, 32, 0]);
        b[54..70].copy_from_slice(&[
            10, 0, 255, 0, 30, 255, 20, 128, 255, 50, 40, 255, 80, 70, 60, 1,
        ]);
        b
    }

    #[test]
    fn bmp_bgra_alpha_and_top_down_rows_survive_pb3_roundtrip() -> Result<()> {
        let b = bmp();
        let (rgba, w, h) = bmp_rgba(&b)?;
        assert_eq!(
            rgba,
            [
                255, 0, 10, 0, 20, 255, 30, 128, 40, 50, 255, 255, 60, 70, 80, 1
            ]
        );
        assert_eq!(
            pb3::decode(&images::encode_rgba(&rgba, w, h)?)?.pixels(),
            rgba
        );
        assert!(images::encode_rgba(&rgba[..15], w, h).is_err());
        for at in [0, 2, 10, 14, 18, 22, 26, 28, 30, 70] {
            let mut invalid = b.clone();
            invalid[at] ^= 1;
            // Changing a negative height by one also changes the expected size.
            assert!(
                bmp_rgba(&invalid).is_err(),
                "accepted mutated header/trailer at {at}"
            );
        }
        assert!(bmp_rgba(&b[..53]).is_err());
        Ok(())
    }

    #[test]
    fn only_the_known_dialog_atlas_may_expand() {
        assert!(image_dimensions_match(
            b"dialog_chip.pb3",
            [1024, 720],
            [1280, 720]
        ));
        assert!(image_dimensions_match(
            b"other.pb3",
            [1024, 720],
            [1024, 720]
        ));
        assert!(!image_dimensions_match(
            b"other.pb3",
            [1024, 720],
            [1280, 720]
        ));
        assert!(!image_dimensions_match(
            b"dialog_chip.pb3",
            [1024, 720],
            [1280, 721]
        ));
    }
}
