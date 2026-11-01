//! Engine +4328 auxiliary text objects (0x14002fa40), distinct from TextObject.
use super::RESEARCH_SCREEN;
use crate::{
    FormatError, MAX_PAYLOAD,
    raster::RgbaImage,
    scene::{DrawSetter, DrawState, ObjectImage, SceneObject},
};
use std::collections::{BTreeMap, BTreeSet};

/// Logical glyph texture allocation. Only the initial tile is known to be clear;
/// unused native texture pixels are not guessed or exposed as a decoded image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuxiliaryGlyphAtlas {
    pub capacity: u32,
    pub cell_side: u32,
    pub texture_size: [u32; 2],
    pub grid_side: u32,
    pub cells_per_page: u32,
    pub initial_clear_tile: RgbaImage,
}

#[derive(Debug, Clone)]
pub(super) struct PositionMotion {
    pub mode: u32,
    pub base: [u32; 2],
    pub target: [u32; 2],
    pub duration: u32,
    pub start: u32,
    pub extra: u32,
}

impl super::MessageHost {
    /// 0x239d binds +1472. The bound branch of 0x2383 also moves that scene object.
    pub fn try_handle_scene(
        &mut self,
        vm: &mut crate::machine::Machine<'_>,
        scene: &mut crate::scene::SceneHost,
    ) -> Result<bool, crate::bytecode::DecodeError> {
        let Some((pc, command @ (0x39d | 0x383))) = vm.pending_system() else {
            return Ok(false);
        };
        let mut call = crate::dispatch::Call::new(vm, pc, command);
        if command == 0x383 {
            let slot = call.argument(4, "auxiliary slot")? as usize;
            let group = self
                .auxiliary_texts
                .get(slot)
                .and_then(Option::as_ref)
                .ok_or_else(|| call.error("auxiliary text group does not exist or outside 0..7"))?;
            if group.associated_scene.is_none() {
                return Ok(false);
            }
        }
        Ok(self.handle_auxiliary_scene(&mut call, Some(scene))? == crate::dispatch::Flow::Done)
    }

    pub(crate) fn handle_auxiliary_scene(
        &mut self,
        call: &mut crate::dispatch::Call<'_, '_>,
        scene: Option<&mut crate::scene::SceneHost>,
    ) -> Result<crate::dispatch::Flow, crate::bytecode::DecodeError> {
        use crate::dispatch::Flow;
        let slot = call.argument(4, "auxiliary slot")? as usize;
        let group = self
            .auxiliary_texts
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or_else(|| call.error("auxiliary text group does not exist or outside 0..7"))?;
        if call.command == 0x39d {
            let Some(scene) = scene else {
                return Ok(Flow::Unavailable);
            };
            let raw = call.argument(8, "scene object")?;
            let target = if (raw as i32) < 0 {
                None
            } else {
                Some(
                    u8::try_from(raw)
                        .ok()
                        .filter(|s| scene.object(*s).is_some())
                        .ok_or_else(|| {
                            call.error("associated scene object does not exist or outside 0..255")
                        })?,
                )
            };
            let flow = call.finish(0x4008)?;
            self.auxiliary_texts[slot]
                .as_mut()
                .expect("validated")
                .associated_scene = target;
            return Ok(flow);
        }
        let target = group.associated_scene;
        let x = call.argument(8, "x")?;
        let y = call.argument(12, "y")?;
        if let Some(target) = target {
            let Some(scene) = scene else {
                return Ok(Flow::Unavailable);
            };
            if scene.object(target).and_then(|o| o.draw.as_ref()).is_none() {
                return Err(call.error("associated scene object has no draw state"));
            }
            let flow = call.finish(0x400c)?;
            self.auxiliary_texts[slot]
                .as_mut()
                .expect("validated")
                .set_position(x, y);
            let position = [x, y].map(|v| {
                let f = (v as i32) as f32;
                if f >= 2147483648.0 {
                    i32::MIN as u32
                } else {
                    (f as i32) as u32
                }
            });
            scene
                .object_mut(target)
                .expect("validated")
                .draw
                .as_mut()
                .expect("validated")
                .set(DrawSetter::Position(position));
            Ok(flow)
        } else {
            let flow = call.finish(0x400c)?;
            self.auxiliary_texts[slot]
                .as_mut()
                .expect("validated")
                .set_position(x, y);
            Ok(flow)
        }
    }
}

