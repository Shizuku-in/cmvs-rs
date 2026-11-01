//! Engine profiles: which analysed CMVS build supplies the semantics for a run.
//! A profile covers one build (its x86/x64 executables); game records map data files to it.
//! Selection never defaults; the command gate rejects absent and unreviewed commands.
mod contract;
mod gate;
mod select;
mod text;

pub use contract::{ContractProof, ContractRules, ContractSide};
pub use gate::{CommandGate, GateCounts, GateError, GateErrorKind};
pub use select::{
    EXECUTABLE_NAMES, Evidence, FileEvidence, Game, GameFiles, ProfileSelection, SelectionBasis,
    SelectionError,
};

use crate::exe::{Arch, Case, Dispatch, PeImage};
use std::{collections::BTreeMap, fmt, sync::OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Cmvs,
}

/// Selected only after the command gate authorizes the dispatch attempt.
/// Research semantics do not constitute a Reviewed or contract proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semantics {
    Root,
    Research(ResearchId),
}

/// Static implementation identifiers; execution obeys the unreviewed block policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchId {
    FontCells,
    Particles,
    BackgroundAudio,
    FallingBillboards,
    CollisionParticles,
}
impl ResearchId {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "font-cells" => Some(Self::FontCells),
            "particles" => Some(Self::Particles),
            "background-audio" => Some(Self::BackgroundAudio),
            "falling-billboards" => Some(Self::FallingBillboards),
            "collision-particles" => Some(Self::CollisionParticles),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::FontCells => "font-cells",
            Self::Particles => "particles",
            Self::BackgroundAudio => "background-audio",
            Self::FallingBillboards => "falling-billboards",
            Self::CollisionParticles => "collision-particles",
        }
    }
}

/// A declared research implementation and its executable-bound evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchOverride {
    pub id: ResearchId,
    pub arch: Arch,
    pub binary_sha256: [u8; 32],
    pub evidence: String,
}

