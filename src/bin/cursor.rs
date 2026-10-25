use cmvs_rs::{cursor::CursorBitmap, platform::MAX_PLATFORM_ASSET};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Write},
};
fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "Usage: cmvs-cursor <file.cur> [--planes <new-output-file>]\nSingle-image 8-bit BI_RGB CUR only. Raw planes are top-down 4-byte pixels: AND (0 or 255), XOR red, green, blue. Not RGBA. No native cursor display."
        );
        return Ok(());
    }
    if args.len() != 1 && !(args.len() == 3 && args[1] == "--planes") {
        return Err("wrong arguments; use --help".into());
    }
    let mut raw = Vec::new();
    File::open(&args[0])?
        .take(MAX_PLATFORM_ASSET as u64 + 1)
        .read_to_end(&mut raw)?;
    if raw.len() > MAX_PLATFORM_ASSET {
        return Err("cursor file exceeds research limit".into());
    }
    let cursor = CursorBitmap::decode(&raw)?;
    println!(
        "CUR width={} height={} hotspot={:?} pixels={}; AND/XOR planes",
        cursor.width(),
        cursor.height(),
        cursor.hotspot(),
        cursor.xor_rgb().len()
    );
    if args.len() == 3 {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[2])?;
        for (rgb, and) in cursor.xor_rgb().iter().zip(cursor.and_mask()) {
            out.write_all(&[if *and { 255 } else { 0 }, rgb[0], rgb[1], rgb[2]])?;
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1)
    }
}
