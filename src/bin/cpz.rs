use cmvs_rs::{
    cpz::{CPZ6_HEADER_SIZE, Cpz6Index, Cpz6Layout},
    ps3::Script,
};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Write},
};

fn run() -> Result<(), Box<dyn Error>> {
    let paths: Vec<_> = env::args_os().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: cmvs-cpz <archive> [archive ...]".into());
    }
    if paths.len() == 1 && paths[0] == "--help" {
        println!(
            "Usage: cmvs-cpz <archive> [archive ...]\n       cmvs-cpz --list <archive>\n       cmvs-cpz --script <archive> <ASCII-basename>\n       cmvs-cpz --extract <archive> <ASCII-basename> <new-output-file>\nMetadata mode reads 64 bytes. Index/resource modes decrypt CPZ6 mode 1; checksums NOT verified."
        );
        return Ok(());
    }
    if paths[0] == "--list" || paths[0] == "--script" || paths[0] == "--extract" {
        let expected = if paths[0] == "--list" {
            2
        } else if paths[0] == "--script" {
            3
        } else {
            4
        };
        if paths.len() != expected {
            return Err("wrong argument count; use --help".into());
        }
        let mut file = File::open(&paths[1])?;
        let index = Cpz6Index::read(&mut file)?;
        if paths[0] == "--list" {
            for (i, entry) in index.entries().iter().enumerate() {
                println!(
                    "{i}: {}/{} size={} offset={} checksum=0x{:08x}",
                    entry.directory.escape_ascii(),
                    entry.name.escape_ascii(),
                    entry.size,
                    entry.offset,
                    entry.checksum_raw
                );
            }
            println!(
                "# {} files; stored checksums listed, NOT verified",
                index.entries().len()
            );
        } else {
            let name = paths[2]
                .to_str()
                .filter(|name| name.is_ascii())
                .ok_or("CLI basename must be ASCII; use library byte names for other encodings")?;
            let entry = index.find(name.as_bytes())?;
            let bytes = index.read_entry(&mut file, entry)?;
            if paths[0] == "--extract" {
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&paths[3])?;
                output.write_all(&bytes)?;
                println!(
                    "Decoded {} bytes to {:?}; checksums NOT verified",
                    bytes.len(),
                    paths[3]
                );
            } else {
                let script = Script::decode(&bytes)?;
                println!(
                    "{name}: PS2A payload={} code={} labels={} strings={} entry_pc=0x{:x}",
                    script.payload().len(),
                    script.code().len(),
                    script.header.label_count,
                    script.strings()?.len(),
                    script.header.entry_pc
                );
            }
        }
        return Ok(());
    }
    for path in paths {
        let mut file = File::open(&path)?;
        let length = file.metadata()?.len();
        let mut header = [0; CPZ6_HEADER_SIZE];
        file.read_exact(&mut header)?;
        let layout = Cpz6Layout::parse(&header, length)?;
        println!("{path:?}: {layout:?}");
    }
    println!("# Metadata only; directory/file indexes and checksums NOT verified");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
