mod support;
use cmvs_rs::machine::Machine;
use support::bytecode::{command, pending, script};

#[test]
fn effect_object_release_pops_its_slot_for_any_value() {
    let mut code = vec![];
    for slot in [0, 3, 4, u32::MAX] {
        command(&mut code, 0x22a0, &[slot]);
    }
    code.extend(0x22a0u16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 7).unwrap();
    for _ in 0..4 {
        pending(&mut vm);
        assert!(cmvs_rs::effect_objects::try_handle(&mut vm).unwrap());
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.pending_system(), None);
    }
    assert_eq!(vm.state().system_integer(0), Some(7));
    // Without its argument the command stays pending.
    pending(&mut vm);
    assert!(cmvs_rs::effect_objects::try_handle(&mut vm).is_err());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x2a0));
}
