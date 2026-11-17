#![no_main]

use cmvs_core::{MAX_PAYLOAD, format::cpz};
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let _ = cpz::ArchiveVersion::detect(data);
    let _ = cpz::ArchiveLayout::read(&mut Cursor::new(data));
    if let Ok(index) = cpz::ArchiveIndex::read(&mut Cursor::new(data)) {
        let _ = index.lookup(b"start.ps3");
        let _ = index.find(b"start.ps3");
        for i in 0..index.entries().len().min(4) {
            if let Ok(bytes) = index.read_entry_verified(&mut Cursor::new(data), i) {
                assert!(bytes.len() <= MAX_PAYLOAD);
            }
        }
    }
});
