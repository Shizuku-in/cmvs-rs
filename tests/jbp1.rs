use cmvs_rs::{image, jbp1, pb3};

/// The decoder's tree: join the two lowest weights (lower index first, it takes bit 0).
fn codes(freq: &[u32; 16]) -> Vec<Vec<u8>> {
    let mut weights: Vec<u64> = freq.iter().map(|&w| u64::from(w)).collect();
    let mut active = vec![true; 16];
    let mut children = vec![[0usize; 2]; 16];
    for _ in 0..15 {
        let mut ids: Vec<usize> = (0..weights.len()).filter(|&i| active[i]).collect();
        ids.sort_by_key(|&i| (weights[i], i));
        let (a, b) = (ids[0], ids[1]);
        active[a] = false;
        active[b] = false;
        weights.push(weights[a] + weights[b]);
        active.push(true);
        children.push([a, b]);
    }
    let mut out = vec![Vec::new(); 16];
    let mut stack = vec![(30usize, Vec::new())];
    while let Some((node, path)) = stack.pop() {
        if node < 16 {
            out[node] = path;
            continue;
        }
        for (bit, &child) in children[node].iter().enumerate() {
            let mut next = path.clone();
            next.push(bit as u8);
            stack.push((child, next));
        }
    }
    out
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    bits: usize,
}
impl Writer {
    fn bit(&mut self, bit: u8) {
        if self.bits.is_multiple_of(8) {
            self.bytes.push(0);
        }
        *self.bytes.last_mut().unwrap() |= bit << (self.bits % 8);
        self.bits += 1;
    }
    fn code(&mut self, code: &[u8]) {
        for &bit in code {
            self.bit(bit);
        }
    }
    /// Bit length symbol, then the magnitude bits; negatives store x + 2^n - 1.
    fn value(&mut self, codes: &[Vec<u8>], value: i32) {
        let n = if value == 0 {
            0
        } else {
            32 - value.unsigned_abs().leading_zeros()
        };
        self.code(&codes[n as usize]);
        let raw = if value < 0 {
            value + (1 << n) - 1
        } else {
            value
        };
        for i in (0..n).rev() {
            self.bit(((raw >> i) & 1) as u8);
        }
    }
    fn finish(mut self) -> Vec<u8> {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
        self.bytes
    }
}

/// One block's AC content: coefficient values at successive scan positions, zero runs
/// (selector index) and the end marker.
enum Ac {
    Value(i32),
    Run(usize),
}

