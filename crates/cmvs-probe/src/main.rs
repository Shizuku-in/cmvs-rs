use cmvs_desktop::font::FontRenderer;
use cmvs_probe::{
    checks,
    device::Device,
    json::Json,
    report::{self, Outcome},
    session::Options,
    target::{Target, cap},
};
use std::{fs, path::PathBuf, process::ExitCode};

const USAGE: &str = "Usage: cmvs-probe <game-root> --out <new-dir> [--checks S1,S2,S3,R1,D1,R2,R3] \
[--profile <id>] [--device mouse|keyboard] [--font <file> [--font-index <n>]] [--frame-ms <n>] \
[--seed <n>] [--max-steps <n>] [--max-routes <n>] [--no-route-skip] [--allow-presumed] [--allow-unreviewed]\n\
Runs game-agnostic checks (R2/R3 explore routes and only run when listed) against a read-only game root and writes summary.json and \
events.jsonl into a new output directory. Exit code 1 when any check fails or is blocked.";

struct Args {
    root: PathBuf,
    out: PathBuf,
    checks: Vec<String>,
    profile: Option<String>,
    font: Option<(PathBuf, u32)>,
    options: Options,
}

fn parse() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut root = None;
    let mut out = None;
    let mut checks = None;
    let mut profile = None;
    let (mut font, mut font_index) = (None, 0);
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        let number = |v: String| v.parse::<u64>().map_err(|_| format!("bad number {v}"));
        match arg.as_str() {
            "--help" | "-h" => return Err(String::new()),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--checks" => checks = Some(value()?.split(',').map(|s| s.trim().to_owned()).collect()),
            "--profile" => profile = Some(value()?),
            "--device" => {
                let v = value()?;
                options.device = Device::parse(&v).ok_or(format!("unknown device {v}"))?;
            }
            "--font" => font = Some(PathBuf::from(value()?)),
            "--font-index" => font_index = number(value()?)? as u32,
            "--frame-ms" => options.frame_ms = number(value()?)? as u32,
            "--seed" => options.seed = number(value()?)? as u32,
            "--max-steps" => options.max_steps = number(value()?)?,
            "--max-routes" => options.max_routes = number(value()?)? as usize,
            "--no-route-skip" => options.route_skip = false,
            "--allow-presumed" => options.allow_presumed = true,
            "--allow-unreviewed" => options.allow_unreviewed = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            path if root.is_none() => root = Some(PathBuf::from(path)),
            extra => return Err(format!("unexpected argument {extra}")),
        }
    }
    let checks: Vec<String> =
        checks.unwrap_or_else(|| checks::DEFAULT.iter().map(|s| s.to_string()).collect());
    if let Some(unknown) = checks
        .iter()
        .find(|c| !checks::IMPLEMENTED.contains(&c.as_str()))
    {
        return Err(format!("unknown or unimplemented check {unknown}"));
    }
    Ok(Args {
        root: root.ok_or("missing game root")?,
        out: out.ok_or("missing --out")?,
        checks,
        profile,
        font: font.map(|f| (f, font_index)),
        options,
    })
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(args) => args,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(clean) => ExitCode::from(u8::from(!clean)),
        Err(e) => {
            eprintln!("cmvs-probe: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Args) -> Result<bool, String> {
    // The output directory must be new; reports are never merged or overwritten.
    fs::create_dir(&args.out).map_err(|e| format!("{}: {e}", args.out.display()))?;
    let mut target = Target::open(&args.root, args.profile.as_deref());
    let font = match &args.font {
        Some((path, index)) => {
            let font = FontRenderer::read(path, *index).map_err(|e| e.to_string())?;
            target.capabilities.insert(cap::FONT);
            Some(font)
        }
        None => None,
    };
    let selected: Vec<&str> = args.checks.iter().map(String::as_str).collect();
    let run = checks::run(&target, &selected, &args.options, &args.out, font.as_ref());
    let initial_system = args.root.join("save/system.dat");
    let header = Json::obj([
        ("probe", Json::from(env!("CARGO_PKG_VERSION"))),
        ("target", target.identity_json()),
        ("options", args.options.json()),
        ("manifest", Json::Null),
        (
            "initial_system_save_from_game",
            initial_system.is_file().into(),
        ),
    ]);
    report::write(&args.out, header, &run.results, &run.events).map_err(|e| e.to_string())?;
    let mut clean = true;
    for result in &run.results {
        let detail = match &result.outcome {
            Outcome::Pass(level) => level.name().to_owned(),
            Outcome::Fail(why) | Outcome::Skipped(why) | Outcome::Inconclusive(why) => why.clone(),
            Outcome::Blocked(b) => format!(
                "{} 0x{:04x} at {} / 0x{:x}",
                b.kind, b.command, b.script, b.pc
            ),
        };
        clean &= !matches!(result.outcome, Outcome::Fail(_) | Outcome::Blocked(_));
        println!("{:<3} {:<12} {detail}", result.id, result.outcome.name());
    }
    println!("report: {}", args.out.join("summary.json").display());
    Ok(clean)
}
