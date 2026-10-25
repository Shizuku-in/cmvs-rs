use cmvs_rs::{
    bytecode::{self, InstructionKind},
    machine::{Machine, Step},
    ps3::Script,
    script_control::{ControlEvent, LoadError, ScriptControl, ScriptSource},
    vm::SystemResult,
};
use std::collections::BTreeMap;

fn word(b: &mut Vec<u8>, v: u16) {
    b.extend(v.to_le_bytes());
}
fn scalar(tag: u16, v: u32) -> Vec<u8> {
    let mut b = tag.to_le_bytes().to_vec();
    b.extend(v.to_le_bytes());
    b
}
fn expr(kind: u16, terms: Vec<Vec<u8>>) -> Vec<u8> {
    let mut b = kind.to_le_bytes().to_vec();
    for t in terms {
        b.extend(t);
    }
    word(&mut b, 0x20f);
    b
}
fn int(v: u32) -> Vec<u8> {
    expr(0x200, vec![scalar(0x100, v)])
}
fn text(v: u32) -> Vec<u8> {
    expr(0x201, vec![scalar(0x120, v)])
}
fn push(b: &mut Vec<u8>, e: Vec<u8>) {
    b.extend(e);
    word(b, 0x430);
}
fn assign(index: u32, v: u32) -> Vec<u8> {
    let mut r = 0x101u16.to_le_bytes().to_vec();
    r.extend(int(index));
    expr(
        0x200,
        vec![r, scalar(0x100, v), 0x170u16.to_le_bytes().to_vec()],
    )
}
fn common_call(b: &mut Vec<u8>, offset: u32, slot: u32) {
    push(b, text(offset));
    push(b, int(slot));
    word(b, 0x2081);
}
fn return_script(b: &mut Vec<u8>) {
    word(b, 0x414);
    word(b, 0);
}
fn raw(code: &[u8], labels: &[u32], text: &[u8], entry: u32) -> Vec<u8> {
    let mut p = vec![];
    for label in labels {
        p.extend(label.to_le_bytes());
    }
    p.extend(code);
    p.extend(text);
    let mut r = b"PS2A".to_vec();
    for n in [
        48,
        0,
        0,
        labels.len() as u32,
        code.len() as u32,
        0,
        text.len() as u32,
        entry,
        0,
        p.len() as u32,
        0,
    ] {
        r.extend(n.to_le_bytes());
    }
    r.extend(p.into_iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    r
}
fn script(code: &[u8], labels: &[u32], text: &[u8], entry: u32) -> Script {
    Script::decode(&raw(code, labels, text, entry)).unwrap()
}

#[derive(Default)]
struct MemorySource {
    files: BTreeMap<Vec<u8>, Vec<u8>>,
    calls: usize,
}
impl MemorySource {
    fn add(&mut self, name: &[u8], code: &[u8], labels: &[u32], text: &[u8], entry: u32) {
        self.files
            .insert(name.to_vec(), raw(code, labels, text, entry));
    }
}
impl ScriptSource for MemorySource {
    fn load(&mut self, name: &[u8]) -> Result<Script, LoadError> {
        self.calls += 1;
        let raw = self.files.get(name).ok_or("missing synthetic script")?;
        Ok(Script::decode(raw)?)
    }
}
fn pending(vm: &mut Machine<'_>) -> Step {
    for _ in 0..1000 {
        let event = vm.step().unwrap();
        if event != Step::Continue {
            return event;
        }
    }
    panic!("budget");
}
fn run(
    vm: &mut Machine<'_>,
    host: &mut ScriptControl,
    source: &mut MemorySource,
) -> Option<ControlEvent> {
    for _ in 0..1000 {
        match vm.step().unwrap() {
            Step::Continue => {}
            Step::Yield => return None,
            Step::System { .. } => {
                let event = host
                    .try_handle(vm, source)
                    .unwrap()
                    .expect("unhandled synthetic command");
                if event == ControlEvent::MainPreparedNeedsSnapshot {
                    return Some(event);
                }
            }
        }
    }
    panic!("budget");
}

#[test]
fn common_initialization_uses_header_entry_and_restores_caller_stack_and_pc() {
    let mut root = vec![];
    common_call(&mut root, 0, 0);
    let resume = root.len();
    word(&mut root, 0x2000);
    let root = script(&root, &[0], b"a.ps3\0", 0);
    let mut vm = Machine::new(&root).unwrap();
    vm.state_mut().set_integer(7, 42).unwrap();
    let mut host = ScriptControl::new(123);
    let mut source = MemorySource::default();
    let mut common = vec![];
    word(&mut common, 0x2000);
    common.extend(assign(7, 99));
    return_script(&mut common);
    source.add(b"a.ps3", &common, &[0], &[], 2);
    pending(&mut vm);
    assert_eq!(
        host.try_handle(&mut vm, &mut source).unwrap(),
        Some(ControlEvent::CommonEntered { slot: 1 })
    );
    assert_eq!(vm.current_slot(), 1);
    assert_eq!(vm.pc(), 2);
    assert_eq!(vm.stack().len_bytes(), 12);
    assert_eq!(vm.stack().peek_u32(4).unwrap(), 0);
    assert_eq!(vm.stack().peek_u32(8).unwrap(), resume as u32);
    vm.step().unwrap();
    vm.step().unwrap();
    assert_eq!(vm.current_slot(), 0);
    assert_eq!(vm.pc(), resume);
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().integer(7), Some(99));
    assert_eq!(vm.loaded_names().len(), 2);
}

