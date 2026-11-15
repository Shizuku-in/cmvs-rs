//! Compact binary wire codec for the built-in profile registry bundle.
//!
//! Eliminates text tokenization, ASCII hex expansion, and redundant per-row fields
//! before [`crate::lzss`] compression, while preserving exact equality with [`super::Registry::parse_with_contracts`].
use super::{
    Binary, CommandStatus, ContractProof, ContractRules, ContractSide, EngineProfile, Family,
    GameRecord, MachineProof, ProfileError, RecordFile, VariantBinding, VariantId,
};
use crate::exe::{Arch, Case};
use std::collections::BTreeMap;

const MAGIC: [u8; 5] = *b"CMVB\x01";

pub(super) type Bundle = (
    Vec<EngineProfile>,
    Vec<GameRecord>,
    BTreeMap<[u8; 32], ContractProof>,
);

fn zigzag_encode(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

fn zigzag_decode(value: u64) -> i64 {
    ((value >> 1) as i64) ^ (-((value & 1) as i64))
}

fn encode_family(family: Family) -> u8 {
    match family {
        Family::Cmvs => 0,
        Family::Cvns => 1,
    }
}

fn decode_family(tag: u8) -> Result<Family, ProfileError> {
    match tag {
        0 => Ok(Family::Cmvs),
        1 => Ok(Family::Cvns),
        _ => Err(ProfileError::new("invalid binary family tag")),
    }
}

fn encode_arch(arch: Arch) -> u8 {
    match arch {
        Arch::X86 => 0,
        Arch::X64 => 1,
    }
}

fn decode_arch(tag: u8) -> Result<Arch, ProfileError> {
    match tag {
        0 => Ok(Arch::X86),
        1 => Ok(Arch::X64),
        _ => Err(ProfileError::new("invalid binary architecture tag")),
    }
}

fn encode_status(status: CommandStatus) -> u8 {
    match status {
        CommandStatus::Absent => 0,
        CommandStatus::NoOp => 1,
        CommandStatus::Base => 2,
        CommandStatus::Inherited => 3,
        CommandStatus::Reviewed => 4,
        CommandStatus::ContractEquivalent => 5,
        CommandStatus::Presumed => 6,
        CommandStatus::Unreviewed => 7,
    }
}

fn decode_status(tag: u8) -> Result<CommandStatus, ProfileError> {
    match tag {
        0 => Ok(CommandStatus::Absent),
        1 => Ok(CommandStatus::NoOp),
        2 => Ok(CommandStatus::Base),
        3 => Ok(CommandStatus::Inherited),
        4 => Ok(CommandStatus::Reviewed),
        5 => Ok(CommandStatus::ContractEquivalent),
        6 => Ok(CommandStatus::Presumed),
        7 => Ok(CommandStatus::Unreviewed),
        _ => Err(ProfileError::new("invalid binary command status tag")),
    }
}

#[allow(dead_code)]
struct Encoder(Vec<u8>);

#[allow(dead_code)]
impl Encoder {
    fn new() -> Self {
        Self(MAGIC.to_vec())
    }

    fn write_u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn write_u32_le(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u64(&mut self, mut value: u64) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            self.0.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_i64(&mut self, value: i64) {
        self.write_u64(zigzag_encode(value));
    }

    fn write_hash(&mut self, hash: &[u8; 32]) {
        self.0.extend_from_slice(hash);
    }

    fn write_str(&mut self, text: &str) {
        self.write_usize(text.len());
        self.0.extend_from_slice(text.as_bytes());
    }

    fn write_record_file(&mut self, file: &RecordFile) {
        self.write_str(&file.path);
        self.write_hash(&file.sha256);
    }

    fn write_contract_side(&mut self, side: &ContractSide) {
        self.write_str(&side.profile);
        self.write_hash(&side.binary_sha256);
        self.write_u64(side.image_base);
        self.write_u64(u64::from(side.case_rva));
        self.write_hash(&side.export_sha256);
        self.write_hash(&side.contract_sha256);
    }
}

struct Decoder<'a>(&'a [u8]);

impl<'a> Decoder<'a> {
    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], ProfileError> {
        let (head, tail) = self
            .0
            .split_at_checked(len)
            .ok_or_else(|| ProfileError::new("truncated binary registry"))?;
        self.0 = tail;
        Ok(head)
    }

    fn read_u8(&mut self) -> Result<u8, ProfileError> {
        Ok(self.read_bytes(1)?[0])
    }

    fn read_u32_le(&mut self) -> Result<u32, ProfileError> {
        let bytes: [u8; 4] = self.read_bytes(4)?.try_into().expect("four bytes checked");
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64(&mut self) -> Result<u64, ProfileError> {
        let mut out = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.read_u8()?;
            if shift == 63 && byte > 1 {
                return Err(ProfileError::new("binary varint overflow"));
            }
            out |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(out);
            }
            shift += 7;
        }
    }

    fn read_usize(&mut self) -> Result<usize, ProfileError> {
        usize::try_from(self.read_u64()?).map_err(|_| ProfileError::new("binary length overflow"))
    }

    fn read_u16(&mut self) -> Result<u16, ProfileError> {
        u16::try_from(self.read_u64()?).map_err(|_| ProfileError::new("binary u16 overflow"))
    }

    fn read_u32(&mut self) -> Result<u32, ProfileError> {
        u32::try_from(self.read_u64()?).map_err(|_| ProfileError::new("binary u32 overflow"))
    }

    fn read_i64(&mut self) -> Result<i64, ProfileError> {
        Ok(zigzag_decode(self.read_u64()?))
    }

    fn read_hash(&mut self) -> Result<[u8; 32], ProfileError> {
        Ok(self.read_bytes(32)?.try_into().expect("32 bytes checked"))
    }

    fn read_str(&mut self) -> Result<String, ProfileError> {
        let len = self.read_usize()?;
        let bytes = self.read_bytes(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| ProfileError::new("invalid UTF-8 in binary registry"))
    }

    fn read_record_file(&mut self) -> Result<RecordFile, ProfileError> {
        Ok(RecordFile {
            path: self.read_str()?,
            sha256: self.read_hash()?,
        })
    }

    fn read_contract_side(&mut self) -> Result<ContractSide, ProfileError> {
        Ok(ContractSide {
            profile: self.read_str()?,
            binary_sha256: self.read_hash()?,
            image_base: self.read_u64()?,
            case_rva: self.read_u32()?,
            export_sha256: self.read_hash()?,
            contract_sha256: self.read_hash()?,
        })
    }
}

