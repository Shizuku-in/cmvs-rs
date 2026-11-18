//! Semantic invariants for source profiles and their registry.
use super::{
    CommandStatus, ContractProof, Family, GameRecord, ProfileError, ResearchProfile, VariantId,
    text,
};
use crate::exe::Case;
use cmvs_core::vm::cvns;
use std::collections::BTreeMap;

/// x86/x64 executables of one build are linked minutes apart (66-147 s in the 2010-2023 survey).
pub const SAME_BUILD_SECONDS: u32 = 300;

pub(crate) fn validate_profile(
    profile: &ResearchProfile,
    has_variant_handler: impl Fn(VariantId, u16) -> bool,
) -> Result<(), ProfileError> {
    if !text::is_identifier(&profile.id) {
        return Err(ProfileError::new("profile id must be lowercase ASCII"));
    }
    if profile.family == Family::Cvns && profile.base.is_none() {
        return Err(ProfileError::new(
            "a CVNS profile derives from a CMVS root profile",
        ));
    }
    let first = profile
        .binaries
        .first()
        .ok_or_else(|| ProfileError::new("profile needs at least one binary"))?;
    for (i, binary) in profile.binaries.iter().enumerate() {
        if profile.binaries[..i].iter().any(|b| b.arch == binary.arch) {
            return Err(ProfileError::new("profile lists one architecture twice"));
        }
        if binary.cases.len() != profile.statuses.len() {
            return Err(ProfileError::new("binaries disagree on the switch size"));
        }
        if binary.timestamp.abs_diff(first.timestamp) > SAME_BUILD_SECONDS {
            return Err(ProfileError::new(
                "executables were not linked minutes apart; not one build",
            ));
        }
    }
    let kind = |case: &Case| std::mem::discriminant(case);
    for (command, status) in profile.statuses.iter().enumerate() {
        let case = profile.binaries[0].cases[command];
        if profile
            .binaries
            .iter()
            .any(|b| kind(&b.cases[command]) != kind(&case))
        {
            return Err(ProfileError::new(
                "executables disagree on a command; not one build",
            ));
        }
        let cvns = profile.family == Family::Cvns;
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
        if (cvns && *status == CommandStatus::Reviewed) != profile.map(command as u16).is_some() {
            return Err(ProfileError::new(
                "only reviewed CVNS commands, and all of them, carry a map",
            ));
        }
        if let Some(target) = profile.map(command as u16)
            && target > cvns::LAST_COMMAND
        {
            return Err(ProfileError::new(
                "map target is outside the system opcodes",
            ));
        }
        let allowed = match (status, case) {
            (CommandStatus::Absent, Case::Default) => true,
            (CommandStatus::NoOp, Case::NoOp(_)) => true,
            (CommandStatus::Base, Case::Handler(_)) => profile.base.is_none(),
            (CommandStatus::Unreviewed, Case::Handler(_)) => profile.base.is_some(),
            (
                CommandStatus::Inherited
                | CommandStatus::Reviewed
                | CommandStatus::Presumed
                | CommandStatus::ContractEquivalent,
                Case::Handler(_),
            ) => profile.base.is_some(),
            _ => false,
        };
        if !allowed {
            return Err(ProfileError::new(
                "command status contradicts the switch or profile kind",
            ));
        }
        if let Some(variant) = profile.variant(command as u16) {
            if *status != CommandStatus::Unreviewed || profile.base.is_none() {
                return Err(ProfileError::new(
                    "variant override requires an unreviewed derived command",
                ));
            }
            if !profile
                .binaries
                .iter()
                .any(|binary| binary.arch == variant.arch && binary.sha256 == variant.binary_sha256)
            {
                return Err(ProfileError::new(
                    "variant evidence does not match the profile binary",
                ));
            }
            if !has_variant_handler(variant.id, command as u16) {
                return Err(ProfileError::new(
                    "variant override has no registered handler",
                ));
            }
        }
        if matches!(status, CommandStatus::Inherited | CommandStatus::Presumed)
            != profile.proof(command as u16).is_some()
        {
            return Err(ProfileError::new(
                "inherited and presumed status require exactly one proof",
            ));
        }
        if (*status == CommandStatus::Reviewed) != profile.review(command as u16).is_some() {
            return Err(ProfileError::new(
                "reviewed status requires exactly one review record",
            ));
        }
        if (*status == CommandStatus::ContractEquivalent)
            != profile.contract_id(command as u16).is_some()
        {
            return Err(ProfileError::new(
                "contract-equivalent status requires exactly one certificate",
            ));
        }
        if let Some(proof) = profile.proof(command as u16)
            && !profile
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

pub(crate) fn validate_registry(
    profiles: &[ResearchProfile],
    games: &[GameRecord],
    contracts: &BTreeMap<[u8; 32], ContractProof>,
    has_cvns_handler: impl Fn(u16) -> bool,
) -> Result<(), ProfileError> {
    let find_profile = |id: &str| profiles.iter().find(|p| p.id == id);
    for (i, profile) in profiles.iter().enumerate() {
        if profiles[..i].iter().any(|p| p.id == profile.id) {
            return Err(ProfileError::new(format!(
                "duplicate profile {}",
                profile.id
            )));
        }
        if let Some(base) = &profile.base {
            let base = find_profile(base).ok_or_else(|| {
                ProfileError::new(format!("profile {} names a missing base", profile.id))
            })?;
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
                Some(target) if target < cvns::CVNS_ONLY => {
                    base.status(target) == CommandStatus::Base
                }
                Some(target) => has_cvns_handler(target),
            };
            if profile
                .reviews
                .iter()
                .any(|&(command, _)| !has_root_semantics(command))
            {
                return Err(ProfileError::new("reviewed command has no root semantics"));
            }
            for (command, proof) in &profile.proofs {
                let command = *command;
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
            for (command, id) in &profile.contracts {
                let command = *command;
                if base.status(command) != CommandStatus::Base {
                    return Err(ProfileError::new("contract command has no root semantics"));
                }
                let proof = contracts.get(id).ok_or_else(|| {
                    ProfileError::new("contract certificate is missing or revoked")
                })?;
                proof.bind(command, base, profile)?;
            }
        }
    }
    for (i, game) in games.iter().enumerate() {
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
        let earlier = &games[..i];
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
        let profile = find_profile(&game.profile).ok_or_else(|| {
            ProfileError::new(format!("game {} names a missing profile", game.game))
        })?;
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
