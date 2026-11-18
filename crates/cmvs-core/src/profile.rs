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
mod quirks;
mod select;
mod text;
mod validate;

pub use contract::{ContractProof, ContractRules, ContractSide};
pub use gate::{CommandGate, GateCounts, GateError, GateErrorKind};
pub use model::{
    Binary, CommandStatus, EngineProfile, Family, GameRecord, MachineProof, ProfileError,
    RecordFile, Semantics, VariantBinding, VariantId,
};
pub(crate) use quirks::InspectedBuild;
pub use select::{
    EXECUTABLE_NAMES, Evidence, FileEvidence, Game, GameFiles, ProfileSelection, SelectionBasis,
    SelectionError,
};
pub use validate::SAME_BUILD_SECONDS;

use crate::exe::{Case, Dispatch, PeImage};
use std::{collections::BTreeMap, sync::OnceLock};

#[cfg(feature = "profile-research")]
impl ContractRules {
    /// Active rules for the built-in research registry.
    pub fn builtin() -> Self {
        Self::parse(include_str!("../profiles/contract_rules.txt"))
            .expect("prevalidated contract rules")
    }
}

pub struct ResolvedCommand {
    pub semantics: Semantics,
    pub first_unreviewed: Option<u16>,
}

impl Binary {
    /// Whether a freshly located switch is exactly the one recorded for this binary.
    pub fn matches(&self, image: &PeImage, dispatch: &Dispatch) -> bool {
        !self.cases.is_empty()
            && self.arch == image.arch
            && self.timestamp == image.timestamp
            && self.linker == image.linker
            && self.image_base == image.image_base
            && self.table_rva == dispatch.table_rva
            && self.default_rva == dispatch.default_rva
            && self.cases == dispatch.cases
    }
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
            proofs: Vec::new(),
            contracts: Vec::new(),
            reviews: Vec::new(),
            variant: Vec::new(),
            variant_ids: Vec::new(),
            maps: Vec::new(),
        };
        profile.validate()?;
        Ok(profile)
    }
    fn validate(&self) -> Result<(), ProfileError> {
        validate::validate_profile(self, |id, command| {
            crate::dispatch::variant_entry(id, command).is_some()
        })
    }
}

/// Engine profiles and game identities, with evidence in research registries.
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

