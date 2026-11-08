use super::*;
use cmvs_core::digest::{sha256, to_hex};
use ogg::PacketReader;
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    process::{Child, Command, Stdio},
    time::Instant,
};

fn hex(text: &str) -> Vec<u8> {
    text.trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}

fn packets(data: &[u8]) -> Vec<Vec<u8>> {
    let mut reader = PacketReader::new(Cursor::new(data));
    let mut out = Vec::new();
    while let Some(packet) = reader.read_packet().unwrap() {
        out.push(packet.data);
    }
    out
}

fn fixture() -> Vec<Vec<u8>> {
    let bytes = hex(include_str!("../../tests/fixtures/mgv1.hex"));
    let index = cmvs_core::mgv::MgvIndex::read(&mut Cursor::new(&bytes)).unwrap();
    packets(&bytes[index.video_offset as usize..])
}

fn top_down(frame: &Frame, ident: &Ident) -> Vec<u8> {
    let mut out = Vec::new();
    for p in 0..3 {
        let div = if p == 0 { 1 } else { 2 };
        let width = ident.picw as usize / div;
        let height = ident.pich as usize / div;
        let stride = ident.width / div;
        let x = usize::from(ident.picx) / div;
        let y = usize::from(ident.picy) / div;
        for row in (y..y + height).rev() {
            out.extend_from_slice(&frame.planes[p][row * stride + x..row * stride + x + width]);
        }
    }
    out
}

#[test]
fn synthetic_corpus_matches_ffmpeg_and_recorded_coded_planes() {
    let cases = [
        (
            include_str!("../../tests/fixtures/theora-tall.hex"),
            include_str!("../../tests/fixtures/theora-tall.sha256"),
            include_str!("../../tests/fixtures/theora-tall.coded.sha256"),
        ),
        (
            include_str!("../../tests/fixtures/theora-wide.hex"),
            include_str!("../../tests/fixtures/theora-wide.sha256"),
            include_str!("../../tests/fixtures/theora-wide.coded.sha256"),
        ),
        (
            include_str!("../../tests/fixtures/theora-crop.hex"),
            include_str!("../../tests/fixtures/theora-crop.sha256"),
            include_str!("../../tests/fixtures/theora-crop.coded.sha256"),
        ),
        (
            include_str!("../../tests/fixtures/theora-noise.hex"),
            include_str!("../../tests/fixtures/theora-noise.sha256"),
            include_str!("../../tests/fixtures/theora-noise.coded.sha256"),
        ),
    ];
    for (case, hashes, coded_hashes) in cases {
        let packets = packets(&hex(case));
        let ident = Ident::parse(&packets[0]).unwrap();
        let mut own = Decoder::new(&ident, &packets[2]).unwrap();
        let hashes: Vec<_> = hashes.lines().collect();
        let coded_hashes: Vec<_> = coded_hashes.lines().collect();
        assert_eq!(hashes.len(), coded_hashes.len());
        assert_eq!(packets.len() - 3, hashes.len());
        for (i, packet) in packets[3..].iter().enumerate() {
            let got = own.decode_frame(packet).unwrap();
            assert_eq!(coded_hash(got), coded_hashes[i], "coded frame {i}");
            assert_eq!(
                to_hex(&sha256(&top_down(got, &ident))),
                hashes[i],
                "FFmpeg frame {i}"
            );
        }
    }
}

#[test]
fn empty_packets_reuse_reference_and_invalid_first_frames_fail() {
    let packets = fixture();
    let ident = Ident::parse(&packets[0]).unwrap();
    let mut decoder = Decoder::new(&ident, &packets[2]).unwrap();
    assert!(decoder.decode_frame(&[]).is_err());
    let first = decoder.decode_frame(&packets[3]).unwrap().planes.clone();
    assert_eq!(decoder.decode_frame(&[]).unwrap().planes, first);
    assert_eq!(decoder.decode_frame(&[]).unwrap().planes, first);
    decoder.decode_frame(&packets[4]).unwrap();
    let mut inter = packets[3].clone();
    inter[0] |= 0x40;
    assert!(
        Decoder::new(&ident, &packets[2])
            .unwrap()
            .decode_frame(&inter)
            .is_err()
    );
}

#[test]
fn invalid_headers_are_rejected_before_frame_allocation() {
    let packets = fixture();
    for format in 1..4 {
        let mut header = packets[0].clone();
        header[41] = (header[41] & !0x18) | (format << 3);
        assert!(Ident::parse(&header).is_err());
    }
    let mut header = packets[0].clone();
    header[41] |= 1;
    assert!(Ident::parse(&header).is_err());
    let mut header = packets[0].clone();
    header[10..14].fill(0xff);
    assert!(Ident::parse(&header).is_err());
    let mut header = packets[0].clone();
    header[20] = 255;
    assert!(Ident::parse(&header).is_err());
    for end in 0..packets[1].len() {
        assert!(comments(&packets[1][..end]).is_err());
    }
    let mut comment = b"\x81theora".to_vec();
    comment.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(comments(&comment).is_err());
}

