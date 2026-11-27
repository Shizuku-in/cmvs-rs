//! Source profiles and executable-bound evidence.
use crate::{
    CommandStatus, Family, Semantics, VariantId,
    exe::{Arch, Case},
};
use std::fmt;

/// A declared variant implementation and its executable-bound evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantBinding {
    pub id: VariantId,
    pub arch: Arch,
    pub binary_sha256: [u8; 32],
    pub evidence: String,
}

/// Identity-bound machine comparison evidence: for `Inherited` the digest covers the
/// complete normalized handler and its dependencies, for `Presumed` everything comparable
/// with blocked parts as placeholders. No original code bytes are embedded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineProof {
    pub arch: Arch,
    pub base_sha256: [u8; 32],
    pub derived_sha256: [u8; 32],
    pub digest: [u8; 32],
}

/// An inspected executable and its dispatch table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchBinary {
    pub arch: Arch,
    pub file: String,
    pub sha256: [u8; 32],
    pub timestamp: u32,
    pub linker: (u8, u8),
    pub image_base: u64,
    pub table_rva: u32,
    pub default_rva: u32,
    pub cases: Vec<Case>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchProfile {
    pub id: String,
    pub family: Family,
    pub base: Option<String>,
    pub binaries: Vec<ResearchBinary>,
    pub(super) relocated_binaries: Vec<ResearchBinary>,
    pub(super) statuses: Vec<CommandStatus>,
    pub(super) proofs: Vec<(u16, MachineProof)>,
    pub(super) contracts: Vec<(u16, [u8; 32])>,
    /// Review records, as paths relative to `docs/` of a section headed by the command.
    pub(super) reviews: Vec<(u16, String)>,
    pub(super) variant: Vec<(u16, VariantBinding)>,
    /// CVNS only: reviewed command to the runtime command whose semantics it shares.
    pub(super) maps: Vec<(u16, u16)>,
}

pub(super) fn lookup_sorted<T>(entries: &[(u16, T)], command: u16) -> Option<&T> {
    entries
        .binary_search_by_key(&command, |&(key, _)| key)
        .ok()
        .map(|idx| &entries[idx].1)
}

impl ResearchProfile {
    pub fn semantics(&self, command: u16) -> Semantics {
        self.variant(command)
            .map_or(Semantics::Root, |binding| Semantics::Variant(binding.id))
    }
    /// The inspected implementation binding, if present.
    pub fn variant(&self, command: u16) -> Option<&VariantBinding> {
        lookup_sorted(&self.variant, command)
    }
    /// Executable-bound variant evidence.
    pub fn variant_bindings(&self) -> impl Iterator<Item = (u16, &VariantBinding)> {
        self.variant
            .iter()
            .map(|(command, entry)| (*command, entry))
    }
    /// Status of system command `0x2000 + command`; cases beyond the switch are absent.
    pub fn status(&self, command: u16) -> CommandStatus {
        self.statuses
            .get(command as usize)
            .copied()
            .unwrap_or(CommandStatus::Absent)
    }
    pub fn statuses(&self) -> &[CommandStatus] {
        &self.statuses
    }
    /// The machine comparison proof of an `Inherited` or `Presumed` command.
    pub fn proof(&self, command: u16) -> Option<&MachineProof> {
        lookup_sorted(&self.proofs, command)
    }
    pub fn review(&self, command: u16) -> Option<&str> {
        lookup_sorted(&self.reviews, command).map(String::as_str)
    }
    pub fn contract_id(&self, command: u16) -> Option<&[u8; 32]> {
        lookup_sorted(&self.contracts, command)
    }
    /// The runtime command a reviewed CVNS command is translated to.
    pub fn map(&self, command: u16) -> Option<u16> {
        lookup_sorted(&self.maps, command).copied()
    }
    pub fn count(&self, status: CommandStatus) -> usize {
        self.statuses.iter().filter(|&&s| s == status).count()
    }
    /// Primary executables and verified relocated builds of the same switch.
    pub fn binaries(&self) -> impl Iterator<Item = &ResearchBinary> {
        self.binaries.iter().chain(&self.relocated_binaries)
    }
    pub fn binary(&self, sha256: &[u8; 32]) -> Option<&ResearchBinary> {
        self.binaries().find(|b| &b.sha256 == sha256)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileError {
    pub line: Option<usize>,
    pub message: String,
}

impl ProfileError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            line: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "profile data line {line}: {}", self.message),
            None => write!(f, "profile data: {}", self.message),
        }
    }
}

impl std::error::Error for ProfileError {}
