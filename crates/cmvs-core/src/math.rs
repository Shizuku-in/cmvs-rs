//! Signed integer utility commands and their VM result values.
//!
//! These handlers are used by [`crate::dispatch`]; they do not advance a frame.

// 0x20d0..0x20d3 are inline at 0x140082db4..0x140082e7f and write system result 0.
use crate::{bytecode::DecodeError, dispatch::Call};

/// Inspected 2007 0x4454a0 -> 0x4120b0: integer degrees and signed radius;
/// x87 uses a wrapped signed angle times 100 and the f32 PI constant promoted to f64.
/// CVNS `0x4e7` (degrees) and `0x4ec` (centidegrees, cvns-20050727 0x442ad0) use `f64::consts::PI`.
pub(crate) fn handle_polar_point(call: &mut Call<'_>) -> Result<(), DecodeError> {
    let degrees = !matches!(call.command, 0xd9 | 0x4ec);
    let pi = if matches!(call.command, 0x4e7 | 0x4ec) {
        std::f64::consts::PI
    } else {
        f64::from(std::f32::consts::PI)
    };
    handle_polar_point_with(call, degrees, pi)
}

/// x87 `ftol` conversion clamps out-of-range floats to integer-indefinite (`i32::MIN`).
fn trunc_f64_i32(v: f64) -> i32 {
    if v.trunc() < f64::from(i32::MIN) || v.trunc() > f64::from(i32::MAX) {
        i32::MIN
    } else {
        v as i32
    }
}

fn handle_polar_point_with(call: &mut Call<'_>, degrees: bool, pi: f64) -> Result<(), DecodeError> {
    let raw = call.argument(4, "angle")? as i32;
    let angle = if degrees { raw.wrapping_mul(100) } else { raw };
    let radius = f64::from(call.argument(8, "radius")? as i32);
    let radians = f64::from(angle) / 18000.0 * pi;
    let x = trunc_f64_i32(radians.cos() * radius);
    let y = trunc_f64_i32(radians.sin() * radius);
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

/// CVNS `0x4e8` (cvns-20040730 0x42d1c0 -> 0x43bae0) and `0x4ed` (cvns-20050727 0x433290 -> 0x442bb0):
/// pops `(x1, y1, x2, y2)` (`0x4010`), writes integer degree `[0, 360)` (`0x4e8`) or
/// centidegree `[0, 36000)` (`0x4ed`) angle to `system_integer(4)` and distance to `system_integer(5)`.
pub(crate) fn handle_cartesian_to_polar(call: &mut Call<'_>) -> Result<(), DecodeError> {
    let x1 = call.argument(4, "x1")? as i32;
    let y1 = call.argument(8, "y1")? as i32;
    let x2 = call.argument(12, "x2")? as i32;
    let y2 = call.argument(16, "y2")? as i32;
    let (scale, full) = if call.command == 0x4e8 {
        (1.0, 360)
    } else {
        (100.0, 36000)
    };
    let dx = f64::from(x2.wrapping_sub(x1));
    let dy = f64::from(y2.wrapping_sub(y1));
    let theta = dy.atan2(dx);
    let mut angle = trunc_f64_i32((180.0 / std::f64::consts::PI) * theta * -scale);
    if angle < 0 {
        angle = angle.wrapping_add(full);
    }
    angle = full.wrapping_sub(angle);
    if angle == full {
        angle = 0;
    }
    let dist = if x2 == x1 {
        y2.wrapping_sub(y1).wrapping_abs()
    } else {
        trunc_f64_i32(dx / theta.cos())
    };
    call.complete(0x4010)?;
    let state = call.vm.state_mut();
    state
        .set_system_integer(4, angle as u32)
        .expect("slot 4 exists");
    state
        .set_system_integer(5, dist as u32)
        .expect("slot 5 exists");
    Ok(())
}

pub(crate) fn handle(call: &mut Call<'_>) -> Result<(), DecodeError> {
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
