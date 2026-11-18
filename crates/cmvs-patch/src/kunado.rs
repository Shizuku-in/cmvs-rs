//! Offline conversion of the digest-pinned Kunado 1.2 patch.

use crate::common::{
    OutputEncoding, PngDepthPolicy, Result, encode_memory, png_to_pb3, rewrite_cpz7_archive,
    rewrite_encoded, write_new,
};
use cmvs_core::{
    format::pb3, format::ps3::Script, runtime::script_control::archive_index_from_start,
    vm::bytecode,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::Path,
};

const GBK_SUBSTITUTIONS: &[(char, &str)] = &[('♪', "☆"), ('・', "·")];

fn patch_rows(
    original: &Script,
    patch: &Script,
    correct_typo: bool,
) -> Result<BTreeMap<u32, Vec<u8>>> {
    original.strings()?;
    patch.strings()?;
    let mut prefix = original.text().to_vec();
    // The pinned patch corrects duplicated Japanese 「ため」 in sn1260.
    if !patch.text().starts_with(&prefix) {
        if !correct_typo {
            return Err("patch does not retain original text prefix".into());
        }
        let typo = [0x82, 0xbd, 0x82, 0xdf, 0x82, 0xbd, 0x82, 0xdf];
        let at = prefix
            .windows(typo.len())
            .position(|b| b == typo)
            .ok_or("patch does not retain original text prefix")?;
        prefix.drain(at..at + 4);
        if !patch.text().starts_with(&prefix) {
            return Err("patch does not retain corrected original text prefix".into());
        }
    }
    let mut rows = BTreeMap::new();
    let mut at = 0;
    for bytes in patch.text().split_inclusive(|&b| b == 0) {
        let bytes = &bytes[..bytes.len() - 1];
        let decoder = if at < prefix.len() {
            encoding_rs::SHIFT_JIS
        } else {
            encoding_rs::GBK
        };
        let (text, _, bad) = decoder.decode(bytes);
        if bad {
            return Err(format!("invalid source string at {at:x}").into());
        }
        rows.insert(at as u32, text.as_bytes().to_vec());
        at += bytes.len() + 1;
    }
    Ok(rows)
}

