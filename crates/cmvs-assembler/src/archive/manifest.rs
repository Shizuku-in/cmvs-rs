//! Ordered archive manifests. Resource names are bytes, disk paths address numbered inputs.
use super::{file_input, verify_file};
use crate::{Result, write_new};
use cmvs_core::{
    MAX_PAYLOAD,
    format::cpz::{ArchiveIndex, ArchiveVersion, Cpz5Mode, DirectoryFormat, DirectoryNames},
    profile::EngineProfile,
};
use cmvs_tooling::{
    table::{Encoding, escape, unescape},
    write_new_dir,
};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};

pub fn version_name(version: ArchiveVersion) -> &'static str {
    use ArchiveVersion::*;
    match version {
        Cpz1 => "cpz1",
        Cpz2 => "cpz2",
        Cpz3 => "cpz3",
        Cpz5 => "cpz5",
        Cpz6 => "cpz6",
        Cpz7 => "cpz7",
    }
}
fn version(name: &str) -> Result<ArchiveVersion> {
    let v = ArchiveVersion::detect(name.to_ascii_uppercase().as_bytes())?;
    if name != version_name(v) {
        return Err("unknown archive format".into());
    }
    Ok(v)
}

pub fn check_family(version: ArchiveVersion, profile: Option<&EngineProfile>) -> Result<()> {
    if profile.is_some_and(|p| {
        p.is_cvns() != matches!(version, ArchiveVersion::Cpz1 | ArchiveVersion::Cpz2)
    }) {
        return Err("archive version differs from selected profile family".into());
    }
    Ok(())
}

pub fn export(index: &ArchiveIndex, source: &mut File, destination: &Path) -> Result<()> {
    write_new_dir(destination, |root| {
        let mut manifest = BufWriter::new(File::create(root.join("archive.tsv"))?);
        writeln!(manifest, "cmvs-cpz-v1\t{}", version_name(index.version()))?;
        match index.directory_format() {
            Some(DirectoryFormat::Cpz5 { mode, encrypted }) => writeln!(
                manifest,
                "mode\t{}\nencryption\t{}",
                mode as u32,
                u8::from(encrypted)
            )?,
            Some(DirectoryFormat::Cpz7 {
                parameters: p,
                encrypted,
            }) => writeln!(
                manifest,
                "parameters\t{:08x}\t{:08x}\t{:08x}\t{:08x}\nencryption\t{}",
                p.directory_step_xor,
                p.file_step_xor,
                p.seed_add,
                p.resource_seed_xor,
                u8::from(encrypted)
            )?,
            _ => {}
        }
        let mut i = 0;
        let mut files = |manifest: &mut BufWriter<File>, count: usize| -> Result<()> {
            for _ in 0..count {
                let file = format!("{i:06}.bin");
                writeln!(
                    manifest,
                    "file\t{}\t{file}",
                    escape(&index.entries()[i].name, Encoding::ShiftJis)
                )?;
                index.copy_entry_verified(source, i, &mut File::create(root.join(file))?)?;
                i += 1;
            }
            Ok(())
        };
        if index.version().is_flat() {
            files(&mut manifest, index.entries().len())?;
        } else {
            for (name, count) in index.directories() {
                writeln!(manifest, "dir\t{}", escape(name, Encoding::ShiftJis))?;
                files(&mut manifest, *count as usize)?;
            }
        }
        Ok(manifest.flush()?)
    })
}

