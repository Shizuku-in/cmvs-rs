//! Integer helpers 0x20d0..0x20d3, inline in the system dispatcher (0x140082db4..
//! 0x140082e7f); addresses refer to the documented binary hash. Arguments are signed and the
//! result goes to system result 0.
use crate::{bytecode::DecodeError, dispatch::Call, machine::Machine};

/// Inspected 2007 0x4454a0 -> 0x4120b0: integer degrees and signed radius;
/// x87 uses a wrapped signed angle times 100 and the f32 PI constant promoted to f64.
pub(crate) fn handle_polar_point(call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
    let angle = (call.argument(4, "angle")? as i32).wrapping_mul(100);
    let radius = f64::from(call.argument(8, "radius")? as i32);
    let radians = f64::from(angle) / 18000.0 * f64::from(std::f32::consts::PI);
    // x87's integer-indefinite value for conversion overflow is i32::MIN.
    let integer = |v: f64| {
        if v.trunc() < f64::from(i32::MIN) || v.trunc() > f64::from(i32::MAX) {
            i32::MIN
        } else {
            v as i32
        }
    };
    let x = integer(radians.cos() * radius);
    let y = integer(radians.sin() * radius);
    call.complete(0x4008)?;
    call.vm
        .state_mut()
        .set_system_integer(1, x as u32)
        .expect("slot 1 exists");
    call.vm
        .state_mut()
        .set_system_integer(2, y as u32)
        .expect("slot 2 exists");
    Ok(())
}

/// 0x20d0: |SP-4| (cdq/xor/sub, so i32::MIN stays), 0x4004. 0x20d1: SP-4 raised to at least
/// SP-8, then lowered to at most SP-12 (the upper bound wins), 0x400c. 0x20d2 / 0x20d3: the
/// larger / smaller of SP-4 and SP-8, 0x4008.
pub fn try_handle(vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
    let Some((pc, command @ 0xd0..=0xd3)) = vm.pending_system() else {
        return Ok(false);
    };
    handle(&mut Call::new(vm, pc, command))?;
    Ok(true)
}

pub(crate) fn handle(call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
    let command = call.command;
    let count = match command {
        0xd0 => 1,
        0xd1 => 3,
        _ => 2,
    };
    let mut args = [0i32; 3];
    for (i, arg) in args.iter_mut().take(count).enumerate() {
        let name = if command == 0xd1 {
            ["value", "lower bound", "upper bound"][i]
        } else {
            ["value", "second value", "upper bound"][i]
        };
        *arg = call.argument(4 * (i + 1), name)? as i32;
    }
    let [a, b, c] = args;
    let value = match command {
        0xd0 => a.wrapping_abs(),
        0xd1 => a.max(b).min(c),
        0xd2 => a.max(b),
        _ => a.min(b),
    };
    call.complete(0x4000 | (4 * count) as u16)?;
    call.vm
        .state_mut()
        .set_system_integer(0, value as u32)
        .expect("slot 0 exists");
    Ok(())
}
