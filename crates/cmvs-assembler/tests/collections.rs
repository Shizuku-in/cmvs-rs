use cmvs_assembler::cursor;
use cmvs_core::{
    format::{
        cursor::CursorBitmap,
        pb2::{self, CollectionWriter},
        pb3::Compression,
    },
    render::raster::RgbaImage,
};

#[test]
fn animation_frames_reference_frame_zero_and_reject_unrepresentable_tiles() {
    let rgba = |changes: &[(usize, [u8; 4])]| {
        let mut p = [10, 20, 30, 255].repeat(9 * 9);
        for (i, c) in changes {
            p[*i * 4..*i * 4 + 4].copy_from_slice(c);
        }
        RgbaImage::new(9, 9, p).unwrap()
    };
    let base = rgba(&[]);
    let changed = rgba(&[(0, [0, 0, 0, 255])]);
    for mode in [Compression::Lzss, Compression::Literals] {
        let mut w = CollectionWriter::new(base.clone(), None, 32, mode).unwrap();
        assert!(w.push(&rgba(&[(8, [1, 2, 3, 255])]), None).is_err());
        assert!(w.push(&rgba(&[(5, [1, 2, 3, 127])]), None).is_err());
        w.push(&changed, None).unwrap();
        w.push(&base, None).unwrap();
        let raw = w.finish();
        assert_eq!(pb2::decode_frame(&raw, 1).unwrap(), changed);
        assert_eq!(pb2::decode_frame(&raw, 2).unwrap(), base);
        let mut w = CollectionWriter::new(base.clone(), Some(b"same"), 4, mode).unwrap();
        let edge = rgba(&[(80, [1, 2, 3, 127])]);
        w.push(&edge, Some(b"same")).unwrap();
        let raw = w.finish();
        assert_eq!(pb2::part_names(&raw).unwrap(), [b"same", b"same"]);
        assert_eq!(pb2::decode_part_index(&raw, 1).unwrap(), edge);
    }
}

#[test]
fn cursor_project_keeps_indices_padding_and_screen_inversion() {
    let table =
        b"cmvs-cur-v1\t3\t1\t4\t2\t0\npalette\t00000000ffffff00ffffff00\nrow\t1200aaff\tc0003434\n";
    let raw = cursor::assemble(table.as_slice()).unwrap();
    let image = CursorBitmap::decode(&raw).unwrap();
    assert_eq!(image.hotspot(), (2, 0));
    assert_eq!(
        image.composite(&[[1, 2, 3]; 3]).unwrap(),
        [[254, 253, 252], [254, 253, 252], [0, 0, 0]]
    );
    let mut out = Vec::new();
    cursor::export(&raw, &mut out).unwrap();
    assert_eq!(out, table);
}
