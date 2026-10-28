//! Platform-independent display state set by verified scene commands.
//! Renderers read it each frame; this module draws nothing and owns no device.
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    image::{self, ImageSource},
    machine::Machine,
    raster::RgbaImage,
    vm::SystemResult,
};
use std::{collections::BTreeMap, error::Error};
mod change;
mod mask;
mod quake;
mod strip;
mod world_motion;
pub use change::ImageChange;
pub use mask::MaskTransition;
pub use quake::Quake;
pub use strip::{Strip, StripWipe};
pub use world_motion::{MotionKey, MotionPlayback, WORLD_MOTION_KEYS, WorldMotion};

/// Engine +1976 holds 256 object pointers (snapshot records 0x700).
pub const OBJECT_SLOTS: usize = 256;

/// CD3DDevice +84/+80; constructor 0x14001d5d0 and reset 0x140080700 both zero them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClearState {
    /// 0x22cb keeps the raw u32; frames clear to opaque 0x00RRGGBB from its low 24 bits.
    pub color: u32,
    /// 0x2250: also clear depth to 1.0, not only the target.
    pub depth: bool,
}
impl ClearState {
    pub fn rgba(&self) -> [u8; 4] {
        let [b, g, r, _] = self.color.to_le_bytes();
        [r, g, b, 255]
    }
}

/// Object +6176 after a successful 0x2030 (image object built by 0x14002fd00).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectImage {
    /// File name as given to 0x2030 (image +1044).
    pub name: Vec<u8>,
    pub pixels: RgbaImage,
    /// Image +1152: the source has 32 bits per pixel.
    pub alpha: bool,
}

/// A 0x38c8-byte object freshly built by 0x14003a3c0, including 768 child pointers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneObject {
    /// None until 0x2030 loads an image; a quiet decode failure also leaves it empty.
    pub image: Option<ObjectImage>,
    /// 768 child pointers at +8, addressed by 0x2022/0x2038/0x2050.
    pub children: BTreeMap<u16, SceneObject>,
    /// +6192, initialized by 0x14001ee40: the first 88 bytes and the modeled later fields.
    pub draw: Option<DrawState>,
    /// +6248: inheritance flags; bit 31 records child creation.
    pub flags: u32,
    /// +6244 (constructor 1): 0x14003de80 skips the object when zero; 0x2215 sets it.
    pub shown: bool,
    /// +6256/+6260 (constructor 0): logical screen size; 0x14003b350 scales by the device
    /// width over +6256, or over the screen width while it is zero. Set by 0x217c (windows)
    /// and 0x202a (slot objects).
    pub logical_size: [u32; 2],
    /// +14500 (constructor 0): set by 0x14003d0c0 when an effect ends with -2; parents then
    /// destroy such children.
    pub dispose: bool,
    /// +14504/+14512/+14520: software image replacement and retained objects.
    pub change: Option<Box<ImageChange>>,
}
impl SceneObject {
    /// 0x14003d0c0 limited to opacity (+128), frame animation (+136), and XYZ (+152) of
    /// 0x14001f5c0, in that order: with a draw state, advance each, freeing a channel when it
    /// ends and marking the object for disposal when it ended with -2; then, once children
    /// were created (+6248 bit 31), update each child and destroy the marked ones. The other
    /// effect channels are not modeled.
    pub(crate) fn advance_effects(&mut self, delta: u32) {
        if let Some(draw) = self.draw.as_mut() {
            if let Some(track) = draw.opacity_track.as_deref_mut()
                && let Some(end) = track.advance(delta, &mut draw.words)
            {
                draw.opacity_track = None;
                self.dispose |= end == -2;
            }
            if let Some(animation) = draw.animation.as_deref_mut()
                && let Some(end) = animation.advance(delta, &mut draw.words)
            {
                draw.animation = None;
                self.dispose |= end == -2;
            }
            if let Some(motion) = draw.world_motion.as_deref_mut()
                && let Some(end) = motion.advance(delta, &mut draw.words)
            {
                draw.world_motion = None;
                self.dispose |= end == -2;
            }
        }
        if (self.flags as i32) < 0 {
            for child in self.children.values_mut() {
                child.advance_effects(delta);
            }
            self.children.retain(|_, child| !child.dispose);
        }
        self.advance_change(delta);
    }
}
impl Default for SceneObject {
    fn default() -> Self {
        Self {
            image: None,
            children: BTreeMap::new(),
            draw: None,
            flags: 0x7fff_ffff,
            shown: true,
            logical_size: [0, 0],
            dispose: false,
            change: None,
        }
    }
}

