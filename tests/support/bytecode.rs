//! Minimal PS2A scripts and argument pushes for scene/sequence integration tests.
use cmvs_rs::{
    machine::{Machine, Step},
    ps3::Script,
};

pub fn script(code: &[u8]) -> Script {
    let mut payload = 0u32.to_le_bytes().to_vec();
    payload.extend(code);
    let mut raw = b"PS2A".to_vec();
    for n in [
        48,
        0,
        0,
        1,
        code.len() as u32,
        0,
        0,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    Script::decode(&raw).unwrap()
}

/// Arguments are in callee order: args[0] is SP-4. Emits pushes in reverse order.
pub fn command(code: &mut Vec<u8>, opcode: u16, args: &[u32]) {
    for &arg in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(arg.to_le_bytes());
    }
    code.extend(opcode.to_le_bytes());
}

pub fn pending(vm: &mut Machine<'_>) {
    while !matches!(vm.step().unwrap(), Step::System { .. }) {}
}
