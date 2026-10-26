//! Message windows; addresses refer to the documented binary hash.
//!
//! Engine +4064 holds eight message window pointers (0x9f0-byte objects, constructor
//! 0x14005cf70), +4128 twelve text objects (0x250 bytes, constructor 0x14005c550) and +4224
//! twelve font objects (0x70 bytes, 0x140059990); the engine constructor 0x140014e60 zeroes
//! all three tables. A window's first word (+0) names its text object, and +2528 caches that
//! object's pointer. 0x215d creates windows and 0x2141 destroys them. 0x2140 (0x140075a50)
//! differs from 0x215d only in taking the logical size from the device, which is not
//! modeled, so it stays unhandled. 0x215b and 0x2142..0x2149 set window and text object
//! fields; 0x2170, 0x2178 and 0x217f drive the window's own scene object. Glyphs, fonts and
//! drawing are not modeled.
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    image::ImageSource,
    machine::Machine,
    scene::{self, SceneObject},
    vm::SystemResult,
};
use std::error::Error;

pub const MESSAGE_WINDOWS: usize = 8;
pub const TEXT_OBJECTS: usize = 12;

/// Fields 0x14005d3b0 sets from the creating command, plus later setters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageWindow {
    /// +0: index into the text object table.
    pub text: usize,
    /// +12/+16 (also copied to +2472/+2476 and +2480/+2484).
    pub position: [i32; 2],
    /// +20/+24.
    pub size: [i32; 2],
    /// +2488 (0x215b): while set, 0x14005de50 moves the window between the +2472 and
    /// +2480 positions; zeroed by the constructor's memset.
    pub position_switch: bool,
    /// +2520: the window's own scene object (0x14003a3c0), outside the +1976 table.
    pub object: SceneObject,
}

