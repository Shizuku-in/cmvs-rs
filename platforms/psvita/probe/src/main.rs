//! On-device toolchain probe: std file, thread and clock services plus core SHA-256.
use std::{fmt::Write as _, fs, time::Instant};

const BASE: &str = "ux0:data/cmvs-rs";

fn probe(log: &mut String) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    fs::create_dir_all(BASE)?;
    for entry in fs::read_dir(BASE)?.take(16) {
        writeln!(log, "entry: {:?}", entry?.file_name())?;
    }
    let worker = std::thread::spawn(|| (1..=1000u64).sum::<u64>());
    writeln!(
        log,
        "thread: {}",
        worker.join().map_err(|_| "thread panicked")?
    )?;
    let mut hash = cmvs_core::digest::Sha256::new();
    hash.update(b"abc");
    let digest = hash.finish();
    if digest[..4] != [0xba, 0x78, 0x16, 0xbf] {
        return Err("core SHA-256 mismatch".into());
    }
    let heap = vec![1u8; 64 << 20];
    writeln!(
        log,
        "64 MiB heap: {}",
        heap.iter().map(|&b| u64::from(b)).sum::<u64>()
    )?;
    writeln!(log, "core SHA-256 OK; elapsed {:?}", start.elapsed())?;
    Ok(())
}

fn main() {
    let mut log = String::new();
    if let Err(error) = probe(&mut log) {
        let _ = writeln!(log, "probe failed: {error}");
    }
    let _ = fs::write(format!("{BASE}/probe.log"), log);
}