/// Frame entries in the 0x510-byte animation object (0x140026060 zeroes them all).
pub const ANIMATION_FRAMES: usize = 64;

/// One 20-byte entry at animation +16 + 20 * index, written by 0x1400261b0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnimationFrame {
    /// Entry +0: the frame 0x1400262b0 moves to next; negative ends the animation and -2
    /// also marks the object for disposal (0x14003d0c0 sets +14500).
    pub next: i32,
    /// Entry +4..+14: source x, y, width, height and origin x, y copied into draw words 2..8
    /// (origin only when non-negative).
    pub words: [i16; 6],
    /// Entry +16: milliseconds before moving on, compared unsigned with the elapsed time.
    pub duration: u32,
}

/// The draw state +136 frame animation (0x21cf). Header words: +0 playing, +4 start frame,
/// +8 current frame, +12 elapsed milliseconds; all zero after construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Animation {
    pub playing: bool,
    pub start: u32,
    pub current: u32,
    pub elapsed: u32,
    pub frames: [AnimationFrame; ANIMATION_FRAMES],
}
impl Animation {
    /// 0x140026200 (0x21d1): play from `frame`, copying it into the draw words.
    pub(crate) fn start(&mut self, frame: usize, words: &mut [u32; 22]) {
        self.playing = true;
        self.start = frame as u32;
        self.current = frame as u32;
        self.elapsed = 0;
        self.frames[frame].apply(words);
    }
    /// 0x1400262b0 for a playing animation: add the frame delta (at most 200 ms), then
    /// while the current frame's duration has elapsed either end with its negative `next`
    /// or move to `next`, subtracting the duration and copying the frame into the draw
    /// words. A cycle of zero-length frames never exits in the original; Rust moves at most
    /// ANIMATION_FRAMES times per update.
    fn advance(&mut self, delta: u32, words: &mut [u32; 22]) -> Option<i32> {
        if !self.playing {
            return None;
        }
        self.elapsed = self.elapsed.wrapping_add(delta.min(200));
        for _ in 0..ANIMATION_FRAMES {
            let frame = self.frames[self.current as usize];
            if frame.duration > self.elapsed {
                return None;
            }
            if frame.next < 0 {
                return Some(frame.next);
            }
            self.elapsed -= frame.duration;
            self.current = frame.next as u32;
            self.frames[self.current as usize].apply(words);
        }
        None
    }
}
impl AnimationFrame {
    /// Sign-extended source rectangle into draw words 2..6; origin words 6/7 only when the
    /// stored value is non-negative.
    fn apply(&self, words: &mut [u32; 22]) {
        for (word, value) in words[2..6].iter_mut().zip(&self.words[..4]) {
            *word = i32::from(*value) as u32;
        }
        for (word, value) in words[6..8].iter_mut().zip(&self.words[4..]) {
            if *value >= 0 {
                *word = i32::from(*value) as u32;
            }
        }
    }
}
impl Default for Animation {
    fn default() -> Self {
        Self {
            playing: false,
            start: 0,
            current: 0,
            elapsed: 0,
            frames: [AnimationFrame::default(); ANIMATION_FRAMES],
        }
    }
}

/// Keys in the 0x94-byte opacity track (0x1400267a0 zeroes them all).
pub const OPACITY_KEYS: usize = 8;

/// One 16-byte key at track +20 + 16 * index, written by 0x140026870.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpacityKey {
    /// +0: the key 0x1400268d0 moves to next; negative ends the track (-2 also marks the
    /// object for disposal).
    pub next: i32,
    /// +4: target opacity, |value| % 256 as C computes it.
    pub value: i32,
    /// +8: while its low byte is set the opacity holds instead of interpolating.
    pub hold: u32,
    /// +12: milliseconds to reach `value`, compared unsigned with the elapsed time.
    pub duration: u32,
}