#[test]
fn nested_common_scripts_return_to_their_actual_callers() {
    let mut root = vec![];
    common_call(&mut root, 0, 0);
    word(&mut root, 0x2000);
    let root = script(&root, &[0], b"a.ps3\0", 0);
    let mut vm = Machine::new(&root).unwrap();
    let mut a = vec![];
    common_call(&mut a, 0, 1);
    return_script(&mut a);
    let mut b = assign(8, 77);
    return_script(&mut b);
    let mut source = MemorySource::default();
    source.add(b"a.ps3", &a, &[0], b"b.ps3\0", 0);
    source.add(b"b.ps3", &b, &[0], &[], 0);
    run(&mut vm, &mut ScriptControl::new(0), &mut source);
    assert_eq!(vm.current_slot(), 0);
    assert_eq!(vm.state().integer(8), Some(77));
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.loaded_names().len(), 3);
}

#[test]
fn failed_load_and_invalid_slot_preserve_the_pending_request() {
    for slot in [0, 3] {
        let mut root = vec![];
        common_call(&mut root, 0, slot);
        word(&mut root, 0x2000);
        let root = script(&root, &[0], b"missing.ps3\0", 0);
        let mut vm = Machine::new(&root).unwrap();
        let request = pending(&mut vm);
        let pc = vm.pc();
        let mut source = MemorySource::default();
        assert!(
            ScriptControl::new(0)
                .try_handle(&mut vm, &mut source)
                .is_err()
        );
        assert_eq!(vm.pc(), pc);
        assert_eq!(vm.stack().len_bytes(), 8);
        assert_eq!(vm.step().unwrap(), request);
        assert_eq!(source.calls, usize::from(slot == 0));
        assert!(vm.loaded_script(1).is_none());
    }
}

#[test]
fn malformed_loaded_bytecode_is_not_installed() {
    let mut root = vec![];
    common_call(&mut root, 0, 0);
    word(&mut root, 0x2000);
    let root = script(&root, &[0], b"bad.ps3\0", 0);
    let mut vm = Machine::new(&root).unwrap();
    pending(&mut vm);
    let mut source = MemorySource::default();
    source.add(b"bad.ps3", &[0xff, 0xff], &[0], &[], 0);
    assert!(
        ScriptControl::new(0)
            .try_handle(&mut vm, &mut source)
            .is_err()
    );
    assert_eq!(vm.current_slot(), 0);
    assert!(vm.loaded_script(1).is_none());
    assert_eq!(vm.stack().len_bytes(), 8);
}

