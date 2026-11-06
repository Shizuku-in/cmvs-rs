//! Optional real-installation regression; the source game is never modified.
use super::Player;
use cmvs_core::save::Csv2Save;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn link_resources(source: &Path, target: &Path) {
    if source.is_dir() {
        fs::create_dir(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            link_resources(&entry.path(), &target.join(entry.file_name()));
        }
    } else {
        // The resource sandbox resolves paths, so directory symlinks would
        // escape the fixture. Hard links require a fixture on the same volume.
        fs::hard_link(source, target).expect("CMVS_TEST_TMPDIR must share the game's filesystem");
    }
}

fn advance(player: &mut Player, frames: u64) {
    let end = player.serial() + frames;
    for _ in 0..frames * 100 {
        if player.serial() >= end {
            return;
        }
        assert!(player.tick(16).unwrap() < 2);
    }
    panic!("frame budget exhausted");
}

#[test]
#[ignore = "requires CMVS_TEST_GAME (Amatsutsumi) and CMVS_TEST_FONT"]
fn amatsutsumi_game_savedata_survives_player_restart() {
    let source = PathBuf::from(std::env::var_os("CMVS_TEST_GAME").expect("CMVS_TEST_GAME"));
    let font = PathBuf::from(std::env::var_os("CMVS_TEST_FONT").expect("CMVS_TEST_FONT"));
    let base = std::env::var_os("CMVS_TEST_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let fixture = base.join(format!("cmvs-savedata-replay-{}", std::process::id()));
    fs::create_dir(&fixture).unwrap();
    let game = fixture.join("game");
    fs::create_dir(&game).unwrap();
    for entry in fs::read_dir(fs::canonicalize(source).unwrap()).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "savedata" {
            link_resources(&entry.path(), &game.join(entry.file_name()));
        }
    }
    let legacy = fixture.join("legacy");
    let savedata = game.join("savedata");
    let mut player = Player::open_game_savedata(&game, &legacy, &font, false).unwrap();
    advance(&mut player, 700);
    player.pointer(640, 280);
    player.key(1, true).unwrap();
    player.key(1, false).unwrap();
    advance(&mut player, 1800);
    for _ in 0..3 {
        player.key(22, true).unwrap();
        player.key(22, false).unwrap();
        advance(&mut player, 90);
    }
    assert!(
        player
            .session
            .with_dependent_mut(|_, s| s.save_slot(1))
            .unwrap()
    );
    player.save_preferences().unwrap();
    let slot = fs::read(savedata.join("save001.dat")).unwrap();
    let saved = Csv2Save::decode(&slot).unwrap();
    assert!(savedata.join("system.dat").is_file());
    assert!(savedata.join("cmvs.cfg").is_file());
    assert!(!legacy.exists());
    drop(player);

    let mut restarted = Player::open_game_savedata(&game, &legacy, &font, false).unwrap();
    advance(&mut restarted, 700);
    restarted
        .session
        .with_dependent_mut(|_, s| s.load_slot(1, &mut restarted.backend))
        .unwrap();
    assert!(
        restarted
            .session
            .with_dependent_mut(|_, s| s.save_slot(2))
            .unwrap()
    );
    let resaved = fs::read(savedata.join("save002.dat")).unwrap();
    assert_eq!(Csv2Save::decode(&resaved).unwrap().snapshot, saved.snapshot);
    advance(&mut restarted, 90);
    restarted.save_preferences().unwrap();
    drop(restarted);
    println!(
        "CMVS_SAVEDATA_RESTART_OK path={} slot_bytes={}",
        savedata.display(),
        slot.len()
    );
    // Keep evidence only when the caller explicitly selected an output directory.
    if std::env::var_os("CMVS_TEST_TMPDIR").is_none() {
        fs::remove_dir_all(fixture).unwrap();
    }
}
