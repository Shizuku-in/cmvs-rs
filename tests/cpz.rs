use cmvs_rs::cpz::{CPZ6_HEADER_SIZE, Cpz6Layout};

fn header(dirs: u32, dir_bytes: u32, file_bytes: u32) -> [u8; CPZ6_HEADER_SIZE] {
    let mut header = [0; CPZ6_HEADER_SIZE];
    header[..4].copy_from_slice(b"CPZ6");
    for (offset, value) in [
        (4, dirs ^ 0xfe3a53da),
        (8, dir_bytes ^ 0x37f298e8),
        (12, file_bytes ^ 0x7a6f3a2d),
        (52, 1 ^ 0xfb73a956),
    ] {
        header[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    header
}

#[test]
fn data_offset_accounts_for_both_indexes_without_reading_archive_body() {
    let layout = Cpz6Layout::parse(&header(1, 24, 200), 1000).unwrap();
    assert_eq!(layout.directory_count, 1);
    assert_eq!(layout.data_offset, 288);
    assert_eq!(layout.data_bytes, 712);
    assert_eq!(layout.encryption_mode, 1);
}

#[test]
fn every_truncated_header_and_wrong_version_rejected() {
    let mut bytes = header(1, 24, 200);
    for length in 0..64 {
        assert!(Cpz6Layout::parse(&bytes[..length], 1000).is_err());
    }
    bytes[3] = b'7';
    assert!(Cpz6Layout::parse(&bytes, 1000).is_err());
}

#[test]
fn truncated_archive_and_impossible_directory_count_rejected() {
    assert!(Cpz6Layout::parse(&header(1, 24, 200), 287).is_err());
    assert!(Cpz6Layout::parse(&header(2, 24, 200), 1000).is_err());
    assert!(Cpz6Layout::parse(&header(u32::MAX, u32::MAX, 0), u64::MAX).is_err());
}

#[test]
fn index_arithmetic_uses_64_bits_and_does_not_allocate_from_header_fields() {
    let layout = Cpz6Layout::parse(&header(1, u32::MAX, u32::MAX), u64::MAX).unwrap();
    assert_eq!(layout.data_offset, 64 + 2 * u64::from(u32::MAX));
    assert!(Cpz6Layout::parse(&header(1, u32::MAX, u32::MAX), u32::MAX as u64).is_err());
}
