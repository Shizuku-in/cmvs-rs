//! Script loading, cross-script calls, and callback scheduling.
//!
//! [`ScriptControl`] manages script registrations and scheduled callback entry points.
//! [`ArchiveScripts`] supplies scripts from the registered archive. The runtime
//! coordinates these operations through [`crate::runtime::Runtime`]; rendering and
//! save-snapshot orchestration remain separate.
use crate::{
    MAX_PAYLOAD,
    bootstrap::Bootstrap,
    cpz::{ArchiveIndex, ArchiveVersion, Cpz7Parameters},
    dispatch::Call,
    evaluate::error,
    filesystem::{File, HostFs, resolve_under_with_fs},
    profile::EngineProfile,
    ps3::Script,
};
use std::{
    collections::BTreeMap,
    error::Error,
    io::{Read, Seek},
    path::{Path, PathBuf},
    sync::Arc,
};

mod startup;
pub use startup::Startup;
use startup::{ArchiveSettings, ArchiveTarget, archive_settings, slot_archive_settings};

pub type LoadError = Box<dyn Error + Send + Sync>;

/// Open an archive using its literal startup registration when CPZ7 requires keys.
/// Other archive versions keep their existing detection policy.
pub fn archive_index_from_start<R: Read + Seek>(
    reader: &mut R,
    script: &Script,
    basename: &[u8],
) -> Result<ArchiveIndex, LoadError> {
    literal_archive_index(reader, script, ArchiveTarget::Basename(basename))
}

fn literal_archive_index(
    reader: &mut (impl Read + Seek),
    script: &Script,
    target: ArchiveTarget<'_>,
) -> Result<ArchiveIndex, LoadError> {
    if crate::cpz::read_version(reader)? == ArchiveVersion::Cpz7 {
        let parameters = literal_parameters(script, target)?.ok_or(crate::FormatError(
            "archive has no literal CPZ7 startup registration",
        ))?;
        Ok(ArchiveIndex::read_with_cpz7_parameters(reader, parameters)?)
    } else {
        Ok(ArchiveIndex::read(reader)?)
    }
}

/// Use executed settings for profiled CPZ5/7 inputs; standalone inspection can still
/// read older formats and literal registrations without choosing an engine profile.
pub fn archive_index_for_profile(
    reader: &mut (impl Read + Seek),
    script: &Script,
    fs: &dyn HostFs,
    root: &Path,
    relative: &Path,
    profile: Option<&EngineProfile>,
) -> Result<ArchiveIndex, LoadError> {
    let root = fs.canonicalize(root)?;
    let target = ArchiveTarget::game(fs, &root, relative);
    if let Some(p) = profile
        && matches!(
            crate::cpz::read_version(reader)?,
            ArchiveVersion::Cpz5 | ArchiveVersion::Cpz7
        )
    {
        return Startup::execute(script, p)?.inspect(reader, target);
    }
    literal_archive_index(reader, script, target)
}

/// Read the literal 0x201f packing-mode setting from the startup prefix.
pub fn packing_mode_from_start(script: &Script) -> Result<u32, crate::FormatError> {
    use crate::{
        FormatError,
        bytecode::{self, InstructionKind, Term},
    };
    let instructions =
        bytecode::decode_layout(script.code(), script.header().entry_pc, &script.labels())
            .map_err(|_| FormatError("invalid startup script layout"))?;
    let mut value = None;
    let mut pushed = None;
    for instruction in instructions {
        if instruction.pc < script.header().entry_pc as usize {
            continue;
        }
        match instruction.kind {
            InstructionKind::Expression(expression) if expression.kind == 0x200 => {
                value = match expression.terms.as_slice() {
                    [
                        Term::Scalar {
                            tag: 0x100, raw, ..
                        },
                    ] => Some(*raw),
                    _ => None,
                };
            }
            InstructionKind::Simple if instruction.opcode == 0x430 => pushed = value.take(),
            InstructionKind::System { command: 0x1f } => {
                return pushed.ok_or(FormatError("dynamic startup archive mode"));
            }
            InstructionKind::Jump { .. }
            | InstructionKind::ConditionalJump { .. }
            | InstructionKind::CompareJump { .. }
            | InstructionKind::LongJump { .. }
            | InstructionKind::ScriptReturn { .. } => break,
            _ => {
                value = None;
                pushed = None;
            }
        }
    }
    Err(FormatError("startup has no literal packing-mode setting"))
}

