//! Move executable code and verify the registered callback through the VM's dispatch path.
use cmvs_assembler::script::{Encoding, assemble};
use cmvs_core::{
    format::ps3::{Container, Script, ScriptFormat},
    profile::{EngineProfile, Registry},
    runtime::script_control::{LoadError, ScriptControl, ScriptSource},
    vm::{
        dispatch::{self, Hosts, ScriptHosts},
        machine::{Machine, Step},
    },
};
use cmvs_disassembler::export_project;

fn fixture(container: Container, call: u16) -> Script {
    // Entry jumps over a yield. The callback target and the label jump both reach PC 54.
    let words: &[u16] = &[
        0x400, 8, 0, 0x2000, 0x440, 4, 11, 0, 0x440, 4, 22, 0, 0x440, 4, 33, 0, 0x440, 4, 54, 0,
        0x440, 4, 5, 0, call, 0x405, 0, 0x2000, 0x201, 0x120, 2, 0, 0x20f, 0x201, 0x125, 0, 0,
        0x20f, 0x200, 0x200, 0x10e, 0, 0, 0x20f, 0x20f,
    ];
    let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    Script::from_segments(
        ScriptFormat {
            container,
            version_raw: 0x20004,
            key: 0x13245678,
            reserved: 0xface,
        },
        0,
        &[54],
        &code,
        b"data\0",
        b"A\0\xff\0",
    )
    .unwrap()
}

fn export(script: &Script, profile: Option<&EngineProfile>) -> String {
    let mut out = Vec::new();
    export_project(script, profile, Encoding::ShiftJis, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

struct NoScripts;
impl ScriptSource for NoScripts {
    fn load(&mut self, _: &[u8]) -> Result<Script, LoadError> {
        panic!("fixture runs in a single script")
    }
}

#[test]
fn profiled_projects_relocate_executable_targets_and_round_trip_both_storage_modes() {
    for (family, selector, call) in [
        (Container::Ps2a, "asukimi", 0x2088),
        (Container::Pss0, "aruto", 0x2028),
    ] {
        let profile = Registry::builtin().resolve_profile(selector).unwrap();
        let original = fixture(family, call);
        let packed = Script::decode(&original.encode_compressed().unwrap()).unwrap();
        let source = export(&packed, Some(profile));
        for mode in [None, Some(false)] {
            let (output, moved) = assemble(source.as_bytes(), None, mode).unwrap();
            let actual = Script::decode(&output).unwrap();
            assert!(!moved);
            assert_eq!(actual.header().compressed_size != 0, mode.is_none());
            assert_eq!(actual.encode().unwrap(), original.encode().unwrap());
        }
        let edited = source
            .replace("code\tp0\t", "code\tprelude\t2000\ncode\tp0\t")
            .replace("label\tl0\t", "label\tnew_label\tp0\nlabel\tl0\t")
            .replace("text\tt0\tA", "text\tt0\tlonger")
            + "append\t7f80\n";
        let (output, moved) = assemble(edited.as_bytes(), None, None).unwrap();
        let actual = Script::decode(&output).unwrap();
        assert!(moved);
        assert_eq!(actual.header().entry_pc, 2);
        assert_eq!(actual.labels(), [2, 56]);
        assert_eq!(actual.data(), b"data\0\x7f\x80");
        assert_eq!(actual.text(), b"longer\0\xff\0");
        let emitted = export(&actual, Some(profile));
        assert!(emitted.contains("0201 0120 $t7 020f"));
        assert!(emitted.contains("0200 0200 010e %l1 020f 020f"));

        let mut host = ScriptControl::for_profile(profile, 123);
        let mut vm = Machine::new(&actual).unwrap();
        vm.set_command_map(profile.command_map());
        for _ in 0..6 {
            assert_eq!(vm.step().unwrap(), Step::Continue);
        }
        assert_eq!(
            vm.step().unwrap(),
            Step::System {
                pc: 50,
                command: 0x88
            }
        );
        dispatch::run(
            profile.semantics(0x88),
            &mut vm,
            &mut Hosts {
                scripts: Some(ScriptHosts {
                    host: &mut host,
                    source: &mut NoScripts,
                }),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(host.callbacks()[&5].target, 56);
        assert_eq!(host.callbacks()[&5].parameters, [11, 22, 33]);
        assert_eq!(vm.step().unwrap(), Step::Continue);
        assert_eq!(vm.pc(), 56);
        assert_eq!(vm.step().unwrap(), Step::Yield);
    }
}

#[test]
fn moving_a_dynamic_callback_requires_its_profile_and_an_explicit_annotation() {
    let original = fixture(Container::Ps2a, 0x2088);
    let raw = export(&original, None);
    assert_eq!(
        assemble(raw.as_bytes(), None, None).unwrap().0,
        original.encode().unwrap()
    );
    let moved = raw.replace("code\tp0\t", "code\tprelude\t2000\ncode\tp0\t");
    assert!(
        assemble(moved.as_bytes(), None, None)
            .unwrap_err()
            .to_string()
            .contains("--profile")
    );
    let selected = Registry::builtin().resolve_profile("asukimi");
    let (bytes, _) = assemble(moved.as_bytes(), selected, None).unwrap();
    assert!(export(&Script::decode(&bytes).unwrap(), selected).contains("0440 0004 @p38"));
    // The target now comes from VM state. The exporter cannot infer the new PC from its value.
    let dynamic = moved.replace(
        "0440 0004 00000036",
        "0200 0103 00000000 020f\ncode\tpush\t0430",
    );
    assert!(
        assemble(dynamic.as_bytes(), selected, None)
            .unwrap_err()
            .to_string()
            .contains("runtime-pc")
    );
    let annotated = dynamic.replace("\t2088\n", "\t2088\truntime-pc\n");
    assemble(annotated.as_bytes(), selected, None).unwrap();
}

#[test]
fn invalid_edits_report_the_operand_or_segment() {
    let script = fixture(Container::Ps2a, 0x2088);
    let source = export(&script, None);
    for (old, replacement, diagnostic) in [
        ("$t2", "$unknown", "undefined symbol"),
        ("&l0", "0000", "symbolic operand"),
        ("2000\n", "2000 2000\n", "one complete instruction"),
        ("data\t0\t6461746100", "data\t0\t6162", "byte length"),
        ("0125 00000000", "0125 000000ff", "Data string"),
        ("text\tt2\t", "text\tother\t", "undefined symbol"),
    ] {
        let edited = source.replacen(old, replacement, 1);
        let error = assemble(edited.as_bytes(), None, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains(diagnostic), "{old}: {error}");
    }
}