#[test]
fn packet_truncation_and_corruption_never_panic() {
    let packets = fixture();
    let ident = Ident::parse(&packets[0]).unwrap();
    for header in [&packets[0], &packets[2]] {
        for end in 0..header.len() {
            if header[0] == 0x80 {
                assert!(Ident::parse(&header[..end]).is_err());
            } else {
                assert!(Decoder::new(&ident, &header[..end]).is_err());
            }
        }
    }
    for (i, packet) in packets[3..].iter().enumerate() {
        for end in 0..packet.len() {
            let mut decoder = Decoder::new(&ident, &packets[2]).unwrap();
            for prior in &packets[3..3 + i] {
                decoder.decode_frame(prior).unwrap();
            }
            // Padding may be truncated, and an empty packet is a valid repeat.
            let _ = decoder.decode_frame(&packet[..end]);
        }
        for byte in 0..packet.len() {
            let mut damaged = packet.clone();
            damaged[byte] ^= 0xff;
            let mut decoder = Decoder::new(&ident, &packets[2]).unwrap();
            for prior in &packets[3..3 + i] {
                decoder.decode_frame(prior).unwrap();
            }
            if decoder.decode_frame(&damaged).is_err() {
                assert!(decoder.decode_frame(packet).is_err());
            }
        }
    }
}

#[test]
fn scalar_idct_matches_recorded_reference_for_extreme_coefficients() {
    let mut random = 0x1234_5678u32;
    let mut recorded = Vec::new();
    for iteration in 0..2048 {
        let mut coeffs = [0; 64];
        for v in coeffs
            .iter_mut()
            .take(if iteration < 64 { iteration + 1 } else { 64 })
        {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            *v = random as i16;
        }
        let scalar = dsp::idct_scalar(&coeffs);
        assert_eq!(dsp::idct(&coeffs, 64), scalar);
        recorded.extend(scalar.into_iter().flatten().flat_map(i16::to_le_bytes));
    }
    assert_eq!(
        to_hex(&sha256(&recorded)),
        "72a0a4f3b373f4b349fbf39b7b7ad69fabf5bbbac86768deaaf322bf075550ce"
    );
}

#[test]
fn sparse_idct_preserves_signed_wrap_and_rounding() {
    const ZIGZAG: [usize; 10] = [0, 1, 8, 16, 9, 2, 3, 10, 17, 24];
    let mut random = 0x3859_a1cdu32;
    let hashes: Vec<_> = include_str!("../../tests/fixtures/theora-idct-sparse.sha256")
        .lines()
        .collect();
    assert_eq!(hashes.len(), 9);
    for count in 2..=10 {
        let mut recorded = Vec::new();
        for _ in 0..1024 {
            let mut coeffs = [0; 64];
            for &ci in &ZIGZAG[..count] {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                coeffs[ci] = random as i16;
            }
            let got = dsp::idct(&coeffs, count as u8);
            assert_eq!(got, dsp::idct_scalar(&coeffs));
            recorded.extend(got.into_iter().flatten().flat_map(i16::to_le_bytes));
        }
        assert_eq!(to_hex(&sha256(&recorded)), hashes[count - 2]);
    }
}

fn coded_hash(frame: &Frame) -> String {
    to_hex(&sha256(&frame.planes.concat()))
}

#[test]
fn quant_ranges_and_three_block_quantizers_match_recorded_reference() {
    let packets = packets(&hex(include_str!("../../tests/fixtures/theora-quant.hex")));
    let ident = Ident::parse(&packets[0]).unwrap();
    let mut decoder = Decoder::new(&ident, &packets[2]).unwrap();
    let matrices: Vec<_> = include_str!("../../tests/fixtures/theora-quant.matrices.sha256")
        .lines()
        .collect();
    assert_eq!(matrices.len(), 6 * 64);
    for (table, expected) in decoder.setup.quant.iter().flatten().zip(matrices) {
        let bytes: Vec<_> = table.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(to_hex(&sha256(&bytes)), expected);
    }
    let qiis: Vec<_> = (0..decoder.geometry.blocks.len())
        .map(|bi| bi % 3)
        .collect();
    let hashes: Vec<_> = include_str!("../../tests/fixtures/theora-quant.coded.sha256")
        .lines()
        .collect();
    assert_eq!(packets.len() - 3, hashes.len());
    for (packet, expected) in packets[3..].iter().zip(hashes) {
        assert_eq!(coded_hash(decoder.decode_frame(packet).unwrap()), expected);
        assert_eq!(decoder.state.qi, qiis);
    }
}