/// Inspect literal archive registrations in the straight-line startup prefix.
/// REALIVE's CPZ7 parameters are per archive, not a game-wide MD5 replacement.
/// Dynamic expressions and control flow are not evaluated by this inspection helper.
pub fn cpz7_parameters_from_start(
    script: &Script,
    basename: &[u8],
) -> Result<Option<Cpz7Parameters>, crate::FormatError> {
    literal_parameters(script, ArchiveTarget::Basename(basename))
}

fn literal_parameters(
    script: &Script,
    target: ArchiveTarget<'_>,
) -> Result<Option<Cpz7Parameters>, crate::FormatError> {
    use crate::{
        FormatError,
        bytecode::{self, InstructionKind, Term},
    };
    let instructions =
        bytecode::decode_layout(script.code(), script.header().entry_pc, &script.labels())
            .map_err(|_| FormatError("invalid startup script layout"))?;
    let strings = script.strings()?;
    let literal = |at: usize, kind: u16, tag: u16| -> Option<u32> {
        let expr = instructions.get(at)?;
        if expr.pc < script.header().entry_pc as usize {
            return None;
        }
        if instructions.get(at + 1)?.opcode != 0x430 {
            return None;
        }
        let InstructionKind::Expression(expression) = &expr.kind else {
            return None;
        };
        if expression.kind != kind {
            return None;
        }
        match expression.terms.as_slice() {
            [
                Term::Scalar {
                    tag: actual, raw, ..
                },
            ] if *actual == tag => Some(*raw),
            _ => None,
        }
    };
    let mut pack_mode = None;
    let mut found = None;
    for (i, instruction) in instructions.iter().enumerate() {
        if instruction.pc < script.header().entry_pc as usize {
            continue;
        }
        match &instruction.kind {
            InstructionKind::Jump { .. }
            | InstructionKind::ConditionalJump { .. }
            | InstructionKind::CompareJump { .. }
            | InstructionKind::LongJump { .. }
            | InstructionKind::ScriptReturn { .. } => break,
            InstructionKind::System { command: 0x1f } => {
                pack_mode = Some(
                    i.checked_sub(2)
                        .and_then(|at| literal(at, 0x200, 0x100))
                        .ok_or(FormatError("dynamic startup archive mode"))?,
                );
            }
            InstructionKind::System {
                command: 0x268..=0x26e,
            } if pack_mode == Some(13) => {
                let Some(at) = i.checked_sub(12) else {
                    return Err(FormatError("incomplete CPZ7 registration"));
                };
                let path_offset = literal(at + 8, 0x201, 0x120)
                    .ok_or(FormatError("dynamic CPZ7 archive path"))?;
                let path = strings
                    .iter()
                    .find(|entry| entry.offset == path_offset as usize)
                    .ok_or(FormatError("invalid CPZ7 archive path reference"))?
                    .bytes;
                if !target.matches(path) {
                    continue;
                }
                if literal(at + 10, 0x200, 0x100).is_none() {
                    return Err(FormatError("dynamic CPZ7 archive slot"));
                }
                let mut keys = [0; 4];
                for (n, key) in keys.iter_mut().enumerate() {
                    *key = literal(at + 6 - n * 2, 0x200, 0x100)
                        .ok_or(FormatError("dynamic CPZ7 archive key"))?;
                }
                let parameters = Cpz7Parameters::from(keys);
                if found.is_some_and(|previous| previous != parameters) {
                    return Err(FormatError("conflicting CPZ7 archive registrations"));
                }
                found = Some(parameters);
            }
            _ => {}
        }
    }
    match pack_mode {
        Some(12) => Ok(Some(Cpz7Parameters::default())),
        Some(13) => Ok(found),
        _ => Err(FormatError("startup does not select a supported CPZ7 mode")),
    }
}

