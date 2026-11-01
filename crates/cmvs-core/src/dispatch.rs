//! Incremental command-table migration. Root entries share one dispatch path;
//! text research overrides are sparse. Other commands still use the legacy chain. Gate checks run
//! before this table. A selected override never falls through to root semantics.
pub mod ownership;

use crate::{
    bytecode::DecodeError,
    evaluate::error,
    input::ActionInput,
    machine::Machine,
    message::{MessageHost, font_cells},
    message_flags::MessageFlags,
    profile::{EngineProfile, Semantics},
    random::CrtRandom,
    timers::VmTimers,
    vm::SystemResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Done,
    Yield,
    Unhandled,
    Unavailable,
}

#[derive(Default)]
pub struct Hosts<'a> {
    pub text: Option<TextHosts<'a>>,
    pub timers: Option<&'a mut VmTimers>,
    pub flags: Option<&'a mut MessageFlags>,
    pub random: Option<&'a mut CrtRandom>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Text,
    Timers,
    Flags,
    Random,
}

impl Hosts<'_> {
    fn has(&self, capability: Capability) -> bool {
        match capability {
            Capability::Text => self.text.is_some(),
            Capability::Timers => self.timers.is_some(),
            Capability::Flags => self.flags.is_some(),
            Capability::Random => self.random.is_some(),
        }
    }
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
impl<'a, 'script> Call<'a, 'script> {
    pub(crate) fn new(vm: &'a mut Machine<'script>, pc: usize, command: u16) -> Self {
        Self {
            vm,
            pc,
            command,
            arguments: 0,
        }
    }

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

type Handler = fn(&mut Hosts<'_>, &mut Call<'_, '_>) -> Result<Flow, DecodeError>;
pub struct Entry {
    pub command: u16,
    pub name: &'static str,
    pub capability: Capability,
    pub handler: Handler,
}

fn root_text(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, DecodeError> {
    let hosts = hosts.text.as_mut().expect("text capability checked");
    Ok(if hosts.messages.try_handle(call.vm)? {
        Flow::Done
    } else {
        Flow::Unhandled
    })
}
fn root_output(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, DecodeError> {
    let hosts = hosts.text.as_mut().expect("text capability checked");
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
fn root_poll(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, DecodeError> {
    let hosts = hosts.text.as_mut().expect("text capability checked");
    Ok(if hosts.messages.try_record_poll(call.vm, hosts.flags)? {
        Flow::Done
    } else {
        Flow::Unhandled
    })
}

fn root_timers(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, DecodeError> {
    hosts
        .timers
        .as_deref_mut()
        .expect("timers capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_flags(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, DecodeError> {
    hosts
        .flags
        .as_deref_mut()
        .expect("flags capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

fn root_random(hosts: &mut Hosts<'_>, call: &mut Call<'_, '_>) -> Result<Flow, DecodeError> {
    hosts
        .random
        .as_deref_mut()
        .expect("random capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub static ROOT: &[Entry] = &[
    Entry {
        command: 0x8,
        name: "frame clock",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0x9,
        name: "frame delta",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0xb,
        name: "reset timer",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0xc,
        name: "query timer",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0xe,
        name: "mark stopwatch",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0xf,
        name: "query stopwatch",
        capability: Capability::Timers,
        handler: root_timers,
    },
    Entry {
        command: 0xd4,
        name: "random remainder",
        capability: Capability::Random,
        handler: root_random,
    },
    Entry {
        command: 0x2d0,
        name: "voice cut mode",
        capability: Capability::Flags,
        handler: root_flags,
    },
    Entry {
        command: 0x2e5,
        name: "automatic message advance",
        capability: Capability::Flags,
        handler: root_flags,
    },
    Entry {
        command: 0x2eb,
        name: "message skip",
        capability: Capability::Flags,
        handler: root_flags,
    },
    Entry {
        command: 0x340,
        name: "message bypass",
        capability: Capability::Flags,
        handler: root_flags,
    },
    Entry {
        command: 0x10b,
        name: "direct text",
        capability: Capability::Text,
        handler: root_text,
    },
    Entry {
        command: 0x152,
        name: "append window",
        capability: Capability::Text,
        handler: root_output,
    },
    Entry {
        command: 0x154,
        name: "direct window",
        capability: Capability::Text,
        handler: root_text,
    },
    Entry {
        command: 0x157,
        name: "append window",
        capability: Capability::Text,
        handler: root_output,
    },
    Entry {
        command: 0x159,
        name: "poll window",
        capability: Capability::Text,
        handler: root_poll,
    },
    Entry {
        command: 0x38b,
        name: "auxiliary text",
        capability: Capability::Text,
        handler: root_text,
    },
];

/// Research-only overrides: all selected commands remain Unreviewed in the
/// bound profile. This is not a registered Reviewed/ContractEquivalent variant.
pub static FONT_CELLS: &[Entry] = &[
    Entry {
        command: 0x10b,
        name: "font-cell direct text",
        capability: Capability::Text,
        handler: |hosts, call| {
            font_cells::direct_text(hosts.text.as_mut().expect("text capability checked"), call)
        },
    },
    Entry {
        command: 0x11d,
        name: "pixel text measurement",
        capability: Capability::Text,
        handler: |hosts, call| {
            font_cells::measure_text(hosts.text.as_mut().expect("text capability checked"), call)
        },
    },
    Entry {
        command: 0x152,
        name: "font-cell append window",
        capability: Capability::Text,
        handler: |hosts, call| {
            font_cells::append_window(hosts.text.as_mut().expect("text capability checked"), call)
        },
    },
    Entry {
        command: 0x154,
        name: "font-cell direct window",
        capability: Capability::Text,
        handler: |hosts, call| {
            font_cells::direct_window(hosts.text.as_mut().expect("text capability checked"), call)
        },
    },
    Entry {
        command: 0x157,
        name: "font-cell append window",
        capability: Capability::Text,
        handler: |hosts, call| {
            font_cells::append_window(hosts.text.as_mut().expect("text capability checked"), call)
        },
    },
    Entry {
        command: 0x159,
        name: "font-cell poll window",
        capability: Capability::Text,
        handler: |hosts, call| {
            font_cells::poll_window(hosts.text.as_mut().expect("text capability checked"), call)
        },
    },
    Entry {
        command: 0x38b,
        name: "font-cell auxiliary text",
        capability: Capability::Text,
        handler: |hosts, call| {
            font_cells::auxiliary_text(hosts.text.as_mut().expect("text capability checked"), call)
        },
    },
    Entry {
        command: 0x3ac,
        name: "font-cell mode",
        capability: Capability::Text,
        handler: |hosts, call| {
            font_cells::set_mode(hosts.text.as_mut().expect("text capability checked"), call)
        },
    },
];

pub fn semantics_name(profile: &EngineProfile, command: u16) -> &'static str {
    if profile.semantics(command) == Semantics::ResearchFontCells {
        "research text override"
    } else {
        "base semantics"
    }
}

/// Called after CommandGate::resolve. None delegates an unmigrated command to the
/// legacy chain. Unavailable preserves the pending command and arguments; the
/// caller may report the missing capability, but must not try another handler.
pub fn run(
    semantics: Semantics,
    vm: &mut Machine<'_>,
    hosts: &mut Hosts<'_>,
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
    if !hosts.has(entry.capability) {
        return Ok(Some(Flow::Unavailable));
    }
    (entry.handler)(hosts, &mut Call::new(vm, pc, command)).map(Some)
}
