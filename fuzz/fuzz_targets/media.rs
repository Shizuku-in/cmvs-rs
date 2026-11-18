#![no_main]

use cmvs_core::{
    MAX_PAYLOAD,
    format::{cmv, cmv9, mgv, mv2},
};
use libfuzzer_sys::fuzz_target;
use std::{io::Cursor, sync::Arc};

fuzz_target!(|data: &[u8]| {
    if let Ok(idx) = cmv::CmvIndex::read(&mut Cursor::new(data))
        && let Ok(img) = idx.decode_frame(&mut Cursor::new(data), 0)
    {
        assert!(img.pixels().len() <= MAX_PAYLOAD);
    }
    if let Ok(idx) = cmv::CmvIndex::read_scene(&mut Cursor::new(data))
        && let Ok(img) = idx.decode_frame(&mut Cursor::new(data), 0)
    {
        assert!(img.pixels().len() <= MAX_PAYLOAD);
    }
    if let Ok(idx) = cmv9::Cmv9Index::parse(data)
        && let Ok(img) = idx.decode_frame(data, 0)
    {
        assert!(img.pixels().len() <= MAX_PAYLOAD);
    }
    let _ = mgv::MgvIndex::read(&mut Cursor::new(data));
    let _ = mv2::Header::parse(data);
    if let Ok((_, pcm)) = mv2::decode(Arc::from(data)) {
        assert!(pcm.len() * 2 <= MAX_PAYLOAD);
    }
});
