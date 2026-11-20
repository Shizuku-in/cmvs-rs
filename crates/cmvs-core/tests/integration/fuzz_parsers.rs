//! Deterministic offline fuzz regression suite exercising all `cmvs-core::format` and
//! `cmvs-core::vm` binary parsers against structured header/payload mutations, truncations,
//! boundary integers, oversized header lengths, and pseudo-random byte streams.
use cmvs_core::{
    MAX_PAYLOAD,
    format::{
        cmv, cmv9, cpz, cursor, jbp1, jbpd, lzss, mgv, msk, mv2, pb2, pb3, ps3, save, snapshot,
        system_save,
    },
    render::raster::RgbaImage,
    vm::{bytecode, machine::Machine},
};
use std::{
    io::{self, Cursor, Read, Seek, SeekFrom},
    sync::Arc,
};

#[derive(Clone)]
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_usize(&mut self, upper_exclusive: usize) -> usize {
        if upper_exclusive <= 1 {
            0
        } else {
            (self.next_u64() % (upper_exclusive as u64)) as usize
        }
    }

    fn fill_bytes(&mut self, out: &mut [u8]) {
        let (chunks, tail) = out.as_chunks_mut::<8>();
        for chunk in chunks {
            *chunk = self.next_u64().to_le_bytes();
        }
        if !tail.is_empty() {
            let bytes = self.next_u64().to_le_bytes();
            tail.copy_from_slice(&bytes[..tail.len()]);
        }
    }
}

/// Sparse reader that reports an arbitrarily large virtual file size while only storing a
/// small prefix buffer, catching any reader that allocates from header fields > MAX_PAYLOAD
/// before validating bounds or hitting EOF.
struct SparseReader<'a> {
    prefix: &'a [u8],
    virtual_len: u64,
    pos: u64,
}

impl<'a> SparseReader<'a> {
    fn new(prefix: &'a [u8], virtual_len: u64) -> Self {
        Self {
            prefix,
            virtual_len: virtual_len.max(prefix.len() as u64),
            pos: 0,
        }
    }
}

impl Read for SparseReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Guard against unbounded single-read buffer allocations in tests.
        assert!(
            buf.len() <= MAX_PAYLOAD,
            "parser requested read buffer of {} bytes (> MAX_PAYLOAD)",
            buf.len()
        );
        if self.pos >= self.prefix.len() as u64 {
            return Ok(0);
        }
        let start = self.pos as usize;
        let available = (self.prefix.len() - start).min(buf.len());
        buf[..available].copy_from_slice(&self.prefix[start..start + available]);
        self.pos += available as u64;
        Ok(available)
    }
}

impl Seek for SparseReader<'_> {
    fn seek(&mut self, style: SeekFrom) -> io::Result<u64> {
        let next = match style {
            SeekFrom::Start(off) => i128::from(off),
            SeekFrom::End(delta) => i128::from(self.virtual_len) + i128::from(delta),
            SeekFrom::Current(delta) => i128::from(self.pos) + i128::from(delta),
        };
        if next < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "negative seek"));
        }
        self.pos = u64::try_from(next)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "seek overflow"))?;
        Ok(self.pos)
    }
}

