use cmvs_desktop::font::FontRenderer;
use cmvs_probe::{
    checks,
    device::Device,
    json::Json,
    manifest::Manifest,
    report::{self, Outcome},
    session::Options,
    target::{Target, cap},
};
use std::{fs, path::PathBuf, process::ExitCode};

const HEAD: &str = "Usage: cmvs-probe <game-root> --out <new-dir> [options]

Runs game-agnostic checks against a read-only game root and writes summary.json and
events.jsonl into a new output directory. Exit code 1 when any check fails or is blocked.

Checks (--checks takes a comma-separated list):";

const OPTIONS: &str = "Options:
  --checks <ids>        checks to run, e.g. S2,R2,R3, or `all`
  --out <new-dir>       required; create reports here (directory must not exist)
  --profile <id>        force an engine profile instead of identifying one
  --manifest <file>     per-game corrections to automatic recognition (optional)
  --device <name>       mouse (default) or keyboard input
  --font <file>         font for glyph metrics
  --font-index <n>      collection face index; requires --font (default 0)
  --render-frames       composite software device frames at VM yields; requires --font
  --frame-ms <n>        positive virtual frame time in milliseconds (default 16)
  --seed <n>            decimal u32 random seed (default 0)
  --max-steps <n>       positive VM step budget per session (default 50000000)
  --max-routes <n>      positive route limit per choice tree (default 16)
  --max-rounds <n>      unlock rounds for R4, at least 2 (default 4)
  --max-page-trials <n> positive action limit per page graph (default 400)
  --no-route-skip       do not hold the skip key while exploring routes
  --fork <mode>         replay (default) runs every route from the start; save starts
                        branches from a checkpoint saved at the choice (needs saves)
  --without-snapshots   research only: continue without save images; skips V1-V4
  --block-presumed      stop at commands with only legacy presumed evidence
  --block-unreviewed    stop at unreviewed commands
  -h, --help           show this help

Example: cmvs-probe game-root --out new-report --checks S1,S2
Exit codes: 0 completed, 1 failed/blocked checks, 2 argument or runtime error.";

/// The check list comes from the catalog, so it always matches what can run.
fn usage() -> String {
    let mut text = format!(
        "{HEAD}
"
    );
    for (id, title) in checks::CATALOG {
        let default = if checks::DEFAULT.contains(&id) {
            " (default)"
        } else {
            ""
        };
        text.push_str(&format!(
            "  {id:<3} {title}{default}
"
        ));
    }
    text.push_str(
        "  Checks without (default) play the scenario and can take hours.

",
    );
    text.push_str(OPTIONS);
    text
}

struct Args {
    root: PathBuf,
    out: PathBuf,
    checks: Vec<String>,
    profile: Option<String>,
    font: Option<(PathBuf, u32)>,
    manifest: Option<PathBuf>,
    options: Options,
}

fn parse() -> Result<Args, String> {
    parse_args(std::env::args().skip(1))
}

