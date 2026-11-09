//! Inverse index/resource transforms; readers retain the decoding implementations.
use super::super::{ArchiveIndex, Cpz5Mode, Cpz7Parameters, crypto};

pub(super) enum Cipher {
    Cpz5(Cpz5Mode, crypto::Keys),
    Cpz6(crypto::Keys),
    Cpz7(Cpz7Parameters, crypto::Keys),
}
impl Cipher {
    pub fn new(index: &ArchiveIndex, header: &[u8]) -> Self {
        match index {
            ArchiveIndex::Cpz5(index) => Self::Cpz5(index.mode, index.mode.keys(header)),
            ArchiveIndex::Cpz6(_) => Self::Cpz6(crypto::Keys::from_header(header)),
            ArchiveIndex::Cpz7(index) => {
                let mut keys = crypto::Keys::from_header_v7(header);
                keys.seed = keys.seed.wrapping_add(index.parameters.seed_add);
                Self::Cpz7(index.parameters, keys)
            }
            ArchiveIndex::Cpz3(_) => unreachable!("CPZ3 has a separate flat-index writer"),
        }
    }
    fn keys(&self) -> &crypto::Keys {
        match self {
            Self::Cpz5(_, keys) | Self::Cpz6(keys) | Self::Cpz7(_, keys) => keys,
        }
    }
    fn table(&self, key: u32, seed: u32) -> [u8; 256] {
        match self {
            Self::Cpz5(mode, _) => mode.table(key, seed),
            _ => crypto::table(key, seed),
        }
    }
    fn phrase(&self) -> &[u8; 96] {
        match self {
            Self::Cpz5(mode, _) => mode.table_bytes(),
            _ => &crypto::TABLE_BYTES,
        }
    }
    fn directory_step(&self) -> u32 {
        match self {
            Self::Cpz7(p, _) => p.directory_step_xor ^ 0x10fb562a,
            _ => 0x10fb562a,
        }
    }
    fn file_step(&self) -> u32 {
        match self {
            Self::Cpz7(p, _) => p.file_step_xor ^ 0x0139fa9b,
            _ => 0x0139fa9b,
        }
    }
    fn inverse_substitute(&self, bytes: &mut [u8], seed: u32, xor: u8) {
        let inverse = inverse(self.table(self.keys().key, seed));
        for byte in bytes {
            *byte = inverse[*byte as usize] ^ xor;
        }
    }
    pub fn decode_outer(&self, bytes: &mut [u8]) {
        match self {
            Self::Cpz5(mode, keys) => mode.outer_index(bytes, keys.key),
            Self::Cpz6(keys) => crypto::outer_index(bytes, keys.key),
            Self::Cpz7(_, keys) => crypto::outer_index_v7(bytes, keys.key),
        }
    }
    pub fn decode_directories(&self, bytes: &mut [u8]) {
        match self {
            Self::Cpz5(mode, keys) => mode.directory_index(bytes, keys),
            _ => crypto::directory_index_with_step(bytes, self.keys(), self.directory_step()),
        }
    }
    pub fn decode_files(&self, bytes: &mut [u8], directory: u32) {
        match self {
            Self::Cpz5(mode, keys) => mode.file_index(bytes, directory, keys),
            _ => crypto::file_index_with_step(bytes, directory, self.keys(), self.file_step()),
        }
    }
    pub fn encode_directories(&self, bytes: &mut [u8]) {
        let keys = self.keys();
        let key = keys.directory_key();
        let mut seed = 0x76548aefu32;
        let mut flag = 0;
        let (words, tail) = bytes.as_chunks_mut::<4>();
        for word in words {
            *word = (u32::from_le_bytes(*word)
                .wrapping_add(seed)
                .rotate_right(3)
                .wrapping_add(0x4a91c262)
                ^ key[flag])
                .to_le_bytes();
            seed = seed.wrapping_add(self.directory_step());
            flag = (flag + 1) & 3;
        }
        for byte in tail {
            *byte = byte.wrapping_sub(0x37) ^ (key[flag] >> 6) as u8;
            flag = (flag + 1) & 3;
        }
        self.inverse_substitute(bytes, keys.digest[1], 0x3a);
    }
    pub fn encode_files(&self, bytes: &mut [u8], directory: u32) {
        let keys = self.keys();
        let early = matches!(self, Self::Cpz5(..));
        let key = if early {
            [
                directory ^ keys.digest[0],
                directory.wrapping_add(0x00112233) ^ keys.digest[1],
                directory ^ keys.digest[2],
                directory.wrapping_add(0x34258765) ^ keys.digest[3],
            ]
        } else {
            keys.file_key(directory)
        };
        let mut seed: u32 = if early { 0x2a65cb4e } else { 0x2a65cb4f };
        let mut flag = 0;
        let (words, tail) = bytes.as_chunks_mut::<4>();
        for word in words {
            *word = (u32::from_le_bytes(*word)
                .wrapping_sub(0x37a19e8b)
                .rotate_right(2)
                .wrapping_add(seed)
                ^ key[flag])
                .to_le_bytes();
            seed = seed.wrapping_sub(self.file_step());
            flag = (flag + 1) & 3;
        }
        for byte in tail {
            *byte = byte.wrapping_sub(if early { 5 } else { 3 }) ^ (key[flag] >> 4) as u8;
            flag = (flag + 1) & 3;
        }
        self.inverse_substitute(bytes, keys.digest[2], 0x7e);
    }
    pub fn encode_outer(&self, bytes: &mut [u8]) {
        let key = self.keys().key ^ 0x3795b39a;
        let table: [u32; 24] = std::array::from_fn(|i| {
            u32::from_le_bytes(self.phrase()[i * 4..i * 4 + 4].try_into().unwrap())
                .wrapping_sub(key)
        });
        let shift = ((key ^ (key >> 8) ^ (key >> 16) ^ (key >> 24) ^ 0xfffffffb) & 15) + 7;
        let add = if matches!(self, Self::Cpz5(..)) {
            0x784c5962
        } else {
            0x784c5062
        };
        let mut flag = 5;
        let (words, tail) = bytes.as_chunks_mut::<4>();
        for word in words {
            *word = (u32::from_le_bytes(*word)
                .wrapping_sub(0x01010101)
                .rotate_left(shift)
                .wrapping_sub(add)
                ^ table[flag])
                .to_le_bytes();
            flag = (flag + 1) % 24;
        }
        let sub = match self {
            Self::Cpz5(Cpz5Mode::Mode9, _) => 0x73,
            Self::Cpz5(..) => 0x79,
            _ => 0x7d,
        };
        let length = tail.len();
        for (i, byte) in tail.iter_mut().enumerate() {
            let shift = if matches!(self, Self::Cpz6(_)) {
                i
            } else {
                length - i
            };
            *byte = byte.wrapping_add(sub) ^ (table[flag] >> (4 * shift)) as u8;
            flag = (flag + 1) % 24;
        }
    }
    pub fn encode_resource(&self, bytes: &mut [u8], directory: u32, file: u32, directories: u32) {
        let keys = self.keys();
        let resource = (keys.key ^ directory.wrapping_add(file)).wrapping_add(directories);
        let seed = match self {
            Self::Cpz5(..) => resource.wrapping_add(0xa3d61785),
            Self::Cpz6(_) => keys.seed ^ resource.wrapping_add(0xa3d61785),
            Self::Cpz7(p, _) => {
                keys.seed ^ (resource ^ p.resource_seed_xor).wrapping_add(0xa3c61785)
            }
        };
        let table = self.table(keys.digest[3], keys.key);
        let key_bytes: [u8; 96] =
            std::array::from_fn(|i| (keys.digest[1] >> 2) as u8 ^ table[self.phrase()[i] as usize]);
        let decrypt: [u32; 24] = std::array::from_fn(|i| {
            u32::from_le_bytes(key_bytes[i * 4..i * 4 + 4].try_into().unwrap()) ^ seed
        });
        let mut key: u32 = match self {
            Self::Cpz5(Cpz5Mode::Mode9, _) => 0x2547b39e,
            Self::Cpz5(..) => 0x2547a39e,
            _ => 0x2748c39e,
        };
        let mut flag = if matches!(self, Self::Cpz5(..)) {
            9
        } else {
            10
        };
        let (words, tail) = bytes.as_chunks_mut::<4>();
        for word in words {
            let value = u32::from_le_bytes(*word);
            *word = ((value ^ keys.digest[(key & 3) as usize]).wrapping_add(seed)
                ^ (decrypt[flag] >> 1)
                ^ decrypt[((key >> 6) & 15) as usize])
                .to_le_bytes();
            key = key.wrapping_add(seed).wrapping_add(value);
            flag = (flag + 1) & 15;
        }
        let xor = match self {
            Self::Cpz5(Cpz5Mode::Mode9, _) => 0xcb,
            Self::Cpz5(..) => 0xbc,
            _ => 0xae,
        };
        let inverse = inverse(table);
        for byte in tail {
            *byte = inverse[*byte as usize] ^ xor;
        }
    }
}
fn inverse(table: [u8; 256]) -> [u8; 256] {
    let mut result = [0; 256];
    for (i, byte) in table.into_iter().enumerate() {
        result[byte as usize] = i as u8;
    }
    result
}
