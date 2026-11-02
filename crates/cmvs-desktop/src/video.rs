//! MGV1's Ogg/Theora stream, with bounded I/O and explicit nominal-frame mapping.
use cmvs_core::{
    FormatError, MAX_PAYLOAD,
    mgv::{self, MgvIndex, ReadSeek},
    raster::RgbaImage,
};
use ogg::PacketReader;
use oxideav_theora::{self as theora, FrameDecoder, TheoraIdentHeader};
use std::{
    cell::Cell,
    error::Error,
    fmt,
    io::{self, Read, Seek, SeekFrom},
    rc::Rc,
};

#[derive(Debug, Default)]
pub struct TheoraFactory;
impl mgv::DecoderFactory for TheoraFactory {
    fn open(
        &self,
        reader: Box<dyn ReadSeek>,
        index: &MgvIndex,
    ) -> Result<Box<dyn mgv::Decoder>, Box<dyn Error>> {
        Ok(Box::new(TheoraStream::open(reader, index)?))
    }
}

struct VideoSection {
    reader: Box<dyn ReadSeek>,
    start: u64,
    size: u64,
    position: u64,
    budget: Rc<Cell<usize>>,
}
impl Read for VideoSection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let size = buf.len().min((self.size - self.position) as usize);
        if size == 0 {
            return Ok(0);
        }
        if self.budget.get() == 0 {
            return Err(io::Error::other("Theora packet exceeds research limit"));
        }
        let size = size.min(self.budget.get());
        let read = self.reader.read(&mut buf[..size])?;
        self.position += read as u64;
        self.budget.set(self.budget.get() - read);
        Ok(read)
    }
}
impl Seek for VideoSection {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        let position = match offset {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.size) + i128::from(n),
        };
        if position < 0 || position > i128::from(self.size) {
            return Err(io::Error::other("seek outside MGV video stream"));
        }
        self.reader
            .seek(SeekFrom::Start(self.start + position as u64))?;
        self.position = position as u64;
        Ok(self.position)
    }
}

