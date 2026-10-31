//! CPZ6 transforms based on the MIT CMVS-Engine reference; see THIRD_PARTY_NOTICES.
pub(super) const TABLE_BYTES: [u8; 96] = [
    0x89, 0xf0, 0x90, 0xcd, 0x82, 0xb7, 0x82, 0xe9, 0x88, 0xab, 0x82, 0xa2, 0x8e, 0x71, 0x82, 0xcd,
    0x83, 0x8a, 0x83, 0x52, 0x82, 0xaa, 0x82, 0xa8, 0x8e, 0x64, 0x92, 0x75, 0x82, 0xab, 0x82, 0xb5,
    0x82, 0xbf, 0x82, 0xe1, 0x82, 0xa2, 0x82, 0xdc, 0x82, 0xb7, 0x81, 0x42, 0x8e, 0xf4, 0x82, 0xed,
    0x82, 0xea, 0x82, 0xbf, 0x82, 0xe1, 0x82, 0xa2, 0x82, 0xdc, 0x82, 0xb7, 0x82, 0xe6, 0x81, 0x60,
    0x81, 0x41, 0x82, 0xc6, 0x82, 0xa2, 0x82, 0xa4, 0x82, 0xa9, 0x82, 0xe0, 0x82, 0xa4, 0x8e, 0xf4,
    0x82, 0xc1, 0x82, 0xbf, 0x82, 0xe1, 0x82, 0xa2, 0x82, 0xdc, 0x82, 0xb5, 0x82, 0xbd, 0x81, 0xf4,
];

fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub(super) fn digest(input: [u32; 4]) -> [u32; 4] {
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let initial = [0xc74a2b01u32, 0xe7c8ab8f, 0xd8bedc4e, 0x7302a4c5];
    let [mut a, mut b, mut c, mut d] = initial;
    let mut block = [0u32; 16];
    block[..4].copy_from_slice(&input);
    block[4] = 0x80;
    block[14] = 128;
    for (i, k) in K.into_iter().enumerate() {
        let (f, g, shift) = match i {
            0..=15 => (d ^ (b & (c ^ d)), i, [7, 12, 17, 22][i % 4]),
            16..=31 => (c ^ (d & (b ^ c)), (5 * i + 1) % 16, [5, 9, 14, 20][i % 4]),
            32..=47 => (b ^ c ^ d, (3 * i + 5) % 16, [4, 11, 16, 23][i % 4]),
            _ => (c ^ (b | !d), (7 * i) % 16, [6, 10, 15, 21][i % 4]),
        };
        let next = a
            .wrapping_add(f)
            .wrapping_add(k)
            .wrapping_add(block[g])
            .rotate_left(shift)
            .wrapping_add(b);
        a = d;
        d = c;
        c = b;
        b = next;
    }
    let state = [
        a.wrapping_add(initial[0]),
        b.wrapping_add(initial[1]),
        c.wrapping_add(initial[2]),
        d.wrapping_add(initial[3]),
    ];
    [state[3], state[1], state[2], state[0]]
}

#[derive(Debug, Clone)]
pub(super) struct Keys {
    pub key: u32,
    pub seed: u32,
    pub digest: [u32; 4],
}
impl Keys {
    pub fn from_header(header: &[u8]) -> Self {
        let masks = [0x43de7c1a, 0xcc65f416, 0xd016a93d, 0x97a3ba9b];
        let mut input = [0; 4];
        for i in 0..4 {
            input[i] = word(&header[32 + 4 * i..]) ^ masks[i];
        }
        let mut hash = digest(input);
        hash.swap(0, 2);
        hash.swap(2, 3);
        hash[0] ^= 0x45a76c2f;
        hash[1] = hash[1].wrapping_sub(0x5ba17fcb);
        hash[2] ^= 0x79abe8ad;
        hash[3] = hash[3].wrapping_sub(0x1c08561b);
        Self {
            key: word(&header[48..]) ^ 0xae7d39b7,
            seed: (word(&header[56..]) ^ 0x37acf832)
                .rotate_right(5)
                .wrapping_mul(0x7da8f173)
                .wrapping_add(0x13712765),
            digest: hash,
        }
    }
    pub fn directory_key(&self) -> [u32; 4] {
        [
            self.digest[0] ^ self.key.wrapping_add(0x76a3bf29),
            self.key ^ self.digest[1],
            self.digest[2] ^ self.key.wrapping_add(0x10000000),
            self.key ^ self.digest[3],
        ]
    }
    pub fn file_key(&self, directory: u32) -> [u32; 4] {
        [
            directory ^ self.digest[0],
            directory.wrapping_add(0x11003322) ^ self.digest[1],
            directory ^ self.digest[2],
            directory.wrapping_add(0x34216785) ^ self.digest[3],
        ]
    }
}