pub trait ScriptSource {
    fn load(&mut self, name: &[u8]) -> Result<Script, LoadError>;
}

enum ScriptIndex {
    Open(Box<ArchiveIndex>),
    Registered {
        version: ArchiveVersion,
        relative: PathBuf,
        settings: Result<ArchiveSettings, crate::FormatError>,
    },
}
pub struct ArchiveScripts<R> {
    reader: R,
    index: ScriptIndex,
    lookup_pc: usize,
}
impl<R: Read + Seek> ArchiveScripts<R> {
    fn opened(reader: R, index: ArchiveIndex) -> Self {
        Self {
            reader,
            index: ScriptIndex::Open(Box::new(index)),
            lookup_pc: 0,
        }
    }
    pub fn from_start(mut reader: R, script: &Script, basename: &[u8]) -> Result<Self, LoadError> {
        let index = archive_index_from_start(&mut reader, script, basename)?;
        Ok(Self::opened(reader, index))
    }
    pub fn new(mut reader: R) -> Result<Self, LoadError> {
        let index = ArchiveIndex::read(&mut reader)?;
        Ok(Self::opened(reader, index))
    }
    pub fn with_cpz7_parameters(
        mut reader: R,
        parameters: Cpz7Parameters,
    ) -> Result<Self, LoadError> {
        let index = ArchiveIndex::read_with_cpz7_parameters(&mut reader, parameters)?;
        Ok(Self::opened(reader, index))
    }
    pub(crate) fn deferred(
        mut reader: R,
        fs: &dyn HostFs,
        root: &Path,
        relative: &Path,
    ) -> Result<Self, LoadError> {
        let v = crate::cpz::read_version(&mut reader)?;
        if !matches!(v, ArchiveVersion::Cpz5 | ArchiveVersion::Cpz7) {
            return Self::new(reader);
        }
        Ok(Self {
            reader,
            index: ScriptIndex::Registered {
                version: v,
                relative: relative.to_owned(),
                settings: archive_settings(
                    &Bootstrap::default(),
                    v,
                    ArchiveTarget::game(fs, root, relative),
                ),
            },
            lookup_pc: 0,
        })
    }
    pub(crate) fn configure(&mut self, state: &Bootstrap, fs: &dyn HostFs, root: &Path) {
        if let ScriptIndex::Registered {
            version,
            relative,
            settings,
        } = &mut self.index
        {
            *settings = archive_settings(state, *version, ArchiveTarget::game(fs, root, relative));
        }
    }
    pub(crate) fn set_lookup_pc(&mut self, pc: usize) {
        self.lookup_pc = pc;
    }
    // Early auto-resume detection must not fix the index used by subsequent live commands.
    pub(crate) fn preview(
        &mut self,
        name: &[u8],
        fs: &dyn HostFs,
        root: &Path,
        script: Option<&Script>,
        profile: &EngineProfile,
    ) -> Result<Script, LoadError> {
        if let ScriptIndex::Registered { relative, .. } = &self.index {
            let startup = Startup::execute(script.ok_or("missing startup script")?, profile)?;
            let index =
                startup.inspect(&mut self.reader, ArchiveTarget::game(fs, root, relative))?;
            return Ok(Script::decode(
                &index.read_entry(&mut self.reader, index.find(name)?)?,
            )?);
        }
        self.load(name)
    }
}
impl<R: Read + Seek> ScriptSource for ArchiveScripts<R> {
    fn load(&mut self, name: &[u8]) -> Result<Script, LoadError> {
        if let ScriptIndex::Registered {
            relative, settings, ..
        } = &self.index
        {
            let settings = settings.as_ref().map_err(|e| {
                format!(
                    "script lookup at PC {:#x} ({}): {e}",
                    self.lookup_pc,
                    relative.display()
                )
            })?;
            self.index = ScriptIndex::Open(Box::new(settings.open(&mut self.reader)?));
        }
        let ScriptIndex::Open(index) = &self.index else {
            unreachable!("archive opened above")
        };
        let entry = index.find(name)?;
        Ok(Script::decode(&index.read_entry(&mut self.reader, entry)?)?)
    }
}

