//! Offline conversion of the digest-pinned Aoi Tori 2018 patch.
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
    io::Write,
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

fn patch_rows(patch: &Script) -> Result<BTreeMap<u32, Vec<u8>>> {
    patch.strings()?;
    let mut rows = BTreeMap::new();
    let mut at = 0;
    for bytes in patch.text().split_inclusive(|&b| b == 0) {
        let bytes = &bytes[..bytes.len() - 1];
        let (text, _, bad) = encoding_rs::GBK.decode(bytes);
        if bad {
            return Err(format!("invalid GBK string at {at:x}").into());
        }
        // Longinus.dll 0x100015B0 maps configured A1E1 to U+266A.
        rows.insert(at as u32, text.replace('♂', "♪").into_bytes());
        at += bytes.len() + 1;
    }
    Ok(rows)
}
fn name_hash(name: &[u8]) -> Result<u32> {
    let name = std::str::from_utf8(name)?.to_ascii_uppercase();
    Ok(name
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .fold(0u32, |h, b| h.wrapping_mul(131).wrapping_add(u32::from(b)))
        & 0x7fffffff)
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
        let patch = stage.join("sc").join(format!("{:08x}", name_hash(name)?));
        let (source, rows, has_patch) = if basename == "start.ps3" {
            let original = Script::decode(&raw)?;
            let mut rows = BTreeMap::new();
            for entry in original.strings()? {
                let (unicode, _, bad) = encoding_rs::SHIFT_JIS.decode(entry.bytes);
                if bad {
                    return Err("invalid original Japanese string".into());
                }
                let text = if unicode.contains("アオイトリ") {
                    "青鸟"
                } else {
                    &unicode
                };
                rows.insert(entry.offset as u32, text.as_bytes().to_vec());
            }
            patched.insert(format!("{:08x}", name_hash(name)?));
            (raw, rows, true)
        } else if patch.is_file() {
            if !patched.insert(format!("{:08x}", name_hash(name)?)) {
                return Err("hash collision".into());
            }
            let source = encode_memory(&fs::read(patch)?)?;
            let rows = patch_rows(&Script::decode(&source)?)?;
            (source, rows, true)
        } else {
            let original = Script::decode(&raw)?;
            let mut rows = BTreeMap::new();
            for entry in original.strings()? {
                let (text, _, bad) = encoding_rs::SHIFT_JIS.decode(entry.bytes);
                if bad {
                    return Err("invalid untranslated Japanese".into());
                }
                rows.insert(entry.offset as u32, text.as_bytes().to_vec());
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
    if patched.len() != 85 {
        return Err("unmatched patch scripts".into());
    }
    write_new(&output.join("start.ps3"), &converted_start)?;
    rewrite_archive(game, &start, &output, "script.cpz", &entries)?;
    write_new(&stage.join("scripts.tsv"), report.as_bytes())?;

    let mut pictures = BTreeMap::new();
    for item in fs::read_dir(stage.join("pic"))? {
        let path = item?.path();
        let hash = u32::from_str_radix(
            path.file_name()
                .and_then(|s| s.to_str())
                .ok_or("invalid hash")?,
            16,
        )?;
        pictures.insert(hash, path);
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
            .any(|e| pictures.contains_key(&name_hash(&e.name).unwrap_or(u32::MAX)))
        {
            continue;
        }
        let mut entries = BTreeMap::new();
        for (i, entry) in index.entries().iter().enumerate() {
            if let Some(path) = pictures.get(&name_hash(&entry.name)?) {
                let original = index.read_entry_verified(&mut file, i)?;
                if !seen.insert(name_hash(&entry.name)?) {
                    return Err("ambiguous image basename".into());
                }
                let header = pb3::Header::parse(&original)?;
                let mut pixels = fs::read(path)?;
                if pixels.len() != usize::from(header.width) * usize::from(header.height) * 4 {
                    return Err(format!(
                        "{}: patch BGRA dimensions differ: {} vs {}x{}",
                        std::str::from_utf8(&entry.name)?,
                        pixels.len(),
                        header.width,
                        header.height
                    )
                    .into());
                }
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                let converted = images::encode_rgba_with_bpp(
                    &pixels,
                    u32::from(header.width),
                    u32::from(header.height),
                    header.bpp,
                )?;
                if pb3::decode(&converted)?.pixels() != pixels {
                    return Err("image pixels differ".into());
                }
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
                .map(|n| format!("{n:08x}"))
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
    fn longinus_hash_uses_uppercase_utf16le_bytes() -> Result<()> {
        assert_eq!(name_hash(b"start.ps3")?, 0x11e8870e);
        assert_eq!(name_hash(b"START.PS3")?, 0x11e8870e);
        assert_ne!(name_hash(b"start.ps2")?, 0x11e8870e);
        Ok(())
    }

    #[test]
    fn gbk_table_and_trailing_empty_strings_survive_relocation() -> Result<()> {
        let text = b"\xd6\xd0\0last.ps3\0\0\0";
        let code = [1, 2, 0x20, 1, 13, 0, 0, 0, 15, 2, 0x30, 4];
        let source = memory_with_code(text, &code)?;
        let rows = patch_rows(&source)?;
        assert_eq!(rows[&0], "中".as_bytes());
        assert!(rows[&12].is_empty());
        assert!(rows[&13].is_empty());
        let raw = encode_memory(&source.memory_image(source.unknown())?)?;
        let (out, _) = scripts::rewrite_encoded(&raw, &rows, scripts::OutputEncoding::Utf8, &[])?;
        let decoded = Script::decode(&out)?;
        assert_eq!(decoded.text(), "中\0last.ps3\0\0\0".as_bytes());
        assert_eq!(&decoded.code()[4..8], &14u32.to_le_bytes());
        assert_eq!(patch_rows(&memory(b"\xa1\xe1\0")?)?[&0], "♪".as_bytes());
        assert!(patch_rows(&memory(b"\x81\0")?).is_err());
        assert!(encode_memory(&raw[..raw.len() - 1]).is_err());
        Ok(())
    }
}
