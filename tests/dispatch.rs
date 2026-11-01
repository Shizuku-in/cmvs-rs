use cmvs_rs::{
    dispatch::{self, Flow, Hosts, TextHosts},
    input::ActionInput,
    machine::Machine,
    message::MessageHost,
    profile::{CommandGate, Registry},
};
mod support;
use support::bytecode::{command, pending, script};

#[test]
fn migrated_tables_have_unique_owners_and_overrides_are_build_bound() {
    for table in [dispatch::ROOT, dispatch::FONT_CELLS] {
        let owners: std::collections::BTreeSet<_> = table.iter().map(|e| e.command).collect();
        assert_eq!(owners.len(), table.len());
    }
    let registry = Registry::builtin();
    for id in ["cmvs-20160201", "cmvs-20150405"] {
        assert!(dispatch::FONT_CELLS.iter().all(|entry| {
            registry.profile(id).unwrap().semantics(entry.command)
                == cmvs_rs::profile::Semantics::Root
        }));
    }
    let derived = registry.profile("cmvs-20171114").unwrap();
    for entry in dispatch::FONT_CELLS {
        assert_eq!(
            derived.semantics(entry.command),
            cmvs_rs::profile::Semantics::ResearchFontCells
        );
        assert_eq!(
            derived.status(entry.command),
            cmvs_rs::profile::CommandStatus::Unreviewed
        );
    }
}

#[test]
fn missing_text_capability_preserves_pending_override_and_never_uses_root() {
    let mut code = vec![];
    command(&mut code, 0x23ac, &[1]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let profile = Registry::builtin().profile("cmvs-20171114").unwrap();
    let before = vm.stack().len_bytes();
    assert_eq!(
        dispatch::run(profile.semantics(0x3ac), &mut vm, &mut Hosts::default()).unwrap(),
        Some(Flow::Unavailable)
    );
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x3ac));
    assert_eq!(vm.stack().len_bytes(), before);
    let root = Registry::builtin().profile("cmvs-20160201").unwrap();
    assert_eq!(
        dispatch::run(root.semantics(0x3ac), &mut vm, &mut Hosts::default()).unwrap(),
        None
    );
}

#[test]
fn new_command_still_requires_research_gate_and_short_stack_is_transactional() {
    let mut code = vec![];
    command(&mut code, 0x23ac, &[]);
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    pending(&mut vm);
    let registry = Registry::builtin();
    let selection = registry
        .select(
            Some("cmvs-20171114"),
            &cmvs_rs::profile::Evidence::default(),
        )
        .unwrap();
    let mut gate = CommandGate::new(selection);
    let (pc, command) = vm.pending_system().unwrap();
    assert!(gate.check(&vm, pc, command).is_err());
    gate.set_allow_unreviewed(true);
    assert_eq!(gate.check(&vm, pc, command).unwrap(), Some(0x3ac));
    let mut messages = MessageHost::default();
    let mut input = ActionInput::default();
    let error = dispatch::run(
        registry.profile("cmvs-20171114").unwrap().semantics(0x3ac),
        &mut vm,
        &mut Hosts {
            text: Some(TextHosts {
                messages: &mut messages,
                input: &mut input,
                flags: Default::default(),
            }),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.reason, "enabled");
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x3ac));
}