/// A JBP1 stream of `blocks` (DC absolute values and AC items) with all-one quantizers and a
/// zero-run table of `runs`.
fn stream(width: u16, height: u16, blocks: &[(i32, Vec<Ac>)], flags: u32) -> Vec<u8> {
    let dc_freq: [u32; 16] = [5, 1, 1, 1, 1, 1, 1, 4, 6, 1, 8, 1, 1, 1, 1, 1];
    let ac_freq: [u32; 16] = [3, 1, 1, 1, 1, 1, 1, 5, 1, 1, 1, 1, 1, 1, 1, 9];
    let (dc_codes, ac_codes) = (codes(&dc_freq), codes(&ac_freq));
    let mut dc = Writer::default();
    let mut previous = 0;
    for (value, _) in blocks {
        dc.value(&dc_codes, value - previous);
        previous = *value;
    }
    let mut ac = Writer::default();
    for (_, items) in blocks {
        for item in items {
            match item {
                Ac::Value(v) => ac.value(&ac_codes, *v),
                Ac::Run(selector) => {
                    ac.code(&ac_codes[0]);
                    for _ in 0..*selector {
                        ac.bit(1);
                    }
                    ac.bit(0);
                }
            }
        }
        ac.code(&ac_codes[15]);
    }
    let (dc, ac) = (dc.finish(), ac.finish());
    let mut b = b"JBP1".to_vec();
    for word in [
        44,
        flags,
        100,
        u32::from(width) | u32::from(height) << 16,
        24,
        0,
    ] {
        b.extend(u32::to_le_bytes(word));
    }
    b.extend((dc.len() as u32).to_le_bytes());
    b.extend((ac.len() as u32).to_le_bytes());
    b.extend([0; 8]);
    for f in dc_freq.iter().chain(&ac_freq) {
        b.extend(f.to_le_bytes());
    }
    b.extend([1u8, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    b.extend([1u8; 128]);
    b.extend(dc);
    b.extend(ac);
    b
}

/// DC-only MCU: four luma DCs and the two chroma DCs, every block ending at once.
fn mcu(dcs: [i32; 6]) -> Vec<(i32, Vec<Ac>)> {
    dcs.iter().map(|&dc| (dc, Vec::new())).collect()
}

/// Wraps JBP1 data (and an optional alpha run stream) in a PB3B type 2/3 file.
fn pb3_file(kind: u16, width: u16, height: u16, bpp: u16, jbp: &[u8], alpha: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 52];
    b[..4].copy_from_slice(b"PB3B");
    b[8] = 1;
    b[28..30].copy_from_slice(&kind.to_le_bytes());
    b[30..32].copy_from_slice(&width.to_le_bytes());
    b[32..34].copy_from_slice(&height.to_le_bytes());
    b[34..36].copy_from_slice(&bpp.to_le_bytes());
    let alpha_at = (52 + jbp.len()) as u32;
    b[44..48].copy_from_slice(&alpha_at.to_le_bytes());
    b[48..52].copy_from_slice(&(alpha.len() as u32).to_le_bytes());
    b.extend(jbp);
    b.extend(alpha);
    let len = b.len() as u32;
    b[4..8].copy_from_slice(&len.to_le_bytes());
    let seed = b[b.len() - 47..b.len() - 3].to_vec();
    let key = [b[b.len() - 3], b[b.len() - 2]];
    for i in 0..44 {
        b[8 + i] = b[8 + i].wrapping_add(seed[i]) ^ key[i % 2];
    }
    b
}

const FLAGS: u32 = 0x58c0_0002;

#[test]
fn dc_only_blocks_fill_quadrants_with_their_level_shifted_value() {
    // Luma levels 10, -20, 0, 100 in the four quadrants (DC = 8 * level with q = 1).
    let data = stream(16, 16, &mcu([80, -160, 0, 800, 0, 0]), FLAGS);
    let image = jbp1::decode(&data).unwrap();
    assert_eq!((image.padded_width, image.padded_height), (16, 16));
    let gray = |x: usize, y: usize| {
        let at = (y * 16 + x) * 3;
        let pixel = &image.bgr[at..at + 3];
        assert!(pixel.iter().all(|&v| v == pixel[0]), "{x},{y}: {pixel:?}");
        pixel[0]
    };
    assert_eq!(
        [gray(0, 0), gray(15, 0), gray(0, 15), gray(15, 15)],
        [138, 108, 128, 228]
    );
    assert_eq!(
        [gray(7, 7), gray(8, 7), gray(7, 8), gray(8, 8)],
        [138, 108, 128, 228]
    );
}

#[test]
fn one_horizontal_coefficient_gives_the_first_cosine_row() {
    // Coefficient 64 at scan position 1 (row 0, column 1); a zero run in Cb only consumes
    // its selector bits, so the following Cr DC still lines up.
    let mut blocks = mcu([0, 0, 0, 0, 0, 16]);
    blocks[0].1.push(Ac::Value(64));
    blocks[4].1.push(Ac::Run(1));
    let image = jbp1::decode(&stream(16, 16, &blocks, FLAGS)).unwrap();
    // Hand-computed: [88, 75, 50, 17, -17, -50, -75, -88] >> 3 = [11, 9, 6, 2, -3, -7, -10, -11].
    let expected: Vec<u8> = [11, 9, 6, 2, -3, -7, -10, -11]
        .iter()
        .map(|v: &i32| (v + 128) as u8)
        .collect();
    for y in 0..8 {
        let row: Vec<u8> = (0..8).map(|x| image.bgr[(y * 16 + x) * 3 + 1]).collect();
        // Cr = 2 gives green - ((46784 * 2) >> 16) = - 1 and red + ((91888 * 2) >> 16) = + 2.
        let green: Vec<u8> = expected.iter().map(|v| v - 1).collect();
        assert_eq!(row, green, "row {y}");
        let red: Vec<u8> = (0..8).map(|x| image.bgr[(y * 16 + x) * 3 + 2]).collect();
        assert_eq!(red, expected.iter().map(|v| v + 2).collect::<Vec<_>>());
    }
}

#[test]
fn pb3_types2_and3_crop_padded_rows_and_fill_alpha() {
    for kind in [2, 3] {
        // 20x10 needs two 16x16 MCUs: left luma level 10, right level -20.
        let mut blocks = mcu([80, 80, 80, 80, 0, 0]);
        blocks.extend(mcu([-160, -160, -160, -160, 0, 0]));
        let jbp = stream(20, 10, &blocks, FLAGS);
        let rgba = pb3::decode(&pb3_file(kind, 20, 10, 24, &jbp, &[])).unwrap();
        assert_eq!((rgba.width(), rgba.height()), (20, 10));
        let pixels = rgba.pixels();
        for y in 0..10 {
            for x in 0..20 {
                let level = if x < 16 { 138 } else { 108 };
                assert_eq!(
                    pixels[(y * 20 + x) * 4..][..4],
                    [level, level, level, 255],
                    "{x},{y}"
                );
            }
        }
        // 32 bits: alpha runs of 0xff/0x00 with counts and stand-alone literals.
        let mut alpha = vec![0xff, 150, 0x40, 0x00, 48, 0x80];
        let decoded = image::decode(&pb3_file(kind, 20, 10, 32, &jbp, &alpha))
            .unwrap()
            .unwrap();
        assert!(decoded.alpha);
        let a: Vec<u8> = decoded.pixels.pixels()[3..]
            .iter()
            .step_by(4)
            .copied()
            .collect();
        assert_eq!(a[..150], [0xff; 150]);
        assert_eq!(a[150], 0x40);
        assert_eq!(a[151..199], [0; 48]);
        assert_eq!(a[199], 0x80);
        // A run past the last pixel is rejected: 150 + 1 + 50 covers 201 of 200 pixels.
        alpha[4] = 50;
        assert!(pb3::decode(&pb3_file(kind, 20, 10, 32, &jbp, &alpha)).is_err());
    }
}

#[test]
fn malformed_streams_are_rejected() {
    let good = stream(16, 16, &mcu([0; 6]), FLAGS);
    assert!(jbp1::decode(&good).is_ok());
    // Unverified flags (here 4:4:4, mode 0).
    assert!(jbp1::decode(&stream(16, 16, &mcu([0; 6]), 0x48c0_0002)).is_err());
    // Luma level 400 indexes 784, past the 768-byte clamp table.
    assert!(jbp1::decode(&stream(16, 16, &mcu([3200, 0, 0, 0, 0, 0]), FLAGS)).is_err());
    // Truncated AC stream: drop the last four bytes and shrink its length.
    let mut cut = good.clone();
    cut.truncate(cut.len() - 4);
    let ac_len = u32::from_le_bytes(cut[32..36].try_into().unwrap()) - 4;
    cut[32..36].copy_from_slice(&ac_len.to_le_bytes());
    assert!(jbp1::decode(&cut).is_err());
    // A zero run of 63 + 1 positions from scan position 1 leaves the block.
    let mut blocks = mcu([0; 6]);
    blocks[0].1.push(Ac::Run(1));
    let mut long = stream(16, 16, &blocks, FLAGS);
    long[44 + 128 + 1] = 63;
    assert!(jbp1::decode(&long).is_err());
    // PB3 and JBP1 sizes must agree.
    for kind in [2, 3] {
        assert!(pb3::decode(&pb3_file(kind, 16, 15, 24, &good, &[])).is_err());
        assert!(pb3::decode(&pb3_file(kind, 16, 16, 8, &good, &[])).is_err());
        assert!(pb3::decode(&pb3_file(kind, 16, 16, 32, &good, &[255])).is_err());
    }
}
