//! The game under test: identity from the profile layer and the capability set. Nothing
//! here names a game; differences between builds are capabilities.
use crate::json::Json;
use cmvs_core::{
    format::cpz::ArchiveIndex, format::cpz::ArchiveVersion, format::digest::sha256_reader,
    format::digest::to_hex, host::input::KeyBindings, profile::Evidence, profile::Game,
    profile::GameFiles, profile::Registry, runtime::snapshot_layout_supported,
};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    path::{Path, PathBuf},
};

/// Capability names. A check lists the ones it needs and is skipped when one is missing.
pub mod cap {
    /// Start script and script archive were found under their fixed names.
    pub const FILES: &str = "files";
    /// The profile layer selected an engine profile.
    pub const PROFILE: &str = "profile";
    /// The data matched a registered game record.
    pub const RECORD: &str = "record";
    /// The script archive opens with a supported decoder.
    pub const ARCHIVE: &str = "archive:readable";
    pub const CPZ3: &str = "archive:cpz3";
    pub const CPZ5: &str = "archive:cpz5";
    /// The successfully opened script archive uses CPZ6.
    pub const CPZ6: &str = "archive:cpz6";
    pub const CPZ7: &str = "archive:cpz7";
    /// The root `key.cfg` parses; keyboard input is then derived from it.
    pub const KEY_CONFIG: &str = "key-config";
    /// The game record verifies the startup auto-resume preview policy.
    pub const AUTO_RESUME_POLICY: &str = "auto-resume-policy";
    /// The profile has a verified gameplay snapshot layout; without it sessions continue
    /// without snapshot images and saving is unavailable.
    pub const SNAPSHOT_LAYOUT: &str = "snapshot-layout";
    /// A font file was supplied for glyph metrics.
    pub const FONT: &str = "font";
}

pub struct Target {
    pub root: PathBuf,
    pub files: Option<GameFiles>,
    pub game: Option<Game<'static>>,
    /// Why discovery or identification failed, when it did.
    pub identify_error: Option<String>,
    pub key_bindings: Option<KeyBindings>,
    pub capabilities: BTreeSet<&'static str>,
}

