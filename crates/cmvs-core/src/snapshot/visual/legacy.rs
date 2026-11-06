//! PURIRING writers 0x415b70 (scene), 0x411950 (image), and their readers
//! 0x415c40 / 0x413e00. Unlike later scene v1, there is no version prefix.
use super::*;

const LAYOUT: SnapshotLayout = SnapshotLayout::TextV1SceneLegacy;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_scene_header_and_image_v1_are_not_later_scene_v1() {
        // Independent writer layout: three words, a v1 image chunk, terminator.
        let mut bytes = Vec::new();
        words(&mut bytes, &[0, 1, 0x7fff_ffff]);
        let mut image = vec![1, 0, 0, 0, 0, 0, 0, 0];
        image.extend(b"bg.pb3\0\0");
        words(&mut image, &[0, 1024, 614]);
        chunk(&mut bytes, 0x740, &image);
        bytes.extend([0xf0, 0xf0]);
        let saved = SavedScene::decode_with_layout(&bytes, 0, &mut 0, LAYOUT).unwrap();
        assert_eq!(saved.image.as_ref().unwrap().name, b"bg.pb3");
        assert_eq!(saved.image.as_ref().unwrap().size, [1024, 614]);
        assert_eq!(saved.logical_size, [0; 2]);
        assert_eq!(saved.encode_with_layout(LAYOUT).unwrap(), bytes);
        assert!(
            SavedScene::decode_with_layout(&bytes, 0, &mut 0, SnapshotLayout::TextV1SceneV1)
                .is_err()
        );
        for end in 0..bytes.len() {
            assert!(SavedScene::decode_with_layout(&bytes[..end], 0, &mut 0, LAYOUT).is_err());
        }
    }

    #[test]
    fn legacy_draw_and_children_resume_old_motion_semantics() {
        use crate::scene::{MotionKey, MotionPlayback, RotationKey, RotationTrack, WorldMotion};
        let mut draw = DrawState::default();
        let mut world = WorldMotion {
            legacy_20070119: true,
            ..Default::default()
        };
        world.keys[0] = MotionKey {
            next: -1,
            xyz: [2f32.to_bits(), 0, 0],
            mode: 1,
            duration: 1000,
        };
        world.playback = Some(MotionPlayback {
            base: [0; 3],
            start: 0,
            current: 0,
            elapsed: 0,
        });
        draw.world_motion = Some(Box::new(world));
        let mut rotation = RotationTrack {
            playing: true,
            base: 0,
            start: 0,
            current: 0,
            elapsed: 0,
            keys: [RotationKey::default(); 8],
        };
        rotation.keys[0] = RotationKey {
            next: -1,
            value: 9000,
            mode: 0xff,
            duration: 1000,
        };
        draw.rotation_track = Some(Box::new(rotation));
        let object = SceneObject {
            draw: Some(draw),
            ..Default::default()
        };
        let mut saved = SavedScene::from_object(&object).unwrap();
        saved.flags |= 0x8000_0000;
        saved.children.insert(
            255,
            SavedScene::from_object(&SceneObject::default()).unwrap(),
        );
        let bytes = saved.encode_with_layout(LAYOUT).unwrap();
        assert_eq!(&bytes[..4], &[1, 0, 0, 0]);
        assert_eq!(&bytes[12..18], &[0x80, 7, 0xde, 1, 0, 0]); // draw: 92 + 156 + 228 + 2
        let restored = SavedScene::decode_with_layout(&bytes, 0, &mut 0, LAYOUT).unwrap();
        assert_eq!(restored.encode_with_layout(LAYOUT).unwrap(), bytes);
        let mut object = SceneObject {
            draw: restored.draw,
            ..Default::default()
        };
        object.advance_effects(200);
        let draw = object.draw.unwrap();
        assert_eq!(draw.words[20], 0); // hold rotation until the key ends
        let x = f32::from_bits(draw.words[11]);
        assert!((x - 0.61).abs() < 0.02); // sine-quarter, not modern quadratic
        saved.children.insert(
            256,
            SavedScene::from_object(&SceneObject::default()).unwrap(),
        );
        assert!(saved.encode_with_layout(LAYOUT).is_err());
    }
}