/// Text object fields that scripts set; the layout readers are 0x14005b6b0 and
/// 0x14005bc90. Unlisted constructor/0x14005d3b0 defaults are documented in vm-semantics.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextObject {
    /// Configuration words 4/5 (+552/+556): the script-space size that 0x14005aac0 divides
    /// by when scaling positions to the device size in words 2/3.
    pub logical_size: [i32; 2],
    /// +8/+12, set by 0x14005aac0.
    pub position: [i32; 2],
    /// +16..+28: text area x, y, width, height (0x14005ab00).
    pub area: [i32; 4],
    /// +156/+160: pen position, reset to the area origin by 0x14005ab00.
    pub cursor: [i32; 2],
    /// +32: glyph size; one character advances size + spacing[0], one line size + spacing[1].
    pub glyph_size: i32,
    /// +36/+40: character and line spacing.
    pub spacing: [i32; 2],
    /// +116: text colour (0xRRGGBB).
    pub color: u32,
    /// +120: decoration colour.
    pub decoration_color: u32,
    /// +132: decoration style 0..3 passed to glyph rendering.
    pub decoration: i32,
    /// +136/+140: per-character reveal mode (1 = alpha from 0, 2 = offset) and duration.
    pub reveal: [i32; 2],
    /// +144: per-character delay factor used by 0x14005bc90.
    pub char_delay: i32,
    /// +152: scene layer for the text and glyph objects (0x14003b0d0, as for 0x2047).
    pub layer: i32,
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
            Some((pc, command @ (0x142..=0x149 | 0x15b))) => {
                self.set(vm, pc, command).map(|()| true)
            }
            Some((pc, command @ (0x178 | 0x17f))) => self.object(vm, pc, command).map(|()| true),
            _ => Ok(false),
        }
    }
    /// 0x2178 / 0x140077180 (0x14005d2a0) and 0x217f / 0x1400773d0: SP-4 window slot, SP-8
    /// child (signed). 0x2178 with a negative child destroys and rebuilds the +2520 object;
    /// otherwise it rebuilds that child as 0x2022 does (0x14003ba60). Either target then
    /// gets +6244 = 0 (hidden). 0x217f clears +6248 bit 0x100 on the object (negative
    /// child) or an existing child. Both pop 8 bytes. Missing windows or children only set
    /// +12056 bit 0x10000; 0x2178 with a child >= 768 would dereference a null child.
    fn object(&mut self, vm: &mut Machine<'_>, pc: usize, command: u16) -> Result<(), DecodeError> {
        let op = command + 0x2000;
        let slot = window_slot(vm, pc, op)?;
        let child = vm.stack().peek_u32(8).map_err(|e| error(pc, op, e.0))? as i32;
        let window = self.windows[slot].as_ref().ok_or_else(|| {
            error(
                pc,
                op,
                "message window slot without a window sets an unmodeled flag",
            )
        })?;
        let child = match u16::try_from(child) {
            Err(_) if child < 0 => None,
            Ok(id)
                if id < 768 && (command == 0x178 || window.object.children.contains_key(&id)) =>
            {
                Some(id)
            }
            _ => return Err(error(pc, op, "window child outside 0..767 or missing")),
        };
        vm.complete_system(SystemResult(0x4008))?;
        let object = &mut self.windows[slot].as_mut().expect("checked above").object;
        match (command, child) {
            (0x178, None) => {
                *object = SceneObject {
                    shown: false,
                    ..SceneObject::default()
                };
            }
            (0x178, Some(id)) => {
                let flags = if object.draw.is_some() {
                    0x7fff_ffff
                } else {
                    0
                };
                object.children.insert(
                    id,
                    SceneObject {
                        flags,
                        shown: false,
                        ..SceneObject::default()
                    },
                );
                object.flags |= 0x8000_0000;
            }
            (_, None) => object.flags &= !0x100,
            (_, Some(id)) => {
                object.children.get_mut(&id).expect("checked above").flags &= !0x100;
            }
        }
        Ok(())
    }
    /// 0x2170 / 0x140076f00: SP-4 window slot, SP-8 file name. Releases the +2520 object's
    /// image (0x14003aa30) and reloads it with 0x14003bb10 as 0x2030 does, without a system
    /// result or the +12032 hook; returns 0x4008. A quiet decode failure only sets +12056
    /// bit 0x80 (unmodeled), so it is reported like the missing-window case.
    pub fn try_handle_image(
        &mut self,
        vm: &mut Machine<'_>,
        images: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, 0x170)) = vm.pending_system() else {
            return Ok(false);
        };
        let slot = window_slot(vm, pc, 0x2170)?;
        let raw = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x2170, e.0))?;
        let name = vm.resolve_string(raw)?;
        if self.windows[slot].is_none() {
            return Err(error(
                pc,
                0x2170,
                "message window slot without a window sets an unmodeled flag",
            )
            .into());
        }
        let image = scene::load_image(images, name, pc, 0x2170)?.ok_or_else(|| {
            error(
                pc,
                0x2170,
                "window image decode failure sets an unmodeled flag",
            )
        })?;
        vm.complete_system(SystemResult(0x4008))?;
        self.windows[slot]
            .as_mut()
            .expect("checked above")
            .object
            .image = Some(image);
        Ok(true)
    }
    /// 0x215b / 0x140076b80 and 0x2142..0x2149 / 0x140076060..0x140076370: SP-4 window
    /// slot (signed < 8, window present, otherwise only +12056 bit 0x10000 is set), then
    /// one, two or four argument words for a window or text object setter. Each pops all
    /// of its words. The text setters go through the window's +2528 pointer:
    /// - 0x215b: window +2488 = bool(SP-8) (0x14005de40).
    /// - 0x2142: +152 layer, also on the scene object (0x14005ac70).
    /// - 0x2143: +32 if SP-8 > 0; 0x2144: +36/+40 for each of SP-8/SP-12 >= 0 (0x14005ab20).
    /// - 0x2145: area = SP-8..SP-20 and cursor = SP-8/SP-12 (0x14005ab00).
    /// - 0x2146: +116/+120 for each of SP-8/SP-12 whose top byte is zero (0x14005ac00).
    /// - 0x2147: +144 (0x14002e850); 0x2148: +136/+140 (0x14005ac40); 0x2149: +132.
    fn set(&mut self, vm: &mut Machine<'_>, pc: usize, command: u16) -> Result<(), DecodeError> {
        let op = command + 0x2000;
        let slot = window_slot(vm, pc, op)?;
        let Some(text_index) = self.windows[slot].as_ref().map(|window| window.text) else {
            return Err(error(
                pc,
                op,
                "message window slot without a window sets an unmodeled flag",
            ));
        };
        let count = match command {
            0x145 => 4,
            0x144 | 0x146 | 0x148 => 2,
            _ => 1,
        };
        let mut args = [0i32; 4];
        for (i, arg) in args.iter_mut().take(count).enumerate() {
            *arg = vm
                .stack()
                .peek_u32(8 + 4 * i)
                .map_err(|e| error(pc, op, e.0))? as i32;
        }
        let Some(text) = self.texts[text_index].as_mut() else {
            return Err(error(pc, op, "message window without its text object"));
        };
        vm.complete_system(SystemResult(0x4004 + 4 * count as u16))?;
        let [a, b, c, d] = args;
        match command {
            0x15b => {
                if let Some(window) = self.windows[slot].as_mut() {
                    window.position_switch = a != 0;
                }
            }
            0x142 => text.layer = a,
            0x143 if a > 0 => text.glyph_size = a,
            0x144 => {
                for (spacing, value) in text.spacing.iter_mut().zip([a, b]) {
                    if value >= 0 {
                        *spacing = value;
                    }
                }
            }
            0x145 => {
                text.area = [a, b, c, d];
                text.cursor = [a, b];
            }
            0x146 => {
                for (color, value) in [&mut text.color, &mut text.decoration_color]
                    .into_iter()
                    .zip([a, b])
                {
                    if value as u32 & 0xff00_0000 == 0 {
                        *color = value as u32;
                    }
                }
            }
            0x147 => text.char_delay = a,
            0x148 => text.reveal = [a, b],
            0x149 => text.decoration = a,
            _ => {}
        }
        Ok(())
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
        // 0x14005d3b0's setter calls override the constructor values (e.g. +144 30 -> 100).
        self.texts[text] = Some(TextObject {
            logical_size,
            position,
            area: [0, 0, size[0], size[1]],
            cursor: [0, 0],
            glyph_size: 27,
            spacing: [0, 0],
            color: 0xff_ffff,
            decoration_color: 0xff_ffff,
            decoration: 0,
            reveal: [0, 0],
            char_delay: 100,
            layer: 1,
        });
        self.windows[slot] = Some(MessageWindow {
            text,
            position,
            size,
            position_switch: false,
            object: SceneObject::default(),
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
