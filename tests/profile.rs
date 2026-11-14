//! Engine profiles: SHA-256, PE switch location, profile tables, selection and the gate.
//! Every executable and table here is synthetic.
mod support;

use cmvs_rs::{
    digest::{Sha256, parse_hex, sha256, sha256_reader, to_hex},
    exe::{Arch, Case, Dispatch, PeImage, find_dispatch},
    machine::Machine,
    profile::{
        CommandGate, CommandStatus, ContractProof, ContractRules, ContractSide, EngineProfile,
        Evidence, Family, FileEvidence, GateErrorKind, Registry, SelectionBasis, SelectionError,
    },
    vm::SystemResult,
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

/// Unoptimized x86 dispatcher whose cases are `mov ecx, [ebp-4]; call f; jmp end` stubs.
/// `kinds` as in `switch_code`; returns the code and the handler and no-op RVAs.
fn frame_switch_code(kinds: &[u8], stub_tail: u8) -> (Vec<u8>, u32, u32) {
    const BASE: u32 = 0x40_0000;
    let rva = |offset: usize| SECTION_RVA + offset as u32;
    let mut code = vec![0xcc; 16];
    code.extend([
        0x55, 0x8b, 0xec, 0x83, 0xec, 0x08, 0x89, 0x4d, 0xfc, 0x0f, 0xb7, 0x45, 0x08, 0x89, 0x45,
        0xf8, 0x81, 0x7d, 0xf8,
    ]);
    code.extend(((kinds.len() - 1) as u32).to_le_bytes());
    code.extend([0x0f, 0x87]);
    let ja = code.len();
    code.extend([0; 4]);
    code.extend([0x8b, 0x4d, 0xf8, 0xff, 0x24, 0x8d]);
    let table_operand = code.len();
    code.extend([0; 4]);
    let stubs = code.len();
    code.resize(stubs + 10 * kinds.len(), 0xcc);
    let end = code.len();
    // Default `xor eax, eax`, then the shared epilogue the stubs jump to.
    code.extend([0x33, 0xc0, 0x8b, 0xe5, 0x5d, 0xc2, 0x04, 0x00]);
    let ja_rel = (end - (ja + 4)) as u32;
    code[ja..ja + 4].copy_from_slice(&ja_rel.to_le_bytes());
    let noop = code.len();
    code.extend([
        0x55, 0x8b, 0xec, 0x51, 0x89, 0x4d, 0xfc, 0xb8, 0x00, 0x40, 0x00, 0x00, 0x8b, 0xe5, 0x5d,
        0xc3,
    ]);
    let handler = code.len();
    code.push(0xc3);
    let table = code.len();
    code[table_operand..table_operand + 4].copy_from_slice(&(BASE + rva(table)).to_le_bytes());
    for (i, kind) in kinds.iter().enumerate() {
        let stub = stubs + 10 * i;
        let target = match kind {
            2 => end,
            _ => {
                let callee = if *kind == 0 { handler } else { noop };
                code[stub..stub + 4].copy_from_slice(&[0x8b, 0x4d, 0xfc, 0xe8]);
                let rel = callee as i32 - (stub + 8) as i32;
                code[stub + 4..stub + 8].copy_from_slice(&rel.to_le_bytes());
                code[stub + 8..stub + 10].copy_from_slice(&[stub_tail, 0x00]);
                stub
            }
        };
        code.extend((BASE + rva(target)).to_le_bytes());
    }
    (code, rva(handler), rva(noop))
}

#[test]
fn dispatch_follows_member_call_stubs_of_unoptimized_builds() {
    let (code, handler, noop) = frame_switch_code(&[0, 1, 2, 0], 0xeb);
    let bytes = pe(Arch::X86, 0x4987_3a5b, &code);
    let image = PeImage::parse(&bytes).unwrap();
    let dispatch = find_dispatch(&image, &bytes).unwrap();
    assert_eq!(
        dispatch.cases,
        [
            Case::Handler(handler),
            Case::NoOp(noop),
            Case::Default,
            Case::Handler(handler)
        ]
    );
    assert_eq!(dispatch.compare_rva, SECTION_RVA + 16 + 16);
    // A table entry that is not a member-call stub is rejected rather than guessed.
    let (mut code, _, _) = frame_switch_code(&[0, 1], 0xeb);
    code[55] = 0x90;
    let bytes = pe(Arch::X86, 0, &code);
    let error = find_dispatch(&PeImage::parse(&bytes).unwrap(), &bytes).unwrap_err();
    assert_eq!(error.0, "switch case is not a member-call stub");
    // The same bytes in an x64 image are not this dispatcher.
    let (code, _, _) = frame_switch_code(&[0, 1], 0xe9);
    let bytes = pe(Arch::X64, 0, &code);
    assert!(find_dispatch(&PeImage::parse(&bytes).unwrap(), &bytes).is_err());
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
fn machine_proofs_bind_both_exes_and_legacy_presumed_needs_its_own_policy() {
    let root = build("synthetic-root", None, &[0, 1, 2, 0], 1).to_text();
    let derived = build(
        "synthetic-derived",
        Some("synthetic-root"),
        &[0, 1, 2, 0],
        3,
    )
    .to_text();
    let header = format!("compare x64 base-sha256={}", to_hex(&[1; 32]));
    for (name, key, status, other) in [
        ("inherited", "inherit", CommandStatus::Inherited, "presume"),
        ("presumed", "presume", CommandStatus::Presumed, "inherit"),
    ] {
        let proof = format!("{key}=x64:{}", to_hex(&[9; 32]));
        let row = format!("0x2000 {name}");
        let text = derived
            .lines()
            .map(|line| {
                if line.starts_with("0x2000 ") {
                    format!("{} {proof}", line.replace("0x2000 unreviewed", &row))
                } else if line.starts_with("binary x86 ") {
                    format!("{line}\n{header}")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let registry = Registry::parse(&[&root, &text], "").unwrap();
        let profile = registry.profile("synthetic-derived").unwrap();
        assert_eq!(profile.status(0), status);
        let parsed = profile.proof(0).unwrap();
        assert_eq!(parsed.digest, [9; 32]);
        // The derived exe is implied by the profile's own binary of the proof architecture.
        assert_eq!(
            (parsed.base_sha256, parsed.derived_sha256),
            ([1; 32], [3; 32])
        );
        let canonical = profile.to_text();
        assert!(canonical.contains(&header) && canonical.contains(&proof));
        assert_eq!(Registry::parse(&[&root, &canonical], "").unwrap(), registry);
        let selection = registry
            .select(Some("synthetic-derived"), &Evidence::default())
            .unwrap();
        let mut gate = CommandGate::new(selection);
        gate.set_block_unreviewed(true);
        gate.set_block_presumed(true);
        let mut code = vec![];
        command(&mut code, 0x201d, &[0]);
        let program = script(&code);
        let (vm, pc) = pending_machine(&program);
        if status == CommandStatus::Presumed {
            assert_eq!(
                gate.check(&vm, pc, 0).unwrap_err().kind,
                GateErrorKind::Presumed
            );
            assert_eq!(gate.counts().presumed, 0);
            gate.set_block_unreviewed(false);
            assert_eq!(
                gate.check(&vm, pc, 0).unwrap_err().kind,
                GateErrorKind::Presumed
            );
            gate.set_block_unreviewed(true);
            gate.set_block_presumed(false);
        }
        assert_eq!(gate.check(&vm, pc, 0), Ok(None));
        assert!(gate.allowed().is_empty());
        let counts = gate.counts();
        assert_eq!(
            (counts.inherited, counts.presumed),
            match status {
                CommandStatus::Inherited => (1, 0),
                _ => (0, 1),
            }
        );
        assert_eq!((counts.base, counts.unreviewed), (0, 0));
        assert_eq!(gate.presumed().len(), counts.presumed as usize);
        assert_eq!(
            gate.presumed_line().is_some(),
            status == CommandStatus::Presumed
        );
        assert!(gate.check(&vm, pc, 3).is_err());
        gate.set_block_unreviewed(false);
        assert_eq!(gate.check(&vm, pc, 3), Ok(Some(3)));
        assert_eq!(gate.counts().unreviewed, 1);
        for invalid in [
            text.replace(&proof, ""),
            text.replace(&format!("{key}=x64"), &format!("{key}=arm64")),
            // Each status takes its own proof key.
            text.replace(&format!("{key}=x64"), &format!("{other}=x64")),
            // The proof must name the root exe of the same architecture.
            text.replace(
                &header,
                &header.replace(&to_hex(&[1; 32]), &to_hex(&[2; 32])),
            ),
            text.replace(&header, &header.replace("x64", "x86")),
            text.replace(&header, ""),
            text.replace(&header, &format!("{header}\n{header}")),
            text.replace("compare x64", "compare arm64"),
            text.replace(&proof, &format!("{proof}:{}", to_hex(&[3; 32]))),
            // A header without proof rows is stale evidence.
            text.replace(&format!(" {proof}"), "")
                .replace(&row, "0x2000 unreviewed"),
            text.replace(&row, "0x2000 unreviewed"),
            text.replace(&row, "0x2000 base"),
        ] {
            assert!(Registry::parse(&[&root, &invalid], "").is_err());
        }
        let root_no_handler = root.replace("0x2000 base", "0x2000 noop");
        assert!(Registry::parse(&[&root_no_handler, &text], "").is_err());
    }
}

fn contract_fixture() -> (EngineProfile, EngineProfile, ContractProof) {
    let root = build("synthetic-root", None, &[0, 1, 2, 0], 1);
    let derived = build(
        "synthetic-derived",
        Some("synthetic-root"),
        &[0, 1, 2, 0],
        3,
    );
    let side = |profile: &EngineProfile, export| {
        let binary = &profile.binaries[0];
        let Case::Handler(case_rva) = binary.cases[0] else {
            panic!("fixture")
        };
        ContractSide {
            profile: profile.id.clone(),
            binary_sha256: binary.sha256,
            image_base: binary.image_base,
            case_rva,
            export_sha256: [export; 32],
            contract_sha256: [9; 32],
        }
    };
    let proof = ContractProof {
        arch: Arch::X64,
        opcode: 0x2000,
        base: side(&root, 5),
        derived: side(&derived, 6),
        rules: ContractRules {
            extractor: [10; 32],
            context: [11; 32],
            mappings: [12; 32],
            summaries: [13; 32],
        },
        report_sha256: [14; 32],
        mappings_used: vec!["vm.flag".into()],
        summaries_used: vec![],
    };
    (root, derived, proof)
}

fn certified_text(profile: &EngineProfile, proof: &ContractProof) -> String {
    profile
        .to_text()
        .lines()
        .map(|l| {
            if l.starts_with("0x2000 ") {
                format!(
                    "{} contract={}",
                    l.replace("unreviewed", "contract-equivalent"),
                    to_hex(&proof.id())
                )
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn contract_certificates_bind_round_trip_and_count_attempts_without_completing_commands() {
    let (root, derived, proof) = contract_fixture();
    let text = certified_text(&derived, &proof);
    let registry = Registry::parse_with_contracts(
        &[&root.to_text(), &text],
        "",
        &proof.to_line(),
        std::slice::from_ref(&proof.rules),
    )
    .unwrap();
    let profile = registry.profile("synthetic-derived").unwrap();
    assert_eq!(profile.status(0), CommandStatus::ContractEquivalent);
    assert_eq!(profile.proof(0), None);
    assert_eq!(profile.review(0), None);
    assert_eq!(profile.contract_id(0), Some(&proof.id()));
    assert_eq!(registry.contract_proof(&proof.id()), Some(&proof));
    assert_eq!(
        Registry::parse_with_contracts(
            &[&root.to_text(), &profile.to_text()],
            "",
            &proof.to_line(),
            std::slice::from_ref(&proof.rules)
        )
        .unwrap(),
        registry
    );
    assert!(Registry::parse(&[&root.to_text(), &text], "").is_err());
    let selection = registry
        .select(Some("synthetic-derived"), &Evidence::default())
        .unwrap();
    let mut gate = CommandGate::new(selection);
    gate.set_block_unreviewed(true);
    gate.set_block_presumed(true);
    let mut code = vec![];
    command(&mut code, 0x201d, &[0]);
    let program = script(&code);
    let (vm, pc) = pending_machine(&program);
    let pending = vm.pending_system();
    for _ in 0..2 {
        assert_eq!(gate.check(&vm, pc, 0), Ok(None));
    }
    assert_eq!(gate.counts().contract_equivalent, 2);
    assert_eq!(gate.contracted().iter().copied().collect::<Vec<_>>(), [0]);
    assert!(gate.diagnostic_lines().join("\n").contains("0x2000"));
    assert_eq!(vm.pending_system(), pending);
    assert_eq!(
        gate.check(&vm, pc, 1).unwrap_err().kind,
        GateErrorKind::NoOp
    );
    assert_eq!(
        gate.check(&vm, pc, 2).unwrap_err().kind,
        GateErrorKind::Absent
    );
    assert_eq!(
        gate.check(&vm, pc, 3).unwrap_err().kind,
        GateErrorKind::Unreviewed
    );
}

#[test]
fn contract_registry_rejects_wrong_identity_domain_rules_and_revoked_evidence() {
    let (root, derived, proof) = contract_fixture();
    let reject = |p: &ContractProof, catalog: String, active: &[ContractRules]| {
        assert!(
            Registry::parse_with_contracts(
                &[&root.to_text(), &certified_text(&derived, p)],
                "",
                &catalog,
                active
            )
            .is_err()
        );
    };
    for mutate in [
        |p: &mut ContractProof| p.opcode += 1,
        |p: &mut ContractProof| p.base.profile = "other-root".into(),
        |p: &mut ContractProof| p.derived.profile = "other-derived".into(),
        |p: &mut ContractProof| p.base.binary_sha256 = [2; 32], // other architecture
        |p: &mut ContractProof| p.derived.binary_sha256 = [4; 32],
        |p: &mut ContractProof| p.base.image_base += 1,
        |p: &mut ContractProof| p.derived.image_base += 1,
        |p: &mut ContractProof| p.base.case_rva += 1,
        |p: &mut ContractProof| p.derived.case_rva += 1,
        |p: &mut ContractProof| p.derived.contract_sha256 = [15; 32],
        |p: &mut ContractProof| p.arch = Arch::X86,
        |p: &mut ContractProof| p.mappings_used.push("vm.flag".into()),
    ] {
        let mut changed = proof.clone();
        mutate(&mut changed);
        reject(
            &changed,
            changed.to_line(),
            std::slice::from_ref(&proof.rules),
        );
    }
    for mutate in [
        |r: &mut ContractRules| r.extractor = [0; 32],
        |r: &mut ContractRules| r.context = [0; 32],
        |r: &mut ContractRules| r.mappings = [0; 32],
        |r: &mut ContractRules| r.summaries = [0; 32],
    ] {
        let mut active = proof.rules.clone();
        mutate(&mut active);
        reject(&proof, proof.to_line(), &[active]);
    }
    for bad in [
        proof.to_line().replace("proof v1", "proof v2"),
        proof
            .to_line()
            .replace("coverage=closed", "coverage=incomplete"),
        proof
            .to_line()
            .replace("binary=checked", "binary=unchecked"),
        proof
            .to_line()
            .replace("result=equivalent", "result=different"),
        proof
            .to_line()
            .replace("model=cmvs-command-contract-v1", "model=unknown"),
        format!("{}\n{}", proof.to_line(), proof.to_line()),
        format!("{}\nrevoked {}", proof.to_line(), to_hex(&proof.id())),
        String::new(),
    ] {
        reject(&proof, bad, std::slice::from_ref(&proof.rules));
    }
    reject(&proof, proof.to_line(), &[]);
    // Retain a revoked certificate for audit even after its old rules are retired.
    // Downgrading the profile removes all authorization; the old proof cannot resolve.
    let revoked = format!("{}\nrevoked {}", proof.to_line(), to_hex(&proof.id()));
    let downgraded =
        Registry::parse_with_contracts(&[&root.to_text(), &derived.to_text()], "", &revoked, &[])
            .unwrap();
    assert_eq!(downgraded.contract_proof(&proof.id()), None);
    assert_eq!(
        downgraded.profile("synthetic-derived").unwrap().status(0),
        CommandStatus::Unreviewed
    );
    // An identical certificate cannot be reused for a different opcode or a chain.
    let text = certified_text(&derived, &proof);
    let extra = text.lines().find(|l| l.starts_with("0x2003 ")).unwrap();
    let with_other = text.replace(
        extra,
        &format!(
            "{} contract={}",
            extra.replace("unreviewed", "contract-equivalent"),
            to_hex(&proof.id())
        ),
    );
    assert!(
        Registry::parse_with_contracts(
            &[&root.to_text(), &with_other],
            "",
            &proof.to_line(),
            std::slice::from_ref(&proof.rules)
        )
        .is_err()
    );
}

#[test]
fn reviewed_commands_require_a_review_record_and_pass_the_default_gate() {
    let root = build("synthetic-root", None, &[0, 1, 2, 0], 1).to_text();
    let derived = build(
        "synthetic-derived",
        Some("synthetic-root"),
        &[0, 1, 2, 0],
        3,
    )
    .to_text();
    let record = "review=games/synthetic/vm-semantics.md";
    let reviewed = derived.replace("0x2000 unreviewed 0x", "0x2000 reviewed 0x");
    let line = reviewed
        .lines()
        .find(|l| l.starts_with("0x2000 "))
        .unwrap()
        .to_owned();
    let reviewed = reviewed.replace(&line, &format!("{line} {record}"));
    let registry = Registry::parse(&[&root, &reviewed], "").unwrap();
    let profile = registry.profile("synthetic-derived").unwrap();
    assert_eq!(profile.status(0), CommandStatus::Reviewed);
    assert_eq!(profile.review(0), Some("games/synthetic/vm-semantics.md"));
    assert_eq!(
        Registry::parse(&[&root, &profile.to_text()], "").unwrap(),
        registry
    );
    let selection = registry
        .select(Some("synthetic-derived"), &Evidence::default())
        .unwrap();
    let mut gate = CommandGate::new(selection);
    gate.set_block_unreviewed(true);
    gate.set_block_presumed(true);
    let mut code = vec![];
    command(&mut code, 0x201d, &[0]);
    let program = script(&code);
    let (vm, pc) = pending_machine(&program);
    assert_eq!(gate.check(&vm, pc, 0), Ok(None));
    assert_eq!(gate.counts().reviewed, 1);
    assert_eq!(gate.counts().inherited + gate.counts().unreviewed, 0);
    for invalid in [
        reviewed.replace(&format!(" {record}"), ""),
        reviewed.replace(record, "review=../vm-semantics.md"),
        reviewed.replace(record, "review=/games/x.md"),
        reviewed.replace(record, "review=games/synthetic/vm-semantics.txt"),
        reviewed.replace(record, "review=games//vm-semantics.md"),
        reviewed.replace("0x2000 reviewed", "0x2000 unreviewed"),
        // A review record on any other status, or a review of a command the root lacks.
        reviewed.replace("0x2000 reviewed", "0x2000 inherited"),
    ] {
        assert!(Registry::parse(&[&root, &invalid], "").is_err());
    }
    let root_noop = root.replace("0x2000 base", "0x2000 noop");
    assert!(Registry::parse(&[&root_noop, &reviewed], "").is_err());
}

/// A CVNS build over the CMVS root `[base, noop, absent, base]`; `0x2001` maps to `0x2003`.
fn cvns_texts() -> (String, String) {
    let root = build("synthetic-root", None, &[0, 1, 2, 0], 1).to_text();
    let (_, image, dispatch) = located(Arch::X86, &[0, 0, 1, 0]);
    let cvns = EngineProfile::from_dispatch(
        "synthetic-cvns",
        Family::Cvns,
        Some("synthetic-root"),
        vec![("cvns.exe".into(), [9; 32], &image, &dispatch)],
    )
    .unwrap()
    .to_text();
    let line = cvns
        .lines()
        .find(|l| l.starts_with("0x2001 "))
        .unwrap()
        .to_owned();
    let mapped =
        line.replace("unreviewed", "reviewed") + " map=0x2003 review=games/synthetic/commands.md";
    (root, cvns.replace(&line, &mapped))
}

#[test]
fn cvns_profiles_translate_reviewed_commands_and_block_the_rest() {
    let (root, cvns) = cvns_texts();
    assert!(cvns.contains("family cvns\nbase synthetic-root\n"));
    let registry = Registry::parse(&[&root, &cvns], "").unwrap();
    let profile = registry.profile("synthetic-cvns").unwrap();
    assert_eq!(profile.to_text(), cvns);
    assert_eq!(profile.status(1), CommandStatus::Reviewed);
    assert_eq!(profile.map(1), Some(3));
    assert_eq!(profile.map(3), None);
    let map = profile.command_map().unwrap();
    assert_eq!((map.runtime(1), map.runtime(3), map.len()), (3, 3, 1));
    assert!(
        registry
            .profile("synthetic-root")
            .unwrap()
            .command_map()
            .is_none()
    );

    let mut code = vec![];
    command(&mut code, 0x2001, &[]);
    command(&mut code, 0x2003, &[]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    vm.set_command_map(profile.command_map());
    pending(&mut vm);
    let (pc, runtime) = vm.pending_system().unwrap();
    assert_eq!((runtime, vm.pending_source()), (3, Some(1)));
    let selection = registry
        .select(Some("synthetic-cvns"), &Evidence::default())
        .unwrap();
    let mut gate = CommandGate::new(selection.clone());
    assert_eq!(gate.check(&vm, pc, runtime), Ok(None));
    assert_eq!(gate.counts().reviewed, 1);

    // An unmapped command keeps its number and stops even though unreviewed CMVS
    // commands would pass the default policy.
    vm.complete_system(SystemResult(0x4000)).unwrap();
    pending(&mut vm);
    let (pc, runtime) = vm.pending_system().unwrap();
    assert_eq!((runtime, vm.pending_source()), (3, Some(3)));
    let error = gate.check(&vm, pc, runtime).unwrap_err();
    assert_eq!((error.kind, error.command), (GateErrorKind::Unmapped, 3));
    assert!(
        error.to_string().contains("0x2003") && error.to_string().contains("no reviewed mapping"),
        "{error}"
    );

    // A VM without the profile's translation never runs a CVNS command.
    let mut plain = Machine::new(&program).unwrap();
    pending(&mut plain);
    let (pc, runtime) = plain.pending_system().unwrap();
    let mut gate = CommandGate::new(selection);
    assert_eq!(
        gate.check(&plain, pc, runtime).map_err(|e| e.kind),
        Err(GateErrorKind::Unmapped)
    );
}

#[test]
fn cvns_maps_need_a_review_and_root_semantics() {
    let (root, cvns) = cvns_texts();
    let map = " map=0x2003 ";
    for (invalid, needle) in [
        // Reviewed without a map, or a map on another status.
        (cvns.replace(map, " "), "carry a map"),
        (
            cvns.replace("0x2001 reviewed", "0x2001 unreviewed"),
            "carry a map",
        ),
        // Only absent/noop/unreviewed/reviewed exist for CVNS.
        (
            cvns.replace("0x2003 unreviewed", "0x2003 base"),
            "CVNS commands are",
        ),
        // The target must be a root Base command or a registered CVNS-only handler.
        (cvns.replace(map, " map=0x2002 "), "no root semantics"),
        (cvns.replace(map, " map=0x2001 "), "no root semantics"),
        (cvns.replace(map, " map=0x2600 "), "no root semantics"),
        (
            cvns.replace(map, " map=0x2800 "),
            "outside the system opcodes",
        ),
        (cvns.replace(map, " map=0x1003 "), "0x2000-based"),
        (
            cvns.replace("base synthetic-root", "base -"),
            "derives from a CMVS root",
        ),
    ] {
        let error = Registry::parse(&[&root, &invalid], "")
            .unwrap_err()
            .to_string();
        assert!(error.contains(needle), "{needle}: {error}");
    }
    // A CMVS profile cannot carry a map.
    let derived = build(
        "synthetic-derived",
        Some("synthetic-root"),
        &[0, 1, 2, 0],
        3,
    )
    .to_text();
    let line = derived
        .lines()
        .find(|l| l.starts_with("0x2000 "))
        .unwrap()
        .to_owned();
    let mapped =
        line.replace("unreviewed", "reviewed") + " map=0x2003 review=games/synthetic/commands.md";
    let error = Registry::parse(&[&root, &derived.replace(&line, &mapped)], "")
        .unwrap_err()
        .to_string();
    assert!(error.contains("only reviewed CVNS"), "{error}");
    // CVNS derives from a CMVS root only, never from another CVNS profile.
    let chained = cvns
        .replace("profile synthetic-cvns", "profile synthetic-cvns-2")
        .replace("base synthetic-root", "base synthetic-cvns")
        .replace("sha256=0909", "sha256=0808");
    assert!(Registry::parse(&[&root, &cvns, &chained], "").is_err());
}

/// Review records point at a `docs/` section headed by the command, so the evidence a
/// profile cites cannot silently disappear.

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
    reject(root.replace("family cmvs", "family pvns"), "unknown family");
    reject(
        root.replace("family cmvs", "family cvns"),
        "derives from a CMVS root",
    );
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

#[test]
fn modified_data_requires_unique_consistent_known_executables() {
    let registry = registry();
    let patched = Evidence {
        start_script: Some(file("start.ps3", 20)),
        script_archive: Some(file("script.cpz", 21)),
        executables: vec![file("cmvs64.exe", 3)],
    };
    let selected = registry.select(None, &patched).unwrap();
    assert_eq!(selected.basis, SelectionBasis::Binary);
    assert_eq!(selected.record.unwrap().game, "synthetic");
    assert_eq!(selected.profile.id, "synthetic-derived");
    assert!(selected.evidence.last().unwrap().contains("modified data"));
    assert!(
        registry
            .select(Some("synthetic-root"), &patched)
            .unwrap()
            .record
            .is_none()
    );
    for executables in [
        vec![],
        vec![file("cmvs64.exe", 9)],
        vec![file("cmvs64.exe", 3), file("cmvs32.exe", 1)],
        vec![file("cmvs64.exe", 3), file("cmvs32.exe", 9)],
    ] {
        assert!(matches!(
            registry.select(
                None,
                &Evidence {
                    executables,
                    ..patched.clone()
                }
            ),
            Err(SelectionError::UnknownGame(_))
        ));
    }
    let root = registry.profile("synthetic-root").unwrap().to_text();
    let derived = registry.profile("synthetic-derived").unwrap().to_text();
    let original = games(&[7; 32], &[8; 32], &[3; 32]);
    // Shared engine binaries do not uniquely identify a game or save directory.
    let ambiguous = games(&[10; 32], &[11; 32], &[3; 32]).replace("game synthetic", "game other");
    let ambiguous = Registry::parse(&[&root, &derived], &format!("{original}{ambiguous}")).unwrap();
    assert!(matches!(
        ambiguous.select(None, &patched),
        Err(SelectionError::UnknownGame(_))
    ));
    let other = games(&[10; 32], &[11; 32], &[1; 32])
        .replace("game synthetic", "game other")
        .replace("profile synthetic-derived", "profile synthetic-root");
    let conflicting = Registry::parse(&[&root, &derived], &format!("{original}{other}")).unwrap();
    for (start, archive) in [(10, 21), (20, 11), (10, 11)] {
        let evidence = Evidence {
            start_script: Some(file("start.ps3", start)),
            script_archive: Some(file("script.cpz", archive)),
            ..patched.clone()
        };
        assert!(conflicting.select(None, &evidence).is_err());
    }
}

#[test]
fn startup_variants_keep_one_identity_with_known_executable_fallback() {
    let root = build("synthetic-root", None, &[0, 1], 1).to_text();
    let derived = build("synthetic-derived", Some("synthetic-root"), &[0, 1], 3).to_text();
    let original = games(&[7; 32], &[8; 32], &[3; 32]);
    let extra = format!("start data/pack/start.ps3 sha256={}\n", to_hex(&[9; 32]));
    let records = original.replace("archive ", &format!("{extra}archive "));
    let registry = Registry::parse(&[&root, &derived], &records).unwrap();
    assert_eq!(registry.games().len(), 1);
    assert_eq!(registry.games()[0].start_scripts.len(), 2);
    for start in [None, Some(file("start.ps3", 7)), Some(file("start.ps3", 9))] {
        let evidence = Evidence {
            start_script: start,
            script_archive: Some(file("script.cpz", 8)),
            executables: vec![file("cmvs64.exe", 3)],
        };
        assert_eq!(
            registry.select(None, &evidence).unwrap().record,
            Some(&registry.games()[0])
        );
        assert_eq!(
            registry
                .select(None, &evidence)
                .unwrap()
                .record
                .unwrap()
                .game,
            "synthetic"
        );
    }
    assert_eq!(
        registry
            .select(
                None,
                &Evidence {
                    start_script: Some(file("start.ps3", 10)),
                    script_archive: Some(file("script.cpz", 8)),
                    executables: vec![file("cmvs64.exe", 3)],
                }
            )
            .unwrap()
            .basis,
        SelectionBasis::Binary
    );
    // Neither a variant hash nor an archive may identify a second save identity.
    for (start, archive) in [(9, 10), (10, 8)] {
        let other =
            games(&[start; 32], &[archive; 32], &[3; 32]).replace("game synthetic", "game other");
        assert!(Registry::parse(&[&root, &derived], &format!("{records}{other}")).is_err());
    }
    let repeated = records.replace("archive ", &format!("{extra}archive "));
    assert!(Registry::parse(&[&root, &derived], &repeated).is_err());
    let missing_start = original
        .lines()
        .filter(|line| !line.starts_with("start "))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(Registry::parse(&[&root, &derived], &missing_start).is_err());
}

#[test]
fn both_amatsutsumi_startups_select_the_existing_save_identity() {
    let registry = Registry::builtin();
    let archive = FileEvidence {
        path: "script.cpz".into(),
        sha256: parse_hex("92b367e8c3d2791f1dd2ca8426581974fbb640d2766f2e82ab12383d2f5359e4")
            .unwrap(),
    };
    for start in [
        "c338313949a803e834acf0d746c93f09c4d41f558af8fb6c13ab1ee55aff2bd0",
        "14a5f200dc0c8dfc00cbc1c3293d09849fd4da060ecdc1d6cbced031691a4986",
    ] {
        let evidence = Evidence {
            start_script: Some(FileEvidence {
                path: "start.ps3".into(),
                sha256: parse_hex(start).unwrap(),
            }),
            script_archive: Some(archive.clone()),
            executables: Vec::new(),
        };
        let selection = registry.select(None, &evidence).unwrap();
        let game = selection.record.unwrap();
        assert_eq!(game.game, "amatsutsumi");
        assert_eq!(game.game_id, 0x1780_0016);
        assert_eq!(game.skip_auto_resume, Some(2198));
        assert_eq!(selection.profile.id, "cmvs-20160201");
    }
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
    gate.set_block_unreviewed(true);
    gate.set_block_presumed(true);
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
    gate.set_block_unreviewed(false);
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
