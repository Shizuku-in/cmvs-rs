//! The probe names no game. Script name literals,
//! game directory names and the like belong in per-game manifests, never in code.
use std::{fs, path::Path};

fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.display().to_string(),
                fs::read_to_string(&path).unwrap(),
            ));
        }
    }
}

#[test]
fn probe_sources_contain_no_script_names_or_game_paths() {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(!files.is_empty());
    // A quoted script name such as "menu.ps3" would let a game's layout decide state. A bare
    // extension (".ps3", for filtering archive entries) names no script and is allowed.
    let script_literal = |line: &str| {
        line.match_indices(".ps").any(|(i, _)| {
            let tail = &line[i + 3..];
            tail.starts_with(['2', '3']) && tail[1..].starts_with('"') && !line[..i].ends_with('"')
        })
    };
    let game_roots = ["E:/", "Download/", "Purple Software"];
    for (path, text) in &files {
        for (n, line) in text.lines().enumerate() {
            assert!(
                !script_literal(line),
                "{path}:{} names a script: {line}",
                n + 1
            );
            for root in game_roots {
                assert!(
                    !line.contains(root),
                    "{path}:{} names a game location: {line}",
                    n + 1
                );
            }
        }
    }
}