/// The draw state +128 opacity track (0x21ea). Header words: +0 playing, +4 base value,
/// +8 start key, +12 current key, +16 elapsed milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpacityTrack {
    pub playing: bool,
    pub base: i32,
    pub start: u32,
    pub current: u32,
    pub elapsed: u32,
    pub keys: [OpacityKey; OPACITY_KEYS],
}
impl OpacityTrack {
    /// 0x1400268d0 for a playing track, writing draw word 17: add the frame delta (at most
    /// 200 ms); while the current key's duration has elapsed, take its value as both base
    /// and opacity, end with its negative `next`, or move on subtracting the duration.
    /// Otherwise interpolate `base + elapsed * (value - base) / duration` (unsigned 32-bit
    /// product, signed division) unless the key holds. A cycle of zero-length keys never
    /// exits in the original; Rust moves at most ANIMATION_FRAMES times per update.
    fn advance(&mut self, delta: u32, words: &mut [u32; 22]) -> Option<i32> {
        if !self.playing {
            return None;
        }
        self.elapsed = self.elapsed.wrapping_add(delta.min(200));
        let key = self.keys[self.current as usize];
        if key.duration > self.elapsed {
            if key.hold & 0xff == 0 {
                let span = (key.value.wrapping_sub(self.base) as u32).wrapping_mul(self.elapsed);
                let step = (span as i32).wrapping_div(key.duration as i32);
                words[17] = self.base.wrapping_add(step) as u32;
            }
            return None;
        }
        for _ in 0..ANIMATION_FRAMES {
            let key = self.keys[self.current as usize];
            self.base = key.value;
            words[17] = key.value as u32;
            if key.next < 0 {
                return Some(key.next);
            }
            self.elapsed -= key.duration;
            self.current = key.next as u32;
            if self.keys[self.current as usize].duration > self.elapsed {
                return None;
            }
        }
        None
    }
}

/// Draw fields retained as original words (offset / 4). Unimplemented setters stay pending.
/// 0=coordinate mode, 1=blend, 2..6=source x/y/width/height, 6..8=origin,
/// 8..10=position, 10=order, 11..14=world x/y/z float bits (0x2072..0x2074),
/// 15/16=camera projection parameter bits (0x2070/0x2071),
/// 17=opacity, 18/19=scale bits, 20=rotation, 21=fade factor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawState {
    pub words: [u32; 22],
    /// +108 (0x2075; constructor 0): raw float bits added to world Z for the projected
    /// queue key only (0x14001f360). 0x22c4 copies it with +88..+104 (0x14001f520).
    pub depth_bias: u32,
    /// +128: opacity track; rebuilding the draw state drops it.
    pub opacity_track: Option<Box<OpacityTrack>>,
    /// +136: frame animation; rebuilding the draw state drops it.
    pub animation: Option<Box<Animation>>,
    /// +152: floating-point world XYZ motion, managed by 0x21f9..0x21fd.
    pub world_motion: Option<Box<WorldMotion>>,
}
impl Default for DrawState {
    fn default() -> Self {
        let mut words = [0; 22];
        words[1] = 1;
        words[17] = 255;
        words[18] = 1.0f32.to_bits();
        words[19] = 1.0f32.to_bits();
        words[21] = 256;
        Self {
            words,
            depth_bias: 0,
            opacity_track: None,
            animation: None,
            world_motion: None,
        }
    }
}

/// Draw-state setters shared by 0x2044..0x2048 and the message window object commands.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DrawSetter {
    /// 0x14003b070: source x/y/width/height.
    Source([u32; 4]),
    /// 0x14003b0a0: origin x/y.
    Origin([u32; 2]),
    /// 0x14003b0b0: position x/y.
    Position([u32; 2]),
    /// 0x14003b0d0: layer.
    Layer(u32),
    /// 0x14003b0e0: opacity, capped at 255 as a signed value (negative values are kept).
    Opacity(u32),
}
impl DrawState {
    pub(crate) fn set(&mut self, setter: DrawSetter) {
        match setter {
            DrawSetter::Source(rect) => self.words[2..6].copy_from_slice(&rect),
            DrawSetter::Origin(point) => self.words[6..8].copy_from_slice(&point),
            DrawSetter::Position(point) => self.words[8..10].copy_from_slice(&point),
            DrawSetter::Layer(layer) => self.words[10] = layer,
            DrawSetter::Opacity(opacity) => self.words[17] = (opacity as i32).min(255) as u32,
        }
    }
}

