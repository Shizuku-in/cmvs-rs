use cmvs_rs::{MAX_PAYLOAD, pb3};
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
            "Usage: cmvs-pb3 <file.pb3> [--rgba <new-output-file>]\nMetadata mode only parses the header. RGBA export supports PB3B type 1, 24/32-bit. Top-down straight RGBA; no VM draw commands or checksum verification."
        );
        return Ok(());
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
        eprintln!("Error: {error}");
        std::process::exit(1)
    }
}
