mod callbacks;
use crate::support::{bytecode as commands, profile::synthetic_profile};
mod frame_driver;
mod lifecycle;
mod snapshots;
mod system_save;
use cmvs_core::{
    format::digest::sha256,
    format::ps3::Script,
    format::snapshot::SaveImage,
    host::platform::PlatformBackend,
    host::platform::PlatformRequest,
    host::platform::RecordingBackend,
    profile::{CommandStatus, EngineProfile, Family, Game, GameRecord, RecordFile, Registry},
    runtime::Runtime,
    runtime::RuntimeEvent,
    runtime::RuntimeOptions,
    runtime::SaveStorage,
    runtime::SnapshotPolicy,
    runtime::load_start,
};
use std::{error::Error, fs, sync::OnceLock};

fn base_registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry::new(vec![synthetic_profile("synthetic-base")], vec![]))
}

#[test]
fn runner_contract_gate_preserves_unimplemented_handlers() {
    let mut code = Vec::new();
    commands::command(&mut code, 0x27fe, &[123]);
    let root = Root::new("contract-unhandled", &code, b"");
    let mut statuses = vec![CommandStatus::Unreviewed; 2048];
    statuses[0x7fe] = CommandStatus::ContractEquivalent;
    let derived = EngineProfile::new(
        "synthetic-derived".into(),
        Family::Cmvs,
        Some("synthetic-base".into()),
        vec![],
        statuses,
        vec![],
        vec![],
    );
    let registry = Registry::new(vec![synthetic_profile("synthetic-base"), derived], vec![]);
    let game = registry
        .identify_game(&root.0, Some("synthetic-derived"))
        .unwrap();
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    let mut backend = RecordingBackend::default();
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::Unhandled {
            command: 0x27fe,
            ..
        }
    ));
    let pending = run.machine().pending_system();
    assert_eq!(run.machine().stack().peek_u32(4).unwrap(), 123);
    assert_eq!(run.gate().counts().contract_equivalent, 1);
    assert!(matches!(
        run.tick(&mut backend).unwrap(),
        RuntimeEvent::Unhandled {
            command: 0x27fe,
            ..
        }
    ));
    assert_eq!(run.machine().pending_system(), pending);
    assert_eq!(run.gate().counts().contract_equivalent, 2);
}