pub fn run(game: &Path, stage: &Path, encoding: OutputEncoding) -> Result<()> {
    let start = Script::decode(&fs::read(game.join("data/pack/start.ps3"))?)?;
    let output = stage.join("pack");
    fs::create_dir(&output)?;
    let mut patched = BTreeSet::new();
    let mut report = String::from("name\tpatched\tstrings\tremapped_references\tbytes\n");
    let mut convert = |name: &[u8], raw: Vec<u8>| -> Result<Vec<u8>> {
        let basename = std::str::from_utf8(name)?;
        let patch = stage.join("sc").join(basename);
        let (source, rows, has_patch) = if basename == "start.ps3" {
            let original = Script::decode(&raw)?;
            let mut rows = patch_rows(&original, &original, false)?;
            let title = original
                .strings()?
                .into_iter()
                .find(|s| encoding_rs::SHIFT_JIS.decode(s.bytes).0.contains("クナド"))
                .ok_or("missing original window title")?;
            rows.insert(title.offset as u32, "久那土国记".as_bytes().to_vec());
            patched.insert(basename.to_owned());
            (raw, rows, true)
        } else if patch.is_file() {
            if !patched.insert(basename.to_owned()) {
                return Err("duplicate script basename".into());
            }
            let original = Script::decode(&raw)?;
            let source = encode_memory(&fs::read(patch)?)?;
            let rows = patch_rows(
                &original,
                &Script::decode(&source)?,
                basename == "sn1260.ps3",
            )
            .map_err(|e| format!("{basename}: {e}"))?;
            (source, rows, true)
        } else {
            let script = Script::decode(&raw)?;
            let mut rows = BTreeMap::new();
            for entry in script.strings()? {
                let (unicode, _, bad) = encoding_rs::SHIFT_JIS.decode(entry.bytes);
                if bad {
                    return Err("invalid original Japanese string".into());
                }
                rows.insert(entry.offset as u32, unicode.as_bytes().to_vec());
            }
            (raw, rows, false)
        };
        let (converted, changed) = rewrite_encoded(&source, &rows, encoding, GBK_SUBSTITUTIONS)
            .map_err(|e| format!("{basename}: {e}"))?;
        let decoded = Script::decode(&converted)?;
        bytecode::decode_layout(decoded.code(), decoded.header().entry_pc, &decoded.labels())?;
        report.push_str(&format!(
            "{basename}\t{has_patch}\t{}\t{changed}\t{}\n",
            rows.len(),
            converted.len()
        ));
        Ok(converted)
    };
    let mut file = File::open(game.join("data/pack/script.cpz"))?;
    let index = archive_index_from_start(&mut file, &start, b"script.cpz")?;
    let mut entries = BTreeMap::new();
    for (i, entry) in index.entries().iter().enumerate() {
        entries.insert(
            i,
            convert(&entry.name, index.read_entry_verified(&mut file, i)?)?,
        );
    }
    let converted_start = convert(b"start.ps3", fs::read(game.join("data/pack/start.ps3"))?)?;
    if patched.len() != 58 {
        return Err("unmatched patch scripts".into());
    }
    write_new(&output.join("start.ps3"), &converted_start)?;
    rewrite_cpz7_archive(game, &start, &output, "script.cpz", &entries)?;
    write_new(&stage.join("scripts.tsv"), report.as_bytes())?;

    let mut pictures = BTreeMap::new();
    for item in fs::read_dir(stage.join("pic"))? {
        let path = item?.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("invalid image name")?;
        if !name.ends_with(".pb3") {
            return Err("invalid image extension".into());
        }
        pictures.insert(name.as_bytes().to_vec(), path);
    }
    let mut seen = BTreeSet::new();
    let mut report =
        String::from("archive\tname\twidth\theight\toriginal_kind\tkind\tbpp\tbytes\n");
    // Only image archives are relevant; leave the original audio/video packages intact.
    for archive in [
        "balloon.cpz",
        "bg.cpz",
        "chip.cpz",
        "event.cpz",
        "stand.cpz",
        "up.cpz",
    ] {
        let mut file = File::open(game.join("data/pack").join(archive))?;
        let index = archive_index_from_start(&mut file, &start, archive.as_bytes())?;
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
                let converted = png_to_pb3(
                    &fs::read(path)?,
                    &original,
                    true,
                    PngDepthPolicy::PreserveOriginal,
                )?;
                let header = pb3::Header::parse(&converted.bytes)?;
                report.push_str(&format!(
                    "{archive}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    std::str::from_utf8(&entry.name)?,
                    header.width,
                    header.height,
                    converted.original_kind,
                    header.kind,
                    header.bpp,
                    converted.bytes.len()
                ));
                entries.insert(i, converted.bytes);
            }
        }
        rewrite_cpz7_archive(game, &start, &output, archive, &entries)?;
        println!("Rebuilt {archive}");
    }
    if seen.len() != pictures.len() {
        return Err(format!(
            "unmatched patch images: {}",
            pictures
                .keys()
                .filter(|n| !seen.contains(*n))
                .map(|n| String::from_utf8_lossy(n))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into());
    }
    write_new(&stage.join("images.tsv"), report.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(text: &[u8]) -> Result<Script> {
        memory_with_code(text, &[0, 4, 0, 0, 0, 0])
    }

    fn memory_with_code(text: &[u8], code: &[u8]) -> Result<Script> {
        let mut raw = b"PS2A".to_vec();
        for value in [
            48u32,
            1,
            0x12345678,
            1,
            code.len() as u32,
            0,
            text.len() as u32,
            0,
            99,
            (text.len() + code.len() + 4) as u32,
            0,
        ] {
            raw.extend(value.to_le_bytes());
        }
        raw.extend([0; 4]);
        raw.extend(code);
        raw.extend(text);
        Script::from_memory_image(&raw).map_err(Into::into)
    }

    #[test]
    fn mixed_prefix_and_appended_gbk_relocate_in_both_encodings() -> Result<()> {
        let original = memory(b"\x82\xa0\0last.ps3\0")?;
        let patch = memory(b"\x82\xa0\0last.ps3\0\xd6\xd0\xce\xc4\0\0")?;
        let rows = patch_rows(&original, &patch, false)?;
        assert_eq!(rows[&0], "あ".as_bytes());
        assert_eq!(rows[&12], "中文".as_bytes());
        assert!(rows[&17].is_empty());
        assert!(patch_rows(&original, &memory(b"wrong\0")?, false).is_err());
        let code = [1, 2, 0x20, 1, 12, 0, 0, 0, 15, 2, 0x30, 4];
        let source = memory_with_code(patch.text(), &code)?;
        let raw = encode_memory(&source.memory_image(source.data())?)?;
        for (encoding, offset) in [(OutputEncoding::Utf8, 13), (OutputEncoding::Gbk, 12)] {
            let (out, _) = rewrite_encoded(&raw, &rows, encoding, GBK_SUBSTITUTIONS)?;
            let decoded = Script::decode(&out)?;
            assert_eq!(decoded.labels(), source.labels());
            assert_eq!(&decoded.code()[4..8], &(offset as u32).to_le_bytes());
        }
        Ok(())
    }

    #[test]
    fn japanese_typo_correction_requires_explicit_permission() -> Result<()> {
        let original = memory(b"\x82\xbd\x82\xdf\x82\xbd\x82\xdf\0")?;
        let patch = memory(b"\x82\xbd\x82\xdf\0\xd6\xd0\0")?;
        assert!(patch_rows(&original, &patch, false).is_err());
        let rows = patch_rows(&original, &patch, true)?;
        assert_eq!(rows[&0], "ため".as_bytes());
        assert_eq!(rows[&5], "中".as_bytes());
        assert!(patch_rows(&original, &memory(b"other\0")?, true).is_err());
        Ok(())
    }
}