pub struct TheoraStream {
    index: MgvIndex,
    packets: PacketReader<VideoSection>,
    budget: Rc<Cell<usize>>,
    decoder: FrameDecoder,
    ident: TheoraIdentHeader,
    next_packet: u32,
    serial: u32,
    cached: Option<RgbaImage>,
    failed: bool,
}
impl fmt::Debug for TheoraStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TheoraStream")
            .field("next_packet", &self.next_packet)
            .finish_non_exhaustive()
    }
}
fn codec<T>(result: Result<T, theora::Error>) -> Result<T, Box<dyn Error>> {
    result.map_err(|e| format!("Theora decode: {e:?}").into())
}
impl TheoraStream {
    pub fn open(mut reader: Box<dyn ReadSeek>, index: &MgvIndex) -> Result<Self, Box<dyn Error>> {
        if index.frames.is_empty() {
            return Err(FormatError("MGV frame map is empty").into());
        }
        reader.seek(SeekFrom::Start(index.video_offset))?;
        let budget = Rc::new(Cell::new(MAX_PAYLOAD));
        let section = VideoSection {
            reader,
            start: index.video_offset,
            size: u64::from(index.video_size),
            position: 0,
            budget: budget.clone(),
        };
        let mut packets = PacketReader::new(section);
        let (ident, decoder, serial) = Self::headers(&mut packets, &budget, index)?;
        Ok(Self {
            index: index.clone(),
            packets,
            budget,
            decoder,
            ident,
            next_packet: 0,
            serial,
            cached: None,
            failed: false,
        })
    }
    fn headers(
        packets: &mut PacketReader<VideoSection>,
        budget: &Cell<usize>,
        index: &MgvIndex,
    ) -> Result<(TheoraIdentHeader, FrameDecoder, u32), Box<dyn Error>> {
        budget.set(MAX_PAYLOAD);
        let packet = packets.read_packet_expected()?;
        let serial = packet.stream_serial();
        let ident = codec(theora::decode_identification_header(&packet.data))?;
        if ident.picw != u32::from(index.width)
            || ident.pich != u32::from(index.height)
            || u64::from(ident.frn) * u64::from(index.rate_time)
                != u64::from(index.rate) * u64::from(ident.frd)
            || ident.pf != theora::PixelFormat::Yuv420
            || u64::from(ident.coded_width()) * u64::from(ident.coded_height()) * 4
                > MAX_PAYLOAD as u64
        {
            return Err(FormatError("MGV/Theora geometry, rate or pixel format mismatch").into());
        }
        budget.set(MAX_PAYLOAD);
        let comment = packets.read_packet_expected()?;
        codec(theora::parse_comment_header(&comment.data))?;
        budget.set(MAX_PAYLOAD);
        let setup = packets.read_packet_expected()?;
        if comment.stream_serial() != serial || setup.stream_serial() != serial {
            return Err(FormatError("MGV Theora headers change Ogg serial").into());
        }
        let decoder = codec(FrameDecoder::new(
            ident.clone(),
            codec(theora::decode_setup_header(&setup.data))?,
        ))?;
        Ok((ident, decoder, serial))
    }
    fn reset(&mut self) -> Result<(), Box<dyn Error>> {
        self.budget.set(MAX_PAYLOAD);
        self.packets.seek_bytes(SeekFrom::Start(0))?;
        let (ident, decoder, serial) = Self::headers(&mut self.packets, &self.budget, &self.index)?;
        self.ident = ident;
        self.decoder = decoder;
        self.serial = serial;
        self.next_packet = 0;
        self.cached = None;
        self.failed = false;
        Ok(())
    }
    /// Decode through an Ogg packet, including all inter-frame dependencies.
    fn decode_through(&mut self, target: u32) -> Result<(), Box<dyn Error>> {
        if self.failed || target + 1 < self.next_packet {
            self.reset()?;
        }
        while self.next_packet <= target {
            self.failed = true;
            self.budget.set(MAX_PAYLOAD);
            let packet = self.packets.read_packet_expected()?;
            if packet.stream_serial() != self.serial {
                return Err(FormatError("MGV video changes Ogg serial").into());
            }
            let frame = codec(self.decoder.decode_frame(&packet.data))?;
            if self.next_packet == target {
                self.cached = Some(to_rgba(&self.ident, &frame.frame)?);
            }
            self.next_packet += 1;
            self.failed = false;
        }
        Ok(())
    }
}
impl mgv::Decoder for TheoraStream {
    fn frame(&mut self, frame: u32) -> Result<RgbaImage, Box<dyn Error>> {
        let target = self
            .index
            .frames
            .get(frame as usize)
            .ok_or(FormatError("MGV nominal frame out of range"))?
            .packet;
        self.decode_through(u32::from(target))?;
        Ok(self
            .cached
            .as_ref()
            .ok_or(FormatError("MGV decoded frame unavailable"))?
            .clone())
    }
    fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        self.decode_through(self.index.packet_count() - 1)?;
        self.failed = true;
        self.budget.set(MAX_PAYLOAD);
        if self.packets.read_packet()?.is_some() {
            return Err(FormatError("MGV video has packets beyond the frame table").into());
        }
        self.failed = false;
        Ok(())
    }
}

fn to_rgba(
    ident: &TheoraIdentHeader,
    frame: &theora::ReconstructedFrame,
) -> Result<RgbaImage, FormatError> {
    let width = ident.picw as usize;
    let height = ident.pich as usize;
    let mut rgba = Vec::with_capacity(width * height * 4);
    let yw = frame.dims_y.width as usize;
    let cw = frame.dims_cb.width as usize;
    for row in 0..height {
        let y = usize::from(ident.picy) + height - 1 - row;
        for column in 0..width {
            let x = usize::from(ident.picx) + column;
            let luma = i32::from(frame.samples_y[y * yw + x]) - 16;
            let cb = i32::from(frame.samples_cb[(y / 2) * cw + x / 2]) - 128;
            let cr = i32::from(frame.samples_cr[(y / 2) * cw + x / 2]) - 128;
            // BT.601 limited-range conversion; the decoded Y/Cb/Cr samples are retained exactly.
            let clip = |v: i32| ((v + 128) >> 8).clamp(0, 255) as u8;
            rgba.extend([
                clip(298 * luma + 409 * cr),
                clip(298 * luma - 100 * cb - 208 * cr),
                clip(298 * luma + 516 * cb),
                255,
            ]);
        }
    }
    RgbaImage::new(ident.picw, ident.pich, rgba)
}
