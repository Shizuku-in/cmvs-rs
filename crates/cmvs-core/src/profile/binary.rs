//! Wire codec for the validated playback registry.
use super::{
    Binary, CommandStatus, EngineProfile, Family, GameRecord, ProfileError, RecordFile, VariantId,
};
use crate::exe::Arch;

const MAGIC: [u8; 5] = *b"CMVB\x02";

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

    fn write_profile(&mut self, profile: &EngineProfile) {
        self.write_str(&profile.id);
        self.write_u8(encode_family(profile.family));
        match &profile.base {
            Some(base) => {
                self.write_u8(1);
                self.write_str(base);
            }
            None => self.write_u8(0),
        }

        self.write_usize(profile.binaries.len());
        for binary in &profile.binaries {
            self.write_u8(encode_arch(binary.arch));
            self.write_str(&binary.file);
            self.write_hash(&binary.sha256);
        }

        self.write_usize(profile.statuses.len());
        for &status in &profile.statuses {
            self.write_u8(encode_status(status));
        }

        self.write_usize(profile.variant.len() + profile.variant_ids.len());
        for (cmd, id) in profile
            .variant_bindings()
            .map(|(command, binding)| (command, binding.id))
            .chain(profile.variant_ids.iter().copied())
        {
            self.write_u64(u64::from(cmd));
            self.write_str(id.name());
        }

        self.write_usize(profile.maps.len());
        for &(cmd, target) in &profile.maps {
            self.write_u64(u64::from(cmd));
            self.write_u64(u64::from(target));
        }
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
}

fn push_sorted<T>(entries: &mut Vec<(u16, T)>, cmd: u16, value: T) -> Result<(), ProfileError> {
    if entries.last().is_some_and(|&(prev, _)| cmd <= prev) {
        return Err(ProfileError::new(
            "binary command entries must be strictly increasing",
        ));
    }
    entries.push((cmd, value));
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
            timestamp: 0,
            linker: (0, 0),
            image_base: 0,
            table_rva: 0,
            default_rva: 0,
            cases: Vec::new(),
        });
    }

    let cmd_count = dec.read_usize()?;
    let statuses = dec
        .read_bytes(cmd_count)?
        .iter()
        .copied()
        .map(decode_status)
        .collect::<Result<Vec<_>, _>>()?;

    let variant_count = dec.read_usize()?;
    let mut variant_ids = Vec::with_capacity(variant_count.min(dec.0.len()));
    for _ in 0..variant_count {
        let cmd = dec.read_u16()?;
        let name = dec.read_str()?;
        let id = VariantId::parse(&name)
            .ok_or_else(|| ProfileError::new("unknown binary variant id"))?;
        push_sorted(&mut variant_ids, cmd, id)?;
    }

    let map_count = dec.read_usize()?;
    let mut maps = Vec::with_capacity(map_count.min(dec.0.len()));
    for _ in 0..map_count {
        push_sorted(&mut maps, dec.read_u16()?, dec.read_u16()?)?;
    }

    Ok(EngineProfile {
        id,
        family,
        base,
        binaries,
        statuses,
        proofs: Vec::new(),
        contracts: Vec::new(),
        reviews: Vec::new(),
        variant: Vec::new(),
        variant_ids,
        maps,
    })
}

#[allow(dead_code)]
pub(crate) fn encode_bundle(profiles: &[EngineProfile], games: &[GameRecord]) -> Vec<u8> {
    let mut enc = Encoder::new();

    enc.write_usize(profiles.len());
    for profile in profiles {
        enc.write_profile(profile);
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

    enc.0
}

pub(super) fn decode_bundle(
    bytes: &[u8],
) -> Result<(Vec<EngineProfile>, Vec<GameRecord>), ProfileError> {
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

    if !dec.0.is_empty() {
        return Err(ProfileError::new("trailing bytes in binary registry"));
    }

    Ok((profiles, games))
}
