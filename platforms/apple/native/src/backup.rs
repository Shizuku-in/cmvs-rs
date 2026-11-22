use crate::{Result, fonts};
use cmvs_core::{
    format::{
        MAX_PAYLOAD,
        save::{Csv2Save, SaveFamily},
        system_save::{Css0Save, Css1Save, PersistentStarts, ProfileSystemSave},
    },
    host::settings::Configuration,
    runtime::script_control::Startup,
};
use cmvs_host::GameBootstrap;
use std::{
    fs::{self, File},
    io::{self, Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const MAX_ENTRIES: usize = 4096;
const MAX_DIRECTORY: usize = 4 << 20;
const MAX_FILE: u64 = 256 << 20;
const MAX_TOTAL: u64 = 1 << 30;

fn account(total: &mut u64, size: u64) -> Result<()> {
    if size > MAX_FILE || size > MAX_TOTAL - *total {
        return Err("save backup too large".into());
    }
    *total += size;
    Ok(())
}

fn allowed(name: &str) -> bool {
    if let Some(face) = name.strip_prefix("font-faces/") {
        return !face.is_empty() && face.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    }
    matches!(
        name,
        "system.dat"
            | "system.bak"
            | "cmvs.cfg"
            | "cvns.cfg"
            | "cmvs.cfg.bak"
            | "cvns.cfg.bak"
            | "save000.bak"
    ) || name
        .as_bytes()
        .strip_prefix(b"save")
        .and_then(|n| n.strip_suffix(b".dat"))
        .is_some_and(|n| n.len() == 3 && n.iter().all(u8::is_ascii_digit))
}
fn files(dir: &Path) -> Result<Vec<(String, u64)>> {
    let mut names = Vec::new();
    let mut total = 0;
    let mut add = |name, size| -> Result<()> {
        if names.len() == MAX_ENTRIES {
            return Err("too many save entries".into());
        }
        account(&mut total, size)?;
        names.push((name, size));
        Ok(())
    };
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name() == "font-faces" {
            if !entry.file_type()?.is_dir() {
                return Err("invalid font-faces directory".into());
            }
            for f in fs::read_dir(entry.path())? {
                let f = f?;
                let name = f.file_name();
                if let Some(name) = name.to_str().filter(|name| fonts::is_reference(name))
                    && f.file_type()?.is_file()
                    && fonts::Reference::read(&f.path()).is_ok()
                {
                    add(format!("font-faces/{name}"), f.metadata()?.len())?;
                }
            }
        } else if let Some(name) = entry.file_name().to_str()
            && allowed(name)
            && entry.file_type()?.is_file()
        {
            add(name.into(), entry.metadata()?.len())?;
        }
    }
    Ok(names)
}

pub fn export(
    local_root: Option<&Path>,
    user: &Path,
    archive: &Path,
    identity: &str,
) -> Result<()> {
    if let Some(root) = local_root {
        let lock = cmvs_host::game_savedata(root, user)?;
        write_archive(lock.directory(), archive, identity)
    } else {
        let _lock = ProfileSystemSave::lock_for_export(user)?;
        write_archive(user, archive, identity)
    }
}

const COMMENT_PREFIX: &str = "cmvs-rs save backup: ";

fn write_archive(dir: &Path, archive: &Path, identity: &str) -> Result<()> {
    let mut temp =
        tempfile::NamedTempFile::new_in(archive.parent().ok_or("missing ZIP destination")?)?;
    let names = files(dir)?;
    if names.is_empty() {
        return Err("no save files to export".into());
    }
    let mut zip = ZipWriter::new(temp.as_file_mut());
    if !identity.is_empty() {
        zip.set_comment(format!("{COMMENT_PREFIX}{identity}"));
    }
    for (name, size) in names {
        let file = File::open(dir.join(&name))?;
        zip.start_file(name, SimpleFileOptions::default())?;
        if io::copy(&mut file.take(size + 1), &mut zip)? != size {
            return Err("save file changed during export".into());
        }
    }
    zip.finish()?;
    open_archive(temp.reopen()?)?;
    temp.persist_noclobber(archive)?;
    Ok(())
}

// Save archives fit in classic, single-disk ZIP. Validate the bounded index in
// isolation first: zip otherwise searches earlier payload bytes for an alternate
// end record when a malformed central header fails to parse.
fn open_archive(mut file: File) -> Result<ZipArchive<File>> {
    let end = file.seek(SeekFrom::End(0))?;
    let tail_len = end.min(22 + u16::MAX as u64) as usize;
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0; tail_len];
    file.read_exact(&mut tail)?;
    let pos = tail
        .windows(4)
        .rposition(|w| w == b"PK\x05\x06")
        .ok_or("missing ZIP end record")?;
    let footer = &tail[pos..];
    if footer.len() < 22 {
        return Err("invalid ZIP end record".into());
    }
    let word = |at| u16::from_le_bytes([footer[at], footer[at + 1]]) as usize;
    let dword = |at: usize| u32::from_le_bytes(footer[at..at + 4].try_into().unwrap());
    let entries = word(10);
    let size = dword(12) as usize;
    let start = u64::from(dword(16));
    if word(4) != 0
        || word(6) != 0
        || word(8) != entries
        || entries == 0
        || entries > MAX_ENTRIES
        || size > MAX_DIRECTORY
        || footer.len() != 22 + word(20)
        || start + size as u64 != end - footer.len() as u64
    {
        return Err("unsupported save ZIP directory".into());
    }
    file.seek(SeekFrom::Start(start))?;
    let mut index = vec![0; size];
    file.read_exact(&mut index)?;
    if !index.starts_with(b"PK\x01\x02") || index.windows(4).any(|w| w == b"PK\x05\x06") {
        return Err("invalid save ZIP directory".into());
    }
    index.extend_from_slice(footer);
    index[size + 16..size + 20].fill(0);
    let probe = ZipArchive::new(Cursor::new(index))?;
    if probe.len() != entries {
        return Err("duplicate save ZIP entries".into());
    }
    drop(probe);
    Ok(ZipArchive::new(file)?)
}

