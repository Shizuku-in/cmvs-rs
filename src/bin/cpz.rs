use cmvs_rs::cpz::{CPZ6_HEADER_SIZE, Cpz6Layout};
use std::{env, error::Error, fs::File, io::Read};

fn run() -> Result<(), Box<dyn Error>> {
    let paths: Vec<_> = env::args_os().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: cmvs-cpz <archive> [archive ...]".into());
    }
    if paths.len() == 1 && paths[0] == "--help" {
        println!(
            "Usage: cmvs-cpz <archive> [archive ...]\nCPZ6 header metadata only; checksums and indexes not verified. Reads 64 bytes per archive."
        );
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
