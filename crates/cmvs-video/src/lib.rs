//! MGV1's Ogg/Theora stream, with bounded I/O and explicit nominal-frame mapping.
//! Pure Rust and window-free, so every host can install [`TheoraFactory`], or
//! [`PrefetchTheoraFactory`] to decode the next packet on a worker thread during playback.
mod prefetch;
mod theora;
use cmvs_core::{
    FormatError, MAX_PAYLOAD,
    mgv::{self, MgvIndex, ReadSeek},
    raster::RgbaImage,
};
use ogg::PacketReader;
pub use prefetch::PrefetchTheoraFactory;
use std::{
    error::Error,
    fmt,
    io::{self, Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use theora::{Decoder as FrameDecoder, Ident as TheoraIdentHeader};

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

/// Bytes the next Ogg packet may still read; shared with the packet reader's section.
#[derive(Clone)]
struct Budget(Arc<AtomicUsize>);
impl Budget {
    fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
    fn set(&self, value: usize) {
        self.0.store(value, Ordering::Relaxed);
    }
}

struct VideoSection {
    reader: Box<dyn ReadSeek>,
    start: u64,
    size: u64,
    position: u64,
    budget: Budget,
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
    budget: Budget,
    decoder: FrameDecoder,
    ident: TheoraIdentHeader,
    next_packet: u32,
    serial: u32,
    /// The last two converted packets: the frame on screen and, after a prefetch, the
    /// next one. Duplicate nominal frames reuse the on-screen packet without a replay.
    recent: [Option<(u32, RgbaImage)>; 2],
    failed: bool,
}
impl fmt::Debug for TheoraStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TheoraStream")
            .field("next_packet", &self.next_packet)
            .finish_non_exhaustive()
    }
}
fn codec<T>(result: Result<T, FormatError>) -> Result<T, Box<dyn Error>> {
    result.map_err(|e| format!("Theora decode: {e:?}").into())
}
impl TheoraStream {
    pub fn open(mut reader: Box<dyn ReadSeek>, index: &MgvIndex) -> Result<Self, Box<dyn Error>> {
        if index.frames.is_empty() {
            return Err(FormatError("MGV frame map is empty").into());
        }
        reader.seek(SeekFrom::Start(index.video_offset))?;
        let budget = Budget(Arc::new(AtomicUsize::new(MAX_PAYLOAD)));
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
            recent: [None, None],
            failed: false,
        })
    }
    fn headers(
        packets: &mut PacketReader<VideoSection>,
        budget: &Budget,
        index: &MgvIndex,
    ) -> Result<(TheoraIdentHeader, FrameDecoder, u32), Box<dyn Error>> {
        budget.set(MAX_PAYLOAD);
        let packet = packets.read_packet_expected()?;
        let serial = packet.stream_serial();
        let ident = codec(TheoraIdentHeader::parse(&packet.data))?;
        let container = (u32::from(index.width), u32::from(index.height));
        if (container != (ident.picw, ident.pich)
            && container != (ident.width as u32, ident.height as u32))
            || u64::from(ident.frn) * u64::from(index.rate_time)
                != u64::from(index.rate) * u64::from(ident.frd)
            || ident.width as u64 * ident.height as u64 * 4 > MAX_PAYLOAD as u64
        {
            return Err(FormatError("MGV/Theora geometry, rate or pixel format mismatch").into());
        }
        budget.set(MAX_PAYLOAD);
        let comment = packets.read_packet_expected()?;
        codec(theora::comments(&comment.data))?;
        budget.set(MAX_PAYLOAD);
        let setup = packets.read_packet_expected()?;
        if comment.stream_serial() != serial || setup.stream_serial() != serial {
            return Err(FormatError("MGV Theora headers change Ogg serial").into());
        }
        let decoder = codec(FrameDecoder::new(&ident, &setup.data))?;
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
        self.recent = [None, None];
        self.failed = false;
        Ok(())
    }
    fn recent(&self, packet: u32) -> Option<&RgbaImage> {
        self.recent
            .iter()
            .flatten()
            .find(|(p, _)| *p == packet)
            .map(|(_, image)| image)
    }
    /// Decodes and converts the packet after the last decoded one, so a later
    /// [`mgv::Decoder::frame`] for it returns at once. A failure surfaces on that call.
    pub fn prefetch(&mut self) {
        if !self.failed && self.next_packet < self.index.packet_count() {
            let _ = self.decode_through(self.next_packet);
        }
    }
    /// Decode through an Ogg packet, including all inter-frame dependencies.
    fn decode_through(&mut self, target: u32) -> Result<(), Box<dyn Error>> {
        if !self.failed && self.recent(target).is_some() {
            return Ok(());
        }
        if self.failed || target < self.next_packet {
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
                let image = to_rgba(&self.ident, frame, &self.index)?;
                self.recent.rotate_left(1);
                self.recent[1] = Some((target, image));
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
            .recent(u32::from(target))
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
    frame: &theora::Frame,
    index: &MgvIndex,
) -> Result<RgbaImage, FormatError> {
    // Some MGV headers describe the padded coding surface (e.g. 1920x1088),
    // which the native DLL returns in full, including pixels outside the picture.
    let width = usize::from(index.width);
    let height = usize::from(index.height);
    let picture = (u32::from(index.width), u32::from(index.height)) == (ident.picw, ident.pich);
    let [left, bottom] = if picture {
        [usize::from(ident.picx), usize::from(ident.picy)]
    } else {
        [0, 0]
    };
    let yw = ident.width;
    let cw = ident.width / 2;
    const BOUNDS: &str = "Theora planes smaller than the MGV picture";
    let mut rgba = vec![0; width * height * 4];
    for (row, out) in rgba.chunks_exact_mut(width * 4).enumerate() {
        // Theora rows run bottom-up.
        let y = bottom + height - 1 - row;
        let luma = frame.planes[0]
            .get(y * yw + left..y * yw + left + width)
            .ok_or(FormatError(BOUNDS))?;
        let chroma = (y / 2) * cw + left / 2..(y / 2) * cw + (left + width).div_ceil(2);
        let cb = frame.planes[1]
            .get(chroma.clone())
            .ok_or(FormatError(BOUNDS))?;
        let cr = frame.planes[2].get(chroma).ok_or(FormatError(BOUNDS))?;
        // An odd left edge starts mid-pair: its first pixel takes the first chroma sample.
        let (head, luma, out) = if left % 2 == 1 {
            let (first, out) = out.split_at_mut(4);
            first.copy_from_slice(&ycbcr(luma[0], cb[0], cr[0]));
            (1, &luma[1..], out)
        } else {
            (0, luma, out)
        };
        let (cb, cr) = (&cb[head..], &cr[head..]);
        let pairs = luma.len() / 2;
        for (((y2, out), &u), &v) in luma
            .as_chunks::<2>()
            .0
            .iter()
            .zip(out.as_chunks_mut::<8>().0)
            .zip(cb)
            .zip(cr)
        {
            out[..4].copy_from_slice(&ycbcr(y2[0], u, v));
            out[4..].copy_from_slice(&ycbcr(y2[1], u, v));
        }
        if let Some(&last) = luma.get(pairs * 2) {
            out[pairs * 8..].copy_from_slice(&ycbcr(last, cb[pairs], cr[pairs]));
        }
    }
    RgbaImage::new(u32::from(index.width), u32::from(index.height), rgba)
}

/// BT.601 limited-range conversion; the decoded Y/Cb/Cr samples are retained exactly.
#[inline(always)]
fn ycbcr(y: u8, cb: u8, cr: u8) -> [u8; 4] {
    let luma = 298 * (i32::from(y) - 16);
    let (cb, cr) = (i32::from(cb) - 128, i32::from(cr) - 128);
    let clip = |v: i32| ((v + 128) >> 8).clamp(0, 255) as u8;
    [
        clip(luma + 409 * cr),
        clip(luma - 100 * cb - 208 * cr),
        clip(luma + 516 * cb),
        255,
    ]
}
