use cmvs_core::{
    digest::{sha256, to_hex},
    mgv::{Decoder, MgvIndex},
};
use cmvs_desktop::video::TheoraStream;
use std::io::Cursor;
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

#[test]
fn padded_container_preserves_the_coding_surface_and_picture_container_crops() {
    use ogg::{PacketReader, PacketWriteEndInfo, PacketWriter};
    let bytes = fixture();
    let index = MgvIndex::read(&mut Cursor::new(&bytes)).unwrap();
    let mut original = TheoraStream::open(Box::new(Cursor::new(bytes.clone())), &index).unwrap();
    let full = original.frame(0).unwrap();
    let mut reader = PacketReader::new(Cursor::new(&bytes[index.video_offset as usize..]));
    let mut writer = PacketWriter::new(Vec::new());
    let mut first = true;
    while let Some(mut packet) = reader.read_packet().unwrap() {
        if first {
            // A 24x24 picture at (4, 8) inside the existing 32x32 coding surface.
            packet.data[14..20].copy_from_slice(&[0, 0, 24, 0, 0, 24]);
            packet.data[20..22].copy_from_slice(&[4, 8]);
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
    let index = MgvIndex::read(&mut Cursor::new(&padded)).unwrap();
    let mut decoder = TheoraStream::open(Box::new(Cursor::new(padded.clone())), &index).unwrap();
    assert_eq!(decoder.frame(0).unwrap(), full);
    let mut picture = index.clone();
    picture.width = 24;
    picture.height = 24;
    let mut decoder = TheoraStream::open(Box::new(Cursor::new(padded.clone())), &picture).unwrap();
    let cropped = decoder.frame(0).unwrap();
    assert_eq!((cropped.width(), cropped.height()), (24, 24));
    for row in 0..24 {
        assert_eq!(
            &cropped.pixels()[row * 24 * 4..(row + 1) * 24 * 4],
            &full.pixels()[(row * 32 + 4) * 4..(row * 32 + 28) * 4]
        );
    }
    picture.height = 25;
    assert!(TheoraStream::open(Box::new(Cursor::new(padded)), &picture).is_err());
}
