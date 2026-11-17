//! Offline conversion of the digest-pinned 2020 Realive patch.

use crate::common::{
    OutputEncoding, PngDepthPolicy, Result, encode_memory, encode_utf8, png_to_pb3,
    rewrite_cpz7_archive, rewrite_encoded, write_new,
};
use cmvs_core::{
    format::ps3::Script, runtime::script_control::archive_index_from_start, vm::bytecode,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::Path,
};

const GBK_SUBSTITUTIONS: &[(char, &str)] = &[
    ('♪', "☆"),
    ('・', "·"),
    ('≪', "《"),
    ('≫', "》"),
    ('‧', "·"),
    ('∀', "▽"),
    ('´', "′"),
    ('｡', "。"),
    ('ﾉ', "ノ"),
    ('ﾞ', "゛"),
    ('ﾟ', "゜"),
];

fn recode(text: &[u8], mapping: &[u16]) -> Result<Vec<u8>> {
    let mut chars = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let mut code = usize::from(text[at]);
        at += 1;
        if (129..=159).contains(&code) || (224..=252).contains(&code) {
            code = code * 256 + usize::from(*text.get(at).ok_or("truncated custom character")?);
            at += 1;
        }
        if (0xa1..=0xdf).contains(&code) {
            // The patch's kaomoji retain single-byte CP932; the DLL's inverse
            // table leaves these as '?'. Recover the source character explicitly.
            let byte = [code as u8];
            let (text, _, bad) = encoding_rs::SHIFT_JIS.decode(&byte);
            if bad {
                return Err("invalid halfwidth character".into());
            }
            chars.extend(text.encode_utf16());
            continue;
        }
        let value = *mapping.get(code).ok_or("character outside mapping")?;
        if value == 0xffff {
            return Err(format!("unmapped patch character: {code:04x}").into());
        }
        chars.push(value);
    }
    encode_utf8(&String::from_utf16(&chars)?)
}

fn patch_rows(
    original: &Script,
    patch: &Script,
    mapping: &[u16],
) -> Result<BTreeMap<u32, Vec<u8>>> {
    original.strings()?;
    patch.strings()?;
    if !patch.text().starts_with(original.text()) || patch.text().len() <= original.text().len() {
        return Err("patch does not retain the exact original text prefix".into());
    }
    let mut rows = BTreeMap::new();
    let mut at = 0;
    for bytes in patch.text().split_inclusive(|&b| b == 0) {
        let bytes = &bytes[..bytes.len() - 1];
        let unicode = if at < original.text().len() {
            let (text, _, bad) = encoding_rs::SHIFT_JIS.decode(bytes);
            if bad {
                return Err("invalid original Japanese string".into());
            }
            encode_utf8(&text)?
        } else {
            recode(bytes, mapping)?
        };
        rows.insert(at as u32, unicode);
        at += bytes.len() + 1;
    }
    Ok(rows)
}

