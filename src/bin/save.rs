use cmvs_rs::{
    MAX_PAYLOAD,
    save::{Csv2Layout, Csv2Save, HEADER_SIZE},
};
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
            "Usage: cmvs-save <save.dat>\n       cmvs-save --section <0|1|2> <save.dat> <new-output-file>\nRead-only CSV2 decoding. Section 0=snapshot, 1=auxiliary, 2=script memory image. Only section 0 has the original additive checksum; this does not authenticate the full file. No VM restoration."
        );
        return Ok(());
    }
    let section = if args.len() == 4 && args[0] == "--section" {
        let index = args[1]
            .to_str()
            .ok_or("section must be 0, 1 or 2")?
            .parse::<usize>()?;
        if index > 2 {
            return Err("section must be 0, 1 or 2".into());
        }
        Some(index)
    } else if args.len() == 1 && args[0] != "--section" {
        None
    } else {
        return Err("wrong argument count; use --help".into());
    };
    let input = if section.is_some() {
        &args[2]
    } else {
        &args[0]
    };
    let mut file = File::open(input)?;
    let length = file.metadata()?.len();
    let mut header = [0; HEADER_SIZE];
    file.read_exact(&mut header)?;
    let layout = Csv2Layout::parse(&header, length)?;
    let mut raw = header.to_vec();
    file.take(MAX_PAYLOAD as u64 + 1 - HEADER_SIZE as u64)
        .read_to_end(&mut raw)?;
    if raw.len() as u64 != length {
        return Err("CSV2 file changed length while reading".into());
    }
    let save = Csv2Save::decode(&raw)?;
    println!(
        "CSV2 label={} key=0x{:08x}",
        layout.header.label.escape_ascii(),
        layout.header.ordering_key
    );
    for (i, range) in layout.sections.iter().enumerate() {
        println!(
            "section[{i}] offset={} packed={} decoded={}",
            range.start,
            range.len(),
            layout.decoded_sizes[i]
        );
    }
    println!(
        "# snapshot additive checksum verified; other sections are not checksummed; no engine restoration"
    );
    if let Some(index) = section {
        let bytes = [&save.snapshot, &save.auxiliary, &save.script_image][index];
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[3])?;
        output.write_all(bytes)?;
        println!("Wrote {} decoded bytes to {:?}", bytes.len(), args[3]);
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