#[allow(dead_code)]
fn encode_profile(enc: &mut Encoder, profile: &EngineProfile) -> Result<(), ProfileError> {
    enc.write_str(&profile.id);
    enc.write_u8(encode_family(profile.family));
    match &profile.base {
        Some(base) => {
            enc.write_u8(1);
            enc.write_str(base);
        }
        None => enc.write_u8(0),
    }

    enc.write_usize(profile.binaries.len());
    for binary in &profile.binaries {
        enc.write_u8(encode_arch(binary.arch));
        enc.write_str(&binary.file);
        enc.write_hash(&binary.sha256);
        enc.write_u32_le(binary.timestamp);
        enc.write_u8(binary.linker.0);
        enc.write_u8(binary.linker.1);
        enc.write_u64(binary.image_base);
        enc.write_u64(u64::from(binary.table_rva));
        enc.write_u64(u64::from(binary.default_rva));
    }

    let cmd_count = profile.statuses.len();
    enc.write_usize(cmd_count);
    let mut prev_handler = vec![0u32; profile.binaries.len()];
    let mut prev_noop = vec![0u32; profile.binaries.len()];
    for (cmd, &status) in profile.statuses.iter().enumerate() {
        enc.write_u8(encode_status(status));
        for (idx, binary) in profile.binaries.iter().enumerate() {
            let case = *binary
                .cases
                .get(cmd)
                .ok_or_else(|| ProfileError::new("binary case count mismatch"))?;
            match (status, case) {
                (CommandStatus::Absent, Case::Default) => {}
                (CommandStatus::NoOp, Case::NoOp(rva)) => {
                    enc.write_i64(i64::from(rva) - i64::from(prev_noop[idx]));
                    prev_noop[idx] = rva;
                }
                (
                    CommandStatus::Base
                    | CommandStatus::Inherited
                    | CommandStatus::Reviewed
                    | CommandStatus::ContractEquivalent
                    | CommandStatus::Presumed
                    | CommandStatus::Unreviewed,
                    Case::Handler(rva),
                ) => {
                    enc.write_i64(i64::from(rva) - i64::from(prev_handler[idx]));
                    prev_handler[idx] = rva;
                }
                _ => {
                    return Err(ProfileError::new(
                        "command status contradicts binary case in encoder",
                    ));
                }
            }
        }
    }

    // Deduplicate (arch, base_sha256, derived_sha256) pairs across proofs in one profile.
    let mut proof_bases: Vec<(Arch, [u8; 32], [u8; 32])> = Vec::new();
    for proof in profile.proofs.values() {
        let key = (proof.arch, proof.base_sha256, proof.derived_sha256);
        if !proof_bases.contains(&key) {
            proof_bases.push(key);
        }
    }
    enc.write_usize(proof_bases.len());
    for &(arch, base_sha, derived_sha) in &proof_bases {
        enc.write_u8(encode_arch(arch));
        enc.write_hash(&base_sha);
        enc.write_hash(&derived_sha);
    }
    enc.write_usize(profile.proofs.len());
    for (&cmd, proof) in &profile.proofs {
        let base_idx = proof_bases
            .iter()
            .position(|&k| k == (proof.arch, proof.base_sha256, proof.derived_sha256))
            .expect("inserted above");
        enc.write_u64(u64::from(cmd));
        enc.write_usize(base_idx);
        enc.write_hash(&proof.digest);
    }

    enc.write_usize(profile.contracts.len());
    for (&cmd, id) in &profile.contracts {
        enc.write_u64(u64::from(cmd));
        enc.write_hash(id);
    }

    // Deduplicate review paths within each profile.
    let mut review_paths: Vec<&str> = Vec::new();
    for path in profile.reviews.values() {
        if !review_paths.contains(&path.as_str()) {
            review_paths.push(path);
        }
    }
    enc.write_usize(review_paths.len());
    for &path in &review_paths {
        enc.write_str(path);
    }
    enc.write_usize(profile.reviews.len());
    for (&cmd, path) in &profile.reviews {
        let path_idx = review_paths
            .iter()
            .position(|&p| p == path)
            .expect("inserted above");
        enc.write_u64(u64::from(cmd));
        enc.write_usize(path_idx);
    }

    enc.write_usize(profile.variant.len());
    for (&cmd, v) in &profile.variant {
        enc.write_u64(u64::from(cmd));
        enc.write_str(v.id.name());
        enc.write_u8(encode_arch(v.arch));
        enc.write_hash(&v.binary_sha256);
        enc.write_str(&v.evidence);
    }

    enc.write_usize(profile.maps.len());
    for (&cmd, &target) in &profile.maps {
        enc.write_u64(u64::from(cmd));
        enc.write_u64(u64::from(target));
    }

    Ok(())
}

