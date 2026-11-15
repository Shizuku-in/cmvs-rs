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
mod binary;
mod contract;
mod gate;
mod model;
mod select;
mod text;

pub use contract::{ContractProof, ContractRules, ContractSide};
pub use gate::{CommandGate, GateCounts, GateError, GateErrorKind};
pub use model::{
    Binary, CommandStatus, EngineProfile, Family, GameRecord, MachineProof, ProfileError,
    RecordFile, Semantics, VariantBinding, VariantId,
};
pub use select::{
    EXECUTABLE_NAMES, Evidence, FileEvidence, Game, GameFiles, ProfileSelection, SelectionBasis,
    SelectionError,
};

use crate::exe::{Case, Dispatch, PeImage};
use std::{collections::BTreeMap, sync::OnceLock};

include!(concat!(env!("OUT_DIR"), "/contract_rules.rs"));

impl ContractRules {
    /// Active rules computed at build time from the bundled extractor sources and
    /// contract tables. Editing any dependency invalidates the registered certificates
    /// until extraction and review are repeated.
    pub fn builtin() -> Self {
        BUILTIN_CONTRACT_RULES
    }
}

pub struct ResolvedCommand {
    pub semantics: Semantics,
    pub first_unreviewed: Option<u16>,
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

impl EngineProfile {
    pub fn semantics(&self, command: u16) -> Semantics {
        self.variant(command)
            .map_or(Semantics::Root, |entry| Semantics::Variant(entry.id))
    }
    pub fn variant(&self, command: u16) -> Option<&VariantBinding> {
        self.variant.get(&command)
    }
    pub fn variant_bindings(&self) -> impl Iterator<Item = (u16, &VariantBinding)> {
        self.variant
            .iter()
            .map(|(&command, entry)| (command, entry))
    }
    /// Verified loader zero-fill bank sizes, in integer/bit/float/string order.
    /// This is startup layout evidence, independent of command gate policy.
    pub(crate) fn initial_globals(&self) -> Option<[u32; 4]> {
        match self.id.as_str() {
            "cmvs-20140217" | "cmvs-20160201" | "cmvs-20171114" => Some([4096, 262144, 2048, 128]),
            // docs/games/puriring: .data file bytes end at 0x4ae000; the banks at 0x4b6708,
            // 0x4dc710 (0x8000 bytes of packed bits, 0x440be0), 0x4da710 and 0x4ba710.
            "cmvs-20070119" => Some([4096, 262144, 2048, 128]),
            // docs/games/miharu/README.md: .data file bytes end at 0x4b6600; integers
            // 0x4bdd18, strings 0x4c1d20 (256 x 1024), bits 0x501d20 (0x8000 bytes); no floats.
            "cvns-20090203" => Some([4096, 262144, 0, 256]),
            _ => None,
        }
    }
    /// String variable slots of this engine: 256 for CVNS, 128 for CMVS.
    pub fn string_slots(&self) -> u32 {
        match self.family {
            Family::Cmvs => 128,
            Family::Cvns => 256,
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
    /// The runtime command a reviewed CVNS command is translated to.
    pub fn map(&self, command: u16) -> Option<u16> {
        self.maps.get(&command).copied()
    }
    /// The translation a VM running this profile's scripts must apply; `None` for CMVS.
    pub fn command_map(&self) -> Option<crate::cvns::CommandMap> {
        (self.family == Family::Cvns)
            .then(|| crate::cvns::CommandMap::new(self.maps.iter().map(|(&s, &t)| (s, t))))
    }
    pub fn count(&self, status: CommandStatus) -> usize {
        self.statuses.iter().filter(|&&s| s == status).count()
    }
    /// Independently inspected ASUKIMI build and its inherited command schema.
    pub(crate) fn is_inspected_20071117(&self) -> bool {
        self.id == "cmvs-20071117"
            && self.base.as_deref() == Some("cmvs-20160201")
            && self
                .binary(
                    &crate::digest::parse_hex(
                        "caff7901a7dc4ec963d7fde358028cdd68dfd87fc7925c9715a20e7a5f36d02b",
                    )
                    .expect("static SHA-256"),
                )
                .is_some()
    }

    /// Independently inspected Primitive Link build; docs/games/puriring.
    pub(crate) fn is_inspected_20070119(&self) -> bool {
        self.id == "cmvs-20070119"
            && self.base.as_deref() == Some("cmvs-20160201")
            && self
                .binary(
                    &crate::digest::parse_hex(
                        "5ce421db393cbc5ca443fa405c5a9e31c348416109cb1279d91718f490c0e602",
                    )
                    .expect("static SHA-256"),
                )
                .is_some()
    }

    /// Independently inspected x86 sine interpolation; docs/games/{asukimi,harouse}.
    pub(crate) fn uses_legacy_sine_modes(&self) -> bool {
        self.is_inspected_20071117()
            || self.is_inspected_20080719()
            || self.is_inspected_20080903()
            || self.is_inspected_20090815()
            || self.is_inspected_20100314()
    }

    /// Independently inspected Harouse build; capabilities still bind separately.
    pub(crate) fn is_inspected_20080719(&self) -> bool {
        self.id == "cmvs-20080719"
            && self.base.as_deref() == Some("cmvs-20160201")
            && self
                .binary(
                    &crate::digest::parse_hex(
                        "afe3b3a895e7b3ba5f78e6f6464296f1d028e87307c8de5a568b88fd6086491e",
                    )
                    .expect("static SHA-256"),
                )
                .is_some()
    }

    /// Independently inspected ASUNANA x86 build; docs/games/asunana.
    pub(crate) fn is_inspected_20080903(&self) -> bool {
        self.id == "cmvs-20080903"
            && self.base.as_deref() == Some("cmvs-20160201")
            && self
                .binary(
                    &crate::digest::parse_hex(
                        "f77ecbfd39d1076c66915544d73b9c2d2c5876fbabeaef3d589050b6094d13ac",
                    )
                    .expect("static SHA-256"),
                )
                .is_some()
    }

    /// Independently inspected MEMORIA x86 build; docs/games/memoria.
    pub(crate) fn is_inspected_20090815(&self) -> bool {
        self.id == "cmvs-20090815"
            && self.base.as_deref() == Some("cmvs-20160201")
            && self
                .binary(
                    &crate::digest::parse_hex(
                        "36fa780a5d2b8df4ac6a6e48a2f78708b79aa10d22eb575a5ae74742d902f71a",
                    )
                    .expect("static SHA-256"),
                )
                .is_some()
    }

    /// NATSUUTA x86 build, matched against MEMORIA function by function; docs/games/natsuuta.
    pub(crate) fn is_inspected_20100314(&self) -> bool {
        self.id == "cmvs-20100314"
            && self.base.as_deref() == Some("cmvs-20160201")
            && self
                .binary(
                    &crate::digest::parse_hex(
                        "97575da34a284fa534084ca8452741e0e176650f3396308f92c2c8cf7ffc9bc8",
                    )
                    .expect("static SHA-256"),
                )
                .is_some()
    }

    /// Independently inspected HASSAKU x64 build; docs/games/hassaku.
    pub(crate) fn is_inspected_20101211(&self) -> bool {
        self.id == "cmvs-20101211"
            && self.base.as_deref() == Some("cmvs-20160201")
            && self
                .binary(
                    &crate::digest::parse_hex(
                        "158409c71cf14c3dedb7d051585e5f48e56dbc621bb19e56d084cd4fe8d4352f",
                    )
                    .expect("static SHA-256"),
                )
                .is_some()
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
            variant: BTreeMap::new(),
            maps: BTreeMap::new(),
        };
        profile.validate()?;
        Ok(profile)
    }
    fn validate(&self) -> Result<(), ProfileError> {
        if !text::is_identifier(&self.id) {
            return Err(ProfileError::new("profile id must be lowercase ASCII"));
        }
        if self.family == Family::Cvns && self.base.is_none() {
            return Err(ProfileError::new(
                "a CVNS profile derives from a CMVS root profile",
            ));
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
            let cvns = self.family == Family::Cvns;
            if cvns
                && !matches!(
                    status,
                    CommandStatus::Absent
                        | CommandStatus::NoOp
                        | CommandStatus::Unreviewed
                        | CommandStatus::Reviewed
                )
            {
                return Err(ProfileError::new(
                    "CVNS commands are absent, noop, unreviewed or reviewed with a map",
                ));
            }
            if (cvns && *status == CommandStatus::Reviewed)
                != self.maps.contains_key(&(command as u16))
            {
                return Err(ProfileError::new(
                    "only reviewed CVNS commands, and all of them, carry a map",
                ));
            }
            if let Some(target) = self.map(command as u16)
                && target > crate::cvns::LAST_COMMAND
            {
                return Err(ProfileError::new(
                    "map target is outside the system opcodes",
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
            if let Some(variant) = self.variant(command as u16) {
                if *status != CommandStatus::Unreviewed || self.base.is_none() {
                    return Err(ProfileError::new(
                        "variant override requires an unreviewed derived command",
                    ));
                }
                if !self.binaries.iter().any(|binary| {
                    binary.arch == variant.arch && binary.sha256 == variant.binary_sha256
                }) {
                    return Err(ProfileError::new(
                        "variant evidence does not match the profile binary",
                    ));
                }
                if crate::dispatch::variant_entry(variant.id, command as u16).is_none() {
                    return Err(ProfileError::new(
                        "variant override has no registered handler",
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

/// Engine profiles, game identities, and reviewed command-contract proofs.
///
/// Use [`Self::builtin`] for the embedded registry or [`Self::parse`] for explicit
/// registry text. [`Self::identify_game`] reads evidence without executing the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    profiles: Vec<EngineProfile>,
    games: Vec<GameRecord>,
    contracts: BTreeMap<[u8; 32], ContractProof>,
}

const BUILTIN_REGISTRY: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/builtin_registry.bin"));

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
        let profiles = profiles
            .iter()
            .map(|t| {
                let profile = text::parse_profile(t)?;
                profile.validate()?;
                Ok(profile)
            })
            .collect::<Result<Vec<_>, ProfileError>>()?;
        let registry = Self {
            profiles,
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
            let (raw_len_bytes, compressed) = BUILTIN_REGISTRY
                .split_at_checked(4)
                .expect("embedded registry has length header");
            let raw_len = u32::from_le_bytes(
                raw_len_bytes
                    .try_into()
                    .expect("four-byte embedded registry length"),
            ) as usize;
            let raw = crate::lzss::decode(compressed, raw_len)
                .expect("embedded registry LZSS payload is valid");
            let (profiles, games, contracts) =
                binary::decode_bundle(&raw).expect("embedded registry binary payload is valid");
            for profile in &profiles {
                profile
                    .validate()
                    .expect("embedded engine profile is valid");
            }
            let active = ContractRules::builtin();
            assert!(
                contracts.values().all(|proof| proof.rules == active),
                "embedded contract rules are current"
            );
            let registry = Self {
                profiles,
                games,
                contracts,
            };
            registry.validate().expect("embedded profile data is valid");
            registry
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
                // Inheritance is one level deep until a reviewed chain exists. CVNS has no
                // root of its own: its reviewed commands map onto CMVS root semantics.
                if base.base.is_some() || base.family != Family::Cmvs {
                    return Err(ProfileError::new(format!(
                        "profile {} must derive from a root profile of its family",
                        profile.id
                    )));
                }
                // A review compares against root semantics, which only a Base command has;
                // a CVNS review names the runtime command, and CVNS-only semantics need
                // their own registered handler.
                let has_root_semantics = |command: u16| match profile.map(command) {
                    None => base.status(command) == CommandStatus::Base,
                    Some(target) if target < crate::cvns::CVNS_ONLY => {
                        base.status(target) == CommandStatus::Base
                    }
                    Some(target) => crate::dispatch::root_entry(target).is_some(),
                };
                if profile
                    .reviews
                    .keys()
                    .any(|&command| !has_root_semantics(command))
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
            for (index, start) in game.start_scripts.iter().enumerate() {
                if game.start_scripts[..index]
                    .iter()
                    .any(|earlier| earlier.sha256 == start.sha256)
                {
                    return Err(ProfileError::new(format!(
                        "game {} repeats a start script hash",
                        game.game
                    )));
                }
            }
            let earlier = &self.games[..i];
            if earlier.iter().any(|g| {
                g.game == game.game
                    || g.start_scripts.iter().any(|start| {
                        game.start_scripts
                            .iter()
                            .any(|other| other.sha256 == start.sha256)
                    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    #[test]
    fn builtin_matches_raw_profile_files_on_disk() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles");
        let mut profile_paths = fs::read_dir(&dir)
            .expect("read profiles directory")
            .map(|entry| entry.expect("dir entry").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("profile"))
            .collect::<Vec<_>>();
        profile_paths.sort();

        let profile_texts = profile_paths
            .iter()
            .map(|path| fs::read_to_string(path).expect("read profile file"))
            .collect::<Vec<_>>();
        let profile_refs = profile_texts.iter().map(String::as_str).collect::<Vec<_>>();
        let games = fs::read_to_string(dir.join("games.txt")).expect("read games.txt");
        let contracts = fs::read_to_string(dir.join("contracts.txt")).expect("read contracts.txt");

        let parsed = Registry::parse_with_contracts(
            &profile_refs,
            &games,
            &contracts,
            &[ContractRules::builtin()],
        )
        .expect("parse raw profile files");

        assert_eq!(Registry::builtin(), &parsed);
    }

    #[test]
    fn binary_bundle_roundtrips_and_rejects_corrupt_inputs() {
        let builtin = Registry::builtin();
        let encoded = binary::encode_bundle(&builtin.profiles, &builtin.games, &builtin.contracts)
            .expect("encode builtin bundle");
        let decoded = binary::decode_bundle(&encoded).expect("decode builtin bundle");
        assert_eq!(decoded.0, builtin.profiles);
        assert_eq!(decoded.1, builtin.games);
        assert_eq!(decoded.2, builtin.contracts);

        assert!(binary::decode_bundle(b"").is_err());
        assert!(binary::decode_bundle(b"CMVB\x02").is_err());
        assert!(binary::decode_bundle(&encoded[..encoded.len() - 1]).is_err());

        let mut with_trailing = encoded;
        with_trailing.push(0);
        assert!(binary::decode_bundle(&with_trailing).is_err());
    }
}