#[derive(Debug, Default)]
pub struct SceneHost {
    clear: ClearState,
    objects: BTreeMap<u8, SceneObject>,
    fade: crate::fade::FadeHost,
    /// Engine +4632 and +4648; see `mask`.
    masks: [Option<MaskTransition>; 2],
    /// Engine +4624; see `quake`.
    quake: Option<Quake>,
    /// Engine +4640; see `strip`.
    strip: Option<StripWipe>,
}
impl SceneHost {
    pub fn fade(&self) -> &crate::fade::FadeHost {
        &self.fade
    }
    pub fn fade_mut(&mut self) -> &mut crate::fade::FadeHost {
        &mut self.fade
    }
    /// The slot part of the frame update 0x140015900: each object runs 0x14003d0c0 and is
    /// destroyed when it is marked for disposal (0x14003a780 reads +14500); then the fade,
    /// the strip wipe and both mask transitions. The quake follows in the render pass
    /// 0x140015da0.
    pub fn advance_frame(&mut self, delta: u32) {
        for object in self.objects.values_mut() {
            object.advance_effects(delta);
        }
        self.objects.retain(|_, object| !object.dispose);
        self.fade.advance(delta);
        self.advance_strip(delta);
        self.advance_mask(delta);
        self.advance_quake(delta);
    }
    pub fn clear(&self) -> ClearState {
        self.clear
    }
    pub fn object(&self, slot: u8) -> Option<&SceneObject> {
        self.objects.get(&slot)
    }
    pub fn objects(&self) -> &BTreeMap<u8, SceneObject> {
        &self.objects
    }
    /// For other verified command families that drive scene objects (0x2215).
    pub(crate) fn object_mut(&mut self, slot: u8) -> Option<&mut SceneObject> {
        self.objects.get_mut(&slot)
    }
    /// 0x2308 creates a fresh object bound to the scene movie's first decoded frame.
    pub(crate) fn replace_movie_object(&mut self, slot: u8, object: SceneObject) {
        self.objects.insert(slot, object);
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        if matches!(vm.pending_system(), Some((_, 0x1f9..=0x1fd))) {
            return self.try_world_motion(vm);
        }
        if self.fade.try_handle(vm)? {
            return Ok(true);
        }
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        if matches!(command, 0x2c5 | 0x2c6) {
            return self.control_change(vm, pc, command);
        }
        if matches!(
            command,
            0x22 | 0x40 | 0x42 | 0x43 | 0x50 | 0x44..=0x49 | 0x4b | 0x4c | 0x70..=0x75
        ) {
            return self.handle_object(vm, pc, command);
        }
        if matches!(command, 0x27 | 0x28) {
            return self.query_or_move(vm, pc, command);
        }
        if command == 0x33 {
            return self.image_info(vm, pc);
        }
        if command == 0x2a {
            return self.set_logical_size(vm, pc);
        }
        if matches!(command, 0x1ea..=0x1ee) {
            return self.opacity(vm, pc, command);
        }
        if command == 0x2f {
            // 0x14006bc50 runs the 0x2021 destructor on all 256 slots and pops nothing.
            // The +4616 fade overlay owns its object outside this table, so it survives.
            vm.complete_system(SystemResult(0x4000))?;
            self.objects.clear();
            return Ok(true);
        }
        if !matches!(command, 0x2cb | 0x250 | 0x20 | 0x21) {
            return Ok(false);
        }
        let op = command + 0x2000;
        let value = vm.stack().peek_u32(4).map_err(|e| error(pc, op, e.0))?;
        let slot = if matches!(command, 0x20 | 0x21) {
            // 0x14006bb00/0x14006bbc0 compare signed against 256: larger values only set
            // +12056 bit 0x10 (unmodeled) and negative values would index before the table.
            match u8::try_from(value as i32) {
                Ok(slot) => Some(slot),
                Err(_) => return Err(error(pc, op, "object slot outside 0..255")),
            }
        } else {
            None
        };
        vm.complete_system(SystemResult(0x4004))?;
        match (command, slot) {
            (0x2cb, _) => self.clear.color = value,
            (0x250, _) => self.clear.depth = value != 0,
            // 0x2020 destroys any existing object in the slot before building a new one.
            (0x20, Some(slot)) => {
                self.objects.insert(slot, SceneObject::default());
            }
            (_, Some(slot)) => {
                self.objects.remove(&slot);
            }
            _ => unreachable!("slot is parsed for object commands"),
        }
        Ok(true)
    }

