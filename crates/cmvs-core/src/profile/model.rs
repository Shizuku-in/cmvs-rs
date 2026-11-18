//! Pure data structures for engine profiles, game records, and validation errors.
use crate::exe::{Arch, Case};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Cmvs,
    /// The predecessor engine. A CVNS profile derives from a CMVS root profile and only runs
    /// commands whose review maps them to a runtime command (see [`crate::cvns`]).
    Cvns,
}

/// Selected only after the command gate authorizes the dispatch attempt.
/// Variant semantics do not constitute a Reviewed or contract proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semantics {
    Root,
    Variant(VariantId),
}

macro_rules! define_variant_ids {
    ($(($variant:ident, $name:literal)),* $(,)?) => {
        /// Static implementation identifiers; execution obeys the unreviewed block policy.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum VariantId {
            $($variant,)*
        }

        impl VariantId {
            pub fn parse(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)*
                    _ => None,
                }
            }

            pub fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }
        }
    };
}

define_variant_ids!(
    (TextDialog, "text-dialog"),
    (ScreenCoordinates, "screen-coordinates"),
    (InputPress, "input-press"),
    (FontCells, "font-cells"),
    (TextAlpha, "text-alpha"),
    (ObjectQueue, "object-queue"),
    (ListSelect, "list-select"),
    (Particles, "particles"),
    (ParticlesX87, "particles-x87"),
    (Rain, "rain"),
    (BackgroundAudio, "background-audio"),
    (SeRestart, "se-restart"),
    (SeGain, "se-gain"),
    (MixerGain, "mixer-gain"),
    (ImageTables, "image-tables"),
    (ActivationPass, "activation-pass"),
    (WindowScaleMotion, "window-scale-motion"),
    (WindowPositionMotion, "window-position-motion"),
    (WindowOpacityTrack, "window-opacity-track"),
    (LogicalText, "logical-text"),
    (ViewMotion, "view-motion"),
    (WindowFullMove, "window-full-move"),
    (LegacyEffects, "legacy-effects"),
    (WindowChildImage, "window-child-image"),
    (WindowDrawScale, "window-draw-scale"),
    (SequenceWindowImage, "sequence-window-image"),
    (DrawQuery, "draw-query"),
    (CameraMoveX, "camera-move-x"),
    (AudioPreload, "audio-preload"),
    (PolarPoint, "polar-point"),
    (FallingBillboards, "falling-billboards"),
    (FallingBillboardsX87, "falling-billboards-x87"),
    (CollisionParticles, "collision-particles"),
    (FadingParticles, "fading-particles"),
    (Fragments, "fragments"),
    (KeyedArchives, "keyed-archives"),
    (DisplayCount, "display-count"),
    (FullscreenMonitor, "fullscreen-monitor"),
    (MgvPlayer, "mgv-player"),
    (MovieSeek, "movie-seek"),
    (MovieControl, "movie-control"),
);

/// A declared variant implementation and its executable-bound evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantBinding {
    pub id: VariantId,
    pub arch: Arch,
    pub binary_sha256: [u8; 32],
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandStatus {
    /// The switch's default (returns 0); every command beyond the switch is absent too.
    Absent,
    /// The shared stub returning 0x4000 with no effect; base semantics never apply.
    NoOp,
    /// Root profile only: this build is the semantic reference.
    Base,
    /// A complete same-architecture fingerprint matches the root; proof is mandatory.
    Inherited,
    /// A recorded manual review found the root semantics apply; the record is mandatory.
    Reviewed,
    /// Complete offline contracts match under an active, registered certificate.
    ContractEquivalent,
    /// Comparison found no difference, but conservative limits (blocked data, indirect
    /// calls, recursion, depth) hide part of the code; the comparable-shape proof is
    /// mandatory. Allowed by default; an explicit block-presumed policy rejects it.
    Presumed,
    /// Present, but not yet compared with or reviewed against the base.
    Unreviewed,
}

impl CommandStatus {
    pub fn name(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::NoOp => "noop",
            Self::Base => "base",
            Self::Inherited => "inherited",
            Self::Reviewed => "reviewed",
            Self::ContractEquivalent => "contract-equivalent",
            Self::Presumed => "presumed",
            Self::Unreviewed => "unreviewed",
        }
    }
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

/// Executable identity and, in research profiles, the inspected dispatch table.
///
/// Playback profiles retain the architecture, name and hash. Their case list is
/// empty and their remaining PE fields are zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binary {
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
pub struct EngineProfile {
    pub id: String,
    pub family: Family,
    pub base: Option<String>,
    pub binaries: Vec<Binary>,
    pub(super) statuses: Vec<CommandStatus>,
    pub(super) proofs: Vec<(u16, MachineProof)>,
    pub(super) contracts: Vec<(u16, [u8; 32])>,
    /// Review records, as paths relative to `docs/` of a section headed by the command.
    pub(super) reviews: Vec<(u16, String)>,
    pub(super) variant: Vec<(u16, VariantBinding)>,
    /// Compact playback profiles carry only the selected implementation IDs.
    pub(super) variant_ids: Vec<(u16, VariantId)>,
    /// CVNS only: reviewed command to the runtime command whose semantics it shares.
    pub(super) maps: Vec<(u16, u16)>,
}

pub(super) fn lookup_sorted<T>(entries: &[(u16, T)], command: u16) -> Option<&T> {
    entries
        .binary_search_by_key(&command, |&(key, _)| key)
        .ok()
        .map(|idx| &entries[idx].1)
}

impl EngineProfile {
    pub fn semantics(&self, command: u16) -> Semantics {
        lookup_sorted(&self.variant_ids, command)
            .copied()
            .or_else(|| self.variant(command).map(|entry| entry.id))
            .map_or(Semantics::Root, Semantics::Variant)
    }
    /// The executable-bound evidence, when this is a research profile.
    pub fn variant(&self, command: u16) -> Option<&VariantBinding> {
        lookup_sorted(&self.variant, command)
    }
    /// Executable-bound variant evidence. Use `semantics` for playback dispatch.
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
    pub fn binary(&self, sha256: &[u8; 32]) -> Option<&Binary> {
        self.binaries.iter().find(|b| &b.sha256 == sha256)
    }
}

/// Maps the data of one game to the profile of the build it ships with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRecord {
    pub game: String,
    pub profile: String,
    /// Exact startup variants sharing this game's identity and script archive.
    pub start_scripts: Vec<RecordFile>,
    pub script_archive: RecordFile,
    /// Value the start script hands to 0x201d.
    pub game_id: u32,
    pub executables: Vec<RecordFile>,
    /// Integer cleared by the explicit `--skip-auto-resume` preview policy, when verified.
    pub skip_auto_resume: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordFile {
    pub path: String,
    pub sha256: [u8; 32],
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