impl Target {
    /// Read-only: hashes and parses files under `root`, never writes there.
    pub fn open(root: &Path, explicit_profile: Option<&str>) -> Self {
        let mut capabilities = BTreeSet::new();
        let mut identify_error = None;
        let files = match GameFiles::discover(root) {
            Ok(files) => {
                capabilities.insert(cap::FILES);
                Some(files)
            }
            Err(e) => {
                identify_error = Some(e.to_string());
                None
            }
        };
        let game = match &files {
            Some(_) => match Registry::builtin().identify_game(root, explicit_profile) {
                Ok(game) => {
                    capabilities.insert(cap::PROFILE);
                    if snapshot_layout_supported(game.selection.profile) {
                        capabilities.insert(cap::SNAPSHOT_LAYOUT);
                    }
                    if let Some(record) = game.selection.record {
                        capabilities.insert(cap::RECORD);
                        if record.skip_auto_resume.is_some() {
                            capabilities.insert(cap::AUTO_RESUME_POLICY);
                        }
                    }
                    Some(game)
                }
                Err(e) => {
                    identify_error = Some(e.to_string());
                    None
                }
            },
            None => None,
        };
        if let Some(files) = &files
            && let Ok((_, index)) = open_profile_archive(
                root,
                files,
                &root.join(&files.script_archive),
                game.as_ref().map(|g| g.selection.profile),
            )
        {
            capabilities.insert(cap::ARCHIVE);
            match index.version() {
                ArchiveVersion::Cpz3 => {
                    capabilities.insert(cap::CPZ3);
                }
                ArchiveVersion::Cpz5 => {
                    capabilities.insert(cap::CPZ5);
                }
                ArchiveVersion::Cpz6 => {
                    capabilities.insert(cap::CPZ6);
                }
                ArchiveVersion::Cpz7 => {
                    capabilities.insert(cap::CPZ7);
                }
                _ => {}
            }
        }
        let key_bindings = fs::read_to_string(root.join("key.cfg"))
            .ok()
            .and_then(|text| KeyBindings::parse(&text).ok());
        if key_bindings.is_some() {
            capabilities.insert(cap::KEY_CONFIG);
        }
        Self {
            root: root.to_path_buf(),
            files,
            game,
            identify_error,
            key_bindings,
            capabilities,
        }
    }
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.contains(capability)
    }
    pub fn is_cvns(&self) -> bool {
        self.game
            .as_ref()
            .is_some_and(|g| g.selection.profile.is_cvns())
    }
    pub fn save_family(&self) -> cmvs_core::runtime::save_menu::SaveFamily {
        self.game
            .as_ref()
            .map_or(cmvs_core::runtime::save_menu::SaveFamily::Cmvs, |g| {
                g.selection.profile.save_family()
            })
    }
    pub fn system_save_name(&self) -> &'static str {
        self.game.as_ref().map_or("system.dat", |g| {
            g.selection.profile.system_save_file_name()
        })
    }
    pub fn config_file_name(&self) -> &'static str {
        self.game
            .as_ref()
            .map_or("cmvs.cfg", |g| g.selection.profile.config_file_name())
    }
    /// The first capability of `needed` that is missing, for `skipped` results.
    pub fn missing(&self, needed: &[&'static str]) -> Option<&'static str> {
        needed.iter().copied().find(|c| !self.has(c))
    }
    /// Identity for the report header: profile, record, basis and file hashes.
    pub fn identity_json(&self) -> Json {
        let mut fields = vec![
            ("root", Json::from(self.root.display().to_string())),
            (
                "capabilities",
                Json::arr(self.capabilities.iter().map(|&c| c.into())),
            ),
        ];
        if let Some(game) = &self.game {
            let selection = &game.selection;
            fields.push(("profile", selection.profile.id.as_str().into()));
            fields.push((
                "base",
                selection.profile.base.as_deref().map(str::to_owned).into(),
            ));
            fields.push(("basis", format!("{:?}", selection.basis).into()));
            fields.push(("record", selection.record.map(|r| r.game.clone()).into()));
        }
        if let Some(error) = &self.identify_error {
            fields.push(("identify_error", error.as_str().into()));
        }
        fields.push(("files", Json::arr(self.file_hashes())));
        Json::obj(fields)
    }
    fn file_hashes(&self) -> Vec<Json> {
        let mut paths: Vec<String> = Vec::new();
        if let Some(files) = &self.files {
            for p in [&files.start_script, &files.script_archive] {
                paths.push(p.to_string_lossy().replace('\\', "/"));
            }
        }
        if let Ok(executables) = Evidence::executables_in(&self.root) {
            paths.extend(executables.into_iter().map(|e| e.path));
        }
        paths.push("key.cfg".into());
        paths
            .into_iter()
            .filter_map(|p| {
                let digest = sha256_reader(File::open(self.root.join(&p)).ok()?).ok()?;
                Some(Json::obj([
                    ("path", Json::from(p)),
                    ("sha256", to_hex(&digest).into()),
                ]))
            })
            .collect()
    }
}

/// Shared by capability discovery and static checks, so keyed archives use the same policy.
pub fn open_archive(
    root: &Path,
    files: &GameFiles,
    path: &Path,
) -> Result<(File, ArchiveIndex), std::io::Error> {
    open_profile_archive(root, files, path, None)
}
pub(crate) fn open_profile_archive(
    root: &Path,
    files: &GameFiles,
    path: &Path,
    profile: Option<&cmvs_core::profile::EngineProfile>,
) -> Result<(File, ArchiveIndex), std::io::Error> {
    use std::io::{Read, Seek};
    let mut file = File::open(path)?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    file.rewind()?;
    if &magic != b"CPZ7" && !(profile.is_some() && &magic == b"CPZ5") {
        let index = ArchiveIndex::read(&mut file).map_err(std::io::Error::other)?;
        return Ok((file, index));
    }
    let mut raw = Vec::new();
    File::open(root.join(&files.start_script))?
        .take(cmvs_core::MAX_PAYLOAD as u64 + 49)
        .read_to_end(&mut raw)?;
    let script = cmvs_core::format::ps3::Script::decode(&raw).map_err(std::io::Error::other)?;
    let relative = path.strip_prefix(root).map_err(std::io::Error::other)?;
    let index = cmvs_core::runtime::script_control::archive_index_for_profile(
        &mut file,
        &script,
        &cmvs_core::host::filesystem::StdFs,
        root,
        relative,
        profile,
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok((file, index))
}
