//! Incremental command-table migration. The text slice has root entries and sparse
//! research overrides; other commands still use the legacy chain. Gate checks run
//! before this table. A selected override never falls through to root semantics.
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    input::ActionInput,
    machine::Machine,
    message::{MessageHost, font_cells},
    message_flags::MessageFlags,
    profile::{EngineProfile, Semantics},
    vm::SystemResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Done,
    Yield,
    Unhandled,
}

pub struct TextHosts<'a> {
    pub messages: &'a mut MessageHost,
    pub input: &'a mut ActionInput,
    pub flags: MessageFlags,
}

pub struct Call<'a, 'script> {
    pub vm: &'a mut Machine<'script>,
    pub pc: usize,
    pub command: u16,
    arguments: u64,
}
impl Call<'_, '_> {
    pub fn argument(&mut self, distance: usize, name: &'static str) -> Result<u32, DecodeError> {
        if distance == 0 || !distance.is_multiple_of(4) || distance > 256 {
            return Err(self.error("invalid named argument distance"));
        }
        let value = self
            .vm
            .stack()
            .peek_u32(distance)
            .map_err(|_| error(self.pc, self.command + 0x2000, name))?;
        self.arguments |= 1 << (distance / 4 - 1);
        Ok(value)
    }
    pub fn error(&self, why: &'static str) -> DecodeError {
        error(self.pc, self.command + 0x2000, why)
    }
    pub fn complete(&mut self, flags: u16) -> Result<(), DecodeError> {
        let words = u32::from(flags & 0x0fff) / 4;
        let expected = 1u64.checked_shl(words).unwrap_or(0).wrapping_sub(1);
        if self.arguments != expected {
            return Err(self.error("named arguments disagree with command stack cleanup"));
        }
        self.vm.complete_system(SystemResult(flags)).map(|_| ())
    }
}

type Handler = fn(&mut TextHosts<'_>, &mut Call<'_, '_>) -> Result<Flow, DecodeError>;
pub struct Entry {
    pub command: u16,
    pub name: &'static str,
    pub handler: Handler,
}

fn root_text(hosts: &mut TextHosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, DecodeError> {
    Ok(if hosts.messages.try_handle(call.vm)? {
        Flow::Done
    } else {
        Flow::Unhandled
    })
}
fn root_output(hosts: &mut TextHosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, DecodeError> {
    Ok(
        if hosts
            .messages
            .try_record_output(call.vm, hosts.input, hosts.flags)?
            .is_some()
        {
            Flow::Yield
        } else {
            Flow::Unhandled
        },
    )
}
fn root_poll(hosts: &mut TextHosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, DecodeError> {
    Ok(if hosts.messages.try_record_poll(call.vm, hosts.flags)? {
        Flow::Done
    } else {
        Flow::Unhandled
    })
}

pub static ROOT: &[Entry] = &[
    Entry {
        command: 0x10b,
        name: "direct text",
        handler: root_text,
    },
    Entry {
        command: 0x152,
        name: "append window",
        handler: root_output,
    },
    Entry {
        command: 0x154,
        name: "direct window",
        handler: root_text,
    },
    Entry {
        command: 0x157,
        name: "append window",
        handler: root_output,
    },
    Entry {
        command: 0x159,
        name: "poll window",
        handler: root_poll,
    },
    Entry {
        command: 0x38b,
        name: "auxiliary text",
        handler: root_text,
    },
];

/// Research-only overrides: all selected commands remain Unreviewed in the
/// bound profile. This is not a registered Reviewed/ContractEquivalent variant.
pub static FONT_CELLS: &[Entry] = &[
    Entry {
        command: 0x10b,
        name: "font-cell direct text",
        handler: font_cells::direct_text,
    },
    Entry {
        command: 0x11d,
        name: "pixel text measurement",
        handler: font_cells::measure_text,
    },
    Entry {
        command: 0x152,
        name: "font-cell append window",
        handler: font_cells::append_window,
    },
    Entry {
        command: 0x154,
        name: "font-cell direct window",
        handler: font_cells::direct_window,
    },
    Entry {
        command: 0x157,
        name: "font-cell append window",
        handler: font_cells::append_window,
    },
    Entry {
        command: 0x159,
        name: "font-cell poll window",
        handler: font_cells::poll_window,
    },
    Entry {
        command: 0x38b,
        name: "font-cell auxiliary text",
        handler: font_cells::auxiliary_text,
    },
    Entry {
        command: 0x3ac,
        name: "font-cell mode",
        handler: font_cells::set_mode,
    },
];

pub fn semantics_name(profile: &EngineProfile, command: u16) -> &'static str {
    if profile.semantics(command) == Semantics::ResearchFontCells {
        "research text override"
    } else {
        "base semantics"
    }
}

/// Called after CommandGate::check. None delegates an unmigrated command to the
/// legacy chain. Missing host capabilities cannot turn an override into root code.
pub fn run_text(
    semantics: Semantics,
    vm: &mut Machine<'_>,
    hosts: Option<&mut TextHosts<'_>>,
) -> Result<Option<Flow>, DecodeError> {
    let Some((pc, command)) = vm.pending_system() else {
        return Ok(None);
    };
    let specific = if semantics == Semantics::ResearchFontCells {
        FONT_CELLS
    } else {
        &[]
    }
    .iter()
    .find(|e| e.command == command);
    let entry = if semantics == Semantics::ResearchFontCells {
        specific
    } else {
        ROOT.iter().find(|e| e.command == command)
    };
    let Some(entry) = entry else {
        return Ok((semantics != Semantics::Root).then_some(Flow::Unhandled));
    };
    let Some(hosts) = hosts else {
        return Ok(specific.map(|_| Flow::Unhandled));
    };
    (entry.handler)(
        hosts,
        &mut Call {
            vm,
            pc,
            command,
            arguments: 0,
        },
    )
    .map(Some)
}
