use cmvs_core::{
    format::digest::{sha256, to_hex},
    profile::{CommandStatus, EXECUTABLE_NAMES, Family, Registry},
};
use cmvs_profile_research::{
    ResearchProfile, ResearchRegistry,
    exe::{Arch, PeImage, find_dispatch},
};
use std::{
    env,
    error::Error,
    fs,
    io::{self, Write},
    path::Path,
};

#[path = "profile/generate.rs"]
mod generate;

const USAGE: &str = "\
Usage:
  cmvs-profile identify <game-root> [--profile <id|alias>] [--profiles-dir <dir>]
  cmvs-profile table <game-root> --id <new-id> [--base <id|alias>]
  cmvs-profile show <id|alias> [--profiles-dir <dir>]
  cmvs-profile generate-playback [--profiles-dir <dir>] [--output <path>]
  cmvs-profile owners [<command>]

Commands:
  identify           hash game files, identify the build and check executable dispatch tables
  table              print a candidate profile from the game's executables
  show               list profile statuses and mappings
  generate-playback  validate research data and generate Rust playback tables
  owners             list candidate runner/trace handlers; command is 0x2000 through 0x23ff

Aliases are registered game names such as aki. For table, --base creates a
derived profile with unreviewed commands. CVNS needs a CMVS root as its base.

--profiles-dir defaults to crates/cmvs-profile-research/profiles in the checkout.
generate-playback writes to stdout by default. --output replaces the target file.
Use -h or --help to show this help.

Example: cmvs-profile show aki";

/// Signatures of an earlier predecessor engine, reported but not supported.
const PVNS_HINTS: [&str; 1] = ["pvns.exe"];

fn load_research(
    directory: Option<&str>,
) -> Result<ResearchRegistry, Box<dyn Error + Send + Sync>> {
    let directory = directory.unwrap_or(concat!(env!("CARGO_MANIFEST_DIR"), "/profiles"));
    ResearchRegistry::load(Path::new(directory))
        .map_err(|err| format!("{err}; use --profiles-dir to locate the research directory").into())
}

fn magic(path: &Path) -> String {
    match fs::read(path) {
        Ok(bytes) => bytes
            .get(..4)
            .map(|m| m.escape_ascii().to_string())
            .unwrap_or_else(|| "<short>".into()),
        Err(e) => format!("<{e}>"),
    }
}

fn identify(
    registry: &ResearchRegistry,
    root: &Path,
    explicit: Option<&str>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    println!("# read-only identification of {}", root.display());
    for name in [
        "data/pack/start.ps3",
        "data/pack/start.ps2",
        "data/pack/start.pss",
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
    for hint in PVNS_HINTS {
        if root.join(hint).is_file() {
            println!("family-hint {hint}: PVNS family, not supported");
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
                        return Err("research profile table disagrees with the executable".into());
                    }
                }
            }
            Err(e) => println!(" switch-error={e}"),
        }
    }
    let game = registry.playback().identify_game(root, explicit)?;
    for line in game.selection.log_lines() {
        println!("{line}");
    }
    let profile = game.selection.profile;
    println!(
        "commands base={} inherited={} reviewed={} contract-equivalent={} presumed={} unreviewed={} noop={} absent={} (of {} cases)",
        profile.count(CommandStatus::Base),
        profile.count(CommandStatus::Inherited),
        profile.count(CommandStatus::Reviewed),
        profile.count(CommandStatus::ContractEquivalent),
        profile.count(CommandStatus::Presumed),
        profile.count(CommandStatus::Unreviewed),
        profile.count(CommandStatus::NoOp),
        profile.count(CommandStatus::Absent),
        profile.statuses().len()
    );
    Ok(())
}