pub(super) fn table(mut key: u32, seed: u32) -> [u8; 256] {
    let mut table = std::array::from_fn(|i| i as u8);
    for _ in 0..256 {
        table.swap(((key >> 16) & 255) as usize, (key & 255) as usize);
        table.swap(((key >> 8) & 255) as usize, (key >> 24) as usize);
        key = key
            .rotate_right(2)
            .wrapping_mul(0x1a74f195)
            .wrapping_add(seed);
    }
    table
}

pub(super) fn outer_index(bytes: &mut [u8], key: u32) {
    let key = key ^ 0x3795b39a;
    let table: [u32; 24] = std::array::from_fn(|i| word(&TABLE_BYTES[4 * i..]).wrapping_sub(key));
    let shift = ((key ^ (key >> 8) ^ (key >> 16) ^ (key >> 24) ^ 0xfffffffb) & 15) + 7;
    let mut flag = 5;
    for chunk in bytes.as_chunks_mut::<4>().0 {
        let value = (word(chunk) ^ table[flag])
            .wrapping_add(0x784c5062)
            .rotate_right(shift)
            .wrapping_add(0x01010101);
        chunk.copy_from_slice(&value.to_le_bytes());
        flag = (flag + 1) % 24;
    }
    let tail = bytes.len() / 4 * 4;
    for (i, byte) in bytes.iter_mut().enumerate().skip(tail) {
        *byte = (*byte ^ ((table[flag] >> (4 * (i % 4))) as u8)).wrapping_sub(0x7d);
        flag = (flag + 1) % 24;
    }
}

pub(super) fn substitute(bytes: &mut [u8], key: u32, seed: u32, xor: u8) {
    let table = table(key, seed);
    for byte in bytes {
        *byte = table[(*byte ^ xor) as usize];
    }
}

pub(super) fn directory_index(bytes: &mut [u8], keys: &Keys) {
    substitute(bytes, keys.key, keys.digest[1], 0x3a);
    let key = keys.directory_key();
    let mut seed = 0x76548aefu32;
    let mut flag = 0;
    for chunk in bytes.as_chunks_mut::<4>().0 {
        let value = (word(chunk) ^ key[flag])
            .wrapping_sub(0x4a91c262)
            .rotate_left(3)
            .wrapping_sub(seed);
        chunk.copy_from_slice(&value.to_le_bytes());
        seed = seed.wrapping_add(0x10fb562a);
        flag = (flag + 1) & 3;
    }
    let tail = bytes.len() / 4 * 4;
    for byte in &mut bytes[tail..] {
        *byte = (*byte ^ ((key[flag] >> 6) as u8)).wrapping_add(0x37);
        flag = (flag + 1) & 3;
    }
}

pub(super) fn file_index(bytes: &mut [u8], directory: u32, keys: &Keys) {
    substitute(bytes, keys.key, keys.digest[2], 0x7e);
    let key = keys.file_key(directory);
    let mut seed = 0x2a65cb4fu32;
    let mut flag = 0;
    for chunk in bytes.as_chunks_mut::<4>().0 {
        let value = (word(chunk) ^ key[flag])
            .wrapping_sub(seed)
            .rotate_left(2)
            .wrapping_add(0x37a19e8b);
        chunk.copy_from_slice(&value.to_le_bytes());
        seed = seed.wrapping_sub(0x0139fa9b);
        flag = (flag + 1) & 3;
    }
    let tail = bytes.len() / 4 * 4;
    for byte in &mut bytes[tail..] {
        *byte = (*byte ^ ((key[flag] >> 4) as u8)).wrapping_add(3);
        flag = (flag + 1) & 3;
    }
}

pub(super) fn resource(bytes: &mut [u8], keys: &Keys, seed: u32) {
    let table = table(keys.digest[3], keys.key);
    let key_bytes: [u8; 96] =
        std::array::from_fn(|i| (keys.digest[1] >> 2) as u8 ^ table[TABLE_BYTES[i] as usize]);
    let decrypt: [u32; 24] = std::array::from_fn(|i| word(&key_bytes[i * 4..]) ^ seed);
    let mut key = 0x2748c39eu32;
    let mut flag = 10;
    for chunk in bytes.as_chunks_mut::<4>().0 {
        let value = ((decrypt[flag] >> 1) ^ decrypt[((key >> 6) & 15) as usize] ^ word(chunk))
            .wrapping_sub(seed)
            ^ keys.digest[(key & 3) as usize];
        chunk.copy_from_slice(&value.to_le_bytes());
        key = key.wrapping_add(seed).wrapping_add(value);
        flag = (flag + 1) & 15;
    }
    let tail = bytes.len() / 4 * 4;
    for byte in &mut bytes[tail..] {
        *byte = table[(*byte ^ 0xae) as usize];
    }
}
