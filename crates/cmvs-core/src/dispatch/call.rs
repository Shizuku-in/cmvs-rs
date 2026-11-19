//! Checked command arguments, completion, and deferred runtime requests.
use super::Flow;
use crate::{bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult};

/// Work requiring the runtime's complete host set. The VM remains pending until
/// the caller executes the request and acknowledges the original invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOperation {
    Quit,
    Reset,
    CaptureSnapshot,
    SaveSlot {
        slot: u32,
    },
    LoadSlot {
        slot: u32,
    },
    EditSlots {
        first: u32,
        second: u32,
    },
    SetSlotComment {
        slot: u32,
        text: u32,
    },
    SetSlotInfo {
        slot: u32,
        values: [u32; 2],
    },
    Thumbnail,
    LatestSlot {
        start: i32,
        count: i32,
    },
    SlotInfo {
        slot: i32,
        object: u32,
        child: i32,
        label_slot: u32,
        text_slot: u32,
    },
    CvnsSlotInfo {
        slot: i32,
        img_slot: u32,
        label_slot: u32,
        text_slot: u32,
    },
    CvnsCaptureScreen {
        slot: u32,
    },
    CvnsMaskCapture {
        mask_ref: u32,
        duration_ms: u32,
        ramp_ms: u32,
    },
    CvnsColorPicker {
        swatches: [u32; 2],
    },
    TextDialog {
        arguments: [u32; 5],
    },
    CvnsMessageDialog {
        arguments: Option<[u32; 3]>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeCall {
    pc: usize,
    command: u16,
    opcode: u16,
    arguments: u64,
    operation: RuntimeOperation,
    stack_len: usize,
    values: [u32; 6],
}

impl RuntimeCall {
    pub fn operation(&self) -> RuntimeOperation {
        self.operation
    }
    pub fn pc(&self) -> usize {
        self.pc
    }
    pub fn command(&self) -> u16 {
        self.command
    }
    /// Reject stale work before any runtime side effect or VM acknowledgement.
    pub fn validate(&self, vm: &Machine) -> Result<(), DecodeError> {
        if vm.pending_system() != Some((self.pc, self.command))
            || vm.stack().len_bytes() != self.stack_len
            || (0..self.arguments.count_ones() as usize)
                .any(|i| vm.stack().peek_u32(4 * (i + 1)).ok() != Some(self.values[i]))
        {
            return Err(error(
                self.pc,
                self.opcode,
                "runtime request no longer matches the pending command",
            ));
        }
        Ok(())
    }
    pub(crate) fn error(&self, why: &'static str) -> DecodeError {
        error(self.pc, self.opcode, why)
    }
    pub fn complete(&self, vm: &mut Machine, flags: u16) -> Result<Flow, DecodeError> {
        self.validate(vm)?;
        Call {
            vm,
            pc: self.pc,
            command: self.command,
            source_command: self.opcode.saturating_sub(0x2000),
            arguments: self.arguments,
        }
        .finish(flags)
    }
}

pub(crate) struct Call<'a> {
    pub vm: &'a mut Machine,
    pub pc: usize,
    pub command: u16,
    source_command: u16,
    arguments: u64,
}

impl<'a> Call<'a> {
    pub(crate) fn new(vm: &'a mut Machine, pc: usize, command: u16) -> Self {
        let source_command = vm.pending_source().unwrap_or(command);
        Self {
            vm,
            pc,
            command,
            source_command,
            arguments: 0,
        }
    }

    pub fn opcode(&self) -> u16 {
        0x2000 + self.source_command
    }

