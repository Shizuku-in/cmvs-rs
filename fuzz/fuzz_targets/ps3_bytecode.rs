#![no_main]

use cmvs_core::{
    MAX_PAYLOAD,
    format::ps3,
    vm::{bytecode, machine::Machine},
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ps3::Header::parse(data);
    if let Ok(script) = ps3::Script::decode(data) {
        assert!(script.payload().len() <= MAX_PAYLOAD);
        let _ = script.strings();
        let labels = script.labels();
        let _ = bytecode::decode_layout(script.code(), script.header().entry_pc, &labels);
        if let Ok(mut vm) = Machine::new(script) {
            for _ in 0..16 {
                if vm.step().is_err() || vm.pending_system().is_some() {
                    break;
                }
            }
        }
    }
    if let Ok(script) = ps3::Script::from_memory_image(data) {
        assert!(script.payload().len() <= MAX_PAYLOAD);
        let _ = script.strings();
        let labels = script.labels();
        let _ = bytecode::decode_layout(script.code(), script.header().entry_pc, &labels);
    }
    let _ = bytecode::decode_at(data, 0);
    let _ = bytecode::decode_layout(data, 0, &[0]);
});
