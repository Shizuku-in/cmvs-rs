use cmvs_core::{format::cursor::CursorBitmap, host::platform::MAX_PLATFORM_ASSET};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Write},
};

const USAGE: &str = "\
Usage: cmvs-cursor <file.cur> [--planes <new-file> | --rgba <new-file>]

Inspect a single-image, 8-bit, 24-bit or 32-bit BI_RGB Windows cursor.
Without export options, print metadata only. Does not display a native cursor.

Options (choose at most one export):
  --planes <new-file>  export top-down AND/R/G/B bytes per pixel (no alpha)
                      AND is 0 or 255
  --rgba <new-file>    export top-down straight RGBA; rejects legacy inversion
  -h, --help           show this help

Exports are raw pixels without a header. Output files must not already exist.
Example: cmvs-cursor 01.cur --rgba cursor.rgba";

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if cmvs_cli::wants_help(&args) {
        println!("{USAGE}");
        return Ok(());
    }
    enum OutputMode {
        Rgba(std::ffi::OsString),
        Planes(std::ffi::OsString),
    }
    let mut input: Option<std::ffi::OsString> = None;
    let mut output: Option<OutputMode> = None;
    let mut positional_only = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if !positional_only && arg == "--" {
            positional_only = true;
            continue;
        }
        if !positional_only && (arg == "--rgba" || arg == "--planes") {
            if output.is_some() {
                return Err("wrong arguments; use --help".into());
            }
            let out_path = iter.next().ok_or("wrong arguments; use --help")?;
            output = Some(if arg == "--rgba" {
                OutputMode::Rgba(out_path)
            } else {
                OutputMode::Planes(out_path)
            });
            continue;
        }
        if !positional_only && arg.to_string_lossy().starts_with('-') {
            return Err(format!("unknown option {}", arg.to_string_lossy()).into());
        }
        if input.is_some() {
            return Err("wrong arguments; use --help".into());
        }
        input = Some(arg);
    }
    let input = input.ok_or("wrong arguments; use --help")?;
    let mut raw = Vec::new();
    File::open(&input)?
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
    if let Some(output) = output {
        let (out_path, rgba) = match output {
            OutputMode::Rgba(out_path) => (out_path, Some(cursor.rgba()?)),
            OutputMode::Planes(out_path) => (out_path, None),
        };
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out_path)?;
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
        std::process::exit(1);
    }
}
