use cmvs_rs::{
    digest::{sha256, to_hex},
    exe::{Arch, PeImage, find_dispatch},
    profile::{CommandStatus, EXECUTABLE_NAMES, EngineProfile, Family, Registry},
};
use std::{env, error::Error, fs, path::Path};

const USAGE: &str = "Usage: cmvs-profile identify <game-root> [--profile <id>]\n       cmvs-profile table <game-root> --id <id> [--base <id>]\n       cmvs-profile show <id>\nidentify hashes the start script, script archive and executables read-only, locates each executable's system-command switch, checks it against the embedded profile tables and reports the profile selection. table prints a new profile from the executables in a game root: present commands become base for a root profile and unreviewed with --base. show summarises an embedded profile.";

/// Signatures that identify the predecessor engine family, reported but not supported.
const CVNS_HINTS: [&str; 3] = ["cvns.exe", "data/pack/start.pss", "pvns.exe"];

fn magic(path: &Path) -> String {
    match fs::read(path) {
        Ok(bytes) => bytes
            .get(..4)
            .map(|m| m.escape_ascii().to_string())
            .unwrap_or_else(|| "<short>".into()),
        Err(e) => format!("<{e}>"),
    }
}

fn identify(root: &Path, explicit: Option<&str>) -> Result<(), Box<dyn Error>> {
    let registry = Registry::builtin();
    println!("# read-only identification of {}", root.display());
    for name in [
        "data/pack/start.ps3",
        "data/pack/start.ps2",
        "data/pack/script.cpz",
    ] {
        let path = root.join(name);
        if path.is_file() {
            let bytes = fs::read(&path)?;
            println!(
                "file {name} magic={} sha256={}",
                magic(&path),
                to_hex(&sha256(&bytes))
            );
        }
    }
    for hint in CVNS_HINTS {
        if root.join(hint).is_file() {
            println!("family-hint {hint}: CVNS/PVNS family, not supported");
        }
    }
    for name in EXECUTABLE_NAMES {
        let path = root.join(name);
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path)?;
        let digest = sha256(&bytes);
        let image = match PeImage::parse(&bytes) {
            Ok(image) => image,
            Err(e) => {
                println!("exe {name} sha256={} error={e}", to_hex(&digest));
                continue;
            }
        };
        print!(
            "exe {name} {} sha256={} timestamp=0x{:08x} linker={}.{} image-base=0x{:x}",
            image.arch.name(),
            to_hex(&digest),
            image.timestamp,
            image.linker.0,
            image.linker.1,
            image.image_base
        );
        match find_dispatch(&image, &bytes) {
            Ok(dispatch) => {
                println!(
                    " switch=0x{:x} table=0x{:x} default=0x{:x} cases={} handlers={} noop={}",
                    dispatch.compare_rva,
                    dispatch.table_rva,
                    dispatch.default_rva,
                    dispatch.cases.len(),
                    dispatch.handlers(),
                    dispatch.noops()
                );
                if let Some((profile, binary)) = registry.binary(&digest) {
                    let verdict = if binary.matches(&image, &dispatch) {
                        "matches"
                    } else {
                        "DIFFERS FROM"
                    };
                    println!(
                        "check {name}: switch {verdict} profile {} {} table",
                        profile.id,
                        binary.arch.name()
                    );
                    if !binary.matches(&image, &dispatch) {
                        return Err("embedded profile table disagrees with the executable".into());
                    }
                }
            }
            Err(e) => println!(" switch-error={e}"),
        }
    }
    let game = registry.identify_game(root, explicit)?;
    for line in game.selection.log_lines() {
        println!("{line}");
    }
    let profile = game.selection.profile;
    println!(
        "commands base={} unreviewed={} noop={} absent={} (of {} cases)",
        profile.count(CommandStatus::Base),
        profile.count(CommandStatus::Unreviewed),
        profile.count(CommandStatus::NoOp),
        profile.count(CommandStatus::Absent),
        profile.statuses().len()
    );
    Ok(())
}