const SCRIPT_SLOTS: u32 = 5;

struct OpenScriptArchive {
    file: File,
    index: Box<ArchiveIndex>,
}

/// Runtime script lookup across `0x2010`/`0x2268` slots in priority order, with fallback
/// to the discovered `script.cpz` when startup has not registered any script slot.
pub(crate) struct RegisteredScripts {
    fs: Arc<dyn HostFs>,
    root: PathBuf,
    fallback: ArchiveScripts<File>,
    paths: [Option<Vec<u8>>; SCRIPT_SLOTS as usize],
    parameters: [Option<Cpz7Parameters>; SCRIPT_SLOTS as usize],
    packing_mode: Option<u32>,
    open: [Option<OpenScriptArchive>; SCRIPT_SLOTS as usize],
    lookup_pc: usize,
}

impl RegisteredScripts {
    pub(crate) fn new(
        fs: Arc<dyn HostFs>,
        root: PathBuf,
        relative: &Path,
    ) -> Result<Self, LoadError> {
        let fallback =
            ArchiveScripts::deferred(fs.open_read(&root.join(relative))?, &*fs, &root, relative)?;
        Ok(Self {
            fs,
            root,
            fallback,
            paths: std::array::from_fn(|_| None),
            parameters: [None; SCRIPT_SLOTS as usize],
            packing_mode: None,
            open: std::array::from_fn(|_| None),
            lookup_pc: 0,
        })
    }

    pub(crate) fn configure(&mut self, state: &Bootstrap) {
        self.fallback.configure(state, &*self.fs, &self.root);
        let new_mode = state.settings().get(&0x201f).copied();
        let mode_changed = self.packing_mode != new_mode;
        self.packing_mode = new_mode;
        for slot in 0..SCRIPT_SLOTS {
            let index = slot as usize;
            let new_path = state
                .paths()
                .get(&(0x2010, slot))
                .filter(|path| !path.is_empty())
                .cloned();
            let new_params = state.archive_parameters().get(&(0x2010, slot)).copied();
            if mode_changed || self.paths[index] != new_path || self.parameters[index] != new_params
            {
                self.open[index] = None;
            }
            self.paths[index] = new_path;
            self.parameters[index] = new_params;
        }
    }

    pub(crate) fn set_lookup_pc(&mut self, pc: usize) {
        self.lookup_pc = pc;
        self.fallback.set_lookup_pc(pc);
    }

    // Early auto-resume detection must not fix the live indices used by later commands.
    pub(crate) fn preview(
        &mut self,
        name: &[u8],
        script: Option<&Script>,
        profile: &EngineProfile,
    ) -> Result<Script, LoadError> {
        if let Some(start) = script {
            let startup = Startup::execute(start, profile)?;
            let mode = startup.bootstrap.settings().get(&0x201f).copied();
            let mut any_registered = false;
            for slot in 0..SCRIPT_SLOTS {
                let Some(path) = startup
                    .bootstrap
                    .paths()
                    .get(&(0x2010, slot))
                    .filter(|path| !path.is_empty())
                else {
                    continue;
                };
                any_registered = true;
                if path.ends_with(b"\\") {
                    if let Some(found) = read_directory_script(&*self.fs, &self.root, path, name)? {
                        return Ok(found);
                    }
                    continue;
                }
                let resolved = match resolve_under_with_fs(&*self.fs, &self.root, path) {
                    Ok(resolved) => resolved,
                    Err(err) if err.is_not_found() => continue,
                    Err(err) => return Err(err.into()),
                };
                let mut file = self.fs.open_read(&resolved)?;
                let version = crate::cpz::read_version(&mut file)?;
                let settings = slot_archive_settings(
                    version,
                    mode,
                    startup
                        .bootstrap
                        .archive_parameters()
                        .get(&(0x2010, slot))
                        .copied(),
                    true,
                )
                .map_err(|e| format!("startup PC {:#x} ({}), {e}", startup.pc, startup.stopped))?;
                let index = settings.open(&mut file)?;
                if let Some(entry) = index.lookup(name)? {
                    return Ok(Script::decode(&index.read_entry(&mut file, entry)?)?);
                }
            }
            if any_registered {
                return Err(format!(
                    "script preview: {} not found in registered script archives",
                    String::from_utf8_lossy(name)
                )
                .into());
            }
        }
        self.fallback
            .preview(name, &*self.fs, &self.root, script, profile)
    }
}

