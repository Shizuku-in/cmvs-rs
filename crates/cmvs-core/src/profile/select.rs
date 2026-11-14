//! Profile selection from read-only evidence, with known-binary fallback for modified data.
use super::{EngineProfile, GameRecord, Registry};
use crate::digest::{sha256_reader, to_hex};
use crate::filesystem as fs;
use crate::filesystem::File;
use std::{
    fmt, io,
    path::{Path, PathBuf},
};

/// Executable names the CMVS and CVNS installers place in the game root.
pub const EXECUTABLE_NAMES: [&str; 4] = ["cmvs.exe", "cmvs32.exe", "cmvs64.exe", "cvns.exe"];
const START_SCRIPTS: [&str; 3] = [
    "data/pack/start.ps3",
    "data/pack/start.ps2",
    "data/pack/start.pss",
];
const SCRIPT_ARCHIVE: &str = "data/pack/script.cpz";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEvidence {
    /// Display label, normally the path relative to the game root.
    pub path: String,
    pub sha256: [u8; 32],
}

impl FileEvidence {
    pub fn read(path: &Path, label: impl Into<String>) -> io::Result<Self> {
        Ok(Self {
            path: label.into(),
            sha256: sha256_reader(File::open(path)?)?,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evidence {
    pub start_script: Option<FileEvidence>,
    pub script_archive: Option<FileEvidence>,
    pub executables: Vec<FileEvidence>,
}

impl Evidence {
    /// Hashes whichever known executables exist directly in `root`.
    pub fn executables_in(root: &Path) -> io::Result<Vec<FileEvidence>> {
        EXECUTABLE_NAMES
            .iter()
            .filter(|name| fs::is_file(root.join(name)))
            .map(|name| FileEvidence::read(&root.join(name), *name))
            .collect()
    }
}

/// Game files relative to the root, found by fixed names rather than by searching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameFiles {
    pub start_script: PathBuf,
    pub script_archive: PathBuf,
}

impl GameFiles {
    pub fn discover(root: &Path) -> Result<Self, SelectionError> {
        let found: Vec<_> = START_SCRIPTS
            .iter()
            .filter(|p| fs::is_file(root.join(p)))
            .collect();
        let start = match found.as_slice() {
            [one] => PathBuf::from(one),
            [] => return Err(SelectionError::MissingFile(START_SCRIPTS.join(" or "))),
            _ => return Err(SelectionError::AmbiguousStartScript),
        };
        if !fs::is_file(root.join(SCRIPT_ARCHIVE)) {
            return Err(SelectionError::MissingFile(SCRIPT_ARCHIVE.into()));
        }
        Ok(Self {
            start_script: start,
            script_archive: PathBuf::from(SCRIPT_ARCHIVE),
        })
    }
    /// Basename recorded as the current script, as the original names it.
    pub fn start_name(&self) -> Vec<u8> {
        self.start_script
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("start.ps3")
            .as_bytes()
            .to_vec()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionBasis {
    /// `--profile`; any record match is only reported.
    Explicit,
    /// Data hashes matched a record; no known executable was present.
    Data,
    /// Data hashes matched and every present executable is a binary of the profile.
    DataAndBinary,
    /// Modified data; every present executable uniquely identifies one game record.
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSelection<'a> {
    pub profile: &'a EngineProfile,
    /// The record identified by data or uniquely matching known executables.
    pub record: Option<&'a GameRecord>,
    pub basis: SelectionBasis,
    pub evidence: Vec<String>,
}

/// A selected profile together with the files it was identified from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game<'a> {
    pub files: GameFiles,
    pub selection: ProfileSelection<'a>,
}

#[derive(Debug)]
pub enum SelectionError {
    Io(io::Error),
    MissingFile(String),
    AmbiguousStartScript,
    UnknownProfile(String),
    /// Neither a start script nor a script archive was supplied.
    NoDataEvidence,
    UnknownGame(Vec<String>),
    UnknownBinary {
        game: String,
        file: String,
        sha256: [u8; 32],
    },
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "profile evidence: {e}"),
            Self::MissingFile(name) => write!(f, "profile evidence: missing {name}"),
            Self::AmbiguousStartScript => {
                f.write_str("profile evidence: more than one of start.ps3/ps2/pss exists")
            }
            Self::UnknownProfile(id) => write!(f, "unknown engine profile {id}"),
            Self::NoDataEvidence => f.write_str(
                "no start script or script archive to identify the game; pass --profile",
            ),
            Self::UnknownGame(evidence) => write!(
                f,
                "game data matches no recorded game ({}); run cmvs-profile identify, or pass --profile for research",
                evidence.join("; ")
            ),
            Self::UnknownBinary { game, file, sha256 } => write!(
                f,
                "{file} sha256={} is not a recorded executable of {game} (unregistered patch?); pass --profile for research",
                to_hex(sha256)
            ),
        }
    }
}

impl std::error::Error for SelectionError {}

impl From<io::Error> for SelectionError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

fn describe(kind: &str, file: &FileEvidence) -> String {
    format!("{kind} {} sha256={}", file.path, to_hex(&file.sha256))
}

impl Registry {
    pub fn select(
        &self,
        explicit: Option<&str>,
        evidence: &Evidence,
    ) -> Result<ProfileSelection<'_>, SelectionError> {
        let mut notes = Vec::new();
        notes.extend(evidence.start_script.iter().map(|f| describe("start", f)));
        notes.extend(
            evidence
                .script_archive
                .iter()
                .map(|f| describe("archive", f)),
        );
        notes.extend(
            evidence
                .executables
                .iter()
                .map(|f| describe("executable", f)),
        );
        let data =
            if evidence.start_script.is_none() && evidence.script_archive.is_none() {
                None
            } else {
                self.games.iter().find(|g| {
                    evidence.start_script.as_ref().is_none_or(|f| {
                        g.start_scripts.iter().any(|start| f.sha256 == start.sha256)
                    }) && evidence
                        .script_archive
                        .as_ref()
                        .is_none_or(|f| f.sha256 == g.script_archive.sha256)
                })
            };
        if let Some(game) = data {
            notes.push(format!(
                "data matches game {} ({})",
                game.game, game.profile
            ));
        }
        if let Some(id) = explicit {
            let profile = self
                .profile(id)
                .ok_or_else(|| SelectionError::UnknownProfile(id.to_owned()))?;
            let record = data.filter(|g| g.profile == profile.id);
            if data.is_some() && record.is_none() {
                notes.push("explicit profile differs; game record not applied".into());
            }
            return Ok(ProfileSelection {
                profile,
                record,
                basis: SelectionBasis::Explicit,
                evidence: notes,
            });
        }
        if evidence.start_script.is_none() && evidence.script_archive.is_none() {
            return Err(SelectionError::NoDataEvidence);
        }
        let game = match data {
            Some(game) => game,
            None => {
                let mut candidates = self.games.iter().filter(|game| {
                    !evidence.executables.is_empty()
                        && evidence.executables.iter().all(|exe| {
                            game.executables
                                .iter()
                                .any(|known| known.sha256 == exe.sha256)
                        })
                });
                let game = candidates.next().filter(|_| candidates.next().is_none());
                let game = game.ok_or_else(|| SelectionError::UnknownGame(notes.clone()))?;
                // Modified hashes are expected; hashes identifying another game are not.
                let conflict = self.games.iter().any(|other| {
                    other.game != game.game
                        && (evidence.start_script.as_ref().is_some_and(|file| {
                            other
                                .start_scripts
                                .iter()
                                .any(|known| known.sha256 == file.sha256)
                        }) || evidence
                            .script_archive
                            .as_ref()
                            .is_some_and(|file| other.script_archive.sha256 == file.sha256))
                });
                if conflict {
                    return Err(SelectionError::UnknownGame(notes));
                }
                notes.push(format!(
                    "modified data; executables uniquely match game {} ({})",
                    game.game, game.profile
                ));
                game
            }
        };
        for exe in &evidence.executables {
            if !game.executables.iter().any(|e| e.sha256 == exe.sha256) {
                return Err(SelectionError::UnknownBinary {
                    game: game.game.clone(),
                    file: exe.path.clone(),
                    sha256: exe.sha256,
                });
            }
        }
        Ok(ProfileSelection {
            profile: self.profile(&game.profile).expect("validated record"),
            record: Some(game),
            basis: if data.is_none() {
                SelectionBasis::Binary
            } else if evidence.executables.is_empty() {
                SelectionBasis::Data
            } else {
                SelectionBasis::DataAndBinary
            },
            evidence: notes,
        })
    }
    /// Identify a game directory using startup-script, archive, and executable hashes.
    ///
    /// `explicit` optionally names an engine profile instead of automatic selection.
    /// Reads files without executing or modifying them. Returns an error for missing
    /// or ambiguous files, inconsistent evidence, an unknown profile, or I/O failures.
    pub fn identify_game(
        &self,
        root: &Path,
        explicit: Option<&str>,
    ) -> Result<Game<'_>, SelectionError> {
        let files = GameFiles::discover(root)?;
        let label = |p: &Path| p.to_string_lossy().replace('\\', "/");
        let evidence = Evidence {
            start_script: Some(FileEvidence::read(
                &root.join(&files.start_script),
                label(&files.start_script),
            )?),
            script_archive: Some(FileEvidence::read(
                &root.join(&files.script_archive),
                label(&files.script_archive),
            )?),
            executables: Evidence::executables_in(root)?,
        };
        Ok(Game {
            selection: self.select(explicit, &evidence)?,
            files,
        })
    }
}

impl ProfileSelection<'_> {
    /// `#` comment lines describing the selection, for logs and replay headers.
    pub fn log_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "# engine profile {} basis={:?} record={}",
            self.profile.id,
            self.basis,
            self.record.map_or("-", |r| r.game.as_str())
        )];
        lines.extend(
            self.evidence
                .iter()
                .map(|e| format!("# profile evidence: {e}")),
        );
        lines
    }
}
