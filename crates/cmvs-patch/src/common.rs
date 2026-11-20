//! Shared encoding, string relocation, script/image encoding, and archive writing helpers.

use cmvs_core::{
    format::cpz::ArchiveDirectory, format::cpz::ArchiveFile, format::cpz::ArchiveIndex,
    format::cpz::Cpz3Index, format::cpz::Cpz5Index, format::pb3, format::pb3::EncodeDepth,
    format::ps3, format::ps3::Script, runtime::script_control::archive_index_from_start,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{Cursor, Read, Seek, Write},
    path::Path,
};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputEncoding {
    Utf8,
    Gbk,
}

impl OutputEncoding {
    pub fn parse(value: &OsStr) -> Result<Self> {
        match value.to_str() {
            Some("utf-8" | "utf8") => Ok(Self::Utf8),
            Some("gbk") => Ok(Self::Gbk),
            _ => Err("encoding must be utf-8, utf8 or gbk".into()),
        }
    }

    pub fn encode(self, text: &str, substitutions: &[(char, &str)]) -> Result<Vec<u8>> {
        validate_utf8(text.as_bytes())?;
        if matches!(self, Self::Utf8) {
            return Ok(text.as_bytes().to_vec());
        }
        let mut text = text.to_owned();
        for &(from, to) in substitutions {
            text = text.replace(from, to);
        }
        validate_utf8(text.as_bytes())?;
        let (bytes, _, bad) = encoding_rs::GBK.encode(&text);
        if bad {
            return Err(format!("text cannot be represented in GBK: {text}").into());
        }
        Ok(bytes.into_owned())
    }
}

/// Match the runtime's u16 glyph model and NUL-terminated string table.
pub fn validate_utf8(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes)?;
    if text.chars().any(|c| c == '\0' || c as u32 > 0xffff) {
        return Err("text contains NUL or a character outside the BMP".into());
    }
    Ok(())
}

pub fn encode_utf8(text: &str) -> Result<Vec<u8>> {
    validate_utf8(text.as_bytes())?;
    Ok(text.as_bytes().to_vec())
}

pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

pub fn encode_memory(raw: &[u8]) -> Result<Vec<u8>> {
    Ok(ps3::encode_memory_image(raw)?)
}

pub fn rewrite_encoded(
    raw: &[u8],
    rows: &BTreeMap<u32, Vec<u8>>,
    encoding: OutputEncoding,
    substitutions: &[(char, &str)],
) -> Result<(Vec<u8>, usize)> {
    let script = Script::decode(raw)?;
    use cmvs_assembler::script::{ScriptText, Segment};
    let layout = ScriptText::new(&script)?;
    let mut edits = BTreeMap::new();
    let mut used = 0;
    for (segment, offset, original) in layout.strings() {
        if segment == Segment::Data {
            continue;
        }
        let value = match rows.get(&(offset as u32)) {
            Some(translated) => {
                used += 1;
                encoding.encode(std::str::from_utf8(translated)?, substitutions)?
            }
            None => {
                let (unicode, _, bad) = encoding_rs::SHIFT_JIS.decode(original);
                if bad {
                    return Err("invalid untranslated Shift-JIS string".into());
                }
                encoding.encode(&unicode, substitutions)?
            }
        };
        if value != original {
            edits.insert((segment, offset), value);
        }
    }
    if used != rows.len() {
        return Err("patch offset outside original string table".into());
    }
    let (updated, relocated) = cmvs_assembler::script::rewrite(&layout, &edits)?;
    Ok((updated.encode()?, relocated))
}