pub struct ResolvedCommand {
    pub semantics: Semantics,
    pub first_unreviewed: Option<u16>,
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

impl Binary {
    /// Whether a freshly located switch is exactly the one recorded for this binary.
    pub fn matches(&self, image: &PeImage, dispatch: &Dispatch) -> bool {
        self.arch == image.arch
            && self.timestamp == image.timestamp
            && self.linker == image.linker
            && self.image_base == image.image_base
            && self.table_rva == dispatch.table_rva
            && self.default_rva == dispatch.default_rva
            && self.cases == dispatch.cases
    }
}

/// x86/x64 executables of one build are linked minutes apart (66-147 s in the 2010-2023 survey).
pub const SAME_BUILD_SECONDS: u32 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineProfile {
    pub id: String,
    pub family: Family,
    pub base: Option<String>,
    pub binaries: Vec<Binary>,
    statuses: Vec<CommandStatus>,
    proofs: BTreeMap<u16, MachineProof>,
    contracts: BTreeMap<u16, [u8; 32]>,
    /// Review records, as paths relative to `docs/` of a section headed by the command.
    reviews: BTreeMap<u16, String>,
    research: BTreeMap<u16, ResearchOverride>,
}

impl EngineProfile {
    pub fn semantics(&self, command: u16) -> Semantics {
        self.research(command)
            .map_or(Semantics::Root, |entry| Semantics::Research(entry.id))
    }
    pub fn research(&self, command: u16) -> Option<&ResearchOverride> {
        self.research.get(&command)
    }
    pub fn research_overrides(&self) -> impl Iterator<Item = (u16, &ResearchOverride)> {
        self.research
            .iter()
            .map(|(&command, entry)| (command, entry))
    }
    /// Verified loader zero-fill bank sizes, in integer/bit/float/string order.
    /// This is startup layout evidence, independent of command research policy.
    pub(crate) fn initial_globals(&self) -> Option<[u32; 4]> {
        match self.id.as_str() {
            "cmvs-20160201" | "cmvs-20171114" => Some([4096, 262144, 2048, 128]),
            _ => None,
        }
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
        self.proofs.get(&command)
    }
    pub fn review(&self, command: u16) -> Option<&str> {
        self.reviews.get(&command).map(String::as_str)
    }
    pub fn contract_id(&self, command: u16) -> Option<&[u8; 32]> {
        self.contracts.get(&command)
    }
    pub fn count(&self, status: CommandStatus) -> usize {
        self.statuses.iter().filter(|&&s| s == status).count()
    }
    pub fn binary(&self, sha256: &[u8; 32]) -> Option<&Binary> {
        self.binaries.iter().find(|b| &b.sha256 == sha256)
    }
    /// Builds a profile from located switches: present cases become `Base` for a root
    /// profile and `Unreviewed` for a derived one.
    pub fn from_dispatch(
        id: &str,
        family: Family,
        base: Option<&str>,
        binaries: Vec<(String, [u8; 32], &PeImage, &Dispatch)>,
    ) -> Result<Self, ProfileError> {
        let present = if base.is_some() {
            CommandStatus::Unreviewed
        } else {
            CommandStatus::Base
        };
        let statuses = binaries
            .first()
            .ok_or(ProfileError::new("profile needs at least one binary"))?
            .3
            .cases
            .iter()
            .map(|case| match case {
                Case::Handler(_) => present,
                Case::NoOp(_) => CommandStatus::NoOp,
                Case::Default => CommandStatus::Absent,
            })
            .collect();
        let profile = Self {
            id: id.to_owned(),
            family,
            base: base.map(str::to_owned),
            binaries: binaries
                .into_iter()
                .map(|(file, sha256, image, dispatch)| Binary {
                    arch: image.arch,
                    file,
                    sha256,
                    timestamp: image.timestamp,
                    linker: image.linker,
                    image_base: image.image_base,
                    table_rva: dispatch.table_rva,
                    default_rva: dispatch.default_rva,
                    cases: dispatch.cases.clone(),
                })
                .collect(),
            statuses,
            proofs: BTreeMap::new(),
            contracts: BTreeMap::new(),
            reviews: BTreeMap::new(),
            research: BTreeMap::new(),
        };
        profile.validate()?;
        Ok(profile)
    }
    fn validate(&self) -> Result<(), ProfileError> {
        if !text::is_identifier(&self.id) {
            return Err(ProfileError::new("profile id must be lowercase ASCII"));
        }
        let first = self
            .binaries
            .first()
            .ok_or(ProfileError::new("profile needs at least one binary"))?;
        for (i, binary) in self.binaries.iter().enumerate() {
            if self.binaries[..i].iter().any(|b| b.arch == binary.arch) {
                return Err(ProfileError::new("profile lists one architecture twice"));
            }
            if binary.cases.len() != self.statuses.len() {
                return Err(ProfileError::new("binaries disagree on the switch size"));
            }
            if binary.timestamp.abs_diff(first.timestamp) > SAME_BUILD_SECONDS {
                return Err(ProfileError::new(
                    "executables were not linked minutes apart; not one build",
                ));
            }
        }
        let kind = |case: &Case| std::mem::discriminant(case);
        for (command, status) in self.statuses.iter().enumerate() {
            let case = self.binaries[0].cases[command];
            if self
                .binaries
                .iter()
                .any(|b| kind(&b.cases[command]) != kind(&case))
            {
                return Err(ProfileError::new(
                    "executables disagree on a command; not one build",
                ));
            }
            let allowed = match (status, case) {
                (CommandStatus::Absent, Case::Default) => true,
                (CommandStatus::NoOp, Case::NoOp(_)) => true,
                (CommandStatus::Base, Case::Handler(_)) => self.base.is_none(),
                (CommandStatus::Unreviewed, Case::Handler(_)) => self.base.is_some(),
                (
                    CommandStatus::Inherited
                    | CommandStatus::Reviewed
                    | CommandStatus::Presumed
                    | CommandStatus::ContractEquivalent,
                    Case::Handler(_),
                ) => self.base.is_some(),
                _ => false,
            };
            if !allowed {
                return Err(ProfileError::new(
                    "command status contradicts the switch or profile kind",
                ));
            }
            if let Some(research) = self.research(command as u16) {
                if *status != CommandStatus::Unreviewed || self.base.is_none() {
                    return Err(ProfileError::new(
                        "research override requires an unreviewed derived command",
                    ));
                }
                if !self.binaries.iter().any(|binary| {
                    binary.arch == research.arch && binary.sha256 == research.binary_sha256
                }) {
                    return Err(ProfileError::new(
                        "research evidence does not match the profile binary",
                    ));
                }
                if crate::dispatch::research_entry(research.id, command as u16).is_none() {
                    return Err(ProfileError::new(
                        "research override has no registered handler",
                    ));
                }
            }
            if matches!(status, CommandStatus::Inherited | CommandStatus::Presumed)
                != self.proofs.contains_key(&(command as u16))
            {
                return Err(ProfileError::new(
                    "inherited and presumed status require exactly one proof",
                ));
            }
            if (*status == CommandStatus::Reviewed) != self.reviews.contains_key(&(command as u16))
            {
                return Err(ProfileError::new(
                    "reviewed status requires exactly one review record",
                ));
            }
            if (*status == CommandStatus::ContractEquivalent)
                != self.contracts.contains_key(&(command as u16))
            {
                return Err(ProfileError::new(
                    "contract-equivalent status requires exactly one certificate",
                ));
            }
            if let Some(proof) = self.proof(command as u16)
                && !self
                    .binaries
                    .iter()
                    .any(|b| b.arch == proof.arch && b.sha256 == proof.derived_sha256)
            {
                return Err(ProfileError::new(
                    "comparison proof does not match derived binary",
                ));
            }
        }
        Ok(())
    }
}

/// Maps the data of one game to the profile of the build it ships with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRecord {
    pub game: String,
    pub profile: String,
    pub start_script: RecordFile,
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
    fn new(message: impl Into<String>) -> Self {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    profiles: Vec<EngineProfile>,
    games: Vec<GameRecord>,
    contracts: BTreeMap<[u8; 32], ContractProof>,
}

const BUILTIN_PROFILES: [&str; 3] = [
    include_str!("../profiles/cmvs-20160201.profile"),
    include_str!("../profiles/cmvs-20150405.profile"),
    include_str!("../profiles/cmvs-20171114.profile"),
];
const BUILTIN_GAMES: &str = include_str!("../profiles/games.txt");

impl Registry {
    pub fn parse(profiles: &[&str], games: &str) -> Result<Self, ProfileError> {
        Self::parse_with_contracts(profiles, games, "", &[])
    }
    /// `catalog` and `active_rules` are trusted, reviewed registry inputs. Reports or
    /// profiles alone cannot authorize contracts; no evidence is loaded from game files.
    pub fn parse_with_contracts(
        profiles: &[&str],
        games: &str,
        catalog: &str,
        active_rules: &[ContractRules],
    ) -> Result<Self, ProfileError> {
        let registry = Self {
            profiles: profiles
                .iter()
                .map(|t| text::parse_profile(t))
                .collect::<Result<_, _>>()?,
            games: text::parse_games(games)?,
            contracts: contract::parse_catalog(catalog, active_rules)?,
        };
        registry.validate()?;
        Ok(registry)
    }
    /// The embedded profiles; their validity is covered by the test suite.
    pub fn builtin() -> &'static Self {
        static REGISTRY: OnceLock<Registry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            Self::parse_with_contracts(
                &BUILTIN_PROFILES,
                BUILTIN_GAMES,
                include_str!("../profiles/contracts.txt"),
                &[ContractRules::builtin()],
            )
            .expect("embedded profile data is valid")
        })
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
    pub fn contract_proof(&self, id: &[u8; 32]) -> Option<&ContractProof> {
        self.contracts.get(id)
    }
    /// The profile and binary an executable hash was analysed as.
    pub fn binary(&self, sha256: &[u8; 32]) -> Option<(&EngineProfile, &Binary)> {
        self.profiles
            .iter()
            .find_map(|p| p.binary(sha256).map(|b| (p, b)))
    }
    fn validate(&self) -> Result<(), ProfileError> {
        for (i, profile) in self.profiles.iter().enumerate() {
            if self.profiles[..i].iter().any(|p| p.id == profile.id) {
                return Err(ProfileError::new(format!(
                    "duplicate profile {}",
                    profile.id
                )));
            }
            if let Some(base) = &profile.base {
                let base = self.profile(base).ok_or(ProfileError::new(format!(
                    "profile {} names a missing base",
                    profile.id
                )))?;
                // Inheritance is one level deep until a reviewed chain exists.
                if base.base.is_some() || base.family != profile.family {
                    return Err(ProfileError::new(format!(
                        "profile {} must derive from a root profile of its family",
                        profile.id
                    )));
                }
                // A review compares against root semantics, which only a Base command has.
                if profile
                    .reviews
                    .keys()
                    .any(|&command| base.status(command) != CommandStatus::Base)
                {
                    return Err(ProfileError::new("reviewed command has no root semantics"));
                }
                for (&command, proof) in &profile.proofs {
                    if base.status(command) != CommandStatus::Base
                        || !base
                            .binaries
                            .iter()
                            .any(|b| b.arch == proof.arch && b.sha256 == proof.base_sha256)
                    {
                        return Err(ProfileError::new(
                            "comparison proof does not match root binary/command",
                        ));
                    }
                }
                for (&command, id) in &profile.contracts {
                    if base.status(command) != CommandStatus::Base {
                        return Err(ProfileError::new("contract command has no root semantics"));
                    }
                    let proof = self.contracts.get(id).ok_or_else(|| {
                        ProfileError::new("contract certificate is missing or revoked")
                    })?;
                    proof.bind(command, base, profile)?;
                }
            }
        }
        for (i, game) in self.games.iter().enumerate() {
            let earlier = &self.games[..i];
            if earlier.iter().any(|g| {
                g.game == game.game
                    || g.start_script.sha256 == game.start_script.sha256
                    || g.script_archive.sha256 == game.script_archive.sha256
            }) {
                return Err(ProfileError::new(format!(
                    "game {} duplicates an earlier record",
                    game.game
                )));
            }
            let profile = self
                .profile(&game.profile)
                .ok_or(ProfileError::new(format!(
                    "game {} names a missing profile",
                    game.game
                )))?;
            for exe in &game.executables {
                if profile.binary(&exe.sha256).is_none() {
                    return Err(ProfileError::new(format!(
                        "game {} executable {} is not a binary of profile {}",
                        game.game, exe.path, profile.id
                    )));
                }
            }
        }
        Ok(())
    }
}
