//! Bounded, version-specific visual records. This is an offline component preview,
//! never a replacement for the engine snapshot loader. Evidence: snapshot-format.md.
use super::{SnapshotLayout, SnapshotRecord, parse_records, unique_body};
use crate::{
    FormatError,
    image::{self, ImageSource},
    message::{MessageHost, MessageWindow, TextObject},
    save::Csv2Save,
    scene::{ClearState, DrawState, ObjectImage, SceneHost, SceneObject},
};
use std::{collections::BTreeMap, error::Error};
mod collision;
mod draw;
pub(super) mod text;
pub(super) mod window;

const MAX_NODES: usize = 2048;
const MAX_DEPTH: usize = 32;
const MAX_PIXELS: usize = 64 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::ImageChange;

    struct NoImages;
    impl ImageSource for NoImages {
        fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
            panic!("retained replacement resources must not be loaded");
        }
    }

    #[test]
    fn v1_world_tracks_restore_and_continue_without_losing_axes_or_timing() {
        use crate::scene::{MotionKey, MotionPlayback, WorldMotion};
        for layout in [SnapshotLayout::TextV1SceneV1, SnapshotLayout::TextV1SceneV2] {
            for mode in [0x20, 0x21, 0x00c80030, 0x00c80040] {
                let mut track = WorldMotion {
                    playback: Some(MotionPlayback {
                        base: [1f32.to_bits(), 2f32.to_bits(), 0x80000000],
                        start: 0,
                        current: 0,
                        elapsed: 100,
                    }),
                    ..Default::default()
                };
                track.keys[0] = MotionKey {
                    next: -1,
                    xyz: [4f32.to_bits(), 5f32.to_bits(), 6f32.to_bits()],
                    mode,
                    duration: 400,
                };
                let mut object = SceneObject {
                    draw: Some(DrawState {
                        world_motion: Some(Box::new(track.clone())),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                object.draw.as_mut().unwrap().words[11..14]
                    .copy_from_slice(&track.playback.as_ref().unwrap().base);
                let saved = SavedScene::from_object(&object).unwrap();
                let bytes = saved.encode_with_layout(layout).unwrap();
                let decoded = SavedScene::decode_with_layout(&bytes, 0, &mut 0, layout).unwrap();
                let loaded = decoded.load(&mut NoImages, &mut 0).unwrap();
                assert_eq!(
                    loaded.draw.as_ref().unwrap().world_motion.as_deref(),
                    Some(&track)
                );
                let mut scene = SceneHost::snapshot_preview(
                    ClearState::default(),
                    BTreeMap::from([(20, loaded)]),
                );
                scene.advance_frame(100);
                let draw = scene.object(20).unwrap().draw.as_ref().unwrap();
                let middle = match mode {
                    0x20 => 3f32,
                    0x21 => 2.25,
                    0x00c80040 => 2.0,
                    _ => 0.0,
                };
                assert_eq!(
                    &draw.words[11..14],
                    &[1f32.to_bits(), middle.to_bits(), 0x80000000]
                );
                if mode == 0x00c80040 {
                    scene.advance_frame(100);
                    let draw = scene.object(20).unwrap().draw.as_ref().unwrap();
                    assert_eq!(
                        &draw.words[11..14],
                        &[(-1f32).to_bits(), 2f32.to_bits(), 0x80000000]
                    );
                    scene.advance_frame(100);
                } else {
                    scene.advance_frame(200);
                }
                let draw = scene.object(20).unwrap().draw.as_ref().unwrap();
                assert_eq!(&draw.words[11..14], &track.keys[0].xyz);
                assert!(draw.world_motion.is_none());
                assert!(SavedScene::decode(&bytes, 0, &mut 0).is_err());
            }
        }
    }

    #[test]
    fn realive_scene_header_and_nested_child_limits_are_layout_specific() {
        let layout = SnapshotLayout::TextV3SceneV4;
        let mut parent = SceneObject {
            flags: 0x8000_0000,
            logical_size: [1280, 720],
            ..SceneObject::default()
        };
        parent.children.insert(1023, SceneObject::default());
        let saved = SavedScene::from_object(&parent).unwrap();
        let bytes = saved.encode_with_layout(layout).unwrap();
        assert_eq!(&bytes[..4], &[255, 255, 4, 0]);
        assert_eq!(&bytes[16..24], &[0, 5, 0, 0, 208, 2, 0, 0]);
        assert_eq!(
            SavedScene::decode_with_layout(&bytes, 0, &mut 0, layout).unwrap(),
            saved
        );
        assert!(SavedScene::decode(&bytes, 0, &mut 0).is_err());
        assert!(saved.encode().is_err());

        let state = MessageWindow {
            text: 0,
            visible: true,
            hidden: false,
            position: [0; 2],
            switch_positions: [[0; 2]; 2],
            size: [1280, 720],
            cursor_mode: 0,
            cursor_offset: [0; 2],
            position_switch: false,
            object: SceneObject::default(),
            buttons: [crate::message::WindowButton::default(); 32],
            pressed_button: -1,
            switch_state: 0,
        };
        let mut window = SavedWindow::from_window(&state, [1280, 720]).unwrap();
        window.scene = saved.clone();
        let bytes = window::encode_with_layout(&window, layout).unwrap();
        assert_eq!(
            window::decode_with_layout(&bytes, &mut 0, layout)
                .unwrap()
                .scene,
            saved
        );
        assert!(window::decode(&bytes, &mut 0).is_err());

        let mut invalid = saved;
        let leaf = invalid.children.remove(&1023).unwrap();
        invalid.children.insert(1024, leaf);
        assert!(invalid.encode_with_layout(layout).is_err());
        // Put an invalid child index into the otherwise valid native record.
        let mut parent = SavedScene::from_object(&parent)
            .unwrap()
            .encode_with_layout(layout)
            .unwrap();
        parent[70..72].copy_from_slice(&1024u16.to_le_bytes());
        assert!(SavedScene::decode_with_layout(&parent, 0, &mut 0, layout).is_err());
    }

    #[test]
    fn collision_mesh_roundtrip_preserves_empty_and_full_streams_in_children() {
        for count in [0, 1, 3, 128] {
            let vertices =
                vec![[(-0.0f32).to_bits(), (-2.75f32).to_bits(), 1.5f32.to_bits()]; count];
            let child = SceneObject {
                collision_vertices: Some(vertices.clone()),
                draw: Some(DrawState::default()),
                ..SceneObject::default()
            };
            let mut parent = SceneObject::default();
            parent.flags |= 0x8000_0000;
            parent.children.insert(767, child);
            let saved = SavedScene::from_object(&parent).unwrap();
            let decoded = SavedScene::decode(&saved.encode().unwrap(), 0, &mut 0).unwrap();
            assert_eq!(decoded, saved);
            let loaded = decoded.load(&mut NoImages, &mut 0).unwrap();
            assert_eq!(loaded.collision_vertices, None);
            assert_eq!(loaded.children[&767].collision_vertices, Some(vertices));
            assert_eq!(loaded.children[&767].draw, parent.children[&767].draw);
        }
    }

    #[test]
    fn anonymous_capture_keeps_live_pixels_but_load_rebuilds_black() {
        for alpha in [false, true] {
            let original = ObjectImage {
                name: Vec::new(),
                pixels: crate::raster::RgbaImage::new(2, 1, vec![10, 20, 30, 40, 50, 60, 70, 80])
                    .unwrap(),
                alpha,
                filter: Some([2, 0x804020, 99]),
                surface: false,
            };
            let before = original.clone();
            let saved = SavedImage::from_image(&original).unwrap();
            let bytes = saved.encode();
            assert_eq!(bytes.len(), 54);
            assert_eq!(original, before);
            assert_eq!(
                SavedImage::decode_with_layout(&bytes, SnapshotLayout::TextV3).unwrap(),
                saved
            );
            let mut budget = 8;
            let loaded = saved.load(&mut NoImages, &mut budget).unwrap();
            assert_eq!(budget, 0);
            assert!(loaded.name.is_empty() && !loaded.surface && loaded.alpha);
            assert_eq!(loaded.filter, None);
            assert_eq!(loaded.pixels.pixels(), [0, 0, 0, 255, 0, 0, 0, 255]);
            let mut small = 7;
            assert!(saved.load(&mut NoImages, &mut small).is_err());
            assert_eq!(small, 7);
        }
    }

    #[test]
    fn capture_during_replacement_keeps_live_transition_and_loads_full_fade() {
        for elapsed in [0, 120, 999] {
            let mut draw = DrawState::default();
            draw.words[21] = elapsed * 256 / 1000;
            let mut current = SceneObject {
                draw: Some(draw),
                ..SceneObject::default()
            };
            // Retained anonymous images and the transition clock do not belong
            // to the native record.
            let retained = SceneObject {
                image: Some(ObjectImage {
                    name: Vec::new(),
                    pixels: crate::raster::RgbaImage::new(1, 1, vec![0; 4]).unwrap(),
                    alpha: true,
                    filter: None,
                    surface: false,
                }),
                ..SceneObject::default()
            };
            current.change = Some(Box::new(ImageChange {
                previous: Box::new(retained.clone()),
                common: Box::new(retained),
                duration: 1000,
                elapsed,
            }));
            // The same rule must work recursively for attached children.
            let mut parent = SceneObject::default();
            parent.flags |= 0x8000_0000;
            parent.children.insert(2, current.clone());
            let before = parent.clone();
            let saved = SavedScene::from_object(&parent).unwrap();
            assert_eq!(parent, before);
            assert_eq!(saved.children[&2].draw, current.draw);
            let decoded = SavedScene::decode(&saved.encode().unwrap(), 0, &mut 0).unwrap();
            assert_eq!(decoded, saved);
            let mut budget = MAX_PIXELS;
            let loaded = decoded.load(&mut NoImages, &mut budget).unwrap();
            let child = &loaded.children[&2];
            assert!(child.change.is_none());
            assert_eq!(child.draw.as_ref().unwrap().words[21], 256);
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], FormatError> {
        let end = self
            .at
            .checked_add(count)
            .filter(|&n| n <= self.bytes.len())
            .ok_or(FormatError("truncated snapshot visual record"))?;
        let bytes = &self.bytes[self.at..end];
        self.at = end;
        Ok(bytes)
    }
    fn u16(&mut self) -> Result<u16, FormatError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, FormatError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn words<const N: usize>(&mut self) -> Result<[u32; N], FormatError> {
        let mut out = [0; N];
        for n in &mut out {
            *n = self.u32()?;
        }
        Ok(out)
    }
    fn string(&mut self, limit: usize) -> Result<&'a [u8], FormatError> {
        let len = self.bytes[self.at..]
            .iter()
            .take(limit)
            .position(|&b| b == 0)
            .ok_or(FormatError(
                "snapshot visual string exceeds bound or lacks NUL",
            ))?;
        let out = self.take(len)?;
        self.take(1)?;
        Ok(out)
    }
    fn peek(&self) -> Option<u16> {
        self.bytes
            .get(self.at..self.at + 2)
            .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
    }
    fn chunk(&mut self) -> Result<&'a [u8], FormatError> {
        let len = self.u32()?;
        self.take(len as usize)
    }
    fn finish(&self) -> Result<(), FormatError> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(FormatError("trailing snapshot visual bytes"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedImage {
    /// Empty for kind-0 anonymous images: reload creates opaque black pixels,
    /// rather than saving a capture or thumbnail's original memory contents.
    pub name: Vec<u8>,
    pub size: [u32; 2],
    pub filter: Option<[u32; 3]>,
    /// +1204..+1212 while +1200 is clear: earlier parameters the loader ignores.
    pub disabled_filter: [u32; 3],
    /// Kind 3 surface (`ObjectImage::surface`): `Some(alpha flag +1232)`, no file name.
    /// The reader 0x140035590 rebuilds it blank with 0x140034f60 from the saved size.
    pub surface: Option<bool>,
}

/// Appends `u16 tag, u32 length, body`, the chunk framing inside visual records.
pub(crate) fn chunk(out: &mut Vec<u8>, tag: u16, body: &[u8]) {
    out.extend(tag.to_le_bytes());
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
}
fn words(out: &mut Vec<u8>, words: &[u32]) {
    for word in words {
        out.extend(word.to_le_bytes());
    }
}

impl SavedImage {
    fn encode_with_layout(&self, layout: SnapshotLayout) -> Result<Vec<u8>, FormatError> {
        let mut bytes = self.encode();
        if layout == SnapshotLayout::TextV1SceneV1 {
            if self.surface == Some(true) {
                return Err(FormatError(
                    "snapshot image v2 surfaces are not implemented",
                ));
            }
            bytes[..4].copy_from_slice(&2u32.to_le_bytes());
            bytes.drain(36..40);
        }
        Ok(bytes)
    }
    /// Image writer 0x140030030 for file, anonymous kind-0 and surface branches.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let (enabled, params) = match self.filter {
            Some(params) => (1, params),
            None => (0, self.disabled_filter),
        };
        let (kind, alpha) = match self.surface {
            Some(alpha) => (3, u32::from(alpha)),
            None => (0, 0),
        };
        let mut out = Vec::new();
        words(&mut out, &[3, 0, 0, kind, 0, enabled]);
        words(&mut out, &params);
        words(&mut out, &[alpha]);
        out.extend(&self.name);
        out.extend([0, 0]);
        words(&mut out, &[0, self.size[0], self.size[1]]);
        out
    }
    pub(crate) fn from_image(image: &ObjectImage) -> Result<Self, FormatError> {
        if image.surface && image.filter.is_some() {
            return Err(FormatError(
                "saving a filtered surface image is not implemented",
            ));
        }
        Ok(Self {
            name: if image.surface {
                Vec::new()
            } else {
                image.name.clone()
            },
            size: [image.pixels.width(), image.pixels.height()],
            filter: image.filter,
            disabled_filter: [0; 3],
            surface: image.surface.then_some(image.alpha),
        })
    }
    fn decode_with_layout(bytes: &[u8], layout: SnapshotLayout) -> Result<Self, FormatError> {
        let text_v1_scene_v1 = layout == SnapshotLayout::TextV1SceneV1;
        let mut r = Reader::new(bytes);
        let h = if text_v1_scene_v1 {
            let v2_header = r.words::<9>()?;
            let mut h = [0; 10];
            h[..9].copy_from_slice(&v2_header);
            h
        } else {
            r.words::<10>()?
        };
        if h[0] != if text_v1_scene_v1 { 2 } else { 3 } {
            return Err(FormatError("unsupported snapshot image version"));
        }
        // Kind 3 is a blank surface with its alpha flag; the loader ignores its filter.
        let surface = h[3] == 3;
        if h[1..3].iter().chain(&h[4..5]).any(|&v| v != 0)
            || !(h[3] == 0 || surface && h[5..9] == [0; 4] && h[9] <= 1)
            || !surface && h[9] != 0
        {
            return Err(FormatError(
                "unsupported snapshot image kind, flags or subobject",
            ));
        }
        if h[5] > 1 {
            return Err(FormatError("unsupported snapshot image filter flag"));
        }
        let filter = (h[5] != 0).then_some([h[6], h[7], h[8]]);
        if filter
            .is_some_and(|p| !(matches!(p[0], 2 | 17 | 256 | 512) || text_v1_scene_v1 && p[0] == 1))
        {
            return Err(FormatError("unsupported snapshot image filter"));
        }
        let name = r.string(64)?.to_vec();
        if surface && !name.is_empty() || !r.string(32)?.is_empty() {
            return Err(FormatError(
                "snapshot named surface or compound image is not implemented",
            ));
        }
        let tail = r.words::<3>()?;
        r.finish()?;
        if tail[0] != 0 {
            return Err(FormatError(
                "snapshot alternate image resource manager is not implemented",
            ));
        }
        if tail[1] == 0
            || tail[2] == 0
            || u64::from(tail[1]) * u64::from(tail[2]) > (MAX_PIXELS / 4) as u64
        {
            return Err(FormatError(
                "snapshot image dimensions exceed preview budget",
            ));
        }
        Ok(Self {
            name,
            size: [tail[1], tail[2]],
            filter,
            disabled_filter: if filter.is_some() {
                [0; 3]
            } else {
                [h[6], h[7], h[8]]
            },
            surface: surface.then_some(h[9] != 0),
        })
    }
    fn load(
        &self,
        source: &mut dyn ImageSource,
        budget: &mut usize,
    ) -> Result<ObjectImage, Box<dyn Error>> {
        // Check the saved dimensions before allocating decoded pixels. The actual
        // resource must match them below; image::load also has its own codec bound.
        let required = u64::from(self.size[0]) * u64::from(self.size[1]) * 4;
        if required > *budget as u64 {
            return Err(FormatError("snapshot preview exceeds decoded image budget").into());
        }
        if self.surface.is_some() || self.name.is_empty() {
            // Kind 3 creates a blank surface; the movie record supplies its frame.
            // Kind 0 with an empty name calls 0x140032b10(..., 1), rebuilding
            // opaque black CPU pixels. Native snapshots never save these pixels.
            *budget -= required as usize;
            let mut pixels = vec![0; required as usize];
            if self.surface.is_none() {
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel[3] = 255;
                }
            }
            return Ok(ObjectImage {
                filter: None,
                name: Vec::new(),
                pixels: crate::raster::RgbaImage::new(self.size[0], self.size[1], pixels)?,
                alpha: self.surface.unwrap_or(true),
                surface: self.surface.is_some(),
            });
        }
        let decoded = image::load(source, &self.name)?.ok_or(FormatError(
            "snapshot image is missing or has an unknown codec",
        ))?;
        if self.size != [decoded.pixels.width(), decoded.pixels.height()] {
            return Err(FormatError("snapshot image dimensions disagree with resource").into());
        }
        *budget = budget
            .checked_sub(decoded.pixels.pixels().len())
            .ok_or(FormatError("snapshot preview exceeds decoded image budget"))?;
        let mut image = ObjectImage {
            name: self.name.clone(),
            pixels: decoded.pixels,
            alpha: decoded.alpha,
            filter: None,
            surface: false,
        };
        if let Some(filter) = self.filter {
            image.apply_filter(filter)?;
        }
        Ok(image)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedScene {
    pub shown: bool,
    pub flags: u32,
    pub logical_size: [u32; 2],
    pub image: Option<SavedImage>,
    pub secondary_image: Option<SavedImage>,
    pub draw: Option<DrawState>,
    /// Optional 0x749 vertex stream, including an allocated but empty mesh.
    pub collision_vertices: Option<Vec<[u32; 3]>>,
    /// +6264..+6304; only the inactive effect branch (word 0 == 0) is supported.
    pub effect_words: [u32; 11],
    pub children: BTreeMap<u16, SavedScene>,
}
impl SavedScene {
    pub(super) fn decode(
        bytes: &[u8],
        depth: usize,
        nodes: &mut usize,
    ) -> Result<Self, FormatError> {
        Self::decode_with_layout(bytes, depth, nodes, SnapshotLayout::TextV3)
    }
    pub(super) fn decode_with_layout(
        bytes: &[u8],
        depth: usize,
        nodes: &mut usize,
        layout: SnapshotLayout,
    ) -> Result<Self, FormatError> {
        if depth > MAX_DEPTH || *nodes == MAX_NODES {
            return Err(FormatError(
                "snapshot scene nesting or node budget exceeded",
            ));
        }
        *nodes += 1;
        let mut r = Reader::new(bytes);
        let scene_v4 = layout == SnapshotLayout::TextV3SceneV4;
        let text_v1_scene_v1 = layout == SnapshotLayout::TextV1SceneV1;
        if r.u16()? != 0xffff
            || r.u16()?
                != if text_v1_scene_v1 {
                    1
                } else if layout == SnapshotLayout::TextV1SceneV2 {
                    2
                } else if scene_v4 {
                    4
                } else {
                    3
                }
        {
            return Err(FormatError("unsupported snapshot scene version"));
        }
        let h = if scene_v4 {
            let h = r.words::<5>()?;
            [h[0], h[1], h[2], 0, 0, 0, h[3], h[4]]
        } else if layout.text_version() == 1 {
            let h = r.words::<6>()?;
            [h[0], h[1], h[2], h[3], 0, 0, h[4], h[5]]
        } else {
            r.words::<8>()?
        };
        if h[0] > 1 || h[3..6] != [0; 3] {
            return Err(FormatError("unsupported snapshot scene kind or metadata"));
        }
        let mut image = None;
        let mut secondary_image = None;
        let mut draw = None;
        let mut collision_vertices = None;
        // The native format has a fixed optional-chunk order, followed by an
        // untagged 44-byte effect block; do not scan arbitrary words for tags.
        for tag in [0x740, 0x741, 0x745, 0x749, 0x780] {
            if r.peek() == Some(tag) {
                r.u16()?;
                let bytes = r.chunk()?;
                match tag {
                    0x740 => image = Some(SavedImage::decode_with_layout(bytes, layout)?),
                    0x741 => secondary_image = Some(SavedImage::decode_with_layout(bytes, layout)?),
                    0x749 if layout.text_version() != 1 => {
                        collision_vertices = Some(collision::decode(bytes)?)
                    }
                    0x780 => draw = Some(draw::decode_with_layout(bytes, layout)?),
                    _ => {
                        return Err(FormatError(
                            "snapshot scene special image or effect is not implemented",
                        ));
                    }
                }
            }
        }
        if (h[0] == 1) != draw.is_some() {
            return Err(FormatError("snapshot scene kind disagrees with draw state"));
        }
        let effect_words = if text_v1_scene_v1 {
            [0; 11]
        } else {
            r.words::<11>()?
        };
        if effect_words[0] != 0 {
            return Err(FormatError(
                "snapshot scene image change is not implemented",
            ));
        }
        let mut children = BTreeMap::new();
        while r.peek() == Some(0x700) {
            r.u16()?;
            let slot = r.u16()?;
            let body = r.chunk()?;
            if slot >= layout.child_slots() || children.contains_key(&slot) {
                return Err(FormatError("snapshot child slot is invalid or duplicated"));
            }
            children.insert(
                slot,
                Self::decode_with_layout(body, depth + 1, nodes, layout)?,
            );
        }
        if !children.is_empty() && h[2] & 0x8000_0000 == 0 {
            return Err(FormatError("snapshot children lack inheritance flag"));
        }
        if r.u16()? != 0xf0f0 {
            return Err(FormatError("invalid snapshot scene terminator"));
        }
        r.finish()?;
        Ok(Self {
            shown: h[1] != 0,
            flags: h[2],
            logical_size: [h[6], h[7]],
            image,
            secondary_image,
            draw,
            collision_vertices,
            effect_words,
            children,
        })
    }
    /// Scene writer 0x14003a4a0 for the supported subset (see `decode`).
    #[cfg(test)]
    pub(crate) fn encode(&self) -> Result<Vec<u8>, FormatError> {
        self.encode_with_layout(SnapshotLayout::TextV3)
    }
    pub(crate) fn encode_with_layout(
        &self,
        layout: SnapshotLayout,
    ) -> Result<Vec<u8>, FormatError> {
        let mut out = 0xffffu16.to_le_bytes().to_vec();
        let scene_v4 = layout == SnapshotLayout::TextV3SceneV4;
        let text_v1_scene_v1 = layout == SnapshotLayout::TextV1SceneV1;
        if layout == SnapshotLayout::TextV1SceneV2 && self.collision_vertices.is_some() {
            return Err(FormatError("scene v2 has no collision mesh representation"));
        }
        if text_v1_scene_v1 && (self.collision_vertices.is_some() || self.effect_words != [0; 11]) {
            return Err(FormatError(
                "scene v1 has no collision or image-change representation",
            ));
        }
        out.extend(
            if text_v1_scene_v1 {
                1u16
            } else if layout == SnapshotLayout::TextV1SceneV2 {
                2u16
            } else if scene_v4 {
                4u16
            } else {
                3u16
            }
            .to_le_bytes(),
        );
        let header = [
            u32::from(self.draw.is_some()),
            u32::from(self.shown),
            self.flags,
            0,
            0,
            0,
            self.logical_size[0],
            self.logical_size[1],
        ];
        if scene_v4 {
            words(&mut out, &header[..3]);
            words(&mut out, &header[6..]);
        } else if layout.text_version() == 1 {
            words(&mut out, &header[..4]);
            words(&mut out, &header[6..]);
        } else {
            words(&mut out, &header);
        }
        if let Some(image) = &self.image {
            chunk(&mut out, 0x740, &image.encode_with_layout(layout)?);
        }
        if let Some(image) = &self.secondary_image {
            chunk(&mut out, 0x741, &image.encode_with_layout(layout)?);
        }
        if let Some(vertices) = &self.collision_vertices {
            chunk(&mut out, 0x749, &collision::encode(vertices)?);
        }
        if let Some(draw) = &self.draw {
            chunk(&mut out, 0x780, &draw::encode_with_layout(draw, layout)?);
        }
        if !text_v1_scene_v1 {
            words(&mut out, &self.effect_words);
        }
        for (&slot, child) in &self.children {
            if slot >= layout.child_slots() {
                return Err(FormatError("snapshot child slot is invalid or duplicated"));
            }
            out.extend(0x700u16.to_le_bytes());
            out.extend(slot.to_le_bytes());
            let body = child.encode_with_layout(layout)?;
            out.extend((body.len() as u32).to_le_bytes());
            out.extend(body);
        }
        out.extend(0xf0f0u16.to_le_bytes());
        Ok(out)
    }
    /// Capture side: the record of a live object, rejecting what the writer here cannot
    /// express. Disposal marks are not saved, as in the original.
    pub(crate) fn from_object(object: &SceneObject) -> Result<Self, FormatError> {
        if let Some(vertices) = &object.collision_vertices {
            collision::validate(vertices)?;
        }
        // 0x14003a4a0 writes the current image/draw, not +14504..+14532
        // (replacement flag, retained objects and clock). Loading resets the
        // draw fade to 256; capturing must not cancel the live replacement.
        let children = object
            .children
            .iter()
            .map(|(&slot, child)| Ok((slot, Self::from_object(child)?)))
            .collect::<Result<_, FormatError>>()?;
        Ok(Self {
            shown: object.shown,
            flags: object.flags,
            logical_size: object.logical_size,
            image: object
                .image
                .as_ref()
                .map(SavedImage::from_image)
                .transpose()?,
            secondary_image: object
                .secondary_image
                .as_ref()
                .map(SavedImage::from_image)
                .transpose()?,
            draw: object.draw.clone(),
            collision_vertices: object.collision_vertices.clone(),
            effect_words: [0; 11],
            children,
        })
    }
    pub(crate) fn load(
        &self,
        source: &mut dyn ImageSource,
        budget: &mut usize,
    ) -> Result<SceneObject, Box<dyn Error>> {
        let mut object = SceneObject {
            shown: self.shown,
            flags: self.flags,
            logical_size: self.logical_size,
            draw: self.draw.clone(),
            collision_vertices: self.collision_vertices.clone(),
            ..SceneObject::default()
        };
        if let Some(draw) = &mut object.draw {
            draw::reset_transients(draw);
        }
        object.image = self
            .image
            .as_ref()
            .map(|i| i.load(source, budget))
            .transpose()?;
        object.secondary_image = self
            .secondary_image
            .as_ref()
            .map(|i| i.load(source, budget))
            .transpose()?;
        for (&slot, child) in &self.children {
            object.children.insert(slot, child.load(source, budget)?);
        }
        Ok(object)
    }
}

/// Parsed records retain window words whose behavior is not yet modeled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedWindow {
    pub(crate) state: MessageWindow,
    pub logical_size: [u32; 2],
    pub button_words: [u32; 32],
    pub switch_state: u32,
    pub scene: SavedScene,
}

impl SavedWindow {
    pub(crate) fn from_window(
        window: &MessageWindow,
        logical_size: [u32; 2],
    ) -> Result<Self, FormatError> {
        Ok(Self {
            state: MessageWindow {
                object: SceneObject::default(),
                ..window.clone()
            },
            logical_size,
            button_words: window.buttons.map(|b| b.state_word),
            switch_state: window.switch_state,
            scene: SavedScene::from_object(&window.object)?,
        })
    }
    /// The loader's window (0x14005d0c0): saved fields, a fresh pressed-button cache and the
    /// scene object rebuilt from its record.
    pub(crate) fn load(
        &self,
        source: &mut dyn ImageSource,
        budget: &mut usize,
    ) -> Result<MessageWindow, Box<dyn Error>> {
        Ok(MessageWindow {
            object: self.scene.load(source, budget)?,
            pressed_button: -1,
            ..self.state.clone()
        })
    }
}

/// Decoded-pixel budget for rebuilding saved objects, shared with the preview.
pub(crate) const RESTORE_PIXELS: usize = MAX_PIXELS;

/// Typed visual subset, not a full-engine restore image. Public fields are read-only
/// through accessors so callers cannot bypass parse limits before resource loading.
#[derive(Debug)]
pub struct VisualSnapshot {
    clear: ClearState,
    texts: [Option<TextObject>; 12],
    windows: [Option<SavedWindow>; 8],
    objects: BTreeMap<u8, SavedScene>,
    pub uninterpreted: Vec<SnapshotRecord>,
}
/// Fresh standalone hosts for rendering and advancing supported object animations.
/// Timers, cameras, VM, audio, auxiliary text and global effects are not installed.
/// Window button_words/switch_state remain in VisualSnapshot, not these hosts;
/// this API does not claim to resume window interaction or execute a loaded game.
#[derive(Debug)]
pub struct VisualPreview {
    pub scene: SceneHost,
    pub messages: MessageHost,
}
impl VisualSnapshot {
    pub fn clear(&self) -> ClearState {
        self.clear
    }
    pub fn texts(&self) -> &[Option<TextObject>; 12] {
        &self.texts
    }
    pub fn windows(&self) -> &[Option<SavedWindow>; 8] {
        &self.windows
    }
    pub fn objects(&self) -> &BTreeMap<u8, SavedScene> {
        &self.objects
    }
    pub fn decode(save: &Csv2Save) -> Result<Self, FormatError> {
        Self::decode_records(&save.snapshot)
    }
    pub fn decode_records(bytes: &[u8]) -> Result<Self, FormatError> {
        let records = parse_records(bytes)?;
        let scalar = |tag| {
            let mut r = Reader::new(unique_body(bytes, &records, tag)?);
            let n = r.u32()?;
            r.finish()?;
            Ok::<_, FormatError>(n)
        };
        let clear = ClearState {
            color: scalar(0x200)?,
            depth: scalar(0x203)? != 0,
        };
        let mut out = Self {
            clear,
            texts: std::array::from_fn(|_| None),
            windows: std::array::from_fn(|_| None),
            objects: BTreeMap::new(),
            uninterpreted: Vec::new(),
        };
        let mut nodes = 0;
        for record in records {
            let body = &bytes[record.body.clone()];
            let index = usize::from(record.index.unwrap_or(0));
            match record.tag {
                0x200 | 0x203 => {}
                0x380 => {
                    if index >= 12 || out.texts[index].is_some() {
                        return Err(FormatError("invalid or duplicate snapshot text slot"));
                    }
                    out.texts[index] = Some(text::decode(body)?);
                }
                0x400 => {
                    if index >= 8 || out.windows[index].is_some() {
                        return Err(FormatError("invalid or duplicate snapshot window slot"));
                    }
                    out.windows[index] = Some(window::decode(body, &mut nodes)?);
                }
                0x700 => {
                    if index >= 256 || out.objects.contains_key(&(index as u8)) {
                        return Err(FormatError("invalid or duplicate snapshot scene slot"));
                    }
                    out.objects
                        .insert(index as u8, SavedScene::decode(body, 0, &mut nodes)?);
                }
                _ => out.uninterpreted.push(record),
            }
        }
        for window in out.windows.iter().flatten() {
            let text = out.texts[window.state.text]
                .as_ref()
                .ok_or(FormatError("snapshot window references absent text"))?;
            if text.logical_size.map(|v| v as u32) != window.logical_size {
                return Err(FormatError(
                    "snapshot window and text logical sizes disagree",
                ));
            }
        }
        Ok(out)
    }
    /// Reads named resources through the caller's existing read-only image source.
    /// Returns hosts only after every resource succeeds; no existing engine is changed.
    pub fn preview(&self, source: &mut dyn ImageSource) -> Result<VisualPreview, Box<dyn Error>> {
        let mut budget = MAX_PIXELS;
        let mut objects = BTreeMap::new();
        for (&slot, saved) in &self.objects {
            objects.insert(slot, saved.load(source, &mut budget)?);
        }
        let mut texts = self.texts.clone();
        // The text loader 0x14005c8f0 regenerates glyphs from the area origin.
        for text in texts.iter_mut().flatten() {
            text.cursor = [text.area[0], text.area[1]];
        }
        let mut windows = std::array::from_fn(|_| None);
        for (slot, saved) in self.windows.iter().enumerate() {
            if let Some(saved) = saved {
                let mut window = saved.state.clone();
                window.object = saved.scene.load(source, &mut budget)?;
                windows[slot] = Some(window);
            }
        }
        Ok(VisualPreview {
            scene: SceneHost::snapshot_preview(self.clear, objects),
            messages: MessageHost::snapshot_preview(texts, windows),
        })
    }
}
