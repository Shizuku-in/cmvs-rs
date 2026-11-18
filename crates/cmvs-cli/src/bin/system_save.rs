use cmvs_core::{
    format::system_save::Css1Save, format::system_save::FileSystemSave,
    format::system_save::SystemSaveSource,
};
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

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if cmvs_cli::wants_help(&args) {
        println!("{USAGE}");
        return Ok(());
    }
    let mut input: Option<std::ffi::OsString> = None;
    let mut payload_out: Option<std::ffi::OsString> = None;
    let mut positional_only = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if !positional_only && arg == "--" {
            positional_only = true;
            continue;
        }
        if !positional_only && arg == "--payload" {
            if payload_out.is_some() {
                return Err("duplicate --payload option".into());
            }
            payload_out = Some(iter.next().ok_or("wrong argument count; use --help")?);
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
    let raw = FileSystemSave::new(&input)
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
    if let Some(payload_out) = payload_out {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&payload_out)?;
        file.write_all(save.payload())?;
        println!(
            "Wrote {} decoded bytes to {:?}",
            save.payload().len(),
            payload_out
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
