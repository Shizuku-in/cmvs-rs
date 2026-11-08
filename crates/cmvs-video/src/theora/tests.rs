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
fn synthetic_corpus_matches_ffmpeg_and_previous_decoder_planes() {
    let cases = [
        (
            include_str!("../../tests/fixtures/theora-tall.hex"),
            include_str!("../../tests/fixtures/theora-tall.sha256"),
        ),
        (
            include_str!("../../tests/fixtures/theora-wide.hex"),
            include_str!("../../tests/fixtures/theora-wide.sha256"),
        ),
        (
            include_str!("../../tests/fixtures/theora-crop.hex"),
            include_str!("../../tests/fixtures/theora-crop.sha256"),
        ),
        (
            include_str!("../../tests/fixtures/theora-noise.hex"),
            include_str!("../../tests/fixtures/theora-noise.sha256"),
        ),
    ];
    for (case, hashes) in cases {
        let packets = packets(&hex(case));
        let ident = Ident::parse(&packets[0]).unwrap();
        let mut own = Decoder::new(&ident, &packets[2]).unwrap();
        let mut old = oxideav_theora::FrameDecoder::new(
            oxideav_theora::decode_identification_header(&packets[0]).unwrap(),
            oxideav_theora::decode_setup_header(&packets[2]).unwrap(),
        )
        .unwrap();
        let hashes: Vec<_> = hashes.lines().collect();
        assert_eq!(packets.len() - 3, hashes.len());
        for (i, packet) in packets[3..].iter().enumerate() {
            let got = own.decode_frame(packet).unwrap();
            let old = old.decode_frame(packet).unwrap();
            assert_eq!(got.planes[0], old.frame.samples_y, "Y, frame {i}");
            assert_eq!(got.planes[1], old.frame.samples_cb, "Cb, frame {i}");
            assert_eq!(got.planes[2], old.frame.samples_cr, "Cr, frame {i}");
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
fn scalar_idct_matches_reference_for_extreme_and_sparse_coefficients() {
    let mut random = 0x1234_5678u32;
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
        let reference = oxideav_theora::inverse_dct_2d(&coeffs);
        assert_eq!(dsp::idct_scalar(&coeffs), reference);
        assert_eq!(dsp::idct(&coeffs, 64), reference);
    }
}

#[test]
fn sparse_idct_preserves_signed_wrap_and_rounding() {
    const ZIGZAG: [usize; 10] = [0, 1, 8, 16, 9, 2, 3, 10, 17, 24];
    let mut random = 0x3859_a1cdu32;
    for count in 2..=10 {
        for _ in 0..1024 {
            let mut coeffs = [0; 64];
            for &ci in &ZIGZAG[..count] {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                coeffs[ci] = random as i16;
            }
            assert_eq!(
                dsp::idct(&coeffs, count as u8),
                oxideav_theora::inverse_dct_2d(&coeffs)
            );
        }
    }
}

#[test]
fn quant_ranges_and_three_block_quantizers_match_reference() {
    use oxideav_theora::{FrameEncoder, SetupHeaderTables, SourceFrame};
    let packets = fixture();
    let old_ident = oxideav_theora::decode_identification_header(&packets[0]).unwrap();
    let ident = Ident::parse(&packets[0]).unwrap();
    let mut setup = SetupHeaderTables::vp3_defaults();
    let q = &mut setup.quantization_parameters;
    q.base_matrices
        .push(std::array::from_fn(|i| (i * 3 + 7) as u8));
    q.num_base_matrices = q.base_matrices.len() as u16;
    let last = q.num_base_matrices - 1;
    for t in 0..2 {
        for p in 0..3 {
            let base = q.quant_range_base_matrix_indices[t][p][0];
            q.num_quant_ranges[t][p] = 4;
            q.quant_range_sizes[t][p][..4].copy_from_slice(&[1, 20, 21, 21]);
            q.quant_range_base_matrix_indices[t][p][..5]
                .copy_from_slice(&[base, last, base, last, base]);
        }
    }
    let setup_packet = oxideav_theora::encode_setup_header(&setup).unwrap();
    let mut own = Decoder::new(&ident, &setup_packet).unwrap();
    for t in 0..2 {
        for p in 0..3 {
            for qi in 0..64 {
                assert_eq!(
                    own.setup.quant[t * 3 + p][qi],
                    oxideav_theora::compute_quantization_matrix(
                        &setup.quantization_parameters,
                        t,
                        p,
                        qi
                    )
                    .unwrap()
                    .values
                );
            }
        }
    }
    let encoder = FrameEncoder::new(old_ident.clone(), setup.clone(), 40).unwrap();
    let mut old = oxideav_theora::FrameDecoder::new(old_ident, setup).unwrap();
    let qiis: Vec<_> = (0..own.geometry.blocks.len())
        .map(|bi| (bi % 3) as u8)
        .collect();
    for n in 0..4 {
        let planes = own.geometry.sizes.map(|(w, h)| {
            (0..w * h)
                .map(|i| ((i * 31 + n * 71) % 256) as u8)
                .collect()
        });
        let [samples_y, samples_cb, samples_cr] = planes;
        let source = SourceFrame {
            samples_y,
            samples_cb,
            samples_cr,
        };
        let packet = encoder
            .encode_intra_frame_with_qiis(&source, &[0, 21, 63], &qiis)
            .unwrap();
        let got = own.decode_frame(&packet).unwrap();
        let expected = old.decode_frame(&packet).unwrap();
        assert_eq!(
            got.planes,
            [
                expected.frame.samples_y,
                expected.frame.samples_cb,
                expected.frame.samples_cr
            ]
        );
        assert_eq!(
            own.state.qi,
            qiis.iter().map(|&v| v as usize).collect::<Vec<_>>()
        );
    }
}

#[test]
fn golden_and_four_vector_frames_match_reference() {
    use oxideav_theora::{FrameEncoder, SetupHeaderTables, SourceFrame};
    let packets = fixture();
    let old_ident = oxideav_theora::decode_identification_header(&packets[0]).unwrap();
    let ident = Ident::parse(&packets[0]).unwrap();
    let setup = SetupHeaderTables::vp3_defaults();
    let mut own = Decoder::new(
        &ident,
        &oxideav_theora::encode_setup_header(&setup).unwrap(),
    )
    .unwrap();
    let encoder = FrameEncoder::new(old_ident.clone(), setup.clone(), 55).unwrap();
    let mut old = oxideav_theora::FrameDecoder::new(old_ident, setup).unwrap();
    let mut mode_mask = 0u8;
    for n in 0..12 {
        let planes = own.geometry.sizes.map(|(w, h)| {
            (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    // Distinct 8x8 tiles move in opposite directions, then return
                    // to the keyframe so the encoder selects the golden reference.
                    let phase = if n % 4 == 3 { 0 } else { n };
                    let dx = if (x / 8 + y / 8) % 2 == 0 {
                        phase * 2
                    } else {
                        w - phase % w
                    };
                    let x = (x + dx) % w;
                    ((x * x * 3 + y * 17 + x * y * 11) % 220 + 16) as u8
                })
                .collect()
        });
        let [samples_y, samples_cb, samples_cr] = planes;
        let source = SourceFrame {
            samples_y,
            samples_cb,
            samples_cr,
        };
        let packet = if n == 0 {
            encoder.encode_intra_frame(&source).unwrap()
        } else {
            let refs = old.reference_store().as_reference_plane_set().unwrap();
            if n % 4 == 3 {
                encoder.encode_inter_frame_golden(&source, &refs).unwrap()
            } else {
                encoder.encode_inter_frame_four_mv(&source, &refs).unwrap()
            }
        };
        let got = own.decode_frame(&packet).unwrap();
        let expected = old.decode_frame(&packet).unwrap();
        assert_eq!(
            got.planes,
            [
                expected.frame.samples_y,
                expected.frame.samples_cb,
                expected.frame.samples_cr
            ]
        );
        for &mode in &own.state.modes {
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

/// Optional large corpus/benchmark. CMVS_THEORA_OGV is an Ogg/Theora file;
/// CMVS_THEORA_YUV, when set, is FFmpeg's full picture-region yuv420p output.
/// CMVS_THEORA_FFMPEG, when set, names an FFmpeg executable that streams the
/// reference planes through a pipe instead of storing a full raw movie on disk.
#[test]
#[ignore = "requires CMVS_THEORA_OGV"]
fn external_corpus_and_decode_timing() {
    let path = std::env::var("CMVS_THEORA_OGV").unwrap();
    let data = std::fs::read(&path).unwrap();
    let packets = packets(&data);
    let ident = Ident::parse(&packets[0]).unwrap();
    let mut own = Decoder::new(&ident, &packets[2]).unwrap();
    let mut old = oxideav_theora::FrameDecoder::new(
        oxideav_theora::decode_identification_header(&packets[0]).unwrap(),
        oxideav_theora::decode_setup_header(&packets[2]).unwrap(),
    )
    .unwrap();
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
                .args(["-v", "error", "-nostdin", "-threads", "1", "-i"])
                .arg(&path)
                .args(["-vsync", "0", "-pix_fmt", "yuv420p", "-f", "rawvideo", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        )
    });
    let picture_size = ident.picw as usize * ident.pich as usize * 3 / 2;
    let mut reference_pixels = vec![0; picture_size];
    let mut ffmpeg_frames = 0;
    let mut offset = 0;
    let mut ours = std::time::Duration::ZERO;
    let mut theirs = std::time::Duration::ZERO;
    for (i, packet) in packets[3..].iter().enumerate() {
        let start = Instant::now();
        let got = own.decode_frame(packet).unwrap();
        ours += start.elapsed();
        let start = Instant::now();
        let expected = old.decode_frame(packet).unwrap();
        theirs += start.elapsed();
        for (p, want) in [
            &expected.frame.samples_y,
            &expected.frame.samples_cb,
            &expected.frame.samples_cr,
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(at) = got.planes[p].iter().zip(want).position(|(a, b)| a != b) {
                panic!(
                    "plane {p} frame {i} sample {at}: own={}, reference={}",
                    got.planes[p][at], want[at]
                );
            }
            assert_eq!(got.planes[p].len(), want.len());
        }
        if let Some(raw) = &external {
            let picture = top_down(got, &ident);
            assert_eq!(
                &picture,
                &raw[offset..offset + picture.len()],
                "FFmpeg frame {i}"
            );
            offset += picture.len();
        }
        if let Some(reference) = &mut reference {
            // FFmpeg does not emit pictures for empty Theora duplicate packets;
            // those packets are still compared against the old decoder above.
            if !packet.is_empty() {
                let picture = top_down(got, &ident);
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
        }
        if i % 500 == 0 || i + 4 == packets.len() {
            eprintln!(
                "validated packet {i}/{} picture_sha256={}",
                packets.len() - 3,
                to_hex(&sha256(&top_down(got, &ident)))
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
    let n = (packets.len() - 3) as f64;
    eprintln!(
        "{}x{}: {n} packets; own {:.3} ms/frame, oxideav {:.3} ms/frame, {:.2}x",
        ident.width,
        ident.height,
        ours.as_secs_f64() * 1000.0 / n,
        theirs.as_secs_f64() * 1000.0 / n,
        theirs.as_secs_f64() / ours.as_secs_f64()
    );
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

/// Exercise the public MGV playback path, including duplicate nominal frames,
/// padded RGBA output, end-of-stream validation and replay after finish.
#[test]
#[ignore = "requires CMVS_THEORA_MGV"]
fn external_mgv_playback_matches_reference() {
    use cmvs_core::mgv::{Decoder as _, MgvIndex};
    let path = std::env::var("CMVS_THEORA_MGV").unwrap();
    let mut source = std::fs::File::open(&path).unwrap();
    let index = MgvIndex::read(&mut source).unwrap();
    source.seek(SeekFrom::Start(index.video_offset)).unwrap();
    let mut ogg = PacketReader::new(source.take(u64::from(index.video_size)));
    let header = ogg.read_packet_expected().unwrap();
    let ident = Ident::parse(&header.data).unwrap();
    let old_ident = oxideav_theora::decode_identification_header(&header.data).unwrap();
    ogg.read_packet_expected().unwrap(); // Comment, validated by the public stream.
    let setup = ogg.read_packet_expected().unwrap();
    let mut old = oxideav_theora::FrameDecoder::new(
        old_ident,
        oxideav_theora::decode_setup_header(&setup.data).unwrap(),
    )
    .unwrap();
    let mut stream =
        crate::TheoraStream::open(Box::new(std::fs::File::open(&path).unwrap()), &index).unwrap();
    let mut previous_packet = None;
    let mut expected = None;
    let mut first = None;
    let mut decoded = 0;
    for (nominal, entry) in index.frames.iter().enumerate() {
        if previous_packet != Some(entry.packet) {
            let packet = ogg.read_packet_expected().unwrap();
            let old = old.decode_frame(&packet.data).unwrap();
            let frame = Frame {
                planes: [
                    old.frame.samples_y,
                    old.frame.samples_cb,
                    old.frame.samples_cr,
                ],
            };
            expected = Some(crate::to_rgba(&ident, &frame, &index).unwrap());
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