fn decode_profile(dec: &mut Decoder<'_>) -> Result<EngineProfile, ProfileError> {
    let id = dec.read_str()?;
    let family = decode_family(dec.read_u8()?)?;
    let base = match dec.read_u8()? {
        0 => None,
        1 => Some(dec.read_str()?),
        _ => return Err(ProfileError::new("invalid binary base flag")),
    };

    let binary_count = dec.read_usize()?;
    let mut binaries = Vec::with_capacity(binary_count);
    for _ in 0..binary_count {
        binaries.push(Binary {
            arch: decode_arch(dec.read_u8()?)?,
            file: dec.read_str()?,
            sha256: dec.read_hash()?,
            timestamp: dec.read_u32_le()?,
            linker: (dec.read_u8()?, dec.read_u8()?),
            image_base: dec.read_u64()?,
            table_rva: dec.read_u32()?,
            default_rva: dec.read_u32()?,
            cases: Vec::new(),
        });
    }

    let cmd_count = dec.read_usize()?;
    for binary in &mut binaries {
        binary.cases.reserve_exact(cmd_count);
    }
    let mut statuses = Vec::with_capacity(cmd_count);
    let mut prev_handler = vec![0u32; binary_count];
    let mut prev_noop = vec![0u32; binary_count];
    for _ in 0..cmd_count {
        let status = decode_status(dec.read_u8()?)?;
        statuses.push(status);
        for (idx, binary) in binaries.iter_mut().enumerate() {
            let case = match status {
                CommandStatus::Absent => Case::Default,
                CommandStatus::NoOp => {
                    let rva = u32::try_from(i64::from(prev_noop[idx]) + dec.read_i64()?)
                        .map_err(|_| ProfileError::new("binary noop rva out of range"))?;
                    prev_noop[idx] = rva;
                    Case::NoOp(rva)
                }
                _ => {
                    let rva = u32::try_from(i64::from(prev_handler[idx]) + dec.read_i64()?)
                        .map_err(|_| ProfileError::new("binary handler rva out of range"))?;
                    prev_handler[idx] = rva;
                    Case::Handler(rva)
                }
            };
            binary.cases.push(case);
        }
    }

    let base_count = dec.read_usize()?;
    let mut proof_bases = Vec::with_capacity(base_count);
    for _ in 0..base_count {
        proof_bases.push((
            decode_arch(dec.read_u8()?)?,
            dec.read_hash()?,
            dec.read_hash()?,
        ));
    }
    let proof_count = dec.read_usize()?;
    let mut proofs = BTreeMap::new();
    for _ in 0..proof_count {
        let cmd = dec.read_u16()?;
        let base_idx = dec.read_usize()?;
        let &(arch, base_sha256, derived_sha256) = proof_bases
            .get(base_idx)
            .ok_or_else(|| ProfileError::new("binary proof base index out of bounds"))?;
        proofs.insert(
            cmd,
            MachineProof {
                arch,
                base_sha256,
                derived_sha256,
                digest: dec.read_hash()?,
            },
        );
    }

    let contract_count = dec.read_usize()?;
    let mut contracts = BTreeMap::new();
    for _ in 0..contract_count {
        contracts.insert(dec.read_u16()?, dec.read_hash()?);
    }

    let path_count = dec.read_usize()?;
    let mut review_paths = Vec::with_capacity(path_count);
    for _ in 0..path_count {
        review_paths.push(dec.read_str()?);
    }
    let review_count = dec.read_usize()?;
    let mut reviews = BTreeMap::new();
    for _ in 0..review_count {
        let cmd = dec.read_u16()?;
        let path_idx = dec.read_usize()?;
        let path = review_paths
            .get(path_idx)
            .ok_or_else(|| ProfileError::new("binary review path index out of bounds"))?;
        reviews.insert(cmd, path.clone());
    }

    let variant_count = dec.read_usize()?;
    let mut variant = BTreeMap::new();
    for _ in 0..variant_count {
        let cmd = dec.read_u16()?;
        let name = dec.read_str()?;
        let id = VariantId::parse(&name)
            .ok_or_else(|| ProfileError::new("unknown binary variant id"))?;
        variant.insert(
            cmd,
            VariantBinding {
                id,
                arch: decode_arch(dec.read_u8()?)?,
                binary_sha256: dec.read_hash()?,
                evidence: dec.read_str()?,
            },
        );
    }

    let map_count = dec.read_usize()?;
    let mut maps = BTreeMap::new();
    for _ in 0..map_count {
        maps.insert(dec.read_u16()?, dec.read_u16()?);
    }

    Ok(EngineProfile {
        id,
        family,
        base,
        binaries,
        statuses,
        proofs,
        contracts,
        reviews,
        variant,
        maps,
    })
}

