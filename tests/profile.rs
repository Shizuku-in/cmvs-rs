//! Engine profiles: SHA-256, PE switch location, profile tables, selection and the gate.
//! Every executable and table here is synthetic.
mod support;

use cmvs_rs::{
    digest::{Sha256, parse_hex, sha256, sha256_reader, to_hex},
    exe::{Arch, Case, Dispatch, PeImage, find_dispatch},
    machine::Machine,
    profile::{
        CommandGate, CommandStatus, EngineProfile, Evidence, Family, FileEvidence, GateErrorKind,
        Registry, SelectionBasis, SelectionError,
    },
};
use support::bytecode::{command, pending, script};

#[test]
fn sha256_matches_fips_180_4_examples_incrementally_and_from_readers() {
    let cases: [(&[u8], &str); 3] = [
        (
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(to_hex(&sha256(input)), expected);
        assert_eq!(parse_hex(expected), Some(sha256(input)));
    }
    let million = vec![b'a'; 1_000_000];
    let expected = "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0";
    assert_eq!(to_hex(&sha256(&million)), expected);
    assert_eq!(to_hex(&sha256_reader(&million[..]).unwrap()), expected);
    let mut split = Sha256::new();
    for chunk in million.chunks(997) {
        split.update(chunk);
    }
    assert_eq!(to_hex(&split.finish()), expected);
    assert_eq!(parse_hex("00"), None);
    assert_eq!(parse_hex(&"g".repeat(64)), None);
}

const SECTION_RVA: u32 = 0x1000;
const SECTION_FILE: usize = 0x200;

/// A one-section PE whose code is laid out by `layout(rva_of_code_start) -> bytes`.
fn pe(arch: Arch, timestamp: u32, code: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; SECTION_FILE];
    out[..2].copy_from_slice(b"MZ");
    out[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    out[0x40..0x44].copy_from_slice(b"PE\0\0");
    let (machine, optional, magic): (u16, u16, u16) = match arch {
        Arch::X86 => (0x14c, 0xe0, 0x10b),
        Arch::X64 => (0x8664, 0xf0, 0x20b),
    };
    out[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
    out[0x46..0x48].copy_from_slice(&1u16.to_le_bytes());
    out[0x48..0x4c].copy_from_slice(&timestamp.to_le_bytes());
    out[0x54..0x56].copy_from_slice(&optional.to_le_bytes());
    let opt = 0x58;
    out[opt..opt + 2].copy_from_slice(&magic.to_le_bytes());
    out[opt + 2] = 10;
    match arch {
        Arch::X86 => out[opt + 28..opt + 32].copy_from_slice(&0x40_0000u32.to_le_bytes()),
        Arch::X64 => out[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes()),
    }
    let section = opt + optional as usize;
    out[section..section + 5].copy_from_slice(b".text");
    let size = code.len() as u32;
    out[section + 8..section + 12].copy_from_slice(&size.to_le_bytes());
    out[section + 12..section + 16].copy_from_slice(&SECTION_RVA.to_le_bytes());
    out[section + 16..section + 20].copy_from_slice(&size.to_le_bytes());
    out[section + 20..section + 24].copy_from_slice(&(SECTION_FILE as u32).to_le_bytes());
    out[section + 36..section + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());
    out.extend(code);
    out
}

const X64_NOOP: [u8; 11] = [0xb8, 0, 0x40, 0, 0, 0x48, 0x83, 0xc4, 0x20, 0x5b, 0xc3];
const X86_NOOP: [u8; 8] = [0xb8, 0, 0x40, 0, 0, 0xc2, 4, 0];

/// Switch code, then default (`xor eax, eax; ret`), no-op stub, handler and jump table.
/// `kinds`: 0 handler, 1 no-op, 2 default.
fn switch_code(arch: Arch, kinds: &[u8], lea_target: i32) -> Vec<u8> {
    let rva = |offset: usize| SECTION_RVA + offset as u32;
    let mut code = vec![0xcc; 16];
    let limit = (kinds.len() - 1) as u32;
    let (prefix, tail): (&[u8], usize) = match arch {
        Arch::X86 => (&[0x0f, 0xb7, 0x44, 0x24, 0x04], 7),
        Arch::X64 => (&[0x0f, 0xb7, 0xc2, 0x48, 0x8b, 0xd9], 19),
    };
    code.extend(prefix);
    code.push(0x3d);
    code.extend(limit.to_le_bytes());
    let after_ja = code.len() + 2;
    let default = after_ja + tail + 1;
    code.extend([0x77, (default - after_ja) as u8]);
    let table = default + 3 + 16 + 1;
    let base: u32 = if arch == Arch::X86 { 0x40_0000 } else { 0 };
    match arch {
        Arch::X86 => {
            code.extend([0xff, 0x24, 0x85]);
            code.extend((base + rva(table)).to_le_bytes());
        }
        Arch::X64 => {
            code.extend([0x48, 0x8d, 0x0d]);
            let next = rva(code.len() + 4) as i32;
            code.extend((lea_target - next).to_le_bytes());
            code.extend([0x8b, 0x94, 0x81]);
            code.extend(rva(table).to_le_bytes());
            code.extend([0x48, 0x03, 0xd1, 0xff, 0xe2]);
        }
    }
    code.push(0xcc);
    assert_eq!(code.len(), default);
    code.extend([0x33, 0xc0, 0xc3]);
    let noop = code.len();
    let stub: &[u8] = if arch == Arch::X86 {
        &X86_NOOP
    } else {
        &X64_NOOP
    };
    code.extend(stub);
    code.resize(noop + 16, 0xcc);
    let handler = code.len();
    code.push(0xc3);
    assert_eq!(code.len(), table);
    for kind in kinds {
        let target = match kind {
            0 => handler,
            1 => noop,
            _ => default,
        };
        code.extend((base + rva(target)).to_le_bytes());
    }
    code
}

fn located(arch: Arch, kinds: &[u8]) -> (Vec<u8>, PeImage, Dispatch) {
    let bytes = pe(arch, 0x5000_0000, &switch_code(arch, kinds, 0));
    let image = PeImage::parse(&bytes).unwrap();
    let dispatch = find_dispatch(&image, &bytes).unwrap();
    (bytes, image, dispatch)
}

#[test]
fn dispatch_classifies_handlers_noop_stubs_and_default_for_both_architectures() {
    for arch in [Arch::X86, Arch::X64] {
        let (_, image, dispatch) = located(arch, &[0, 1, 2, 0]);
        assert_eq!(image.arch, arch);
        assert_eq!(image.linker, (10, 0));
        assert_eq!(dispatch.cases.len(), 4);
        assert!(matches!(dispatch.cases[0], Case::Handler(_)));
        assert!(matches!(dispatch.cases[1], Case::NoOp(_)));
        assert_eq!(dispatch.cases[2], Case::Default);
        assert_eq!(dispatch.cases[0], dispatch.cases[3]);
        assert_eq!((dispatch.handlers(), dispatch.noops()), (2, 1));
        assert_eq!(
            dispatch.compare_rva,
            SECTION_RVA + 16 + 5 + u32::from(arch == Arch::X64)
        );
    }
}

#[test]
fn dispatch_rejects_missing_duplicate_and_oversized_switches() {
    let code = switch_code(Arch::X64, &[0, 1], 0x10);
    let bytes = pe(Arch::X64, 0, &code);
    // The lea must resolve to the image base, otherwise the shape is not the dispatcher.
    assert!(find_dispatch(&PeImage::parse(&bytes).unwrap(), &bytes).is_err());
    let mut twice = switch_code(Arch::X86, &[0, 1], 0);
    let copy = twice.clone();
    twice.extend(copy);
    let bytes = pe(Arch::X86, 0, &twice);
    let error = find_dispatch(&PeImage::parse(&bytes).unwrap(), &bytes).unwrap_err();
    assert_eq!(error.0, "several system-command switch candidates");
    let mut code = switch_code(Arch::X86, &[0, 1], 0);
    code[16 + 6..16 + 10].copy_from_slice(&0x800u32.to_le_bytes());
    let bytes = pe(Arch::X86, 0, &code);
    assert!(find_dispatch(&PeImage::parse(&bytes).unwrap(), &bytes).is_err());
    assert!(PeImage::parse(b"MZ").is_err());
}

/// One build with both executables; `kinds` as in `switch_code`.
fn build(id: &str, base: Option<&str>, kinds: &[u8], seed: u8) -> EngineProfile {
    let (_, x64_image, x64) = located(Arch::X64, kinds);
    let (_, x86_image, x86) = located(Arch::X86, kinds);
    EngineProfile::from_dispatch(
        id,
        Family::Cmvs,
        base,
        vec![
            ("cmvs64.exe".into(), [seed; 32], &x64_image, &x64),
            ("cmvs32.exe".into(), [seed + 1; 32], &x86_image, &x86),
        ],
    )
    .unwrap()
}

#[test]
fn profile_text_round_trips_and_marks_present_commands_by_profile_kind() {
    let root = build("synthetic-root", None, &[0, 1, 2, 0], 1);
    let derived = build(
        "synthetic-derived",
        Some("synthetic-root"),
        &[0, 1, 2, 0],
        3,
    );
    assert_eq!(
        root.statuses(),
        [
            CommandStatus::Base,
            CommandStatus::NoOp,
            CommandStatus::Absent,
            CommandStatus::Base
        ]
    );
    assert_eq!(derived.status(0), CommandStatus::Unreviewed);
    assert_eq!(derived.status(1), CommandStatus::NoOp);
    // Commands beyond the switch are absent.
    assert_eq!(derived.status(4), CommandStatus::Absent);
    assert_eq!(derived.status(0x7ff), CommandStatus::Absent);
    let registry = Registry::parse(&[&root.to_text(), &derived.to_text()], "").unwrap();
    assert_eq!(registry.profile("synthetic-root"), Some(&root));
    assert_eq!(registry.profile("synthetic-derived"), Some(&derived));
    assert_eq!(
        registry
            .binary(&[4; 32])
            .map(|(p, b)| (p.id.as_str(), b.arch)),
        Some(("synthetic-derived", Arch::X86))
    );
}

#[test]
fn profile_data_parses_the_same_with_crlf_line_endings() {
    let root = build("synthetic-root", None, &[0, 1, 2, 0], 1).to_text();
    let derived = build(
        "synthetic-derived",
        Some("synthetic-root"),
        &[0, 1, 2, 0],
        3,
    )
    .to_text();
    let records = games(&[7; 32], &[8; 32], &[3; 32]);
    let crlf = |text: &str| text.replace('\n', "\r\n");
    assert_eq!(
        Registry::parse(&[&crlf(&root), &crlf(&derived)], &crlf(&records)).unwrap(),
        Registry::parse(&[&root, &derived], &records).unwrap()
    );
}

#[test]
fn inherited_commands_require_identity_bound_proofs_and_pass_the_default_gate() {
    let root = build("synthetic-root", None, &[0, 1, 2, 0], 1).to_text();
    let derived = build(
        "synthetic-derived",
        Some("synthetic-root"),
        &[0, 1, 2, 0],
        3,
    )
    .to_text();
    let header = format!("inherit x64 base-sha256={}", to_hex(&[1; 32]));
    let proof = format!("inherit=x64:{}", to_hex(&[9; 32]));
    let inherited = derived
        .lines()
        .map(|line| {
            if line.starts_with("0x2000 ") {
                format!("{} {proof}", line.replace("unreviewed", "inherited"))
            } else if line.starts_with("binary x86 ") {
                format!("{line}\n{header}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let registry = Registry::parse(&[&root, &inherited], "").unwrap();
    let profile = registry.profile("synthetic-derived").unwrap();
    assert_eq!(profile.status(0), CommandStatus::Inherited);
    let parsed = profile.inheritance(0).unwrap();
    assert_eq!(parsed.fingerprint, [9; 32]);
    // The derived exe is implied by the profile's own binary of the proof architecture.
    assert_eq!(
        (parsed.base_sha256, parsed.derived_sha256),
        ([1; 32], [3; 32])
    );
    let canonical = profile.to_text();
    assert!(canonical.contains(&header));
    assert_eq!(Registry::parse(&[&root, &canonical], "").unwrap(), registry);
    let selection = registry
        .select(Some("synthetic-derived"), &Evidence::default())
        .unwrap();
    let mut gate = CommandGate::new(selection);
    let mut code = vec![];
    command(&mut code, 0x201d, &[0]);
    let program = script(&code);
    let (vm, pc) = pending_machine(&program);
    assert_eq!(gate.check(&vm, pc, 0), Ok(None));
    assert!(gate.allowed().is_empty());
    assert_eq!(gate.counts().inherited, 1);
    assert_eq!(gate.counts().base, 0);
    assert_eq!(gate.counts().unreviewed, 0);
    assert!(gate.check(&vm, pc, 3).is_err());
    assert_eq!(gate.counts().unreviewed, 0);
    gate.set_allow_unreviewed(true);
    assert_eq!(gate.check(&vm, pc, 3), Ok(Some(3)));
    assert_eq!(gate.counts().unreviewed, 1);
    for invalid in [
        inherited.replace(&proof, ""),
        inherited.replace("inherit=x64", "inherit=arm64"),
        // The proof must name the root exe of the same architecture.
        inherited.replace(
            &header,
            &header.replace(&to_hex(&[1; 32]), &to_hex(&[2; 32])),
        ),
        inherited.replace(&header, &header.replace("x64", "x86")),
        inherited.replace(&header, ""),
        inherited.replace(&header, &format!("{header}\n{header}")),
        inherited.replace("inherit x64", "inherit arm64"),
        inherited.replace(&proof, &format!("{proof}:{}", to_hex(&[3; 32]))),
        // A header without inherited rows is stale evidence.
        inherited
            .replace(&format!(" {proof}"), "")
            .replace("0x2000 inherited", "0x2000 unreviewed"),
        inherited.replace("0x2000 inherited", "0x2000 unreviewed"),
        inherited.replace("inherited", "base"),
    ] {
        assert!(Registry::parse(&[&root, &invalid], "").is_err());
    }
    let root_no_handler = root.replace("0x2000 base", "0x2000 noop");
    assert!(Registry::parse(&[&root_no_handler, &inherited], "").is_err());
}

#[test]
fn profile_tables_reject_contradictions_and_mixed_builds() {
    let root = build("synthetic-root", None, &[0, 1, 2], 1).to_text();
    let reject = |text: String, needle: &str| {
        let error = Registry::parse(&[&text], "").unwrap_err().to_string();
        assert!(error.contains(needle), "{error}");
    };
    reject(
        root.replace("0x2000 base", "0x2000 unreviewed"),
        "contradicts",
    );
    reject(
        root.replace("0x2002 absent - -", "0x2002 base - -"),
        "lack an address",
    );
    reject(
        root.replace("0x2002 absent - -", "0x2002 absent 0x10 -"),
        "have no address",
    );
    reject(root.replace("0x2001 noop", "0x2003 noop"), "consecutive");
    reject(root.replace("cases=3", "cases=4"), "do not match");
    reject(
        root.replacen("timestamp=0x50000000", "timestamp=0x50001000", 1),
        "minutes apart",
    );
    reject(root.replace("base -", "base missing"), "contradicts");
    reject(
        build("synthetic-derived", Some("missing"), &[0, 1, 2], 1).to_text(),
        "missing base",
    );
    reject(root.replace("family cmvs", "family cvns"), "unknown family");
    // x86 and x64 disagreeing on one command are not one build.
    let (_, x64_image, x64) = located(Arch::X64, &[0, 1]);
    let (_, x86_image, x86) = located(Arch::X86, &[0, 0]);
    let error = EngineProfile::from_dispatch(
        "synthetic-mixed",
        Family::Cmvs,
        None,
        vec![
            ("cmvs64.exe".into(), [1; 32], &x64_image, &x64),
            ("cmvs32.exe".into(), [2; 32], &x86_image, &x86),
        ],
    )
    .unwrap_err();
    assert!(error.to_string().contains("not one build"), "{error}");
}

fn games(start: &[u8; 32], archive: &[u8; 32], exe: &[u8; 32]) -> String {
    format!(
        "game synthetic\nprofile synthetic-derived\nstart data/pack/start.ps3 sha256={}\n\
         archive data/pack/script.cpz sha256={}\ngame-id 0x12345678\n\
         executable cmvs64.exe sha256={}\nend\n",
        to_hex(start),
        to_hex(archive),
        to_hex(exe)
    )
}

fn registry() -> Registry {
    let root = build("synthetic-root", None, &[0, 1, 2, 0], 1).to_text();
    let derived = build(
        "synthetic-derived",
        Some("synthetic-root"),
        &[0, 1, 2, 0],
        3,
    )
    .to_text();
    Registry::parse(&[&root, &derived], &games(&[7; 32], &[8; 32], &[3; 32])).unwrap()
}

fn file(path: &str, byte: u8) -> FileEvidence {
    FileEvidence {
        path: path.into(),
        sha256: [byte; 32],
    }
}

#[test]
fn game_records_must_reference_binaries_of_their_profile() {
    let root = build("synthetic-root", None, &[0, 1], 1).to_text();
    let derived = build("synthetic-derived", Some("synthetic-root"), &[0, 1], 3).to_text();
    let error = Registry::parse(&[&root, &derived], &games(&[7; 32], &[8; 32], &[1; 32]))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("is not a binary of profile synthetic-derived"),
        "{error}"
    );
    let twice = games(&[7; 32], &[8; 32], &[3; 32]).repeat(2);
    assert!(Registry::parse(&[&root, &derived], &twice).is_err());
    assert!(Registry::parse(&[&root, &root], "").is_err());
}

#[test]
fn selection_needs_data_evidence_and_confirms_every_present_executable() {
    let registry = registry();
    let data = Evidence {
        start_script: Some(file("start.ps3", 7)),
        script_archive: Some(file("script.cpz", 8)),
        executables: vec![],
    };
    let chosen = registry.select(None, &data).unwrap();
    assert_eq!(chosen.profile.id, "synthetic-derived");
    assert_eq!(chosen.basis, SelectionBasis::Data);
    assert_eq!(chosen.record.unwrap().game, "synthetic");
    assert!(chosen.log_lines()[0].starts_with("# engine profile synthetic-derived"));
    let confirmed = Evidence {
        executables: vec![file("cmvs64.exe", 3)],
        ..data.clone()
    };
    assert_eq!(
        registry.select(None, &confirmed).unwrap().basis,
        SelectionBasis::DataAndBinary
    );
    let patched = Evidence {
        executables: vec![file("cmvs64.exe", 9)],
        ..data.clone()
    };
    assert!(matches!(
        registry.select(None, &patched),
        Err(SelectionError::UnknownBinary { .. })
    ));
    // An archive from another game contradicts the start script.
    let mixed = Evidence {
        script_archive: Some(file("script.cpz", 9)),
        ..data.clone()
    };
    assert!(matches!(
        registry.select(None, &mixed),
        Err(SelectionError::UnknownGame(_))
    ));
    assert!(matches!(
        registry.select(None, &Evidence::default()),
        Err(SelectionError::NoDataEvidence)
    ));
    assert!(matches!(
        registry.select(Some("nope"), &data),
        Err(SelectionError::UnknownProfile(_))
    ));
    // Explicit selection keeps a matching record and drops one of another profile.
    let explicit = registry
        .select(Some("synthetic-derived"), &patched)
        .unwrap();
    assert_eq!(explicit.basis, SelectionBasis::Explicit);
    assert!(explicit.record.is_some());
    let other = registry.select(Some("synthetic-root"), &data).unwrap();
    assert!(other.record.is_none());
    assert!(
        registry
            .select(Some("synthetic-root"), &Evidence::default())
            .is_ok()
    );
}

/// A machine stopped at the script's first system command, and that command's PC.
fn pending_machine(program: &cmvs_rs::ps3::Script) -> (Machine<'_>, usize) {
    let mut vm = Machine::new(program).unwrap();
    pending(&mut vm);
    let (pc, _) = vm.pending_system().unwrap();
    (vm, pc)
}

#[test]
fn gate_rejects_absent_noop_unreviewed_and_mismatched_game_ids() {
    let registry = registry();
    let data = Evidence {
        start_script: Some(file("start.ps3", 7)),
        ..Evidence::default()
    };
    let mut code = vec![];
    command(&mut code, 0x201d, &[0x1234_5678]);
    let recorded = script(&code);
    let (vm, pc) = pending_machine(&recorded);
    let mut gate = CommandGate::new(registry.select(None, &data).unwrap());
    let mut kind = |vm: &Machine, command| gate.check(vm, pc, command).map_err(|e| e.kind);
    assert_eq!(kind(&vm, 0), Err(GateErrorKind::Unreviewed));
    assert_eq!(kind(&vm, 1), Err(GateErrorKind::NoOp));
    assert_eq!(kind(&vm, 2), Err(GateErrorKind::Absent));
    assert_eq!(kind(&vm, 0x7ff), Err(GateErrorKind::Absent));
    // The recorded game ID passes its check; 0x201d is then beyond this four-case switch.
    assert_eq!(kind(&vm, 0x1d), Err(GateErrorKind::Absent));
    let mut code = vec![];
    command(&mut code, 0x201d, &[0x8765_4321]);
    let other = script(&code);
    let (other_vm, _) = pending_machine(&other);
    assert_eq!(
        kind(&other_vm, 0x1d),
        Err(GateErrorKind::GameIdMismatch {
            expected: 0x1234_5678,
            found: 0x8765_4321
        })
    );
    gate.set_allow_unreviewed(true);
    let mut kind = |command| gate.check(&vm, pc, command).map_err(|e| e.kind);
    assert_eq!(kind(0), Ok(Some(0)));
    assert_eq!(kind(0), Ok(None));
    assert_eq!(kind(3), Ok(Some(3)));
    assert_eq!(kind(1), Err(GateErrorKind::NoOp));
    assert_eq!(gate.allowed().iter().copied().collect::<Vec<_>>(), [0, 3]);
    // Without a matched record no game ID is enforced.
    let mut root = CommandGate::new(registry.select(Some("synthetic-root"), &data).unwrap());
    assert_eq!(root.check(&vm, pc, 0).map_err(|e| e.kind), Ok(None));
    let message = root.check(&other_vm, pc, 0x1d).unwrap_err().to_string();
    assert!(
        message.contains("0x201d at entry.ps3 / 0x") && message.contains("does not exist"),
        "{message}"
    );
}

#[test]
fn embedded_profiles_parse_and_bind_the_recorded_games() {
    let registry = Registry::builtin();
    let base = registry.profile("cmvs-20160201").unwrap();
    let chrono = registry.profile("cmvs-20150405").unwrap();
    assert_eq!(base.base, None);
    assert_eq!(chrono.base.as_deref(), Some("cmvs-20160201"));
    for (profile, handlers, noops) in [(base, 721, 255), (chrono, 712, 264)] {
        assert_eq!(profile.statuses().len(), 976);
        assert_eq!(profile.binaries.len(), 2);
        assert_eq!(
            profile.count(CommandStatus::Base)
                + profile.count(CommandStatus::Inherited)
                + profile.count(CommandStatus::Unreviewed),
            handlers
        );
        assert_eq!(profile.count(CommandStatus::NoOp), noops);
    }
    // Base semantics exist for 0x211e, which the older build only stubs out.
    assert_eq!(base.status(0x11e), CommandStatus::Base);
    assert_eq!(chrono.status(0x11e), CommandStatus::NoOp);
    let games: Vec<_> = registry
        .games()
        .iter()
        .map(|g| (g.game.as_str(), g.profile.as_str(), g.skip_auto_resume))
        .collect();
    assert_eq!(
        games,
        [
            ("amatsutsumi", "cmvs-20160201", Some(2198)),
            ("chronoclock", "cmvs-20150405", None)
        ]
    );
}

#[test]
fn game_files_are_found_by_fixed_names_only() {
    use cmvs_rs::profile::GameFiles;
    let root = std::env::temp_dir().join(format!("cmvs-profile-files-{}", std::process::id()));
    std::fs::create_dir_all(root.join("data/pack")).unwrap();
    assert!(matches!(
        GameFiles::discover(&root),
        Err(SelectionError::MissingFile(_))
    ));
    std::fs::write(root.join("data/pack/start.ps2"), b"PS2A").unwrap();
    assert!(GameFiles::discover(&root).is_err());
    std::fs::write(root.join("data/pack/script.cpz"), b"CPZ6").unwrap();
    let files = GameFiles::discover(&root).unwrap();
    assert_eq!(files.start_name(), b"start.ps2");
    std::fs::write(root.join("data/pack/start.ps3"), b"PS2A").unwrap();
    assert!(matches!(
        GameFiles::discover(&root),
        Err(SelectionError::AmbiguousStartScript)
    ));
    for name in ["start.ps2", "start.ps3", "script.cpz"] {
        std::fs::remove_file(root.join("data/pack").join(name)).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}