fn number<T: std::str::FromStr>(value: String, option: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{option}: invalid or out-of-range integer {value}"))
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut root = None;
    let mut out = None;
    let mut checks = None;
    let mut profile = None;
    let mut manifest = None;
    let (mut font, mut font_index) = (None, None);
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .filter(|v| !v.starts_with("--") && v != "-h")
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--help" | "-h" => return Err(String::new()),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--checks" => checks = Some(value()?.split(',').map(|s| s.trim().to_owned()).collect()),
            "--profile" => profile = Some(value()?),
            "--manifest" => manifest = Some(PathBuf::from(value()?)),
            "--device" => {
                let v = value()?;
                options.device = Device::parse(&v).ok_or(format!("unknown device {v}"))?;
            }
            "--font" => font = Some(PathBuf::from(value()?)),
            "--font-index" => font_index = Some(number(value()?, &arg)?),
            "--frame-ms" => options.frame_ms = number(value()?, &arg)?,
            "--seed" => options.seed = number(value()?, &arg)?,
            "--max-steps" => options.max_steps = number(value()?, &arg)?,
            "--max-routes" => options.max_routes = number(value()?, &arg)?,
            "--max-rounds" => options.max_rounds = number(value()?, &arg)?,
            "--max-page-trials" => options.max_page_trials = number(value()?, &arg)?,
            "--no-route-skip" => options.route_skip = false,
            "--without-snapshots" => options.capture_snapshots = false,
            "--render-frames" => options.render_frames = true,
            "--fork" => {
                options.fork_saves = match value()?.as_str() {
                    "replay" => false,
                    "save" => true,
                    other => return Err(format!("unknown fork mode {other}")),
                }
            }
            "--block-presumed" => options.block_presumed = true,
            "--block-unreviewed" => options.block_unreviewed = true,
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            path if root.is_none() => root = Some(PathBuf::from(path)),
            extra => return Err(format!("unexpected argument {extra}")),
        }
    }
    if options.fork_saves && !options.capture_snapshots {
        return Err("--fork save requires snapshot capture".into());
    }
    if font_index.is_some() && font.is_none() {
        return Err("--font-index requires --font".into());
    }
    if options.render_frames && font.is_none() {
        return Err("--render-frames requires --font".into());
    }
    if options.frame_ms == 0
        || options.max_steps == 0
        || options.max_routes == 0
        || options.max_page_trials == 0
    {
        return Err(
            "--frame-ms, --max-steps, --max-routes and --max-page-trials must be positive".into(),
        );
    }
    if options.max_rounds < 2 {
        return Err("--max-rounds must be at least 2".into());
    }
    let checks: Vec<String> = match checks {
        Some(list) if list == ["all"] => checks::CATALOG
            .iter()
            .map(|(id, _)| id.to_string())
            .collect(),
        Some(list) => list,
        None => checks::DEFAULT.iter().map(|s| s.to_string()).collect(),
    };
    if let Some(unknown) = checks.iter().find(|c| !checks::known(c)) {
        return Err(format!("unknown check {unknown}"));
    }
    Ok(Args {
        root: root.ok_or("missing game root")?,
        out: out.ok_or("missing --out")?,
        checks,
        profile,
        font: font.map(|f| (f, font_index.unwrap_or(0))),
        manifest,
        options,
    })
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(args) => args,
        Err(e) => {
            if e.is_empty() {
                println!("{}", usage());
                return ExitCode::SUCCESS;
            }
            eprintln!("Error: {e}\nRun 'cmvs-probe --help' for usage.");
            return ExitCode::from(2);
        }
    };
    // Snapshot capture and restore contain bounded VM banks and object tables. In
    // debug builds their nested stack frames exceed Windows' default main stack.
    let result = std::thread::Builder::new()
        .name("cmvs-probe".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || run(args));
    let result = match result {
        Ok(worker) => worker
            .join()
            .unwrap_or_else(|_| Err("probe worker panicked".into())),
        Err(e) => Err(format!("could not start probe worker: {e}")),
    };
    match result {
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
    let manifest = match &args.manifest {
        Some(path) => Some(Manifest::read(path)?),
        None => None,
    };
    if let Some(manifest) = &manifest
        && let Some(wanted) = manifest.profile()
    {
        let selected = target
            .game
            .as_ref()
            .map(|g| g.selection.profile.id.as_str());
        if selected != Some(wanted) {
            return Err(format!(
                "the manifest is for profile {wanted}, but {} was selected",
                selected.unwrap_or("none")
            ));
        }
        manifest.mark_profile_used();
    }
    let selected: Vec<&str> = args.checks.iter().map(String::as_str).collect();
    let run = checks::run(
        &target,
        &selected,
        &args.options,
        &args.out,
        font.as_ref(),
        manifest.as_ref(),
    );
    let initial_system = args.root.join("save/system.dat");
    let header = Json::obj([
        ("probe", Json::from(env!("CARGO_PKG_VERSION"))),
        ("target", target.identity_json()),
        ("options", args.options.json()),
        (
            "manifest",
            manifest.as_ref().map_or(Json::Null, Manifest::json),
        ),
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

#[cfg(test)]
mod tests {
    use super::parse_args;

    #[test]
    fn rendering_requires_a_font_and_is_opt_in() {
        let parse = |flags: &[&str]| {
            parse_args(
                ["game-root", "--out", "new-report"]
                    .into_iter()
                    .chain(flags.iter().copied())
                    .map(String::from),
            )
        };
        assert!(!parse(&[]).unwrap().options.render_frames);
        assert_eq!(
            parse(&["--render-frames"]).err().unwrap(),
            "--render-frames requires --font"
        );
        assert!(
            parse(&["--font", "font.ttf", "--render-frames"])
                .unwrap()
                .options
                .render_frames
        );
    }

    #[test]
    fn disabling_snapshots_cannot_claim_saved_forks() {
        for flags in [
            vec!["--without-snapshots", "--fork", "save"],
            vec!["--fork", "save", "--without-snapshots"],
        ] {
            let error = parse_args(flags.into_iter().map(String::from))
                .err()
                .unwrap();
            assert_eq!(error, "--fork save requires snapshot capture");
        }
        let args = parse_args(
            ["game-root", "--out", "new-report", "--without-snapshots"]
                .into_iter()
                .map(String::from),
        )
        .unwrap();
        assert!(!args.options.capture_snapshots);
        assert!(!args.options.fork_saves);
    }
}
