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

/// Static implementation identifiers; execution obeys the unreviewed block policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantId {
    TextDialog,
    ScreenCoordinates,
    InputPress,
    FontCells,
    TextAlpha,
    ObjectQueue,
    ListSelect,
    Particles,
    ParticlesX87,
    Rain,
    BackgroundAudio,
    SeRestart,
    SeGain,
    MixerGain,
    ImageTables,
    ActivationPass,
    WindowScaleMotion,
    WindowPositionMotion,
    WindowOpacityTrack,
    LogicalText,
    ViewMotion,
    WindowFullMove,
    LegacyEffects,
    WindowChildImage,
    WindowDrawScale,
    SequenceWindowImage,
    DrawQuery,
    CameraMoveX,
    AudioPreload,
    PolarPoint,
    FallingBillboards,
    FallingBillboardsX87,
    CollisionParticles,
    FadingParticles,
    Fragments,
    KeyedArchives,
    DisplayCount,
    FullscreenMonitor,
    MgvPlayer,
    MovieSeek,
    MovieControl,
}

impl VariantId {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "text-dialog" => Some(Self::TextDialog),
            "screen-coordinates" => Some(Self::ScreenCoordinates),
            "input-press" => Some(Self::InputPress),
            "list-select" => Some(Self::ListSelect),
            "object-queue" => Some(Self::ObjectQueue),
            "text-alpha" => Some(Self::TextAlpha),
            "font-cells" => Some(Self::FontCells),
            "particles" => Some(Self::Particles),
            "particles-x87" => Some(Self::ParticlesX87),
            "rain" => Some(Self::Rain),
            "background-audio" => Some(Self::BackgroundAudio),
            "se-restart" => Some(Self::SeRestart),
            "se-gain" => Some(Self::SeGain),
            "mixer-gain" => Some(Self::MixerGain),
            "image-tables" => Some(Self::ImageTables),
            "activation-pass" => Some(Self::ActivationPass),
            "window-scale-motion" => Some(Self::WindowScaleMotion),
            "window-position-motion" => Some(Self::WindowPositionMotion),
            "window-opacity-track" => Some(Self::WindowOpacityTrack),
            "logical-text" => Some(Self::LogicalText),
            "view-motion" => Some(Self::ViewMotion),
            "window-full-move" => Some(Self::WindowFullMove),
            "legacy-effects" => Some(Self::LegacyEffects),
            "window-child-image" => Some(Self::WindowChildImage),
            "window-draw-scale" => Some(Self::WindowDrawScale),
            "sequence-window-image" => Some(Self::SequenceWindowImage),
            "draw-query" => Some(Self::DrawQuery),
            "camera-move-x" => Some(Self::CameraMoveX),
            "audio-preload" => Some(Self::AudioPreload),
            "polar-point" => Some(Self::PolarPoint),
            "falling-billboards" => Some(Self::FallingBillboards),
            "falling-billboards-x87" => Some(Self::FallingBillboardsX87),
            "collision-particles" => Some(Self::CollisionParticles),
            "fading-particles" => Some(Self::FadingParticles),
            "fragments" => Some(Self::Fragments),
            "keyed-archives" => Some(Self::KeyedArchives),
            "display-count" => Some(Self::DisplayCount),
            "fullscreen-monitor" => Some(Self::FullscreenMonitor),
            "mgv-player" => Some(Self::MgvPlayer),
            "movie-seek" => Some(Self::MovieSeek),
            "movie-control" => Some(Self::MovieControl),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::TextDialog => "text-dialog",
            Self::ScreenCoordinates => "screen-coordinates",
            Self::InputPress => "input-press",
            Self::ListSelect => "list-select",
            Self::ObjectQueue => "object-queue",
            Self::TextAlpha => "text-alpha",
            Self::FontCells => "font-cells",
            Self::Particles => "particles",
            Self::ParticlesX87 => "particles-x87",
            Self::Rain => "rain",
            Self::BackgroundAudio => "background-audio",
            Self::SeRestart => "se-restart",
            Self::SeGain => "se-gain",
            Self::MixerGain => "mixer-gain",
            Self::ImageTables => "image-tables",
            Self::ActivationPass => "activation-pass",
            Self::WindowScaleMotion => "window-scale-motion",
            Self::WindowPositionMotion => "window-position-motion",
            Self::WindowOpacityTrack => "window-opacity-track",
            Self::LogicalText => "logical-text",
            Self::ViewMotion => "view-motion",
            Self::WindowFullMove => "window-full-move",
            Self::LegacyEffects => "legacy-effects",
            Self::WindowChildImage => "window-child-image",
            Self::WindowDrawScale => "window-draw-scale",
            Self::SequenceWindowImage => "sequence-window-image",
            Self::DrawQuery => "draw-query",
            Self::CameraMoveX => "camera-move-x",
            Self::AudioPreload => "audio-preload",
            Self::PolarPoint => "polar-point",
            Self::FallingBillboards => "falling-billboards",
            Self::FallingBillboardsX87 => "falling-billboards-x87",
            Self::CollisionParticles => "collision-particles",
            Self::FadingParticles => "fading-particles",
            Self::Fragments => "fragments",
            Self::KeyedArchives => "keyed-archives",
            Self::DisplayCount => "display-count",
            Self::FullscreenMonitor => "fullscreen-monitor",
            Self::MgvPlayer => "mgv-player",
            Self::MovieSeek => "movie-seek",
            Self::MovieControl => "movie-control",
        }
    }
}

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

/// One analysed executable of a build; every address is relative to this file only.
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
    /// CVNS only: reviewed command to the runtime command whose semantics it shares.
    pub(super) maps: Vec<(u16, u16)>,
}

pub(super) fn lookup_sorted<T>(entries: &[(u16, T)], command: u16) -> Option<&T> {
    entries
        .binary_search_by_key(&command, |&(key, _)| key)
        .ok()
        .map(|idx| &entries[idx].1)
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