impl AuxiliaryGlyphAtlas {
    fn new(glyph_size: u32, capacity: u32) -> Result<Self, FormatError> {
        let fail = FormatError("auxiliary glyph atlas exceeds supported allocation geometry");
        if (glyph_size as i32) < 0 {
            return Err(fail);
        }
        // 0x14002e325..0x14002e337. Reject overflowing source arithmetic instead
        // of reproducing allocations based on the native implementation's wrap.
        let cell_side = glyph_size
            .checked_mul(RESEARCH_SCREEN[0])
            .ok_or(fail.clone())?
            / RESEARCH_SCREEN[0]
            + 6;
        let capacity = if capacity == 0 { 512 } else { capacity };
        let side = if capacity <= 64 { 512u32 } else { 1024u32 };
        let grid_side = side / cell_side;
        if grid_side == 0 {
            return Err(fail);
        }
        let cells_per_page = grid_side * grid_side;
        let height = capacity
            .div_ceil(cells_per_page)
            .checked_mul(side)
            .ok_or(fail.clone())?;
        if u64::from(side) * u64::from(height) * 4 > MAX_PAYLOAD as u64 {
            return Err(fail);
        }
        let tile_bytes = cell_side as usize * cell_side as usize * 4;
        Ok(Self {
            capacity,
            cell_side,
            texture_size: [side, height],
            grid_side,
            cells_per_page,
            initial_clear_tile: RgbaImage::new(cell_side, cell_side, vec![0; tile_bytes])?,
        })
    }
}

/// A +184 record of 0x14002f3e0: the 40-byte glyph descriptor at +12, and the atlas cell
/// (+16) and layer that 0x14002eb10 gives it while it has an attached object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuxiliaryGlyph {
    /// The group's +4392 font face (game encoding) when the glyph was laid out.
    pub face: std::sync::Arc<[u8]>,
    /// Descriptor word 0 high half: Shift-JIS pair, lead byte high.
    pub code: u16,
    /// Descriptor words 3/4: truncated pen position relative to the group position.
    pub position: [i32; 2],
    /// Descriptor word 5: +48 when laid out.
    pub size: u32,
    /// Descriptor words 6/7: +132 and +136.
    pub colors: [u32; 2],
    /// Descriptor word 2: +148 values 1..3, other values map to zero.
    pub gradient: u32,
    /// Descriptor words 8/9: +140 (0x238b passes -1, selecting it) and +144.
    pub outline: [u32; 2],
    /// Atlas cell 1..1023 while attached, 0 otherwise.
    pub cell: u16,
    /// +172 when the attached object was created; later 0x238f calls do not change it.
    pub layer: u32,
}

/// The auxiliary text list: constructor state, layout fields, glyph records and the
/// glyphs currently attached to the atlas object. Glyph pixels belong to the backend.
#[derive(Debug, Clone)]
pub struct AuxiliaryText {
    /// Face of the slot's persistent +4392 font object, fixed when it was created.
    pub face: std::sync::Arc<[u8]>,
    pub(super) words: BTreeMap<u16, u32>,
    /// +184: glyph records in layout order.
    pub glyphs: Vec<AuxiliaryGlyph>,
    /// The atlas object's +6152 list: indexes of attached glyph records, oldest first.
    pub(super) attached: Vec<usize>,
    /// +416: atlas cells in use. Only atlas creation (0x14002e2f0) clears the table;
    /// 0x2386 drops the records without releasing their cells.
    pub(super) cells: BTreeSet<u16>,
    pub atlas: AuxiliaryGlyphAtlas,
    /// Owned atlas scene object; backing texture metadata belongs to `atlas`.
    /// It is not a scene slot and must never be drawn using guessed texture pixels.
    pub object: SceneObject,
    /// 0x2397 attaches this at object +6184 and selects draw mode +88 = 4.
    /// It is a second texture, not a replacement for the glyph atlas.
    pub secondary_image: Option<ObjectImage>,
    /// +1472: a non-owned scene object moved along with this text group.
    pub associated_scene: Option<u8>,
    motion: Option<PositionMotion>,
}