pub fn pack(
    manifest: &Path,
    destination: &Path,
    format: Option<&str>,
    profile: Option<&EngineProfile>,
) -> Result<()> {
    let mut lines = BufReader::new(File::open(manifest)?).lines();
    let header = lines.next().ok_or("empty archive manifest")??;
    let recorded = header
        .strip_prefix("cmvs-cpz-v1\t")
        .ok_or("expected cmvs-cpz-v1 header")?;
    let target = version(if recorded == "-" {
        format.ok_or("choose --format for this manifest")?
    } else {
        recorded
    })?;
    if format.is_some_and(|v| v != version_name(target)) {
        return Err("selected format differs from archive manifest".into());
    }
    check_family(target, profile)?;
    let parent = manifest
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let mut dirs: Vec<DirectoryNames> = Vec::new();
    let mut inputs = Vec::new();
    let mut index_size = 0;
    let (mut mode, mut encryption, mut parameters) = (None, None, None);
    for (i, line) in lines.enumerate() {
        let line = line?;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["mode", v] if target == ArchiveVersion::Cpz5 && mode.is_none() => {
                mode = Some(Cpz5Mode::try_from(v.parse::<u32>()?)?)
            }
            ["encryption", v @ ("0" | "1")]
                if matches!(target, ArchiveVersion::Cpz5 | ArchiveVersion::Cpz7)
                    && encryption.is_none() =>
            {
                encryption = Some(*v == "1")
            }
            ["parameters", a, b, c, d]
                if target == ArchiveVersion::Cpz7 && parameters.is_none() =>
            {
                let mut words = [0; 4];
                for (w, v) in words.iter_mut().zip([a, b, c, d]) {
                    *w = u32::from_str_radix(v, 16)?;
                }
                parameters = Some(words.into());
            }
            ["dir", name] if !target.is_flat() => {
                let name = unescape(name, Encoding::ShiftJis)?;
                index_size += 17 + name.len();
                dirs.push(DirectoryNames {
                    name,
                    files: Vec::new(),
                });
            }
            ["file", name, input] if !input.is_empty() => {
                let name = unescape(name, Encoding::ShiftJis)?;
                if name.is_empty()
                    || name.contains(&0)
                    || (matches!(
                        target,
                        ArchiveVersion::Cpz3 | ArchiveVersion::Cpz5 | ArchiveVersion::Cpz6
                    ) && name.len() >= 64)
                {
                    return Err(format!("manifest line {}: invalid runtime filename", i + 2).into());
                }
                if dirs.is_empty() {
                    dirs.push(DirectoryNames::default());
                }
                index_size += 29 + name.len();
                dirs.last_mut().unwrap().files.push(name);
                inputs.push(crate::manifest_input(&parent, &parent, Path::new(input))?);
            }
            _ => {
                return Err(format!(
                    "manifest line {}: invalid or repeated record for {}",
                    i + 2,
                    version_name(target)
                )
                .into());
            }
        }
        if inputs.len() > 1_000_000 || index_size > MAX_PAYLOAD {
            return Err("archive index exceeds bounds".into());
        }
    }
    let encrypted = encryption.unwrap_or(true);
    let directory_format = match target {
        ArchiveVersion::Cpz5 => Some(DirectoryFormat::Cpz5 {
            mode: mode.unwrap_or_default(),
            encrypted,
        }),
        ArchiveVersion::Cpz6 => Some(DirectoryFormat::Cpz6),
        ArchiveVersion::Cpz7 => Some(DirectoryFormat::Cpz7 {
            parameters: parameters.unwrap_or_default(),
            encrypted,
        }),
        _ => None,
    };
    write_new(destination, |output| {
        let open = |i: usize| file_input(&inputs[i]);
        let index = if let Some(f) = directory_format {
            f.write(output, &dirs, open)?;
            f.read(output)?
        } else {
            target.write_flat(
                output,
                dirs.first().map_or(&[], |d| d.files.as_slice()),
                open,
            )?;
            ArchiveIndex::read(output)?
        };
        index.verify_written_index(output)?;
        if index.version() != target
            || index.entries().len() != inputs.len()
            || (!target.is_flat()
                && !index
                    .directories()
                    .iter()
                    .map(|(n, c)| (n.as_ref(), *c as usize))
                    .eq(dirs.iter().map(|d| (d.name.as_slice(), d.files.len()))))
        {
            return Err("packed archive layout differs from manifest".into());
        }
        for (i, name) in dirs.iter().flat_map(|d| &d.files).enumerate() {
            if index.entries()[i].name != *name {
                return Err(format!("packed filename differs at entry {i}").into());
            }
            verify_file(&index, output, i, &inputs[i])?;
        }
        Ok(())
    })
}
