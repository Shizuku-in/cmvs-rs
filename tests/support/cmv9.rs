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
    movie_frames(1, 15, 1, reuse)
}

/// Synthetic opaque CMV6 with 16x16 independent frames and an empty final audio entry.
/// Audio flag 2 does not imply a nonempty soundtrack (as in the real scene sample).
#[allow(dead_code)]
pub fn cmv6(frames: u32, bitmap_header: bool) -> Vec<u8> {
    let resource = 44 + (frames as usize + 1) * 20;
    let mut b = vec![0; resource];
    b[..4].copy_from_slice(b"CMV6");
    for (at, v) in [
        (4, resource as u32),
        (16, frames),
        (20, 15),
        (24, 1),
        (28, 16),
        (32, 16),
        (36, 24),
        (40, 2),
    ] {
        put(&mut b, at, v);
    }
    for i in 0..frames {
        let mut payload = jbpd(false, false, (i * 64) as i16);
        put(&mut payload, 16, 16 | (16 << 16));
        payload[325] = 1;
        if i % 2 != 0 {
            put(&mut payload, 8, 0x58c01002);
            payload.splice(328..328, [255; 4]);
        }
        let offset = (b.len() - resource) as u32;
        for (n, v) in [
            i,
            payload.len() as u32,
            16 * 16 * 3 + if bitmap_header { 54 } else { 0 },
            2,
            offset,
        ]
        .into_iter()
        .enumerate()
        {
            put(&mut b, 44 + i as usize * 20 + n * 4, v);
        }
        b.extend(payload);
    }
    put(&mut b, 44 + frames as usize * 20, frames);
    let length = b.len() as u32;
    put(&mut b, 60 + frames as usize * 20, length - resource as u32);
    put(&mut b, 8, length);
    b
}

pub fn movie_frames(frames: u32, rate: u32, time: u32, reuse: bool) -> Vec<u8> {
    let mut base = vec![0; 9];
    base[4..8].copy_from_slice(&[2, 0, 1, 0]);
    base[8] = 128;
    base.extend(jbpd(true, false, 64));
    let size = base.len() as u32;
    put(&mut base, 0, size);
    let payloads: Vec<_> = (0..frames)
        .map(|i| jbpd(false, reuse, 128 + (i % 8) as i16 * 64))
        .collect();
    let head = 44 + (frames as usize + 1) * 20;
    let resource = head + base.len();
    let length = resource + payloads.iter().map(Vec::len).sum::<usize>();
    let mut b = vec![0; head];
    b[..4].copy_from_slice(b"CMV9");
    for (at, v) in [
        (4, resource as u32),
        (8, length as u32),
        (16, frames),
        (20, rate),
        (24, time),
        (28, 17),
        (32, 16),
        (36, 24),
    ] {
        put(&mut b, at, v);
    }
    let mut offset = 0;
    for (i, payload) in payloads.iter().enumerate() {
        let at = 44 + i * 20;
        for (n, v) in [i as u32, payload.len() as u32, 17 * 16 * 3 + 54, 2, offset]
            .into_iter()
            .enumerate()
        {
            put(&mut b, at + n * 4, v);
        }
        offset += payload.len() as u32;
    }
    put(&mut b, 44 + frames as usize * 20, frames);
    put(&mut b, 60 + frames as usize * 20, offset);
    b.extend(base);
    for p in payloads {
        b.extend(p);
    }
    b
}