impl SavedImage {
    pub(super) fn encode_legacy(&self) -> Result<Vec<u8>, FormatError> {
        if self.filter.is_some() || self.disabled_filter != [0; 3] || self.surface.is_some() {
            return Err(FormatError(
                "legacy image cannot represent filters or surfaces",
            ));
        }
        let mut out = vec![1, 0, 0, 0];
        words(&mut out, &[0]);
        out.extend(&self.name);
        out.extend([0, 0]);
        words(&mut out, &[0, self.size[0], self.size[1]]);
        // Apply the same bounds to captured data and external save records.
        Self::decode_legacy(&out)?;
        Ok(out)
    }

    pub(super) fn decode_legacy(bytes: &[u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(bytes);
        if r.u16()? != 1 || r.u16()? != 0 || r.u32()? != 0 {
            return Err(FormatError("unsupported legacy image version or kind"));
        }
        let name = r.string(64)?.to_vec();
        if !r.string(32)?.is_empty() {
            return Err(FormatError("unsupported legacy second image name"));
        }
        let [manager, width, height] = r.words::<3>()?;
        r.finish()?;
        if manager != 0 {
            return Err(FormatError(
                "legacy alternate image manager is not implemented",
            ));
        }
        if width == 0
            || height == 0
            || u64::from(width) * u64::from(height) > (MAX_PIXELS / 4) as u64
        {
            return Err(FormatError("legacy image dimensions exceed preview budget"));
        }
        Ok(Self {
            name,
            size: [width, height],
            filter: None,
            disabled_filter: [0; 3],
            surface: None,
        })
    }
}

impl SavedScene {
    pub(super) fn encode_legacy(&self) -> Result<Vec<u8>, FormatError> {
        if self.secondary_image.is_some()
            || self.collision_vertices.is_some()
            || self.effect_words != [0; 11]
        {
            return Err(FormatError(
                "legacy scene cannot represent secondary images or effects",
            ));
        }
        if !self.children.is_empty() && self.flags & 0x8000_0000 == 0 {
            return Err(FormatError("snapshot children lack inheritance flag"));
        }
        let mut out = Vec::new();
        words(
            &mut out,
            &[
                u32::from(self.draw.is_some()),
                u32::from(self.shown),
                self.flags,
            ],
        );
        if let Some(image) = &self.image {
            chunk(&mut out, 0x740, &image.encode_legacy()?);
        }
        if let Some(draw) = &self.draw {
            chunk(&mut out, 0x780, &draw::encode_with_layout(draw, LAYOUT)?);
        }
        for (&slot, child) in &self.children {
            if slot >= 256 {
                return Err(FormatError("legacy child slot exceeds 255"));
            }
            out.extend(0x700u16.to_le_bytes());
            out.extend(slot.to_le_bytes());
            let body = child.encode_with_layout(LAYOUT)?;
            out.extend((body.len() as u32).to_le_bytes());
            out.extend(body);
        }
        out.extend(0xf0f0u16.to_le_bytes());
        Ok(out)
    }

    pub(super) fn decode_legacy(
        bytes: &[u8],
        depth: usize,
        nodes: &mut usize,
    ) -> Result<Self, FormatError> {
        let mut r = Reader::new(bytes);
        let [kind, shown, flags] = r.words::<3>()?;
        if kind > 1 {
            return Err(FormatError("unsupported legacy scene kind"));
        }
        let image = if r.peek() == Some(0x740) {
            r.u16()?;
            Some(SavedImage::decode_legacy(r.chunk()?)?)
        } else {
            None
        };
        let draw = if r.peek() == Some(0x780) {
            r.u16()?;
            Some(draw::decode_with_layout(r.chunk()?, LAYOUT)?)
        } else {
            None
        };
        if (kind == 1) != draw.is_some() {
            return Err(FormatError("legacy scene kind disagrees with draw state"));
        }
        let mut children = BTreeMap::new();
        while r.peek() == Some(0x700) {
            r.u16()?;
            let slot = r.u16()?;
            let body = r.chunk()?;
            if slot >= 256 || children.contains_key(&slot) || flags & 0x8000_0000 == 0 {
                return Err(FormatError("invalid legacy child slot or inheritance flag"));
            }
            children.insert(
                slot,
                Self::decode_with_layout(body, depth + 1, nodes, LAYOUT)?,
            );
        }
        if r.u16()? != 0xf0f0 {
            return Err(FormatError("invalid legacy scene terminator"));
        }
        r.finish()?;
        Ok(Self {
            shown: shown != 0,
            flags,
            logical_size: [0; 2],
            image,
            draw,
            children,
            secondary_image: None,
            collision_vertices: None,
            effect_words: [0; 11],
        })
    }
}