fn table(root: &Path, id: &str, base: Option<&str>) -> Result<(), Box<dyn Error + Send + Sync>> {
    let base = base.map(|selector| {
        Registry::builtin()
            .resolve_profile(selector)
            .map_or(selector, |profile| profile.id.as_str())
    });
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
    let family = if names.contains(&"cvns.exe") {
        if names.len() > 1 || base.is_none() {
            return Err("a CVNS profile has only cvns.exe and needs --base <CMVS root>".into());
        }
        Family::Cvns
    } else {
        Family::Cmvs
    };
    let profile = ResearchProfile::from_dispatch(
        id,
        family,
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

fn show(registry: &ResearchRegistry, selector: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let profile = registry
        .playback()
        .resolve_profile(selector)
        .and_then(|profile| registry.profile(&profile.id))
        .ok_or_else(|| format!("unknown engine profile {selector}"))?;
    println!(
        "profile {} family={:?} base={}",
        profile.id,
        profile.family,
        profile.base.as_deref().unwrap_or("-")
    );
    for b in profile.binaries() {
        println!(
            "binary {} {} sha256={} timestamp=0x{:08x} cases={}",
            b.arch.name(),
            b.file,
            to_hex(&b.sha256),
            b.timestamp,
            b.cases.len()
        );
    }
    for game in registry
        .playback()
        .games()
        .iter()
        .filter(|g| g.profile == profile.id)
    {
        println!("game {} game-id=0x{:08x}", game.game, game.game_id);
    }
    for status in [
        CommandStatus::Base,
        CommandStatus::Inherited,
        CommandStatus::Reviewed,
        CommandStatus::ContractEquivalent,
        CommandStatus::Presumed,
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
    if let Some(map) = profile.command_map() {
        println!("cvns-map {} commands", map.len());
        for command in 0..profile.statuses().len() as u16 {
            if let Some(target) = profile.map(command) {
                println!(
                    "map 0x{:04x} -> 0x{:04x}",
                    0x2000 + command,
                    0x2000 + target
                );
            }
        }
    }
    Ok(())
}

fn owners(opcode: Option<&str>) -> Result<(), Box<dyn Error + Send + Sync>> {
    use cmvs_core::{vm::dispatch::ownership::Chain, vm::dispatch::ownership::candidates};
    let selected = opcode
        .map(|value| {
            value
                .strip_prefix("0x")
                .and_then(|hex| u16::from_str_radix(hex, 16).ok())
                .filter(|opcode| (0x2000..0x2400).contains(opcode))
                .map(|opcode| opcode - 0x2000)
                .ok_or("opcode must be hexadecimal in 0x2000..0x23ff")
        })
        .transpose()?;
    println!("# Handler candidates in chain order; no handlers or evidence gates executed");
    println!("# Shared root inventory; profile-declared variant bindings are separate");
    println!(
        "# trace candidates may require optional hosts/resources/flags; root text has no trace host"
    );
    println!("| opcode | runner | trace |");
    println!("|---|---|---|");
    for command in 0..0x400 {
        if selected.is_some_and(|selected| selected != command) {
            continue;
        }
        let runner = candidates(command, Chain::Runner);
        let trace = candidates(command, Chain::Trace);
        if selected.is_none() && runner.is_empty() && trace.is_empty() {
            continue;
        }
        let names = |claims: Vec<&cmvs_core::vm::dispatch::ownership::Claim>| {
            if claims.is_empty() {
                "—".into()
            } else {
                claims
                    .iter()
                    .map(|claim| claim.handler)
                    .collect::<Vec<_>>()
                    .join(" → ")
            }
        };
        println!(
            "| 0x{:04x} | {} | {} |",
            command + 0x2000,
            names(runner),
            names(trace)
        );
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let raw_args: Vec<_> = env::args_os().skip(1).collect();
    if raw_args
        .iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "-h" || arg == "--help")
    {
        println!("{USAGE}");
        return Ok(());
    }
    let mut args = raw_args.into_iter().peekable();
    let mut positional_only = false;
    if args.peek().is_some_and(|a| a == "--") {
        args.next();
        positional_only = true;
    }
    let Some(command) = args.next() else {
        return Err(
            "missing subcommand: identify, table, show, generate-playback or owners".into(),
        );
    };
    if !positional_only && command.to_string_lossy().starts_with('-') {
        return Err(format!("unknown option {}", command.to_string_lossy()).into());
    }
    let command = command
        .into_string()
        .map_err(|_| "arguments must be valid UTF-8")?;
    type ParsedArgs = (Vec<std::ffi::OsString>, Vec<(String, String)>);
    let parse_rest = |args: &mut std::iter::Peekable<std::vec::IntoIter<std::ffi::OsString>>,
                      allowed: &[&str]|
     -> Result<ParsedArgs, Box<dyn Error + Send + Sync>> {
        let mut positionals = Vec::new();
        let mut parsed = Vec::new();
        let mut only_pos = positional_only;
        while let Some(arg) = args.next() {
            if !only_pos && arg == "--" {
                only_pos = true;
                continue;
            }
            if !only_pos && arg.to_string_lossy().starts_with('-') {
                let flag = arg
                    .into_string()
                    .map_err(|_| "arguments must be valid UTF-8")?;
                if !allowed.contains(&flag.as_str()) {
                    return Err(format!("unknown option {flag}").into());
                }
                let value = args
                    .next()
                    .filter(|v| !v.to_string_lossy().starts_with("--") && v != "-h")
                    .ok_or_else(|| format!("{flag} requires a value"))?
                    .into_string()
                    .map_err(|_| "arguments must be valid UTF-8")?;
                parsed.push((flag, value));
                continue;
            }
            positionals.push(arg);
        }
        Ok((positionals, parsed))
    };
    let find = |options: &[(String, String)], name: &str| {
        options
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    match command.as_str() {
        "identify" => {
            let (positionals, options) = parse_rest(&mut args, &["--profile", "--profiles-dir"])?;
            let [root]: [_; 1] = positionals
                .try_into()
                .map_err(|_| "invalid arguments for identify")?;
            let registry = load_research(find(&options, "--profiles-dir").as_deref())?;
            identify(
                &registry,
                Path::new(&root),
                find(&options, "--profile").as_deref(),
            )
        }
        "table" => {
            let (positionals, options) = parse_rest(&mut args, &["--id", "--base"])?;
            let [root]: [_; 1] = positionals
                .try_into()
                .map_err(|_| "invalid arguments for table")?;
            let id = find(&options, "--id").ok_or("table requires --id")?;
            table(Path::new(&root), &id, find(&options, "--base").as_deref())
        }
        "show" => {
            let (positionals, options) = parse_rest(&mut args, &["--profiles-dir"])?;
            let [id]: [_; 1] = positionals
                .try_into()
                .map_err(|_| "invalid arguments for show")?;
            let id = id
                .into_string()
                .map_err(|_| "arguments must be valid UTF-8")?;
            let registry = load_research(find(&options, "--profiles-dir").as_deref())?;
            show(&registry, &id)
        }
        "generate-playback" => {
            let (positionals, options) = parse_rest(&mut args, &["--profiles-dir", "--output"])?;
            if !positionals.is_empty() {
                return Err("invalid arguments for generate-playback".into());
            }
            let registry = load_research(find(&options, "--profiles-dir").as_deref())?;
            let mut source = String::new();
            generate::write_playback(&mut source, registry.playback())?;
            if let Some(output) = find(&options, "--output") {
                generate::replace_table(Path::new(&output), source.as_bytes())?;
            } else {
                io::stdout().lock().write_all(source.as_bytes())?;
            }
            Ok(())
        }
        "owners" => {
            let (positionals, _) = parse_rest(&mut args, &[])?;
            if positionals.len() > 1 {
                return Err("invalid arguments for owners".into());
            }
            let cmd = positionals
                .into_iter()
                .next()
                .map(|s| s.into_string().map_err(|_| "arguments must be valid UTF-8"))
                .transpose()?;
            owners(cmd.as_deref())
        }
        _ => Err(format!("unknown subcommand {command}").into()),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-profile --help' for usage.");
        std::process::exit(1);
    }
}
