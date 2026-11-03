use cmvs_rs::system_save::{Css1Save, FileSystemSave, SystemSaveSource};
use std::{env, error::Error, fs::OpenOptions, io::Write};

const USAGE: &str = "\
Usage: cmvs-system-save <system.dat> [--payload <new-file>]

Decode a CSS1 system save and print its layout and persistent variable ranges.
The original checksum covers only the compressed payload.

Options:
  --payload <new-file>  export the decoded payload
  -h, --help            show this help

Output files must not already exist. Does not restore engine state or modify saves.
Example: cmvs-system-save save/system.dat";

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
    if args.len() != 1 && !(args.len() == 3 && args[1] == "--payload") {
        return Err("wrong argument count; use --help".into());
    }
    let raw = FileSystemSave::new(&args[0])
        .read()?
        .ok_or("system save does not exist")?;
    let save = Css1Save::decode(&raw)?;
    println!(
        "CSS1 game_id=0x{:08x} packed={} decoded={} lists={:?}; payload checksum verified",
        save.layout().game_id,
        save.layout().packed_size,
        save.payload().len(),
        save.layout().list_counts
    );
    println!(
        "persistent bits=2048..262144 integers=2048..4096 floats=1024..2048 strings=64..128 (exclusive ends)"
    );
    if args.len() == 3 {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[2])?;
        file.write_all(save.payload())?;
        println!(
            "Wrote {} decoded bytes to {:?}",
            save.payload().len(),
            args[2]
        );
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-system-save --help' for usage.");
        std::process::exit(1);
    }
}