#[allow(dead_code)]
pub(crate) fn encode_bundle(
    profiles: &[EngineProfile],
    games: &[GameRecord],
    contracts: &BTreeMap<[u8; 32], ContractProof>,
) -> Result<Vec<u8>, ProfileError> {
    let mut enc = Encoder::new();

    enc.write_usize(profiles.len());
    for profile in profiles {
        encode_profile(&mut enc, profile)?;
    }

    enc.write_usize(games.len());
    for game in games {
        enc.write_str(&game.game);
        enc.write_str(&game.profile);
        enc.write_usize(game.start_scripts.len());
        for start in &game.start_scripts {
            enc.write_record_file(start);
        }
        enc.write_record_file(&game.script_archive);
        enc.write_u32_le(game.game_id);
        enc.write_usize(game.executables.len());
        for exe in &game.executables {
            enc.write_record_file(exe);
        }
        match game.skip_auto_resume {
            Some(value) => {
                enc.write_u8(1);
                enc.write_u64(u64::from(value));
            }
            None => enc.write_u8(0),
        }
    }

    enc.write_usize(contracts.len());
    for (id, proof) in contracts {
        enc.write_hash(id);
        enc.write_u8(encode_arch(proof.arch));
        enc.write_u64(u64::from(proof.opcode));
        enc.write_contract_side(&proof.base);
        enc.write_contract_side(&proof.derived);
        enc.write_hash(&proof.rules.extractor);
        enc.write_hash(&proof.rules.context);
        enc.write_hash(&proof.rules.mappings);
        enc.write_hash(&proof.rules.summaries);
        enc.write_hash(&proof.report_sha256);
        enc.write_usize(proof.mappings_used.len());
        for dep in &proof.mappings_used {
            enc.write_str(dep);
        }
        enc.write_usize(proof.summaries_used.len());
        for dep in &proof.summaries_used {
            enc.write_str(dep);
        }
    }

    Ok(enc.0)
}

