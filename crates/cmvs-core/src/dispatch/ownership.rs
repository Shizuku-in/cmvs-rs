//! Read-only inventory of the two existing dispatch chains.
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

macro_rules! claim {
    ($handler:literal, $runner:expr, $trace:expr, $($pattern:pat_param)|+) => {
        Claim {
            handler: $handler,
            runner_order: $runner,
            trace_order: $trace,
            runner_wait_order: None,
            accepts: |command| matches!(command, $($pattern)|+),
            trace_accepts: |command| matches!(command, $($pattern)|+),
        }
    };
}

/// All public chain entry points, including direct handling in runner. Internal
/// delegates (scene fade/motion, button lists, auxiliary motion, audio branches)
/// are folded into the entry that calls them. The root table combines migrated
/// state handlers with text adapters over the existing MessageHost methods.
pub static CLAIMS: &[Claim] = &[
    claim!("runner::try_lifecycle", Some(10), None, 0x01 | 0x03),
    claim!("capture::try_immediate", Some(20), None, 0x3e),
    claim!("system_save::try_write", Some(30), None, 0x129),
    claim!(
        "message::try_wait_with_auto",
        Some(100),
        None,
        0x153 | 0x158
    ),
    Claim {
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
                        super::Capability::Text
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
                    )
            })
        },
    },
    claim!("system_save::try_handle", Some(120), Some(310), 0x128),
    claim!("runner::choose_font", Some(130), None, 0x13d),
    claim!("platform::try_handle", Some(140), Some(60),
        0x8f | 0x99 | 0x160 | 0x161 | 0x163 | 0x164 | 0x16b..=0x16f
        | 0x2b0 | 0x2c8..=0x2ca | 0x2fc | 0x2fd | 0x2ff),
    claim!(
        "script_control::try_handle",
        Some(150),
        Some(320),
        0x80 | 0x81 | 0x83 | 0x88..=0x8c
    ),
    claim!("runner::capture_snapshot", Some(160), None, 0x2b2),
    claim!("runner::try_load_slot", Some(170), None, 0x2b5),
    claim!("runner::try_edit_slots", Some(180), None, 0x28c | 0x28e),
    claim!("runner::try_save_slot", Some(190), None, 0x2b6),
    claim!("runner::try_thumbnail", Some(200), None, 0x2b8),
    claim!("runner::try_latest_slot", Some(210), None, 0x2b7),
    claim!("runner::try_slot_info", Some(220), None, 0x2bd),
    claim!(
        "save_menu::try_handle",
        Some(230),
        Some(330),
        0x2b3 | 0x2b4 | 0x2bb
    ),
    claim!(
        "snapshot::SaveImage::try_handle",
        Some(240),
        Some(10),
        0x2b2
    ),
    claim!("bootstrap::try_handle", Some(260), Some(340),
        0x10..=0x16 | 0x19 | 0x1a | 0x1d..=0x1f | 0x90..=0x94 | 0x98 | 0x34d),
    claim!(
        "scene::try_handle_quake",
        Some(280),
        Some(220),
        0x20c | 0x225..=0x227 | 0x229
    ),
    claim!(
        "scene::try_handle_strip",
        Some(290),
        Some(230),
        0x22f..=0x232
    ),
    claim!("message::try_record_poll", Some(400), None, 0x159),
    claim!("message::try_handle_scene", Some(410), None, 0x383 | 0x39d),
    claim!("message::try_handle", Some(420), None, 0x100..=0x107 | 0x10b..=0x110 | 0x112..=0x117 | 0x119..=0x11a | 0x11e..=0x11f | 0x154 | 0x380 | 0x382..=0x383 | 0x385..=0x387 | 0x389 | 0x38b..=0x38c | 0x38e..=0x390 | 0x392..=0x393 | 0x396 | 0x398 | 0x39a..=0x39c | 0x39e..=0x3a0),
    claim!("movie::try_create_scene", Some(500), None, 0x308),
    claim!("movie::try_start_scene", Some(510), None, 0x309),
    claim!(
        "movie::try_handle",
        Some(520),
        Some(150),
        0x30a..=0x30c | 0x31a
    ),
    claim!(
        "movie::try_handle_player",
        Some(530),
        Some(250),
        0xc0..=0xc2
    ),
    claim!("input::try_handle", Some(540), Some(260), 0x34d),
    claim!(
        "sequence::try_handle_with_hosts",
        Some(570),
        Some(140),
        0x280..=0x28a
    ),
    claim!("capture::try_handle", Some(590), None, 0x3c | 0x304..=0x306),
];

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
