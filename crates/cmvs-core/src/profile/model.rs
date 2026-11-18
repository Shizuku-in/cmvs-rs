//! Runtime engine profiles and game identities.
use crate::exe::Arch;

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

/// Executable identity used to select build quirks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binary {
    pub arch: Arch,
    pub file: String,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineProfile {
    pub id: String,
    pub family: Family,
    pub base: Option<String>,
    pub binaries: Vec<Binary>,
    pub(super) statuses: Vec<CommandStatus>,
    pub(super) variant_ids: Vec<(u16, VariantId)>,
    pub(super) maps: Vec<(u16, u16)>,
}

fn lookup_sorted<T>(entries: &[(u16, T)], command: u16) -> Option<&T> {
    entries
        .binary_search_by_key(&command, |&(key, _)| key)
        .ok()
        .map(|index| &entries[index].1)
}

impl EngineProfile {
    /// Create a runtime profile from trusted command statuses, variants and translations.
    /// Commands in each override table must have distinct numbers.
    pub fn new(
        id: String,
        family: Family,
        base: Option<String>,
        binaries: Vec<Binary>,
        statuses: Vec<CommandStatus>,
        mut variant_ids: Vec<(u16, VariantId)>,
        mut maps: Vec<(u16, u16)>,
    ) -> Self {
        variant_ids.sort_unstable_by_key(|&(command, _)| command);
        maps.sort_unstable_by_key(|&(command, _)| command);
        Self {
            id,
            family,
            base,
            binaries,
            statuses,
            variant_ids,
            maps,
        }
    }

    pub fn semantics(&self, command: u16) -> Semantics {
        lookup_sorted(&self.variant_ids, command)
            .copied()
            .map_or(Semantics::Root, Semantics::Variant)
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
    /// Sparse overrides of the default command semantics.
    pub fn variants(&self) -> &[(u16, VariantId)] {
        &self.variant_ids
    }
    /// The runtime command a CVNS command is translated to.
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
