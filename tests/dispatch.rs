use cmvs_rs::{
    dispatch::{self, Flow, TextHosts},
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
        dispatch::run_text(profile.semantics(0x3ac), &mut vm, None).unwrap(),
        Some(Flow::Unhandled)
    );
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x3ac));
    assert_eq!(vm.stack().len_bytes(), before);
    let root = Registry::builtin().profile("cmvs-20160201").unwrap();
    assert_eq!(
        dispatch::run_text(root.semantics(0x3ac), &mut vm, None).unwrap(),
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
    let error = dispatch::run_text(
        registry.profile("cmvs-20171114").unwrap().semantics(0x3ac),
        &mut vm,
        Some(&mut TextHosts {
            messages: &mut messages,
            input: &mut input,
            flags: Default::default(),
        }),
    )
    .unwrap_err();
    assert_eq!(error.reason, "enabled");
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x3ac));
}