impl AuxiliaryText {
    pub(super) fn motion(&self) -> Option<&PositionMotion> {
        self.motion.as_ref()
    }
    pub(super) fn start_motion(&mut self, motion: PositionMotion) {
        for (at, value) in [
            (368, 1),
            (372, motion.mode),
            (376, motion.base[0]),
            (380, motion.base[1]),
            (384, motion.target[0]),
            (388, motion.target[1]),
            (392, motion.extra),
            (396, motion.duration),
            (400, motion.start),
        ] {
            self.words.insert(at, value);
        }
        self.motion = Some(motion);
    }
    pub(super) fn commit_motion_position(&mut self, position: [u32; 2], active: bool) {
        self.set_position(position[0], position[1]);
        self.words.insert(368, u32::from(active));
        if !active {
            self.motion = None;
        }
    }
    /// Known state words only; native pointers and unknown fields are omitted.
    pub fn word(&self, offset: u16) -> Option<u32> {
        self.words.get(&offset).copied()
    }

    /// 0x14002e5d0 -> 0x14003b0c0 / 0x14001f410. Both axes use
    /// width +352 / +360, with separate single-precision multiply and divide.
    pub(super) fn set_position(&mut self, x: u32, y: u32) {
        let width = self.words[&352] as f32;
        let logical_width = self.words[&360] as f32;
        let x = (x as i32) as f32;
        let y = (y as i32) as f32;
        self.words.insert(8, x.to_bits());
        self.words.insert(12, y.to_bits());
        let draw = self.object.draw.as_mut().expect("owned atlas draw state");
        draw.words[11] = (width * x / logical_width).to_bits();
        draw.words[12] = (width * y / logical_width).to_bits();
        draw.words[13] = 0;
        // try_handle_scene handles the optional +1472 object separately.
    }

    /// 0x14002e6e0 stores four signed layout parameters as f32 and repeats
    /// the first pair at +176/+180. It does not change the atlas draw state.
    pub(super) fn set_layout(&mut self, args: [u32; 4]) {
        for (offset, value) in [16, 20, 24, 28, 176, 180]
            .into_iter()
            .zip(args.into_iter().chain(args[..2].iter().copied()))
        {
            self.words.insert(offset, ((value as i32) as f32).to_bits());
        }
    }

    /// 0x14002e730: +48 is an integer size, +52/+56 are f32 parameters.
    /// Signed nonpositive size and negative float parameters mean "keep".
    /// This setter never reallocates the atlas created by 0x2380.
    pub(super) fn set_text_parameters(&mut self, size: u32, first: u32, second: u32) {
        if (size as i32) > 0 {
            self.words.insert(48, size);
        }
        for (offset, value) in [(52, first), (56, second)] {
            if (value as i32) >= 0 {
                self.words.insert(offset, ((value as i32) as f32).to_bits());
            }
        }
    }

    /// 0x14002e7e0: each color updates only when its high byte is zero.
    /// Nonzero high bytes mean "keep", rather than an alpha component.
    pub(super) fn set_colors(&mut self, first: u32, second: u32) {
        for (offset, color) in [(132, first), (136, second)] {
            if color & 0xff00_0000 == 0 {
                self.words.insert(offset, color);
            }
        }
    }

    /// 0x14002e840 writes the complete parameter word without validation; 0x238b maps
    /// 1..3 to the glyph gradient and other values to none.
    pub(super) fn set_field_148(&mut self, value: u32) {
        self.words.insert(148, value);
    }

    /// 0x14001d2a0 stores the complete parameter word at +140, the glyph outline mode.
    pub(super) fn set_field_140(&mut self, value: u32) {
        self.words.insert(140, value);
    }

    /// 0x14002e850 stores the complete parameter word at +144, the glyph outline color.
    pub(super) fn set_field_144(&mut self, value: u32) {
        self.words.insert(144, value);
    }

    /// 0x14002e870 -> 0x14003b0d0 -> 0x14001f430.
    pub(super) fn set_layer(&mut self, value: u32) {
        self.words.insert(172, value);
        self.object
            .draw
            .as_mut()
            .expect("owned atlas draw state")
            .set(DrawSetter::Layer(value));
    }

    /// 0x14002e8e0 destroys the attached glyph objects (0x14003a880) and the glyph records
    /// (0x14002e540), then resets the layout cursor. The atlas cell table is kept.
    pub(super) fn clear_content(&mut self) {
        self.object.children.clear();
        self.glyphs.clear();
        self.attached.clear();
        self.words.insert(176, self.words[&16]);
        self.words.insert(180, self.words[&20]);
    }

