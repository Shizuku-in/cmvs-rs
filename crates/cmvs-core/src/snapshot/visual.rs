//! Bounded, version-specific visual records. This is an offline component preview,
//! never a replacement for the engine snapshot loader. Evidence: snapshot-format.md.
use super::{SnapshotRecord, parse_records, unique_body};
use crate::{
    FormatError,
    image::{self, ImageSource},
    message::{MessageHost, MessageWindow, TextObject},
    save::Csv2Save,
    scene::{ClearState, DrawState, ObjectImage, SceneHost, SceneObject},
};
use std::{collections::BTreeMap, error::Error};
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
            assert_eq!(SavedImage::decode(&bytes).unwrap(), saved);
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
    fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(bytes);
        let h = r.words::<10>()?;
        if h[0] != 3 {
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
        if filter.is_some_and(|p| !matches!(p[0], 2 | 17)) {
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
        if depth > MAX_DEPTH || *nodes == MAX_NODES {
            return Err(FormatError(
                "snapshot scene nesting or node budget exceeded",
            ));
        }
        *nodes += 1;
        let mut r = Reader::new(bytes);
        if r.u16()? != 0xffff || r.u16()? != 3 {
            return Err(FormatError("unsupported snapshot scene version"));
        }
        let h = r.words::<8>()?;
        if h[0] > 1 || h[3..6] != [0; 3] {
            return Err(FormatError("unsupported snapshot scene kind or metadata"));
        }
        let mut image = None;
        let mut secondary_image = None;
        let mut draw = None;
        // The native format has a fixed optional-chunk order, followed by an
        // untagged 44-byte effect block; do not scan arbitrary words for tags.
        for tag in [0x740, 0x741, 0x745, 0x749, 0x780] {
            if r.peek() == Some(tag) {
                r.u16()?;
                let bytes = r.chunk()?;
                match tag {
                    0x740 => image = Some(SavedImage::decode(bytes)?),
                    0x741 => secondary_image = Some(SavedImage::decode(bytes)?),
                    0x780 => draw = Some(draw::decode(bytes)?),
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
        let effect_words = r.words::<11>()?;
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
            if slot >= 768 || children.contains_key(&slot) {
                return Err(FormatError("snapshot child slot is invalid or duplicated"));
            }
            children.insert(slot, Self::decode(body, depth + 1, nodes)?);
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
            effect_words,
            children,
        })
    }
    /// Scene writer 0x14003a4a0 for the supported subset (see `decode`).
    pub(crate) fn encode(&self) -> Result<Vec<u8>, FormatError> {
        let mut out = 0xffffu16.to_le_bytes().to_vec();
        out.extend(3u16.to_le_bytes());
        words(
            &mut out,
            &[
                u32::from(self.draw.is_some()),
                u32::from(self.shown),
                self.flags,
                0,
                0,
                0,
                self.logical_size[0],
                self.logical_size[1],
            ],
        );
        if let Some(image) = &self.image {
            chunk(&mut out, 0x740, &image.encode());
        }
        if let Some(image) = &self.secondary_image {
            chunk(&mut out, 0x741, &image.encode());
        }
        if let Some(draw) = &self.draw {
            chunk(&mut out, 0x780, &draw::encode(draw)?);
        }
        words(&mut out, &self.effect_words);
        for (&slot, child) in &self.children {
            out.extend(0x700u16.to_le_bytes());
            out.extend(slot.to_le_bytes());
            let body = child.encode()?;
            out.extend((body.len() as u32).to_le_bytes());
            out.extend(body);
        }
        out.extend(0xf0f0u16.to_le_bytes());
        Ok(out)
    }
    /// Capture side: the record of a live object, rejecting what the writer here cannot
    /// express. Disposal marks are not saved, as in the original.
    pub(crate) fn from_object(object: &SceneObject) -> Result<Self, FormatError> {
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
