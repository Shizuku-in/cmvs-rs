//! Thin CLI entry point; desktop implementation lives in `cmvs-desktop`.
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if cmvs_cli::wants_help(&args)
        && !args
            .first()
            .is_some_and(|first| first == "-h" || first == "--help")
        && let Ok(exe) = std::env::current_exe()
        && let Ok(status) = std::process::Command::new(exe).arg("--help").status()
    {
        std::process::exit(status.code().unwrap_or(0));
    }
    cmvs_desktop::run_cli();
}
