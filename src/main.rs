use cmvs_rs::ps3::Script;
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

const USAGE: &str = "\
Usage: cmvs-inspect <script> [--payload <new-file>]

Inspect a PS2A script (.ps2/.ps3) without executing the VM.
Print header, segment sizes and strings as hexadecimal bytes.

Options:
  --payload <new-file>  export the decoded payload without its header
  -h, --help            show this help

Output files must not already exist. Input files are read only.
Example: cmvs-inspect data/pack/start.ps3";

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let Some(input) = args.next() else {
        return Err("usage: cmvs-inspect <script> [--payload <new-output-file>]".into());
    };
    if input == OsStr::new("--help") || input == OsStr::new("-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if input.to_string_lossy().starts_with('-') {
        return Err(format!("unknown option {}", input.to_string_lossy()).into());
    }
    let output = match args.next() {
        Some(flag) if flag == OsStr::new("--payload") => {
            Some(PathBuf::from(args.next().ok_or("missing payload path")?))
        }
        Some(flag) if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            return Ok(());
        }
        Some(flag) => return Err(format!("unknown option {}", flag.to_string_lossy()).into()),
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
        eprintln!("Error: {error}\nRun 'cmvs-inspect --help' for usage.");
        std::process::exit(1);
    }
}
