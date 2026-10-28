//! Message windows; addresses refer to the documented binary hash.
//!
//! Engine +4064 holds eight message window pointers (0x9f0-byte objects, constructor
//! 0x14005cf70), +4128 twelve text objects (0x250 bytes, constructor 0x14005c550) and +4224
//! twelve font objects (0x70 bytes, 0x140059990); the engine constructor 0x140014e60 zeroes
//! all three tables. A window's first word (+0) names its text object, and +2528 caches that
//! object's pointer. 0x215d creates windows and 0x2141 destroys them. 0x2140 (0x140075a50)
//! differs from 0x215d only in taking the logical size from the device, which is not
//! modeled, so it stays unhandled. 0x215b and 0x2142..0x2149 set window and text object
//! fields; 0x2170/0x2171, 0x2178..0x217b, 0x217f and 0x2180..0x2185 drive the window's own
//! scene object; 0x2190..0x2192 fill its 32 button records and 0x2150 moves the window;
//! 0x21cf/0x21d0 build frame animations. 0x2100 creates a text object without a window;
//! 0x2101..0x2113, 0x2114..0x2117 and 0x211e address text objects directly. Glyph records and font/layout metadata live in core; font rasterization
//! belongs to the desktop backend.
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    image::ImageSource,
    input::ActionInput,
    machine::Machine,
    scene::{self, ANIMATION_FRAMES, AnimationFrame, DrawSetter, DrawState, SceneObject},
    vm::SystemResult,
};
use std::error::Error;
mod measure;
mod output;
mod wait;
pub use output::RecordedGlyph;

pub const MESSAGE_WINDOWS: usize = 8;
/// Screen info +8/+12 (configuration words 0/1 of every window), which is not modeled; the
/// research viewport stands in for it, as everywhere else in this project.
pub const RESEARCH_SCREEN: [u32; 2] = [1280, 720];
pub const TEXT_OBJECTS: usize = 12;
pub const WINDOW_BUTTONS: usize = 32;
/// State groups that fit before the hit rectangle in a 76-byte record.
pub const BUTTON_STATES: usize = 5;

/// One 76-byte record at window +40 + 76 * index; the constructor's memset zeroes it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowButton {
    /// Record +0: 0x2192 sets it (0x14005df70); hit testing (0x14005dff0) skips clear records.
    pub enabled: bool,
    /// Record +4 + 12 * state: six 16-bit words per state from 0x2190 (0x14005dee0). Their
    /// reader is not verified; on the real path they match the button child's source
    /// rectangle and position.
    pub states: [[u16; 6]; BUTTON_STATES],
    /// Record +64..+70: hit rectangle x, y, width, height relative to the window position,
    /// read unsigned by 0x14005dff0 (0x2191 / 0x14005df40).
    pub hit: [u16; 4],
}

/// Fields 0x14005d3b0 sets from the creating command, plus later setters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageWindow {
    /// +0: index into the text object table.
    pub text: usize,
    /// +4 (constructor 0): shown, toggled by 0x214e.
    pub visible: bool,
    /// +8 (constructor 0): hidden for a while although shown (0x214c / 0x14005dbc0), which
    /// hides the text and +2520 objects; 0x214d reports it and 0x214e clears it.
    pub hidden: bool,
    /// +12/+16, the current position.
    pub position: [i32; 2],
    /// The positions 0x14005de50 switches between: [0] = +2480/+2484 (switch state 0, also
    /// moved by 0x2150) and [1] = +2472/+2476 (state 1). Creation sets both to `position`.
    pub switch_positions: [[i32; 2]; 2],
    /// +20/+24.
    pub size: [i32; 2],
    /// +28 (0x2151, zeroed by the constructor): wait cursor mode. With 3, the window updates
    /// 0x14005e220/0x14005d4e0 place children 2 and 3 at the text pen plus `cursor_offset`;
    /// no reader treats the other modes specially.
    pub cursor_mode: u16,
    /// +30/+32: wait cursor offset, kept for modes 1 and 3 and cleared for 0 and 2.
    pub cursor_offset: [i16; 2],
    /// +2488 (0x215b): while set, 0x14005de50 moves the window between the +2472 and
    /// +2480 positions; zeroed by the constructor's memset.
    pub position_switch: bool,
    /// +2520: the window's own scene object (0x14003a3c0), outside the +1976 table. Button
    /// record `i` shows child `i + 16`.
    pub object: SceneObject,
    pub buttons: [WindowButton; WINDOW_BUTTONS],
    /// +2536: button pressed on action 0's down edge; constructor -1.
    pub pressed_button: i32,
}

/// 0x211e line-break rules (+328 set, +332, +336, +400), used by 0x14005b120 when the pen
/// reaches the end of a line and +112 is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineBreakRules {
    /// +332: how many consecutive characters may run past the line end.
    pub max_overflow: i32,
    /// +336: characters that may hang past the line end (0x14005b050 returns 1).
    pub hanging: Vec<u8>,
    /// +400: characters kept on the line while they repeat the previous one (returns 2).
    pub inseparable: Vec<u8>,
}

