//! Unified offline fan-patch script, image, and archive converters.

pub mod amatsutsumi;
pub mod aoitori;
pub mod asukimi;
pub mod chronoclock;
pub mod common;
pub mod hapymaher;
pub mod kunado;
pub mod miranosu;
pub mod realive;

use common::{OutputEncoding, Result};
use std::{ffi::OsString, path::Path};

pub const USAGE: &str = "\
Usage:
  cmvs-patch <game> <original-game> <stage-dir> <encoding>
  cmvs-patch asukimi <original-game> <normalized-gbk-patch> <empty-output>
  cmvs-patch asukimi --check-gbk <normalized-gbk-patch>
  cmvs-patch miranosu <original-game> <decoded-mirai> <empty-output> <encoding>

For the first form, <game> is amatsutsumi, aoitori, chronoclock, hapymaher,
kunado or realive. <encoding> is gbk or utf-8 (also accepts utf8).
Use -h or --help by itself to show this help.

The Python converters prepare the inputs for this helper.
See tools/fan_patch/README.md for conversion workflows.";

pub fn run_cli<I: IntoIterator<Item = OsString>>(args: I) -> Result<()> {
    let args: Vec<OsString> = args.into_iter().collect();
    if args.is_empty() {
        return Err("missing game subcommand; use --help".into());
    }
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let game = args[0]
        .to_str()
        .ok_or("game subcommand must be valid UTF-8")?;
    let rest = &args[1..];
    match game {
        "amatsutsumi" => {
            let [game_dir, stage, enc] =
                three_args(rest, "<original-game> <stage-dir> <encoding>")?;
            amatsutsumi::run(
                Path::new(game_dir),
                Path::new(stage),
                OutputEncoding::parse(enc)?,
            )
        }
        "aoitori" => {
            let [game_dir, stage, enc] =
                three_args(rest, "<original-game> <stage-dir> <encoding>")?;
            aoitori::run(
                Path::new(game_dir),
                Path::new(stage),
                OutputEncoding::parse(enc)?,
            )
        }
        "asukimi" => {
            if let [flag, input] = rest
                && flag == "--check-gbk"
            {
                return asukimi::check_gbk(Path::new(input));
            }
            let [game_dir, input, output] = three_args(
                rest,
                "<original-game> <normalized-gbk-patch> <empty-output>",
            )?;
            asukimi::run(Path::new(game_dir), Path::new(input), Path::new(output))
        }
        "chronoclock" => {
            let [game_dir, stage, enc] =
                three_args(rest, "<original-game> <stage-dir> <encoding>")?;
            chronoclock::run(
                Path::new(game_dir),
                Path::new(stage),
                OutputEncoding::parse(enc)?,
            )
        }
        "hapymaher" => {
            let [game_dir, stage, enc] =
                three_args(rest, "<original-game> <stage-dir> <encoding>")?;
            hapymaher::run(
                Path::new(game_dir),
                Path::new(stage),
                OutputEncoding::parse(enc)?,
            )
        }
        "kunado" => {
            let [game_dir, stage, enc] =
                three_args(rest, "<original-game> <stage-dir> <encoding>")?;
            kunado::run(
                Path::new(game_dir),
                Path::new(stage),
                OutputEncoding::parse(enc)?,
            )
        }
        "miranosu" => {
            if rest.len() != 4 {
                return Err(
                    "expected <original-game> <decoded-mirai> <empty-output> <encoding>".into(),
                );
            }
            miranosu::run(
                Path::new(&rest[0]),
                Path::new(&rest[1]),
                Path::new(&rest[2]),
                OutputEncoding::parse(&rest[3])?,
            )
        }
        "realive" => {
            let [game_dir, stage, enc] =
                three_args(rest, "<original-game> <stage-dir> <encoding>")?;
            realive::run(
                Path::new(game_dir),
                Path::new(stage),
                OutputEncoding::parse(enc)?,
            )
        }
        _ => Err(format!("unknown game subcommand: {game}").into()),
    }
}

fn three_args<'a>(args: &'a [OsString], expected: &str) -> Result<&'a [OsString; 3]> {
    args.try_into()
        .map_err(|_| format!("expected {expected}").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_args(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn cli_validates_help_subcommands_and_argument_counts() -> Result<()> {
        run_cli(os_args(&["--help"]))?;
        run_cli(os_args(&["-h"]))?;
        assert_eq!(
            run_cli(os_args(&[])).unwrap_err().to_string(),
            "missing game subcommand; use --help"
        );
        assert_eq!(
            run_cli(os_args(&["unknown"])).unwrap_err().to_string(),
            "unknown game subcommand: unknown"
        );
        for game in [
            "amatsutsumi",
            "aoitori",
            "asukimi",
            "chronoclock",
            "hapymaher",
            "kunado",
            "miranosu",
            "realive",
        ] {
            let error = run_cli(os_args(&[game])).unwrap_err().to_string();
            assert!(
                error.starts_with("expected <original-game>"),
                "{game}: {error}"
            );
        }
        for args in [
            &["amatsutsumi", "g", "s", "utf-16"][..],
            &["miranosu", "g", "p", "o", "utf-16"],
        ] {
            assert_eq!(
                run_cli(os_args(args)).unwrap_err().to_string(),
                "encoding must be utf-8, utf8 or gbk"
            );
        }
        Ok(())
    }
}
