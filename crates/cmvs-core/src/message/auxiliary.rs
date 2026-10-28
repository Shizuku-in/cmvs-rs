//! Engine +4328 auxiliary text objects (0x14002fa40), distinct from TextObject.
use super::RESEARCH_SCREEN;
use crate::{
    FormatError, MAX_PAYLOAD,
    raster::RgbaImage,
    scene::{DrawSetter, DrawState, SceneObject},
};
use std::collections::BTreeMap;

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

/// Constructor state for the auxiliary text list. Content/list commands remain
/// barriers, so it has no text rows or glyphs and produces no visible pixels.
#[derive(Debug, Clone)]
pub struct AuxiliaryText {
    words: BTreeMap<u16, u32>,
    pub atlas: AuxiliaryGlyphAtlas,
    /// Owned atlas scene object; backing texture metadata belongs to `atlas`.
    /// It is not a scene slot and must never be drawn using guessed texture pixels.
    pub object: SceneObject,
}

impl AuxiliaryText {
    /// Known constructor words only; native pointers and unknown fields are omitted.
    pub fn word(&self, offset: u16) -> Option<u32> {
        self.words.get(&offset).copied()
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
            words,
            atlas,
            object: SceneObject {
                draw: Some(draw),
                logical_size: RESEARCH_SCREEN,
                ..SceneObject::default()
            },
        })
    }
}
