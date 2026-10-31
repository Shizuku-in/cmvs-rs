//! Reviewed, identity-bound contract certificates. This is a trusted registry input,
//! not an IR interpreter: hashes bind the offline evidence, not a proof of the lifter.
use super::{EngineProfile, ProfileError, text};
use crate::{
    digest::{Sha256, sha256, to_hex},
    exe::{Arch, Case},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRules {
    pub extractor: [u8; 32],
    pub context: [u8; 32],
    pub mappings: [u8; 32],
    pub summaries: [u8; 32],
}

impl ContractRules {
    /// Recompute active rules from the actual bundled sources. Editing any dependency
    /// invalidates the registered certificates until extraction and review are repeated.
    pub fn builtin() -> Self {
        let mut extractor = Sha256::new();
        for (name, bytes) in [
            (
                "contract_extract.py",
                include_bytes!("../../../../tools/contract_extract.py").as_slice(),
            ),
            (
                "contract_x64.py",
                include_bytes!("../../../../tools/contract_x64.py").as_slice(),
            ),
            (
                "command_contract.py",
                include_bytes!("../../../../tools/command_contract.py").as_slice(),
            ),
            (
                "profile_compare.py",
                include_bytes!("../../../../tools/profile_compare.py").as_slice(),
            ),
        ] {
            extractor.update(name.as_bytes());
            extractor.update(&[0]);
            extractor.update(bytes);
            extractor.update(&[0]);
        }
        Self {
            extractor: extractor.finish(),
            context: sha256(include_bytes!(
                "../../../../docs/games/chronoclock/contract-context.json"
            )),
            mappings: sha256(include_bytes!(
                "../../../../docs/games/chronoclock/contract-mappings.json"
            )),
            summaries: sha256(include_bytes!(
                "../../../../docs/games/chronoclock/contract-summaries.json"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractSide {
    pub profile: String,
    pub binary_sha256: [u8; 32],
    pub image_base: u64,
    pub case_rva: u32,
    pub export_sha256: [u8; 32],
    pub contract_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractProof {
    pub arch: Arch,
    /// Full system opcode, not the zero-based dispatcher index.
    pub opcode: u16,
    pub base: ContractSide,
    pub derived: ContractSide,
    pub rules: ContractRules,
    pub report_sha256: [u8; 32],
    pub mappings_used: Vec<String>,
    pub summaries_used: Vec<String>,
}

const KEYS: &[&str] = &[
    "arch",
    "opcode",
    "base-profile",
    "derived-profile",
    "base-sha256",
    "derived-sha256",
    "base-image",
    "derived-image",
    "base-case",
    "derived-case",
    "base-export",
    "derived-export",
    "extractor",
    "context",
    "mappings",
    "summaries",
    "base-contract",
    "derived-contract",
    "report",
    "mappings-used",
    "summaries-used",
    "model",
    "export-format",
    "result",
    "coverage",
    "binary",
];

impl ContractProof {
    /// The certificate ID hashes this canonical line, without its trailing newline.
    pub fn id(&self) -> [u8; 32] {
        sha256(self.to_line().as_bytes())
    }

    pub fn to_line(&self) -> String {
        let deps = |names: &[String]| {
            if names.is_empty() {
                "-".into()
            } else {
                names.join(",")
            }
        };
        format!(
            "proof v1 arch={} opcode=0x{:x} base-profile={} derived-profile={} base-sha256={} derived-sha256={} base-image=0x{:x} derived-image=0x{:x} base-case=0x{:x} derived-case=0x{:x} base-export={} derived-export={} extractor={} context={} mappings={} summaries={} base-contract={} derived-contract={} report={} mappings-used={} summaries-used={} model=cmvs-command-contract-v1 export-format=cmvs-command-graph-v3 result=equivalent coverage=closed binary=checked",
            self.arch.name(),
            self.opcode,
            self.base.profile,
            self.derived.profile,
            to_hex(&self.base.binary_sha256),
            to_hex(&self.derived.binary_sha256),
            self.base.image_base,
            self.derived.image_base,
            self.base.case_rva,
            self.derived.case_rva,
            to_hex(&self.base.export_sha256),
            to_hex(&self.derived.export_sha256),
            to_hex(&self.rules.extractor),
            to_hex(&self.rules.context),
            to_hex(&self.rules.mappings),
            to_hex(&self.rules.summaries),
            to_hex(&self.base.contract_sha256),
            to_hex(&self.derived.contract_sha256),
            to_hex(&self.report_sha256),
            deps(&self.mappings_used),
            deps(&self.summaries_used),
        )
    }

    fn parse(line: usize, words: &[&str]) -> Result<Self, ProfileError> {
        if words.get(1) != Some(&"v1") {
            return Err(text::at(line, "unknown contract proof version"));
        }
        let v = text::fields(line, &words[2..], KEYS)?;
        let deps = |value: &str| -> Result<Vec<String>, ProfileError> {
            if value == "-" {
                return Ok(Vec::new());
            }
            let names: Vec<_> = value.split(',').map(str::to_owned).collect();
            if names.iter().any(|n| {
                n.is_empty()
                    || !n
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            }) || names.windows(2).any(|w| w[0] >= w[1])
            {
                return Err(text::at(
                    line,
                    "contract dependencies must be unique sorted identifiers",
                ));
            }
            Ok(names)
        };
        if v[0] != "x64"
            || v[21..]
                != [
                    "cmvs-command-contract-v1",
                    "cmvs-command-graph-v3",
                    "equivalent",
                    "closed",
                    "checked",
                ]
        {
            return Err(text::at(
                line,
                "unsupported or incomplete contract evidence",
            ));
        }
        let opcode = u16::try_from(text::hex_number(line, v[1])?)
            .ok()
            .filter(|n| (0x2000..0x2800).contains(n))
            .ok_or_else(|| text::at(line, "contract opcode outside dispatcher domain"))?;
        let proof = Self {
            arch: Arch::X64,
            opcode,
            base: ContractSide {
                profile: text::identifier(line, v[2])?,
                binary_sha256: text::sha(line, v[4])?,
                image_base: text::hex_number(line, v[6])?,
                case_rva: text::hex32(line, v[8])?,
                export_sha256: text::sha(line, v[10])?,
                contract_sha256: text::sha(line, v[16])?,
            },
            derived: ContractSide {
                profile: text::identifier(line, v[3])?,
                binary_sha256: text::sha(line, v[5])?,
                image_base: text::hex_number(line, v[7])?,
                case_rva: text::hex32(line, v[9])?,
                export_sha256: text::sha(line, v[11])?,
                contract_sha256: text::sha(line, v[17])?,
            },
            rules: ContractRules {
                extractor: text::sha(line, v[12])?,
                context: text::sha(line, v[13])?,
                mappings: text::sha(line, v[14])?,
                summaries: text::sha(line, v[15])?,
            },
            report_sha256: text::sha(line, v[18])?,
            mappings_used: deps(v[19])?,
            summaries_used: deps(v[20])?,
        };
        if proof.base.contract_sha256 != proof.derived.contract_sha256
            || proof.base.binary_sha256 == proof.derived.binary_sha256
            || proof.base.profile == proof.derived.profile
            || proof.to_line() != words.join(" ")
        {
            return Err(text::at(
                line,
                "contract certificate is unequal or noncanonical",
            ));
        }
        Ok(proof)
    }

    pub(super) fn bind(
        &self,
        command: u16,
        base: &EngineProfile,
        derived: &EngineProfile,
    ) -> Result<(), ProfileError> {
        if self.opcode != command + 0x2000 {
            return Err(ProfileError::new(
                "contract opcode differs from profile row",
            ));
        }
        for (side, profile) in [(&self.base, base), (&self.derived, derived)] {
            let valid = profile.id == side.profile
                && profile.binaries.iter().any(|b| {
                    b.arch == self.arch
                        && b.sha256 == side.binary_sha256
                        && b.image_base == side.image_base
                        && b.cases.get(command as usize) == Some(&Case::Handler(side.case_rva))
                });
            if !valid {
                return Err(ProfileError::new(
                    "contract identity differs from profile binary/case",
                ));
            }
        }
        Ok(())
    }
}

pub(super) fn parse_catalog(
    text: &str,
    active: &[ContractRules],
) -> Result<BTreeMap<[u8; 32], ContractProof>, ProfileError> {
    let mut proofs = BTreeMap::new();
    let mut revoked = BTreeSet::new();
    for (line, words) in text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.split_whitespace().collect::<Vec<_>>()))
    {
        if words.is_empty() || words[0].starts_with('#') {
            continue;
        }
        match words[0] {
            "proof" => {
                let proof = ContractProof::parse(line, &words)?;
                if proofs.insert(proof.id(), proof).is_some() {
                    return Err(text::at(line, "duplicate contract certificate"));
                }
            }
            "revoked" if words.len() == 2 => {
                if !revoked.insert(text::sha(line, words[1])?) {
                    return Err(text::at(line, "duplicate contract revocation"));
                }
            }
            _ => return Err(text::at(line, "unknown contract catalog entry")),
        }
    }
    proofs.retain(|id, _| !revoked.contains(id));
    if proofs.values().any(|proof| !active.contains(&proof.rules)) {
        return Err(ProfileError::new("contract rules are stale or unknown"));
    }
    Ok(proofs)
}