pub(super) fn decode_bundle(bytes: &[u8]) -> Result<Bundle, ProfileError> {
    let mut dec = Decoder(bytes);
    if dec.read_bytes(MAGIC.len())? != MAGIC {
        return Err(ProfileError::new("invalid binary registry magic"));
    }

    let profile_count = dec.read_usize()?;
    let mut profiles = Vec::with_capacity(profile_count);
    for _ in 0..profile_count {
        profiles.push(decode_profile(&mut dec)?);
    }

    let game_count = dec.read_usize()?;
    let mut games = Vec::with_capacity(game_count);
    for _ in 0..game_count {
        let game = dec.read_str()?;
        let profile = dec.read_str()?;
        let start_count = dec.read_usize()?;
        let mut start_scripts = Vec::with_capacity(start_count);
        for _ in 0..start_count {
            start_scripts.push(dec.read_record_file()?);
        }
        let script_archive = dec.read_record_file()?;
        let game_id = dec.read_u32_le()?;
        let exe_count = dec.read_usize()?;
        let mut executables = Vec::with_capacity(exe_count);
        for _ in 0..exe_count {
            executables.push(dec.read_record_file()?);
        }
        let skip_auto_resume = match dec.read_u8()? {
            0 => None,
            1 => Some(dec.read_u32()?),
            _ => return Err(ProfileError::new("invalid binary policy flag")),
        };
        games.push(GameRecord {
            game,
            profile,
            start_scripts,
            script_archive,
            game_id,
            executables,
            skip_auto_resume,
        });
    }

    let contract_count = dec.read_usize()?;
    let mut contracts = BTreeMap::new();
    for _ in 0..contract_count {
        let id = dec.read_hash()?;
        let arch = decode_arch(dec.read_u8()?)?;
        let opcode = dec.read_u16()?;
        let base = dec.read_contract_side()?;
        let derived = dec.read_contract_side()?;
        let rules = ContractRules {
            extractor: dec.read_hash()?,
            context: dec.read_hash()?,
            mappings: dec.read_hash()?,
            summaries: dec.read_hash()?,
        };
        let report_sha256 = dec.read_hash()?;
        let map_count = dec.read_usize()?;
        let mut mappings_used = Vec::with_capacity(map_count);
        for _ in 0..map_count {
            mappings_used.push(dec.read_str()?);
        }
        let sum_count = dec.read_usize()?;
        let mut summaries_used = Vec::with_capacity(sum_count);
        for _ in 0..sum_count {
            summaries_used.push(dec.read_str()?);
        }
        contracts.insert(
            id,
            ContractProof {
                arch,
                opcode,
                base,
                derived,
                rules,
                report_sha256,
                mappings_used,
                summaries_used,
            },
        );
    }

    if !dec.0.is_empty() {
        return Err(ProfileError::new("trailing bytes in binary registry"));
    }

    Ok((profiles, games, contracts))
}
