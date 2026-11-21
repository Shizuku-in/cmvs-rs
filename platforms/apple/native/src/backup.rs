use crate::Result;
use cmvs_core::format::system_save::ProfileSystemSave;
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
                let name = format!("font-faces/{}", f.file_name().to_string_lossy());
                if allowed(&name) && f.file_type()?.is_file() {
                    add(name, f.metadata()?.len())?;
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

pub fn run(root: &Path, user: &Path, local: bool, archive: &Path, import: bool) -> Result<()> {
    let lock = if import {
        ProfileSystemSave::open_for_restore(user, root, local)?
    } else if local {
        cmvs_host::game_savedata(root, user)?
    } else {
        ProfileSystemSave::open(user, root)?
    };
    let dir = lock.directory();
    if import {
        return restore(dir, archive);
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(archive.parent().ok_or("missing ZIP destination")?)?;
    let names = files(dir)?;
    if names.is_empty() {
        return Err("no save files to export".into());
    }
    let mut zip = ZipWriter::new(temp.as_file_mut());
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

fn restore(dir: &Path, path: &Path) -> Result<()> {
    let staging = tempfile::tempdir_in(dir)?;
    let mut zip = open_archive(File::open(path)?)?;
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
        account(&mut total, n)?;
        let dest = staging.path().join(&name);
        fs::create_dir_all(dest.parent().unwrap())?;
        if io::copy(&mut (&mut file).take(n + 1), &mut File::create_new(&dest)?)? != n {
            return Err("invalid save entry size".into());
        }
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
    #[test]
    fn backup_round_trip_preserves_fonts_and_profile_lock() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let game = tmp.path().join("game");
        let user = tmp.path().join("saves");
        let archive = tmp.path().join("save.zip");
        fs::create_dir(&game)?;
        fs::create_dir_all(user.join("font-faces"))?;
        fs::write(user.join("system.dat"), b"system")?;
        fs::write(user.join("save001.dat"), b"original slot")?;
        fs::write(user.join("font-faces/CMVS-F-ab"), b"face")?;
        run(&game, &user, false, &archive, false)?;
        let held = ProfileSystemSave::open(&user, &game)?;
        assert!(run(&game, &user, false, &archive, true).is_err());
        drop(held);
        fs::rename(user.join("system.dat"), user.join("system.bak"))?;
        fs::write(user.join("save001.dat"), b"updated slot")?;
        fs::remove_dir_all(user.join("font-faces"))?;
        run(&game, &user, false, &archive, true)?;
        assert_eq!(fs::read(user.join("system.dat"))?, b"system");
        assert_eq!(fs::read(user.join("save001.dat"))?, b"original slot");
        assert_eq!(fs::read(user.join("font-faces/CMVS-F-ab"))?, b"face");
        Ok(())
    }

    #[test]
    fn export_size_limits() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let game = tmp.path().join("game");
        let user = tmp.path().join("user");
        fs::create_dir(&game)?;
        fs::create_dir(&user)?;
        File::create(user.join("save001.dat"))?.set_len(MAX_FILE + 1)?;
        let archive = tmp.path().join("save.zip");
        assert!(run(&game, &user, false, &archive, false).is_err());
        assert!(!archive.exists());

        for i in 1..=5 {
            File::create(user.join(format!("save{i:03}.dat")))?.set_len(MAX_FILE)?;
        }
        assert!(run(&game, &user, false, &archive, false).is_err());
        assert!(!archive.exists());
        Ok(())
    }

    #[test]
    fn oversized_directory_is_rejected_before_indexing() -> Result<()> {
        let mut archive = tempfile::tempfile()?;
        let mut zip = ZipWriter::new(&mut archive);
        zip.start_file("system.dat", SimpleFileOptions::default())?;
        zip.finish()?;
        archive.seek(SeekFrom::End(-14))?;
        use std::io::Write;
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
    fn rejected_entry_leaves_original_slot_intact() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path();
        fs::write(dir.join("save001.dat"), b"original")?;
        let archive = dir.join("saves.zip");
        let mut zip = ZipWriter::new(File::create(&archive)?);
        for name in ["save001.dat", "font-faces/../save002.dat"] {
            zip.start_file(name, SimpleFileOptions::default())?;
        }
        zip.finish()?;
        assert!(restore(dir, &archive).is_err());
        assert_eq!(fs::read(dir.join("save001.dat"))?, b"original");
        Ok(())
    }
}
