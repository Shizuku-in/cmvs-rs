//! Offline startup execution and shared lookup of live archive registrations.
use super::LoadError;
use crate::{
    bootstrap::Bootstrap,
    cpz::{ArchiveIndex, ArchiveVersion, Cpz5Mode, Cpz7Parameters},
    filesystem::{HostFs, resolve_under_with_fs},
    profile::{CommandGate, EngineProfile, ProfileSelection, SelectionBasis},
    ps3::Script,
};
use std::{
    io::{Read, Seek},
    path::Path,
};

/// Registrations in effect at the first host-dependent command in an offline build.
pub struct Startup {
    pub bootstrap: Bootstrap,
    pub pc: usize,
    pub stopped: String,
}

impl Startup {
    pub fn execute(script: &Script, profile: &EngineProfile) -> Result<Self, LoadError> {
        use crate::{
            dispatch::{self, Capability, Flow, FunctionKeys, Hosts},
            machine::{Machine, Step},
        };
        let mut vm = Machine::new(script)?;
        vm.initialize_profile(profile)?;
        let mut gate = CommandGate::new(ProfileSelection {
            profile,
            record: None,
            basis: SelectionBasis::Explicit,
            evidence: Vec::new(),
        });
        let mut bootstrap = Bootstrap::default();
        let mut messages = crate::message::MessageHost::for_profile(profile);
        let mut input = crate::input::ActionInput::default();
        let mut audio = crate::audio::AudioHost::for_profile(profile, b"");
        let mut backend = crate::audio::RecordingAudio::default();
        let mut stopped = String::from("startup instruction budget exhausted");
        for _ in 0..100_000 {
            if vm.pc() == script.code().len() {
                stopped = "end of startup".into();
                break;
            }
            let Step::System { pc, command } = vm.step()? else {
                continue;
            };
            let resolved = match gate.resolve(&vm, pc, command) {
                Ok(resolved) => resolved,
                Err(error) => {
                    return Ok(Self {
                        bootstrap,
                        pc,
                        stopped: error.to_string(),
                    });
                }
            };
            let Some(entry) = dispatch::entry(resolved.semantics, command).filter(|e| {
                e.uses_bootstrap()
                    || matches!(e.capability, Capability::Math | Capability::Strings)
                    || matches!(
                        (command, e.capability),
                        (0x3ac, Capability::Text) | (0x4ac, Capability::Audio)
                    )
            }) else {
                stopped = format!("external command {:#06x}", command + 0x2000);
                return Ok(Self {
                    bootstrap,
                    pc,
                    stopped,
                });
            };
            let mut hosts = Hosts {
                math: true,
                strings: true,
                text: Some(dispatch::TextHosts {
                    messages: &mut messages,
                    input: &mut input,
                    flags: Default::default(),
                }),
                audio: Some(dispatch::AudioHosts {
                    audio: &mut audio,
                    backend: &mut backend,
                }),
                ..Default::default()
            };
            if entry.capability == Capability::FunctionKeys {
                hosts.function_keys = Some(FunctionKeys::Runtime(&mut bootstrap));
            } else {
                hosts.bootstrap = Some(&mut bootstrap);
            }
            if entry.run(&mut vm, &mut hosts)? != Some(Flow::Done) {
                stopped = format!("pending command {:#06x}", command + 0x2000);
                break;
            }
        }
        Ok(Self {
            bootstrap,
            pc: vm.pc(),
            stopped,
        })
    }
    pub fn open(
        &self,
        reader: &mut (impl Read + Seek),
        basename: &[u8],
    ) -> Result<ArchiveIndex, LoadError> {
        self.open_target(reader, ArchiveTarget::Basename(basename))
    }
    /// Resolve CPZ7 registrations under the canonical game root used by the resource loader.
    #[cfg(feature = "std-fs")]
    pub fn open_relative(
        &self,
        reader: &mut (impl Read + Seek),
        root: &Path,
        relative: &Path,
    ) -> Result<ArchiveIndex, LoadError> {
        self.open_target(
            reader,
            ArchiveTarget::game(&crate::filesystem::StdFs, root, relative),
        )
    }
    // Encoding and auto-resume inspection may run before the live VM sets the CPZ5 mode.
    pub(super) fn inspect(
        &self,
        reader: &mut (impl Read + Seek),
        target: ArchiveTarget<'_>,
    ) -> Result<ArchiveIndex, LoadError> {
        if !self.bootstrap.settings().contains_key(&0x201f)
            && crate::cpz::read_version(reader)? == ArchiveVersion::Cpz5
        {
            return Ok(ArchiveIndex::read(reader)?);
        }
        self.open_target(reader, target)
    }
    fn open_target(
        &self,
        reader: &mut (impl Read + Seek),
        target: ArchiveTarget<'_>,
    ) -> Result<ArchiveIndex, LoadError> {
        let version = crate::cpz::read_version(reader)?;
        let settings = archive_settings(&self.bootstrap, version, target)
            .map_err(|e| format!("startup PC {:#x} ({}), {e}", self.pc, self.stopped))?;
        settings.open(reader)
    }
}

