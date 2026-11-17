use cmvs_core::{MAX_PAYLOAD, format::ps3::Script};
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

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let raw_args: Vec<_> = env::args_os().skip(1).collect();
    if cmvs_cli::wants_help(&raw_args) {
        println!("{USAGE}");
        return Ok(());
    }
    if raw_args.is_empty() {
        return Err("usage: cmvs-inspect <script> [--payload <new-output-file>]".into());
    }
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut positional_only = false;
    let mut args = raw_args.into_iter();
    while let Some(arg) = args.next() {
        if !positional_only && arg == OsStr::new("--") {
            positional_only = true;
            continue;
        }
        if !positional_only && arg == OsStr::new("--payload") {
            if output.is_some() {
                return Err("duplicate --payload option".into());
            }
            output = Some(PathBuf::from(args.next().ok_or("missing payload path")?));
            continue;
        }
        if !positional_only && arg.to_string_lossy().starts_with('-') {
            return Err(format!("unknown option {}", arg.to_string_lossy()).into());
        }
        if input.is_some() {
            return Err("too many arguments".into());
        }
        input = Some(PathBuf::from(arg));
    }
    let input = input.ok_or("usage: cmvs-inspect <script> [--payload <new-output-file>]")?;
    let max_file = MAX_PAYLOAD as u64 + 48;
    let mut raw = Vec::new();
    fs::File::open(input)?
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
        script.header().label_count,
        script.code().len(),
        script.text().len(),
        strings.len()
    );
    println!(
        "entry_pc=0x{:x}, trailing_file_bytes={}",
        script.header().entry_pc,
        script.trailing_file_bytes
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
