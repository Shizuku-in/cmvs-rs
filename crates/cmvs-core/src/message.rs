//! Message windows; addresses refer to the documented binary hash.
//!
//! Engine +4064 holds eight message window pointers (0x9f0-byte objects, constructor
//! 0x14005cf70), +4128 twelve text objects (0x250 bytes, constructor 0x14005c550) and +4224
//! twelve font objects (0x70 bytes, 0x140059990); the engine constructor 0x140014e60 zeroes
//! all three tables. A window's first word (+0) names its text object, and +2528 caches that
//! object's pointer. 0x215d creates windows and 0x2141 destroys them. 0x2140 (0x140075a50)
//! differs from 0x215d only in taking the logical size from the device, which is not
//! modeled, so it stays unhandled. Glyphs, fonts and drawing are not modeled.
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

pub const MESSAGE_WINDOWS: usize = 8;
pub const TEXT_OBJECTS: usize = 12;

/// Fields 0x14005d3b0 sets from the creating command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageWindow {
    /// +0: index into the text object table.
    pub text: usize,
    /// +12/+16 (also copied to +2472/+2476 and +2480/+2484).
    pub position: [i32; 2],
    /// +20/+24.
    pub size: [i32; 2],
}

/// Script-supplied text object fields. The constructor and 0x14005d3b0 also set fixed
/// defaults (documented in vm-semantics.md) that are not modeled until a reader is verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextObject {
    /// Configuration words 4/5 (+552/+556): the script-space size that 0x14005aac0 divides
    /// by when scaling positions to the device size in words 2/3.
    pub logical_size: [i32; 2],
    /// +8/+12, set by 0x14005aac0.
    pub position: [i32; 2],
    /// +16..+28, set by 0x14005ab00 as (0, 0, width, height).
    pub area: [i32; 4],
}

#[derive(Debug, Default)]
pub struct MessageHost {
    windows: [Option<MessageWindow>; MESSAGE_WINDOWS],
    texts: [Option<TextObject>; TEXT_OBJECTS],
    /// +4224: created on first use of a text index and never freed by these commands.
    fonts: [bool; TEXT_OBJECTS],
}

impl MessageHost {
    pub fn window(&self, slot: usize) -> Option<&MessageWindow> {
        self.windows.get(slot)?.as_ref()
    }
    pub fn text(&self, index: usize) -> Option<&TextObject> {
        self.texts.get(index)?.as_ref()
    }
    pub fn font_present(&self, index: usize) -> bool {
        self.fonts.get(index).copied().unwrap_or(false)
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        match vm.pending_system() {
            Some((pc, 0x141)) => self.destroy(vm, pc).map(|()| true),
            Some((pc, 0x15d)) => self.create(vm, pc).map(|()| true),
            _ => Ok(false),
        }
    }
    /// 0x2141 / 0x140075f70: SP-4 slot compared signed against 8. A present window first
    /// frees the text object its +0 word names (destructor 0x14005c710) and clears that
    /// entry, then frees itself (0x14005e1e0 releases its +2520 scene object, which is not
    /// in the +1976 table) and clears the slot; empty slots change nothing. Returns 0x4004.
    /// Slots >= 8 only set +12056 bit 0x10000 (unmodeled); negative slots would index
    /// before the table.
    fn destroy(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let slot = window_slot(vm, pc, 0x2141)?;
        vm.complete_system(SystemResult(0x4004))?;
        if let Some(window) = self.windows[slot].take() {
            self.texts[window.text] = None;
        }
        Ok(())
    }
    /// 0x215d / 0x140075ce0: SP-4 window slot (signed < 8), SP-8 text index (signed < 12),
    /// SP-12/SP-16 position, SP-20/SP-24 size, SP-28/SP-32 logical size. It frees the
    /// slot's old window but not that window's text object, frees the text object at the
    /// new index, creates the font object there if absent, then builds the text object and
    /// the window. Returns 0x4020. Out-of-range indices only set +12056 bits 0x10000 and
    /// 0x1000 (unmodeled).
    fn create(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let slot = window_slot(vm, pc, 0x215d)?;
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map(|value| value as i32)
                .map_err(|e| error(pc, 0x215d, e.0))
        };
        let text = word(8)?;
        if text < 0 {
            return Err(error(
                pc,
                0x215d,
                "negative text object index indexes before the table",
            ));
        }
        let text = text as usize;
        if text >= TEXT_OBJECTS {
            return Err(error(
                pc,
                0x215d,
                "text object index outside 0..11 sets an unmodeled flag",
            ));
        }
        let [x, y, width, height, logical_width, logical_height] =
            [12, 16, 20, 24, 28, 32].map(word);
        let (position, size) = ([x?, y?], [width?, height?]);
        let logical_size = [logical_width?, logical_height?];
        // The original frees the text object without clearing other windows' +2528 cache.
        if (0..MESSAGE_WINDOWS)
            .any(|other| other != slot && self.window(other).is_some_and(|w| w.text == text))
        {
            return Err(error(
                pc,
                0x215d,
                "replacing a text object another window uses leaves a dangling pointer",
            ));
        }
        vm.complete_system(SystemResult(0x4020))?;
        self.fonts[text] = true;
        self.texts[text] = Some(TextObject {
            logical_size,
            position,
            area: [0, 0, size[0], size[1]],
        });
        self.windows[slot] = Some(MessageWindow {
            text,
            position,
            size,
        });
        Ok(())
    }
}

/// SP-4 compared signed against 8, as both handlers do.
fn window_slot(vm: &Machine<'_>, pc: usize, op: u16) -> Result<usize, DecodeError> {
    let slot = vm.stack().peek_u32(4).map_err(|e| error(pc, op, e.0))? as i32;
    if slot < 0 {
        return Err(error(
            pc,
            op,
            "negative message window slot indexes before the table",
        ));
    }
    if slot as usize >= MESSAGE_WINDOWS {
        return Err(error(
            pc,
            op,
            "message window slot outside 0..7 sets an unmodeled flag",
        ));
    }
    Ok(slot as usize)
}
