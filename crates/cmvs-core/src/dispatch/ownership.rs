//! Read-only shared ownership and runner/trace capability inventory.
//!
//! Commands use the VM's zero-based system-command number (opcode - 0x2000).
//! A claim means a handler can recognize a command, not that its arguments,
//! resources, evidence gate or host capabilities will let it complete. Delegated
//! helpers belong to their public chain entry. Orders describe the current code;
//! they are deliberately not used to dispatch commands.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chain {
    Runner,
    Trace,
}

pub struct Claim {
    pub handler: &'static str,
    pub runner_order: Option<u16>,
    pub trace_order: Option<u16>,
    runner_wait_order: Option<u16>,
    accepts: fn(u16) -> bool,
    trace_accepts: fn(u16) -> bool,
}

impl Claim {
    pub fn order_for(&self, command: u16, chain: Chain) -> Option<u16> {
        if chain == Chain::Runner
            && let Some(order) = self.runner_wait_order
            && super::is_wait(command)
        {
            return Some(order);
        }
        if chain == Chain::Runner
            && self.handler == "dispatch::ROOT"
            && super::uses_bootstrap(command)
        {
            return Some(35);
        }
        if chain == Chain::Runner && self.handler == "dispatch::ROOT" && command == 0x129 {
            return Some(30);
        }
        if chain == Chain::Runner && self.handler == "dispatch::ROOT" {
            if super::is_lifecycle(command) {
                return Some(10);
            }
            if super::is_message_wait(command) {
                return Some(100);
            }
        }
        self.order(chain)
    }

    /// Default call position; order_for includes command-specific early stages.
    pub fn order(&self, chain: Chain) -> Option<u16> {
        match chain {
            Chain::Runner => self.runner_order,
            Chain::Trace => self.trace_order,
        }
    }

    pub fn recognizes_in(&self, command: u16, chain: Chain) -> bool {
        self.recognizes(command) && (chain != Chain::Trace || (self.trace_accepts)(command))
    }

    pub fn recognizes(&self, command: u16) -> bool {
        command < 0x400 && (self.accepts)(command)
    }
}

/// Both environments route registered commands through ROOT. Capability filtering
/// records which commands each environment can expose, not evidence approval or
/// a promise that resource checks, runtime requests, or waits will complete.
pub static CLAIMS: &[Claim] = &[Claim {
    handler: "dispatch::ROOT",
    runner_order: Some(110),
    runner_wait_order: Some(40),
    // Trace exposes state hosts conditionally and has no text capability.
    trace_order: Some(0),
    accepts: |command| super::ROOT.iter().any(|entry| entry.command == command),
    trace_accepts: |command| {
        super::ROOT.iter().any(|entry| {
            entry.command == command
                && !matches!(
                    entry.capability,
                    super::Capability::Runtime
                        | super::Capability::MessageWait
                        | super::Capability::SystemWrite
                        | super::Capability::FontDialog
                        | super::Capability::Capture
                        | super::Capability::CaptureDevice
                        | super::Capability::MovieScene
                        | super::Capability::MovieStart
                        | super::Capability::Text
                        | super::Capability::Math
                        | super::Capability::SaveInfo
                        | super::Capability::Effects
                        | super::Capability::ChangeWait
                        | super::Capability::CameraWait
                        | super::Capability::ImageChange
                        | super::Capability::ListInput
                        | super::Capability::DragPress
                        | super::Capability::Windows
                        | super::Capability::WindowPointer
                        | super::Capability::WindowButtons
                        | super::Capability::WindowImages
                        | super::Capability::TextState
                        | super::Capability::AuxiliaryScene
                )
        })
    },
}];

/// Potential candidates, in the actual order for the requested chain. This does
/// not resolve state-dependent ownership or run a handler/evidence gate.
pub fn candidates(command: u16, chain: Chain) -> Vec<&'static Claim> {
    let mut claims: Vec<_> = CLAIMS
        .iter()
        .filter(|claim| {
            claim.order_for(command, chain).is_some() && claim.recognizes_in(command, chain)
        })
        .collect();
    claims.sort_by_key(|claim| claim.order_for(command, chain));
    claims
}
