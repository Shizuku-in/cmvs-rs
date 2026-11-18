#![no_main]

use cmvs_core::{MAX_PAYLOAD, format::cpz};
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    // Raw archives remain valid corpus seeds. A length-prefixed query can also
    // precede a complete archive so mutations of the name leave its index intact.
    let (query, archive) = if !data.starts_with(b"CPZ")
        && let Some((len, body)) = data.split_first_chunk::<2>()
        && let Some(parts) = body.split_at_checked(usize::from(u16::from_le_bytes(*len)))
    {
        parts
    } else {
        (b"start.ps3".as_slice(), data)
    };
    let _ = cpz::ArchiveVersion::detect(archive);
    let _ = cpz::ArchiveLayout::read(&mut Cursor::new(archive));
    if let Ok(index) = cpz::ArchiveIndex::read(&mut Cursor::new(archive)) {
        if let Ok(Some(i)) = index.lookup(query) {
            assert!(i < index.entries().len());
        }
        if let Ok(i) = index.find(query) {
            assert_eq!(index.entries()[i].name, query);
        }
        for i in 0..index.entries().len().min(4) {
            if let Ok(bytes) = index.read_entry(&mut Cursor::new(archive), i) {
                assert!(bytes.len() <= MAX_PAYLOAD);
            }
            if let Ok(bytes) = index.read_entry_verified(&mut Cursor::new(archive), i) {
                assert!(bytes.len() <= MAX_PAYLOAD);
            }
        }
    }
});
