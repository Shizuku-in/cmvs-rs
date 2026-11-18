use std::process::Command;

#[test]
fn profile_cli_honors_help_positions_and_double_dash() {
    let exe = env!("CARGO_BIN_EXE_cmvs-profile");
    for args in [
        &["identify", "game-dir", "--help"][..],
        &["identify", "game-dir", "--profile", "-h"],
        &["show", "cmvs-20160201", "--help"],
        &["owners", "0x2140", "-h"],
    ] {
        let output = Command::new(exe).args(args).output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout.starts_with(b"Usage:"));
        assert!(output.stderr.is_empty());
    }
    let after_separator = Command::new(exe)
        .args(["identify", "--", "--help"])
        .output()
        .unwrap();
    assert!(
        after_separator
            .stdout
            .starts_with(b"# read-only identification of --help")
    );
    assert!(!String::from_utf8_lossy(&after_separator.stderr).contains("unknown option"));
}

#[test]
fn profile_cli_shows_research_profiles_and_handler_owners() {
    let exe = env!("CARGO_BIN_EXE_cmvs-profile");
    let show = Command::new(exe)
        .args(["show", "cmvs-20160201"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(
        show.status.success(),
        "{}",
        String::from_utf8_lossy(&show.stderr)
    );
    assert!(
        stdout.contains("profile cmvs-20160201 family=Cmvs base=-"),
        "{stdout}"
    );

    let owners = Command::new(exe)
        .args(["owners", "0x2140"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(owners.stdout).unwrap();
    assert!(
        owners.status.success(),
        "{}",
        String::from_utf8_lossy(&owners.stderr)
    );
    assert!(stdout.contains("| 0x2140 |"), "{stdout}");

    assert!(
        !Command::new(exe)
            .args(["show", "cmvs-nonexistent"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn profile_cli_shows_the_same_profiles_by_game_alias() {
    let show = |selector| {
        let output = Command::new(env!("CARGO_BIN_EXE_cmvs-profile"))
            .args(["show", selector])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    for (alias, id) in [("aki", "cvns-20050216"), ("akifd", "cvns-20050727")] {
        assert_eq!(show(alias), show(id), "{alias}");
    }
}