#[derive(Clone, Copy)]
pub(super) enum ArchiveTarget<'a> {
    Game {
        fs: &'a dyn HostFs,
        root: &'a Path,
        relative: &'a Path,
    },
    Basename(&'a [u8]),
}

impl<'a> ArchiveTarget<'a> {
    pub(super) fn game(fs: &'a dyn HostFs, root: &'a Path, relative: &'a Path) -> Self {
        Self::Game { fs, root, relative }
    }
    pub(super) fn matches(self, registration: &[u8]) -> bool {
        match self {
            Self::Game { fs, root, relative } => resolve_under_with_fs(fs, root, registration)
                .is_ok_and(|registered| {
                    fs.canonicalize(&root.join(relative))
                        .is_ok_and(|archive| archive == registered)
                }),
            Self::Basename(name) => registration
                .rsplit(|b| matches!(b, b'/' | b'\\'))
                .next()
                .is_some_and(|s| s.eq_ignore_ascii_case(name)),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum ArchiveSettings {
    Auto,
    Five(Cpz5Mode),
    Seven(Cpz7Parameters),
}
impl ArchiveSettings {
    pub(super) fn open(self, r: &mut (impl Read + Seek)) -> Result<ArchiveIndex, LoadError> {
        Ok(match self {
            Self::Auto => ArchiveIndex::read(r)?,
            Self::Five(mode) => ArchiveIndex::read_with_cpz5_mode(r, mode)?,
            Self::Seven(keys) => ArchiveIndex::read_with_cpz7_parameters(r, keys)?,
        })
    }
}
pub(super) fn archive_settings(
    state: &Bootstrap,
    version: ArchiveVersion,
    target: ArchiveTarget<'_>,
) -> Result<ArchiveSettings, crate::FormatError> {
    use crate::FormatError;
    let mode = state.settings().get(&0x201f).copied();
    match version {
        ArchiveVersion::Cpz5 => Ok(ArchiveSettings::Five(Cpz5Mode::try_from(
            mode.ok_or(FormatError("missing CPZ5 packing mode"))?,
        )?)),
        ArchiveVersion::Cpz7 if mode == Some(12) => {
            Ok(ArchiveSettings::Seven(Cpz7Parameters::default()))
        }
        ArchiveVersion::Cpz7 if mode == Some(13) => {
            let mut found = None;
            for (slot, keys) in state.archive_parameters() {
                if state.paths().get(slot).is_some_and(|p| target.matches(p)) {
                    if let Some(previous) = found
                        && previous != *keys
                    {
                        return Err(FormatError("conflicting CPZ7 archive registrations"));
                    }
                    found = Some(*keys);
                }
            }
            Ok(ArchiveSettings::Seven(found.ok_or(FormatError(
                "missing CPZ7 per-archive parameters",
            ))?))
        }
        ArchiveVersion::Cpz7 => Err(FormatError("missing or unsupported CPZ7 packing mode")),
        _ => Ok(ArchiveSettings::Auto),
    }
}
