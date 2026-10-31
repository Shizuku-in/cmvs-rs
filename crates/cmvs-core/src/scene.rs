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
mod animation;
mod change;
mod mask;
mod opacity;
mod position_motion;
mod quake;
mod rotation;
mod scale_motion;
mod strip;
mod world_motion;
pub use animation::{ANIMATION_FRAMES, Animation, AnimationFrame};
pub use change::ImageChange;
pub use mask::MaskTransition;
pub use opacity::{OPACITY_KEYS, OpacityKey, OpacityTrack};
pub use position_motion::{POSITION_KEYS, PositionKey, PositionMotion, PositionPlayback};
pub use quake::Quake;
pub use rotation::{ROTATION_KEYS, RotationKey, RotationTrack};
pub use scale_motion::{SCALE_KEYS, ScaleKey, ScaleMotion, ScalePlayback};
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
    /// Image +1200 enables the mode/color/extra words at +1204..+1212.
    /// Replacement loads reuse these parameters (0x1400156ef..0x140015768).
    pub filter: Option<[u32; 3]>,
    /// File name as given to 0x2030 (image +1044).
    pub name: Vec<u8>,
    pub pixels: RgbaImage,
    /// Image +1152: the source has 32 bits per pixel.
    pub alpha: bool,
    /// Kind 3 (+1224, 0x140034f60): a blank surface the engine draws into, such as a scene
    /// movie frame, rather than a loaded file. Saves record only its size and alpha flag;
    /// Rust keeps the movie name in `name` to check the player's binding.
    pub surface: bool,
}

impl ObjectImage {
    pub(crate) fn apply_filter(&mut self, parameters: [u32; 3]) -> Result<(), crate::FormatError> {
        let [mode, color, _extra] = parameters;
        if mode != 2 {
            return Err(crate::FormatError(
                "image filter mode not implemented; only mode 2 is verified",
            ));
        }
        // 0x14003364c..0x1400336d9: original BGRA, ours RGBA. Alpha is untouched.
        let [b, g, r, _] = color.to_le_bytes();
        let mut pixels = self.pixels.pixels().to_vec();
        for pixel in pixels.as_chunks_mut::<4>().0 {
            for (channel, factor) in pixel[..3].iter_mut().zip([r, g, b]) {
                *channel = ((u32::from(*channel) * u32::from(factor)) >> 7).min(255) as u8;
            }
        }
        self.pixels = RgbaImage::new(self.pixels.width(), self.pixels.height(), pixels)?;
        self.filter = Some(parameters);
        Ok(())
    }
}