fn hex_fixture(text: &str) -> Vec<u8> {
    text.trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn assert_bounded_image(image: &RgbaImage) {
    assert!(image.pixels().len() <= MAX_PAYLOAD);
    assert!(image.width() > 0 && image.height() > 0);
}

const BOUNDARY_U32: [u32; 10] = [
    0,
    1,
    24,
    32,
    u16::MAX as u32,
    (u16::MAX as u32) + 1,
    i32::MIN as u32,
    i32::MAX as u32,
    (MAX_PAYLOAD as u32) + 1,
    u32::MAX,
];

const BOUNDARY_U16: [u16; 6] = [0, 1, 8, 24, 32, u16::MAX];

fn mutate(rng: &mut SplitMix64, seed: &[u8]) -> Vec<u8> {
    let mut buf = seed.to_vec();
    if buf.is_empty() {
        buf.resize(rng.next_usize(96), 0);
        rng.fill_bytes(&mut buf);
        return buf;
    }
    let ops = 1 + rng.next_usize(4);
    for _ in 0..ops {
        match rng.next_usize(6) {
            // Truncate prefix
            0 => {
                let new_len = rng.next_usize(buf.len() + 1);
                buf.truncate(new_len);
            }
            // Bit flip
            1 if !buf.is_empty() => {
                let idx = rng.next_usize(buf.len());
                buf[idx] ^= 1u8 << (rng.next_u32() & 7);
            }
            // Splice boundary u32
            2 if buf.len() >= 4 => {
                let idx = rng.next_usize(buf.len() - 3);
                let val = BOUNDARY_U32[rng.next_usize(BOUNDARY_U32.len())];
                buf[idx..idx + 4].copy_from_slice(&val.to_le_bytes());
            }
            // Splice boundary u16
            3 if buf.len() >= 2 => {
                let idx = rng.next_usize(buf.len() - 1);
                let val = BOUNDARY_U16[rng.next_usize(BOUNDARY_U16.len())];
                buf[idx..idx + 2].copy_from_slice(&val.to_le_bytes());
            }
            // Extend with random tail (bounded to keep fuzz iterations ultra-fast)
            4 if buf.len() < 512 => {
                let extra = 1 + rng.next_usize(64);
                let old = buf.len();
                buf.resize(old + extra, 0);
                rng.fill_bytes(&mut buf[old..]);
            }
            // Overwrite random byte
            _ if !buf.is_empty() => {
                let idx = rng.next_usize(buf.len());
                buf[idx] = rng.next_u32() as u8;
            }
            _ => {}
        }
    }
    buf
}

fn feed_all_decoders(bytes: &[u8], query_rng: &mut SplitMix64) {
    // 1. PS2 / PS3 / PSS0 script & memory-image decoders + VM bytecode layout & bounded step
    let _ = ps3::Header::parse(bytes);
    if let Ok(script) = ps3::Script::decode(bytes) {
        assert!(script.payload().len() <= MAX_PAYLOAD);
        let _ = script.strings();
        let labels = script.labels();
        let _ = bytecode::decode_layout(script.code(), script.header().entry_pc, &labels);
        if let Ok(mut vm) = Machine::new(script) {
            for _ in 0..8 {
                if vm.step().is_err() || vm.pending_system().is_some() {
                    break;
                }
            }
        }
    }
    if let Ok(script) = ps3::Script::from_memory_image(bytes) {
        assert!(script.payload().len() <= MAX_PAYLOAD);
        let _ = script.strings();
        let labels = script.labels();
        let _ = bytecode::decode_layout(script.code(), script.header().entry_pc, &labels);
    }

    // 2. Raw VM bytecode decoder
    let _ = bytecode::decode_at(bytes, 0);
    let _ = bytecode::decode_layout(bytes, 0, &[0]);

    // 3. CPZ1/2/3/5/6/7 layout & archive index readers
    let _ = cpz::ArchiveVersion::detect(bytes);
    let _ = cpz::ArchiveLayout::read(&mut Cursor::new(bytes));
    if let Ok(index) = cpz::ArchiveIndex::read(&mut Cursor::new(bytes)) {
        let mut query = vec![0; query_rng.next_usize(257)];
        query_rng.fill_bytes(&mut query);
        let _ = index.lookup(b"fixture.bin");
        let _ = index.find(b"fixture.bin");
        let _ = index.lookup(&query);
        let _ = index.find(&query);
        if !index.entries().is_empty() {
            if let Ok(entry_bytes) = index.read_entry(&mut Cursor::new(bytes), 0) {
                assert!(entry_bytes.len() <= MAX_PAYLOAD);
            }
            if let Ok(entry_bytes) = index.read_entry_verified(&mut Cursor::new(bytes), 0) {
                assert!(entry_bytes.len() <= MAX_PAYLOAD);
            }
        }
    }

    // 4. PB2 & PB3 image containers
    let _ = pb2::Header::parse(bytes);
    let _ = pb2::part_names(bytes);
    if let Ok(img) = pb2::decode(bytes) {
        assert_bounded_image(&img);
    }
    let _ = pb3::Header::parse(bytes);
    let _ = pb3::reference_name(bytes);
    if let Ok(img) = pb3::decode(bytes) {
        assert_bounded_image(&img);
    }

    // 5. JBP1 & JBPD / JBPE codecs
    if let Ok(jbp) = jbp1::decode(bytes) {
        assert!(jbp.rgba.len() <= MAX_PAYLOAD);
    }
    if let Ok(img) = jbpd::decode(bytes) {
        assert_bounded_image(&img);
    }
    if let Ok(img) = jbpd::decode_alpha(bytes) {
        assert_bounded_image(&img);
    }

    // 6. LZSS codec across boundary expected lengths
    for expected in [0, 1, bytes.len(), 256, MAX_PAYLOAD + 1, usize::MAX] {
        if let Ok(out) = lzss::decode(bytes, expected) {
            assert!(out.len() <= MAX_PAYLOAD);
        }
    }

    // 7. MSK mask & MV2 audio
    let _ = msk::Header::parse(bytes);
    if let Ok((_, _, mask)) = msk::decode_bytes(bytes) {
        assert!(mask.len() <= MAX_PAYLOAD);
    }
    if let Ok(img) = msk::decode(bytes) {
        assert_bounded_image(&img);
    }
    let _ = mv2::Header::parse(bytes);
    if let Ok((_, pcm)) = mv2::decode(Arc::from(bytes)) {
        assert!(pcm.len() * 2 <= MAX_PAYLOAD);
    }

    // 8. CMV / CMV9 / MGV video containers
    if let Ok(idx) = cmv::CmvIndex::read(&mut Cursor::new(bytes))
        && let Ok(img) = idx.decode_frame(&mut Cursor::new(bytes), 0)
    {
        assert_bounded_image(&img);
    }
    if let Ok(idx) = cmv::CmvIndex::read_scene(&mut Cursor::new(bytes))
        && let Ok(img) = idx.decode_frame(&mut Cursor::new(bytes), 0)
    {
        assert_bounded_image(&img);
    }
    if let Ok(idx) = cmv9::Cmv9Index::parse(bytes)
        && let Ok(img) = idx.decode_frame(bytes, 0)
    {
        assert_bounded_image(&img);
    }
    let _ = mgv::MgvIndex::read(&mut Cursor::new(bytes));

    // 9. Windows CUR / ICO cursor bitmaps
    if let Ok(cur) = cursor::CursorBitmap::decode(bytes)
        && let Ok(rgba) = cur.rgba()
    {
        assert!(rgba.len() <= MAX_PAYLOAD);
    }

    // 10. CSV2 / CNS0 gameplay saves, CSS1 system saves, and Engine/CVNS snapshots
    let _ = save::SaveHeader::parse(bytes);
    let _ = save::Csv2Layout::parse(bytes, bytes.len() as u64);
    let _ = save::Csv2Save::decode(bytes);
    if let Ok((_, decoded)) = save::Csv2Save::decode_any(bytes) {
        let _ = snapshot::VmSnapshot::decode(&decoded);
        let _ = snapshot::VisualSnapshot::decode(&decoded);
    }
    let _ = system_save::Css1Layout::parse(bytes, bytes.len() as u64);
    let _ = system_save::Css1Save::decode(bytes);

    let _ = snapshot::parse_records(bytes);
    let _ = snapshot::EngineSnapshot::decode(bytes);
    let _ = snapshot::CvnsSnapshot::has_cmvs_rs_records(bytes);
    let _ = snapshot::CvnsSnapshot::decode(bytes);
    let _ = snapshot::VisualSnapshot::decode_records(bytes);
    let _ = snapshot::SavedEffect::decode(bytes);
    let _ = snapshot::LocalVariables::decode(bytes);
}

fn seed_corpus() -> Vec<Vec<u8>> {
    let mut seeds = Vec::new();

    // Valid PS2A script & memory image
    let mut ps2_mem = b"PS2A".to_vec();
    let code = [0x40u8, 0x04, 0x04, 0x00, 0x2a, 0x00, 0x00, 0x00, 0x02, 0x00];
    let payload = [0u32.to_le_bytes().as_slice(), &code, b"hello\0"].concat();
    for word in [
        48u32,
        0,
        0,
        1,
        code.len() as u32,
        0,
        6,
        0,
        0,
        payload.len() as u32,
        0,
    ] {
        ps2_mem.extend(word.to_le_bytes());
    }
    let mut ps2_disk = ps2_mem.clone();
    ps2_mem.extend_from_slice(&payload);
    ps2_disk.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    seeds.push(ps2_mem);
    seeds.push(ps2_disk);

    // Valid PB3 encoded image + LZSS stream
    let alpha_img = RgbaImage::new(4, 4, vec![0xaa; 64]).unwrap();
    let opaque_img = RgbaImage::new(4, 4, [0x22u8, 0x66, 0xaa, 0xff].repeat(16)).unwrap();
    seeds.push(pb3::encode(&alpha_img, pb3::EncodeDepth::Rgba32).unwrap());
    seeds.push(pb3::encode(&opaque_img, pb3::EncodeDepth::Rgb24).unwrap());
    seeds.push(lzss::encode(
        b"CMVS repeated LZSS pattern CMVS repeated LZSS pattern!",
    ));

    // Valid CPZ3/5/6/7 fixtures + synthetic CMV6 & CMV9
    for hex in [
        include_str!("../fixtures/cpz3-0.hex"),
        include_str!("../fixtures/cpz5-9.hex"),
        include_str!("../fixtures/cpz6-0.hex"),
        include_str!("../fixtures/cpz7-0.hex"),
    ] {
        seeds.push(hex_fixture(hex));
    }
    seeds.push(crate::support::cmv9::cmv6(2, false));
    seeds.push(crate::support::cmv9::movie(false));

    // Magic-prefixed header skeletons for every remaining container signature
    for magic in [
        b"PS3A".as_slice(),
        b"PSS0",
        b"CPZ1",
        b"CPZ2",
        b"CPZ3",
        b"CPZ5",
        b"CPZ6",
        b"CPZ7",
        b"PB2A",
        b"PB3B",
        b"JBP1",
        b"JBPD",
        b"JBPE",
        b"MSK0",
        b"MV2X",
        b"CMV3",
        b"CMV4",
        b"CMV6",
        b"CMV8",
        b"CMV9",
        b"MGV0",
        b"MGV1",
        b"CSV2",
        b"CNS0",
        b"CSS1",
        b"CSS0",
        b"\x00\x00\x01\x00",
        b"\x00\x00\x02\x00",
    ] {
        let mut skeleton = vec![0u8; 96];
        skeleton[..magic.len()].copy_from_slice(magic);
        seeds.push(skeleton);
    }

    // Snapshot records (EngineSnapshot & CvnsSnapshot framing)
    let mut snap = Vec::new();
    for (tag, body) in [
        (0x0100u16, &[1u8, 0, 0, 0][..]),
        (0x0700u16, &[0u8; 16][..]),
        (0x5253u16, b"CVNS\x01\x00\x00\x00".as_slice()),
    ] {
        snap.extend(tag.to_le_bytes());
        snap.extend((body.len() as u32).to_le_bytes());
        snap.extend_from_slice(body);
    }
    seeds.push(snap);

    seeds
}

#[test]
fn structured_mutations_and_random_inputs_never_panic_or_exceed_max_payload() {
    let seeds = seed_corpus();
    let mut query_rng = SplitMix64::new(0x0063_707a);
    for seed in &seeds {
        feed_all_decoders(seed, &mut query_rng);
        // Deterministic prefix truncations along header boundaries.
        for cut in [0, 1, 3, 4, 8, 15, 16, 20, 32, 44, 48, 63, 64, 72] {
            if cut < seed.len() {
                feed_all_decoders(&seed[..cut], &mut query_rng);
            }
        }
    }

    let mut rng = SplitMix64::new(0x5eed_c0de_f00d_2026);
    // Structured mutations over every seed
    for seed in &seeds {
        for _ in 0..32 {
            let mutated = mutate(&mut rng, seed);
            feed_all_decoders(&mutated, &mut query_rng);
        }
    }

    // Pure pseudo-random byte slices across varied sizes
    for _ in 0..256 {
        let len = rng.next_usize(257);
        let mut random = vec![0u8; len];
        rng.fill_bytes(&mut random);
        feed_all_decoders(&random, &mut query_rng);
    }
}

#[test]
fn oversized_headers_on_sparse_streams_reject_before_unbounded_allocation() {
    let huge_file = (MAX_PAYLOAD as u64) * 4 + 4096;
    let over = (MAX_PAYLOAD as u32) + 64;

    // CPZ1: [magic, file_count, index_bytes, 0]
    let mut cpz1 = [0u8; 16];
    cpz1[..4].copy_from_slice(b"CPZ1");
    cpz1[4..8].copy_from_slice(&1u32.to_le_bytes());
    cpz1[8..12].copy_from_slice(&over.to_le_bytes());
    assert!(cpz::ArchiveIndex::read(&mut SparseReader::new(&cpz1, huge_file)).is_err());

    // CPZ2: masked file_count & index_bytes
    let mut cpz2 = [0u8; 20];
    cpz2[..4].copy_from_slice(b"CPZ2");
    cpz2[4..8].copy_from_slice(&(1u32 ^ 0xe47c_59f3).to_le_bytes());
    cpz2[8..12].copy_from_slice(&(over ^ 0x3f71_de2a).to_le_bytes());
    assert!(cpz::ArchiveIndex::read(&mut SparseReader::new(&cpz2, huge_file)).is_err());

    // CPZ3: masked file_count & index_bytes
    let mut cpz3 = [0u8; 20];
    cpz3[..4].copy_from_slice(b"CPZ3");
    cpz3[4..8].copy_from_slice(&(1u32 ^ 0x5e9c_4f37).to_le_bytes());
    cpz3[8..12].copy_from_slice(&(over ^ 0xf32a_ed17).to_le_bytes());
    assert!(cpz::ArchiveIndex::read(&mut SparseReader::new(&cpz3, huge_file)).is_err());

    // CPZ5: authenticated header with index_bytes > MAX_PAYLOAD
    let mut cpz5 = hex_fixture(include_str!("../fixtures/cpz5-9.hex"));
    cpz5[8..12].copy_from_slice(&(over ^ 0x37f2_98e7).to_le_bytes());
    let sum5 = cpz::data_checksum(&cpz5[..60])
        .wrapping_sub(0x5a90_2b7c)
        .wrapping_sub(0x6dc5_a9b4);
    cpz5[60..64].copy_from_slice(&sum5.to_le_bytes());
    assert!(cpz::ArchiveIndex::read(&mut SparseReader::new(&cpz5[..64], huge_file)).is_err());

    // CPZ6: directory_index_bytes > MAX_PAYLOAD
    let mut cpz6 = hex_fixture(include_str!("../fixtures/cpz6-0.hex"));
    cpz6[8..12].copy_from_slice(&(over ^ 0x37f2_98e8).to_le_bytes());
    assert!(cpz::ArchiveIndex::read(&mut SparseReader::new(&cpz6[..64], huge_file)).is_err());

    // CPZ7: authenticated header with directory_index_bytes > MAX_PAYLOAD
    let mut cpz7 = hex_fixture(include_str!("../fixtures/cpz7-0.hex"));
    cpz7[8..12].copy_from_slice(&(over ^ 0x37f2_98e8).to_le_bytes());
    let init7 = u32::from_le_bytes(cpz7[64..68].try_into().unwrap()).wrapping_sub(0x6dc5_a9b4);
    let sum7 = cpz::data_checksum(&cpz7[..64])
        .wrapping_sub(0x5a90_2b7c)
        .wrapping_add(init7);
    cpz7[68..72].copy_from_slice(&sum7.to_le_bytes());
    assert!(cpz::ArchiveIndex::read(&mut SparseReader::new(&cpz7[..72], huge_file)).is_err());

    // CMV6 & MGV0 with oversized sequence/frame counts on sparse streams
    let mut cmv6 = [0u8; 44];
    cmv6[..4].copy_from_slice(b"CMV6");
    cmv6[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    cmv6[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    cmv6[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(cmv::CmvIndex::read(&mut SparseReader::new(&cmv6, u64::MAX)).is_err());

    let mut mgv0 = [0u8; 64];
    mgv0[..4].copy_from_slice(b"MGV1");
    mgv0[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(mgv::MgvIndex::read(&mut SparseReader::new(&mgv0, u64::MAX)).is_err());
}
