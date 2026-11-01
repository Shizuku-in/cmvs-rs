use cmvs_rs::dispatch::Capability;
pub fn cases() -> Vec<(u16, Vec<u32>, Capability)> {
    vec![
        (0x2280, vec![], Capability::Sequence),
        (0x2281, vec![], Capability::Sequence),
        (0x2282, vec![], Capability::Sequence),
        (0x2283, vec![], Capability::Sequence),
        (0x2284, vec![], Capability::Sequence),
        (0x2285, vec![], Capability::Sequence),
        (0x2286, vec![16], Capability::Sequence),
        (
            0x2287,
            vec![0, 1065353216, 1073741824, 1077936128, 32],
            Capability::Sequence,
        ),
        (0x2288, vec![11, 0, 32], Capability::Sequence),
        (
            0x2289,
            vec![11, 1065353216, 1073741824, 1077936128, 0, 32],
            Capability::Sequence,
        ),
        (
            0x228a,
            vec![11, 1065353216, 1073741824, 1077936128, 0, 32],
            Capability::Sequence,
        ),
        (
            0x2308,
            vec![0, 11, 0, 3735928559, 0],
            Capability::MovieScene,
        ),
        (0x2309, vec![0, 0, 0, 4294967295, 1], Capability::MovieStart),
        (0x230a, vec![0], Capability::MovieState),
        (0x230b, vec![0], Capability::MovieState),
        (0x230c, vec![0], Capability::MovieState),
        (0x231a, vec![0], Capability::MovieState),
        (0x20c0, vec![0], Capability::MoviePlayer),
        (0x20c1, vec![], Capability::MoviePlayer),
        (0x20c2, vec![], Capability::MoviePlayer),
    ]
}
pub fn cmv(frames: u32, rate: u32, rate_time: u32, audio: bool) -> Vec<u8> {
    let count = frames + u32::from(audio);
    let resource = 44 + count * 20;
    let data = frames * 4 + if audio { 3 } else { 0 };
    let mut bytes = b"CMV6".to_vec();
    // max_sequence is the frame limit; the index holds max_sequence + 1 entries.
    let max_sequence = count - 1;
    for word in [
        resource,
        resource + data,
        0,
        max_sequence,
        rate,
        rate_time,
        1280,
        720,
        24,
        2,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    for i in 0..frames {
        for word in [i, 4, 16, 2, i * 4] {
            bytes.extend(word.to_le_bytes());
        }
    }
    if audio {
        for word in [frames, 3, 3, 0, frames * 4] {
            bytes.extend(word.to_le_bytes());
        }
    }
    bytes.resize(bytes.len() + data as usize, 0);
    bytes
}