/// A 0x38c8-byte object freshly built by 0x14003a3c0, including 768 child pointers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneObject {
    /// None until 0x2030 loads an image; a quiet decode failure also leaves it empty.
    pub image: Option<ObjectImage>,
    /// +6184: second texture loaded by 0x2035, separate from the main image.
    pub secondary_image: Option<ObjectImage>,
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
    /// 0x14003d0c0 limited to scale (+112), rotation (+120), opacity (+128), frame animation (+136), XY (+144), and XYZ (+152) of
    /// 0x14001f5c0, in that order: with a draw state, advance each, freeing a channel when it
    /// ends and marking the object for disposal when it ended with -2; then, once children
    /// were created (+6248 bit 31), update each child and destroy the marked ones. The other
    /// effect types outside these channels are not modeled.
    pub(crate) fn advance_effects(&mut self, delta: u32) {
        if let Some(draw) = self.draw.as_mut() {
            if let Some(track) = draw.scale_motion.as_deref_mut()
                && let Some(end) = track.advance(delta, &mut draw.words)
            {
                draw.scale_motion = None;
                self.dispose |= end == -2;
            }
            if let Some(track) = draw.rotation_track.as_deref_mut()
                && let Some(end) = track.advance(delta, &mut draw.words)
            {
                draw.rotation_track = None;
                self.dispose |= end == -2;
            }
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
            if let Some(motion) = draw.position_motion.as_deref_mut()
                && let Some(end) = motion.advance(delta, &mut draw.words)
            {
                draw.position_motion = None;
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
            secondary_image: None,
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

/// Draw fields retained as original words (offset / 4). Unimplemented setters stay pending.
/// 0=coordinate mode, 1=blend, 2..6=source x/y/width/height, 6..8=origin,
/// 8..10=position, 10=order, 11..14=world x/y/z float bits (0x2072..0x2074),
/// 15/16=camera projection parameter bits (0x2070/0x2071),
/// 17=opacity, 18/19=scale bits, 20=rotation, 21=fade factor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawState {
    pub words: [u32; 22],
    /// +88: second-texture mapping mode, set by 0x2321.
    pub texture_mode: u32,
    /// +104: 0x2322 selects texture stage 0 COLOROP=SUBTRACT instead of MODULATE.
    pub texture_subtract: u32,
    /// +108 (0x2075; constructor 0): raw float bits added to world Z for the projected
    /// queue key only (0x14001f360). 0x22c4 copies it with +88..+104 (0x14001f520).
    pub depth_bias: u32,
    /// +112: floating-point XY scale track, before opacity in the update order.
    pub scale_motion: Option<Box<ScaleMotion>>,
    /// +120: signed hundredth-degree rotation track, before opacity.
    pub rotation_track: Option<Box<RotationTrack>>,
    /// +128: opacity track; rebuilding the draw state drops it.
    pub opacity_track: Option<Box<OpacityTrack>>,
    /// +136: frame animation; rebuilding the draw state drops it.
    pub animation: Option<Box<Animation>>,
    /// +144: signed 16-bit key positions, writing the 32-bit XY position words.
    pub position_motion: Option<Box<PositionMotion>>,
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
            texture_mode: 0,
            texture_subtract: 0,
            depth_bias: 0,
            scale_motion: None,
            rotation_track: None,
            opacity_track: None,
            animation: None,
            position_motion: None,
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
    pub(crate) fn snapshot_preview(clear: ClearState, objects: BTreeMap<u8, SceneObject>) -> Self {
        Self {
            clear,
            objects,
            ..Self::default()
        }
    }
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
    /// A fresh slot object, as 0x22bd creates for a negative child index.
    pub(crate) fn replace_object(&mut self, slot: u8, object: SceneObject) {
        self.objects.insert(slot, object);
    }
    /// 0x2308 creates a fresh object bound to the scene movie's first decoded frame.
    pub(crate) fn replace_movie_object(&mut self, slot: u8, object: SceneObject) {
        self.objects.insert(slot, object);
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        if let Some((pc, command @ 0x1e5..=0x1e9)) = vm.pending_system() {
            return self.rotation(vm, pc, command);
        }
        if matches!(vm.pending_system(), Some((_, 0x1e0..=0x1e4))) {
            return self.try_scale_motion(vm);
        }
        if matches!(vm.pending_system(), Some((_, 0x1ef..=0x1f3))) {
            return self.try_frame_animation(vm);
        }
        if matches!(vm.pending_system(), Some((_, 0x1f4..=0x1f8))) {
            return self.try_position_motion(vm);
        }
        if matches!(vm.pending_system(), Some((_, 0x1f9..=0x1fd))) {
            return self.try_world_motion(vm);
        }
        if self.fade.try_handle(vm)? {
            return Ok(true);
        }
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        if command == 0x78 {
            return self.world_position(vm, pc);
        }
        if command == 0x329 {
            let slot = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2329, e.0))?;
            let object = u8::try_from(slot)
                .ok()
                .and_then(|slot| self.objects.get_mut(&slot))
                .ok_or_else(|| {
                    error(
                        pc,
                        0x2329,
                        "effect reset needs an existing object in slots 0..255",
                    )
                })?;
            vm.complete_system(SystemResult(0x4004))?;
            // 0x14003b6a0 -> 0x14003a880: resets the special-effect header and
            // destroys every child, retaining the parent image/draw state and flags.
            // Special-effect creation (0x2328) is still an explicit barrier.
            object.children.clear();
            return Ok(true);
        }
        if matches!(command, 0x2c5 | 0x2c6) {
            return self.control_change(vm, pc, command);
        }
        if matches!(
            command,
            0x22 | 0x2d | 0x2e | 0x40 | 0x42..=0x4c | 0x50 | 0x70..=0x75 | 0x321 | 0x322
        ) {
            return self.handle_object(vm, pc, command);
        }
        if command == 0x23 {
            return self.destroy_child(vm, pc);
        }
        if command == 0x26 {
            return self.set_shown(vm, pc);
        }
        if matches!(command, 0x27 | 0x28) {
            return self.query_or_move(vm, pc, command);
        }
        if command == 0x33 {
            return self.image_info(vm, pc);
        }
        if command == 0x34 {
            return self.pixel(vm, pc);
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

    /// 0x2026: SP-4 object, SP-8 child (negative selects self), SP-12 boolean.
    /// Evidence and build-specific addresses: docs/games/chronoclock/vm-semantics.md.
    /// Missing/invalid targets stay explicit errors: VM +12056 bit 0x10 is unmodeled.
    fn set_shown(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, 0x2026, why);
        let slot = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let child = vm.stack().peek_u32(8).map_err(|e| fail(e.0))? as i32;
        let shown = vm.stack().peek_u32(12).map_err(|e| fail(e.0))? != 0;
        let parent = u8::try_from(slot)
            .ok()
            .and_then(|slot| self.objects.get_mut(&slot))
            .ok_or_else(|| fail("visibility needs an existing object in slots 0..255"))?;
        let target = if child < 0 {
            parent
        } else {
            u16::try_from(child)
                .ok()
                .filter(|id| *id < 768)
                .and_then(|id| parent.children.get_mut(&id))
                .ok_or_else(|| fail("visibility needs an existing child in slots 0..767"))?
        };
        vm.complete_system(SystemResult(0x400c))?;
        target.shown = shown;
        Ok(true)
    }

    /// 0x2023 / 0x14006bd10 -> 0x14003a7d0: remove SP-8 from parent SP-4.
    /// The child index is unsigned: values >=768, including negative i32 bits, are no-ops.
    /// Drop owns the subtree, images and effect state; parent flags/state remain untouched.
    fn destroy_child(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, 0x2023, why);
        let raw = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let child = vm.stack().peek_u32(8).map_err(|e| fail(e.0))?;
        let slot = u8::try_from(raw).map_err(|_| fail("object slot outside 0..255"))?;
        if !self.objects.contains_key(&slot) {
            return Err(fail("child deletion needs an existing parent object"));
        }
        vm.complete_system(SystemResult(0x4008))?;
        if child < 768 {
            self.objects
                .get_mut(&slot)
                .expect("validated parent")
                .children
                .remove(&(child as u16));
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

    /// 0x2034 / 0x14006ca80: SP-4 slot (as 0x2033), SP-8 child (negative selects the object,
    /// 0x14003a760 otherwise), SP-12/SP-16 signed x/y. Result 0 is image vtable +0x28
    /// (0x140030190): the BGRA dword of the CPU copy at +1160, or 0 outside +1144/+1148 or
    /// without an image. Only file-loaded, unfiltered images are verified to keep that copy
    /// equal to the decoded pixels. Pops 16.
    fn pixel(&mut self, vm: &mut Machine<'_>, pc: usize) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, 0x2034, why);
        let mut args = [0; 4];
        for (i, arg) in args.iter_mut().enumerate() {
            *arg = vm.stack().peek_u32(4 * (i + 1)).map_err(|e| fail(e.0))?;
        }
        let [slot, child, x, y] = args;
        let object = u8::try_from(slot as i32)
            .ok()
            .and_then(|slot| self.objects.get(&slot))
            .ok_or_else(|| fail("pixel query needs an existing object in slots 0..255"))?;
        let target = if (child as i32) < 0 {
            object
        } else {
            u16::try_from(child)
                .ok()
                .filter(|id| *id < 768)
                .and_then(|id| object.children.get(&id))
                .ok_or_else(|| fail("pixel query needs an existing child in slots 0..767"))?
        };
        let value = match &target.image {
            None => 0,
            Some(image) if image.surface || image.filter.is_some() || image.name.is_empty() => {
                return Err(fail(
                    "pixel reads are verified only for unfiltered file-loaded images",
                ));
            }
            Some(image) => {
                let pixels = &image.pixels;
                let (x, y) = (x as i32, y as i32);
                if x < 0 || y < 0 || x as u32 >= pixels.width() || y as u32 >= pixels.height() {
                    0
                } else {
                    let at = (y as usize * pixels.width() as usize + x as usize) * 4;
                    let [r, g, b, a] = pixels.pixels()[at..at + 4] else {
                        unreachable!("validated RGBA bounds")
                    };
                    u32::from_le_bytes([b, g, r, a])
                }
            }
        };
        vm.complete_system(SystemResult(0x4010))?;
        vm.state_mut()
            .set_system_integer(0, value)
            .expect("slot 0 exists");
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

    fn handle_object(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, command + 0x2000, why);
        let count = match command {
            0x40 => 1,
            0x22 | 0x2d | 0x2e | 0x42 | 0x50 => 2,
            0x43 => 5,
            0x44 => 6,
            0x45 | 0x46 | 0x49 => 4,
            0x47 | 0x48 | 0x4a..=0x4c | 0x70..=0x75 | 0x321 | 0x322 => 3,
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
            || (matches!(command, 0x2d | 0x2e | 0x42..=0x4c | 0x70..=0x75 | 0x321 | 0x322)
                && (args[1] as i32) < 0)
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
            if matches!(command, 0x42..=0x4c | 0x70..=0x75 | 0x321 | 0x322) && object.draw.is_none()
            {
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
                // 0x202d / 0x14006bfe0 -> 0x14003a720: attach to the parent subqueue.
                0x2d => object.flags |= 2,
                // 0x202e / 0x14006c0a0 -> 0x14003a740: clear +6248 bit 0x100.
                0x2e => object.flags &= !0x100,
                0x40 | 0x50 => object.draw = Some(DrawState::default()),
                // 0x2042 / 0x14006d130 -> 0x14003b190 -> 0x14001f550:
                // set the existing draw state's coordinate mode without rebuilding it.
                0x42 => object.draw.as_mut().expect("validated").words[0] = 2,
                0x321 => object.draw.as_mut().expect("validated").texture_mode = args[2],
                0x322 => {
                    object.draw.as_mut().expect("validated").texture_subtract =
                        u32::from(args[2] != 0)
                }
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
                // 0x204a / 0x14006d7f0 -> 0x14003b110 -> 0x14001f4a0:
                // cdq/xor/sub absolute value wraps for i32::MIN; remainder is signed.
                // +80 stores hundredths of a degree, independently of effect tracks.
                0x4a => {
                    object.draw.as_mut().expect("validated").words[20] =
                        ((args[2] as i32).wrapping_abs() % 360 * 100) as u32;
                }
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

    /// 0x2078 / 0x14006f4c0 -> 0x14003b210: raw XYZ to float results 8..10.
    fn world_position(&self, vm: &mut Machine<'_>, pc: usize) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, 0x2078, why);
        let slot = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let child = vm.stack().peek_u32(8).map_err(|e| fail(e.0))? as i32;
        let parent = u8::try_from(slot)
            .ok()
            .and_then(|slot| self.objects.get(&slot))
            .ok_or_else(|| fail("world query needs an existing object in slots 0..255"))?;
        let object = if child < 0 {
            parent
        } else {
            u16::try_from(child)
                .ok()
                .filter(|&id| id < 768)
                .and_then(|id| parent.children.get(&id))
                .ok_or_else(|| fail("world query needs an existing child in slots 0..767"))?
        };
        let words = &object
            .draw
            .as_ref()
            .ok_or_else(|| fail("world query needs a draw state"))?
            .words;
        let xyz = [words[11], words[12], words[13]];
        vm.complete_system(SystemResult(0x4008))?;
        for (index, bits) in (8..=10).zip(xyz) {
            vm.state_mut()
                .set_system_float_bits(index, bits)
                .expect("float result exists");
        }
        Ok(true)
    }

    /// 0x2030 / 0x14006c220: SP-4 object slot, SP-8 file name. Rebuilds the object image
    /// (0x14003bb10) and sets system result 0 to whether loading succeeded; returns 0x4008.
    /// 0x2038 uses SP-8 child slot and SP-12 name, and returns 0x400c.
    /// 0x203b / 0x14006c3b0 -> 0x14003bbc0 -> 0x140035260 adds mode/color/extra,
    /// filters decoded pixels before upload, and returns 0x4014. Only mode 2 is verified.
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
        let Some((pc, command @ (0x30 | 0x35 | 0x38 | 0x3b))) = vm.pending_system() else {
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
        let filter = if command == 0x3b {
            let mode = vm.stack().peek_u32(12).map_err(|e| fail(e.0))?;
            let color = vm.stack().peek_u32(16).map_err(|e| fail(e.0))?;
            // Stored at image +1212; not read by the mode 2 branch.
            let extra = vm.stack().peek_u32(20).map_err(|e| fail(e.0))?;
            if mode != 2 {
                return Err(
                    fail("image filter mode not implemented; only mode 2 is verified").into(),
                );
            }
            Some([mode, color, extra])
        } else {
            None
        };
        let name = vm.resolve_string(raw_name)?;
        if command == 0x35 {
            let first = vm.stack().peek_u32(12).map_err(|e| fail(e.0))?;
            let second = vm.stack().peek_u32(16).map_err(|e| fail(e.0))?;
            if first != 0 || second != 0 {
                return Err(fail("second texture nonzero load flags are not verified").into());
            }
            let decoded = load_image(images, name, pc, 0x2035)?;
            vm.complete_system(SystemResult(0x4010))?;
            vm.state_mut()
                .set_system_integer(0, u32::from(decoded.is_some()))
                .expect("slot 0 exists");
            self.objects
                .get_mut(&slot)
                .expect("validated")
                .secondary_image = decoded;
            return Ok(true);
        }
        let mut decoded = load_image(images, name, pc, command + 0x2000)?;
        if let Some(filter) = filter
            && let Some(image) = decoded.as_mut()
        {
            image.apply_filter(filter)?;
        }
        vm.complete_system(SystemResult(match command {
            0x38 => 0x400c,
            0x3b => 0x4014,
            _ => 0x4008,
        }))?;
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
        filter: None,
        name,
        pixels: decoded.pixels,
        alpha: decoded.alpha,
        surface: false,
    }))
}

impl SceneHost {
    /// Record 0x620 while the +4632 mask transition exists; the +4648 one is never saved.
    pub(crate) fn saved_mask(&self) -> Option<Vec<u8>> {
        self.masks[0].as_ref().map(MaskTransition::saved)
    }
    /// Record 0x600 when a quake runs.
    pub(crate) fn saved_quake(&self) -> Option<Vec<u8>> {
        self.quake.as_ref().map(Quake::saved)
    }
    /// Record 0x610 while the +4616 fade exists.
    pub(crate) fn saved_fade(&self) -> Option<Vec<u8>> {
        self.fade.current().map(crate::fade::Fade::saved)
    }
    /// Record 0x630 while the +4640 strip wipe exists.
    pub(crate) fn saved_strip(&self) -> Option<Vec<u8>> {
        self.strip.as_ref().map(StripWipe::saved)
    }
    /// Loader 0x140081960 destroys the slot objects and the +4616/+4624/+4632/+4640/+4648
    /// effects; the reader then rebuilds the saved objects, clear state, quake, fade, mask
    /// transition and strip wipe.
    pub(crate) fn restore(
        &mut self,
        clear: ClearState,
        objects: BTreeMap<u8, SceneObject>,
        quake: Option<Quake>,
        fade: Option<crate::fade::Fade>,
        mask: Option<MaskTransition>,
        strip: Option<StripWipe>,
    ) {
        *self = Self {
            clear,
            objects,
            quake,
            fade: crate::fade::FadeHost::restored(fade),
            masks: [mask, None],
            strip,
        };
    }
}
