#[path = "runner/callbacks.rs"]
mod callbacks;
#[path = "support/bytecode.rs"]
#[allow(dead_code)]
mod commands;
#[path = "support/frame_driver.rs"]
mod frame_driver;
#[path = "runner/lifecycle.rs"]
mod lifecycle;
#[path = "runner/snapshots.rs"]
mod snapshots;
#[path = "runner/system_save.rs"]
mod system_save;
use cmvs_rs::{
    digest::{sha256, to_hex},
    platform::{PlatformBackend, PlatformRequest, RecordingBackend},
    profile::{Game, Registry},
    ps3::Script,
    runtime::{Runtime, RuntimeEvent, RuntimeOptions, SaveStorage, SnapshotPolicy, load_start},
    snapshot::SaveImage,
};
use std::{error::Error, fs, path::PathBuf, sync::OnceLock};

/// A synthetic build whose 2048 cases all count as base semantics, so these tests exercise
/// the runner rather than any real profile table.
fn synthetic_profile() -> String {
    let mut text = format!(
        "profile synthetic-base\nfamily cmvs\nbase -\nbinary x64 file=cmvs64.exe sha256={} \
         timestamp=0x00000000 linker=10.0 image-base=0x140000000 table=0x1000 default=0x2000 \
         cases=2048\n",
        "0".repeat(64)
    );
    for command in 0..2048 {
        text.push_str(&format!("0x{:04x} base 0x3000\n", 0x2000 + command));
    }
    text
}
fn base_registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry::parse(&[&synthetic_profile()], "").unwrap())
}

#[test]
fn runner_contract_gate_preserves_unimplemented_handlers() {
    use cmvs_rs::{
        exe::Arch,
        profile::{ContractProof, ContractRules, ContractSide},
    };
    let mut code = Vec::new();
    commands::command(&mut code, 0x27fe, &[123]);
    let root = Root::new("contract-unhandled", &code, b"");
    let base = synthetic_profile();
    let derived = base
        .replace("profile synthetic-base", "profile synthetic-derived")
        .replace("base -", "base synthetic-base")
        .replace(&"0".repeat(64), &"1".repeat(64))
        .replace(" base 0x3000", " unreviewed 0x3000");
    let side = |name: &str, seed| ContractSide {
        profile: name.into(),
        binary_sha256: [seed; 32],
        image_base: 0x140000000,
        case_rva: 0x3000,
        export_sha256: [2 + seed; 32],
        contract_sha256: [4; 32],
    };
    let proof = ContractProof {
        arch: Arch::X64,
        opcode: 0x27fe,
        base: side("synthetic-base", 0),
        derived: side("synthetic-derived", 0x11),
        rules: ContractRules {
            extractor: [5; 32],
            context: [6; 32],
            mappings: [7; 32],
            summaries: [8; 32],
        },
        report_sha256: [9; 32],
        mappings_used: vec![],
        summaries_used: vec![],
    };
    let derived = derived.replace(
        "0x27fe unreviewed 0x3000",
        &format!(
            "0x27fe contract-equivalent 0x3000 contract={}",
            to_hex(&proof.id())
        ),
    );
    let registry = Registry::parse_with_contracts(
        &[&base, &derived],
        "",
        &proof.to_line(),
        std::slice::from_ref(&proof.rules),
    )
    .unwrap();
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

struct Root(PathBuf);
impl Root {
    fn new(name: &str, code: &[u8], text: &[u8]) -> Self {
        let root = std::env::temp_dir().join(format!("cmvs-startup-{}-{name}", std::process::id()));
        fs::create_dir(&root).unwrap();
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
        let bytes: Vec<_> = include_str!("fixtures/cpz6-script.hex")
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
        let hash = |name: &str| to_hex(&sha256(&fs::read(self.0.join(name)).unwrap()));
        let games = format!(
            "game synthetic\nprofile synthetic-base\nstart data/pack/start.ps3 sha256={}\n\
             archive data/pack/script.cpz sha256={}\ngame-id 0x{game_id:08x}\n\
             policy skip-auto-resume integer=2198\nend\n",
            hash("data/pack/start.ps3"),
            hash("data/pack/script.cpz")
        );
        Registry::parse(&[&synthetic_profile()], &games).unwrap()
    }
    fn script(&self) -> Script {
        load_start(&self.0, &self.game().files).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        for name in [
            "data/pack/start.ps3",
            "data/pack/script.cpz",
            "save/system.dat",
            "cmvs.cfg",
        ] {
            let _ = fs::remove_file(self.0.join(name));
        }
        for name in ["data/pack", "data", "save"] {
            let _ = fs::remove_dir(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
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
