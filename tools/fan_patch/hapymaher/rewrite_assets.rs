//! Offline helper for convert_hapymaher_patch.py; shares the verified PS2A/CPZ3 writer.
#[allow(dead_code)]
#[path = "../miranosu/rewrite_scripts.rs"]
mod scripts;

use cmvs_core::{bytecode, cpz::ArchiveIndex, pb3, ps3::Script};
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
        let value = *mapping.get(code).ok_or("character outside mapping")?;
        if value == 0xffff {
            return Err(format!("unmapped patch character: {code:04x}").into());
        }
        chars.push(value);
    }
    encode_gbk(&String::from_utf16(&chars)?)
}

fn encode_gbk(unicode: &str) -> Result<Vec<u8>> {
    let unicode = unicode.replace('♪', "☆").replace('・', "·");
    let (encoded, _, bad) = encoding_rs::GBK.encode(&unicode);
    if bad {
        return Err(format!("patch text cannot be represented in GBK: {unicode}").into());
    }
    Ok(encoded.into_owned())
}

fn patch_rows(
    original: &Script,
    patch: &Script,
    mapping: &[u16],
) -> Result<BTreeMap<u32, Vec<u8>>> {
    // The patch's base revision contains small Japanese wording edits. Match the
    // original table by nonempty entry count, checking ASCII resource names, rather
    // than assuming byte-identical offsets across those edits.
    let before = original.strings()?;
    let after = patch.strings()?;
    if before.is_empty()
        || after.len() <= before.len()
        || before
            .iter()
            .zip(&after)
            .any(|(a, b)| a.bytes.is_ascii() && a.bytes != b.bytes)
        || before.last().map(|s| s.bytes) != after.get(before.len() - 1).map(|s| s.bytes)
    {
        return Err("patch base string table does not match original".into());
    }
    let mut at = after[before.len()].offset;
    let mut rows = BTreeMap::new();
    for entry in &after[..before.len()] {
        let (unicode, _, bad) = encoding_rs::SHIFT_JIS.decode(entry.bytes);
        if bad {
            return Err("invalid base Japanese string".into());
        }
        rows.insert(entry.offset as u32, encode_gbk(&unicode)?);
    }
    for bytes in patch.text()[at..].split_inclusive(|&b| b == 0) {
        rows.insert(at as u32, recode(&bytes[..bytes.len() - 1], mapping)?);
        at += bytes.len();
    }
    Ok(rows)
}