#[test]
fn state_entries_preserve_missing_capabilities_and_execute_once_when_available() {
    use cmvs_rs::{
        dispatch::Capability, message_flags::MessageFlags, profile::Semantics, random::CrtRandom,
        timers::VmTimers,
    };
    for entry in dispatch::ROOT
        .iter()
        .filter(|entry| entry.capability != Capability::Text)
    {
        let args: &[u32] = match entry.command {
            0x0b | 0x0c => &[2],
            0xd4 => &[10],
            0x2d0 | 0x2e5 | 0x2eb => &[1, 1],
            0x340 => &[1],
            _ => &[],
        };
        let mut code = vec![];
        command(&mut code, entry.command + 0x2000, args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        vm.state_mut().set_system_integer(0, 42).unwrap();
        let before = (vm.pc(), vm.stack().len_bytes());
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!((vm.pc(), vm.stack().len_bytes()), before);
        assert_eq!(vm.state().system_integer(0), Some(42));
        let mut timers = VmTimers::default();
        timers.begin_run(120);
        let mut flags = MessageFlags::default();
        let mut random = CrtRandom::new(0);
        let mut expected_random = random.clone();
        let random_result = expected_random.next_value() % 10;
        let mut hosts = Hosts {
            timers: (entry.capability == Capability::Timers).then_some(&mut timers),
            flags: (entry.capability == Capability::Flags).then_some(&mut flags),
            random: (entry.capability == Capability::Random).then_some(&mut random),
            ..Default::default()
        };
        // An explicitly selected semantic source cannot fall back to these root entries.
        assert_eq!(
            dispatch::run(Semantics::ResearchFontCells, &mut vm, &mut hosts).unwrap(),
            Some(Flow::Unhandled)
        );
        assert_eq!((vm.pc(), vm.stack().len_bytes()), before);
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut hosts).unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(vm.pending_system(), None);
        assert_eq!(vm.pc(), script.code().len());
        assert_eq!(vm.stack().len_bytes(), 0);
        let expected_result = match entry.command {
            0x08 | 0x09 | 0x0c | 0x0f => 120,
            0xd4 => random_result,
            _ => 42,
        };
        assert_eq!(vm.state().system_integer(0), Some(expected_result));
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut hosts).unwrap(),
            None
        );
        assert_eq!(
            timers.values()[2],
            if entry.command == 0x0b { 0 } else { 120 }
        );
        assert_eq!(flags.skip(), entry.command == 0x2eb);
        assert_eq!(flags.auto_advance(), entry.command == 0x2e5);
        assert_eq!(flags.bypass(), entry.command == 0x340);
        assert_eq!(flags.mode_1548(), u32::from(entry.command == 0x2d0));
        assert_eq!(
            random,
            if entry.command == 0xd4 {
                expected_random
            } else {
                CrtRandom::new(0)
            }
        );
    }
}

#[test]
fn state_dispatch_errors_name_arguments_and_preserve_vm_and_hosts() {
    use cmvs_rs::{
        message_flags::MessageFlags, profile::Semantics, random::CrtRandom, timers::VmTimers,
    };
    for (opcode, args, reason) in [
        (0x200b, vec![], "timer slot"),
        (0x200c, vec![10], "timer slot outside 0..9"),
        (0x20d4, vec![], "divisor"),
        (0x20d4, vec![0], "rand() % 0 faults in the original"),
        (0x22d0, vec![1], "value"),
        (0x22eb, vec![0], "unused value"),
        (0x22e5, vec![], "mode"),
        (0x2340, vec![], "bypass"),
    ] {
        let mut code = vec![];
        command(&mut code, opcode, &args);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let before = (vm.pc(), vm.stack().len_bytes(), vm.pending_system());
        vm.state_mut().set_system_integer(0, 42).unwrap();
        let mut timers = VmTimers::default();
        timers.begin_run(19);
        let mut flags = MessageFlags::default();
        let mut random = CrtRandom::new(7);
        let error = dispatch::run(
            Semantics::Root,
            &mut vm,
            &mut Hosts {
                timers: Some(&mut timers),
                flags: Some(&mut flags),
                random: Some(&mut random),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.reason, reason);
        assert_eq!(
            (vm.pc(), vm.stack().len_bytes(), vm.pending_system()),
            before
        );
        assert_eq!(vm.state().system_integer(0), Some(42));
        assert_eq!(timers.values(), &[19; 10]);
        assert_eq!(flags, MessageFlags::default());
        assert_eq!(random, CrtRandom::new(7));
    }
}

#[test]
fn shared_flag_queries_discard_unknown_value_words_without_reading_them() {
    use cmvs_rs::{message_flags::MessageFlags, profile::Semantics};
    for opcode in [0x22d0u16, 0x22e5, 0x22eb] {
        let mut code = vec![];
        // Reserving eight bytes initializes only the top word. After the mode
        // push, the query's second word exists but has unknown bytes.
        code.extend(0x440u16.to_le_bytes());
        code.extend(8u16.to_le_bytes());
        code.extend(77u32.to_le_bytes());
        command(&mut code, opcode, &[0]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        pending(&mut vm);
        let mut flags = MessageFlags::default();
        assert_eq!(
            dispatch::run(
                Semantics::Root,
                &mut vm,
                &mut Hosts {
                    flags: Some(&mut flags),
                    ..Default::default()
                }
            )
            .unwrap(),
            Some(Flow::Done)
        );
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.state().system_integer(0), Some(0));
        assert_eq!(flags, MessageFlags::default());
    }
}
