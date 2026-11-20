//! Editable images for PB2A, PB3B and MSK0 resources.
use crate::Result;
use cmvs_core::{
    MAX_PAYLOAD,
    format::{
        msk, pb2,
        pb3::{self, EncodeDepth},
    },
    render::raster::RgbaImage,
};
use std::io::{BufRead, Read, Seek, SeekFrom, Write};

fn check_dimensions(width: u32, height: u32) -> Result<()> {
    if !(1..=65535).contains(&width)
        || !(1..=65535).contains(&height)
        || u64::from(width) * u64::from(height) * 4 > MAX_PAYLOAD as u64
    {
        return Err("image exceeds dimension or pixel limit".into());
    }
    Ok(())
}

/// Read an editable PNG or BMP selected by its signature.
pub fn read_image<R: BufRead + Seek>(mut input: R) -> Result<RgbaImage> {
    let mut magic = [0; 2];
    input.read_exact(&mut magic)?;
    input.rewind()?;
    if &magic == b"BM" {
        return decode_bmp(input);
    }
    let mut decoder = png::Decoder::new(input);
    // Expand palettes, low-bit grayscale and tRNS before converting to straight RGBA.
    decoder.set_transformations(png::Transformations::EXPAND);
    let reader = decoder.read_info()?;
    if reader.info().animation_control.is_some() {
        return Err("expected a static PNG".into());
    }
    png_pixels(reader)
}

/// Decode one frame with the caller's PNG transformations and input constraints.
pub fn png_pixels<R: BufRead + Seek>(mut reader: png::Reader<R>) -> Result<RgbaImage> {
    use png::ColorType::*;
    let (width, height) = reader.info().size();
    check_dimensions(width, height)?;
    if reader.output_color_type().1 != png::BitDepth::Eight {
        return Err("expected 8-bit PNG pixel channels".into());
    }
    let mut bytes = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("PNG exceeds size limit")?
    ];
    let frame = reader.next_frame(&mut bytes)?;
    bytes.truncate(frame.buffer_size());
    reader.finish()?;
    let rgba = match frame.color_type {
        Rgba => bytes,
        Rgb => bytes
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        Grayscale => bytes.into_iter().flat_map(|v| [v, v, v, 255]).collect(),
        GrayscaleAlpha => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        Indexed => return Err("PNG palette did not expand".into()),
    };
    Ok(RgbaImage::new(width, height, rgba)?)
}

/// Read a 32-bit BI_RGB bitmap, preserving the fourth byte as straight alpha.
pub fn decode_bmp<R: Read + Seek>(mut input: R) -> Result<RgbaImage> {
    let file_size = input.seek(SeekFrom::End(0))?;
    input.rewind()?;
    let mut header = [0; 54];
    input.read_exact(&mut header)?;
    if &header[..2] != b"BM" {
        return Err("expected a BMP file header".into());
    }
    let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    let offset = u64::from(word(10));
    let dib = word(14);
    if u64::from(word(2)) != file_size
        || !matches!(dib, 40 | 108 | 124)
        || offset < 14 + u64::from(dib)
        || header[26..30] != [1, 0, 32, 0]
        || word(30) != 0
    {
        return Err("expected a 32-bit BI_RGB bitmap".into());
    }
    let width = word(18);
    let signed_height = word(22) as i32;
    let height = signed_height.unsigned_abs();
    check_dimensions(width, height)?;
    let stride = width as usize * 4;
    let size = stride * height as usize;
    if offset + size as u64 > file_size {
        return Err("truncated BMP pixels".into());
    }
    input.seek(SeekFrom::Start(offset))?;
    let mut rgba = vec![0; size];
    for y in 0..height as usize {
        let dest_y = if signed_height < 0 {
            y
        } else {
            height as usize - 1 - y
        };
        let row = &mut rgba[dest_y * stride..][..stride];
        input.read_exact(row)?;
        for pixel in row.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    }
    Ok(RgbaImage::new(width, height, rgba)?)
}

pub fn encode_pb2(img: &RgbaImage, kind: pb2::StaticType, depth: EncodeDepth) -> Result<Vec<u8>> {
    let raw = pb2::encode(img, kind, depth)?;
    if pb2::decode(&raw)? != *img {
        return Err("PB2A pixel roundtrip differs".into());
    }
    Ok(raw)
}

pub fn pb2_to_png(raw: &[u8], out: impl Write) -> Result<()> {
    let h = pb2::Header::parse(raw)?;
    if !matches!(h.kind, 1..=4) {
        return Err("single-image export requires a static PB2 type (1 through 4)".into());
    }
    let img = pb2::decode(raw)?;
    write_png(
        out,
        img.width(),
        img.height(),
        png::ColorType::Rgba,
        img.pixels(),
    )
}

pub fn encode_pb3(image: &RgbaImage, depth: EncodeDepth) -> Result<Vec<u8>> {
    let bytes = pb3::encode(image, depth)?;
    if pb3::decode(&bytes)? != *image {
        return Err("PB3B pixel roundtrip differs".into());
    }
    Ok(bytes)
}

/// Encode a static 8-bit grayscale PNG without palette or color conversions.
pub fn png_to_msk(input: impl BufRead + Seek) -> Result<Vec<u8>> {
    let mut reader = png::Decoder::new(input).read_info()?;
    let info = reader.info();
    if info.animation_control.is_some()
        || info.trns.is_some()
        || (info.color_type, info.bit_depth) != (png::ColorType::Grayscale, png::BitDepth::Eight)
    {
        return Err("MSK0 requires a static, opaque 8-bit grayscale PNG".into());
    }
    let (width, height) = info.size();
    let mut samples = vec![0; msk::pixel_count(width, height)?];
    reader.next_frame(&mut samples)?;
    reader.finish()?;
    Ok(msk::encode(width, height, &samples)?)
}

pub fn msk_to_png(raw: &[u8], out: impl Write) -> Result<()> {
    let h = msk::Header::parse(raw)?;
    let start = h.data_offset as usize;
    let count = msk::pixel_count(h.width, h.height)?;
    write_png(
        out,
        h.width,
        h.height,
        png::ColorType::Grayscale,
        &raw[start..start + count],
    )
}

pub(crate) fn write_rgba(img: &RgbaImage, out: impl Write) -> Result<()> {
    write_png(
        out,
        img.width(),
        img.height(),
        png::ColorType::Rgba,
        img.pixels(),
    )
}

fn write_png(
    out: impl Write,
    width: u32,
    height: u32,
    color: png::ColorType,
    pixels: &[u8],
) -> Result<()> {
    let mut encoder = png::Encoder::new(out, width, height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(pixels)?;
    writer.finish()?;
    Ok(())
}