#[test]
fn golden_and_four_vector_frames_match_recorded_reference() {
    let packets = packets(&hex(include_str!("../../tests/fixtures/theora-motion.hex")));
    let ident = Ident::parse(&packets[0]).unwrap();
    let mut decoder = Decoder::new(&ident, &packets[2]).unwrap();
    let hashes: Vec<_> = include_str!("../../tests/fixtures/theora-motion.coded.sha256")
        .lines()
        .collect();
    assert_eq!(packets.len() - 3, hashes.len());
    let mut mode_mask = 0u8;
    for (packet, expected) in packets[3..].iter().zip(hashes) {
        assert_eq!(coded_hash(decoder.decode_frame(packet).unwrap()), expected);
        for &mode in &decoder.state.modes {
            mode_mask |= 1 << mode;
        }
    }
    assert_ne!(mode_mask & (1 << 7), 0, "four-vector path was exercised");
    assert_ne!(
        mode_mask & ((1 << 5) | (1 << 6)),
        0,
        "golden-reference path was exercised"
    );
}

/// Optional large-corpus comparison. CMVS_THEORA_OGV is an Ogg/Theora file;
/// CMVS_THEORA_YUV, when set, is FFmpeg's full picture-region yuv420p output.
/// CMVS_THEORA_FFMPEG, when set, names an FFmpeg executable that streams the
/// reference planes through a pipe instead of storing a full raw movie on disk.
/// Set CMVS_THEORA_CODED=1 to compare the entire coding surface instead.
#[test]
#[ignore = "requires CMVS_THEORA_OGV"]
fn external_corpus_matches_ffmpeg() {
    let path = std::env::var("CMVS_THEORA_OGV").unwrap();
    let data = std::fs::read(&path).unwrap();
    let packets = packets(&data);
    let ident = Ident::parse(&packets[0]).unwrap();
    let mut own = Decoder::new(&ident, &packets[2]).unwrap();
    let coded = std::env::var("CMVS_THEORA_CODED").is_ok_and(|v| v == "1");
    let comparison = if coded {
        Ident {
            picw: ident.width as u32,
            pich: ident.height as u32,
            picx: 0,
            picy: 0,
            ..ident.clone()
        }
    } else {
        ident
    };
    let external = std::env::var("CMVS_THEORA_YUV")
        .ok()
        .map(|path| std::fs::read(path).unwrap());
    struct ReferenceProcess(Child);
    impl Drop for ReferenceProcess {
        fn drop(&mut self) {
            // Also reap the child if a pixel comparison panics.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut reference = std::env::var("CMVS_THEORA_FFMPEG").ok().map(|program| {
        ReferenceProcess(
            Command::new(program)
                .args([
                    "-v",
                    "error",
                    "-nostdin",
                    "-c:v",
                    "theora",
                    "-threads",
                    "1",
                    "-apply_cropping",
                    if coded { "0" } else { "1" },
                    "-i",
                ])
                .arg(&path)
                .args(["-vsync", "0", "-pix_fmt", "yuv420p", "-f", "rawvideo", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        )
    });
    assert!(
        external.is_some() || reference.is_some(),
        "set CMVS_THEORA_YUV or CMVS_THEORA_FFMPEG for an independent reference"
    );
    let picture_size = comparison.picw as usize * comparison.pich as usize * 3 / 2;
    let mut reference_pixels = vec![0; picture_size];
    let mut ffmpeg_frames = 0;
    let mut offset = 0;
    for (i, packet) in packets[3..].iter().enumerate() {
        let got = own.decode_frame(packet).unwrap();
        // FFmpeg emits no picture for an empty duplicate packet.
        if packet.is_empty() {
            continue;
        }
        if let Some(raw) = &external {
            let picture = top_down(got, &comparison);
            assert_eq!(
                &picture,
                &raw[offset..offset + picture.len()],
                "FFmpeg frame {i}"
            );
            offset += picture.len();
        }
        if let Some(reference) = &mut reference {
            let picture = top_down(got, &comparison);
            assert_eq!(
                picture.len(),
                reference_pixels.len(),
                "requires even picture dimensions"
            );
            reference
                .0
                .stdout
                .as_mut()
                .unwrap()
                .read_exact(&mut reference_pixels)
                .unwrap();
            if let Some(at) = picture
                .iter()
                .zip(&reference_pixels)
                .position(|(a, b)| a != b)
            {
                panic!(
                    "FFmpeg frame {i} sample {at}: own={}, reference={}",
                    picture[at], reference_pixels[at]
                );
            }
            ffmpeg_frames += 1;
        }
        if i % 500 == 0 || i + 4 == packets.len() {
            eprintln!(
                "validated packet {i}/{} picture_sha256={}",
                packets.len() - 3,
                to_hex(&sha256(&top_down(got, &comparison)))
            );
        }
    }
    if let Some(raw) = external {
        assert_eq!(offset, raw.len());
    }
    if let Some(reference) = &mut reference {
        assert_eq!(
            reference.0.stdout.as_mut().unwrap().read(&mut [0]).unwrap(),
            0,
            "FFmpeg emitted extra frames"
        );
        assert!(reference.0.wait().unwrap().success());
        eprintln!("FFmpeg matched {ffmpeg_frames} nonempty packets");
    }
}

/// Measure the crate's decoder without an oracle or another live decoder
/// competing for the CPU cache. Header setup, packet I/O and output handling
/// are outside the timed section. Use the pixel-comparison test separately.
#[test]
#[ignore = "requires CMVS_THEORA_OGV"]
fn external_decoder_timing() {
    let data = std::fs::read(std::env::var("CMVS_THEORA_OGV").unwrap()).unwrap();
    let packets = packets(&data);
    let ident = Ident::parse(&packets[0]).unwrap();
    let mut elapsed = std::time::Duration::ZERO;
    let repeats = 2;
    for _ in 0..repeats {
        let mut decoder = Decoder::new(&ident, &packets[2]).unwrap();
        for packet in &packets[3..] {
            let start = Instant::now();
            let frame = decoder.decode_frame(packet).unwrap();
            elapsed += start.elapsed();
            std::hint::black_box(&frame.planes);
        }
    }
    let frames = packets.len() - 3;
    eprintln!(
        "decoder benchmark: {frames} packets, {repeats} passes, {:.6} ms/frame",
        elapsed.as_secs_f64() * 1000.0 / (frames * repeats) as f64
    );
}

/// Check public MGV indexing, duplicate nominal frames, padded RGBA and replay
/// against direct packet decoding. Codec pixels have separate pinned/FFmpeg tests.
#[test]
#[ignore = "requires CMVS_THEORA_MGV"]
fn external_mgv_playback_matches_packet_decoder() {
    use cmvs_core::mgv::{Decoder as _, MgvIndex};
    let path = std::env::var("CMVS_THEORA_MGV").unwrap();
    let mut source = std::fs::File::open(&path).unwrap();
    let index = MgvIndex::read(&mut source).unwrap();
    source.seek(SeekFrom::Start(index.video_offset)).unwrap();
    let mut ogg = PacketReader::new(source.take(u64::from(index.video_size)));
    let header = ogg.read_packet_expected().unwrap();
    let ident = Ident::parse(&header.data).unwrap();
    ogg.read_packet_expected().unwrap(); // Comment, validated by the public stream.
    let setup = ogg.read_packet_expected().unwrap();
    let mut direct = Decoder::new(&ident, &setup.data).unwrap();
    let mut stream =
        crate::TheoraStream::open(Box::new(std::fs::File::open(&path).unwrap()), &index).unwrap();
    let mut previous_packet = None;
    let mut expected = None;
    let mut first = None;
    let mut decoded = 0;
    for (nominal, entry) in index.frames.iter().enumerate() {
        if previous_packet != Some(entry.packet) {
            let packet = ogg.read_packet_expected().unwrap();
            let frame = direct.decode_frame(&packet.data).unwrap();
            expected = Some(crate::to_rgba(&ident, frame, &index).unwrap());
            previous_packet = Some(entry.packet);
            decoded += 1;
        }
        let image = stream.frame(nominal as u32).unwrap();
        assert!(
            Some(&image) == expected.as_ref(),
            "MGV RGBA differs at nominal {nominal}, packet {}",
            entry.packet
        );
        if nominal == 0 {
            first = Some(image.clone());
        }
        if nominal % 500 == 0 || nominal + 1 == index.frames.len() {
            eprintln!(
                "validated nominal {nominal}/{} packet={} rgba_sha256={}",
                index.frames.len(),
                entry.packet,
                to_hex(&sha256(image.pixels()))
            );
        }
    }
    assert_eq!(decoded, index.packet_count());
    stream.finish().unwrap();
    assert!(ogg.read_packet().unwrap().is_none());
    assert!(
        Some(stream.frame(0).unwrap()) == first,
        "MGV replay after finish differs"
    );
    eprintln!(
        "MGV matched {} nominal frames, {decoded} decoded packets, {} duplicates; replay after finish matched",
        index.frames.len(),
        index.frames.len() - decoded as usize
    );
}