fn unpack_builtin(blob: &[u8]) -> Result<Vec<u8>, ProfileError> {
    let (raw_len, compressed) = blob
        .split_at_checked(4)
        .ok_or_else(|| ProfileError::new("embedded registry missing length header"))?;
    let raw_len = u32::from_le_bytes(raw_len.try_into().expect("four-byte header")) as usize;
    crate::lzss::decode(compressed, raw_len)
        .map_err(|_| ProfileError::new("invalid embedded registry LZSS payload"))
}

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
    /// Decodes the runtime projection of already validated research profiles.
    fn decode_prevalidated(blob: &[u8]) -> Result<Self, ProfileError> {
        let (profiles, games) = binary::decode_bundle(&unpack_builtin(blob)?)?;
        Ok(Self {
            profiles,
            games,
            contracts: BTreeMap::new(),
        })
    }
    /// Built-in playback data for game identification, command dispatch and build quirks.
    /// Command statuses are validated at build time. PE dispatch tables, variant
    /// evidence, machine proofs and contract certificates are kept in research profiles.
    pub fn builtin() -> &'static Self {
        static REGISTRY: OnceLock<Registry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            Self::decode_prevalidated(BUILTIN_REGISTRY)
                .expect("prevalidated embedded registry bundle is valid")
        })
    }
    /// Full built-in profiles and proofs for offline tools that inspect executable tables.
    #[cfg(feature = "profile-research")]
    pub fn builtin_research() -> &'static Self {
        static REGISTRY: OnceLock<Registry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            let blob = include_bytes!(concat!(env!("OUT_DIR"), "/builtin_research_profiles.bin"));
            let raw = unpack_builtin(blob).expect("unpack validated research profiles");
            let profiles: Vec<_> = std::str::from_utf8(&raw)
                .expect("embedded profile sources are UTF-8")
                .split('\0')
                .collect();
            Self::parse_with_contracts(
                &profiles,
                include_str!("../profiles/games.txt"),
                include_str!("../profiles/contracts.txt"),
                &[ContractRules::builtin()],
            )
            .expect("prevalidated research profiles are valid")
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
    /// Look up the executable's profile and identity by hash.
    pub fn binary(&self, sha256: &[u8; 32]) -> Option<(&EngineProfile, &Binary)> {
        self.profiles
            .iter()
            .find_map(|p| p.binary(sha256).map(|b| (p, b)))
    }
    fn validate(&self) -> Result<(), ProfileError> {
        validate::validate_registry(&self.profiles, &self.games, &self.contracts, |target| {
            crate::dispatch::cvns_entry(target).is_some()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    #[test]
    fn playback_bundle_preserves_runtime_fields_from_validated_profiles() {
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
        let rules_text =
            fs::read_to_string(dir.join("contract_rules.txt")).expect("read contract_rules.txt");
        let rules = ContractRules::parse(&rules_text).expect("parse contract_rules.txt");
        let parsed = Registry::parse_with_contracts(&profile_refs, &games, &contracts, &[rules])
            .expect("parse raw profile files");

        #[cfg(feature = "profile-research")]
        assert_eq!(Registry::builtin_research(), &parsed);

        let playback = Registry::builtin();
        assert_eq!(playback.games(), parsed.games());
        assert_eq!(playback.profiles().len(), parsed.profiles().len());
        assert!(!parsed.contracts.is_empty());
        assert!(playback.contracts.is_empty());
        for (runtime, source) in playback.profiles().iter().zip(parsed.profiles()) {
            assert_eq!(runtime.id, source.id);
            assert_eq!(runtime.family, source.family);
            assert_eq!(runtime.base, source.base);
            assert_eq!(runtime.inspected_build(), source.inspected_build());
            assert_eq!(runtime.statuses(), source.statuses(), "{}", source.id);
            assert_eq!(runtime.maps, source.maps, "{}", source.id);
            assert_eq!(runtime.binaries.len(), source.binaries.len());
            for (binary, known) in runtime.binaries.iter().zip(&source.binaries) {
                assert_eq!(
                    (binary.arch, &binary.file, binary.sha256),
                    (known.arch, &known.file, known.sha256)
                );
                assert_eq!(
                    (
                        binary.timestamp,
                        binary.linker,
                        binary.image_base,
                        binary.table_rva,
                        binary.default_rva
                    ),
                    (0, (0, 0), 0, 0, 0)
                );
                assert!(binary.cases.is_empty());
            }
            assert!(runtime.proofs.is_empty());
            assert!(runtime.reviews.is_empty());
            assert!(runtime.contracts.is_empty());
            assert_eq!(runtime.variant_bindings().count(), 0);
            assert_eq!(
                runtime.variant_ids,
                source
                    .variant_bindings()
                    .map(|(command, binding)| (command, binding.id))
                    .collect::<Vec<_>>()
            );
            for command in 0..runtime.statuses().len() as u16 {
                assert_eq!(
                    runtime.semantics(command),
                    source.semantics(command),
                    "{} 0x{:04x}",
                    source.id,
                    command + 0x2000
                );
            }
        }
    }

    #[test]
    fn contract_rules_roundtrip_and_reject_invalid_lines() {
        let builtin = ContractRules::parse(include_str!("../profiles/contract_rules.txt")).unwrap();
        let canonical = builtin.to_text();
        assert_eq!(ContractRules::parse(&canonical), Ok(builtin.clone()));
        assert_eq!(
            ContractRules::parse(&format!("# comment\n\n{canonical}# trailing comment\n")),
            Ok(builtin),
        );

        assert!(ContractRules::parse("").is_err());
        assert!(ContractRules::parse(&canonical.replace("extractor ", "other ")).is_err());
        assert!(ContractRules::parse(&canonical.replace("0f", "ZZ")).is_err());
        assert!(ContractRules::parse(&format!("{canonical}extra 00\n")).is_err());
    }

    #[test]
    fn prevalidated_blob_roundtrips_and_rejects_corrupt_envelopes() {
        let decoded = Registry::decode_prevalidated(BUILTIN_REGISTRY).expect("decode builtin blob");
        assert_eq!(&decoded, Registry::builtin());

        assert!(Registry::decode_prevalidated(b"").is_err());
        assert!(Registry::decode_prevalidated(b"\x01\x02\x03").is_err());
        assert!(Registry::decode_prevalidated(b"\xff\x00\x00\x00\xff").is_err());

        let invalid_bundle = crate::lzss::encode(b"CMVB\x01");
        let mut framed = (5u32).to_le_bytes().to_vec();
        framed.extend_from_slice(&invalid_bundle);
        assert!(Registry::decode_prevalidated(&framed).is_err());
    }

    #[test]
    fn binary_bundle_roundtrips_and_rejects_corrupt_inputs() {
        let builtin = Registry::builtin();
        let encoded = binary::encode_bundle(&builtin.profiles, &builtin.games);
        let decoded = binary::decode_bundle(&encoded).expect("decode builtin bundle");
        assert_eq!(decoded.0, builtin.profiles);
        assert_eq!(decoded.1, builtin.games);

        assert!(binary::decode_bundle(b"").is_err());
        assert!(binary::decode_bundle(b"CMVB\x01").is_err());
        assert!(binary::decode_bundle(&encoded[..encoded.len() - 1]).is_err());

        let mut with_trailing = encoded;
        with_trailing.push(0);
        assert!(binary::decode_bundle(&with_trailing).is_err());
    }
}
