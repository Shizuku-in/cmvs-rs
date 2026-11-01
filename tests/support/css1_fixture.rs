fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn body() -> Vec<u8> {
    vec![0; 44864]
}
fn literals(b: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    for chunk in b.chunks(8) {
        out.push(((1u16 << chunk.len()) - 1) as u8);
        out.extend(chunk);
    }
    out
}
fn wrap(payload: &[u8], packed: &[u8], rotation: u32, counts: [u32; 2]) -> Vec<u8> {
    let mut raw = vec![0; 360];
    raw[..4].copy_from_slice(b"CSS1");
    put(&mut raw, 4, 104);
    put(&mut raw, 68, 0x12345678);
    raw[76] = 1;
    raw[78] = 7;
    raw[80] = 161;
    raw[82..84].copy_from_slice(&(((rotation - 2) << 3) as u16).to_le_bytes());
    put(&mut raw, 84, counts[1]);
    put(&mut raw, 88, counts[0]);
    put(&mut raw, 92, payload.len() as u32);
    put(&mut raw, 96, packed.len() as u32);
    put(&mut raw, 100, 360 + packed.len() as u32);
    let key = b"YUKO_KAWAI_PURPLE_SYSTEM_SAVE_CHECK";
    for (i, b) in packed.iter().enumerate() {
        raw.push(
            ((b.wrapping_add(64) ^ 161u8.wrapping_add(key[i % key.len()])).wrapping_add(7))
                .rotate_left(rotation),
        );
    }
    let sum = raw[360..]
        .iter()
        .fold(0u16, |s, b| s.wrapping_add(u16::from(*b)));
    raw.extend(sum.to_le_bytes());
    raw
}
fn fixture(payload: &[u8], counts: [u32; 2]) -> Vec<u8> {
    wrap(payload, &literals(payload), 4, counts)
}

pub fn css1() -> Vec<u8> {
    fixture(&body(), [0, 0])
}
