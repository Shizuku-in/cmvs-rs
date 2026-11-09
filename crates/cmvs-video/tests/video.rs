use cmvs_core::{
    digest::{sha256, to_hex},
    mgv::{Decoder, MgvIndex},
};
use cmvs_video::TheoraStream;
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
struct CountingReader<R> {
    data: R,
    seeks: Arc<AtomicUsize>,
}
impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.data.read(out)
    }
}
impl<R: Seek> Seek for CountingReader<R> {
    fn seek(&mut self, offset: SeekFrom) -> std::io::Result<u64> {
        self.seeks.fetch_add(1, Ordering::Relaxed);
        self.data.seek(offset)
    }
}

fn fixture() -> Vec<u8> {
    include_str!("fixtures/mgv1.hex")
        .trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn synthetic_inter_frames_duplicates_and_backwards_access_are_deterministic() {
    let bytes = fixture();
    let index = MgvIndex::read(&mut Cursor::new(&bytes)).unwrap();
    let mut decoder = TheoraStream::open(Box::new(Cursor::new(bytes)), &index).unwrap();
    let first = decoder.frame(0).unwrap();
    assert_eq!((first.width(), first.height()), (32, 32));
    let second = decoder.frame(1).unwrap();
    assert_ne!(first, second);
    assert_eq!(decoder.frame(2).unwrap(), second);
    assert_ne!(decoder.frame(3).unwrap(), second);
    decoder.finish().unwrap();
    assert_eq!(decoder.frame(0).unwrap(), first);
    assert_eq!(decoder.frame(2).unwrap(), second);
    assert!(decoder.frame(4).is_err());
    // Independently decoded with FFmpeg/libtheora and converted from its top-down YUV.
    assert_eq!(
        to_hex(&sha256(first.pixels())),
        "fe911eee671616f6b45be1128bd5a0ad643b00d746cd33f4e0da3d1fd9edec40"
    );
    assert_eq!(
        to_hex(&sha256(second.pixels())),
        "3d8be55c8a446c5af423f8c6e175dd46f1307bc99b2b05984421083b0228083e"
    );
}
#[test]
fn geometry_and_packet_corruption_fail_without_poisoning_repeatable_errors() {
    let bytes = fixture();
    let mut index = MgvIndex::read(&mut Cursor::new(&bytes)).unwrap();
    index.width += 1;
    assert!(TheoraStream::open(Box::new(Cursor::new(bytes.clone())), &index).is_err());
    let index = MgvIndex::read(&mut Cursor::new(&bytes)).unwrap();
    let mut damaged = bytes.clone();
    *damaged.last_mut().unwrap() ^= 1;
    match TheoraStream::open(Box::new(Cursor::new(damaged)), &index) {
        Err(_) => {}
        Ok(mut decoder) => {
            assert!(decoder.frame(3).is_err());
            assert!(decoder.frame(3).is_err());
        }
    }
    let mut short = index;
    short.frames.truncate(1);
    let mut decoder = TheoraStream::open(Box::new(Cursor::new(bytes)), &short).unwrap();
    assert!(decoder.finish().is_err());
    assert!(decoder.finish().is_err());
}

/// The fixture with a 24x24 picture at (`x`, `y`) inside its 32x32 coding surface.
fn with_picture(bytes: &[u8], index: &MgvIndex, x: u8, y: u8) -> Vec<u8> {
    use ogg::{PacketReader, PacketWriteEndInfo, PacketWriter};
    let mut reader = PacketReader::new(Cursor::new(&bytes[index.video_offset as usize..]));
    let mut writer = PacketWriter::new(Vec::new());
    let mut first = true;
    while let Some(mut packet) = reader.read_packet().unwrap() {
        if first {
            packet.data[14..20].copy_from_slice(&[0, 0, 24, 0, 0, 24]);
            packet.data[20..22].copy_from_slice(&[x, y]);
            first = false;
        }
        let end = if packet.last_in_stream() {
            PacketWriteEndInfo::EndStream
        } else {
            PacketWriteEndInfo::EndPage
        };
        let serial = packet.stream_serial();
        let granule = packet.absgp_page();
        writer
            .write_packet(packet.data, serial, end, granule)
            .unwrap();
    }
    let video = writer.into_inner();
    let mut padded = bytes[..index.video_offset as usize].to_vec();
    padded[8..12].copy_from_slice(&(video.len() as u32).to_le_bytes());
    padded.extend(video);
    padded
}

fn assert_crop(
    cropped: &cmvs_core::raster::RgbaImage,
    full: &cmvs_core::raster::RgbaImage,
    x: usize,
) {
    assert_eq!((cropped.width(), cropped.height()), (24, 24));
    for row in 0..24 {
        assert_eq!(
            &cropped.pixels()[row * 24 * 4..(row + 1) * 24 * 4],
            &full.pixels()[(row * 32 + x) * 4..(row * 32 + x + 24) * 4]
        );
    }
}

#[test]
fn padded_container_preserves_the_coding_surface_and_picture_container_crops() {
    let bytes = fixture();
    let index = MgvIndex::read(&mut Cursor::new(&bytes)).unwrap();
    let mut original = TheoraStream::open(Box::new(Cursor::new(bytes.clone())), &index).unwrap();
    let full = original.frame(0).unwrap();
    let padded = with_picture(&bytes, &index, 4, 8);
    let index = MgvIndex::read(&mut Cursor::new(&padded)).unwrap();
    let mut decoder = TheoraStream::open(Box::new(Cursor::new(padded.clone())), &index).unwrap();
    assert_eq!(decoder.frame(0).unwrap(), full);
    let mut picture = index.clone();
    picture.width = 24;
    picture.height = 24;
    let mut decoder = TheoraStream::open(Box::new(Cursor::new(padded.clone())), &picture).unwrap();
    assert_crop(&decoder.frame(0).unwrap(), &full, 4);
    picture.height = 25;
    assert!(TheoraStream::open(Box::new(Cursor::new(padded)), &picture).is_err());
}

#[test]
fn odd_picture_left_edge_keeps_absolute_chroma_columns() {
    let bytes = fixture();
    let index = MgvIndex::read(&mut Cursor::new(&bytes)).unwrap();
    let mut original = TheoraStream::open(Box::new(Cursor::new(bytes.clone())), &index).unwrap();
    let padded = with_picture(&bytes, &index, 5, 8);
    let mut picture = MgvIndex::read(&mut Cursor::new(&padded)).unwrap();
    picture.width = 24;
    picture.height = 24;
    let mut decoder = TheoraStream::open(Box::new(Cursor::new(padded)), &picture).unwrap();
    for frame in 0..4 {
        assert_crop(
            &decoder.frame(frame).unwrap(),
            &original.frame(frame).unwrap(),
            5,
        );
    }
}

#[test]
fn prefetching_decoder_matches_the_inline_stream() {
    use cmvs_core::mgv::DecoderFactory;
    use cmvs_video::{PrefetchTheoraFactory, TheoraFactory};
    let bytes = fixture();
    let index = MgvIndex::read(&mut Cursor::new(&bytes)).unwrap();
    let open = |factory: &dyn DecoderFactory| {
        factory
            .open(Box::new(Cursor::new(bytes.clone())), &index)
            .unwrap()
    };
    let mut inline = open(&TheoraFactory);
    let mut prefetch = open(&PrefetchTheoraFactory::default());
    // Forward with a duplicate, then backwards and forwards again.
    for frame in [0, 1, 2, 3, 0, 2, 3] {
        assert_eq!(prefetch.frame(frame).unwrap(), inline.frame(frame).unwrap());
    }
    assert!(prefetch.frame(4).is_err());
    assert_eq!(prefetch.frame(1).unwrap(), inline.frame(1).unwrap());
    prefetch.finish().unwrap();
    let mut short = index.clone();
    short.frames.truncate(1);
    let mut prefetch = PrefetchTheoraFactory::default()
        .open(Box::new(Cursor::new(bytes.clone())), &short)
        .unwrap();
    assert!(prefetch.finish().is_err());
}

#[test]
fn repeated_frames_do_not_rewind_the_prefetching_decoder() {
    use cmvs_core::mgv::DecoderFactory;
    use cmvs_video::PrefetchTheoraFactory;
    let bytes = fixture();
    let mut index = MgvIndex::read(&mut Cursor::new(&bytes)).unwrap();
    let mut frames = vec![index.frames[0]; 16];
    frames.extend_from_slice(&index.frames[1..]);
    index.frames = frames;
    let seeks = Arc::new(AtomicUsize::new(0));
    let mut inline = TheoraStream::open(Box::new(Cursor::new(bytes.clone())), &index).unwrap();
    let mut prefetch = PrefetchTheoraFactory::default()
        .open(
            Box::new(CountingReader {
                data: Cursor::new(bytes),
                seeks: seeks.clone(),
            }),
            &index,
        )
        .unwrap();
    for frame in 0..index.frames.len() as u32 {
        assert_eq!(prefetch.frame(frame).unwrap(), inline.frame(frame).unwrap());
    }
    prefetch.finish().unwrap();
    assert_eq!(
        seeks.load(Ordering::Relaxed),
        1,
        "sequential playback replayed"
    );
    assert_eq!(prefetch.frame(0).unwrap(), inline.frame(0).unwrap());
    assert_eq!(
        seeks.load(Ordering::Relaxed),
        2,
        "backward seek must replay"
    );
}

#[test]
#[ignore = "requires CMVS_THEORA_MGV"]
fn external_prefetch_playback_never_rewinds_forward_requests() {
    use cmvs_core::mgv::DecoderFactory;
    use cmvs_video::PrefetchTheoraFactory;
    let path = std::env::var("CMVS_THEORA_MGV").unwrap();
    let mut source = std::fs::File::open(&path).unwrap();
    let index = MgvIndex::read(&mut source).unwrap();
    let seeks = Arc::new(AtomicUsize::new(0));
    let mut inline =
        TheoraStream::open(Box::new(std::fs::File::open(&path).unwrap()), &index).unwrap();
    let mut prefetch = PrefetchTheoraFactory::default()
        .open(
            Box::new(CountingReader {
                data: source,
                seeks: seeks.clone(),
            }),
            &index,
        )
        .unwrap();
    let mut waits = Vec::new();
    for frame in 0..index.frames.len() as u32 {
        let start = std::time::Instant::now();
        let image = prefetch.frame(frame).unwrap();
        waits.push(start.elapsed());
        assert!(image == inline.frame(frame).unwrap(), "frame {frame}");
    }
    prefetch.finish().unwrap();
    waits.sort_unstable();
    eprintln!(
        "{} frames: rewinds={}, mean={:.3} ms, p99={:.3} ms, max={:.3} ms",
        waits.len(),
        seeks.load(Ordering::Relaxed) - 1,
        waits.iter().sum::<std::time::Duration>().as_secs_f64() * 1000.0 / waits.len() as f64,
        waits[waits.len() * 99 / 100].as_secs_f64() * 1000.0,
        waits.last().unwrap().as_secs_f64() * 1000.0
    );
    assert_eq!(
        seeks.load(Ordering::Relaxed),
        1,
        "forward playback replayed"
    );
}
