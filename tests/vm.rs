use cmvs_rs::vm::{ParameterStack, StringRef, SystemResult};

#[test]
fn tagged_string_offsets_are_not_host_pointers() {
    assert_eq!(StringRef::decode(351), StringRef::Text(351));
    assert_eq!(StringRef::decode(0x4000_0020), StringRef::ScriptData(32));
    assert_eq!(StringRef::decode(0x8000_0007), StringRef::StringBank(7));
    assert_eq!(StringRef::decode(0xc000_0018), StringRef::Frame(24));
}

#[test]
fn path_command_reads_slot_from_top_and_string_from_preceding_word() {
    let mut stack = ParameterStack::new(32);
    stack.push_u32(31).unwrap();
    stack.push_u32(2).unwrap();
    assert_eq!(stack.peek_u32(4).unwrap(), 2);
    assert_eq!(stack.peek_u32(8).unwrap(), 31);
    stack.apply_cleanup(SystemResult(0x4008)).unwrap();
    assert_eq!(stack.len_bytes(), 0);
}

#[test]
fn wait_yields_and_advances_but_script_switch_does_neither() {
    let wait = SystemResult(0xc000);
    assert!(wait.yields());
    assert!(wait.advances_pc());
    assert_eq!(wait.cleanup_bytes(), 0);
    let switch = SystemResult(0);
    assert!(!switch.yields());
    assert!(!switch.advances_pc());
    let mut stack = ParameterStack::new(4);
    stack.push_u32(123).unwrap();
    stack.apply_cleanup(switch).unwrap();
    assert_eq!(stack.peek_u32(4).unwrap(), 123);
    assert_eq!(SystemResult(0x2008).cleanup_bytes(), 0);
    assert!(SystemResult(0x2008).increments_repeat_counter());
}

#[test]
fn failed_operations_preserve_stack_and_byte_cleanup_can_be_unaligned() {
    let mut stack = ParameterStack::new(4);
    stack.push_u32(0x12345678).unwrap();
    assert!(stack.push_u32(0).is_err());
    assert!(stack.discard(5).is_err());
    assert_eq!(stack.peek_u32(4).unwrap(), 0x12345678);
    assert!(stack.peek_u32(3).is_err());
    assert!(stack.peek_u32(5).is_err());
    stack.discard(1).unwrap();
    assert_eq!(stack.len_bytes(), 3);
    assert!(stack.peek_u32(4).is_err());
}