pub fn rebuild_script(raw: &[u8], code: &[u8], text: &[u8]) -> Result<Vec<u8>> {
    let script = Script::decode(raw)?;
    let updated = script.with_segments(code, text)?;
    let out = updated.encode()?;
    let verified = Script::decode(&out)?;
    if verified.payload() != updated.payload() {
        return Err("PS2A roundtrip differs".into());
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PngDepthPolicy {
    /// Always encode 32-bit RGBA PB3B (`amatsutsumi`, `hapymaher`).
    ForceRgba32,
    /// Preserve the original PB3B header's 24-bit or 32-bit depth (`kunado`, `realive`).
    PreserveOriginal,
}

pub struct ConvertedImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub original_kind: u16,
    pub original_bpp: u16,
    pub encoded_bpp: u16,
}

pub fn png_to_pb3(
    raw: &[u8],
    original: &[u8],
    allow_rgb: bool,
    policy: PngDepthPolicy,
) -> Result<ConvertedImage> {
    let reader = png::Decoder::new(Cursor::new(raw)).read_info()?;
    let color = reader.info().color_type;
    if reader.info().bit_depth != png::BitDepth::Eight
        || (color != png::ColorType::Rgba && !(allow_rgb && color == png::ColorType::Rgb))
    {
        return Err("expected 8-bit RGB or RGBA patch PNG".into());
    }
    let (width, height) = reader.info().size();
    let header = pb3::Header::parse(original)?;
    if u32::from(header.width) != width || u32::from(header.height) != height {
        return Err(format!(
            "PNG/original dimensions differ: {width}x{height} vs {}x{}",
            header.width, header.height
        )
        .into());
    }
    let depth = match policy {
        PngDepthPolicy::ForceRgba32 => EncodeDepth::Rgba32,
        PngDepthPolicy::PreserveOriginal => EncodeDepth::from_bpp(header.bpp)?,
    };
    let bytes =
        cmvs_assembler::image::encode_pb3(&cmvs_assembler::image::png_pixels(reader)?, depth)?;
    Ok(ConvertedImage {
        bytes,
        width,
        height,
        original_kind: header.kind,
        original_bpp: header.bpp,
        encoded_bpp: depth.bpp(),
    })
}

pub fn pack_cpz3(entries: &[(Vec<u8>, Vec<u8>)]) -> Result<Vec<u8>> {
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

/// Additional files inherit the directory of an existing template entry.
pub struct AddedEntry {
    pub template: usize,
    pub name: Vec<u8>,
    pub bytes: Vec<u8>,
}

/// Preserve the source format, using a fresh CPZ5 index only when files are added.
pub fn write_archive<R: Read + Seek>(
    source: &mut R,
    index: &ArchiveIndex,
    destination: &Path,
    replacements: &BTreeMap<usize, Vec<u8>>,
    additions: &[AddedEntry],
) -> Result<()> {
    if replacements.keys().any(|i| *i >= index.entries().len()) {
        return Err("replacement outside source archive".into());
    }
    if additions.is_empty() {
        return cmvs_assembler::archive::replace(source, index, destination, replacements, true);
    }
    let ArchiveIndex::Cpz5(original) = index else {
        return Err("additional patch files require CPZ5".into());
    };
    if original.layout.encryption_mode != 1 {
        return Err("additional patch files require CPZ5 resource mode 1".into());
    }
    let mut dirs: Vec<ArchiveDirectory> = Vec::new();
    for (i, entry) in index.entries().iter().enumerate() {
        if dirs
            .last()
            .is_none_or(|d| d.name != entry.directory.as_ref())
        {
            dirs.push(ArchiveDirectory {
                name: entry.directory.to_vec(),
                files: Vec::new(),
            });
        }
        let raw = index.read_entry_verified(source, i)?;
        dirs.last_mut().expect("directory").files.push(ArchiveFile {
            name: entry.name.clone(),
            data: replacements.get(&i).cloned().unwrap_or(raw),
        });
    }
    let mut names: BTreeSet<_> = index.entries().iter().map(|e| e.name.clone()).collect();
    for addition in additions {
        if !names.insert(addition.name.clone()) {
            return Err("duplicate additional filename".into());
        }
        let directory = &index
            .entries()
            .get(addition.template)
            .ok_or("invalid additional file template")?
            .directory;
        let targets: Vec<_> = dirs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.name == directory.as_ref())
            .map(|(i, _)| i)
            .collect();
        if targets.len() != 1 {
            return Err("ambiguous additional file directory".into());
        }
        dirs[targets[0]].files.push(ArchiveFile {
            name: addition.name.clone(),
            data: addition.bytes.clone(),
        });
    }
    cmvs_assembler::write_new(destination, |output| {
        Cpz5Index::write(output, original.mode, &dirs)?;
        let checked = index.read_with_same_settings(output)?;
        let expected: Vec<_> = dirs
            .iter()
            .flat_map(|d| d.files.iter().map(move |f| (d, f)))
            .collect();
        if checked.entries().len() != expected.len() {
            return Err("rebuilt archive entry count differs".into());
        }
        for (i, (entry, (dir, file))) in checked.entries().iter().zip(expected).enumerate() {
            if entry.directory.as_ref() != dir.name
                || entry.name != file.name
                || checked.read_entry_verified(output, i)? != file.data
            {
                return Err("rebuilt archive roundtrip differs".into());
            }
        }
        Ok(())
    })
}

pub fn rewrite_cpz7_archive(
    game: &Path,
    start: &Script,
    output: &Path,
    name: &str,
    replacements: &BTreeMap<usize, Vec<u8>>,
) -> Result<()> {
    let mut source = File::open(game.join("data/pack").join(name))?;
    let index = archive_index_from_start(&mut source, start, name.as_bytes())?;
    if !matches!(index, ArchiveIndex::Cpz7(_)) {
        return Err("expected a CPZ7 archive".into());
    }
    cmvs_assembler::archive::replace(&mut source, &index, &output.join(name), replacements, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

    fn word(data: &[u8], at: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(
            data.get(at..at + 4)
                .ok_or("truncated patch field")?
                .try_into()?,
        ))
    }

    fn push_u32(data: &mut Vec<u8>, value: u32) {
        data.extend(value.to_le_bytes());
    }

    fn rewrite(raw: &[u8], rows: &BTreeMap<u32, Vec<u8>>) -> Result<(Vec<u8>, usize)> {
        rewrite_encoded(raw, rows, OutputEncoding::Utf8, &[])
    }

    fn fixture_script() -> Vec<u8> {
        let mut code = Vec::new();
        for (kind, tag, value) in [
            (0x200u16, 0x100u16, 3u32),
            (0x201, 0x120, 3),
            (0x201, 0x120, 0),
        ] {
            code.extend(kind.to_le_bytes());
            code.extend(tag.to_le_bytes());
            push_u32(&mut code, value);
            code.extend(0x20fu16.to_le_bytes());
        }
        code.extend(0x400u16.to_le_bytes());
        push_u32(&mut code, 0);
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
            push_u32(&mut out, v);
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
        assert_eq!(after.data(), before.data());
        assert_eq!(word(after.code(), 4)?, 3);
        assert_eq!(word(after.code(), 14)?, 17);
        assert_eq!(word(after.code(), 24)?, 0);
        let mut expected = before.code().to_vec();
        expected[14..18].copy_from_slice(&17u32.to_le_bytes());
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
        assert!(OutputEncoding::parse(OsStr::new("utf16")).is_err());
        Ok(())
    }

    #[test]
    fn gbk_rejects_nul_introduced_by_substitution() {
        assert!(OutputEncoding::Gbk.encode("x", &[('x', "a\0b")]).is_err());
    }

    #[test]
    fn cpz3_writer_uses_existing_strict_reader_and_rejects_ambiguous_names() -> Result<()> {
        let entries = vec![
            (b"first.ps2".to_vec(), fixture_script()),
            (b"second.ps2".to_vec(), vec![1, 2, 3, 4, 5, 6, 7]),
        ];
        let bytes = pack_cpz3(&entries)?;
        let mut stream = Cursor::new(bytes);
        let index = ArchiveIndex::read(&mut stream)?;
        for (name, expected) in &entries {
            assert_eq!(
                &index.read_entry_verified(&mut stream, index.find(name)?)?,
                expected
            );
        }
        assert!(pack_cpz3(&[entries[0].clone(), entries[0].clone()]).is_err());
        assert!(pack_cpz3(&[(b"../escape".to_vec(), vec![])]).is_err());
        Ok(())
    }

    #[test]
    fn cpz5_additions_follow_template_directory_and_keep_other_files() -> Result<()> {
        use cmvs_core::format::cpz::Cpz5Mode;
        let dirs = vec![
            ArchiveDirectory {
                name: b"first".to_vec(),
                files: vec![ArchiveFile {
                    name: b"old.bin".to_vec(),
                    data: vec![1],
                }],
            },
            ArchiveDirectory {
                name: b"second".to_vec(),
                files: vec![ArchiveFile {
                    name: b"keep.bin".to_vec(),
                    data: vec![2],
                }],
            },
        ];
        let mut source = Cursor::new(Vec::new());
        Cpz5Index::write(&mut source, Cpz5Mode::Mode8, &dirs)?;
        let index = ArchiveIndex::read(&mut source)?;
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("cmvs-patch-append-{}-{id}", std::process::id()));
        std::fs::create_dir(&dir)?;
        let destination = dir.join("new.cpz");
        let additions = [AddedEntry {
            template: 0,
            name: b"added.bin".to_vec(),
            bytes: vec![3],
        }];
        write_archive(
            &mut source,
            &index,
            &destination,
            &BTreeMap::from([(0, vec![4])]),
            &additions,
        )?;
        let mut file = File::open(&destination)?;
        let result = ArchiveIndex::read(&mut file)?;
        assert_eq!(result.entries().len(), 3);
        assert_eq!(result.entries()[1].directory.as_ref(), b"first");
        assert_eq!(result.entries()[1].name, b"added.bin");
        assert_eq!(result.read_entry_verified(&mut file, 0)?, [4]);
        assert_eq!(result.read_entry_verified(&mut file, 1)?, [3]);
        assert_eq!(result.read_entry_verified(&mut file, 2)?, [2]);
        let invalid = dir.join("bad.cpz");
        for (template, name) in [(99, b"bad.bin".to_vec()), (0, b"keep.bin".to_vec())] {
            assert!(
                write_archive(
                    &mut source,
                    &index,
                    &invalid,
                    &BTreeMap::new(),
                    &[AddedEntry {
                        template,
                        name,
                        bytes: vec![]
                    }]
                )
                .is_err()
            );
            assert!(!invalid.exists());
        }
        drop(file);
        std::fs::remove_file(destination)?;
        std::fs::remove_dir(dir)?;
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
