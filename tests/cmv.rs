use cmvs_rs::cmv::{CmvIndex, KIND_JBPX, KIND_OGG};
use std::io::Cursor;

/// A CMV6 file with `frames` JBPX entries of 4 bytes each and one 3-byte Ogg entry.
pub fn cmv(frames: u32, rate: u32, rate_time: u32) -> Vec<u8> {
    let count = frames + 1;
    let resource = 44 + count * 20;
    let data = frames * 4 + 3;
    let mut bytes = b"CMV6".to_vec();
    for word in [
        resource,
        resource + data,
        0,
        frames,
        rate,
        rate_time,
        1280,
        720,
        24,
        2,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    for i in 0..frames {
        for word in [i, 4, 1280 * 720 * 3, KIND_JBPX, i * 4] {
            bytes.extend(word.to_le_bytes());
        }
    }
    for word in [frames, 3, 3, KIND_OGG, frames * 4] {
        bytes.extend(word.to_le_bytes());
    }
    for i in 0..frames {
        bytes.extend(b"JBP");
        bytes.push(i as u8);
    }
    bytes.extend(b"Ogg");
    bytes
}

#[test]
fn header_and_index_locate_frames_and_soundtrack() {
    let index = CmvIndex::read(&mut Cursor::new(cmv(3, 24, 1))).unwrap();
    assert_eq!(index.header.max_sequence, 3);
    assert_eq!((index.header.rate, index.header.rate_time), (24, 1));
    assert_eq!((index.header.width, index.header.height), (1280, 720));
    assert_eq!(index.entries.len(), 4);
    assert_eq!(index.frames().count(), 3);
    let last = index.frames().last().unwrap();
    assert_eq!(last.file_offset(&index.header), 44 + 4 * 20 + 8);
    let audio = index.audio().unwrap();
    assert_eq!(
        (audio.stored_size, audio.file_offset(&index.header)),
        (3, 44 + 80 + 12)
    );
}

#[test]
fn malformed_containers_are_rejected() {
    let good = cmv(2, 24, 1);
    let mut cases = vec![];
    let mut signature = good.clone();
    signature[3] = b'5';
    cases.push(signature);
    let mut size = good.clone();
    size[8] ^= 1;
    cases.push(size);
    let mut rate = good.clone();
    rate[20..24].copy_from_slice(&0u32.to_le_bytes());
    cases.push(rate);
    let mut resources = good.clone();
    resources[4] += 1;
    cases.push(resources);
    let mut outside = good.clone();
    // First entry's offset points past the end.
    outside[44 + 16..44 + 20].copy_from_slice(&1000u32.to_le_bytes());
    cases.push(outside);
    cases.push(good[..60].to_vec());
    for bytes in cases {
        assert!(CmvIndex::read(&mut Cursor::new(bytes)).is_err());
    }
}
