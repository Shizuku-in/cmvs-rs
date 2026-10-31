//! Checks each pending system command against the selected profile before any host runs it.
use super::{CommandStatus, ProfileSelection};
use crate::machine::Machine;
use std::{collections::BTreeSet, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateErrorKind {
    /// The build's switch has no such case.
    Absent,
    /// The build only returns 0x4000 here; that behaviour is not implemented.
    NoOp,
    /// Present, but its semantics are not yet established for this build.
    Unreviewed,
    /// 0x201d declares a different game than the matched record.
    GameIdMismatch { expected: u32, found: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateError {
    pub profile: String,
    pub base: Option<String>,
    pub script: String,
    pub pc: usize,
    pub command: u16,
    pub kind: GateErrorKind,
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let site = format!(
            "system command 0x{:04x} at {} / 0x{:x}",
            0x2000 + self.command,
            self.script,
            self.pc
        );
        match self.kind {
            GateErrorKind::Absent => write!(
                f,
                "{site} does not exist in engine profile {}",
                self.profile
            ),
            GateErrorKind::NoOp => write!(
                f,
                "{site} is a no-op stub in engine profile {} (returns 0x4000 without effects); not implemented",
                self.profile
            ),
            GateErrorKind::Unreviewed => write!(
                f,
                "{site} is unreviewed for engine profile {} (base {}); pass --allow-unreviewed only for research",
                self.profile,
                self.base.as_deref().unwrap_or("-")
            ),
            GateErrorKind::GameIdMismatch { expected, found } => write!(
                f,
                "{site} declares game ID 0x{found:08x}, but profile {} recorded 0x{expected:08x}",
                self.profile
            ),
        }
    }
}

impl std::error::Error for GateError {}

#[derive(Debug, Clone)]
pub struct CommandGate<'a> {
    selection: ProfileSelection<'a>,
    allow_unreviewed: bool,
    allowed: BTreeSet<u16>,
    presumed: BTreeSet<u16>,
    counts: GateCounts,
}

/// Successful gate checks by semantic source. Wait retries count again; these are
/// dispatch attempts, not completed commands or address/branch coverage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GateCounts {
    pub base: u64,
    pub inherited: u64,
    pub reviewed: u64,
    pub presumed: u64,
    pub unreviewed: u64,
}

impl<'a> CommandGate<'a> {
    pub fn new(selection: ProfileSelection<'a>) -> Self {
        Self {
            selection,
            allow_unreviewed: false,
            allowed: BTreeSet::new(),
            presumed: BTreeSet::new(),
            counts: GateCounts::default(),
        }
    }
    /// Research only: run unreviewed commands with the base semantics.
    pub fn set_allow_unreviewed(&mut self, allow: bool) {
        self.allow_unreviewed = allow;
    }
    pub fn selection(&self) -> &ProfileSelection<'a> {
        &self.selection
    }
    /// Unreviewed commands let through so far.
    pub fn allowed(&self) -> &BTreeSet<u16> {
        &self.allowed
    }
    /// Presumed commands run so far: suspects first when a run goes wrong.
    pub fn presumed(&self) -> &BTreeSet<u16> {
        &self.presumed
    }
    pub fn counts(&self) -> GateCounts {
        self.counts
    }
    pub fn summary_line(&self) -> String {
        format!(
            "# profile gate accepted attempts: base={} inherited={} reviewed={} presumed={} unreviewed={} (includes wait retries; not completion/coverage)",
            self.counts.base,
            self.counts.inherited,
            self.counts.reviewed,
            self.counts.presumed,
            self.counts.unreviewed
        )
    }
    /// The presumed commands that ran, for a stop report; `None` when there were none.
    pub fn presumed_line(&self) -> Option<String> {
        (!self.presumed.is_empty()).then(|| {
            let list: Vec<_> = self
                .presumed
                .iter()
                .map(|c| format!("0x{:04x}", 0x2000 + c))
                .collect();
            format!("# presumed commands run: {}", list.join(" "))
        })
    }
    /// Checks the pending command; returns it on the first time an unreviewed command passes.
    /// Pending commands are re-checked when a wait resumes, which is harmless.
    pub fn check(
        &mut self,
        vm: &Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<Option<u16>, GateError> {
        let profile = self.selection.profile;
        let error = |kind| GateError {
            profile: profile.id.clone(),
            base: profile.base.clone(),
            script: vm.current_name().escape_ascii().to_string(),
            pc,
            command,
            kind,
        };
        // 0x201d/0x201e/0x201f read the top stack word; 0x201d carries the game ID.
        if command == 0x1d
            && let Some(record) = self.selection.record
            && let Ok(found) = vm.stack().peek_u32(4)
            && found != record.game_id
        {
            return Err(error(GateErrorKind::GameIdMismatch {
                expected: record.game_id,
                found,
            }));
        }
        match profile.status(command) {
            CommandStatus::Base => {
                self.counts.base += 1;
                Ok(None)
            }
            CommandStatus::Inherited => {
                self.counts.inherited += 1;
                Ok(None)
            }
            CommandStatus::Reviewed => {
                self.counts.reviewed += 1;
                Ok(None)
            }
            CommandStatus::Presumed => {
                self.counts.presumed += 1;
                self.presumed.insert(command);
                Ok(None)
            }
            CommandStatus::Absent => Err(error(GateErrorKind::Absent)),
            CommandStatus::NoOp => Err(error(GateErrorKind::NoOp)),
            CommandStatus::Unreviewed if self.allow_unreviewed => {
                self.counts.unreviewed += 1;
                Ok(self.allowed.insert(command).then_some(command))
            }
            CommandStatus::Unreviewed => Err(error(GateErrorKind::Unreviewed)),
        }
    }
}
