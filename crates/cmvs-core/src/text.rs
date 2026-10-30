//! Stateless string utility commands of the 0x20f0..0x20ff group; only verified ones are handled.
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    machine::Machine,
    vm::{StringRef, SystemResult},
};

pub fn try_handle(vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
    match vm.pending_system() {
        Some((pc, 0xf1)) => compare(vm, pc).map(|()| true),
        Some((pc, 0xf8)) => strlen(vm, pc).map(|()| true),
        Some((pc, 0xfc)) => format(vm, pc).map(|()| true),
        _ => Ok(false),
    }
}

/// 0x20f1 / 0x140074400: system result 0 = 1 when the SP-4 and SP-8 strings are equal under
/// 0x140008140, else 0; returns 0x4008.
fn compare(vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
    let arg = |distance| {
        vm.stack()
            .peek_u32(distance)
            .map_err(|e| error(pc, 0x20f1, e.0))
    };
    let (first, second) = (arg(4)?, arg(8)?);
    let first = vm.resolve_string(first)?;
    let second = vm.resolve_string(second)?;
    let equal = equal_ignoring_ascii_case(&first, &second).map_err(|why| error(pc, 0x20f1, why))?;
    vm.complete_system(SystemResult(0x4008))?;
    vm.state_mut()
        .set_system_integer(0, u32::from(equal))
        .expect("slot 0 exists");
    Ok(())
}

/// 0x140008140 walks the first string: a Shift-JIS lead byte compares itself and the next
/// byte exactly, any other byte compares with a-z folded to A-Z; the second string must
/// end where the first does. A trailing lead byte whose pair matches steps past the first
/// string's terminator, which is not modeled.
fn equal_ignoring_ascii_case(first: &[u8], second: &[u8]) -> Result<bool, &'static str> {
    let at = |text: &[u8], i: usize| text.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while at(first, i) != 0 {
        let b = at(first, i);
        if (0x81..=0x9f).contains(&b) || (0xe0..=0xfc).contains(&b) {
            if b != at(second, i) || at(first, i + 1) != at(second, i + 1) {
                return Ok(false);
            }
            if at(first, i + 1) == 0 {
                return Err("string comparison steps past a trailing lead byte");
            }
            i += 2;
        } else {
            if !b.eq_ignore_ascii_case(&at(second, i)) {
                return Ok(false);
            }
            i += 1;
        }
    }
    Ok(at(second, i) == 0)
}

/// 0x20f8 / 0x1400747c0: system result 0 = lstrlenA of the SP-4 string reference.
fn strlen(vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
    let raw = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x20f8, e.0))?;
    let length = vm.resolve_string(raw)?.len() as u32;
    vm.complete_system(SystemResult(0x4004))?;
    vm.state_mut()
        .set_system_integer(0, length)
        .expect("slot 0 exists");
    Ok(())
}

/// 0x20fc / 0x140074990: formats the SP-8 string with 0x140006ed0 into the SP-4 string
/// reference, passing SP-12 as the only variadic argument; returns 0x400c without a system
/// result. The original writes unbounded into the destination; only a string-bank slot
/// (1024 bytes, index > 127 meaning slot 0 as in 0x14006b850) is modeled, and it must not
/// be the format's own slot.
fn format(vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
    let arg = |distance| {
        vm.stack()
            .peek_u32(distance)
            .map_err(|e| error(pc, 0x20fc, e.0))
    };
    let (target, pattern, value) = (arg(4)?, arg(8)?, arg(12)?);
    let slot = bank_slot(target).ok_or_else(|| {
        error(
            pc,
            0x20fc,
            "format destination outside the string bank is not modeled",
        )
    })?;
    if bank_slot(pattern) == Some(slot) {
        return Err(error(pc, 0x20fc, "format string overlaps its destination"));
    }
    let bytes = vm.resolve_string(pattern)?;
    let output = sprintf(&bytes, value).map_err(|why| error(pc, 0x20fc, why))?;
    if output.len() >= 1024 {
        return Err(error(
            pc,
            0x20fc,
            "formatted string overflows its 1024-byte slot",
        ));
    }
    vm.complete_system(SystemResult(0x400c))?;
    vm.state_mut()
        .set_string(slot, &output)
        .expect("slot and length checked above");
    Ok(())
}

/// String-bank slot of a reference; 0x14006b850 maps indexes above 127 to slot 0.
pub(crate) fn bank_slot(raw: u32) -> Option<u32> {
    match StringRef::decode(raw) {
        StringRef::StringBank(index) => Some(if index > 127 { 0 } else { index }),
        _ => None,
    }
}

/// 0x140006ed0 for literal bytes, `%%` and `%z`, the only conversion this title's scripts
/// use (`mask%z3a.pb3`). `%z` skips one sign character, reads at most two width digits and
/// calls 0x140006450 with zero padding: positions are filled from the right with powers of
/// ten that wrap as u32, so digits beyond the width are dropped. Other conversions, a second
/// argument (an undefined register in the original) and a trailing `%` are errors.
fn sprintf(pattern: &[u8], value: u32) -> Result<Vec<u8>, &'static str> {
    let mut output = Vec::new();
    let mut used = false;
    let mut i = 0;
    while i < pattern.len() {
        let byte = pattern[i];
        i += 1;
        if byte != b'%' {
            output.push(byte);
            continue;
        }
        match pattern.get(i) {
            Some(b'%') => {
                output.push(b'%');
                i += 1;
            }
            Some(b'z') => {
                i += 1;
                if matches!(pattern.get(i), Some(b'+' | b'-')) {
                    i += 1;
                }
                let mut width = 0usize;
                for _ in 0..2 {
                    let Some(digit @ b'0'..=b'9') = pattern.get(i) else {
                        break;
                    };
                    width = width * 10 + usize::from(digit - b'0');
                    i += 1;
                }
                if width == 0 {
                    return Err("%z without a width writes before the original buffer");
                }
                if used {
                    return Err("a second format argument reads an undefined register");
                }
                used = true;
                let mut digits = vec![0; width];
                let mut power = 1u32;
                for digit in digits.iter_mut().rev() {
                    *digit = if value < power {
                        b'0'
                    } else {
                        let quotient = value
                            .checked_div(power)
                            .ok_or("format width reaches a zero divisor in the original")?;
                        b'0' + (quotient % 10) as u8
                    };
                    power = power.wrapping_mul(10);
                }
                output.extend(digits);
            }
            None => return Err("format ends in %; the original reads past the terminator"),
            Some(_) => return Err("format conversion not implemented"),
        }
    }
    Ok(output)
}
