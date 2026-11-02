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
