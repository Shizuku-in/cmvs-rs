use cmvs_rs::ps3::Script;
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let Some(input) = args.next() else {
        return Err("usage: cmvs-inspect <script> [--payload <new-output-file>]".into());
    };
    if input == OsStr::new("--help") || input == OsStr::new("-h") {
        println!(
            "Usage: cmvs-inspect <script> [--payload <new-output-file>]\nRead-only PS2A inspection. Text displayed as hex; no VM execution."
        );
        return Ok(());
    }
    let output = match args.next() {
        Some(flag) if flag == OsStr::new("--payload") => {
            Some(PathBuf::from(args.next().ok_or("missing payload path")?))
        }
        Some(_) => return Err("unknown option".into()),
        None => None,
    };
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let max_file = cmvs_rs::MAX_PAYLOAD as u64 + 48;
    let mut raw = Vec::new();
    fs::File::open(PathBuf::from(input))?
        .take(max_file + 1)
        .read_to_end(&mut raw)?;
    if raw.len() as u64 > max_file {
        return Err("input exceeds research allocation limit".into());
    }
    let script = Script::decode(&raw)?;
    let strings = script.strings()?;
    if let Some(path) = output {
        // create_new rejects existing files, including the input and symlinks to it.
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(script.payload())?;
    }
    println!(
        "PS2A: {} decoded bytes, {} labels, {} code bytes, {} text bytes, {} strings",
        script.payload().len(),
        script.header.label_count,
        script.code().len(),
        script.text().len(),
        strings.len()
    );
    println!(
        "entry_pc=0x{:x}, trailing_file_bytes={}",
        script.header.entry_pc, script.trailing_file_bytes
    );
    for entry in strings {
        print!("text+0x{:04x}: ", entry.offset);
        for byte in entry.bytes {
            print!("{byte:02x}");
        }
        println!();
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
