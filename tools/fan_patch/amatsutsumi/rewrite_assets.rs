//! Offline conversion of the digest-pinned 2022 Amatsutsumi patch.
#[path = "../pb3_writer.rs"]
mod images;
#[allow(dead_code)]
#[path = "../rewrite_scripts.rs"]
mod scripts;
use cmvs_core::{bytecode, cmv::CmvIndex, cpz::ArchiveIndex, pb3, ps3::Script};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs::{self, File, OpenOptions},
    io::{Cursor, Write},
    path::Path,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

fn decode_translation(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut result = String::new();
    let mut at = 0;
    while at < bytes.len() {
        let size = if bytes[at] >= 0x81 { 2 } else { 1 };
        let raw = bytes
            .get(at..at + size)
            .ok_or("truncated patch character")?;
        if raw == [0xa2, 0xe1] {
            // amatsu.dll GetGlyphOutlineA hook 0x10003570 maps this to U+266A.
            result.push('♪');
        } else {
            let (text, _, bad) = encoding_rs::GBK.decode(raw);
            if bad {
                return Err("invalid patch GBK character".into());
            }
            result.push_str(&text);
        }
        at += size;
    }
    scripts::validate_utf8(result.as_bytes())?;
    Ok(result.into_bytes())
}

fn patch_rows(original: &Script, patched: &Script) -> Result<BTreeMap<u32, Vec<u8>>> {
    // Retain the pinned patch revision's code and labels together. Its original
    // text prefix is checked string by string, including empty entries.
    original.strings()?;
    patched.strings()?;
    let before: Vec<_> = original.text().split_inclusive(|&b| b == 0).collect();
    let after: Vec<_> = patched.text().split_inclusive(|&b| b == 0).collect();
    if after.len() < before.len() {
        return Err("truncated patch base text".into());
    }
    let mut rows = BTreeMap::new();
    let mut at = 0;
    for (i, entry) in after.iter().enumerate() {
        let bytes = &entry[..entry.len() - 1];
        let value = if i < before.len() && *entry == before[i] {
            let (text, _, bad) = encoding_rs::SHIFT_JIS.decode(bytes);
            if bad {
                return Err("invalid original Shift-JIS string".into());
            }
            scripts::validate_utf8(text.as_bytes())?;
            text.as_bytes().to_vec()
        } else if i >= before.len()
            || (before[i] == b"\x82\xd9\x82\xbd\x82\xe9\0" && bytes == b"\xa1\xb0\xd3\xa9\xa1\xb1")
        {
            // sn4001n also translates the existing name ほたる to “萤”.
            decode_translation(bytes)?
        } else {
            return Err(format!("unexpected base text edit at {at:x}").into());
        };
        rows.insert(at as u32, value);
        at += entry.len();
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
    let out = images::encode_rgba(&rgba, width, height)?;
    Ok((out, width, height, header.bpp))
}

fn run(game: &Path, stage: &Path, encoding: scripts::OutputEncoding) -> Result<()> {
    let output = stage.join("pack");
    fs::create_dir(&output)?;
    let mut seen = BTreeSet::new();
    let mut report = String::from("name\tpatched\tstrings\tremapped_references\tbytes\n");
    let mut convert = |name: &[u8], raw: Vec<u8>| -> Result<Vec<u8>> {
        let name = std::str::from_utf8(name)?;
        let path = stage.join("sc").join(name);
        let original = Script::decode(&raw)?;
        let patched = path.is_file();
        let source = if patched && name != "start.ps3" {
            if !seen.insert(name.to_owned()) {
                return Err("duplicate patched script".into());
            }
            fs::read(&path)?
        } else {
            raw
        };
        let mut rows = patch_rows(&original, &Script::decode(&source)?)?;
        if name == "start.ps3" {
            // The patch startup belongs to v1.03 and mounts absent update.cpz twice.
            // Retain the installed startup's mounts/initialization and translate its title.
            let patch = Script::decode(&fs::read(&path)?)?;
            let title = patch
                .strings()?
                .last()
                .ok_or("missing Chinese window title")?
                .bytes;
            let title = decode_translation(title)?;
            let offset = original
                .strings()?
                .iter()
                .find(|s| s.bytes == b"\x83A\x83}\x83c\x83c\x83~")
                .ok_or("missing original window title")?
                .offset;
            rows.insert(offset as u32, title);
            seen.insert(name.to_owned());
        }
        let (out, changed) =
            scripts::rewrite_encoded(&source, &rows, encoding, &[('♪', "☆"), ('・', "·")])?;
        let decoded = Script::decode(&out)?;
        bytecode::decode_layout(decoded.code(), decoded.header.entry_pc, &decoded.labels())?;
        report.push_str(&format!(
            "{name}\t{patched}\t{}\t{changed}\t{}\n",
            rows.len(),
            out.len()
        ));
        Ok(out)
    };
    let mut file = File::open(game.join("data/pack/script.cpz"))?;
    let index = ArchiveIndex::read(&mut file)?;
    let mut entries = Vec::new();
    for (i, entry) in index.entries().iter().enumerate() {
        let result = convert(&entry.name, index.read_entry_verified(&mut file, i)?)
            .map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&entry.name)))?;
        entries.push((entry.name.clone(), result));
    }
    let start = convert(b"start.ps3", fs::read(game.join("data/pack/start.ps3"))?)?;
    if seen.len() != 86 {
        return Err("unmatched patched scripts".into());
    }
    write_new(&output.join("start.ps3"), &start)?;
    write_new(&output.join("script.cpz"), &scripts::pack(&entries)?)?;
    write_new(&stage.join("scripts.tsv"), report.as_bytes())?;

    let mut pictures = BTreeMap::new();
    for item in fs::read_dir(stage.join("pic"))? {
        let path = item?.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("invalid image name")?;
        let stem = name.strip_suffix(".png").ok_or("invalid image extension")?;
        pictures.insert(format!("{stem}.pb3").into_bytes(), path);
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
        let mut entries = Vec::new();
        for (i, entry) in index.entries().iter().enumerate() {
            let original = index.read_entry_verified(&mut file, i)?;
            let out = if let Some(path) = pictures.get(&entry.name) {
                if !seen.insert(entry.name.clone()) {
                    return Err("ambiguous image basename".into());
                }
                let (out, w, h, bpp) = png_to_pb3(&fs::read(path)?, &original)
                    .map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&entry.name)))?;
                report.push_str(&format!(
                    "{archive}\t{}\t{w}\t{h}\t{bpp}\t{}\n",
                    std::str::from_utf8(&entry.name)?,
                    out.len()
                ));
                out
            } else {
                original
            };
            entries.push((entry.name.clone(), out));
        }
        write_new(&output.join(archive), &scripts::pack(&entries)?)?;
        println!("Rebuilt {archive}");
    }
    if seen.len() != 114 {
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
    let mut report = String::from("name\twidth\theight\tframes\tsampled_frames\n");
    for name in ["c1", "c2", "c3", "c4", "op", "op2"] {
        let mut file = File::open(stage.join("video").join(format!("{name}.cmv")))?;
        let movie = CmvIndex::read(&mut file)?;
        let frames = movie.header.max_sequence;
        for frame in [0, frames / 2, frames - 1] {
            movie
                .decode_frame(&mut file, frame)
                .map_err(|e| format!("{name}.cmv frame {frame}: {e}"))?;
        }
        report.push_str(&format!(
            "{name}.cmv\t{}\t{}\t{frames}\t0,{},{}\n",
            movie.header.width,
            movie.header.height,
            frames / 2,
            frames - 1
        ));
    }
    write_new(&stage.join("videos.tsv"), report.as_bytes())?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("expected original-game, stage and encoding".into());
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
    fn fixture(text: &[u8], offset: u32) -> Vec<u8> {
        let mut code = vec![1, 2, 0x20, 1];
        code.extend(offset.to_le_bytes());
        code.extend([15, 2, 0x30, 4]);
        let mut out = b"PS2A".to_vec();
        for n in [
            48,
            1,
            0,
            1,
            code.len() as u32,
            4,
            text.len() as u32,
            0,
            0,
            (4 + code.len() + 4 + text.len()) as u32,
            0,
        ] {
            out.extend(n.to_le_bytes());
        }
        let mut payload = vec![0; 4];
        payload.extend(code);
        payload.extend([1, 2, 3, 4]);
        payload.extend(text);
        out.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        out
    }
    #[test]
    fn mixed_source_encodings_restore_notes_and_relocate_references() -> Result<()> {
        let base = fixture(b"\0\x82\xa0\0", 1); // Japanese あ
        let patch = fixture(b"\0\x82\xa0\0\xd6\xd0\xa2\xe1\0Z\0", 9);
        let original = Script::decode(&base)?;
        let source = Script::decode(&patch)?;
        let rows = patch_rows(&original, &source)?;
        assert_eq!(rows[&4], "中♪".as_bytes());
        for (encoding, expected, offset) in [
            (
                scripts::OutputEncoding::Utf8,
                "\0あ\0中♪\0Z\0".as_bytes().to_vec(),
                12,
            ),
            (
                scripts::OutputEncoding::Gbk,
                b"\0\xa4\xa2\0\xd6\xd0\xa1\xee\0Z\0".to_vec(),
                9,
            ),
        ] {
            let (out, _) =
                scripts::rewrite_encoded(&patch, &rows, encoding, &[('♪', "☆"), ('・', "·")])?;
            let out = Script::decode(&out)?;
            assert_eq!(out.text(), expected);
            assert_eq!(out.labels(), source.labels());
            assert_eq!(out.unknown(), source.unknown());
            assert_eq!(u32::from_le_bytes(out.code()[4..8].try_into()?), offset);
        }
        Ok(())
    }
    #[test]
    fn unexpected_base_edits_and_truncated_characters_fail() -> Result<()> {
        assert!(decode_translation(&[0x81]).is_err());
        assert!(decode_translation(&[0x81, 0x20]).is_err());
        assert!(decode_translation(&[0]).is_err());
        let a = Script::decode(&fixture(b"A\0", 0))?;
        let b = Script::decode(&fixture(b"B\0", 0))?;
        assert!(patch_rows(&a, &b).is_err());
        Ok(())
    }
    #[test]
    fn translated_existing_name_is_gbk() -> Result<()> {
        let a = Script::decode(&fixture(b"\x82\xd9\x82\xbd\x82\xe9\0", 0))?;
        let b = Script::decode(&fixture(b"\xa1\xb0\xd3\xa9\xa1\xb1\0", 0))?;
        assert_eq!(patch_rows(&a, &b)?[&0], "“萤”".as_bytes());
        Ok(())
    }
    #[test]
    fn rgb_and_rgba_png_preserve_pixels_and_validate_dimensions() -> Result<()> {
        for color in [png::ColorType::Rgb, png::ColorType::Rgba] {
            let rgba = [
                12,
                34,
                56,
                if color == png::ColorType::Rgb {
                    255
                } else {
                    78
                },
            ];
            let original = images::encode_rgba(&rgba, 1, 1)?;
            let mut raw = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut raw, 1, 1);
                encoder.set_color(color);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()?
                    .write_image_data(&rgba[..if color == png::ColorType::Rgb { 3 } else { 4 }])?;
            }
            let (out, _, _, _) = png_to_pb3(&raw, &original)?;
            assert_eq!(pb3::decode(&out)?.pixels(), rgba);
            assert!(png_to_pb3(&raw, &images::encode_rgba(&[0; 8], 2, 1)?).is_err());
        }
        Ok(())
    }
}
