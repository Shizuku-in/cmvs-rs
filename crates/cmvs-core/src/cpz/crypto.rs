//! Shared CPZ6/7 cipher primitives.
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

fn compress(initial: [u32; 4], block: [u32; 16]) -> [u32; 4] {
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
    let [mut a, mut b, mut c, mut d] = initial;
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
    [
        a.wrapping_add(initial[0]),
        b.wrapping_add(initial[1]),
        c.wrapping_add(initial[2]),
        d.wrapping_add(initial[3]),
    ]
}

fn custom_digest(input: [u32; 4], initial: [u32; 4]) -> [u32; 4] {
    let mut block = [0; 16];
    block[..4].copy_from_slice(&input);
    block[4] = 0x80;
    block[14] = 128;
    compress(initial, block)
}

pub(super) fn digest(input: [u32; 4]) -> [u32; 4] {
    let state = custom_digest(input, [0xc74a2b01, 0xe7c8ab8f, 0xd8bedc4e, 0x7302a4c5]);
    [state[3], state[1], state[2], state[0]]
}

/// Standard MD5 for CPZ7 encrypted index integrity (not key derivation).
pub(super) fn md5(bytes: &[u8]) -> [u8; 16] {
    let mut state = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    let (blocks, tail) = bytes.as_chunks::<64>();
    for block in blocks {
        state = compress(state, std::array::from_fn(|i| word(&block[i * 4..])));
    }
    let mut padding = [0u8; 128];
    padding[..tail.len()].copy_from_slice(tail);
    padding[tail.len()] = 0x80;
    let length = if tail.len() < 56 { 64 } else { 128 };
    padding[length - 8..length]
        .copy_from_slice(&(bytes.len() as u64).wrapping_mul(8).to_le_bytes());
    for block in padding[..length].as_chunks::<64>().0 {
        state = compress(state, std::array::from_fn(|i| word(&block[i * 4..])));
    }
    std::array::from_fn(|i| state[i / 4].to_le_bytes()[i % 4])
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
    /// AOITORI, REALIVE, SEISHUNF and KUNADO share this digest state and output order.
    pub fn from_header_v7(header: &[u8]) -> Self {
        let mut keys = Self::from_header(header);
        let masks = [0x43de7c1a, 0xcc65f416, 0xd016a93d, 0x97a3ba9b];
        let input = std::array::from_fn(|i| word(&header[32 + 4 * i..]) ^ masks[i]);
        let state = custom_digest(input, [0xc74a2b02, 0xe7c8ab8f, 0x38bebc4e, 0x7531a4c3]);
        keys.digest = [
            state[2] ^ 0x53a76d2e,
            state[1].wrapping_add(0x5bb17fda),
            state[0].wrapping_add(0x6853e14d),
            state[3] ^ 0xf5c6a9a3,
        ];
        keys
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
    outer_index_inner(bytes, key, false);
}

pub(super) fn outer_index_v7(bytes: &mut [u8], key: u32) {
    outer_index_inner(bytes, key, true);
}

fn outer_index_inner(bytes: &mut [u8], key: u32, countdown_tail: bool) {
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
    let length = bytes.len();
    for (i, byte) in bytes.iter_mut().enumerate().skip(tail) {
        let shift = if countdown_tail { length - i } else { i % 4 };
        *byte = (*byte ^ ((table[flag] >> (4 * shift)) as u8)).wrapping_sub(0x7d);
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
    directory_index_with_step(bytes, keys, 0x10fb562a);
}

pub(super) fn directory_index_with_step(bytes: &mut [u8], keys: &Keys, step: u32) {
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
        seed = seed.wrapping_add(step);
        flag = (flag + 1) & 3;
    }
    let tail = bytes.len() / 4 * 4;
    for byte in &mut bytes[tail..] {
        *byte = (*byte ^ ((key[flag] >> 6) as u8)).wrapping_add(0x37);
        flag = (flag + 1) & 3;
    }
}

pub(super) fn file_index(bytes: &mut [u8], directory: u32, keys: &Keys) {
    file_index_with_step(bytes, directory, keys, 0x0139fa9b);
}

pub(super) fn file_index_with_step(bytes: &mut [u8], directory: u32, keys: &Keys, step: u32) {
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
        seed = seed.wrapping_sub(step);
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

/// 0x140062fc0: wrapping sum of little-endian u32 words, then of the tail bytes, from 0x5a902b7c.
pub fn data_checksum(bytes: &[u8]) -> u32 {
    let (words, remainder) = bytes.as_chunks::<4>();
    let sum = words.iter().fold(0x5a902b7c_u32, |sum, word| {
        sum.wrapping_add(u32::from_le_bytes(*word))
    });
    remainder
        .iter()
        .fold(sum, |sum, &byte| sum.wrapping_add(u32::from(byte)))
}

#[cfg(test)]
mod tests {
    use super::md5;

    #[test]
    fn standard_md5_matches_independent_vectors_across_padding_boundaries() {
        // Python hashlib outputs, including one- and two-block padding cases.
        for (bytes, expected) in [
            (Vec::new(), "d41d8cd98f00b204e9800998ecf8427e"),
            (b"abc".to_vec(), "900150983cd24fb0d6963f7d28e17f72"),
            (vec![b'a'; 55], "ef1772b6dff9a122358552954ad0df65"),
            (vec![b'a'; 56], "3b0c8ac703f828b04c6c197006d17218"),
            (vec![b'a'; 64], "014842d480b571495a4a0363793f7367"),
            (vec![b'a'; 1000], "cabe45dcc9ae5b66ba86600cca6b8ba8"),
        ] {
            let actual: String = md5(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(actual, expected);
        }
    }
}
