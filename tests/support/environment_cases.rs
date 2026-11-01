use cmvs_rs::dispatch::Capability;
pub fn cases() -> Vec<(u16, Vec<u32>, Capability)> {
    vec![
        (0x2010, vec![0, 1], Capability::Bootstrap),
        (0x2011, vec![0, 1], Capability::Bootstrap),
        (0x2012, vec![0, 1], Capability::Bootstrap),
        (0x2013, vec![0, 1], Capability::Bootstrap),
        (0x2014, vec![0, 1], Capability::Bootstrap),
        (0x2015, vec![1], Capability::Bootstrap),
        (0x2016, vec![1], Capability::Bootstrap),
        (0x2019, vec![0, 1], Capability::Bootstrap),
        (0x201a, vec![0, 1], Capability::Bootstrap),
        (0x201d, vec![77], Capability::Bootstrap),
        (0x201e, vec![77], Capability::Bootstrap),
        (0x201f, vec![77], Capability::Bootstrap),
        (0x2090, vec![], Capability::Bootstrap),
        (0x2091, vec![0, 2, 1], Capability::Bootstrap),
        (0x2092, vec![0, 2, 1], Capability::Bootstrap),
        (0x2093, vec![0, 2, 1], Capability::Bootstrap),
        (0x2094, vec![0, 2, 1], Capability::Bootstrap),
        (0x2098, vec![15], Capability::Bootstrap),
        (0x234d, vec![], Capability::FunctionKeys),
        (0x208f, vec![6], Capability::Platform),
        (0x2099, vec![1], Capability::Platform),
        (0x2160, vec![1], Capability::Platform),
        (0x2161, vec![1], Capability::Platform),
        (0x2163, vec![], Capability::Platform),
        (0x2164, vec![1], Capability::Platform),
        (0x216b, vec![], Capability::Platform),
        (0x216c, vec![], Capability::Platform),
        (0x216d, vec![1], Capability::Platform),
        (0x216e, vec![], Capability::Platform),
        (0x216f, vec![], Capability::Platform),
        (0x22b0, vec![64, 32], Capability::Platform),
        (0x22c8, vec![], Capability::Platform),
        (0x22c9, vec![0], Capability::Platform),
        (0x22ca, vec![], Capability::Platform),
        (0x22fc, vec![], Capability::Platform),
        (0x22fd, vec![2], Capability::Platform),
        (0x22ff, vec![], Capability::Platform),
        (0x213d, vec![], Capability::FontDialog),
        (0x203c, vec![11, 4294967295], Capability::Capture),
        (0x2304, vec![], Capability::Capture),
        (0x2305, vec![], Capability::Capture),
        (0x2306, vec![], Capability::Capture),
        (0x203e, vec![11, 4294967295], Capability::CaptureDevice),
    ]
}
fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
pub fn cur() -> Vec<u8> {
    let mut b = vec![0; 94];
    b[2] = 2;
    b[4] = 1;
    b[6] = 2;
    b[7] = 2;
    b[8] = 4;
    b[10] = 1;
    put(&mut b, 14, 72);
    put(&mut b, 18, 22);
    put(&mut b, 22, 40);
    put(&mut b, 26, 2);
    put(&mut b, 30, 4);
    b[34] = 1;
    b[36] = 8;
    put(&mut b, 54, 4);
    b[66..70].copy_from_slice(&[30, 20, 10, 0]);
    b[70..74].copy_from_slice(&[255, 255, 255, 0]);
    b[78..86].copy_from_slice(&[2, 3, 0, 0, 0, 1, 0, 0]);
    b[86] = 0xc0;
    b
}
pub fn ico() -> Vec<u8> {
    let mut b = vec![0; 23];
    b[2] = 1;
    b[4] = 1;
    b[6] = 1;
    b[7] = 1;
    put(&mut b, 14, 1);
    put(&mut b, 18, 22);
    b
}
