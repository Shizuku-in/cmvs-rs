//! Offline conversion of the digest-pinned 2020 Realive patch.
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
    let key = script.header().key;
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

fn encode_utf8(text: &str) -> Result<Vec<u8>> {
    scripts::validate_utf8(text.as_bytes())?;
    Ok(text.as_bytes().to_vec())
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
        let (converted, changed) =
            scripts::rewrite_encoded(&source, &rows, encoding, GBK_SUBSTITUTIONS)
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
    rewrite_archive(game, &start, &output, "script.cpz", &entries)?;
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
                let (converted, width, height, bpp) = png_to_pb3(&fs::read(path)?, &original)
                    .map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&entry.name)))?;
                report.push_str(&format!(
                    "{archive}\t{}\t{width}\t{height}\t{}\t1\t{bpp}\t{}\n",
                    std::str::from_utf8(&entry.name)?,
                    pb3::Header::parse(&original)?.kind,
                    converted.len()
                ));
                entries.insert(i, converted);
            }
        }
        rewrite_archive(game, &start, &output, archive, &entries)?;
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
        let (utf8, _) = scripts::rewrite_encoded(
            &raw,
            &rows,
            scripts::OutputEncoding::Utf8,
            GBK_SUBSTITUTIONS,
        )?;
        assert_eq!(
            Script::decode(&utf8)?.text(),
            format!("{text}\0").as_bytes()
        );
        let (gbk, _) =
            scripts::rewrite_encoded(&raw, &rows, scripts::OutputEncoding::Gbk, GBK_SUBSTITUTIONS)?;
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
            let (out, width, height, bpp) = png_to_pb3(&png, &original)?;
            assert_eq!((width, height, bpp), (2, 2, original_bpp));
            let mut expected = rgba;
            if color == png::ColorType::Rgb {
                for p in expected.as_chunks_mut::<4>().0 {
                    p[3] = 255;
                }
            }
            assert_eq!(pb3::decode(&out)?.pixels(), expected);
            assert_eq!(pb3::Header::parse(&out)?.bpp, original_bpp);
            if color == png::ColorType::Rgba {
                original[34..36].copy_from_slice(&24u16.to_le_bytes());
                assert!(png_to_pb3(&png, &original).is_err());
            }
            original[30..32].copy_from_slice(&3u16.to_le_bytes());
            assert!(png_to_pb3(&png, &original).is_err());
        }
        Ok(())
    }
}
