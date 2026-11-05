//! Temporary on-device std/core probe. No writes to game data.
use std::{fs, io, time::Instant};

fn probe() -> io::Result<()> {
    let start = Instant::now();
    let root = "sdmc:/switch/cmvs-rs";
    if !fs::metadata(root)?.is_dir() {
        return Err(io::Error::other("probe directory is not a directory"));
    }
    let mut count = 0;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        println!("entry: {:?}", entry.file_name());
        count += 1;
        if count == 16 {
            break;
        }
    }
    let executable = format!("{root}/cmvs-switch-smoke.nro");
    let metadata = fs::metadata(&executable)?;
    let mut file = fs::File::open(&executable)?;
    let mut header = [0u8; 20];
    std::io::Read::read_exact(&mut file, &mut header)?;
    if !metadata.is_file() || metadata.len() < 20 || &header[16..20] != b"NRO0" {
        return Err(io::Error::other("NRO header or metadata mismatch"));
    }
    println!("NRO read OK: {} bytes", metadata.len());
    let mut hash = cmvs_core::digest::Sha256::new();
    hash.update(b"abc");
    if hash.finish()
        != [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ]
    {
        return Err(io::Error::other("core SHA-256 mismatch"));
    }
    println!(
        "core SHA-256 OK; directory entries: {count}; elapsed: {:?}",
        start.elapsed()
    );
    Ok(())
}

// SAFETY: exports a unique, pointer-free C ABI entry; panic strategy is abort.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn cmvs_switch_probe() -> i32 {
    match probe() {
        Ok(()) => 0,
        Err(error) => {
            println!("Rust probe failed: {error}");
            1
        }
    }
}

fn inspect_game(root: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let game = cmvs_core::profile::Registry::builtin().identify_game(root, None)?;
    let record = game
        .selection
        .record
        .ok_or("game has no registered identity")?;
    println!(
        "Game: {}\nProfile: {}",
        record.game, game.selection.profile.id
    );
    let script = cmvs_core::runner::load_start(root, &game.files)?;
    println!("Startup script decoded: {} code bytes", script.code().len());
    let startup = cmvs_core::runner::Startup::new(&script, root, &game, 0)?;
    println!("VM initialized; canvas: {:?}", startup.screen_size());
    println!("Inspection complete. Playback is not connected yet.");
    Ok(())
}

// SAFETY: unique pointer-free C ABI entry, using abort rather than unwinding.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn cmvs_switch_inspect_game() -> i32 {
    match inspect_game(std::path::Path::new("sdmc:/switch/cmvs-rs/game")) {
        Ok(()) => 0,
        Err(error) => {
            println!("Game initialization failed: {error}");
            1
        }
    }
}
