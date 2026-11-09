//! Offline conversion of the digest-pinned Kunado 1.2 patch.
#[path = "../pb3_writer.rs"]
mod images;
#[allow(dead_code)]
#[path = "../rewrite_scripts.rs"]
mod scripts;

use cmvs_core::{
    bytecode,
    cpz::Cpz7Index,
    pb3,
    ps3::Script,
    script_control::{archive_index_from_start, cpz7_parameters_from_start},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs::{self, File, OpenOptions},
    io::{Cursor, Write},
    path::Path,
};
const GBK_SUBSTITUTIONS: &[(char, &str)] = &[('♪', "☆"), ('・', "·")];
type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}
fn encode_memory(raw: &[u8]) -> Result<Vec<u8>> {
    let script = Script::from_memory_image(raw)?;
    let key = script.header.key;
    let xor = ((key >> 3).wrapping_add(key >> 24)) as u8;
    let shift = (key >> 20) % 5 + 1;
    let mut out = raw[..48].to_vec();
    out[36..40].fill(0);
    out.extend(
        script
            .payload()
            .iter()
            .map(|&b| (b.rotate_left(shift) ^ xor).wrapping_add(124)),
    );
    if Script::decode(&out)?.payload() != script.payload() {
        return Err("patch memory image roundtrip differs".into());
    }
    Ok(out)
}

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

fn png_to_pb3(raw: &[u8], original: &[u8]) -> Result<(Vec<u8>, u32, u32, u16)> {
    let mut reader = png::Decoder::new(Cursor::new(raw)).read_info()?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    if !matches!(info.color_type, png::ColorType::Rgba | png::ColorType::Rgb)
        || info.bit_depth != png::BitDepth::Eight
        || width == 0
        || height == 0
        || width > 65535
        || height > 65535
        || u64::from(width) * u64::from(height) * 4 > cmvs_core::MAX_PAYLOAD as u64
    {
        return Err("expected bounded RGB8/RGBA8 patch PNG".into());
    }
    let header = pb3::Header::parse(original)?;
    if u32::from(header.width) != width || u32::from(header.height) != height {
        return Err(format!(
            "PNG/original dimensions differ: {width}x{height} vs {}x{}",
            header.width, header.height
        )
        .into());
    }
    let mut rgba = vec![0; reader.output_buffer_size().ok_or("PNG exceeds limit")?];
    let color = reader.info().color_type;
    let frame = reader.next_frame(&mut rgba)?;
    rgba.truncate(frame.buffer_size());
    reader.finish()?;
    if color == png::ColorType::Rgb {
        rgba = rgba
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
    }
    let count = (width * height) as usize;
    if rgba.len() != count * 4 {
        return Err("PNG pixel count differs".into());
    }
    let out = images::encode_rgba_with_bpp(&rgba, width, height, header.bpp)?;
    Ok((out, width, height, header.bpp))
}

fn rewrite_archive(
    game: &Path,
    start: &Script,
    output: &Path,
    name: &str,
    replacements: &BTreeMap<usize, Vec<u8>>,
) -> Result<()> {
    let parameters = cpz7_parameters_from_start(start, name.as_bytes())?
        .ok_or("missing original CPZ7 registration")?;
    let mut source = File::open(game.join("data/pack").join(name))?;
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join(name))?;
    let index = archive_index_from_start(&mut source, start, name.as_bytes())?;
    index.write_replacements(&mut source, &mut destination, replacements)?;
    destination.flush()?;
    drop(destination);
    let mut verified = File::open(output.join(name))?;
    let index = Cpz7Index::read_with_parameters(&mut verified, parameters)?;
    for (&i, expected) in replacements {
        if index.read_entry_verified(&mut verified, i)? != *expected {
            return Err("CPZ7 replacement roundtrip differs".into());
        }
    }
    Ok(())
}

fn run(game: &Path, stage: &Path, encoding: scripts::OutputEncoding) -> Result<()> {
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
        let (converted, changed) =
            scripts::rewrite_encoded(&source, &rows, encoding, GBK_SUBSTITUTIONS)
                .map_err(|e| format!("{basename}: {e}"))?;
        let decoded = Script::decode(&converted)?;
        bytecode::decode_layout(decoded.code(), decoded.header.entry_pc, &decoded.labels())?;
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
    rewrite_archive(game, &start, &output, "script.cpz", &entries)?;
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
                let (converted, _, _, _) = png_to_pb3(&fs::read(path)?, &original)?;
                let header = pb3::Header::parse(&converted)?;
                let base = pb3::Header::parse(&original)?;
                if (header.width, header.height) != (base.width, base.height) {
                    return Err("patch image dimensions differ".into());
                }
                pb3::decode(&converted)?;
                report.push_str(&format!(
                    "{archive}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    std::str::from_utf8(&entry.name)?,
                    header.width,
                    header.height,
                    base.kind,
                    header.kind,
                    header.bpp,
                    converted.len()
                ));
                entries.insert(i, converted);
            }
        }
        rewrite_archive(game, &start, &output, archive, &entries)?;
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
        let raw = encode_memory(&source.memory_image(source.unknown())?)?;
        for (encoding, offset) in [
            (scripts::OutputEncoding::Utf8, 13),
            (scripts::OutputEncoding::Gbk, 12),
        ] {
            let (out, _) = scripts::rewrite_encoded(&raw, &rows, encoding, GBK_SUBSTITUTIONS)?;
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