    /// SP-4 slot (signed < 256; larger values only set +12056 bit 0x10, negative ones would
    /// index before the table); both pop 8 bytes.
    /// - 0x2027 / 0x14006be00: system result 0 = whether the slot holds an object (negative
    ///   SP-8) or whether that object has child SP-8 (0x14003a760: none at 768 and above).
    /// - 0x2028 / 0x14006bea0: SP-8 is the source slot (same bounds). Destroys the target
    ///   object, moves the source pointer into the target and clears the source. With the
    ///   same occupied slot the original stores the pointer it just freed, which Rust rejects.
    fn query_or_move(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<bool, DecodeError> {
        let op = command + 0x2000;
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map(|value| value as i32)
                .map_err(|e| error(pc, op, e.0))
        };
        let slot = |value: i32| {
            u8::try_from(value).map_err(|_| error(pc, op, "object slot outside 0..255"))
        };
        let target = slot(word(4)?)?;
        let second = word(8)?;
        if command == 0x27 {
            let exists = match self.objects.get(&target) {
                None => false,
                Some(_) if second < 0 => true,
                Some(object) => u16::try_from(second)
                    .is_ok_and(|id| id < 768 && object.children.contains_key(&id)),
            };
            vm.complete_system(SystemResult(0x4008))?;
            vm.state_mut()
                .set_system_integer(0, u32::from(exists))
                .expect("slot 0 exists");
            return Ok(true);
        }
        let source = slot(second)?;
        if source == target && self.objects.contains_key(&target) {
            return Err(error(
                pc,
                op,
                "moving an object onto its own slot leaves a freed pointer",
            ));
        }
        vm.complete_system(SystemResult(0x4008))?;
        self.objects.remove(&target);
        if let Some(object) = self.objects.remove(&source) {
            self.objects.insert(target, object);
        }
        Ok(true)
    }