/// Text object fields that scripts set; the layout readers are 0x14005b6b0 and
/// 0x14005bc90. Unlisted constructor/0x14005d3b0 defaults are documented in vm-semantics.md.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextObject {
    /// Uniform scale of the text's scene object, kept as f32 bits (0x20ec).
    pub scale_bits: u32,
    /// Configuration words 4/5 (+552/+556): the script-space size that 0x14005aac0 divides
    /// by when scaling positions to the device size in words 2/3.
    pub logical_size: [i32; 2],
    /// +8/+12, set by 0x14005aac0.
    pub position: [i32; 2],
    /// +16..+28: text area x, y, width, height (0x14005ab00).
    pub area: [i32; 4],
    /// +156/+160: pen position, reset to the area origin by 0x14005ab00.
    pub cursor: [i32; 2],
    /// +196..+323: ruby-line flags. Zero until text output marks a line.
    pub ruby_lines: [u32; 32],
    /// +528/+532: overflow count and previous 16-bit character, reset by measurement.
    pub layout_scratch: [u32; 2],
    /// Explicit research records, without glyph pixels or a font device.
    pub glyphs: Vec<RecordedGlyph>,
    /// +32: glyph size; one character advances size + spacing[0], one line size + spacing[1].
    pub glyph_size: i32,
    /// +36/+40: character and line spacing.
    pub spacing: [i32; 2],
    /// +116: text colour (0xRRGGBB).
    pub color: u32,
    /// +120: decoration colour.
    pub decoration_color: u32,
    /// +124 (constructor 2, set by 0x2113): outline mode 0..2 that 0x14005bc90 copies into
    /// glyph descriptor word 8 for direct output (0x210a). Window output passes its own
    /// setting instead (0x14005d870), so window texts do not read it.
    pub outline_mode: u32,
    /// +128: raw glyph parameter set by 0x2155, copied into glyph descriptor word 9
    /// by 0x14005bc90. Its rendering interpretation is not modeled yet.
    pub glyph_parameter: u32,
    /// +132: decoration style 0..3 passed to glyph rendering.
    pub decoration: i32,
    /// +136/+140: per-character reveal mode (1 = alpha from 0, 2 = offset) and duration.
    pub reveal: [i32; 2],
    /// +144: per-character delay factor used by 0x14005bc90.
    pub char_delay: i32,
    /// +152: scene layer for the text and glyph objects (0x14003b0d0, as for 0x2047).
    pub layer: i32,
    /// +148 (constructor 1): shown, also copied to the text scene object (0x14005ac80).
    pub visible: bool,
    /// +176: ruby layout; 0x14005b360 marks lines holding `{base/ruby}` groups.
    pub ruby: bool,
    /// +184: reserve the ruby height above every line, not only marked ones.
    pub ruby_every_line: bool,
    /// +188 (constructor 12): ruby glyph size, also the height reserved above a line.
    pub ruby_size: i32,
    /// +192: extra spacing between ruby glyphs.
    pub ruby_spacing: i32,
    /// None until 0x211e; the constructor zeroes +328.
    pub line_break: Option<LineBreakRules>,
}

#[derive(Debug, Default)]
pub struct MessageHost {
    record_output: bool,
    windows: [Option<MessageWindow>; MESSAGE_WINDOWS],
    texts: [Option<TextObject>; TEXT_OBJECTS],
    /// +4224: created on first use of a text index and never freed by these commands.
    fonts: [bool; TEXT_OBJECTS],
}