enum SaveFormat {
    Cmvs(Option<u32>),
    Cvns(PersistentStarts),
}

pub fn import(boot: &GameBootstrap, user: &Path, local: bool, archive: &Path) -> Result<()> {
    let lock = ProfileSystemSave::open_for_restore(user, &boot.root, local)?;
    let dir = lock.directory();
    let selection = &boot.game.selection;
    let format = if let Some(map) = selection.profile.command_map() {
        SaveFormat::Cvns(PersistentStarts::from_start(&boot.script, &map)?)
    } else {
        let declared = selection.record.map(|r| r.game_id).or_else(|| {
            Startup::execute(&boot.script, selection.profile)
                .ok()?
                .bootstrap
                .settings()
                .get(&0x201d)
                .copied()
        });
        SaveFormat::Cmvs(selection.profile.css1_game_id(declared))
    };
    restore(dir, archive, &crate::inspect::backup_identity(boot), format)
}

impl SaveFormat {
    fn validate(&self, name: &str, path: &Path, size: u64) -> Result<()> {
        if name.starts_with("font-faces/") {
            return fonts::Reference::read(path).map(|_| ());
        }
        let family = match self {
            Self::Cmvs(_) => SaveFamily::Cmvs,
            Self::Cvns(_) => SaveFamily::Cvns,
        };
        if name.contains(".cfg") {
            let cfg = if family == SaveFamily::Cvns {
                "cvns.cfg"
            } else {
                "cmvs.cfg"
            };
            if !name.starts_with(cfg) {
                return Err("configuration belongs to another engine family".into());
            }
            Configuration::read(path)?;
            return Ok(());
        }
        if size > MAX_PAYLOAD as u64 {
            return Err("save exceeds supported size".into());
        }
        let bytes = fs::read(path)?;
        match (self, name) {
            (Self::Cmvs(expected), "system.dat" | "system.bak") => {
                let id = Css1Save::decode(&bytes)?.layout().game_id;
                if id != 0 && *expected != Some(id) {
                    return Err("CSS1 game identifier differs from target".into());
                }
            }
            (Self::Cvns(starts), "save000.dat" | "save000.bak") => {
                Css0Save::decode(&bytes, *starts)?;
            }
            (_, "system.dat" | "system.bak" | "save000.bak") => {
                return Err("system save belongs to another engine family".into());
            }
            _ => {
                Csv2Save::decode_family(&bytes, family)?;
            }
        }
        Ok(())
    }
}