    /// 0x14002e800: the layout cursor +176/+180 from two signed integers.
    pub(super) fn set_cursor(&mut self, x: u32, y: u32) {
        self.words.insert(176, ((x as i32) as f32).to_bits());
        self.words.insert(180, ((y as i32) as f32).to_bits());
    }

    /// 0x14002eb00 clears only the motion flag +368; the frame update then skips the motion.
    pub(super) fn stop_motion(&mut self) {
        self.words.insert(368, 0);
        self.motion = None;
    }

    /// Glyph records with an attached object, in the atlas object's list order.
    pub fn attached_glyphs(&self) -> impl Iterator<Item = &AuxiliaryGlyph> {
        self.attached.iter().map(|&i| &self.glyphs[i])
    }

    /// 0x14002e880 stores four signed integer parameters as single precision.
    pub(super) fn set_bounds(&mut self, args: [u32; 4]) {
        for (offset, value) in [32, 36, 40, 44].into_iter().zip(args) {
            self.words.insert(offset, ((value as i32) as f32).to_bits());
        }
    }

    /// 0x14002e8c0 stores four complete signed bounds without normalization.
    pub(super) fn set_position_bounds(&mut self, args: [u32; 4]) {
        for (offset, value) in [152, 156, 160, 164].into_iter().zip(args) {
            self.words.insert(offset, value);
        }
    }

    /// 0x14002e6d0: CVTTSS2SI of stored f32 positions, including integer indefinite.
    pub(super) fn position(&self) -> [u32; 2] {
        [8, 12].map(|at| {
            let value = f32::from_bits(self.words[&at]);
            if !(-2147483648.0..2147483648.0).contains(&value) {
                i32::MIN as u32
            } else {
                (value as i32) as u32
            }
        })
    }

    /// 0x14002eaa0 performs sequential signed lower/upper clamps, even inverted bounds.
    pub(super) fn clamp_position(&self, x: u32, y: u32) -> [u32; 2] {
        [(x, 152, 156), (y, 160, 164)].map(|(v, lo, hi)| {
            (v as i32)
                .max(self.words[&lo] as i32)
                .min(self.words[&hi] as i32) as u32
        })
    }

    pub(super) fn new(glyph_size: u32, capacity: u32) -> Result<Self, FormatError> {
        let atlas = AuxiliaryGlyphAtlas::new(glyph_size, capacity)?;
        let mut words: BTreeMap<_, _> = (8..184)
            .step_by(4)
            .chain((192..340).step_by(4))
            .map(|at| (at, 0))
            .collect();
        for (offset, value) in [
            (40, (RESEARCH_SCREEN[0] as f32).to_bits()),
            (44, (RESEARCH_SCREEN[1] as f32).to_bits()),
            (48, glyph_size),
            (124, 0),
            (128, 1),
            (132, 0xffffff),
            (136, 0xffffff),
            (140, 2),
            (148, 0),
            (168, 1),
            (172, 1),
            (192, 0),
            (196, 148),
            (200, 0),
            (204, 12),
            (208, 0),
            (340, atlas.capacity),
            (368, 0),
            (404, atlas.grid_side),
            (408, atlas.grid_side),
            (412, atlas.cells_per_page),
            (1440, atlas.texture_size[0]),
        ] {
            words.insert(offset, value);
        }
        for (i, value) in RESEARCH_SCREEN.into_iter().cycle().take(6).enumerate() {
            words.insert(344 + 4 * i as u16, value);
        }
        let mut draw = DrawState::default();
        draw.words[0] = 5;
        draw.set(DrawSetter::Source([0, 0, 4, 4]));
        draw.set(DrawSetter::Layer(1));
        Ok(Self {
            face: std::sync::Arc::from(crate::settings::DEFAULT_FONT_FACE),
            words,
            glyphs: Vec::new(),
            attached: Vec::new(),
            cells: BTreeSet::new(),
            atlas,
            secondary_image: None,
            associated_scene: None,
            motion: None,
            object: SceneObject {
                draw: Some(draw),
                logical_size: RESEARCH_SCREEN,
                ..SceneObject::default()
            },
        })
    }
}