    pub fn argument(&mut self, distance: usize, name: &'static str) -> Result<u32, DecodeError> {
        if distance == 0 || !distance.is_multiple_of(4) || distance > 256 {
            return Err(self.error("invalid named argument distance"));
        }
        let value = self
            .vm
            .stack()
            .peek_u32(distance)
            .map_err(|_| self.error(name))?;
        self.arguments |= 1 << (distance / 4 - 1);
        Ok(value)
    }
    /// Account for an unused parameter without requiring its bytes to be known.
    /// The word must still exist and agree with the command's stack cleanup.
    pub fn ignore_argument(
        &mut self,
        distance: usize,
        name: &'static str,
    ) -> Result<(), DecodeError> {
        if distance == 0 || !distance.is_multiple_of(4) || distance > 256 {
            return Err(self.error("invalid named argument distance"));
        }
        if self.vm.stack().len_bytes() < distance {
            return Err(self.error(name));
        }
        self.arguments |= 1 << (distance / 4 - 1);
        Ok(())
    }
    pub fn error(&self, why: &'static str) -> DecodeError {
        error(self.pc, self.opcode(), why)
    }
    fn require_arguments(&self, bytes: usize) -> Result<(), DecodeError> {
        if bytes > 256 || !bytes.is_multiple_of(4) {
            return Err(self.error("invalid command argument byte count"));
        }
        let words = (bytes / 4) as u32;
        let expected = 1u64.checked_shl(words).unwrap_or(0).wrapping_sub(1);
        if self.arguments != expected {
            return Err(self.error("named arguments disagree with command stack cleanup"));
        }
        Ok(())
    }

    /// Transfers validate named parameters without completing the old instruction.
    /// Machine owns the return frame or main replacement and clears pending once.
    pub fn enter_common(
        &mut self,
        slot: u32,
        name: Vec<u8>,
        script: crate::ps3::Script,
    ) -> Result<(), DecodeError> {
        self.require_arguments(8)?;
        self.vm.enter_common(slot, name, script)
    }
    pub fn enter_cvns_common(
        &mut self,
        name: Vec<u8>,
        script: crate::ps3::Script,
    ) -> Result<(), DecodeError> {
        self.require_arguments(4)?;
        self.vm.enter_common_with(0, name, script, 4)
    }
    pub fn switch_main(
        &mut self,
        name: Vec<u8>,
        script: crate::ps3::Script,
    ) -> Result<(), DecodeError> {
        self.require_arguments(4)?;
        self.vm.switch_main_core(name, script)
    }
    pub fn call_script(&mut self, slot: u8, target: usize) -> Result<(), DecodeError> {
        self.require_arguments(4)?;
        self.vm.call_script(slot, target)
    }

    pub(super) fn defer(
        &self,
        operation: RuntimeOperation,
        bytes: usize,
    ) -> Result<Flow, DecodeError> {
        if bytes > 24 {
            return Err(self.error("runtime request exceeds its parameter capacity"));
        }
        self.require_arguments(bytes)?;
        let mut values = [0; 6];
        for (i, value) in values.iter_mut().take(bytes / 4).enumerate() {
            *value = self
                .vm
                .stack()
                .peek_u32(4 * (i + 1))
                .map_err(|_| self.error("runtime argument is unknown"))?;
        }
        Ok(Flow::Runtime(RuntimeCall {
            pc: self.pc,
            command: self.command,
            opcode: self.opcode(),
            arguments: self.arguments,
            operation,
            stack_len: self.vm.stack().len_bytes(),
            values,
        }))
    }
    pub fn complete(&mut self, flags: u16) -> Result<(), DecodeError> {
        self.finish(flags).map(|_| ())
    }

    pub fn finish(&mut self, flags: u16) -> Result<Flow, DecodeError> {
        self.require_arguments(SystemResult(flags).cleanup_bytes())?;
        let step = self.vm.complete_system(SystemResult(flags))?;
        Ok(Flow::from_step(step, false))
    }

    /// Validate the retained arguments before yielding without advancing PC or SP.
    pub fn wait(&mut self, bytes: usize) -> Result<Flow, DecodeError> {
        self.require_arguments(bytes)?;
        self.vm.yield_pending_system()?;
        Ok(Flow::Wait)
    }
}