    /// 0x2033 / 0x14006c7c0: SP-4 slot (signed < 256 with an object; otherwise only +12056
    /// bit 0x10 is set). With an object image (+6176), its info block (vtable +0x18 returns
    /// image +1040) gives system results 1/2 = width/height (+1144/+1148) and 3 = the
    /// 32-bit flag (+1152), and result 0 = 1; without one only result 0 = 0 is written.
    /// Pops 4 bytes.
    fn image_info(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<bool, DecodeError> {
        let raw = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2033, e.0))? as i32;
        let object = u8::try_from(raw)
            .ok()
            .and_then(|slot| self.objects.get(&slot))
            .ok_or_else(|| {
                error(
                    pc,
                    0x2033,
                    "image query needs an existing object in slots 0..255",
                )
            })?;
        let info = object.image.as_ref().map(|image| {
            [
                image.pixels.width(),
                image.pixels.height(),
                u32::from(image.alpha),
            ]
        });
        vm.complete_system(SystemResult(0x4004))?;
        let state = vm.state_mut();
        state
            .set_system_integer(0, u32::from(info.is_some()))
            .expect("slot 0 exists");
        if let Some(info) = info {
            for (slot, value) in (1..).zip(info) {
                state
                    .set_system_integer(slot, value)
                    .expect("slots 1..3 exist");
            }
        }
        Ok(true)
    }

    /// 0x202a / 0x14006c160 -> 0x14003b410: SP-4 slot (signed; 256 and above only set +12056
    /// bit 0x10), SP-8 child (negative selects the object, 0x14003a760 otherwise), SP-12/SP-16
    /// logical width/height written to +6256/+6260. A missing object or child (none at 768
    /// and above) would be dereferenced. Pops 16.
    fn set_logical_size(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, 0x202a, why);
        let mut args = [0; 4];
        for (i, arg) in args.iter_mut().enumerate() {
            *arg = vm.stack().peek_u32(4 * (i + 1)).map_err(|e| fail(e.0))?;
        }
        let [slot, child, width, height] = args;
        let object = u8::try_from(slot as i32)
            .ok()
            .and_then(|slot| self.objects.get_mut(&slot))
            .ok_or_else(|| fail("logical size needs an existing object in slots 0..255"))?;
        let target = if (child as i32) < 0 {
            object
        } else {
            u16::try_from(child)
                .ok()
                .filter(|id| *id < 768)
                .and_then(|id| object.children.get_mut(&id))
                .ok_or_else(|| fail("logical size needs an existing child in slots 0..767"))?
        };
        vm.complete_system(SystemResult(0x4010))?;
        target.logical_size = [width, height];
        Ok(true)
    }

    /// The opacity track family 0x21ea..0x21ee (0x14006e260..0x14006e500): SP-4 slot (signed
    /// < 256 with an object, otherwise +12056 bit 0x10), SP-8 child (>= 768 sets bit 0x20,
    /// negative selects the object; a missing child or draw state would be dereferenced).
    /// - 0x21ea (0x14001fab0): replace the track with an empty one whose base is |SP-12| % 256.
    /// - 0x21eb (0x14001fbc0 -> 0x140026870): key SP-12 gets next SP-16, hold SP-20, duration
    ///   SP-24 and value |SP-28| % 256; no track is a no-op. Pops 28.
    /// - 0x21ec (0x14001fba0 -> 0x1400268b0): play from key SP-12 (no opacity change yet).
    /// - 0x21ed (0x14001fb40 -> 0x140026430): a playing track writes its current key's value
    ///   to opacity; the track is freed. Pops 8.
    /// - 0x21ee (0x14001fbf0): system result 0 = current key while playing, -2 when stopped,
    ///   -1 without a track. Pops 8.
    ///
    /// The original bounds neither key indices nor `next`; Rust accepts keys 0..7.
    fn opacity(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<bool, DecodeError> {
        let op = command + 0x2000;
        let count = match command {
            0x1eb => 5,
            0x1ea | 0x1ec => 1,
            _ => 0,
        };
        let mut args = [0i32; 7];
        for (i, arg) in args.iter_mut().take(count + 2).enumerate() {
            *arg = vm
                .stack()
                .peek_u32(4 + 4 * i)
                .map_err(|e| error(pc, op, e.0))? as i32;
        }
        let [slot, child, a, b, c, d, e] = args;
        let slot = u8::try_from(slot)
            .ok()
            .filter(|slot| self.objects.contains_key(slot))
            .ok_or_else(|| error(pc, op, "effect needs an existing object in slots 0..255"))?;
        let child = match u16::try_from(child) {
            Err(_) if child < 0 => None,
            Ok(id) if id < 768 && self.objects[&slot].children.contains_key(&id) => Some(id),
            _ => return Err(error(pc, op, "effect child outside 0..767 or missing")),
        };
        let key_ok = |index: i32| usize::try_from(index).is_ok_and(|i| i < OPACITY_KEYS);
        if (matches!(command, 0x1eb | 0x1ec) && !key_ok(a))
            || (command == 0x1eb && b >= 0 && !key_ok(b))
        {
            return Err(error(
                pc,
                op,
                "opacity key outside 0..7 leaves the original track",
            ));
        }
        let parent = self.objects.get_mut(&slot).expect("checked above");
        let object = match child {
            Some(id) => parent.children.get_mut(&id).expect("checked above"),
            None => parent,
        };
        let draw = object
            .draw
            .as_mut()
            .ok_or_else(|| error(pc, op, "effect target has no draw state"))?;
        vm.complete_system(SystemResult(0x4008 + 4 * count as u16))?;
        let magnitude = |value: i32| value.wrapping_abs() % 256;
        match command {
            0x1ea => {
                draw.opacity_track = Some(Box::new(OpacityTrack {
                    playing: false,
                    base: magnitude(a),
                    start: 0,
                    current: 0,
                    elapsed: 0,
                    keys: [OpacityKey::default(); OPACITY_KEYS],
                }));
            }
            0x1eb => {
                if let Some(track) = draw.opacity_track.as_deref_mut() {
                    track.keys[a as usize] = OpacityKey {
                        next: b,
                        value: magnitude(e),
                        hold: c as u32,
                        duration: d as u32,
                    };
                }
            }
            0x1ec => {
                if let Some(track) = draw.opacity_track.as_deref_mut() {
                    track.playing = true;
                    track.start = a as u32;
                    track.current = a as u32;
                    track.elapsed = 0;
                }
            }
            0x1ed => {
                if let Some(track) = draw.opacity_track.take()
                    && track.playing
                {
                    draw.words[17] = track.keys[track.current as usize].value as u32;
                }
            }
            _ => {
                let result = match draw.opacity_track.as_deref() {
                    None => u32::MAX,
                    Some(track) if !track.playing => u32::MAX - 1,
                    Some(track) => track.current,
                };
                vm.state_mut()
                    .set_system_integer(0, result)
                    .expect("slot 0 exists");
            }
        }
        Ok(true)
    }

    fn handle_object(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, command + 0x2000, why);
        let count = match command {
            0x40 => 1,
            0x22 | 0x42 | 0x50 => 2,
            0x43 => 5,
            0x44 => 6,
            0x45 | 0x46 | 0x49 => 4,
            0x47 | 0x48 | 0x4b | 0x4c | 0x70..=0x75 => 3,
            _ => unreachable!(),
        };
        let mut args = Vec::with_capacity(count);
        for i in 1..=count {
            args.push(vm.stack().peek_u32(i * 4).map_err(|e| fail(e.0))?);
        }
        let slot = u8::try_from(args[0]).map_err(|_| fail("object slot outside 0..255"))?;
        let parent = self
            .objects
            .get(&slot)
            .ok_or_else(|| fail("object slot does not exist"))?;
        let child = if command == 0x40
            || (matches!(command, 0x42..=0x49 | 0x4b | 0x4c | 0x70..=0x75) && (args[1] as i32) < 0)
        {
            None
        } else {
            Some(
                u16::try_from(args[1])
                    .ok()
                    .filter(|v| *v < 768)
                    .ok_or_else(|| fail("child slot outside 0..767"))?,
            )
        };
        if command != 0x22 {
            let object = match child {
                Some(id) => parent
                    .children
                    .get(&id)
                    .ok_or_else(|| fail("child object does not exist"))?,
                None => parent,
            };
            if matches!(command, 0x42..=0x49 | 0x4b | 0x4c | 0x70..=0x75) && object.draw.is_none() {
                return Err(fail("object has no draw state"));
            }
        }
        vm.complete_system(SystemResult(0x4000 | (count * 4) as u16))?;
        let parent = self.objects.get_mut(&slot).expect("validated");
        if command == 0x22 {
            let flags = if parent.draw.is_some() {
                0x7fff_ffff
            } else {
                0
            };
            parent.children.insert(
                child.expect("child"),
                SceneObject {
                    flags,
                    ..SceneObject::default()
                },
            );
            parent.flags |= 0x8000_0000;
        } else {
            let object = match child {
                Some(id) => parent.children.get_mut(&id).expect("validated"),
                None => parent,
            };
            let setter = match command {
                0x44 => Some(DrawSetter::Source([args[2], args[3], args[4], args[5]])),
                0x46 => Some(DrawSetter::Origin([args[2], args[3]])),
                0x45 => Some(DrawSetter::Position([args[2], args[3]])),
                0x47 => Some(DrawSetter::Layer(args[2])),
                0x48 => Some(DrawSetter::Opacity(args[2])),
                _ => None,
            };
            match command {
                0x40 | 0x50 => object.draw = Some(DrawState::default()),
                // 0x2042 / 0x14006d130 -> 0x14003b190 -> 0x14001f550:
                // set the existing draw state's coordinate mode without rebuilding it.
                0x42 => object.draw.as_mut().expect("validated").words[0] = 2,
                // 0x14006d220 -> 0x14003b150 -> 0x14001f3b0. Retain the image,
                // origin and animation tracks; only reset the source and fill fields.
                0x43 => {
                    let words = &mut object.draw.as_mut().expect("validated").words;
                    words[2..4].fill(0);
                    words[4..6].copy_from_slice(&args[2..4]);
                    words[14] = args[4] | 0xff00_0000;
                }
                // 0x204c / 0x14006d1a0 -> 0x14003b1a0 -> 0x14001f560: mode 3 for a
                // nonzero SP-12, else 2. 0x14003de80 projects mode 2 through camera 0
                // (0x140027070) and mode 3 through camera 1 (0x140027080).
                0x4c => {
                    object.draw.as_mut().expect("validated").words[0] =
                        if args[2] != 0 { 3 } else { 2 };
                }
                // 0x2070 / 0x14006f1a0 -> 0x14003b1b0 -> 0x14001f5a0:
                // 0x2071 / 0x14006f240 -> 0x14003b1c0 -> 0x14001f5b0:
                // movss preserves SP-12's bits in draw +60/+64, camera projection inputs.
                0x70..=0x71 => {
                    object.draw.as_mut().expect("validated").words
                        [15 + (command - 0x70) as usize] = args[2];
                }
                // 0x2072 / 0x14006f2e0 -> 0x14003b1d0 -> 0x14004ede0:
                // 0x2073 / 0x14006f380 -> 0x14003b1e0 -> 0x14001f570:
                // 0x2074 / 0x14006f420 -> 0x14003b1f0 -> 0x14001f580:
                // movss preserves SP-12's bits in draw +44/+48/+52 (world x/y/z).
                0x72..=0x74 => {
                    let index = 11 + usize::from(command - 0x72);
                    object.draw.as_mut().expect("validated").words[index] = args[2];
                }
                // 0x2075 / 0x14006f5b0 -> 0x14003b200 -> 0x14001f590:
                // movss preserves SP-12's bits in draw +108 (projected depth bias).
                0x75 => object.draw.as_mut().expect("validated").depth_bias = args[2],
                // 0x2049 / 0x14006d730 -> 0x14003b100 -> 0x14001f490:
                // SP-12/SP-16 pass through xmm1/xmm2 and movss to +72/+76.
                0x49 => object.draw.as_mut().expect("validated").words[18..20]
                    .copy_from_slice(&args[2..4]),
                0x44..=0x48 => object
                    .draw
                    .as_mut()
                    .expect("validated")
                    .set(setter.expect("setter command")),
                0x4b => {
                    if args[2] <= 3 {
                        object.draw.as_mut().expect("validated").words[1] = args[2];
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok(true)
    }

    /// 0x2030 / 0x14006c220: SP-4 object slot, SP-8 file name. Rebuilds the object image
    /// (0x14003bb10) and sets system result 0 to whether loading succeeded; returns 0x4008.
    /// 0x2038 uses SP-8 child slot and SP-12 name, and returns 0x400c.
    /// +12032 is set only by 0x212a, which stays unhandled, so its 0x140018bc0 call never runs.
    pub fn try_handle_image(
        &mut self,
        vm: &mut Machine<'_>,
        images: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn Error>> {
        if matches!(vm.pending_system(), Some((_, 0x3f))) {
            // Dispatcher 0x1400827a7 -> 0x140035a90: arm, do not flush or load.
            images.cache_next_read()?;
            vm.complete_system(SystemResult(0x4000))?;
            return Ok(true);
        }
        let Some((pc, command @ (0x30 | 0x38))) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, command + 0x2000, why);
        let raw_slot = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let raw_name = vm
            .stack()
            .peek_u32(if command == 0x38 { 12 } else { 8 })
            .map_err(|e| fail(e.0))?;
        // Slots >= 256 or without an object only set +12056 bit 0x10 (unmodeled);
        // negative slots would index before the table.
        let slot = u8::try_from(raw_slot as i32)
            .ok()
            .filter(|slot| self.objects.contains_key(slot))
            .ok_or_else(|| fail("image load needs an existing object in slots 0..255"))?;
        let child = if command == 0x38 {
            let id = vm.stack().peek_u32(8).map_err(|e| fail(e.0))?;
            Some(
                u16::try_from(id)
                    .ok()
                    .filter(|id| *id < 768 && self.objects[&slot].children.contains_key(id))
                    .ok_or_else(|| fail("image load needs an existing child in slots 0..767"))?,
            )
        } else {
            None
        };
        let name = vm.resolve_string(raw_name)?;
        let decoded = load_image(images, name, pc, command + 0x2000)?;
        vm.complete_system(SystemResult(if child.is_some() { 0x400c } else { 0x4008 }))?;
        vm.state_mut()
            .set_system_integer(0, u32::from(decoded.is_some()))
            .expect("slot 0 exists");
        let parent = self.objects.get_mut(&slot).expect("checked above");
        let object = match child {
            Some(id) => parent.children.get_mut(&id).expect("checked above"),
            None => parent,
        };
        object.image = decoded;
        Ok(true)
    }
}

/// The 0x14003bb10 lookup shared by object image commands: the first 0x2011 entry holding
/// `name`, decoded by magic. Ok(None) is the quiet decode failure.
pub(crate) fn load_image(
    images: &mut dyn ImageSource,
    name: Vec<u8>,
    pc: usize,
    op: u16,
) -> Result<Option<ObjectImage>, Box<dyn Error>> {
    if name.len() >= 64 {
        return Err(error(pc, op, "image name exceeds the original 64-byte buffers").into());
    }
    let decoded = image::load(images, &name).map_err(|e| format!("PC=0x{pc:x} 0x{op:04x}: {e}"))?;
    Ok(decoded.map(|decoded| ObjectImage {
        name,
        pixels: decoded.pixels,
        alpha: decoded.alpha,
    }))
}
