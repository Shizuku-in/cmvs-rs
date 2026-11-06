//! BASE 0x2328: 0x14003d190 construction, 0x14003b6c0 wave update.
use super::{DrawState, SceneHost, SceneObject};
use crate::{FormatError, bytecode::DecodeError, dispatch::Call};

/// The native writer saves the header, then recreates the independent list on load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectEffect {
    /// Active, nine parameters, elapsed (native +6264..+6304).
    pub words: [u32; 11],
    pub children: Vec<SceneObject>,
    active_start: usize,
}

impl ObjectEffect {
    pub(crate) fn inactive(words: [u32; 11]) -> Self {
        Self {
            words,
            children: Vec::new(),
            active_start: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.words[..3].fill(0);
        self.words[10] = 0;
        self.children.clear();
        self.active_start = 0;
    }

    pub(crate) fn generated_count(
        draw: &DrawState,
        parameters: &[u32; 9],
    ) -> Result<usize, FormatError> {
        let d = &draw.words;
        if !matches!(parameters[0], 1 | 16) {
            return Err(FormatError(
                "object effect supports only row wave (1) and afterimages (16)",
            ));
        }
        let count = if parameters[0] == 1 {
            d[5]
        } else {
            parameters[5]
        };
        if count > if parameters[0] == 1 { 2048 } else { 1024 }
            || d[4] > i16::MAX as u32
            || d[5] > i16::MAX as u32
        {
            return Err(FormatError(
                "object effect exceeds its native index table or geometry bounds",
            ));
        }
        if parameters[0] == 1 && count != 0 && (parameters[4] == 0 || parameters[8] == 0) {
            return Err(FormatError("row wave spacing and period must be nonzero"));
        }
        Ok(if parameters[0] == 1 {
            count
        } else {
            count.saturating_mul(2).saturating_sub(1)
        } as usize)
    }

    pub(crate) fn create(
        draw: &DrawState,
        logical_size: [u32; 2],
        parameters: [u32; 9],
    ) -> Result<Self, FormatError> {
        let mut words = [0; 11];
        words[0] = 1;
        words[1..10].copy_from_slice(&parameters);
        let d = &draw.words;
        Self::generated_count(draw, &parameters)?;
        let count = if words[1] == 1 { d[5] } else { words[6] };
        let mut effect = Self {
            words,
            children: Vec::new(),
            active_start: 0,
        };
        for i in 0..count {
            let child = |negative: bool| {
                let mut draw = DrawState::default();
                let w = &mut draw.words;
                w[4] = d[4];
                w[5] = if words[1] == 1 { 1 } else { d[5] };
                w[3] = if words[1] == 1 { i } else { 0 };
                w[6..8].copy_from_slice(&d[6..8]);
                w[8..10].copy_from_slice(&d[8..10]);
                w[10] = if words[1] == 1 {
                    d[10]
                } else {
                    d[10].wrapping_sub(i)
                };
                w[17] = if words[1] == 1 { d[17] } else { 255 / (i + 1) };
                if words[1] == 1 {
                    w[18] = (words[4] as f32 / 1000.0).to_bits();
                    let offset = Self::wave(&words, i, d[4]).wrapping_add(d[4] / 2);
                    w[6] = d[6].wrapping_add(offset);
                    w[8] = d[8].wrapping_add(offset);
                    w[9] = d[9].wrapping_add(i);
                } else {
                    let offset = words[5].wrapping_mul(i);
                    w[8] = if negative {
                        d[8].wrapping_sub(offset)
                    } else {
                        d[8].wrapping_add(offset)
                    };
                }
                SceneObject {
                    draw: Some(draw),
                    logical_size,
                    flags: 0,
                    ..Default::default()
                }
            };
            effect.children.push(child(false));
            if words[1] == 16 && i != 0 {
                effect.children.push(child(true));
            }
        }
        Ok(effect)
    }

    fn wave(words: &[u32; 11], row: u32, width: u32) -> u32 {
        let phase = words[10].wrapping_add(row.wrapping_mul(words[8]) / words[5]);
        let angle = (phase % words[9]).wrapping_mul(36000) / words[9];
        let radians = f64::from(angle as i32) / 18000.0 * std::f64::consts::PI;
        (radians.sin() * f64::from(width / 2)) as i32 as u32
    }

    pub(crate) fn advance(&mut self, delta: u32, draw: &DrawState) {
        if self.words[1] != 1 {
            return;
        }
        self.words[10] = self.words[10].wrapping_add(delta);
        let d = &draw.words;
        for row in 0..(self.children.len() - self.active_start).min(d[5] as usize) {
            let offset = Self::wave(&self.words, row as u32, d[4]);
            let child = &mut self.children[self.active_start + row];
            let w = &mut child.draw.as_mut().expect("effect child draw").words;
            w[6] = d[6].wrapping_add(d[4] / 2).wrapping_add(offset);
            w[8] = d[8].wrapping_add(d[4] / 2).wrapping_add(offset);
            w[9] = d[9].wrapping_add(row as u32);
            w[10] = d[10];
            w[17] = d[17];
        }
    }

    pub(crate) fn validate_live(&self, draw: &DrawState) -> Result<(), FormatError> {
        if self.words[1] == 1 && draw.words[5] as usize > self.children.len() - self.active_start {
            return Err(FormatError(
                "row wave source height exceeds its allocated child table",
            ));
        }
        Ok(())
    }

    pub(crate) fn append_previous(&mut self, mut previous: Self) -> Result<(), FormatError> {
        if previous.children.len() + self.children.len() > 16_384 {
            return Err(FormatError(
                "repeated object effects exceed the independent child budget",
            ));
        }
        self.active_start = previous.children.len();
        previous.children.append(&mut self.children);
        self.children = previous.children;
        Ok(())
    }
}

impl SceneHost {
    pub(super) fn create_object_effect(
        &mut self,
        call: &mut Call<'_, '_>,
    ) -> Result<(), DecodeError> {
        let slot = call.argument(4, "object slot")?;
        let mut parameters = [0; 9];
        for (i, parameter) in parameters.iter_mut().enumerate() {
            *parameter = call.argument(8 + i * 4, "object effect parameter")?;
        }
        let object = u8::try_from(slot)
            .ok()
            .and_then(|s| self.objects.get(&s))
            .ok_or_else(|| call.error("object effect needs an existing object in slots 0..255"))?;
        let draw = object
            .draw
            .as_ref()
            .ok_or_else(|| call.error("object effect needs a draw state"))?;
        let mut effect = ObjectEffect::create(draw, object.logical_size, parameters)
            .map_err(|e| call.error(e.0))?;
        if let Some(previous) = &object.special_effect {
            effect
                .append_previous((**previous).clone())
                .map_err(|e| call.error(e.0))?;
        }
        call.complete(0x4028)?;
        self.objects
            .get_mut(&(slot as u8))
            .expect("validated")
            .special_effect = Some(Box::new(effect));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{raster::RgbaImage, scene::ObjectImage, scene_render};

    fn parent() -> SceneObject {
        let mut draw = DrawState::default();
        draw.words[4..6].copy_from_slice(&[4, 3]);
        draw.words[10] = 10;
        SceneObject {
            image: Some(ObjectImage {
                pixels: RgbaImage::new(
                    4,
                    3,
                    [
                        [255, 0, 0, 255].repeat(4),
                        [0, 255, 0, 255].repeat(4),
                        [0, 0, 255, 255].repeat(4),
                    ]
                    .concat(),
                )
                .unwrap(),
                alpha: false,
                filter: None,
                name: Vec::new(),
                surface: false,
            }),
            draw: Some(draw),
            ..Default::default()
        }
    }
    fn wave() -> [u32; 9] {
        [1, 0, 0, 500, 1, 0, 0, 250, 1000]
    }

    #[test]
    fn row_wave_uses_native_phase_origin_scale_and_inherited_texture() {
        let mut parent = parent();
        let mut effect =
            ObjectEffect::create(parent.draw.as_ref().unwrap(), [0; 2], wave()).unwrap();
        assert_eq!(effect.children.len(), 3);
        assert_eq!(
            effect
                .children
                .iter()
                .map(|c| c.draw.as_ref().unwrap().words[8])
                .collect::<Vec<_>>(),
            [2, 4, 2]
        );
        for (row, c) in effect.children.iter().enumerate() {
            let d = &c.draw.as_ref().unwrap().words;
            assert_eq!(&d[2..6], &[0, row as u32, 4, 1]);
            assert_eq!(d[18], 0.5f32.to_bits());
            assert!(c.image.is_none());
        }
        let initial = effect.children.clone();
        effect.advance(1000, parent.draw.as_ref().unwrap());
        assert_eq!(effect.children, initial);
        assert_eq!(effect.words[10], 1000); // no 200/500ms clamp
        effect.advance(250, parent.draw.as_ref().unwrap());
        assert_eq!(effect.children[0].draw.as_ref().unwrap().words[8], 4);
        parent.special_effect = Some(Box::new(
            ObjectEffect::create(parent.draw.as_ref().unwrap(), [0; 2], wave()).unwrap(),
        ));
        let mut scene = SceneHost::default();
        scene.objects.insert(29, parent);
        let frame = scene_render::prepare_scene(&scene, None, &[], 4, 3).unwrap();
        assert_eq!(frame.commands().len(), 3);
        let pixels = frame.render_cpu().unwrap();
        assert_eq!(&pixels.pixels()[4..8], &[255, 0, 0, 255]);
        assert_eq!(&pixels.pixels()[24..28], &[0, 255, 0, 255]);
        scene.objects.get_mut(&29).unwrap().shown = false;
        assert!(
            scene_render::prepare_scene(&scene, None, &[], 4, 3)
                .unwrap()
                .commands()
                .is_empty()
        );
    }

    #[test]
    fn afterimages_keep_native_order_opacity_and_prior_independent_children() {
        let parent = parent();
        let draw = parent.draw.as_ref().unwrap();
        let mut effect =
            ObjectEffect::create(draw, [1280, 720], [16, 0, 0, 0, 3, 3, 0, 0, 0]).unwrap();
        assert_eq!(effect.children.len(), 5);
        let values: Vec<_> = effect
            .children
            .iter()
            .map(|c| {
                let d = &c.draw.as_ref().unwrap().words;
                (d[8] as i32, d[10], d[17])
            })
            .collect();
        assert_eq!(
            values,
            [
                (0, 10, 255),
                (3, 9, 127),
                (-3, 9, 127),
                (6, 8, 85),
                (-6, 8, 85)
            ]
        );
        let before = effect.clone();
        effect.advance(100000, draw);
        assert_eq!(effect, before);
        let mut next = ObjectEffect::create(draw, [0; 2], wave()).unwrap();
        next.append_previous(effect).unwrap();
        assert_eq!(next.children.len(), 8);
        let old = next.children[..5].to_vec();
        next.advance(250, draw);
        assert_eq!(next.children[..5], old);
    }
}
