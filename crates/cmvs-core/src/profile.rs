//! Game identification, engine-build profiles, and command authorization.
//!
//! Start with [`Registry::builtin`] and [`Registry::identify_game`] to match a game
//! directory to a known build. [`Game`] contains the selected profile and resource
//! paths needed by [`crate::runtime::Runtime`]. An unknown game is an error unless
//! the caller explicitly selects a profile.
//!
//! [`CommandGate`] checks system commands before dispatch. Absent and no-op commands
//! are rejected; unreviewed and presumed commands are allowed by default and can be
//! blocked through the gate policy. Authorization does not imply a handler exists
//! or that a complete game has been validated.
mod gate;
mod model;
mod playback;
mod quirks;
mod select;

pub use gate::{CommandGate, GateCounts, GateError, GateErrorKind};
pub use model::{
    Binary, CommandStatus, EngineProfile, Family, GameRecord, RecordFile, Semantics, VariantId,
};
pub(crate) use quirks::InspectedBuild;
pub use select::{
    EXECUTABLE_NAMES, Evidence, FileEvidence, Game, GameFiles, ProfileSelection, SelectionBasis,
    SelectionError,
};
use std::sync::OnceLock;

pub struct ResolvedCommand {
    pub semantics: Semantics,
    pub first_unreviewed: Option<u16>,
}

impl EngineProfile {
    /// Whether this profile belongs to the predecessor CVNS family.
    pub fn is_cvns(&self) -> bool {
        self.family == Family::Cvns
    }
    /// Startup INI filename for this engine family (`cvns.cfg` or `cmvs.cfg`).
    pub fn config_file_name(&self) -> &'static str {
        if self.is_cvns() {
            "cvns.cfg"
        } else {
            "cmvs.cfg"
        }
    }
    /// System save filename for this engine family (`save000.dat` or `system.dat`).
    pub fn system_save_file_name(&self) -> &'static str {
        if self.is_cvns() {
            "save000.dat"
        } else {
            "system.dat"
        }
    }
    /// CSS1 header identifier for this build and the game's 0x201d startup setting.
    pub fn css1_game_id(&self, declared: Option<u32>) -> Option<u32> {
        // 20070119 0x408b00 / 20071117 0x40bdf0 / 20080719 0x40bea0 zero the CSS1
        // header and never write offset 68: a format constant, not a game identifier.
        if matches!(
            self.inspected_build(),
            Some(
                InspectedBuild::Cmvs20070119
                    | InspectedBuild::Cmvs20071117
                    | InspectedBuild::Cmvs20080719
                    | InspectedBuild::Cmvs20080903
                    | InspectedBuild::Cmvs20090815
            )
        ) {
            Some(0)
        } else {
            declared
        }
    }
    /// Gameplay save slot format and slot-numbering family.
    pub fn save_family(&self) -> crate::save::SaveFamily {
        if self.is_cvns() {
            crate::save::SaveFamily::Cvns
        } else {
            crate::save::SaveFamily::Cmvs
        }
    }
    /// Whether native desktop input binds keys from `key.cfg` (false for CVNS fixed codes).
    pub fn uses_key_cfg(&self) -> bool {
        !self.is_cvns()
    }
    /// String variable slots of this engine: 256 for CVNS, 128 for CMVS.
    pub fn string_slots(&self) -> u32 {
        match self.family {
            Family::Cmvs => 128,
            Family::Cvns => 256,
        }
    }
    /// The translation a VM running this profile's scripts must apply; `None` for CMVS.
    pub fn command_map(&self) -> Option<crate::cvns::CommandMap> {
        (self.family == Family::Cvns)
            .then(|| crate::cvns::CommandMap::new(self.maps.iter().copied()))
    }
}

/// Runtime engine profiles and game identities.
/// [`Self::identify_game`] hashes files without executing the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    profiles: Vec<EngineProfile>,
    games: Vec<GameRecord>,
}

impl Registry {
    /// A registry of trusted runtime profiles and matching game records.
    pub fn new(profiles: Vec<EngineProfile>, games: Vec<GameRecord>) -> Self {
        Self { profiles, games }
    }

    /// Built-in playback data for game identification, command dispatch and build quirks.
    /// The generated tables contain the runtime fields of the validated source profiles.
    pub fn builtin() -> &'static Self {
        static REGISTRY: OnceLock<Registry> = OnceLock::new();
        REGISTRY.get_or_init(playback::registry)
    }
    pub fn profiles(&self) -> &[EngineProfile] {
        &self.profiles
    }
    pub fn games(&self) -> &[GameRecord] {
        &self.games
    }
    pub fn profile(&self, id: &str) -> Option<&EngineProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }
    /// Resolve an explicit selector by profile ID first, then by a registered game name.
    pub fn resolve_profile(&self, selector: &str) -> Option<&EngineProfile> {
        self.profile(selector).or_else(|| {
            let game = self.games.iter().find(|game| game.game == selector)?;
            self.profile(&game.profile)
        })
    }
    /// Look up the executable's profile and identity by hash.
    pub fn binary(&self, sha256: &[u8; 32]) -> Option<(&EngineProfile, &Binary)> {
        self.profiles
            .iter()
            .find_map(|p| p.binary(sha256).map(|b| (p, b)))
    }
}
