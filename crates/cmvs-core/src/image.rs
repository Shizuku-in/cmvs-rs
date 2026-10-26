//! Image-specific lookup and codec dispatch; shared I/O lives in `resources`.
use crate::{
    FormatError, pb3,
    raster::RgbaImage,
    resources::{RegisteredResources, ResourceArchives, ResourceKind, ResourceSource},
};
use std::{collections::BTreeMap, error::Error, path::Path};
/// 0x2011 / 0x1400717c0: image manager has ten registration slots.
pub const IMAGE_ENTRIES: u32 = ResourceKind::Image.slot_count();
pub trait ImageSource {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>>;
}
/// Compatibility image view over the common read-only resource store.
pub struct ImageArchives {
    archives: ResourceArchives,
}
impl ImageArchives {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            archives: ResourceArchives::new(root)?,
        })
    }
    pub fn with_paths<'a>(
        &'a mut self,
        paths: &'a BTreeMap<(u16, u32), Vec<u8>>,
    ) -> Registered<'a> {
        Registered {
            inner: self.archives.with_paths(paths, ResourceKind::Image),
        }
    }
}
pub struct Registered<'a> {
    inner: RegisteredResources<'a>,
}
impl ImageSource for Registered<'_> {
    fn read(&mut self, name: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        self.inner.read(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub pixels: RgbaImage,
    /// Image info +24 (object image +1152): the source has 32 bits per pixel.
    pub alpha: bool,
}

/// 0x140036700 picks the codec by magic; only PB3B type 1 decodes here, other known codecs
/// are explicit errors. Ok(None) is the original quiet failure for an unknown magic or a
/// PB3 type outside 1..8: the load returns 0 without a MessageBox.
pub fn decode(bytes: &[u8]) -> Result<Option<DecodedImage>, FormatError> {
    let unsupported = FormatError("image codec not implemented; only PB3 type 1 is supported");
    let magic = bytes
        .get(..4)
        .ok_or(FormatError("image file shorter than its 4-byte magic"))?;
    match magic {
        b"PB3B" => {
            let header = pb3::Header::parse(bytes)?;
            match header.kind {
                1 => Ok(Some(DecodedImage {
                    pixels: pb3::decode(bytes)?,
                    alpha: header.bpp == 32,
                })),
                2..=8 => Err(unsupported),
                _ => Ok(None),
            }
        }
        [0x89, b'P', b'N', b'G'] | b"MSK0" | b"PB2A" | [b'B', b'M', ..] => Err(unsupported),
        _ => Ok(None),
    }
}