impl ScriptSource for RegisteredScripts {
    fn load(&mut self, name: &[u8]) -> Result<Script, LoadError> {
        if self.paths.iter().all(Option::is_none) {
            return self.fallback.load(name);
        }
        for slot in 0..SCRIPT_SLOTS as usize {
            let Some(path) = self.paths[slot].as_deref() else {
                continue;
            };
            if path.ends_with(b"\\") {
                if let Some(script) = read_directory_script(&*self.fs, &self.root, path, name)? {
                    return Ok(script);
                }
                continue;
            }
            if self.open[slot].is_none() {
                let resolved = match resolve_under_with_fs(&*self.fs, &self.root, path) {
                    Ok(resolved) => resolved,
                    Err(err) if err.is_not_found() => continue,
                    Err(err) => return Err(err.into()),
                };
                let mut file = self.fs.open_read(&resolved)?;
                let version = crate::cpz::read_version(&mut file)?;
                let settings =
                    slot_archive_settings(version, self.packing_mode, self.parameters[slot], false)
                        .map_err(|e| {
                            format!(
                                "script lookup at PC {:#x} ({}): {e}",
                                self.lookup_pc,
                                String::from_utf8_lossy(path)
                            )
                        })?;
                let index = settings.open(&mut file)?;
                self.open[slot] = Some(OpenScriptArchive {
                    file,
                    index: Box::new(index),
                });
            }
            let open = self.open[slot].as_mut().expect("opened above");
            if let Some(entry) = open.index.lookup(name)? {
                let bytes = open.index.read_entry(&mut open.file, entry)?;
                return Ok(Script::decode(&bytes)?);
            }
        }
        Err(format!(
            "script lookup at PC {:#x}: {} not found in registered script archives",
            self.lookup_pc,
            String::from_utf8_lossy(name)
        )
        .into())
    }
}