pub fn run(game: &Path, stage: &Path, encoding: OutputEncoding) -> Result<()> {
    let start = Script::decode(&fs::read(game.join("data/pack/start.ps3"))?)?;
    let output = stage.join("pack");
    fs::create_dir(&output)?;
    let table = fs::read(stage.join("characters.bin"))?;
    if table.len() != 131072 {
        return Err("invalid character map size".into());
    }
    let mapping: Vec<_> = table
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let mut patched = BTreeSet::new();
    let mut report = String::from("name\tpatched\tstrings\tremapped_references\tbytes\n");
    let mut convert = |name: &[u8], raw: Vec<u8>| -> Result<Vec<u8>> {
        let basename = std::str::from_utf8(name)?;
        let patch = stage.join("sc").join(basename);
        let (source, rows, has_patch) = if patch.is_file() {
            if !patched.insert(basename.to_owned()) {
                return Err("duplicate script basename".into());
            }
            let original = Script::decode(&raw)?;
            let source = encode_memory(&fs::read(patch)?)?;
            let rows = patch_rows(&original, &Script::decode(&source)?, &mapping)
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
                rows.insert(entry.offset as u32, encode_utf8(&unicode)?);
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
    if patched.len() != 71 {
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
        let name = name.strip_suffix(".mai").ok_or("invalid image extension")?;
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
                )
                .map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&entry.name)))?;
                report.push_str(&format!(
                    "{archive}\t{}\t{}\t{}\t{}\t1\t{}\t{}\n",
                    std::str::from_utf8(&entry.name)?,
                    converted.width,
                    converted.height,
                    converted.original_kind,
                    converted.encoded_bpp,
                    converted.bytes.len()
                ));
                entries.insert(i, converted.bytes);
            }
        }
        rewrite_cpz7_archive(game, &start, &output, archive, &entries)?;
        println!("Rebuilt {archive}");
    }
    if seen.len() != 188 {
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
    use cmvs_core::format::pb3;

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
    fn custom_codes_preserve_utf8_symbols() -> Result<()> {
        let mut mapping = vec![0xffff; 65536];
        mapping[0x889f] = '简' as u16;
        assert_eq!(
            String::from_utf8_lossy(&recode(&[0x88, 0x9f], &mapping)?),
            "简"
        );
        for (code, value) in mapping.iter_mut().take(128).enumerate() {
            *value = code as u16;
        }
        assert_eq!(
            String::from_utf8_lossy(&recode(b"(\xa1>\x88\x9f<\xa1)\xc9\xde", &mapping)?),
            "(｡>简<｡)ﾉﾞ"
        );
        assert!(recode(&[0x88], &mapping).is_err());
        assert!(recode(&[0x88, 0xa0], &mapping).is_err());
        assert_eq!(String::from_utf8_lossy(&encode_utf8("♪・")?), "♪・");
        assert!(encode_utf8("😀").is_err());
        Ok(())
    }

    #[test]
    fn mixed_text_requires_the_exact_prefix_and_preserves_empty_entries() -> Result<()> {
        let original = memory(b"\x82\xa0\0last.ps3\0")?;
        let patch = memory(b"\x82\xa0\0last.ps3\0\x88\x9f\0\0")?;
        let mut mapping = vec![0xffff; 65536];
        mapping[0x889f] = '简' as u16;
        let rows = patch_rows(&original, &patch, &mapping)?;
        assert_eq!(String::from_utf8_lossy(&rows[&0]), "あ");
        assert_eq!(String::from_utf8_lossy(&rows[&12]), "简");
        assert!(rows[&15].is_empty());
        assert!(patch_rows(&original, &memory(b"x\0last.ps3\0z\0")?, &mapping).is_err());
        let raw = patch.memory_image(patch.data())?;
        assert_eq!(
            Script::decode(&encode_memory(&raw)?)?.payload(),
            patch.payload()
        );
        assert!(encode_memory(&raw[..raw.len() - 1]).is_err());
        Ok(())
    }

    #[test]
    fn gbk_replacements_are_explicit_and_utf8_keeps_source_characters() -> Result<()> {
        let original = memory(b"old\0")?;
        let raw = encode_memory(&original.memory_image(original.data())?)?;
        let text: String = GBK_SUBSTITUTIONS.iter().map(|(c, _)| *c).collect();
        let expected: String = GBK_SUBSTITUTIONS.iter().map(|(_, s)| *s).collect();
        let rows = BTreeMap::from([(0, text.as_bytes().to_vec())]);
        let (utf8, _) = rewrite_encoded(&raw, &rows, OutputEncoding::Utf8, GBK_SUBSTITUTIONS)?;
        assert_eq!(
            Script::decode(&utf8)?.text(),
            format!("{text}\0").as_bytes()
        );
        let (gbk, _) = rewrite_encoded(&raw, &rows, OutputEncoding::Gbk, GBK_SUBSTITUTIONS)?;
        assert_eq!(
            encoding_rs::GBK.decode(Script::decode(&gbk)?.text()).0,
            format!("{expected}\0")
        );
        Ok(())
    }

    #[test]
    fn png_pixels_alpha_and_row_order_survive_pb3_conversion() -> Result<()> {
        let rgba = [
            255, 0, 10, 0, 20, 255, 30, 128, 40, 50, 255, 255, 60, 70, 80, 1,
        ];
        for color in [png::ColorType::Rgba, png::ColorType::Rgb] {
            let pixels: Vec<_> = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    if color == png::ColorType::Rgb {
                        p[..3].to_vec()
                    } else {
                        p.to_vec()
                    }
                })
                .collect();
            let mut png = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut png, 2, 2);
                encoder.set_color(color);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header()?.write_image_data(&pixels)?;
            }
            let mut original = vec![0; 99];
            original[..4].copy_from_slice(b"PB3B");
            original[30..32].copy_from_slice(&2u16.to_le_bytes());
            original[32..34].copy_from_slice(&2u16.to_le_bytes());
            let original_bpp: u16 = if color == png::ColorType::Rgb { 24 } else { 32 };
            original[34..36].copy_from_slice(&original_bpp.to_le_bytes());
            let converted = png_to_pb3(&png, &original, true, PngDepthPolicy::PreserveOriginal)?;
            assert_eq!(
                (converted.width, converted.height, converted.original_bpp),
                (2, 2, original_bpp)
            );
            let mut expected = rgba;
            if color == png::ColorType::Rgb {
                for p in expected.as_chunks_mut::<4>().0 {
                    p[3] = 255;
                }
            }
            assert_eq!(pb3::decode(&converted.bytes)?.pixels(), expected);
            assert_eq!(pb3::Header::parse(&converted.bytes)?.bpp, original_bpp);
            if color == png::ColorType::Rgba {
                original[34..36].copy_from_slice(&24u16.to_le_bytes());
                assert!(
                    png_to_pb3(&png, &original, true, PngDepthPolicy::PreserveOriginal).is_err()
                );
            }
            original[30..32].copy_from_slice(&3u16.to_le_bytes());
            assert!(png_to_pb3(&png, &original, true, PngDepthPolicy::PreserveOriginal).is_err());
        }
        Ok(())
    }
}
