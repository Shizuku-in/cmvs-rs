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