#[test]
fn script_return_requires_a_real_cross_script_context_and_consumes_reserved_word() {
    let bytes = [0x14, 0x04, 0x34, 0x12];
    let insn = bytecode::decode_at(&bytes, 0).unwrap();
    assert_eq!(insn.end, 4);
    assert_eq!(
        insn.kind,
        InstructionKind::ScriptReturn { reserved: 0x1234 }
    );
    let script = script(&bytes, &[0], &[], 0);
    let mut vm = Machine::new(&script).unwrap();
    assert!(vm.step().is_err());
    assert_eq!(vm.pc(), 0);
}

fn callback_initializer(slot: u32) -> (Vec<u8>, u32) {
    let mut code = vec![];
    for value in [11, 22, 33] {
        push(&mut code, int(value));
    }
    push(&mut code, expr(0x200, vec![scalar(0x10e, 1)]));
    push(&mut code, int(slot));
    word(&mut code, 0x2088);
    return_script(&mut code);
    let target = code.len() as u32;
    code.extend(assign(42, 77));
    return_script(&mut code);
    (code, target)
}

#[test]
fn label_callback_registration_and_explicit_cross_slot_call_preserve_parameters() {
    let mut root = vec![];
    common_call(&mut root, 0, 0);
    push(&mut root, int(69));
    word(&mut root, 0x208a);
    word(&mut root, 0x2000);
    let root = script(&root, &[0], b"a.ps3\0", 0);
    let mut vm = Machine::new(&root).unwrap();
    let (code, target) = callback_initializer(5);
    let mut source = MemorySource::default();
    source.add(b"a.ps3", &code, &[0, target], &[], 0);
    let mut host = ScriptControl::new(123);
    run(&mut vm, &mut host, &mut source);
    let callback = &host.callbacks()[&5];
    assert_eq!(callback.script_slot, 1);
    assert_eq!(callback.target, target);
    assert_eq!(callback.parameters, [11, 22, 33]);
    assert_eq!(callback.registered_tick, 123);
    assert_eq!(vm.current_slot(), 0);
    assert_eq!(vm.state().integer(42), Some(77));
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn main_transition_preserves_globals_and_common_scripts_but_requests_snapshot() {
    let mut root = vec![];
    for value in [0, 0, 0, 0, 4] {
        push(&mut root, int(value));
    }
    word(&mut root, 0x2088);
    root.extend(assign(7, 42));
    common_call(&mut root, 0, 0);
    push(&mut root, text(6));
    word(&mut root, 0x2080);
    let root = script(&root, &[0], b"a.ps3\0main.ps3\0", 0);
    let mut vm = Machine::new(&root).unwrap();
    let (code, target) = callback_initializer(5);
    let mut source = MemorySource::default();
    source.add(b"a.ps3", &code, &[0, target], &[], 0);
    let mut main = vec![];
    word(&mut main, 0x2000);
    main.extend(int(8));
    source.add(b"main.ps3", &main, &[0], &[], 2);
    let mut host = ScriptControl::new(0);
    assert_eq!(
        run(&mut vm, &mut host, &mut source),
        Some(ControlEvent::MainPreparedNeedsSnapshot)
    );
    assert_eq!(vm.current_slot(), 0);
    assert_eq!(vm.current_name(), b"main.ps3");
    assert_eq!(vm.pc(), 2);
    assert_eq!(vm.state().integer(7), Some(42));
    assert!(vm.loaded_script(1).is_some());
    assert_eq!(vm.stack().len_bytes(), 0);
    assert!(host.callbacks()[&5].enabled);
    assert!(!host.callbacks()[&4].enabled);
    assert_eq!(host.callbacks()[&4].target, u32::MAX);
}

#[test]
fn repeat_counter_survives_common_initialization_return() {
    let mut root = vec![];
    word(&mut root, 0x2100);
    common_call(&mut root, 0, 0);
    word(&mut root, 0x2000);
    let root = script(&root, &[0], b"a.ps3\0", 0);
    let mut vm = Machine::new(&root).unwrap();
    pending(&mut vm);
    vm.complete_system(SystemResult(0x6000)).unwrap();
    assert_eq!(vm.repeat_count(), 1);
    let mut common = vec![];
    return_script(&mut common);
    let mut source = MemorySource::default();
    source.add(b"a.ps3", &common, &[0], &[], 0);
    pending(&mut vm);
    ScriptControl::new(0)
        .try_handle(&mut vm, &mut source)
        .unwrap();
    assert_eq!(vm.repeat_count(), 0);
    vm.step().unwrap();
    assert_eq!(vm.repeat_count(), 1);
}

#[test]
fn callback_disable_blocks_explicit_dispatch_without_consuming_request() {
    let mut root = vec![];
    common_call(&mut root, 0, 0);
    push(&mut root, int(5));
    word(&mut root, 0x208b);
    push(&mut root, int(5));
    word(&mut root, 0x208a);
    word(&mut root, 0x2000);
    let root = script(&root, &[0], b"a.ps3\0", 0);
    let mut vm = Machine::new(&root).unwrap();
    let (code, target) = callback_initializer(5);
    let mut source = MemorySource::default();
    source.add(b"a.ps3", &code, &[0, target], &[], 0);
    let mut host = ScriptControl::new(0);
    loop {
        let request = pending(&mut vm);
        if let Step::System { command: 0x8a, .. } = request {
            assert!(host.try_handle(&mut vm, &mut source).is_err());
            assert_eq!(vm.step().unwrap(), request);
            break;
        }
        host.try_handle(&mut vm, &mut source).unwrap();
    }
    assert!(!host.callbacks()[&5].enabled);
    assert_eq!(host.callbacks()[&5].target, u32::MAX);
}

#[test]
fn label_operand_validates_index_and_changes_only_on_success() {
    let code = expr(0x200, vec![scalar(0x10e, 1)]);
    let e = match bytecode::decode_at(&code, 0).unwrap().kind {
        InstructionKind::Expression(e) => e,
        _ => unreachable!(),
    };
    let mut state = cmvs_rs::evaluate::State::default();
    assert!(state.evaluate_integer_with_labels(&e, &[0]).is_err());
    assert_eq!(state.result, None);
    assert_eq!(
        state.evaluate_integer_with_labels(&e, &[0, 1234]).unwrap(),
        1234
    );
}

#[test]
fn common_script_cannot_replace_its_currently_executing_slot() {
    let mut code = vec![];
    common_call(&mut code, 0, 0);
    word(&mut code, 0x2000);
    let root = script(&code, &[0], b"a.ps3\0", 0);
    let mut vm = Machine::new(&root).unwrap();
    let mut source = MemorySource::default();
    source.add(b"a.ps3", &code, &[0], b"a.ps3\0", 0);
    let mut host = ScriptControl::new(0);
    pending(&mut vm);
    host.try_handle(&mut vm, &mut source).unwrap();
    pending(&mut vm);
    let pc = vm.pc();
    let len = vm.stack().len_bytes();
    assert!(host.try_handle(&mut vm, &mut source).is_err());
    assert_eq!(vm.current_slot(), 1);
    assert_eq!(vm.pc(), pc);
    assert_eq!(vm.stack().len_bytes(), len);
    assert_eq!(vm.current_name(), b"a.ps3");
}

#[test]
fn script_version_and_callback_flags_use_verified_argument_order() {
    let mut code = vec![];
    push(&mut code, int(0x1234));
    push(&mut code, int(2));
    word(&mut code, 0x2083);
    push(&mut code, int(3));
    word(&mut code, 0x2089);
    push(&mut code, int(0));
    word(&mut code, 0x208c);
    word(&mut code, 0x2000);
    let root = script(&code, &[0], &[], 0);
    let mut vm = Machine::new(&root).unwrap();
    let mut host = ScriptControl::new(0);
    run(&mut vm, &mut host, &mut MemorySource::default());
    assert_eq!(host.settings()[&0x2083], 0x21234);
    assert_eq!(host.settings()[&0x2089], 1);
    assert_eq!(host.settings()[&0x208c], 0);
    assert_eq!(vm.stack().len_bytes(), 0);
}
