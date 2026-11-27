//! Offline source profiles, command evidence, and PE dispatch inspection.
mod contract;
pub mod exe;
mod model;
mod text;
mod validate;

pub use contract::{ContractProof, ContractRules, ContractSide};
pub use model::{MachineProof, ProfileError, ResearchBinary, ResearchProfile, VariantBinding};
pub use validate::SAME_BUILD_SECONDS;

use cmvs_core::{
    profile::{
        Binary, CommandStatus, EngineProfile, Family, GameRecord, RecordFile, Registry, Semantics,
        VariantId,
    },
    vm::{cvns, dispatch},
};
use exe::{Case, Dispatch, PeImage};
use std::{collections::BTreeMap, error::Error, fs, io, path::Path};

impl ResearchBinary {
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

impl ResearchProfile {
    pub fn is_cvns(&self) -> bool {
        self.family == Family::Cvns
    }

    /// The translation a VM running this profile's scripts must apply; `None` for CMVS.
    pub fn command_map(&self) -> Option<cvns::CommandMap> {
        (self.family == Family::Cvns).then(|| cvns::CommandMap::new(self.maps.iter().copied()))
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
                .map(|(file, sha256, image, dispatch)| ResearchBinary {
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
            relocated_binaries: Vec::new(),
            statuses,
            proofs: Vec::new(),
            contracts: Vec::new(),
            reviews: Vec::new(),
            variant: Vec::new(),
            maps: Vec::new(),
        };
        profile.validate()?;
        Ok(profile)
    }
    fn validate(&self) -> Result<(), ProfileError> {
        validate::validate_profile(self, |id, command| {
            dispatch::variant_entry(id, command).is_some()
        })
    }
}

/// Validated source profiles and their projection for the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchRegistry {
    profiles: Vec<ResearchProfile>,
    playback: Registry,
    contracts: BTreeMap<[u8; 32], ContractProof>,
}

impl ResearchRegistry {
    pub fn parse(profiles: &[&str], games: &str) -> Result<Self, ProfileError> {
        Self::parse_with_contracts(profiles, games, "", &[])
    }

    /// The catalog and active rules are trusted inputs from the research directory.
    pub fn parse_with_contracts(
        profiles: &[&str],
        games: &str,
        catalog: &str,
        active_rules: &[ContractRules],
    ) -> Result<Self, ProfileError> {
        let profiles = profiles
            .iter()
            .map(|source| {
                let profile = text::parse_profile(source)?;
                profile.validate()?;
                Ok(profile)
            })
            .collect::<Result<Vec<_>, ProfileError>>()?;
        let games = text::parse_games(games)?;
        let contracts = contract::parse_catalog(catalog, active_rules)?;
        validate::validate_registry(&profiles, &games, &contracts, |target| {
            dispatch::CVNS.iter().any(|entry| entry.command == target)
        })?;

        let playback = Registry::new(
            profiles.iter().map(ResearchProfile::playback).collect(),
            games,
        );
        Ok(Self {
            profiles,
            playback,
            contracts,
        })
    }

    /// Load the reviewed catalog, active rules, game records, and .profile files.
    pub fn load(directory: &Path) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let entries = fs::read_dir(directory).map_err(|err| {
            io::Error::new(
                err.kind(),
                format!(
                    "cannot open research profiles at {}: {err}",
                    directory.display()
                ),
            )
        })?;
        let mut paths = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "profile") {
                paths.push(path);
            }
        }
        if paths.is_empty() {
            return Err(format!("no .profile files in {}", directory.display()).into());
        }
        paths.sort_unstable();
        let profiles = paths
            .iter()
            .map(fs::read_to_string)
            .collect::<io::Result<Vec<_>>>()?;
        let profiles = profiles.iter().map(String::as_str).collect::<Vec<_>>();
        let rules =
            ContractRules::parse(&fs::read_to_string(directory.join("contract_rules.txt"))?)?;
        Ok(Self::parse_with_contracts(
            &profiles,
            &fs::read_to_string(directory.join("games.txt"))?,
            &fs::read_to_string(directory.join("contracts.txt"))?,
            &[rules],
        )?)
    }

    pub fn profiles(&self) -> &[ResearchProfile] {
        &self.profiles
    }

    pub fn profile(&self, id: &str) -> Option<&ResearchProfile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    pub fn binary(&self, sha256: &[u8; 32]) -> Option<(&ResearchProfile, &ResearchBinary)> {
        self.profiles
            .iter()
            .find_map(|profile| profile.binary(sha256).map(|binary| (profile, binary)))
    }

    pub fn contract_proof(&self, id: &[u8; 32]) -> Option<&ContractProof> {
        self.contracts.get(id)
    }

    pub fn playback(&self) -> &Registry {
        &self.playback
    }
}

impl ResearchProfile {
    fn playback(&self) -> EngineProfile {
        EngineProfile::new(
            self.id.clone(),
            self.family,
            self.base.clone(),
            self.binaries()
                .map(|binary| Binary {
                    arch: binary.arch,
                    file: binary.file.clone(),
                    sha256: binary.sha256,
                })
                .collect(),
            self.statuses.clone(),
            self.variant_bindings()
                .map(|(command, binding)| (command, binding.id))
                .collect(),
            self.maps.clone(),
        )
    }
}
