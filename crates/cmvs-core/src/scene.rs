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
    /// First 88 bytes of +6192, initialized by 0x14001ee40.
    pub draw: Option<DrawState>,
    /// +6248: inheritance flags; bit 31 records child creation.
    pub flags: u32,
    /// +6244 (constructor 1): 0x14003de80 skips the object when zero; 0x2215 sets it.
    pub shown: bool,
    /// +6256/+6260 (constructor 0): logical screen size; 0x14003b350 scales by the device
    /// width over +6256, or over the screen width while it is zero. Set by 0x217c.
    pub logical_size: [u32; 2],
    /// +14500 (constructor 0): set by 0x14003d0c0 when an effect ends with -2; parents then
    /// destroy such children.
    pub dispose: bool,
}
impl SceneObject {
    /// 0x14003d0c0 limited to the frame animation channel of 0x14001f5c0: with a draw state,
    /// advance its animation, freeing it when it ends and marking the object for disposal
    /// when it ended with -2; then, once children were created (+6248 bit 31), update each
    /// child and destroy the marked ones. Other effect channels are not modeled.
    pub(crate) fn advance_effects(&mut self, delta: u32) {
        if let Some(draw) = self.draw.as_mut()
            && let Some(animation) = draw.animation.as_deref_mut()
            && let Some(end) = animation.advance(delta, &mut draw.words)
        {
            draw.animation = None;
            if end == -2 {
                self.dispose = true;
            }
        }
        if (self.flags as i32) < 0 {
            for child in self.children.values_mut() {
                child.advance_effects(delta);
            }
            self.children.retain(|_, child| !child.dispose);
        }
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

/// Draw fields retained as original words (offset / 4). Unimplemented setters stay pending.
/// 0=coordinate mode, 1=blend, 2..6=source x/y/width/height, 6..8=origin,
/// 8..10=position, 10=order, 17=opacity, 18/19=scale bits, 20=rotation, 21=fade factor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawState {
    pub words: [u32; 22],
    /// +136: frame animation; rebuilding the draw state drops it.
    pub animation: Option<Box<Animation>>,
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
            animation: None,
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
}
impl SceneHost {
    pub fn fade(&self) -> &crate::fade::FadeHost {
        &self.fade
    }
    pub fn fade_mut(&mut self) -> &mut crate::fade::FadeHost {
        &mut self.fade
    }
    pub fn advance_frame(&mut self, delta: u32) {
        self.fade.advance(delta);
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
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        if self.fade.try_handle(vm)? {
            return Ok(true);
        }
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        if matches!(command, 0x22 | 0x40 | 0x50 | 0x44..=0x48 | 0x4b) {
            return self.handle_object(vm, pc, command);
        }
        if matches!(command, 0x27 | 0x28) {
            return self.query_or_move(vm, pc, command);
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

    fn handle_object(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, command + 0x2000, why);
        let count = match command {
            0x40 => 1,
            0x22 | 0x50 => 2,
            0x44 => 6,
            0x45 | 0x46 => 4,
            0x47 | 0x48 | 0x4b => 3,
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
        let child =
            if command == 0x40 || (matches!(command, 0x44..=0x48 | 0x4b) && (args[1] as i32) < 0) {
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
            if matches!(command, 0x44..=0x48 | 0x4b) && object.draw.is_none() {
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
    let bytes = images.read(&name)?.ok_or_else(|| {
        error(
            pc,
            op,
            "image not found in any 0x2011 entry; the original shows a modal MessageBox",
        )
    })?;
    let decoded =
        image::decode(&bytes).map_err(|e| format!("0x{op:04x} {}: {e}", name.escape_ascii()))?;
    Ok(decoded.map(|decoded| ObjectImage {
        name,
        pixels: decoded.pixels,
        alpha: decoded.alpha,
    }))
}