fn read_directory_script(
    fs: &dyn HostFs,
    root: &Path,
    dir: &[u8],
    name: &[u8],
) -> Result<Option<Script>, LoadError> {
    let joined = [dir, name].concat();
    let resolved = match resolve_under_with_fs(fs, root, &joined) {
        Ok(resolved) => resolved,
        Err(err) if err.is_not_found() => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let mut bytes = Vec::new();
    fs.open_read(&resolved)?
        .take(MAX_PAYLOAD as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PAYLOAD {
        return Err("script file exceeds research limit".into());
    }
    Ok(Some(Script::decode(&bytes)?))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    pub script_slot: u8,
    pub target: u32,
    /// Fields +14812, +14808, +14804 in original argument-push order.
    pub parameters: [u32; 3],
    pub registered_tick: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlEvent {
    Applied,
    CommonEntered {
        slot: u8,
    },
    /// Original 0x2080 also captures an engine-wide save snapshot.
    /// The core transition is ready, but callers must stop until they implement it.
    MainPreparedNeedsSnapshot,
}

#[derive(Clone)]
pub struct ScriptControl {
    callbacks: BTreeMap<u8, Callback>,
    armed: [bool; 64],
    cvns: bool,
    settings: BTreeMap<u16, u32>,
    virtual_tick: u32,
    close_requested: bool,
}

impl ScriptControl {
    /// An explicit virtual time domain for deterministic research, not wall-clock emulation.
    pub fn new(virtual_tick: u32) -> Self {
        Self {
            callbacks: BTreeMap::new(),
            armed: [false; 64],
            cvns: false,
            settings: BTreeMap::new(),
            virtual_tick,
            close_requested: false,
        }
    }

    /// Configures script control rules for `profile` (`cvns` uses `0x446660` callback rules).
    pub fn for_profile(profile: &crate::profile::EngineProfile, virtual_tick: u32) -> Self {
        Self {
            cvns: profile.family == crate::profile::Family::Cvns,
            ..Self::new(virtual_tick)
        }
    }
    pub fn callbacks(&self) -> &BTreeMap<u8, Callback> {
        &self.callbacks
    }
    /// Queue a host close request for the script's confirmation handler, if registered.
    pub fn request_close(&mut self) -> bool {
        if !self.callbacks.get(&26).is_some_and(|c| c.enabled) {
            return false;
        }
        self.close_requested = true;
        true
    }
    /// The automatic dispatch 0x140064f50 runs at every VM run entry: nothing while the
    /// 0x208c flag (+12240) is set. With the 0x2089 setting (+14788) the registered entries
    /// 3..6 and 28 are due every time, 10..17 while message window 0..7 is shown (+4) and
    /// 26 when its dispatch word is set; without it only 26 (word set) or else 28. Entry 26's
    /// word is set by the host window-close path. Requests survive blocking and are
    /// consumed once dispatched. Returned in push order: the last one runs
    /// first.
    pub fn due_callbacks(&mut self, windows_shown: [bool; 8]) -> Vec<(u8, usize)> {
        if !self.cvns && self.settings.get(&0x208c).copied().unwrap_or(0) != 0 {
            return Vec::new();
        }
        let close_requested = std::mem::take(&mut self.close_requested);
        let registered = |index: u8| {
            self.callbacks
                .get(&index)
                .filter(|c| c.enabled)
                .map(|c| (c.script_slot, c.target as usize))
        };
        let armed = std::mem::replace(&mut self.armed, [false; 64]);
        if self.settings.get(&0x2089).copied().unwrap_or(0) == 0 {
            let slot = if !self.cvns && close_requested {
                26
            } else {
                28
            };
            return registered(slot).into_iter().collect();
        }
        if self.cvns {
            return (0..64u8)
                .filter(|&index| match index {
                    3..=5 | 28 => true,
                    10..=13 => windows_shown[usize::from(index - 10)],
                    32..=63 => armed[usize::from(index)],
                    _ => false,
                })
                .filter_map(registered)
                .collect();
        }
        (0..32u8)
            .filter(|&index| match index {
                3..=6 | 28 => true,
                10..=17 => windows_shown[usize::from(index - 10)],
                26 => close_requested,
                _ => false,
            })
            .filter_map(registered)
            .collect()
    }
    pub fn settings(&self) -> &BTreeMap<u16, u32> {
        &self.settings
    }
    pub fn set_virtual_tick(&mut self, tick: u32) {
        self.virtual_tick = tick;
    }

    /// 0x140080530: a main replacement disables only callbacks owned by slot 0.
    pub(crate) fn main_replaced(&mut self) {
        for callback in self.callbacks.values_mut() {
            if callback.script_slot == 0 {
                callback.enabled = false;
                callback.target = u32::MAX;
            }
        }
    }

    pub(crate) fn handle(
        &mut self,
        call: &mut Call<'_>,
        source: &mut dyn ScriptSource,
    ) -> Result<ControlEvent, LoadError> {
        let (pc, command) = (call.pc, call.command);
        let op = command + 0x2000;
        let flags = match command {
            0x81 => {
                let slot = call.argument(4, argument_names(command)[0])?;
                if slot >= 3 {
                    return Err(error(pc, op, "common slot must be 0..2").into());
                }
                let name = {
                    let raw = call.argument(8, argument_names(command)[1])?;
                    call.vm.resolve_string(raw)?
                };
                let script = source.load(&name)?;
                call.enter_common(slot, name, script)?;
                return Ok(ControlEvent::CommonEntered {
                    slot: call.vm.current_slot(),
                });
            }
            0x412 => {
                let name = {
                    let raw = call.argument(4, argument_names(command)[0])?;
                    call.vm.resolve_string(raw)?
                };
                let script = source.load(&name)?;
                call.enter_cvns_common(name, script)?;
                return Ok(ControlEvent::CommonEntered {
                    slot: call.vm.current_slot(),
                });
            }
            0x80 => {
                let name = {
                    let raw = call.argument(4, argument_names(command)[0])?;
                    call.vm.resolve_string(raw)?
                };
                let script = source.load(&name)?;
                call.switch_main(name, script)?;
                self.main_replaced();
                return Ok(ControlEvent::MainPreparedNeedsSnapshot);
            }
            0x89 | 0x8c => {
                self.settings.insert(
                    op,
                    u32::from(call.argument(4, argument_names(command)[0])? != 0),
                );
                0x4004
            }
            0x83 => {
                let high = call.argument(4, argument_names(command)[0])?;
                let low = call.argument(8, argument_names(command)[1])?;
                self.settings.insert(op, low | high.wrapping_shl(16));
                0x4008
            }
            0x88 => {
                let slot = call.argument(4, argument_names(command)[0])?;
                let target = call.argument(8, argument_names(command)[1])?;
                let parameters = [
                    call.argument(20, argument_names(command)[4])?,
                    call.argument(16, argument_names(command)[3])?,
                    call.argument(12, argument_names(command)[2])?,
                ];
                if slot > 63 {
                    return Err(error(pc, op, "callback slot outside 0..63").into());
                }
                if target != u32::MAX
                    && !call.vm.is_boundary(call.vm.current_slot(), target as usize)
                {
                    return Err(
                        error(pc, op, "callback target is not an instruction boundary").into(),
                    );
                }
                self.callbacks.insert(
                    slot as u8,
                    Callback {
                        script_slot: call.vm.current_slot(),
                        target,
                        parameters,
                        registered_tick: self.virtual_tick,
                        enabled: target != u32::MAX,
                    },
                );
                0x4014
            }
            0x8a => {
                let slot = (call.argument(4, argument_names(command)[0])? & 63) as u8;
                let callback = self
                    .callbacks
                    .get(&slot)
                    .filter(|c| c.enabled)
                    .ok_or_else(|| error(pc, op, "callback is absent or disabled"))?;
                call.call_script(callback.script_slot, callback.target as usize)?;
                return Ok(ControlEvent::Applied);
            }
            0x8b => {
                let slot = call.argument(4, argument_names(command)[0])?;
                if slot > 63 {
                    return Err(error(pc, op, "callback slot outside 0..63").into());
                }
                if let Some(callback) = self.callbacks.get_mut(&(slot as u8)) {
                    callback.enabled = false;
                    callback.target = u32::MAX;
                }
                self.armed[slot as usize] = false;
                0x4004
            }
            0x42a => {
                let slot = call.argument(4, argument_names(command)[0])?;
                let enabled = self.settings.get(&0x2089).copied().unwrap_or(0) != 0;
                if enabled && (1..64).contains(&(slot as i32)) {
                    let index = slot as usize;
                    if self.callbacks.get(&(slot as u8)).is_some_and(|c| c.enabled) {
                        self.armed[index] = true;
                    }
                }
                0x4004
            }
            _ => unreachable!("registered script-control command"),
        };
        call.complete(flags)?;
        Ok(ControlEvent::Applied)
    }
}

fn argument_names(command: u16) -> &'static [&'static str] {
    match command {
        0x80 => &["main script"],
        0x412 => &["common script"],
        0x81 => &["common slot", "common script"],
        0x83 => &["high word", "low word"],
        0x88 => &[
            "callback slot",
            "target PC",
            "parameter 12",
            "parameter 16",
            "parameter 20",
        ],
        0x89 | 0x8c => &["enabled"],
        0x8a | 0x8b | 0x42a => &["callback slot"],
        _ => unreachable!("registered script-control arguments"),
    }
}

/// Record 0x10b: +14788 (the 0x2089 setting) and 64 callback entries of 28 bytes:
/// +0 dispatch word (zeroed on registration), +4 script slot, +8 target (-1 = none),
/// +12/+16/+20 the SP-12/SP-16/SP-20 parameters, +24 registration clock (0x140071ed0).
pub const CALLBACK_RECORD: usize = 4 + 64 * 28;

impl ScriptControl {
    /// Writer 0x140069270 copies +14788..+16584 verbatim. Rust never runs the automatic
    /// dispatcher, so every dispatch word is the zero that registration leaves.
    pub fn saved_callbacks(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CALLBACK_RECORD);
        let setting = self.settings.get(&0x2089).copied().unwrap_or(0);
        out.extend(setting.to_le_bytes());
        for slot in 0..64u8 {
            let entry = match self.callbacks.get(&slot) {
                Some(c) => [
                    0,
                    u32::from(c.script_slot),
                    c.target,
                    c.parameters[2],
                    c.parameters[1],
                    c.parameters[0],
                    c.registered_tick,
                ],
                // Constructor 0x140068580: slot 0, target -1, other words zero.
                None => [0, 0, u32::MAX, 0, 0, 0, 0],
            };
            for word in entry {
                out.extend(word.to_le_bytes());
            }
        }
        out
    }

    /// Loader 0x140081960 after 0x140069fd0: callbacks registered by common scripts survive
    /// the load, saved ones of common scripts are disabled, and main-script callbacks come
    /// from the save. The 0x2089 setting is restored from +14788.
    pub fn restore_callbacks(&mut self, record: &[u8]) -> Result<(), &'static str> {
        if record.len() != CALLBACK_RECORD {
            return Err("callback record has unexpected length");
        }
        let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
        let mut callbacks = BTreeMap::new();
        for slot in 0..64u8 {
            let at = 4 + 28 * usize::from(slot);
            let [dispatch, script, target, p12, p16, p20, tick] =
                std::array::from_fn(|i| word(at + 4 * i));
            if dispatch != 0 {
                return Err("saved callback is mid-dispatch, which is not modeled");
            }
            if script > 3 {
                return Err("saved callback script slot is out of range");
            }
            let mut saved = Callback {
                script_slot: script as u8,
                target,
                parameters: [p20, p16, p12],
                registered_tick: tick,
                enabled: target != u32::MAX,
            };
            if script != 0 {
                saved.target = u32::MAX;
                saved.enabled = false;
            }
            let current = self.callbacks.get(&slot).filter(|c| c.script_slot != 0);
            let entry = match current {
                Some(current) => Some(current.clone()),
                None if saved == Callback::unused() => None,
                None => Some(saved),
            };
            if let Some(entry) = entry {
                callbacks.insert(slot, entry);
            }
        }
        self.callbacks = callbacks;
        self.settings.insert(0x2089, u32::from(word(0) != 0));
        Ok(())
    }
}

impl Callback {
    /// The constructor's entry, indistinguishable from never registered.
    fn unused() -> Self {
        Self {
            script_slot: 0,
            target: u32::MAX,
            parameters: [0; 3],
            registered_tick: 0,
            enabled: false,
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn main_replacement_disables_only_main_callbacks_and_preserves_control_settings() {
        let mut host = ScriptControl::new(123);
        host.settings.insert(0x2089, 1);
        for slot in 0..4 {
            host.callbacks.insert(
                slot,
                Callback {
                    script_slot: slot,
                    target: 42,
                    parameters: [1, 2, 3],
                    registered_tick: 123,
                    enabled: true,
                },
            );
        }
        let before = host.callbacks.clone();
        host.main_replaced();
        assert!(!host.callbacks[&0].enabled);
        assert_eq!(host.callbacks[&0].target, u32::MAX);
        assert_eq!(host.callbacks[&0].parameters, [1, 2, 3]);
        for slot in 1..4 {
            assert_eq!(host.callbacks[&slot], before[&slot]);
        }
        assert_eq!(host.settings[&0x2089], 1);
    }
}
