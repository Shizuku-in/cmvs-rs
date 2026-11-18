//! Compile-time playback data projected from the validated research registry.
use super::{
    Binary, CommandStatus, EngineProfile, Family, GameRecord, RecordFile, Registry, VariantId,
};
use crate::exe::Arch;

mod data;

struct FileData {
    path: &'static str,
    sha256: [u8; 32],
}

struct BinaryData(Arch, FileData);

struct ProfileData {
    id: &'static str,
    family: Family,
    base: Option<&'static str>,
    binaries: &'static [BinaryData],
    statuses: &'static [CommandStatus],
    variants: &'static [(u16, VariantId)],
    maps: &'static [(u16, u16)],
}

struct GameData {
    game: &'static str,
    profile: &'static str,
    start_scripts: &'static [FileData],
    script_archive: FileData,
    game_id: u32,
    executables: &'static [FileData],
    skip_auto_resume: Option<u32>,
}

const fn hex_digit(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => panic!("playback hash is not lowercase hexadecimal"),
    }
}

const fn file(path: &'static str, hex: &str) -> FileData {
    let hex = hex.as_bytes();
    assert!(hex.len() == 64, "playback hash must be SHA-256");
    let mut sha256 = [0; 32];
    let mut i = 0;
    while i < sha256.len() {
        sha256[i] = hex_digit(hex[i * 2]) << 4 | hex_digit(hex[i * 2 + 1]);
        i += 1;
    }
    FileData { path, sha256 }
}

const fn statuses<const N: usize>(codes: &[u8; N]) -> [CommandStatus; N] {
    use CommandStatus::*;
    let mut out = [Absent; N];
    let mut i = 0;
    while i < N {
        out[i] = match codes[i] {
            b'a' => Absent,
            b'n' => NoOp,
            b'b' => Base,
            b'i' => Inherited,
            b'r' => Reviewed,
            b'c' => ContractEquivalent,
            b'p' => Presumed,
            b'u' => Unreviewed,
            _ => panic!("unknown playback command status"),
        };
        i += 1;
    }
    out
}

impl FileData {
    fn owned(&self) -> RecordFile {
        RecordFile {
            path: self.path.to_owned(),
            sha256: self.sha256,
        }
    }
}

impl ProfileData {
    fn owned(&self) -> EngineProfile {
        EngineProfile {
            id: self.id.to_owned(),
            family: self.family,
            base: self.base.map(str::to_owned),
            binaries: self
                .binaries
                .iter()
                .map(|BinaryData(arch, file)| Binary {
                    arch: *arch,
                    file: file.path.to_owned(),
                    sha256: file.sha256,
                })
                .collect(),
            statuses: self.statuses.to_vec(),
            variant_ids: self.variants.to_vec(),
            maps: self.maps.to_vec(),
        }
    }
}

impl GameData {
    fn owned(&self) -> GameRecord {
        GameRecord {
            game: self.game.to_owned(),
            profile: self.profile.to_owned(),
            start_scripts: self.start_scripts.iter().map(FileData::owned).collect(),
            script_archive: self.script_archive.owned(),
            game_id: self.game_id,
            executables: self.executables.iter().map(FileData::owned).collect(),
            skip_auto_resume: self.skip_auto_resume,
        }
    }
}

pub(super) fn registry() -> Registry {
    Registry::new(
        data::PROFILES.iter().map(ProfileData::owned).collect(),
        data::GAMES.iter().map(GameData::owned).collect(),
    )
}