fn restore(dir: &Path, path: &Path, identity: &str, format: SaveFormat) -> Result<()> {
    let staging = tempfile::tempdir_in(dir)?;
    let mut zip = open_archive(File::open(path)?)?;
    if let Some(label) = zip.comment().strip_prefix(COMMENT_PREFIX.as_bytes())
        && label != identity.as_bytes()
    {
        return Err("save backup belongs to another game".into());
    }
    let mut total = 0;
    let mut names = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if file.is_dir() && file.name() == "font-faces/" {
            continue;
        }
        let name = file.name().to_owned();
        if !allowed(&name) || !file.is_file() || file.is_symlink() {
            return Err("invalid save entry".into());
        }
        let n = file.size();
        if let Some(face) = name.strip_prefix("font-faces/") {
            if !fonts::is_reference(face) {
                continue;
            }
            if n > fonts::MAX_REFERENCE {
                return Err("font reference is too large".into());
            }
        }
        account(&mut total, n)?;
        let dest = staging.path().join(&name);
        fs::create_dir_all(dest.parent().unwrap())?;
        if io::copy(&mut (&mut file).take(n + 1), &mut File::create_new(&dest)?)? != n {
            return Err("invalid save entry size".into());
        }
        format.validate(&name, &dest, n)?;
        names.push(name);
    }
    if names.is_empty() {
        return Err("empty save backup".into());
    }
    if names.iter().any(|n| n.contains('/')) {
        let fonts = dir.join("font-faces");
        match fs::create_dir(&fonts) {
            Err(e)
                if e.kind() == io::ErrorKind::AlreadyExists
                    && fs::symlink_metadata(&fonts)?.is_dir() => {}
            result => result?,
        }
    }
    let previous = tempfile::tempdir_in(dir)?;
    let mut committed = Vec::<(PathBuf, Option<PathBuf>)>::new();
    let result = (|| -> Result<()> {
        for (i, name) in names.iter().enumerate() {
            let dest = dir.join(name);
            let old = match fs::symlink_metadata(&dest) {
                Ok(m) if m.is_file() => {
                    let old = previous.path().join(i.to_string());
                    fs::copy(&dest, &old)?;
                    Some(old)
                }
                Ok(_) => return Err("invalid existing save entry".into()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            fs::rename(staging.path().join(name), &dest)?;
            committed.push((dest, old));
        }
        Ok(())
    })();
    if result.is_err() {
        for (dest, old) in committed.into_iter().rev() {
            if let Some(old) = old {
                fs::rename(old, dest)?
            } else {
                fs::remove_file(dest)?
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn backup_carries_font_references_and_restores_legacy_archives() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let game = tmp.path().join("game");
        let user = tmp.path().join("saves");
        let archive = tmp.path().join("save.zip");
        fs::create_dir(&game)?;
        fs::create_dir(&user)?;
        use cmvs_core::format::save::HEADER_SIZE;
        let mut header = [0; HEADER_SIZE];
        header[..4].copy_from_slice(b"CSV2");
        header[4..8].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        header[16..35].copy_from_slice(b"2026-01-01 00:00:00");
        let slot = Csv2Save::encode(&header, b"snapshot", b"", b"script")?;
        fs::write(user.join("save001.dat"), &slot)?;
        let key = fonts::Reference {
            source: fonts::Source::Imported {
                hash: "a".repeat(64),
                index: 2,
            },
            name: "Example".into(),
        }
        .write(&user.join("font-faces"))?;
        let reference = user.join("font-faces").join(&key);
        let legacy_font = "font-faces/CMVS-F-ab";
        fs::write(user.join(legacy_font), b"face")?;
        let held = ProfileSystemSave::open(&user, &game)?;
        assert!(export(None, &user, &archive, "game").is_err());
        drop(held);
        fs::remove_dir(&game)?;
        export(None, &user, &archive, "game")?;
        fs::write(user.join("save001.dat"), b"updated slot")?;
        fs::remove_dir_all(user.join("font-faces"))?;
        assert!(restore(&user, &archive, "other", SaveFormat::Cmvs(Some(0))).is_err());
        assert_eq!(fs::read(user.join("save001.dat"))?, b"updated slot");
        restore(&user, &archive, "game", SaveFormat::Cmvs(Some(0)))?;
        assert_eq!(fs::read(user.join("save001.dat"))?, slot);
        fonts::Reference::read(&reference)?;
        assert!(!user.join(legacy_font).exists());

        let legacy = tmp.path().join("legacy.zip");
        let mut zip = ZipWriter::new(File::create(&legacy)?);
        zip.start_file("save001.dat", SimpleFileOptions::default())?;
        zip.write_all(&slot)?;
        zip.start_file(legacy_font, SimpleFileOptions::default())?;
        zip.write_all(b"face")?;
        zip.finish()?;
        fs::write(user.join("save001.dat"), b"updated slot again")?;
        restore(&user, &legacy, "game", SaveFormat::Cmvs(Some(0)))?;
        assert_eq!(fs::read(user.join("save001.dat"))?, slot);
        assert!(!user.join(legacy_font).exists());
        Ok(())
    }

    #[test]
    fn export_size_limits() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let user = tmp.path().join("user");
        fs::create_dir(&user)?;
        File::create(user.join("save001.dat"))?.set_len(MAX_FILE + 1)?;
        let archive = tmp.path().join("save.zip");
        assert!(export(None, &user, &archive, "").is_err());
        assert!(!archive.exists());

        for i in 1..=5 {
            File::create(user.join(format!("save{i:03}.dat")))?.set_len(MAX_FILE)?;
        }
        assert!(export(None, &user, &archive, "").is_err());
        assert!(!archive.exists());
        Ok(())
    }

    #[test]
    fn legacy_backup_with_foreign_system_id_preserves_both_generations() -> Result<()> {
        use cmvs_core::{
            format::system_save::Css1Parameters, host::random::CrtRandom, vm::evaluate::State,
        };
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path();
        let archive = dir.join("saves.zip");
        fs::write(dir.join("system.dat"), b"original primary")?;
        fs::write(dir.join("system.bak"), b"original backup")?;
        let mut state = State::default();
        state.fill_bits(2048, 260096, false)?;
        state.fill_integers(2048, 2048, 0)?;
        state.fill_floats(1024, 1024, 0)?;
        state.fill_strings(64, 64, b"")?;
        let mut zip = ZipWriter::new(File::create(&archive)?);
        let params = Css1Parameters::draw(&mut CrtRandom::new(1));
        for (name, id) in [("system.dat", 42), ("system.bak", 51)] {
            zip.start_file(name, SimpleFileOptions::default())?;
            zip.write_all(&Css1Save::encode_initial_state(
                &state,
                id,
                params,
                [&[], &[]],
            )?)?;
        }
        zip.finish()?;
        let err = restore(dir, &archive, "game", SaveFormat::Cmvs(Some(42))).unwrap_err();
        assert_eq!(err.to_string(), "CSS1 game identifier differs from target");
        assert_eq!(fs::read(dir.join("system.dat"))?, b"original primary");
        assert_eq!(fs::read(dir.join("system.bak"))?, b"original backup");
        Ok(())
    }

    #[test]
    fn oversized_directory_is_rejected_before_indexing() -> Result<()> {
        let mut archive = tempfile::tempfile()?;
        let mut zip = ZipWriter::new(&mut archive);
        zip.start_file("system.dat", SimpleFileOptions::default())?;
        zip.finish()?;
        archive.seek(SeekFrom::End(-14))?;
        for _ in 0..2 {
            archive.write_all(&(MAX_ENTRIES as u16 + 1).to_le_bytes())?;
        }
        assert_eq!(
            open_archive(archive).unwrap_err().to_string(),
            "unsupported save ZIP directory"
        );
        Ok(())
    }

    #[test]
    fn rejected_entry_leaves_original_file_intact() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path();
        fs::write(dir.join("cmvs.cfg"), b"original")?;
        let archive = dir.join("saves.zip");
        let mut zip = ZipWriter::new(File::create(&archive)?);
        for name in ["cmvs.cfg", "font-faces/../save002.dat"] {
            zip.start_file(name, SimpleFileOptions::default())?;
        }
        zip.finish()?;
        assert_eq!(
            restore(dir, &archive, "game", SaveFormat::Cmvs(Some(0)))
                .unwrap_err()
                .to_string(),
            "invalid save entry"
        );
        assert_eq!(fs::read(dir.join("cmvs.cfg"))?, b"original");
        Ok(())
    }
}
