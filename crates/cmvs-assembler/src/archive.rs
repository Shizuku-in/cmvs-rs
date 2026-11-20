//! Write archive replacements by entry number, preserving original lookup order.
mod manifest;
use crate::{Result, write_new};
use cmvs_core::{
    FormatError,
    format::{
        cpz::{ArchiveError, ArchiveIndex},
        digest::{Sha256, sha256_reader},
    },
};
pub use manifest::{check_family, export, pack, version_name};

use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek},
    path::Path,
};

pub fn open(
    source: &mut File,
    path: &Path,
    start: Option<&Path>,
    profile: Option<&cmvs_core::profile::EngineProfile>,
    keys: Option<cmvs_core::format::cpz::Cpz7Parameters>,
) -> Result<ArchiveIndex> {
    use cmvs_core::{
        format::{
            cpz::{ArchiveVersion, Cpz5Mode},
            ps3::Script,
        },
        runtime::script_control::{Startup, archive_index_from_start, packing_mode_from_start},
    };
    source.rewind()?;
    let mut magic = [0; 4];
    source.read_exact(&mut magic)?;
    let version = ArchiveVersion::detect(&magic)?;
    check_family(version, profile)?;
    if let Some(keys) = keys {
        if version != ArchiveVersion::Cpz7 || start.is_some() {
            return Err("--keys requires CPZ7 and excludes --start-script".into());
        }
        return Ok(ArchiveIndex::read_with_cpz7_parameters(source, keys)?);
    }
    let keyed = matches!(version, ArchiveVersion::Cpz5 | ArchiveVersion::Cpz7);
    if !keyed && start.is_some() {
        return Err("--start-script applies to CPZ5 and CPZ7".into());
    }
    let adjacent = ["start.ps3", "start.ps2"]
        .map(|s| path.with_file_name(s))
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| path.with_file_name("start.ps3"));
    if keyed
        && (start.is_some()
            || version == ArchiveVersion::Cpz7
            || profile.is_some() && adjacent.is_file())
    {
        let script = Script::decode(&cmvs_tooling::read_bounded(
            start.unwrap_or(&adjacent),
            cmvs_core::MAX_PAYLOAD + 48,
        )?)?;
        let name = path
            .file_name()
            .ok_or("missing archive basename")?
            .as_encoded_bytes();
        if let Some(p) = profile {
            return Startup::execute(&script, p)?.open(source, name);
        }
        if version == ArchiveVersion::Cpz7 {
            return archive_index_from_start(source, &script, name);
        }
        return Ok(ArchiveIndex::read_with_cpz5_mode(
            source,
            Cpz5Mode::try_from(packing_mode_from_start(&script)?)?,
        )?);
    }
    Ok(ArchiveIndex::read(source)?)
}
/// Verify replacement plaintext before publishing. With `verify_all`, also decrypt
/// each unchanged output entry and compare it with the corresponding source entry.
pub fn replace<R: Read + Seek>(
    source: &mut R,
    index: &ArchiveIndex,
    destination: &Path,
    replacements: &BTreeMap<usize, Vec<u8>>,
    verify_all: bool,
) -> Result<()> {
    write_new(destination, |output| {
        index.write_replacements(source, output, replacements)?;
        let checked = index.read_with_same_settings(output)?;
        checked.verify_written_index(output)?;
        if checked.entries().len() != index.entries().len() {
            return Err("archive entry count changed".into());
        }
        for (i, (before, after)) in index.entries().iter().zip(checked.entries()).enumerate() {
            if before.directory != after.directory || before.name != after.name {
                return Err(format!("archive name changed at entry {i}").into());
            }
            if let Some(expected) = replacements.get(&i) {
                if checked.read_entry_verified(output, i)? != *expected {
                    return Err(format!("replacement differs at entry {i}").into());
                }
            } else if before.size != after.size
                || before.checksum_raw != after.checksum_raw
                || (verify_all
                    && checked.read_entry_verified(output, i)?
                        != index.read_entry_verified(source, i)?)
            {
                return Err(format!("unchanged entry {i} differs").into());
            }
        }
        Ok(())
    })
}

/// Open archive replacement files on demand and verify their plaintext before publication.
pub fn replace_files(
    source: &mut File,
    index: &ArchiveIndex,
    destination: &Path,
    paths: &BTreeMap<usize, &Path>,
    verify_all: bool,
) -> Result<()> {
    if paths.keys().any(|&i| i >= index.entries().len()) {
        return Err("replacement index outside archive".into());
    }
    write_new(destination, |output| {
        index.write_replacement_streams(source, output, |i| {
            paths.get(&i).map(|path| file_input(path)).transpose()
        })?;
        let checked = index.read_with_same_settings(output)?;
        checked.verify_written_index(output)?;
        if checked.entries().len() != index.entries().len() {
            return Err("archive entry count changed".into());
        }
        for (i, (before, after)) in index.entries().iter().zip(checked.entries()).enumerate() {
            if before.name != after.name || before.directory != after.directory {
                return Err(format!("archive name changed at entry {i}").into());
            }
            if let Some(path) = paths.get(&i) {
                verify_file(&checked, output, i, path)?;
            } else {
                if before.size != after.size || before.checksum_raw != after.checksum_raw {
                    return Err(format!("unchanged entry {i} differs").into());
                }
                if !verify_all {
                    continue;
                }
                if entry_digest(&checked, output, i)? != entry_digest(index, source, i)? {
                    return Err(format!("unchanged entry {i} differs").into());
                }
            }
        }
        Ok(())
    })
}

pub(crate) fn file_input(path: &Path) -> std::result::Result<(u32, File), ArchiveError> {
    let file = File::open(path)?;
    let size = u32::try_from(file.metadata()?.len())
        .map_err(|_| FormatError("archive entry exceeds 32 bits"))?;
    Ok((size, file))
}

fn entry_digest(index: &ArchiveIndex, archive: &mut File, i: usize) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    index.copy_entry_verified(archive, i, &mut hash)?;
    Ok(hash.finish())
}

pub(crate) fn verify_file(
    index: &ArchiveIndex,
    archive: &mut File,
    i: usize,
    path: &Path,
) -> Result<()> {
    if entry_digest(index, archive, i)? != sha256_reader(File::open(path)?)? {
        return Err(format!("plaintext differs at entry {i}").into());
    }
    Ok(())
}
