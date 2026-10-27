// Synthetic 17x16 CMV9: a static 16-pixel-wide gray block and a dynamic edge column.
fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn symbol(bits: &mut Vec<u8>, n: u8) {
    for i in (0..4).rev() {
        bits.push((n >> i) & 1);
    }
}
fn value(bits: &mut Vec<u8>, v: i16) {
    let n = 16 - v.unsigned_abs().leading_zeros() as u8;
    symbol(bits, n);
    if n != 0 {
        let raw = if v < 0 {
            (i32::from(v) + (1 << n) - 1) as u16
        } else {
            v as u16
        };
        for i in (0..n).rev() {
            bits.push(((raw >> i) & 1) as u8);
        }
    }
}
fn pack(bits: &[u8]) -> Vec<u8> {
    let mut b = vec![0; bits.len().div_ceil(32) * 4];
    for (i, &v) in bits.iter().enumerate() {
        b[i / 8] |= v << (i % 8);
    }
    b
}
fn jbpd(base: bool, reuse: bool, luma: i16) -> Vec<u8> {
    let mut dc = vec![];
    let mut old = 0;
    for v in [luma, luma, luma, luma, 0, 0] {
        value(&mut dc, v - old);
        old = v;
    }
    let dc = pack(&dc);
    let ac = pack(&[1; 24]);
    let mut b = vec![0; if base { 44 } else { 48 }];
    b[..4].copy_from_slice(b"JBPD");
    for (at, v) in [
        (4, 44),
        (8, 0x58c00002 | if reuse { 0x1000 } else { 0 }),
        (16, 17 | (16 << 16)),
        (20, 24),
        (28, dc.len() as u32),
        (32, ac.len() as u32),
        (36, 6),
    ] {
        put(&mut b, at, v);
    }
    for _ in 0..32 {
        b.extend(1u32.to_le_bytes());
    }
    b.extend([0; 16]);
    b.extend([1; 128]);
    b.extend([1, 1, 1, 0, 1, 2, 0, 0]);
    if reuse {
        b.extend([1, 0, 255]);
    }
    b.extend(dc);
    b.extend(ac);
    b
}
pub fn movie(reuse: bool) -> Vec<u8> {
    let mut base = vec![0; 9];
    base[4..8].copy_from_slice(&[2, 0, 1, 0]);
    base[8] = 128;
    base.extend(jbpd(true, false, 64));
    let size = base.len() as u32;
    put(&mut base, 0, size);
    let frame = jbpd(false, reuse, 128);
    let resource = 84 + base.len();
    let length = resource + frame.len();
    let mut b = vec![0; 84];
    b[..4].copy_from_slice(b"CMV9");
    for (at, v) in [
        (4, resource as u32),
        (8, length as u32),
        (16, 1),
        (20, 15),
        (24, 1),
        (28, 17),
        (32, 16),
        (36, 24),
        (48, frame.len() as u32),
        (52, 17 * 16 * 3 + 54),
        (56, 2),
        (64, 1),
        (80, frame.len() as u32),
    ] {
        put(&mut b, at, v);
    }
    b.extend(base);
    b.extend(frame);
    b
}
