//! Stable file and entry ordinals. Raw archive names never become host filesystem paths.
use super::*;
use cmvs_core::{
    format::{
        digest::{sha256, sha256_reader, to_hex},
        pb2, pb3,
        ps3::Script,
    },
    host::filesystem::resolve_under,
    profile::{EXECUTABLE_NAMES, Registry},
};
use cmvs_tooling::table::{Encoding, escape, unescape};
use std::{
    io::{BufRead, BufWriter},
    path::Component,
    sync::OnceLock,
};

pub(super) struct SourceFile {
    pub relative: PathBuf,
    pub index: Option<ArchiveIndex>,
    pub first: usize,
    digest: String,
    verified: OnceLock<()>,
}
pub(super) struct Asset {
    pub file: usize,
    pub ordinal: Option<usize>,
    pub codec: String,
    pub reference: Vec<u8>,
    pub edit: Option<PathBuf>,
    pub pb3_bytes: usize,
}
pub(super) struct Catalog {
    pub root: PathBuf,
    pub files: Vec<SourceFile>,
    pub assets: Vec<Asset>,
    pub profile: &'static EngineProfile,
    pub startup: Startup,
}
fn ext(name: &[u8]) -> Vec<u8> {
    name.rsplit(|&b| b == b'.')
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
}
struct Description {
    codec: String,
    reference: Vec<u8>,
    pb3_bytes: usize,
}