fn table(root: &Path, id: &str, base: Option<&str>) -> Result<(), Box<dyn Error>> {
    let mut found = Vec::new();
    for name in EXECUTABLE_NAMES {
        let path = root.join(name);
        if path.is_file() {
            let bytes = fs::read(&path)?;
            let image = PeImage::parse(&bytes)?;
            let dispatch = find_dispatch(&image, &bytes)?;
            found.push((name.to_owned(), sha256(&bytes), image, dispatch));
        }
    }
    // x64 first: base-profile addresses in code comments refer to the x64 executable.
    found.sort_by_key(|(_, _, image, _)| image.arch != Arch::X64);
    let names: Vec<_> = found.iter().map(|(name, ..)| name.as_str()).collect();
    let profile = EngineProfile::from_dispatch(
        id,
        Family::Cmvs,
        base,
        found
            .iter()
            .map(|(name, digest, image, dispatch)| (name.clone(), *digest, image, dispatch))
            .collect(),
    )?;
    println!(
        "# Generated by `cmvs-profile table` from {}; review statuses before changing them.",
        names.join(", ")
    );
    print!("{}", profile.to_text());
    Ok(())
}

fn ranges(commands: impl Iterator<Item = usize>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut run: Option<(usize, usize)> = None;
    for c in commands {
        run = match run {
            Some((start, end)) if end + 1 == c => Some((start, c)),
            Some((start, end)) => {
                out.push(range(start, end));
                Some((c, c))
            }
            None => Some((c, c)),
        };
    }
    if let Some((start, end)) = run {
        out.push(range(start, end));
    }
    out.join(" ")
}
fn range(start: usize, end: usize) -> String {
    if start == end {
        format!("0x{:04x}", 0x2000 + start)
    } else {
        format!("0x{:04x}-0x{:04x}", 0x2000 + start, 0x2000 + end)
    }
}

fn show(id: &str) -> Result<(), Box<dyn Error>> {
    let registry = Registry::builtin();
    let profile = registry
        .profile(id)
        .ok_or_else(|| format!("unknown engine profile {id}"))?;
    println!(
        "profile {} family={:?} base={}",
        profile.id,
        profile.family,
        profile.base.as_deref().unwrap_or("-")
    );
    for b in &profile.binaries {
        println!(
            "binary {} {} sha256={} timestamp=0x{:08x} cases={}",
            b.arch.name(),
            b.file,
            to_hex(&b.sha256),
            b.timestamp,
            b.cases.len()
        );
    }
    for game in registry.games().iter().filter(|g| g.profile == profile.id) {
        println!("game {} game-id=0x{:08x}", game.game, game.game_id);
    }
    for status in [
        CommandStatus::Base,
        CommandStatus::Unreviewed,
        CommandStatus::NoOp,
        CommandStatus::Absent,
    ] {
        let matching = profile
            .statuses()
            .iter()
            .enumerate()
            .filter(move |(_, s)| **s == status)
            .map(|(c, _)| c);
        println!(
            "{} {}: {}",
            status.name(),
            profile.count(status),
            if status == CommandStatus::Base {
                "(see profile file)".into()
            } else {
                ranges(matching)
            }
        );
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    // Options follow the subcommand's positional argument as `--name value` pairs.
    let options = |allowed: &[&str]| -> Result<Vec<(String, String)>, Box<dyn Error>> {
        let rest = args.get(2..).unwrap_or_default();
        if rest.len() % 2 != 0 {
            return Err("missing option value".into());
        }
        rest.as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if allowed.contains(&pair[0].as_str()) {
                    Ok((pair[0].clone(), pair[1].clone()))
                } else {
                    Err(format!("unknown option {}", pair[0]).into())
                }
            })
            .collect()
    };
    let find = |options: &[(String, String)], name: &str| {
        options
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    match args.first().map(String::as_str) {
        Some("--help") | None => {
            println!("{USAGE}");
            Ok(())
        }
        Some("identify") if args.len() >= 2 => {
            let options = options(&["--profile"])?;
            identify(Path::new(&args[1]), find(&options, "--profile").as_deref())
        }
        Some("table") if args.len() >= 2 => {
            let options = options(&["--id", "--base"])?;
            let id = find(&options, "--id").ok_or("table requires --id")?;
            table(
                Path::new(&args[1]),
                &id,
                find(&options, "--base").as_deref(),
            )
        }
        Some("show") if args.len() == 2 => show(&args[1]),
        _ => Err(USAGE.into()),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
