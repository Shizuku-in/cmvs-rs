use cmvs_rs::dispatch::Capability;
pub fn cases() -> Vec<(u16, Vec<u32>, Capability)> {
    vec![
        (0x2080, vec![0], Capability::ScriptControl),
        (0x2081, vec![0, 9], Capability::ScriptControl),
        (0x2083, vec![1, 2], Capability::ScriptControl),
        (0x2088, vec![1, 0, 3, 4, 5], Capability::ScriptControl),
        (0x2089, vec![1], Capability::ScriptControl),
        (0x208a, vec![1], Capability::ScriptControl),
        (0x208b, vec![1], Capability::ScriptControl),
        (0x208c, vec![1], Capability::ScriptControl),
        (0x2128, vec![], Capability::SystemRead),
        (0x2129, vec![], Capability::SystemWrite),
        (0x22b3, vec![2], Capability::SaveMenu),
        (0x22b4, vec![1], Capability::SaveMenu),
        (0x22bb, vec![0], Capability::SaveMenu),
    ]
}
