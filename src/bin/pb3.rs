use cmvs_rs::{MAX_PAYLOAD, pb3};
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
    if args.len() != 1 && !(args.len() == 3 && args[1] == "--rgba") {
        return Err("wrong argument count; use --help".into());
    }
    let mut bytes = Vec::new();
    File::open(&args[0])?
        .take(MAX_PAYLOAD as u64 + 1)
        .read_to_end(&mut bytes)?;
    let header = pb3::Header::parse(&bytes)?;
    println!("PB3B {header:?}");
    if args.len() == 3 {
        let image = pb3::decode(&bytes)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[2])?;
        output.write_all(image.pixels())?;
        println!(
            "Wrote {} RGBA bytes ({}x{}) to {:?}",
            image.pixels().len(),
            image.width(),
            image.height(),
            args[2]
        );
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-pb3 --help' for usage.");
        std::process::exit(1)
    }
}