fn png_to_pb3(raw: &[u8], original: &[u8]) -> Result<(Vec<u8>, u32, u32, u16)> {
    let mut reader = png::Decoder::new(Cursor::new(raw)).read_info()?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    if info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
        || width == 0
        || height == 0
        || width > 65535
        || height > 65535
        || u64::from(width) * u64::from(height) * 4 > cmvs_core::MAX_PAYLOAD as u64
    {
        return Err("expected bounded RGBA8 patch PNG".into());
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
    let frame = reader.next_frame(&mut rgba)?;
    rgba.truncate(frame.buffer_size());
    reader.finish()?;
    let count = (width * height) as usize;
    if rgba.len() != count * 4 {
        return Err("PNG pixel count differs".into());
    }
    // The hook copies RGBA pixels over the original decoded image, including alpha.
    let mut flags = Vec::new();
    for value in [1u32, 0, count as u32] {
        flags.extend(value.to_le_bytes());
    }
    flags.resize(13 + count.div_ceil(8), 0);
    let data_at = 52 + 16 + flags.len() * 4;
    let mut out = vec![0; 52];
    out[..4].copy_from_slice(b"PB3B");
    out[8..12].copy_from_slice(&1u32.to_le_bytes());
    out[24..28].copy_from_slice(&width.max(height).to_le_bytes());
    for (i, value) in [1, width as u16, height as u16, 32].iter().enumerate() {
        out[28 + i * 2..30 + i * 2].copy_from_slice(&value.to_le_bytes());
    }
    out[44..48].copy_from_slice(&52u32.to_le_bytes());
    out[48..52].copy_from_slice(&(data_at as u32).to_le_bytes());
    for _ in 0..4 {
        out.extend((flags.len() as u32).to_le_bytes());
    }
    for _ in 0..4 {
        out.extend(&flags);
    }
    for _ in 0..4 {
        out.extend((count as u32).to_le_bytes());
    }
    for channel in [2, 1, 0, 3] {
        out.extend(rgba.iter().skip(channel).step_by(4));
    }
    out.extend([0; 47]);
    let size = out.len() as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    let decoded = pb3::decode(&out)?;
    if decoded.width() != width || decoded.height() != height || decoded.pixels() != rgba {
        return Err("PB3B full RGBA roundtrip differs".into());
    }
    Ok((out, width, height, header.bpp))
}

fn run(game: &Path, stage: &Path) -> Result<()> {
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
                rows.insert(entry.offset as u32, encode_gbk(&unicode)?);
            }
            (raw, rows, false)
        };
        let (converted, changed) =
            scripts::rewrite(&source, &rows).map_err(|e| format!("{basename}: {e}"))?;
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
    let index = ArchiveIndex::read(&mut file)?;
    let mut entries = Vec::new();
    for (i, entry) in index.entries().iter().enumerate() {
        entries.push((
            entry.name.clone(),
            convert(&entry.name, index.read_entry_verified(&mut file, i)?)?,
        ));
    }
    let start = convert(b"start.ps3", fs::read(game.join("data/pack/start.ps3"))?)?;
    if patched.len() != 151 {
        return Err("unmatched patch scripts".into());
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
        let name = name.strip_suffix(".mai").ok_or("invalid image extension")?;
        pictures.insert(name.as_bytes().to_vec(), path);
    }
    let mut seen = BTreeSet::new();
    let mut report = String::from("archive\tname\twidth\theight\toriginal_bpp\tbytes\n");
    // Only image archives are relevant; leave the original audio/video packages intact.
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
            let converted = if let Some(path) = pictures.get(&entry.name) {
                if !seen.insert(entry.name.clone()) {
                    return Err("ambiguous image basename".into());
                }
                let (converted, width, height, bpp) = png_to_pb3(&fs::read(path)?, &original)
                    .map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&entry.name)))?;
                report.push_str(&format!(
                    "{archive}\t{}\t{width}\t{height}\t{bpp}\t{}\n",
                    std::str::from_utf8(&entry.name)?,
                    converted.len()
                ));
                converted
            } else {
                original
            };
            entries.push((entry.name.clone(), converted));
        }
        // This patch adds a second backlog atlas and points its scripts at it.
        if archive == "chip.cpz" && !seen.contains(b"message_log1.pb3".as_slice()) {
            let name = b"message_log1.pb3".to_vec();
            let original = index.read_entry_verified(&mut file, index.find(b"message_log.pb3")?)?;
            let path = pictures.get(&name).ok_or("missing added backlog image")?;
            let (converted, width, height, bpp) = png_to_pb3(&fs::read(path)?, &original)?;
            report.push_str(&format!(
                "{archive}\tmessage_log1.pb3\t{width}\t{height}\t{bpp}\t{}\n",
                converted.len()
            ));
            seen.insert(name.clone());
            entries.push((name, converted));
        }
        write_new(&output.join(archive), &scripts::pack(&entries)?)?;
        println!("Rebuilt {archive}");
    }
    if seen.len() != 111 {
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
    if args.len() != 2 {
        return Err("internal helper: <original-game> <stage-directory>".into());
    }
    run(Path::new(&args[0]), Path::new(&args[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(text: &[u8]) -> Result<Script> {
        let mut raw = b"PS2A".to_vec();
        for value in [
            48u32,
            1,
            0x12345678,
            1,
            2,
            0,
            text.len() as u32,
            0,
            99,
            text.len() as u32 + 6,
            0,
        ] {
            raw.extend(value.to_le_bytes());
        }
        raw.extend([0; 4]);
        raw.extend(0x400u16.to_le_bytes());
        raw.extend(text);
        Script::from_memory_image(&raw).map_err(Into::into)
    }

    #[test]
    fn custom_codes_and_explicit_gbk_symbol_substitutions() -> Result<()> {
        let mut mapping = vec![0xffff; 65536];
        mapping[0x889f] = '简' as u16;
        assert_eq!(
            encoding_rs::GBK.decode(&recode(&[0x88, 0x9f], &mapping)?).0,
            "简"
        );
        assert!(recode(&[0x88], &mapping).is_err());
        assert!(recode(&[0x88, 0xa0], &mapping).is_err());
        assert_eq!(encoding_rs::GBK.decode(&encode_gbk("♪・")?).0, "☆·");
        assert!(encode_gbk("😀").is_err());
        Ok(())
    }

    #[test]
    fn base_wording_edits_do_not_shift_the_custom_table_boundary() -> Result<()> {
        let original = memory(b"\x82\xa0\0last.ps3\0")?;
        let patch = memory(b"\x82\xa0\x82\xa0\0last.ps3\0\x88\x9f\0\0")?;
        let mut mapping = vec![0xffff; 65536];
        mapping[0x889f] = '简' as u16;
        let rows = patch_rows(&original, &patch, &mapping)?;
        assert_eq!(encoding_rs::GBK.decode(&rows[&0]).0, "ああ");
        assert_eq!(encoding_rs::GBK.decode(&rows[&14]).0, "简");
        assert!(rows[&17].is_empty());
        assert!(patch_rows(&original, &memory(b"x\0wrong.ps3\0z\0")?, &mapping).is_err());
        let raw = patch.memory_image(patch.unknown())?;
        assert_eq!(
            Script::decode(&encode_memory(&raw)?)?.payload(),
            patch.payload()
        );
        assert!(encode_memory(&raw[..raw.len() - 1]).is_err());
        Ok(())
    }

    #[test]
    fn png_pixels_alpha_and_row_order_survive_pb3_conversion() -> Result<()> {
        let rgba = [
            255, 0, 10, 0, 20, 255, 30, 128, 40, 50, 255, 255, 60, 70, 80, 1,
        ];
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header()?.write_image_data(&rgba)?;
        }
        let mut original = vec![0; 99];
        original[..4].copy_from_slice(b"PB3B");
        original[30..32].copy_from_slice(&2u16.to_le_bytes());
        original[32..34].copy_from_slice(&2u16.to_le_bytes());
        original[34..36].copy_from_slice(&24u16.to_le_bytes());
        let (out, width, height, bpp) = png_to_pb3(&png, &original)?;
        assert_eq!((width, height, bpp), (2, 2, 24));
        assert_eq!(pb3::decode(&out)?.pixels(), rgba);
        assert_eq!(pb3::Header::parse(&out)?.bpp, 32);
        original[30..32].copy_from_slice(&3u16.to_le_bytes());
        assert!(png_to_pb3(&png, &original).is_err());
        Ok(())
    }
}