fn describe(
    name: &[u8],
    raw: impl FnOnce() -> Result<Vec<u8>>,
    profile: &EngineProfile,
) -> Result<Description> {
    let suffix = ext(name);
    let codec = match suffix.as_slice() {
        b"pb3" if !profile.is_cvns() => {
            let raw = raw()?;
            let mut pb3_bytes = 0;
            let h = charge_pb3(&raw, &mut pb3_bytes)?;
            return Ok(Description {
                codec: format!("pb3/{}/{}", h.kind, h.bpp),
                reference: pb3::reference_name(&raw)?.unwrap_or_default(),
                pb3_bytes,
            });
        }
        b"pb2" if profile.is_cvns() => {
            let h = pb2::Header::parse(&raw()?)?;
            format!("pb2/{}/{}", h.kind, h.bpp)
        }
        b"ps2" | b"ps3" | b"pss" => {
            use cmvs_core::format::ps3::{Container, Header};
            let cvns = suffix == b"pss";
            if cvns != profile.is_cvns()
                || (Header::parse(&raw()?)?.container == Container::Pss0) != cvns
            {
                return Err("script container differs from selected profile family".into());
            }
            if cvns { "pss0" } else { "ps2a" }.into()
        }
        b"msk" if profile.is_cvns() => {
            cmvs_core::format::msk::Header::parse(&raw()?)?;
            "msk".into()
        }
        b"cur" => {
            cmvs_core::format::cursor::CursorDib::parse(&raw()?)?;
            "cur".into()
        }
        b"pb2" | b"pb3" | b"msk" => {
            return Err("resource extension differs from selected profile family".into());
        }
        _ => "raw".into(),
    };
    Ok(Description {
        codec,
        reference: Vec::new(),
        pb3_bytes: 0,
    })
}
fn resolved(root: &Path, name: &[u8]) -> Result<Option<PathBuf>> {
    match resolve_under(root, name) {
        Ok(path) => Ok(Some(path.strip_prefix(root)?.to_owned())),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[derive(Default)]
struct SavePaths {
    directories: BTreeSet<PathBuf>,
    templates: BTreeSet<PathBuf>,
}

impl SavePaths {
    fn resolve(catalog: &Catalog) -> Result<Self> {
        let mut saves = Self::default();
        for directory in std::iter::once(&b"save"[..]).chain(
            catalog
                .startup
                .bootstrap
                .paths()
                .get(&(0x2016, 0))
                .map(Vec::as_slice),
        ) {
            let dir = std::str::from_utf8(directory)?.trim_end_matches(['/', '\\']);
            if !directory.is_empty()
                && let Some(rel) = resolved(&catalog.root, dir.as_bytes())?
            {
                saves.directories.insert(rel);
            }
            let name = catalog.profile.system_save_file_name();
            let template = if dir.is_empty() {
                name.into()
            } else {
                format!("{dir}/{name}")
            };
            if let Some(rel) = resolved(&catalog.root, template.as_bytes())? {
                saves.templates.insert(rel);
            }
        }
        Ok(saves)
    }

    fn excludes(&self, relative: &Path) -> bool {
        self.directories.iter().any(|dir| relative.starts_with(dir))
            && !self.templates.contains(relative)
    }
}

fn is_archive(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cpz"))
}

fn collect_resources(
    root: &Path,
    relative: PathBuf,
    save_dirs: &BTreeSet<PathBuf>,
    out: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    if save_dirs.iter().any(|dir| relative.starts_with(dir)) {
        return Ok(());
    }
    let path = root.join(&relative);
    let ty = fs::symlink_metadata(&path)?.file_type();
    if ty.is_dir() {
        for ent in fs::read_dir(path)? {
            collect_resources(root, relative.join(ent?.file_name()), save_dirs, out)?;
        }
    } else if ty.is_file() {
        out.insert(relative);
    } else {
        return Err(format!("unsupported game file {}", relative.display()).into());
    }
    Ok(())
}
fn relative(text: &str) -> Result<&Path> {
    let path = Path::new(text);
    if text.is_empty()
        || text.contains(['\t', '\n', '\r', '\\', ':'])
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("expected a relative project path".into());
    }
    Ok(path)
}
fn init(root: &Path, profile: Option<&str>) -> Result<(Catalog, GameFiles, String)> {
    let game = Registry::builtin().identify_game(root, profile)?;
    let profile = game.selection.profile;
    let bytes = read_bounded(&root.join(&game.files.start_script), MAX_PAYLOAD + 48)?;
    let script = Script::decode(&bytes)?;
    if profile.is_cvns() != (script.header().container == cmvs_core::format::ps3::Container::Pss0) {
        return Err("startup script differs from profile family".into());
    }
    Ok((
        Catalog {
            root: root.canonicalize()?,
            files: Vec::new(),
            assets: Vec::new(),
            profile,
            startup: Startup::execute(&script, profile)?,
        },
        game.files,
        to_hex(&sha256(&bytes)),
    ))
}
impl SourceFile {
    fn verify_digest(&self, path: &Path) -> Result<()> {
        if to_hex(&sha256_reader(File::open(path)?)?) != self.digest {
            return Err(format!("source differs for {}", self.relative.display()).into());
        }
        Ok(())
    }

    pub fn copy_unchanged(&self, source: &Path, destination: &Path) -> Result<()> {
        fs::copy(source, destination)?;
        self.verify_digest(destination)
    }

    pub fn verify(&self, root: &Path) -> Result<()> {
        if self.verified.get().is_none() {
            self.verify_digest(&root.join(&self.relative))?;
            let _ = self.verified.set(());
        }
        Ok(())
    }
}

impl Catalog {
    pub fn validate_asset(&self, id: usize) -> Result<usize> {
        let a = &self.assets[id];
        let f = &self.files[a.file];
        let name = match a.ordinal {
            Some(n) => &f.index.as_ref().unwrap().entries()[n].name[..],
            None => f.relative.file_name().unwrap().as_encoded_bytes(),
        };
        let source = describe(name, || self.raw(id), self.profile)?;
        if source.codec != a.codec || source.reference != a.reference {
            return Err(format!("asset {id} source codec or reference differs").into());
        }
        Ok(source.pb3_bytes)
    }
    pub fn raw(&self, id: usize) -> Result<Vec<u8>> {
        let a = &self.assets[id];
        let file = &self.files[a.file];
        file.verify(&self.root)?;
        let path = self.root.join(&file.relative);
        if let Some(i) = a.ordinal {
            Ok(file
                .index
                .as_ref()
                .unwrap()
                .read_entry_verified(&mut File::open(path)?, i)?)
        } else {
            read_bounded(&path, MAX_PAYLOAD + 48)
        }
    }
    pub fn base(&self, id: usize, startup: &Startup) -> Result<Option<usize>> {
        let name = &self.assets[id].reference;
        if name.is_empty() {
            return Ok(None);
        }
        for (&(command, _), reg) in startup.bootstrap.paths() {
            if !matches!(command, 0x2011 | 0x2017) || reg.is_empty() {
                continue;
            }
            let dir = reg.ends_with(b"\\");
            let mut path = reg.clone();
            if dir {
                path.extend(name);
            }
            let Some(path) = resolved(&self.root, &path)? else {
                continue;
            };
            if let Some(f) = self.files.iter().find(|f| f.relative == path) {
                if dir && f.index.is_none() {
                    return Ok(Some(f.first));
                }
                if let Some(index) = &f.index
                    && let Some(n) = index.lookup(name)?
                {
                    return Ok(Some(f.first + n));
                }
            }
        }
        Err(format!(
            "asset {id}: no registered image slot contains {}",
            escape(name, Encoding::ShiftJis)
        )
        .into())
    }
}

pub fn export(source: &Path, out: &Path, profile: Option<&str>) -> Result<()> {
    let (catalog, game, start_digest) = init(source, profile)?;
    // Do the directory walk before creating a staging directory.
    let registrations = catalog.startup.bootstrap.paths();
    let saves = SavePaths::resolve(&catalog)?;
    let mut paths = saves.templates.clone();
    let mut roots = BTreeSet::from([PathBuf::from("data")]);
    for (&(command, _), path) in registrations {
        if command == 0x2016 || path.is_empty() {
            continue;
        }
        let name = std::str::from_utf8(path)?.trim_end_matches(['/', '\\']);
        if let Some(rel) = resolved(&catalog.root, name.as_bytes())? {
            roots.insert(rel);
        }
    }
    for root in &roots {
        if !root.ancestors().skip(1).any(|p| roots.contains(p)) {
            collect_resources(&catalog.root, root.clone(), &saves.directories, &mut paths)?;
        }
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let s = name.to_string_lossy().to_lowercase();
        if entry.file_type()?.is_file()
            && (EXECUTABLE_NAMES.contains(&s.as_str())
                || s == catalog.profile.config_file_name()
                || matches!(
                    s.as_str(),
                    "key.cfg" | "initial.cfg" | "encoding.txt" | "conversion.json" | "font.ttf"
                )
                || s.ends_with(".ico")
                || s.ends_with(".dll"))
        {
            paths.insert(PathBuf::from(name));
        }
    }
    write_new_dir(out, |dir| {
        let mut table = BufWriter::new(File::create(dir.join("game.tsv"))?);
        writeln!(table, "cmvs-game-v1\t{}", catalog.profile.id)?;
        let mut id = 0;
        for rel in paths {
            let disk = source.join(&rel);
            let path = rel
                .iter()
                .map(|s| s.to_str().ok_or("game path is not UTF-8"))
                .collect::<std::result::Result<Vec<_>, _>>()?
                .join("/");
            relative(&path)?;
            let mut file = File::open(&disk)?;
            let index = if is_archive(&rel) {
                Some(
                    catalog
                        .startup
                        .open_relative(&mut file, &catalog.root, &rel)?,
                )
            } else {
                None
            };
            let format = index
                .as_ref()
                .map_or("loose", |i| archive::version_name(i.version()));
            if let Some(i) = &index {
                archive::check_family(i.version(), Some(catalog.profile))?;
            }
            let digest = to_hex(&sha256_reader(File::open(&disk)?)?);
            if rel == game.start_script && digest != start_digest {
                return Err("startup script changed during export".into());
            }
            writeln!(table, "file\t{path}\t{format}\t{digest}")?;
            let mut row =
                |ord: Option<usize>, dir: &[u8], name: &[u8], info: Description| -> Result<()> {
                    writeln!(
                        table,
                        "asset\t{id}\t{}\t{}\t{}\t{}\t{}\t-",
                        ord.map_or("-".to_owned(), |i| i.to_string()),
                        escape(dir, Encoding::ShiftJis),
                        escape(name, Encoding::ShiftJis),
                        info.codec,
                        escape(&info.reference, Encoding::ShiftJis)
                    )?;
                    id += 1;
                    Ok(())
                };
            if let Some(i) = index {
                for (n, e) in i.entries().iter().enumerate() {
                    let info = describe(
                        &e.name,
                        || Ok(i.read_entry_verified(&mut file, n)?),
                        catalog.profile,
                    )?;
                    row(Some(n), &e.directory, &e.name, info)?;
                }
            } else {
                let name = rel.file_name().unwrap().as_encoded_bytes();
                row(
                    None,
                    b"",
                    name,
                    describe(name, || read_bounded(&disk, MAX_PAYLOAD), catalog.profile)?,
                )?;
            }
        }
        Ok(table.flush()?)
    })
}

pub(super) fn load(
    source: &Path,
    project: &Path,
    selected: Option<&str>,
    verify_all: bool,
) -> Result<Catalog> {
    let mut lines = BufReader::new(File::open(project.join("game.tsv"))?).lines();
    let header = lines.next().ok_or("empty game manifest")??;
    let recorded = header
        .strip_prefix("cmvs-game-v1\t")
        .ok_or("invalid game manifest")?;
    let (mut cat, game, start_digest) = init(source, Some(selected.unwrap_or(recorded)))?;
    if cat.profile.id != recorded {
        return Err("project profile differs from selected profile".into());
    }
    let saves = verify_all.then(|| SavePaths::resolve(&cat)).transpose()?;
    let project = project.canonicalize()?;
    let mut seen = BTreeSet::new();
    for (line, row) in lines.enumerate() {
        let row = row?;
        let fields: Vec<_> = row.split('\t').collect();
        match fields.as_slice() {
            ["file", path, format, digest] => {
                let rel = relative(path)?;
                if rel == game.start_script && *digest != start_digest {
                    return Err("startup script differs from game manifest".into());
                }
                if !seen.insert(rel.to_owned()) {
                    return Err("repeated game path".into());
                }
                let disk = cat.root.join(rel).canonicalize()?;
                let canonical_rel = disk
                    .strip_prefix(&cat.root)
                    .map_err(|_| "source resolves outside the game directory")?;
                if saves
                    .as_ref()
                    .is_some_and(|s| s.excludes(rel) || s.excludes(canonical_rel))
                {
                    return Err("manifest includes a file in the gameplay save directory".into());
                }
                if is_archive(rel) == (*format == "loose") {
                    return Err("manifest archive format differs from source path".into());
                }
                let index = if *format == "loose" {
                    None
                } else {
                    let i = cat
                        .startup
                        .open_relative(&mut File::open(disk)?, &cat.root, rel)?;
                    archive::check_family(i.version(), Some(cat.profile))?;
                    if archive::version_name(i.version()) != *format {
                        return Err("source archive format differs".into());
                    }
                    Some(i)
                };
                let source_file = SourceFile {
                    digest: (*digest).to_owned(),
                    verified: OnceLock::new(),
                    relative: rel.into(),
                    index,
                    first: cat.assets.len(),
                };
                if verify_all {
                    source_file.verify(&cat.root)?;
                }
                cat.files.push(source_file);
            }
            ["asset", id, n, dir, name, codec, r, edit] => {
                let id = id.parse::<usize>()?;
                let f = cat.files.last().ok_or("missing file record")?;
                if id != cat.assets.len() {
                    return Err("game asset IDs must follow source order".into());
                }
                let ordinal = if *n == "-" {
                    None
                } else {
                    Some(n.parse::<usize>()?)
                };
                let (d, base) = match (&f.index, ordinal) {
                    (None, None) if id == f.first => (
                        &[][..],
                        f.relative
                            .file_name()
                            .ok_or("empty source path")?
                            .as_encoded_bytes(),
                    ),
                    (Some(index), Some(n)) if n == id - f.first => {
                        let e = index.entries().get(n).ok_or("entry outside source index")?;
                        (e.directory.as_ref(), e.name.as_slice())
                    }
                    _ => return Err("incorrect game entry ordinal".into()),
                };
                if unescape(name, Encoding::ShiftJis)? != base
                    || unescape(dir, Encoding::ShiftJis)? != d
                {
                    return Err(format!("asset {id} source name differs").into());
                }
                let edit = if !verify_all || *edit == "-" {
                    None
                } else {
                    Some(crate::manifest_input(&project, &project, relative(edit)?)?)
                };
                cat.assets.push(Asset {
                    file: cat.files.len() - 1,
                    ordinal,
                    codec: (*codec).into(),
                    reference: unescape(r, Encoding::ShiftJis)?,
                    pb3_bytes: 0,
                    edit,
                });
                if verify_all {
                    cat.assets[id].pb3_bytes = cat.validate_asset(id)?;
                }
            }
            _ => return Err(format!("invalid game manifest line {}", line + 2).into()),
        }
    }
    let mut end = 0;
    for f in &cat.files {
        if f.first != end {
            return Err("missing asset rows".into());
        }
        end += f.index.as_ref().map_or(1, |i| i.entries().len());
    }
    if end != cat.assets.len() {
        return Err("missing final asset rows".into());
    }
    if !seen.contains(&game.start_script) || !seen.contains(&game.script_archive) {
        return Err("manifest missing startup script or archive".into());
    }
    Ok(cat)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_copy_rejects_a_source_update_after_catalog_verification() {
        let temp = tempfile::tempdir().unwrap();
        let file = SourceFile {
            relative: "resource".into(),
            index: None,
            first: 0,
            digest: to_hex(&sha256(b"original")),
            verified: OnceLock::new(),
        };
        let source = temp.path().join(&file.relative);
        fs::write(&source, b"original").unwrap();
        file.verify(temp.path()).unwrap();
        fs::write(&source, b"updated").unwrap();
        let out = temp.path().join("out");
        assert!(
            write_new_dir(&out, |stage| file
                .copy_unchanged(&source, &stage.join(&file.relative)))
            .is_err()
        );
        assert!(!out.exists());
    }
}
