use cmvs_rs::dispatch::Capability;
pub fn cases() -> Vec<(u16, Vec<u32>, Capability)> {
    vec![
        (0x20ec, vec![0x0, 0x3f800000], Capability::Windows),
        (
            0x2140,
            vec![0x0, 0x0, 0xa, 0x14, 0x64, 0x50],
            Capability::Windows,
        ),
        (0x2141, vec![0x0], Capability::Windows),
        (0x2142, vec![0x0, 0x2], Capability::Windows),
        (0x2143, vec![0x0, 0x1b], Capability::Windows),
        (0x2144, vec![0x0, 0x1, 0x2], Capability::Windows),
        (0x2145, vec![0x0, 0x1, 0x2, 0x63, 0x4f], Capability::Windows),
        (0x2146, vec![0x0, 0xffffff, 0x111111], Capability::Windows),
        (0x2147, vec![0x0, 0xa], Capability::Windows),
        (0x2148, vec![0x0, 0x1, 0x2], Capability::Windows),
        (0x2149, vec![0x0, 0x2], Capability::Windows),
        (0x214a, vec![0x0], Capability::Windows),
        (0x214b, vec![0x0], Capability::Windows),
        (0x214c, vec![0x0, 0x1], Capability::Windows),
        (0x214d, vec![0x0], Capability::Windows),
        (0x214e, vec![0x0, 0x1], Capability::Windows),
        (0x214f, vec![0x0, 0x1, 0x2], Capability::Windows),
        (0x2150, vec![0x0, 0x1, 0x3, 0x4], Capability::Windows),
        (0x2151, vec![0x0, 0x1, 0xffff, 0x8000], Capability::Windows),
        (0x2155, vec![0x0, 0x63], Capability::Windows),
        (0x215a, vec![0x0, 0x1e, 0x28], Capability::Windows),
        (0x215b, vec![0x0, 0x1], Capability::Windows),
        (
            0x215d,
            vec![0x0, 0x0, 0xa, 0x14, 0x64, 0x50, 0x500, 0x2d0],
            Capability::Windows,
        ),
        (0x2171, vec![0x0], Capability::Windows),
        (0x2178, vec![0x0, 0x11], Capability::Windows),
        (0x2179, vec![0x0, 0x10], Capability::Windows),
        (0x217a, vec![0x0, 0xffffffff, 0x1], Capability::Windows),
        (0x217b, vec![0x0, 0xffffffff], Capability::Windows),
        (
            0x217c,
            vec![0x0, 0xffffffff, 0x500, 0x2d0],
            Capability::Windows,
        ),
        (0x217f, vec![0x0, 0xffffffff], Capability::Windows),
        (0x2180, vec![0x0, 0xffffffff], Capability::Windows),
        (
            0x2182,
            vec![0x0, 0xffffffff, 0x0, 0x0, 0x2, 0x1],
            Capability::Windows,
        ),
        (
            0x2183,
            vec![0x0, 0xffffffff, 0xa, 0x14],
            Capability::Windows,
        ),
        (0x2184, vec![0x0, 0xffffffff, 0x2], Capability::Windows),
        (0x2185, vec![0x0, 0xffffffff, 0x1, 0x0], Capability::Windows),
        (0x2186, vec![0x0, 0xffffffff, 0x80], Capability::Windows),
        (0x218d, vec![0x0, 0xffffffff], Capability::Windows),
        (
            0x2190,
            vec![0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0x1, 0x0, 0x0],
            Capability::Windows,
        ),
        (
            0x2191,
            vec![0x0, 0x0, 0x0, 0x0, 0x28, 0x28],
            Capability::Windows,
        ),
        (0x2192, vec![0x0, 0x0], Capability::Windows),
        (0x2195, vec![0x0, 0x0], Capability::Windows),
        (0x2196, vec![0x0, 0x0, 0x1], Capability::Windows),
        (0x2197, vec![0x0, 0x0], Capability::Windows),
        (0x21cf, vec![0x0, 0xffffffff], Capability::Windows),
        (
            0x21d0,
            vec![
                0x0, 0xffffffff, 0x0, 0xffffffff, 0x14, 0x0, 0x0, 0x2, 0x1, 0x0, 0x0,
            ],
            Capability::Windows,
        ),
        (0x21d1, vec![0x0, 0xffffffff, 0x0], Capability::Windows),
        (
            0x215f,
            vec![0x0, 0x0, 0x0, 0x28, 0x28],
            Capability::WindowPointer,
        ),
        (0x2194, vec![0x0], Capability::WindowButtons),
        (0x2170, vec![0x0, 0x1], Capability::WindowImages),
        (0x2397, vec![0x0, 0x1], Capability::WindowImages),
    ]
}
pub fn setup() -> Vec<(u16, Vec<u32>)> {
    vec![
        (0x2140, vec![0x0, 0x0, 0xa, 0x14, 0x64, 0x50]),
        (0x2180, vec![0x0, 0xffffffff]),
        (0x2178, vec![0x0, 0x10]),
        (0x2180, vec![0x0, 0x10]),
        (0x21cf, vec![0x0, 0xffffffff]),
        (0x2380, vec![0x0, 0x0, 0xc, 0x8]),
    ]
}

pub fn image_bytes() -> Vec<u8> {
    fn put(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut bytes = vec![0; 52];
    bytes[..4].copy_from_slice(b"PB3B");
    put(&mut bytes, 8, 1);
    put(&mut bytes, 24, 1);
    bytes[28] = 1;
    bytes[30] = 2;
    bytes[32] = 1;
    bytes[34] = 24;
    put(&mut bytes, 44, 52);
    put(&mut bytes, 48, 52 + 19 * 3);
    for _ in 0..3 {
        bytes.extend(15u32.to_le_bytes());
    }
    for _ in 0..3 {
        for value in [1u32, 2, 0] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend([0xc0, 255, 255]);
    }
    for _ in 0..3 {
        bytes.extend(0u32.to_le_bytes());
    }
    let len = bytes.len() as u32;
    put(&mut bytes, 4, len);
    let seed = bytes[bytes.len() - 47..bytes.len() - 3].to_vec();
    let key = [bytes[bytes.len() - 3], bytes[bytes.len() - 2]];
    for i in 0..44 {
        bytes[8 + i] = bytes[8 + i].wrapping_add(seed[i]) ^ key[i % 2];
    }
    bytes
}