impl MessageHost {
    /// Explicit research mode only: records the supported single-line 0x2152 branch,
    /// without font rasterization. Default false preserves the output barrier.
    pub fn set_record_output(&mut self, enabled: bool) {
        self.record_output = enabled;
    }
    pub fn window(&self, slot: usize) -> Option<&MessageWindow> {
        self.windows.get(slot)?.as_ref()
    }
    pub fn text(&self, index: usize) -> Option<&TextObject> {
        self.texts.get(index)?.as_ref()
    }
    pub fn font_present(&self, index: usize) -> bool {
        self.fonts.get(index).copied().unwrap_or(false)
    }
    /// The window part of the frame update 0x140015900: after the slot objects and text
    /// objects, each window's +2520 object runs 0x14003d0c0. The window object itself is
    /// never destroyed here, only its marked children. Text object updates (0x14005ae70)
    /// are not modeled.
    pub fn advance_frame(&mut self, delta: u32) {
        for text in self.texts.iter_mut().flatten() {
            for glyph in &mut text.glyphs {
                glyph.advance(delta);
            }
        }
        for window in self.windows.iter_mut().flatten() {
            window.object.advance_effects(delta);
        }
    }
    /// 0x215f / 0x140076be0 -> 0x14005dd10: SP-4 window slot (a missing window or a slot of
    /// 8 or more only sets +12056 bit 0x10000), SP-8/SP-12 offset and SP-16/SP-20 size of a
    /// rectangle relative to the window position. The input pointer (+868/+872, 0x1400565a0)
    /// is scaled by configuration word 4 over word 0 (and 5 over 1) with unsigned arithmetic;
    /// system result 0 = whether it lies in [x, x + w) x [y, y + h). Pops 20.
    pub fn try_handle_pointer(
        &mut self,
        vm: &mut Machine<'_>,
        input: &ActionInput,
    ) -> Result<bool, DecodeError> {
        let Some((pc, 0x15f)) = vm.pending_system() else {
            return Ok(false);
        };
        let slot = window_slot(vm, pc, 0x215f)?;
        let mut args = [0i32; 4];
        for (i, arg) in args.iter_mut().enumerate() {
            *arg = vm
                .stack()
                .peek_u32(8 + 4 * i)
                .map_err(|e| error(pc, 0x215f, e.0))? as i32;
        }
        let window = self.windows[slot].as_ref().ok_or_else(|| {
            error(
                pc,
                0x215f,
                "message window slot without a window sets an unmodeled flag",
            )
        })?;
        let logical = self.texts[window.text]
            .as_ref()
            .ok_or_else(|| error(pc, 0x215f, "message window without its text object"))?
            .logical_size;
        let [px, py] = input.pointer();
        let scale = |value: i32, logical: i32, screen: u32| {
            ((value as u32).wrapping_mul(logical as u32) / screen) as i32
        };
        let (px, py) = (
            scale(px, logical[0], RESEARCH_SCREEN[0]),
            scale(py, logical[1], RESEARCH_SCREEN[1]),
        );
        let [x, y, w, h] = args;
        let left = x.wrapping_add(window.position[0]);
        let top = y.wrapping_add(window.position[1]);
        let hit = left <= px && top <= py && left.wrapping_add(w) > px && top.wrapping_add(h) > py;
        vm.complete_system(SystemResult(0x4014))?;
        vm.state_mut()
            .set_system_integer(0, u32::from(hit))
            .expect("slot 0 exists");
        Ok(true)
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        match vm.pending_system() {
            Some((pc, 0xec)) => self.scale(vm, pc).map(|()| true),
            Some((pc, 0x141)) => self.destroy(vm, pc).map(|()| true),
            Some((pc, 0x15d)) => self.create(vm, pc).map(|()| true),
            Some((pc, command @ (0x142..=0x149 | 0x14f | 0x15b))) => {
                self.set(vm, pc, command).map(|()| true)
            }
            Some((pc, command @ (0x178 | 0x17f))) => self.object(vm, pc, command).map(|()| true),
            Some((pc, 0x17b)) => self.object_presence(vm, pc).map(|()| true),
            Some((pc, 0x171)) => self.release_image(vm, pc).map(|()| true),
            Some((pc, 0x179)) => self.destroy_child(vm, pc).map(|()| true),
            Some((pc, command @ (0x17a | 0x17c | 0x180 | 0x182..=0x186))) => {
                self.draw(vm, pc, command).map(|()| true)
            }
            Some((pc, command @ 0x190..=0x192)) => self.button(vm, pc, command).map(|()| true),
            Some((pc, 0x150)) => self.move_window(vm, pc).map(|()| true),
            Some((pc, 0x151)) => self.cursor(vm, pc).map(|()| true),
            Some((pc, 0x155)) => self.glyph_parameter(vm, pc).map(|()| true),
            Some((pc, 0x11f)) => self.measure(vm, pc).map(|()| true),
            Some((pc, 0x14e)) => self.show(vm, pc).map(|()| true),
            Some((pc, command @ (0x14b | 0x14c))) => self.presence(vm, pc, command).map(|()| true),
            Some((pc, command @ (0x14a | 0x15a))) => {
                self.window_text(vm, pc, command).map(|()| true)
            }
            Some((pc, command @ (0x1cf..=0x1d1))) => self.animation(vm, pc, command).map(|()| true),
            Some((pc, command @ (0x114..=0x117 | 0x11e))) => {
                self.text_command(vm, pc, command).map(|()| true)
            }
            Some((pc, 0x100)) => self.create_text(vm, pc).map(|()| true),
            Some((pc, 0x10b)) if self.record_output => self.place_text(vm, pc).map(|()| true),
            Some((pc, command @ (0x101..=0x105 | 0x107 | 0x10c..=0x110 | 0x113 | 0x11a))) => {
                self.text_setter(vm, pc, command).map(|()| true)
            }
            _ => Ok(false),
        }
    }
    /// 0x2178 / 0x140077180 (0x14005d2a0) and 0x217f / 0x1400773d0: SP-4 window slot, SP-8
    /// child (signed). 0x2178 with a negative child destroys and rebuilds the +2520 object;
    /// otherwise it rebuilds that child as 0x2022 does (0x14003ba60). Either target then
    /// gets +6244 = 0 (hidden). 0x217f clears +6248 bit 0x100 on the object (negative
    /// child) or an existing child. Both pop 8 bytes. Missing windows or children only set
    /// +12056 bit 0x10000; 0x2178 with a child >= 768 would dereference a null child.
    /// 0x217a / 0x140077240, 0x2180 / 0x1400774b0 and 0x2182..0x2185 / 0x140077660..0x140077820:
    /// SP-4 window slot,
    /// SP-8 child selected by 0x14005d280 (negative = the +2520 object, else an existing
    /// child below 768), then arguments from SP-12. Missing windows or children only set
    /// +12056 bit 0x10000; a setter without a draw state would dereference null.
    /// - 0x217a: +6244 shown = bool(SP-12) (0x14003a6f0); pops 12, needs no draw state.
    /// - 0x217c / 0x140077340: +6256/+6260 logical size = SP-12/SP-16 (0x14003b410); pops
    ///   16, needs no draw state.
    /// - 0x2180: rebuild the draw state (0x14003c7a0, as 0x2040) and zero its coordinate
    ///   mode word (0x14003b130); pops 8 bytes.
    /// - 0x2182: source rectangle (0x14003b070, as 0x2044); pops 24.
    /// - 0x2183: position (0x14003b0b0, as 0x2045); 0x2185: origin (0x14003b0a0, as 0x2046);
    ///   both pop 16.
    /// - 0x2184: layer (0x14003b0d0, as 0x2047); 0x2186 / 0x140077970: opacity
    ///   (0x14003b0e0, as 0x2048); both pop 12.
    fn draw(&mut self, vm: &mut Machine<'_>, pc: usize, command: u16) -> Result<(), DecodeError> {
        let op = command + 0x2000;
        let slot = window_slot(vm, pc, op)?;
        let count = match command {
            0x180 => 0,
            0x182 => 4,
            0x17a | 0x184 | 0x186 => 1,
            _ => 2,
        };
        let mut args = [0u32; 4];
        for (i, arg) in args.iter_mut().take(count).enumerate() {
            *arg = vm
                .stack()
                .peek_u32(12 + 4 * i)
                .map_err(|e| error(pc, op, e.0))?;
        }
        let child = vm.stack().peek_u32(8).map_err(|e| error(pc, op, e.0))? as i32;
        let target = self.windows[slot]
            .as_ref()
            .and_then(|window| window_target(&window.object, child))
            .ok_or_else(|| error(pc, op, "message window or window child does not exist"))?;
        if !matches!(command, 0x17a | 0x17c | 0x180) && target.draw.is_none() {
            return Err(error(pc, op, "window object has no draw state"));
        }
        vm.complete_system(SystemResult(0x4008 + 4 * count as u16))?;
        let window = self.windows[slot].as_mut().expect("checked above");
        let target = window_target_mut(&mut window.object, child).expect("checked above");
        let [a, b, c, d] = args;
        let setter = match command {
            0x17a => {
                target.shown = a != 0;
                return Ok(());
            }
            0x17c => {
                target.logical_size = [a, b];
                return Ok(());
            }
            0x180 => {
                let mut draw = DrawState::default();
                draw.words[0] = 0;
                target.draw = Some(draw);
                return Ok(());
            }
            0x182 => DrawSetter::Source([a, b, c, d]),
            0x183 => DrawSetter::Position([a, b]),
            0x184 => DrawSetter::Layer(a),
            0x186 => DrawSetter::Opacity(a),
            _ => DrawSetter::Origin([a, b]),
        };
        target.draw.as_mut().expect("checked above").set(setter);
        Ok(())
    }
    /// 0x2190 / 0x1400786c0, 0x2191 / 0x140078750 and 0x2192 / 0x1400787d0: SP-4 window
    /// slot, SP-8 record index (signed < 32; otherwise, or without a window, only +12056
    /// bit 0x20000 is set). 0x2190 stores six 16-bit words SP-16..SP-36 in state group
    /// SP-12, which the original does not bound (Rust accepts 0..5); pops 36. 0x2191 stores
    /// the hit rectangle SP-12..SP-24; pops 24. 0x2192 enables the record and shows window
    /// child index + 16 if it exists; pops 8.
    fn button(&mut self, vm: &mut Machine<'_>, pc: usize, command: u16) -> Result<(), DecodeError> {
        let op = command + 0x2000;
        let slot = window_slot(vm, pc, op)?;
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, op, e.0))
        };
        let index = word(8)? as i32;
        if index < 0 {
            return Err(error(
                pc,
                op,
                "negative window button indexes before the records",
            ));
        }
        let index = index as usize;
        if index >= WINDOW_BUTTONS || self.windows[slot].is_none() {
            return Err(error(
                pc,
                op,
                "window button outside 0..31 or without a window sets an unmodeled flag",
            ));
        }
        let count = match command {
            0x190 => 7,
            0x191 => 4,
            _ => 0,
        };
        let mut args = [0u32; 7];
        for (i, arg) in args.iter_mut().take(count).enumerate() {
            *arg = word(12 + 4 * i)?;
        }
        if command == 0x190 && args[0] as usize >= BUTTON_STATES {
            return Err(error(
                pc,
                op,
                "window button state outside 0..4 overlaps other record fields",
            ));
        }
        vm.complete_system(SystemResult(0x4008 + 4 * count as u16))?;
        let window = self.windows[slot].as_mut().expect("checked above");
        let button = &mut window.buttons[index];
        // The records keep 16-bit words; higher bits of each argument are dropped.
        match command {
            0x190 => {
                for (field, value) in button.states[args[0] as usize].iter_mut().zip(&args[1..]) {
                    *field = *value as u16;
                }
            }
            0x191 => {
                for (field, value) in button.hit.iter_mut().zip(&args) {
                    *field = *value as u16;
                }
            }
            _ => {
                button.enabled = true;
                if let Some(child) = window.object.children.get_mut(&(index as u16 + 16)) {
                    child.shown = true;
                }
            }
        }
        Ok(())
    }
    /// 0x2150 / 0x140076640 -> 0x14005dc90: SP-4 window slot, SP-8 mode, SP-12/SP-16 x/y.
    /// Mode 0 takes the point as is and mode 1 adds the current +12/+16; other modes read an
    /// uninitialized stack slot. The result becomes +12/+16 and +2480/+2484 (not +2472), the
    /// +2520 object position (0x14003b0b0, which dereferences its draw state) and the text
    /// object position (0x14005aac0). Pops 16 bytes; a missing window only sets +12056 bit
    /// 0x10000.
    /// 0x2114..0x2117 / 0x140072d50..0x140072e70 and 0x211e / 0x140073130 address a text
    /// object directly: SP-4 index (signed < 12 with an object, otherwise only +12056 bit
    /// 0x1000 is set).
    /// - 0x2114: +176 ruby = bool(SP-8); 0x2115: +188 ruby size; 0x2116: +192 ruby spacing;
    ///   0x2117: +184 = bool(SP-8). Each pops 8 bytes.
    /// - 0x211e (0x14005afd0): copies the strings SP-8 and SP-12 into the 64-byte +336 and
    ///   +400 buffers with lstrcpyA, sets +332 = SP-16, clears +464 and the +528/+532 layout
    ///   counters and sets +328 = 1; pops 16.
    fn text_command(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<(), DecodeError> {
        let op = command + 0x2000;
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, op, e.0))
        };
        let index = word(4)? as i32;
        if index < 0 {
            return Err(error(
                pc,
                op,
                "negative text object index indexes before the table",
            ));
        }
        let index = index as usize;
        let rules = if command == 0x11e {
            let hanging = vm.resolve_string(word(8)?)?;
            let inseparable = vm.resolve_string(word(12)?)?;
            if hanging.len() >= 64 || inseparable.len() >= 64 {
                return Err(error(
                    pc,
                    op,
                    "line-break string overflows its 64-byte buffer",
                ));
            }
            Some(LineBreakRules {
                max_overflow: word(16)? as i32,
                hanging,
                inseparable,
            })
        } else {
            None
        };
        let value = word(8)?;
        if self.text(index).is_none() {
            return Err(error(
                pc,
                op,
                "text object index outside 0..11 or without an object sets an unmodeled flag",
            ));
        }
        vm.complete_system(SystemResult(if rules.is_some() { 0x4010 } else { 0x4008 }))?;
        let text = self.texts[index].as_mut().expect("checked above");
        match command {
            0x114 => text.ruby = value != 0,
            0x115 => text.ruby_size = value as i32,
            0x116 => text.ruby_spacing = value as i32,
            0x117 => text.ruby_every_line = value != 0,
            _ => text.line_break = rules,
        }
        Ok(())
    }
    /// 0x2100 / 0x140072320: SP-4 text index (signed >= 12 only sets +12056 bit 0x1000),
    /// SP-8 font face string, SP-12 glyph capacity. It creates the font object if absent,
    /// then overwrites the +4128 entry with a new text object without freeing the old one
    /// (0x215d frees it), and writes system integer 0 = created. Returns 0x400c.
    ///
    /// Constructor 0x14005c550 without 0x14005d3b0: the memset leaves position, area, pen,
    /// glyph size and spacing 0; colours 0xffffff, +144 delay 30, +148 shown, +152 layer 1,
    /// ruby size 12. The logical size (+552/+556) repeats screen info +8/+12, which the
    /// research viewport stands in for. The face is copied into +44 and selected on the
    /// font object (0x140059b90); only the default empty face is modeled. The capacity
    /// (+324) sizes the glyph texture in 0x14005a6a0: 0 and 196 (0x215d's value) both give
    /// a 1024x2048 texture; other capacities are rejected because textures are not modeled.
    /// Replacing a text object a window caches (+2528) would leave that window stale.
    fn create_text(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, 0x2100, e.0))
        };
        let index = word(4)? as i32;
        if index < 0 {
            return Err(error(
                pc,
                0x2100,
                "negative text object index indexes before the table",
            ));
        }
        let index = index as usize;
        if index >= TEXT_OBJECTS {
            return Err(error(
                pc,
                0x2100,
                "text object index outside 0..11 sets an unmodeled flag",
            ));
        }
        if !vm.resolve_string(word(8)?)?.is_empty() {
            return Err(error(pc, 0x2100, "font face selection is not modeled"));
        }
        if !matches!(word(12)?, 0 | 196) {
            return Err(error(
                pc,
                0x2100,
                "glyph texture capacity other than 0 or 196 is not modeled",
            ));
        }
        if (0..MESSAGE_WINDOWS).any(|slot| self.window(slot).is_some_and(|w| w.text == index)) {
            return Err(error(
                pc,
                0x2100,
                "replacing a text object a window uses leaves a dangling pointer",
            ));
        }
        vm.complete_system(SystemResult(0x400c))?;
        vm.state_mut()
            .set_system_integer(0, 1)
            .expect("slot 0 exists");
        self.fonts[index] = true;
        self.texts[index] = Some(TextObject {
            scale_bits: 1.0f32.to_bits(),
            logical_size: RESEARCH_SCREEN.map(|v| v as i32),
            position: [0, 0],
            area: [0; 4],
            cursor: [0, 0],
            ruby_lines: [0; 32],
            layout_scratch: [0; 2],
            glyphs: Vec::new(),
            glyph_size: 0,
            spacing: [0, 0],
            color: 0xff_ffff,
            decoration_color: 0xff_ffff,
            outline_mode: 2,
            glyph_parameter: 0,
            decoration: 0,
            reveal: [0, 0],
            char_delay: 30,
            layer: 1,
            visible: true,
            ruby: false,
            ruby_every_line: false,
            ruby_size: 12,
            ruby_spacing: 0,
            line_break: None,
        });
        Ok(())
    }
    /// 0x2101..0x2113 / 0x140072680..0x140073010 and 0x211a address a text object: SP-4 index
    /// (signed >= 12, or a missing object for the setters, only sets +12056 bit 0x1000).
    /// They reach the same callees as the window setters and return 0x4000 + 4 x words:
    /// - 0x2101: free the object (0x14005c710) and clear the entry; an empty entry is a
    ///   no-op and the font object stays. A window's +2528 cache would dangle.
    /// - 0x2102 (0x14005aac0): +8/+12 position; the scene object gets it scaled by screen
    ///   info over the logical size. 0x2103 (0x14005ab00): area SP-8..SP-20, pen = its
    ///   origin. 0x2104 (0x14005ad50): clear the glyphs and return the pen to the area
    ///   origin. 0x2105 (0x14005ab20): +32 if SP-8 > 0, +36/+40 for SP-12/SP-16 >= 0.
    /// - 0x2107 (0x14005ac00): colours whose top byte is zero. 0x210c (0x14005ac20): pen.
    ///   0x210d (0x14005ac40): reveal mode/duration. 0x210e (0x14002e850): +144 delay.
    ///   0x210f (0x14005ac30): +132 decoration. 0x2110 (0x14005ac70): +152 layer, also on
    ///   the scene object. 0x2113 (0x14005ac50): +124 outline mode, raw. 0x211a /
    ///   0x140073070 (0x14002e860, as 0x2155): +128 glyph parameter, raw.
    fn text_setter(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<(), DecodeError> {
        let op = command + 0x2000;
        let count = match command {
            0x101 | 0x104 => 1,
            0x10e | 0x10f | 0x110 | 0x113 | 0x11a => 2,
            0x102 | 0x107 | 0x10c | 0x10d => 3,
            0x105 => 4,
            _ => 5,
        };
        let mut words = [0i32; 5];
        for (i, word) in words.iter_mut().take(count).enumerate() {
            *word = vm
                .stack()
                .peek_u32(4 + 4 * i)
                .map_err(|e| error(pc, op, e.0))? as i32;
        }
        let [index, a, b, c, d] = words;
        if index < 0 {
            return Err(error(
                pc,
                op,
                "negative text object index indexes before the table",
            ));
        }
        let index = index as usize;
        if index >= TEXT_OBJECTS {
            return Err(error(
                pc,
                op,
                "text object index outside 0..11 sets an unmodeled flag",
            ));
        }
        if command == 0x101 {
            if self.texts[index].is_some()
                && (0..MESSAGE_WINDOWS)
                    .any(|slot| self.window(slot).is_some_and(|w| w.text == index))
            {
                return Err(error(
                    pc,
                    op,
                    "freeing a text object a window uses leaves a dangling pointer",
                ));
            }
            vm.complete_system(SystemResult(0x4004))?;
            self.texts[index] = None;
            return Ok(());
        }
        if self.texts[index].is_none() {
            return Err(error(
                pc,
                op,
                "text object index without an object sets an unmodeled flag",
            ));
        }
        vm.complete_system(SystemResult(0x4000 + 4 * count as u16))?;
        let text = self.texts[index].as_mut().expect("checked above");
        match command {
            0x102 => text.position = [a, b],
            0x103 => {
                text.area = [a, b, c, d];
                text.cursor = [a, b];
            }
            0x104 => {
                text.cursor = [text.area[0], text.area[1]];
                text.glyphs.clear();
            }
            0x105 => {
                if a > 0 {
                    text.glyph_size = a;
                }
                for (spacing, value) in text.spacing.iter_mut().zip([b, c]) {
                    if value >= 0 {
                        *spacing = value;
                    }
                }
            }
            0x107 => {
                for (color, value) in [&mut text.color, &mut text.decoration_color]
                    .into_iter()
                    .zip([a, b])
                {
                    if value as u32 & 0xff00_0000 == 0 {
                        *color = value as u32;
                    }
                }
            }
            0x10c => text.cursor = [a, b],
            0x10d => text.reveal = [a, b],
            0x10e => text.char_delay = a,
            0x10f => text.decoration = a,
            0x110 => text.layer = a,
            0x113 => text.outline_mode = a as u32,
            _ => text.glyph_parameter = a as u32,
        }
        Ok(())
    }
    /// 0x21cf / 0x140078210 and 0x21d0 / 0x140078280: SP-4 window slot, SP-8 child selected
    /// by 0x14005d280, then the draw state's +136 frame animation (0x14003b060 returns the
    /// draw state, which both dereference). Missing windows or children only set +12056 bit
    /// 0x10000.
    /// - 0x21cf (0x14001fc10): replace the animation with an empty, stopped one; pops 8.
    /// - 0x21d0 (0x14001fd60 -> 0x1400261b0): entry SP-12 gets next = SP-16, duration =
    ///   SP-20 and the 16-bit words SP-24..SP-44; no animation is a no-op. The original does
    ///   not bound the entry index (Rust accepts 0..63) and only reads past the table when
    ///   playback reaches a `next` >= 64, which Rust rejects here. Pops 44.
    /// - 0x21d1 / 0x140078380 (0x14001fd10 -> 0x140026200): play from frame SP-12 (Rust
    ///   accepts 0..63; the original does not bound it); no animation is a no-op. Pops 12.
    ///
    /// Stopping (0x21d2 / 0x140026110) is not on the real path yet and stays unhandled;
    /// `advance_frame` runs the per-frame update.
    fn animation(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<(), DecodeError> {
        let op = command + 0x2000;
        let slot = window_slot(vm, pc, op)?;
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, op, e.0))
        };
        let child = word(8)? as i32;
        let mut args = [0u32; 9];
        let count = match command {
            0x1d0 => 9,
            0x1d1 => 1,
            _ => 0,
        };
        for (i, arg) in args.iter_mut().take(count).enumerate() {
            *arg = word(12 + 4 * i)?;
        }
        if count > 0 && args[0] as usize >= ANIMATION_FRAMES {
            return Err(error(
                pc,
                op,
                "animation frame outside 0..63 overflows the original buffer",
            ));
        }
        if command == 0x1d0 && args[1] as i32 >= ANIMATION_FRAMES as i32 {
            return Err(error(
                pc,
                op,
                "next animation frame >= 64 would be read past the original buffer",
            ));
        }
        let target = self.windows[slot]
            .as_ref()
            .and_then(|window| window_target(&window.object, child))
            .ok_or_else(|| error(pc, op, "message window or window child does not exist"))?;
        if target.draw.is_none() {
            return Err(error(pc, op, "window object has no draw state"));
        }
        vm.complete_system(SystemResult(0x4008 + 4 * count as u16))?;
        let window = self.windows[slot].as_mut().expect("checked above");
        let draw = window_target_mut(&mut window.object, child)
            .and_then(|target| target.draw.as_mut())
            .expect("checked above");
        if command == 0x1cf {
            draw.animation = Some(Box::default());
        } else if command == 0x1d1 {
            if let Some(animation) = draw.animation.as_deref_mut() {
                animation.start(args[0] as usize, &mut draw.words);
            }
        } else if let Some(animation) = draw.animation.as_deref_mut() {
            let mut words = [0i16; 6];
            for (field, value) in words.iter_mut().zip(&args[3..]) {
                *field = *value as u16 as i16;
            }
            animation.frames[args[0] as usize] = AnimationFrame {
                next: args[1] as i32,
                words,
                duration: args[2],
            };
        }
        Ok(())
    }
    /// 0x2151 / 0x1400766a0 -> 0x14005db90: SP-4 window slot, then the low 16 bits of SP-8
    /// become +28; modes 1 and 3 store SP-12/SP-16 as 16-bit +30/+32, modes 0 and 2 clear
    /// them and other modes leave them. Pops 16; a missing window only sets +12056 bit
    /// 0x10000.
    /// SP-4 window slot for both; a missing window only sets +12056 bit 0x10000.
    /// - 0x214a / 0x1400763d0 -> 0x14005db70 -> 0x14005ad50: clear the window's text. The
    ///   original destroys the glyph objects (0x14003a880 on the +6152 list) and frees the
    ///   +168 glyph records, then resets the pen to the area origin. Rust holds no glyphs
    ///   yet, so only the pen moves. Pops 4.
    /// - 0x215a / 0x140076b20 -> 0x14005de30: +2472/+2476 = SP-8/SP-12, the switch state 1
    ///   position (the current position is not moved). Pops 12.
    fn window_text(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<(), DecodeError> {
        let op = command + 0x2000;
        let slot = window_slot(vm, pc, op)?;
        let point = if command == 0x15a {
            let word = |distance| {
                vm.stack()
                    .peek_u32(distance)
                    .map(|value| value as i32)
                    .map_err(|e| error(pc, op, e.0))
            };
            Some([word(8)?, word(12)?])
        } else {
            None
        };
        let Some(text) = self.windows[slot].as_ref().map(|window| window.text) else {
            return Err(error(
                pc,
                op,
                "message window slot without a window sets an unmodeled flag",
            ));
        };
        if self.texts[text].is_none() {
            return Err(error(pc, op, "message window without its text object"));
        }
        vm.complete_system(SystemResult(if point.is_some() { 0x400c } else { 0x4004 }))?;
        if let Some(point) = point {
            self.windows[slot]
                .as_mut()
                .expect("checked above")
                .switch_positions[1] = point;
        } else {
            let text = self.texts[text].as_mut().expect("checked above");
            text.cursor = [text.area[0], text.area[1]];
            text.glyphs.clear();
        }
        Ok(())
    }
    /// 0x1400731d0 -> 0x14005ca80: SP-4 text index, SP-8 string reference.
    /// System results 0/1/2/4 = success/max count/lines/last count; pops 8.
    fn measure(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, 0x211f, e.0))
        };
        let index = word(4)? as usize;
        let bytes = vm.resolve_string(word(8)?)?;
        let text = self.text(index).ok_or_else(|| {
            error(
                pc,
                0x211f,
                "text object index outside 0..11 or without an object sets an unmodeled flag",
            )
        })?;
        let (counts, scratch) = measure::measure(text, &bytes).map_err(|e| error(pc, 0x211f, e))?;
        vm.complete_system(SystemResult(0x4008))?;
        self.texts[index]
            .as_mut()
            .expect("checked above")
            .layout_scratch = scratch;
        for (slot, value) in [(0, 1), (1, counts[0]), (2, counts[1]), (4, counts[2])] {
            vm.state_mut()
                .set_system_integer(slot, value)
                .expect("known result slot");
        }
        Ok(())
    }
    /// 0x140076250 -> 0x14005db60 -> 0x14002e860: SP-4 window slot, SP-8
    /// raw u32 stored at text +128. The wrapper preserves edx despite the decompiler
    /// omitting that parameter. Missing windows set an unmodeled engine error flag.
    fn glyph_parameter(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let slot = window_slot(vm, pc, 0x2155)?;
        let value = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x2155, e.0))?;
        let text = self.windows[slot]
            .as_ref()
            .ok_or_else(|| {
                error(
                    pc,
                    0x2155,
                    "message window slot without a window sets an unmodeled flag",
                )
            })?
            .text;
        if self.texts[text].is_none() {
            return Err(error(pc, 0x2155, "message window without its text object"));
        }
        vm.complete_system(SystemResult(0x4008))?;
        self.texts[text]
            .as_mut()
            .expect("checked above")
            .glyph_parameter = value;
        Ok(())
    }
    /// 0x140076ea0 -> 0x14005ddc0: SP-4 slot, SP-8 f32 scale. Both axes of
    /// the window and text draw state receive it; [0.95, 1.05] snaps to 1.0.
    fn scale(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let slot = window_slot(vm, pc, 0x20ec)?;
        let raw = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x20ec, e.0))?;
        let window = self.windows[slot].as_ref().ok_or_else(|| {
            error(
                pc,
                0x20ec,
                "message window slot without a window sets an unmodeled flag",
            )
        })?;
        if window.object.draw.is_none() {
            return Err(error(
                pc,
                0x20ec,
                "window scale without draw state dereferences null",
            ));
        }
        let text = window.text;
        if self.texts[text].is_none() {
            return Err(error(pc, 0x20ec, "message window without its text object"));
        }
        let scale = f32::from_bits(raw);
        let bits = if (0.95..=1.05).contains(&scale) {
            1.0f32.to_bits()
        } else {
            raw
        };
        vm.complete_system(SystemResult(0x4008))?;
        self.windows[slot]
            .as_mut()
            .expect("checked above")
            .object
            .draw
            .as_mut()
            .expect("checked above")
            .words[18..20]
            .fill(bits);
        self.texts[text].as_mut().expect("checked above").scale_bits = bits;
        Ok(())
    }
    /// 0x214e / 0x140076540: SP-4 window slot, SP-8 shown (as bool). Only when it differs
    /// from +4, 0x14005da80 clears +8, sets +4, shows or hides the text object (+148 and its
    /// scene object, 0x14005ac80) and the +2520 object, and on showing moves the text object
    /// to the window position (0x14005aac0); +8 is cleared again. Pops 8. The original does
    /// not check the window pointer, and for slots >= 8 it sets +12056 bit 0x10000 and still
    /// writes past the table, so both are errors here.
    fn show(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let slot = window_slot(vm, pc, 0x214e)?;
        let shown = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x214e, e.0))? != 0;
        let Some(text) = self.windows[slot].as_ref().map(|window| window.text) else {
            return Err(error(
                pc,
                0x214e,
                "showing a missing message window dereferences null",
            ));
        };
        if self.texts[text].is_none() {
            return Err(error(pc, 0x214e, "message window without its text object"));
        }
        vm.complete_system(SystemResult(0x4008))?;
        let window = self.windows[slot].as_mut().expect("checked above");
        if window.visible != shown {
            window.visible = shown;
            window.hidden = false;
            window.object.shown = shown;
            let position = window.position;
            let text = self.texts[text].as_mut().expect("checked above");
            text.visible = shown;
            if shown {
                text.position = position;
            }
        }
        Ok(())
    }
    /// SP-4 window slot, compared signed against 8 (larger ones only set +12056 bit 0x10000).
    /// - 0x214b / 0x140076430: system result 0 = whether the slot holds a window. Pops 4.
    /// - 0x214c / 0x1400764e0 -> 0x14005dbc0: for a shown window (+4), +8 = bool(SP-8) and
    ///   the text object and +2520 object are shown exactly when it is clear; a window that
    ///   is not shown is left alone. A missing window only sets the flag. Pops 8.
    fn presence(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<(), DecodeError> {
        let op = command + 0x2000;
        let slot = window_slot(vm, pc, op)?;
        if command == 0x14b {
            vm.complete_system(SystemResult(0x4004))?;
            vm.state_mut()
                .set_system_integer(0, u32::from(self.windows[slot].is_some()))
                .expect("slot 0 exists");
            return Ok(());
        }
        let hidden = vm.stack().peek_u32(8).map_err(|e| error(pc, op, e.0))? != 0;
        let Some(text) = self.windows[slot].as_ref().map(|window| window.text) else {
            return Err(error(
                pc,
                op,
                "message window slot without a window sets an unmodeled flag",
            ));
        };
        if self.texts[text].is_none() {
            return Err(error(pc, op, "message window without its text object"));
        }
        vm.complete_system(SystemResult(0x4008))?;
        let window = self.windows[slot].as_mut().expect("checked above");
        if window.visible {
            window.hidden = hidden;
            window.object.shown = !hidden;
            self.texts[text].as_mut().expect("checked above").visible = !hidden;
        }
        Ok(())
    }
    fn cursor(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let slot = window_slot(vm, pc, 0x2151)?;
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, 0x2151, e.0))
        };
        let (mode, x, y) = (
            word(8)? as u16,
            word(12)? as u16 as i16,
            word(16)? as u16 as i16,
        );
        if self.windows[slot].is_none() {
            return Err(error(
                pc,
                0x2151,
                "message window slot without a window sets an unmodeled flag",
            ));
        }
        vm.complete_system(SystemResult(0x4010))?;
        let window = self.windows[slot].as_mut().expect("checked above");
        window.cursor_mode = mode;
        match mode {
            1 | 3 => window.cursor_offset = [x, y],
            0 | 2 => window.cursor_offset = [0, 0],
            _ => {}
        }
        Ok(())
    }
    fn move_window(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let slot = window_slot(vm, pc, 0x2150)?;
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, 0x2150, e.0))
        };
        let (mode, x, y) = (word(8)?, word(12)? as i32, word(16)? as i32);
        let window = self.windows[slot].as_ref().ok_or_else(|| {
            error(
                pc,
                0x2150,
                "message window slot without a window sets an unmodeled flag",
            )
        })?;
        let point = match mode {
            0 => [x, y],
            1 => [
                window.position[0].wrapping_add(x),
                window.position[1].wrapping_add(y),
            ],
            _ => {
                return Err(error(
                    pc,
                    0x2150,
                    "window move mode other than 0/1 reads an uninitialized stack slot",
                ));
            }
        };
        if window.object.draw.is_none() {
            return Err(error(pc, 0x2150, "window object has no draw state"));
        }
        if self.texts[window.text].is_none() {
            return Err(error(pc, 0x2150, "message window without its text object"));
        }
        vm.complete_system(SystemResult(0x4010))?;
        let window = self.windows[slot].as_mut().expect("checked above");
        window.position = point;
        window.switch_positions[0] = point;
        let [x, y] = point.map(|v| v as u32);
        window
            .object
            .draw
            .as_mut()
            .expect("checked above")
            .set(DrawSetter::Position([x, y]));
        self.texts[window.text]
            .as_mut()
            .expect("checked above")
            .position = point;
        Ok(())
    }
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
    /// 0x2179 / 0x1400771e0 (0x14005d350): SP-4 window slot, SP-8 child (signed); pops 8
    /// bytes. A child below 768 is destroyed by 0x14003a7d0 (0x14003c590 also releases its own
    /// children, images and draw state); a missing or larger child is a no-op without a flag.
    /// A negative child frees the +2520 object and leaves its pointer null, which
    /// `MessageWindow` cannot represent, so it stays an error. Missing windows only set
    /// +12056 bit 0x10000.
    fn destroy_child(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let slot = window_slot(vm, pc, 0x2179)?;
        let child = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x2179, e.0))? as i32;
        if self.windows[slot].is_none() {
            return Err(error(
                pc,
                0x2179,
                "message window slot without a window sets an unmodeled flag",
            ));
        }
        if child < 0 {
            return Err(error(
                pc,
                0x2179,
                "freeing the window's own object leaves an unmodeled null object",
            ));
        }
        vm.complete_system(SystemResult(0x4008))?;
        let object = &mut self.windows[slot].as_mut().expect("checked above").object;
        if let Ok(id) = u16::try_from(child)
            && id < 768
        {
            object.children.remove(&id);
        }
        Ok(())
    }
    /// 0x217b / 0x1400772d0 -> 0x14005d280: tests the window's own object
    /// (negative child) or a child pointer, regardless of image, visibility or draw state.
    /// Existing window with missing/out-of-range child returns 0, not an error.
    fn object_presence(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let slot = window_slot(vm, pc, 0x217b)?;
        let child = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x217b, e.0))? as i32;
        let window = self.windows[slot].as_ref().ok_or_else(|| {
            error(
                pc,
                0x217b,
                "message window slot without a window sets an unmodeled flag",
            )
        })?;
        let exists = window_target(&window.object, child).is_some();
        vm.complete_system(SystemResult(0x4008))?;
        vm.state_mut()
            .set_system_integer(0, u32::from(exists))
            .expect("slot 0 exists");
        Ok(())
    }

    /// 0x2171 / 0x140076fd0 -> 0x14003aa30: releases only +6176 on the
    /// window's +2520 object. Children, draw state, text and visibility are retained.
    fn release_image(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<(), DecodeError> {
        let slot = window_slot(vm, pc, 0x2171)?;
        if self.windows[slot].is_none() {
            return Err(error(
                pc,
                0x2171,
                "message window slot without a window sets an unmodeled flag",
            ));
        }
        vm.complete_system(SystemResult(0x4004))?;
        self.windows[slot].as_mut().expect("validated").object.image = None;
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
    /// - 0x214f / 0x1400765e0 -> 0x14005db80 -> 0x14005ac20: cursor +156/+160 =
    ///   SP-8/SP-12, unconditionally; area and window position stay unchanged. Pops 12.
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
            0x144 | 0x146 | 0x148 | 0x14f => 2,
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
            0x14f => text.cursor = [a, b],
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
            scale_bits: 1.0f32.to_bits(),
            logical_size,
            position,
            area: [0, 0, size[0], size[1]],
            cursor: [0, 0],
            ruby_lines: [0; 32],
            layout_scratch: [0; 2],
            glyphs: Vec::new(),
            glyph_size: 27,
            spacing: [0, 0],
            color: 0xff_ffff,
            decoration_color: 0xff_ffff,
            outline_mode: 2,
            glyph_parameter: 0,
            decoration: 0,
            reveal: [0, 0],
            char_delay: 100,
            layer: 1,
            visible: true,
            ruby: false,
            ruby_every_line: false,
            ruby_size: 12,
            ruby_spacing: 0,
            line_break: None,
        });
        self.windows[slot] = Some(MessageWindow {
            text,
            visible: false,
            hidden: false,
            position,
            size,
            switch_positions: [position; 2],
            cursor_mode: 0,
            cursor_offset: [0, 0],
            position_switch: false,
            object: SceneObject::default(),
            buttons: [WindowButton::default(); WINDOW_BUTTONS],
            pressed_button: -1,
        });
        Ok(())
    }
}

/// 0x14005d280: a negative child selects the object itself, others an existing child.
fn window_target(object: &SceneObject, child: i32) -> Option<&SceneObject> {
    match u16::try_from(child) {
        Err(_) if child < 0 => Some(object),
        Ok(id) if id < 768 => object.children.get(&id),
        _ => None,
    }
}
fn window_target_mut(object: &mut SceneObject, child: i32) -> Option<&mut SceneObject> {
    match u16::try_from(child) {
        Err(_) if child < 0 => Some(object),
        Ok(id) if id < 768 => object.children.get_mut(&id),
        _ => None,
    }
}

/// SP-4 compared signed against 8, as all window handlers do.
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
