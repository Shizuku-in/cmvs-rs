use cmvs_rs::{cursor::CursorBitmap, platform::MAX_PLATFORM_ASSET};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Write},
};
const USAGE: &str = "\
Usage: cmvs-cursor <file.cur> [--planes <new-file> | --rgba <new-file>]

Inspect a single-image, 8-bit or 32-bit BI_RGB Windows cursor.
Without export options, print metadata only. Does not display a native cursor.

Options (choose at most one export):
  --planes <new-file>  export top-down AND/R/G/B bytes per pixel (no alpha)
                      AND is 0 or 255
  --rgba <new-file>    export top-down straight RGBA; rejects legacy inversion
  -h, --help           show this help

Exports are raw pixels without a header. Output files must not already exist.
Example: cmvs-cursor 01.cur --rgba cursor.rgba";

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if let Some(flag) = args.first()
        && flag.to_string_lossy().starts_with('-')
    {
        return Err(format!("unknown option {}", flag.to_string_lossy()).into());
    }
    if args.len() != 1 && !(args.len() == 3 && (args[1] == "--planes" || args[1] == "--rgba")) {
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
        "CUR width={} height={} hotspot={:?} pixels={}; alpha={}",
        cursor.width(),
        cursor.height(),
        cursor.hotspot(),
        cursor.xor_rgb().len(),
        cursor.alpha().is_some()
    );
    if args.len() == 3 {
        let rgba = if args[1] == "--rgba" {
            Some(cursor.rgba()?)
        } else {
            None
        };
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[2])?;
        if let Some(rgba) = rgba {
            out.write_all(&rgba)?;
        } else {
            for (rgb, and) in cursor.xor_rgb().iter().zip(cursor.and_mask()) {
                out.write_all(&[if *and { 255 } else { 0 }, rgb[0], rgb[1], rgb[2]])?;
            }
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-cursor --help' for usage.");
        std::process::exit(1)
    }
}
