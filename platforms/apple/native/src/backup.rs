use crate::Result;
use cmvs_core::format::system_save::ProfileSystemSave;
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

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
fn files(dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
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
                    names.push(name)
                }
            }
        } else if let Some(name) = entry.file_name().to_str()
            && allowed(name)
            && entry.file_type()?.is_file()
        {
            names.push(name.into())
        }
    }
    Ok(names)
}

pub fn run(root: &Path, user: &Path, local: bool, archive: &Path, import: bool) -> Result<()> {
    let lock = if local {
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
    for name in names {
        let mut file = File::open(dir.join(&name))?;
        zip.start_file(name, SimpleFileOptions::default())?;
        io::copy(&mut file, &mut zip)?;
    }
    zip.finish()?;
    temp.persist_noclobber(archive)?;
    Ok(())
}

fn restore(dir: &Path, path: &Path) -> Result<()> {
    let staging = tempfile::tempdir_in(dir)?;
    let mut zip = ZipArchive::new(File::open(path)?)?;
    if zip.len() > 4096 {
        return Err("too many save entries".into());
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
        if n > 256 << 20 || total + n > 1024 << 20 {
            return Err("save backup too large".into());
        }
        total += n;
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
                    fs::hard_link(&dest, &old)?;
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
        fs::write(user.join("save001.dat"), b"original slot")?;
        fs::write(user.join("font-faces/CMVS-F-ab"), b"face")?;
        run(&game, &user, false, &archive, false)?;
        let held = ProfileSystemSave::open(&user, &game)?;
        assert!(run(&game, &user, false, &archive, true).is_err());
        drop(held);
        fs::write(user.join("save001.dat"), b"updated slot")?;
        fs::remove_dir_all(user.join("font-faces"))?;
        run(&game, &user, false, &archive, true)?;
        assert_eq!(fs::read(user.join("save001.dat"))?, b"original slot");
        assert_eq!(fs::read(user.join("font-faces/CMVS-F-ab"))?, b"face");
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
