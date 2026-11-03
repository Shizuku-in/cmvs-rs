use cmvs_rs::{
    cpz::{ArchiveIndex, ArchiveLayout, Cpz5Mode, Cpz7Parameters},
    ps3::Script,
    script_control::{cpz7_parameters_from_start, packing_mode_from_start},
};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, Write},
    path::Path,
};

const USAGE: &str = "\
Usage:
  cmvs-cpz <archive> [archive ...]
  cmvs-cpz [settings] --list <archive>
  cmvs-cpz [settings] --script <archive> <ASCII-basename>
  cmvs-cpz [settings] --extract <archive> <ASCII-basename> <new-file>

Modes:
  <archive> [...]  print headers only; indexes/checksums are not verified
  --list           list index entries and stored checksums
  --script         decode and summarise one PS2A script
  --extract        decode one entry to a new file; refuses overwrite

Settings (choose at most one; place before the mode):
  --start-script <file>   read CPZ5/7 settings from start.ps2 or start.ps3
  --packing-mode <5..9>   select a CPZ5 packing mode
  --keys <k0,k1,k2,k3>    four CPZ7 keys, each decimal or 0x-prefixed
  -h, --help             show this help

CPZ5/7 automatically read adjacent startup scripts unless settings are explicit.
Without startup settings, CPZ5 detects the mode from its index and CPZ7 uses zero parameters.
Index modes verify CPZ3 index and CPZ5/7 header/index integrity.
Resource checksums are not verified. Input archives are read only.
Example: cmvs-cpz --list data/pack/script.cpz
Example: cmvs-cpz --packing-mode 5 --extract script.cpz main.ps3 main.ps3";

fn read_start(file: File) -> Result<Script, Box<dyn Error>> {
    let mut raw = Vec::new();
    file.take(cmvs_rs::MAX_PAYLOAD as u64 + 49)
        .read_to_end(&mut raw)?;
    if raw.len() > cmvs_rs::MAX_PAYLOAD + 48 {
        return Err("startup script exceeds research limit".into());
    }
    Ok(Script::decode(&raw)?)
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut paths: Vec<_> = env::args_os().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: cmvs-cpz <archive> [archive ...]".into());
    }
    if paths.len() == 1 && (paths[0] == "--help" || paths[0] == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let mut start = if paths[0] == "--start-script" {
        let path = paths.get(1).ok_or("expected startup script path")?;
        let start = read_start(File::open(path)?)?;
        paths.drain(..2);
        if paths.is_empty() {
            return Err("expected archive mode; use --help".into());
        }
        Some(start)
    } else {
        None
    };
    let explicit_mode = paths[0] == "--packing-mode";
    let packing_mode = if explicit_mode {
        if start.is_some() {
            return Err("choose either --start-script or --packing-mode".into());
        }
        let value: u32 = paths
            .get(1)
            .and_then(|s| s.to_str())
            .ok_or("expected CPZ5 packing mode")?
            .parse()?;
        let mode = Cpz5Mode::try_from(value)?;
        paths.drain(..2);
        if paths.is_empty() {
            return Err("expected archive mode; use --help".into());
        }
        Some(mode)
    } else {
        None
    };
    let explicit_parameters = paths[0] == "--keys";
    let parameters = if explicit_parameters {
        if start.is_some() || explicit_mode {
            return Err("choose one archive settings option".into());
        }
        let raw = paths
            .get(1)
            .and_then(|s| s.to_str())
            .ok_or("expected four CPZ7 keys")?;
        let values: Vec<u32> = raw
            .split(',')
            .map(|s| {
                if let Some(hex) = s.strip_prefix("0x") {
                    u32::from_str_radix(hex, 16)
                } else {
                    s.parse()
                }
            })
            .collect::<Result<_, _>>()?;
        let keys: [u32; 4] = values
            .try_into()
            .map_err(|_| "expected exactly four CPZ7 keys")?;
        paths.drain(..2);
        if paths.is_empty() {
            return Err("expected archive mode; use --help".into());
        }
        Cpz7Parameters::from(keys)
    } else {
        Cpz7Parameters::default()
    };
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
        let mut magic = [0; 4];
        file.read_exact(&mut magic)?;
        file.rewind()?;
        if start.is_none() && !explicit_parameters && !explicit_mode {
            let candidates: &[&str] = match &magic {
                b"CPZ5" => &["start.ps3", "start.ps2"],
                b"CPZ7" => &["start.ps3"],
                _ => &[],
            };
            for name in candidates {
                match File::open(Path::new(&paths[1]).with_file_name(name)) {
                    Ok(file) => {
                        start = Some(read_start(file)?);
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        let index = if &magic == b"CPZ5" {
            if explicit_parameters {
                return Err("--keys requires CPZ7".into());
            }
            match (packing_mode, &start) {
                (Some(mode), _) => ArchiveIndex::read_with_cpz5_mode(&mut file, mode)?,
                (_, Some(script)) => ArchiveIndex::read_with_cpz5_mode(
                    &mut file,
                    Cpz5Mode::try_from(packing_mode_from_start(script)?)?,
                )?,
                _ => ArchiveIndex::read(&mut file)?,
            }
        } else {
            if explicit_mode {
                return Err("--packing-mode requires CPZ5".into());
            }
            if &magic != b"CPZ7" && (explicit_parameters || start.is_some()) {
                return Err("--keys and --start-script require CPZ5/CPZ7".into());
            }
            let parameters = if let Some(start) = &start {
                let basename = Path::new(&paths[1])
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or("expected UTF-8 archive basename")?;
                cpz7_parameters_from_start(start, basename.as_bytes())?
                    .ok_or("archive has no literal key registration in startup prefix")?
            } else {
                parameters
            };
            ArchiveIndex::read_with_cpz7_parameters(&mut file, parameters)?
        };
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
    if start.is_some() || explicit_parameters || explicit_mode {
        return Err("parameter options require --list, --script or --extract".into());
    }
    if let Some(flag) = paths
        .iter()
        .find(|path| path.to_string_lossy().starts_with('-'))
    {
        return Err(format!("unknown option {}", flag.to_string_lossy()).into());
    }
    for path in paths {
        let mut file = File::open(&path)?;
        let layout = ArchiveLayout::read(&mut file)?;
        println!("{path:?}: {layout:?}");
    }
    println!("# Metadata only; directory/file indexes and checksums NOT verified");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-cpz --help' for usage.");
        std::process::exit(1);
    }
}
