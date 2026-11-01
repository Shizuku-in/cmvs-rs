use cmvs_rs::dispatch::Capability;
pub fn cases() -> Vec<(u16, Vec<u32>, Capability)> {
    vec![
        (0x2001, vec![], Capability::Runtime),
        (0x2003, vec![], Capability::Runtime),
        (0x228c, vec![1, 2], Capability::Runtime),
        (0x228e, vec![1, 2], Capability::Runtime),
        (0x22b5, vec![1000], Capability::Runtime),
        (0x22b6, vec![1000], Capability::Runtime),
        (0x22b8, vec![], Capability::Runtime),
        (0x22b7, vec![0, 2], Capability::Runtime),
        (
            0x22bd,
            vec![1000, 0, 4294967295, 2147483648, 2147483649],
            Capability::Runtime,
        ),
        (0x22b2, vec![], Capability::Snapshot),
        (0x2153, vec![0, 0, 4294967295], Capability::MessageWait),
        (0x2158, vec![0, 0, 4294967295], Capability::MessageWait),
    ]
}