#[test]
fn runner_legacy_presumed_policy_keeps_pending_state_on_rejection() {
    let mut code = Vec::new();
    commands::command(&mut code, 0x200e, &[]);
    let root = Root::new("legacy-presumed-gate", &code, b"");
    let game = Registry::builtin()
        .identify_game(&root.0, Some("cmvs-20150405"))
        .unwrap();
    let script = root.script();
    let mut run = Runtime::with_options(
        &script,
        &root.0,
        &game,
        RuntimeOptions {
            block_presumed: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut backend = RecordingBackend::default();
    assert!(
        run.tick(&mut backend)
            .unwrap_err()
            .to_string()
            .contains("legacy presumed")
    );
    let pc = run.machine().pc();
    let pending = run.machine().pending_system();
    run.set_block_unreviewed(false);
    assert!(
        run.tick(&mut backend)
            .unwrap_err()
            .to_string()
            .contains("legacy presumed")
    );
    assert_eq!(run.machine().pc(), pc);
    assert_eq!(run.machine().pending_system(), pending);
    assert_eq!(run.gate().counts().presumed, 0);
    run.set_block_presumed(false);
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(run.gate().counts().presumed, 1);
}

#[test]
fn unknown_derived_layout_does_not_enable_the_root_gameplay_snapshot_format() {
    let mut code = Vec::new();
    commands::command(&mut code, 0x2000, &[]);
    let root = Root::new("derived-snapshot-layout", &code, b"");
    let registry = Registry::builtin();
    let mut game = registry
        .identify_game(&root.0, Some("cmvs-20150405"))
        .unwrap();
    let mut unsupported = game.selection.profile.clone();
    unsupported.id = "synthetic-unsupported-derived".into();
    game.selection.profile = &unsupported;
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    let pc = run.machine().pc();
    let stack = run.machine().stack().len_bytes();
    let mut backend = RecordingBackend::default();
    for error in [
        run.inspect_snapshot().unwrap_err(),
        run.load_slot(0, &mut backend).unwrap_err(),
    ] {
        assert!(
            error
                .to_string()
                .contains("snapshot layout is not implemented")
        );
    }
    assert_eq!(run.machine().pc(), pc);
    assert_eq!(run.machine().stack().len_bytes(), stack);
    assert!(matches!(run.held_snapshot(), SaveImage::Absent));
    assert!(!root.0.join("save/save998.dat").exists());
}

#[test]
fn reviewed_unknown_derived_scenario_point_keeps_the_capture_format_barrier() {
    let mut code = Vec::new();
    commands::command(&mut code, 0x22b2, &[]);
    let root = Root::new("derived-scenario-point", &code, b"");
    let mut game = Registry::builtin()
        .identify_game(&root.0, Some("cmvs-20150405"))
        .unwrap();
    let mut unsupported = game.selection.profile.clone();
    unsupported.id = "synthetic-unsupported-derived".into();
    game.selection.profile = &unsupported;
    let script = root.script();
    let mut run = Runtime::new(&script, &root.0, &game, 0).unwrap();
    let mut backend = RecordingBackend::default();
    let pc = run.machine().pc();
    assert!(
        run.tick(&mut backend)
            .unwrap_err()
            .to_string()
            .contains("snapshot layout is not implemented")
    );
    assert_eq!(run.machine().pc(), pc);
    assert_eq!(run.machine().pending_system(), Some((pc, 0x2b2)));
    assert!(matches!(run.held_snapshot(), SaveImage::Absent));
    run.set_snapshot_policy(SnapshotPolicy::ContinueWithoutImage);
    assert_eq!(run.tick(&mut backend).unwrap(), RuntimeEvent::Progress);
    assert_eq!(run.machine().pc(), pc + 2);
    assert_eq!(run.machine().pending_system(), None);
    assert_eq!(run.machine().repeat_count(), 0);
    assert_eq!(run.machine().stack().len_bytes(), 0);
    assert!(matches!(run.held_snapshot(),SaveImage::Unavailable { pc:next,.. } if *next==pc+2));
    assert_eq!(run.gate().counts().reviewed, 2);
}

struct Root(crate::support::temp_dir::TempDir);
impl Root {
    fn new(name: &str, code: &[u8], text: &[u8]) -> Self {
        let root = crate::support::temp_dir::TempDir::new(&format!("startup-{name}"));
        fs::create_dir(root.join("data")).unwrap();
        fs::create_dir(root.join("data/pack")).unwrap();
        fs::create_dir(root.join("save")).unwrap();
        let mut raw = b"PS2A".to_vec();
        for n in [
            48u32,
            0,
            0,
            0,
            code.len() as u32,
            0,
            text.len() as u32,
            0,
            0,
            (code.len() + text.len()) as u32,
            0,
        ] {
            raw.extend(n.to_le_bytes());
        }
        raw.extend(
            code.iter()
                .chain(text)
                .map(|b| b.rotate_left(1).wrapping_add(124)),
        );
        fs::write(root.join("data/pack/start.ps3"), raw).unwrap();
        let bytes: Vec<_> = include_str!("../../fixtures/cpz6-script.hex")
            .trim()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        fs::write(root.join("data/pack/script.cpz"), bytes).unwrap();
        Self(root)
    }
    /// Synthetic roots match no game record, so they name the synthetic profile explicitly.
    fn game(&self) -> Game<'static> {
        base_registry()
            .identify_game(&self.0, Some("synthetic-base"))
            .unwrap()
    }
    /// A registry whose record matches this root's files and verifies skip-auto-resume.
    fn recorded_registry(&self, game_id: u32) -> Registry {
        let file = |path: &str| RecordFile {
            path: path.into(),
            sha256: sha256(&fs::read(self.0.join(path)).unwrap()),
        };
        let record = GameRecord {
            game: "synthetic".into(),
            profile: "synthetic-base".into(),
            start_scripts: vec![file("data/pack/start.ps3")],
            script_archive: file("data/pack/script.cpz"),
            game_id,
            executables: vec![],
            skip_auto_resume: Some(2198),
        };
        Registry::new(vec![synthetic_profile("synthetic-base")], vec![record])
    }
    fn script(&self) -> Script {
        load_start(&self.0, &self.game().files).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0.with_extension("user"));
        let _ = fs::remove_file(self.0.with_extension("css1"));
    }
}
fn string_call(op: u16) -> Vec<u8> {
    let mut b = 0x201u16.to_le_bytes().to_vec();
    b.extend(0x120u16.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend(0x20fu16.to_le_bytes());
    b.extend(0x430u16.to_le_bytes());
    b.extend(op.to_le_bytes());
    b
}

// Synthetic CSS1, literal-only LZSS; no game bytes or external assets.
fn auto_resume_save(game_id: u32) -> Vec<u8> {
    let mut payload = vec![0u8; 44864];
    payload[32512 + 4 * (2198 - 2048)..32516 + 4 * (2198 - 2048)]
        .copy_from_slice(&1i32.to_le_bytes());
    payload[32512 + 4 * (2199 - 2048)..32516 + 4 * (2199 - 2048)]
        .copy_from_slice(&77i32.to_le_bytes());
    payload[10] = 4; // Persistent bit 2130 remains set.
    let mut packed = Vec::new();
    for chunk in payload.chunks(8) {
        packed.push(((1u16 << chunk.len()) - 1) as u8);
        packed.extend_from_slice(chunk);
    }
    let mut raw = vec![0u8; 360];
    raw[..4].copy_from_slice(b"CSS1");
    for (offset, value) in [
        (4, 104),
        (68, game_id),
        (92, 44864),
        (96, packed.len() as u32),
        (100, 360 + packed.len() as u32),
    ] {
        raw[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    raw[76] = 1;
    let key = b"YUKO_KAWAI_PURPLE_SYSTEM_SAVE_CHECK";
    for (i, byte) in packed.iter().enumerate() {
        raw.push((byte.wrapping_add(64) ^ key[i % key.len()]).rotate_left(2));
    }
    let checksum = raw[360..]
        .iter()
        .fold(0u16, |sum, &b| sum.wrapping_add(u16::from(b)));
    raw.extend(checksum.to_le_bytes());
    raw
}

/// Pushes `args` so that `args[0]` lies at SP-4, then issues `op`.
fn system_call(code: &mut Vec<u8>, op: u16, args: &[u32]) {
    for value in args.iter().rev() {
        code.extend(0x440u16.to_le_bytes());
        code.extend(4u16.to_le_bytes());
        code.extend(value.to_le_bytes());
    }
    code.extend(op.to_le_bytes());
}
