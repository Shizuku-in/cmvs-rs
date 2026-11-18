use cmvs_core::{MAX_PAYLOAD, format::pb3};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Write},
};

const USAGE: &str = "\
Usage: cmvs-pb3 <file.pb3> [--rgba <new-file>]

Inspect a PB3B header. Without --rgba, image pixels are not decoded.
RGBA export supports standalone types 1/2/3/5; no VM draw commands are executed.

Options:
  --rgba <new-file>  export top-down straight RGBA bytes without a header
  -h, --help         show this help

Output files must not already exist. Resource checksums are not verified.
Example: cmvs-pb3 background.pb3 --rgba background.rgba";

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if cmvs_cli::wants_help(&args) {
        println!("{USAGE}");
        return Ok(());
    }
    let mut input: Option<std::ffi::OsString> = None;
    let mut rgba_out: Option<std::ffi::OsString> = None;
    let mut positional_only = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if !positional_only && arg == "--" {
            positional_only = true;
            continue;
        }
        if !positional_only && arg == "--rgba" {
            if rgba_out.is_some() {
                return Err("duplicate --rgba option".into());
            }
            rgba_out = Some(iter.next().ok_or("wrong argument count; use --help")?);
            continue;
        }
        if !positional_only && arg.to_string_lossy().starts_with('-') {
            return Err(format!("unknown option {}", arg.to_string_lossy()).into());
        }
        if input.is_some() {
            return Err("wrong argument count; use --help".into());
        }
        input = Some(arg);
    }
    let input = input.ok_or("wrong argument count; use --help")?;
    let mut bytes = Vec::new();
    File::open(&input)?
        .take(MAX_PAYLOAD as u64 + 1)
        .read_to_end(&mut bytes)?;
    let header = pb3::Header::parse(&bytes)?;
    println!("PB3B {header:?}");
    if let Some(rgba_out) = rgba_out {
        let image = pb3::decode(&bytes)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&rgba_out)?;
        output.write_all(image.pixels())?;
        println!(
            "Wrote {} RGBA bytes ({}x{}) to {:?}",
            image.pixels().len(),
            image.width(),
            image.height(),
            rgba_out
        );
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-pb3 --help' for usage.");
        std::process::exit(1);
    }
}
