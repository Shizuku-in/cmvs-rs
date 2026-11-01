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
    accepts: fn(u16) -> bool,
    trace_accepts: fn(u16) -> bool,
}

impl Claim {
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
    claim!("fade::try_wait", Some(40), Some(20), 0x224),
    claim!("scene::try_wait_mask", Some(50), Some(30), 0x22b | 0x235),
    claim!("scene::try_wait_quake", Some(60), Some(40), 0x228),
    claim!("scene::try_wait_change", Some(70), None, 0x2c7),
    claim!("scene::try_wait_strip", Some(80), Some(50), 0x233),
    claim!("camera::try_wait_move", Some(90), None, 0x69),
    claim!(
        "message::try_wait_with_auto",
        Some(100),
        None,
        0x153 | 0x158
    ),
    Claim {
        handler: "dispatch::ROOT",
        runner_order: Some(110),
        // Trace exposes state hosts conditionally and has no text capability.
        trace_order: Some(0),
        accepts: |command| super::ROOT.iter().any(|entry| entry.command == command),
        trace_accepts: |command| {
            super::ROOT.iter().any(|entry| {
                entry.command == command && entry.capability != super::Capability::Text
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
    claim!("scene::try_handle", Some(270), Some(210),
        0x20..=0x23 | 0x26..=0x28 | 0x2a | 0x2d..=0x2f | 0x33 | 0x34 | 0x40
        | 0x42..=0x4c | 0x50 | 0x70..=0x75 | 0x78 | 0x1e0..=0x1fd
        | 0x220..=0x223 | 0x250 | 0x2c5 | 0x2c6 | 0x2cb | 0x321 | 0x322 | 0x329),
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
    claim!(
        "scene::try_handle_image",
        Some(300),
        Some(280),
        0x30 | 0x35 | 0x38 | 0x3b | 0x3f
    ),
    claim!("scene::try_change_image", Some(310), None, 0x2c4),
    claim!(
        "scene::try_handle_mask",
        Some(320),
        Some(290),
        0x22a | 0x22d | 0x234 | 0x237
    ),
    claim!(
        "snapshot::SaveTitle::try_handle",
        Some(330),
        Some(70),
        0x162
    ),
    claim!(
        "snapshot::SaveInfo::try_handle",
        Some(340),
        None,
        0x2b9 | 0x2ba
    ),
    claim!("gesture::try_handle_drag_press", Some(350), None, 0x369),
    claim!("gesture::try_handle", Some(360), Some(200),
        0x350..=0x353 | 0x356..=0x35b | 0x360 | 0x362 | 0x364..=0x368),
    claim!(
        "save_menu::try_handle_popup",
        Some(370),
        Some(110),
        0x2cd | 0x2ce | 0x2d3 | 0x2d4 | 0x2ee
    ),
    claim!("settings::try_handle", Some(390), Some(180),
        0x2d5 | 0x2de..=0x2e4 | 0x2e6..=0x2e9 | 0x2f8),
    claim!("message::try_record_poll", Some(400), None, 0x159),
    claim!("message::try_handle_scene", Some(410), None, 0x383 | 0x39d),
    claim!("message::try_handle", Some(420), None,
        0xec | 0x100..=0x107 | 0x10b..=0x110 | 0x112..=0x117 | 0x119 | 0x11a | 0x11e | 0x11f
        | 0x140..=0x151 | 0x154 | 0x155 | 0x15a | 0x15b | 0x15d
        | 0x171 | 0x178..=0x17c | 0x17f | 0x180 | 0x182..=0x186 | 0x18d
        | 0x190..=0x192 | 0x195..=0x197 | 0x1cf..=0x1d1
        | 0x380 | 0x382 | 0x383 | 0x385..=0x387 | 0x389 | 0x38b | 0x38c | 0x38e..=0x390
        | 0x392 | 0x393 | 0x396 | 0x398 | 0x39a..=0x39c | 0x39e..=0x3a0),
    claim!("history::try_handle", Some(430), Some(120), 0x12f..=0x136),
    claim!("message::try_handle_pointer", Some(440), None, 0x15f),
    claim!("message::try_handle_buttons", Some(450), None, 0x194),
    claim!("message::try_handle_image", Some(460), None, 0x170 | 0x397),
    claim!("buttons::try_handle", Some(470), Some(170),
        0x87 | 0x210..=0x215 | 0x21b | 0x370..=0x376 | 0x378 | 0x379 | 0x37c..=0x37f),
    claim!("buttons::try_handle_list_input", Some(480), None, 0x377),
    claim!(
        "buttons::try_handle_input",
        Some(490),
        Some(160),
        0x216 | 0x217 | 0x21a
    ),
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
    claim!("input::try_handle", Some(540), Some(260),
        0xc8..=0xca | 0x19a | 0x19c..=0x1a4 | 0x1a6 | 0x1a8..=0x1bd
        | 0x344..=0x347 | 0x34a | 0x34b | 0x34d..=0x34f | 0x35c | 0x35d),
    claim!("fade::try_handle_effects", Some(550), None, 0x2d1),
    claim!("input::try_handle_skipping", Some(560), Some(270), 0x2ef),
    claim!(
        "sequence::try_handle_with_hosts",
        Some(570),
        Some(140),
        0x280..=0x28a
    ),
    claim!("camera::try_handle", Some(580), Some(130), 0x56 | 0x58..=0x68 | 0x6a..=0x6d | 0x9c | 0x126 | 0x260..=0x264),
    claim!("capture::try_handle", Some(590), None, 0x3c | 0x304..=0x306),
    claim!("math::try_handle", Some(610), None, 0xd0..=0xd3),
    claim!(
        "text::try_handle",
        Some(620),
        Some(80),
        0xf1 | 0xf3 | 0xf8 | 0xfc
    ),
    claim!("effect_objects::try_handle", Some(630), Some(90), 0x2a0),
    claim!("audio::try_handle", Some(640), Some(100),
        0xa0..=0xa2 | 0xa4 | 0xa6 | 0xa8..=0xaa | 0xaf..=0xb1 | 0xb3 | 0xb5..=0xb7 | 0xba
        | 0x238..=0x23f | 0x241 | 0x2d6..=0x2dd),
];

/// Potential candidates, in the actual order for the requested chain. This does
/// not resolve state-dependent ownership or run a handler/evidence gate.
pub fn candidates(command: u16, chain: Chain) -> Vec<&'static Claim> {
    let mut claims: Vec<_> = CLAIMS
        .iter()
        .filter(|claim| claim.order(chain).is_some() && claim.recognizes_in(command, chain))
        .collect();
    claims.sort_by_key(|claim| claim.order(chain));
    claims
}
