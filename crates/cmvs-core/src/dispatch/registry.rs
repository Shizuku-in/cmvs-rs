//! Command lookup, host provisioning queries, and variant metadata.
mod root;
mod variants;

use super::{Capability, Entry};
use crate::profile::{EngineProfile, Semantics};

pub use root::ROOT;
pub use variants::{
    ACTIVATION_PASS, AUDIO_PRELOAD, BACKGROUND_AUDIO, CAMERA_MOVE_X, COLLISION_PARTICLES,
    DISPLAY_COUNT, DRAW_QUERY, FADING_PARTICLES, FALLING_BILLBOARDS, FALLING_BILLBOARDS_X87,
    FONT_CELLS, FRAGMENTS, FULLSCREEN_MONITOR, IMAGE_TABLES, INPUT_PRESS, KEYED_ARCHIVES,
    LEGACY_EFFECTS, LIST_SELECT, LOGICAL_TEXT, MGV_PLAYER, MIXER_GAIN, MOVIE_CONTROL, MOVIE_SEEK,
    OBJECT_QUEUE, PARTICLES, PARTICLES_X87, POLAR_POINT, RAIN, SCREEN_COORDINATES, SE_GAIN,
    SE_RESTART, SEQUENCE_WINDOW_IMAGE, TEXT_ALPHA, TEXT_DIALOG, VIEW_MOTION, WINDOW_CHILD_IMAGE,
    WINDOW_DRAW_SCALE, WINDOW_FULL_MOVE, WINDOW_OPACITY_TRACK, WINDOW_POSITION_MOTION,
    WINDOW_SCALE_MOTION,
};

pub fn uses_message_state(command: u16) -> bool {
    // The font-name query needs the message host's backend to show a readable name.
    command == 0x2de
        || uses_windows(command)
        || entries_any_table(command).iter().copied().any(|e| {
            e.command == command
                && matches!(
                    e.capability,
                    Capability::TextState | Capability::AuxiliaryScene | Capability::FontDialog
                )
        })
}

pub fn uses_windows(command: u16) -> bool {
    entries_any_table(command).iter().copied().any(|entry| {
        entry.command == command
            && matches!(
                entry.capability,
                Capability::Windows
                    | Capability::WindowPointer
                    | Capability::WindowButtons
                    | Capability::WindowImages
            )
    })
}

pub fn uses_window_images(command: u16) -> bool {
    entries_any_table(command)
        .iter()
        .copied()
        .any(|entry| entry.command == command && entry.capability == Capability::WindowImages)
}

fn input_capability(capability: Capability) -> bool {
    matches!(
        capability,
        Capability::Input
            | Capability::ButtonInput
            | Capability::ListInput
            | Capability::DragPress
            | Capability::WindowPointer
            | Capability::WindowButtons
    )
}

pub fn uses_input(command: u16) -> bool {
    root_entry(command)
        .into_iter()
        .any(|entry| entry.command == command && input_capability(entry.capability))
}

pub fn uses_input_for(semantics: Semantics, command: u16) -> bool {
    match semantics {
        Semantics::Root => uses_input(command),
        Semantics::Variant(id) => {
            variant_entry(id, command).is_some_and(|entry| input_capability(entry.capability))
        }
    }
}

pub fn is_message_wait(command: u16) -> bool {
    root_entry(command)
        .into_iter()
        .any(|e| e.command == command && e.capability == Capability::MessageWait)
}

pub fn is_lifecycle(command: u16) -> bool {
    root_entry(command).into_iter().any(|e| {
        e.command == command && e.capability == Capability::Runtime && matches!(command, 1 | 3)
    })
}

/// Platform and capture backends must be borrowed independently of image sources.
pub fn uses_bootstrap(command: u16) -> bool {
    root_entry(command).into_iter().any(|e| {
        e.command == command
            && matches!(
                e.capability,
                Capability::Bootstrap | Capability::FunctionKeys
            )
    })
}

pub fn uses_bootstrap_for(semantics: Semantics, command: u16) -> bool {
    match semantics {
        Semantics::Root => uses_bootstrap(command),
        Semantics::Variant(id) => variant_entry(id, command)
            .is_some_and(|entry| entry.capability == Capability::Bootstrap),
    }
}

pub fn uses_platform(command: u16) -> bool {
    root_entry(command)
        .into_iter()
        .any(|e| e.command == command && e.capability == Capability::Platform)
}

pub fn uses_platform_for(semantics: Semantics, command: u16) -> bool {
    match semantics {
        Semantics::Root => uses_platform(command),
        Semantics::Variant(id) => {
            variant_entry(id, command).is_some_and(|entry| entry.capability == Capability::Platform)
        }
    }
}

pub fn uses_device_frame_for(semantics: Semantics, command: u16) -> bool {
    match semantics {
        Semantics::Root => uses_device_frame(command),
        Semantics::Variant(id) => variant_entry(id, command)
            .is_some_and(|entry| entry.capability == Capability::TextDialog),
    }
}

pub fn uses_device_frame(command: u16) -> bool {
    root_entry(command).into_iter().any(|e| {
        e.command == command
            && matches!(
                e.capability,
                Capability::CaptureDevice | Capability::FontDialog
            )
    })
}

pub fn uses_sequence(command: u16) -> bool {
    root_entry(command)
        .into_iter()
        .chain(SEQUENCE_WINDOW_IMAGE.iter())
        .any(|e| e.command == command && e.capability == Capability::Sequence)
}

pub fn uses_movie_scene(command: u16) -> bool {
    root_entry(command)
        .into_iter()
        .any(|e| e.command == command && e.capability == Capability::MovieScene)
}

pub fn uses_movie_scene_for(semantics: Semantics, command: u16) -> bool {
    match semantics {
        Semantics::Root => uses_movie_scene(command),
        Semantics::Variant(id) => variant_entry(id, command)
            .is_some_and(|entry| entry.capability == Capability::MovieScene),
    }
}

pub fn uses_movie_player(command: u16) -> bool {
    root_entry(command)
        .into_iter()
        .any(|e| e.command == command && e.capability == Capability::MoviePlayer)
}

pub fn uses_movie_player_for(semantics: Semantics, command: u16) -> bool {
    match semantics {
        Semantics::Root => uses_movie_player(command),
        Semantics::Variant(id) => {
            variant_entry(id, command).is_some_and(|e| e.capability == Capability::MoviePlayer)
        }
    }
}

pub fn uses_images(command: u16) -> bool {
    entries_any_table(command).iter().copied().any(|e| {
        e.command == command
            && matches!(
                e.capability,
                Capability::SceneImages
                    | Capability::MaskImages
                    | Capability::ImageChange
                    | Capability::ParticleImages
            )
    })
}

fn is_wait_capability(capability: Capability) -> bool {
    matches!(
        capability,
        Capability::FadeWait
            | Capability::MaskWait
            | Capability::QuakeWait
            | Capability::StripWait
            | Capability::ChangeWait
            | Capability::CameraWait
    )
}

pub fn is_wait(command: u16) -> bool {
    root_entry(command)
        .into_iter()
        .any(|e| e.command == command && is_wait_capability(e.capability))
}

/// Waits of the selected semantics, including profile-bound variants.
pub fn is_wait_for(semantics: Semantics, command: u16) -> bool {
    match semantics {
        Semantics::Root => is_wait(command),
        Semantics::Variant(id) => {
            variant_entry(id, command).is_some_and(|e| is_wait_capability(e.capability))
        }
    }
}

/// The ROOT entry for `command`, through an index built once: system commands are
/// classified many times per VM step, and ROOT holds each command at most once
/// (`root_commands_are_unique`).
pub(crate) fn root_entry(command: u16) -> Option<&'static Entry> {
    static INDEX: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let size = ROOT
            .iter()
            .map(|e| usize::from(e.command) + 1)
            .max()
            .unwrap_or(0);
        let mut index = vec![u16::MAX; size];
        for (at, entry) in ROOT.iter().enumerate().rev() {
            index[usize::from(entry.command)] = at as u16;
        }
        index
    });
    match index.get(usize::from(command)) {
        Some(&at) if at != u16::MAX => Some(&ROOT[usize::from(at)]),
        _ => None,
    }
}

/// Every entry for `command` in ROOT and then [`VARIANT_TABLES`], in table order.
fn entries_any_table(command: u16) -> &'static [&'static Entry] {
    static INDEX: std::sync::OnceLock<Vec<Vec<&'static Entry>>> = std::sync::OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let all = || {
            ROOT.iter()
                .chain(VARIANT_TABLES.iter().flat_map(|table| table.iter()))
        };
        let size = all().map(|e| usize::from(e.command) + 1).max().unwrap_or(0);
        let mut index = vec![Vec::new(); size];
        for entry in all() {
            index[usize::from(entry.command)].push(entry);
        }
        index
    });
    index.get(usize::from(command)).map_or(&[], Vec::as_slice)
}

#[rustfmt::skip]
pub fn variant_entry(id: crate::profile::VariantId, command: u16) -> Option<&'static Entry> {
    let entries = match id {
        crate::profile::VariantId::TextDialog => TEXT_DIALOG,
        crate::profile::VariantId::SeGain => SE_GAIN,
        crate::profile::VariantId::MixerGain => MIXER_GAIN,
        crate::profile::VariantId::ImageTables => IMAGE_TABLES,
        crate::profile::VariantId::ActivationPass => ACTIVATION_PASS,
        crate::profile::VariantId::WindowScaleMotion => WINDOW_SCALE_MOTION,
        crate::profile::VariantId::WindowPositionMotion => WINDOW_POSITION_MOTION,
        crate::profile::VariantId::WindowOpacityTrack => WINDOW_OPACITY_TRACK,
        crate::profile::VariantId::LogicalText => LOGICAL_TEXT,
        crate::profile::VariantId::ViewMotion => VIEW_MOTION,
        crate::profile::VariantId::WindowFullMove => WINDOW_FULL_MOVE,
        crate::profile::VariantId::LegacyEffects => LEGACY_EFFECTS,
        crate::profile::VariantId::WindowChildImage => WINDOW_CHILD_IMAGE,
        crate::profile::VariantId::WindowDrawScale => WINDOW_DRAW_SCALE,
        crate::profile::VariantId::SequenceWindowImage => SEQUENCE_WINDOW_IMAGE,
        crate::profile::VariantId::DrawQuery => DRAW_QUERY,
        crate::profile::VariantId::CameraMoveX => CAMERA_MOVE_X,
        crate::profile::VariantId::MovieControl => MOVIE_CONTROL,
        crate::profile::VariantId::MovieSeek => MOVIE_SEEK,
        crate::profile::VariantId::ScreenCoordinates => SCREEN_COORDINATES,
        crate::profile::VariantId::InputPress => INPUT_PRESS,
        crate::profile::VariantId::MgvPlayer => MGV_PLAYER,
        crate::profile::VariantId::DisplayCount => DISPLAY_COUNT,
        crate::profile::VariantId::FullscreenMonitor => FULLSCREEN_MONITOR,
        crate::profile::VariantId::KeyedArchives => KEYED_ARCHIVES,
        crate::profile::VariantId::ListSelect => LIST_SELECT,
        crate::profile::VariantId::ObjectQueue => OBJECT_QUEUE,
        crate::profile::VariantId::TextAlpha => TEXT_ALPHA,
        crate::profile::VariantId::FontCells => FONT_CELLS,
        crate::profile::VariantId::Particles => PARTICLES,
        crate::profile::VariantId::ParticlesX87 => PARTICLES_X87,
        crate::profile::VariantId::Rain => RAIN,
        crate::profile::VariantId::BackgroundAudio => BACKGROUND_AUDIO,
        crate::profile::VariantId::SeRestart => SE_RESTART,
        crate::profile::VariantId::AudioPreload => AUDIO_PRELOAD,
        crate::profile::VariantId::PolarPoint => POLAR_POINT,
        crate::profile::VariantId::FallingBillboards => FALLING_BILLBOARDS,
        crate::profile::VariantId::FallingBillboardsX87 => FALLING_BILLBOARDS_X87,
        crate::profile::VariantId::CollisionParticles => COLLISION_PARTICLES,
        crate::profile::VariantId::FadingParticles => FADING_PARTICLES,
        crate::profile::VariantId::Fragments => FRAGMENTS,
    };
    entries.iter().find(|entry| entry.command == command)
}

#[rustfmt::skip]
pub fn semantics_name(profile: &EngineProfile, command: u16) -> &'static str {
    match profile.semantics(command) {
        Semantics::Variant(crate::profile::VariantId::TextDialog) => "variant text dialog",
        Semantics::Variant(crate::profile::VariantId::SeGain) => "variant SE gain multiplier",
        Semantics::Variant(crate::profile::VariantId::MixerGain) => "variant device mixer gain",
        Semantics::Variant(crate::profile::VariantId::ImageTables) => "variant separate image tables",
        Semantics::Variant(crate::profile::VariantId::ActivationPass) => "variant activation check reported as passed",
        Semantics::Variant(crate::profile::VariantId::WindowScaleMotion) => "variant window scale motion",
        Semantics::Variant(crate::profile::VariantId::WindowPositionMotion) => "variant window position motion",
        Semantics::Variant(crate::profile::VariantId::WindowOpacityTrack) => "variant window opacity track",
        Semantics::Variant(crate::profile::VariantId::LogicalText) => "variant text with logical size",
        Semantics::Variant(crate::profile::VariantId::ViewMotion) => "variant device view motion",
        Semantics::Variant(crate::profile::VariantId::WindowFullMove) => "variant unscaled fullscreen origin",
        Semantics::Variant(crate::profile::VariantId::LegacyEffects) => "variant legacy effect slots",
        Semantics::Variant(crate::profile::VariantId::WindowChildImage) => "variant window child image",
        Semantics::Variant(crate::profile::VariantId::WindowDrawScale) => "variant window draw scale",
        Semantics::Variant(crate::profile::VariantId::SequenceWindowImage) => "variant sequence window image",
        Semantics::Variant(crate::profile::VariantId::DrawQuery) => "variant object draw query",
        Semantics::Variant(crate::profile::VariantId::CameraMoveX) => "variant camera X move",
        Semantics::Variant(crate::profile::VariantId::MovieControl) => "variant scene movie control",
        Semantics::Variant(crate::profile::VariantId::MovieSeek) => "variant scene movie seek",
        Semantics::Variant(crate::profile::VariantId::ScreenCoordinates) => "variant screen coordinates",
        Semantics::Variant(crate::profile::VariantId::InputPress) => "variant input down edge",
        Semantics::Variant(crate::profile::VariantId::MgvPlayer) => "variant MGV player",
        Semantics::Variant(crate::profile::VariantId::DisplayCount) => "variant display count",
        Semantics::Variant(crate::profile::VariantId::FullscreenMonitor) => "variant fullscreen monitor",
        Semantics::Variant(crate::profile::VariantId::KeyedArchives) => "variant keyed archives",
        Semantics::Variant(crate::profile::VariantId::ListSelect) => "variant list selection",
        Semantics::Variant(crate::profile::VariantId::ObjectQueue) => "variant object queue",
        Semantics::Variant(crate::profile::VariantId::TextAlpha) => "variant text alpha revision",
        Semantics::Variant(crate::profile::VariantId::FontCells) => "variant text override",
        Semantics::Variant(crate::profile::VariantId::Particles) => "variant particle override",
        Semantics::Variant(crate::profile::VariantId::Rain) => "variant rain streaks",
        Semantics::Variant(crate::profile::VariantId::ParticlesX87) => "variant x87 particle override",
        Semantics::Variant(crate::profile::VariantId::BackgroundAudio) => "variant background audio override",
        Semantics::Variant(crate::profile::VariantId::FallingBillboards) => "variant falling billboard override",
        Semantics::Variant(crate::profile::VariantId::FallingBillboardsX87) => "variant x87 falling billboard override",
        Semantics::Variant(crate::profile::VariantId::CollisionParticles) => "variant collision particle override",
        Semantics::Variant(crate::profile::VariantId::FadingParticles) => "variant fading particle override",
        Semantics::Variant(crate::profile::VariantId::Fragments) => "variant image fragment override",
        Semantics::Variant(crate::profile::VariantId::SeRestart) => "variant SE restart",
        Semantics::Variant(crate::profile::VariantId::AudioPreload) => "variant audio preload",
        Semantics::Variant(crate::profile::VariantId::PolarPoint) => "variant integer polar point",
        Semantics::Root => "base semantics",
    }
}

pub static VARIANT_TABLES: &[&[Entry]] = &[
    TEXT_DIALOG,
    SE_GAIN,
    MIXER_GAIN,
    IMAGE_TABLES,
    ACTIVATION_PASS,
    WINDOW_SCALE_MOTION,
    WINDOW_POSITION_MOTION,
    WINDOW_OPACITY_TRACK,
    LOGICAL_TEXT,
    VIEW_MOTION,
    WINDOW_FULL_MOVE,
    LEGACY_EFFECTS,
    WINDOW_CHILD_IMAGE,
    WINDOW_DRAW_SCALE,
    SEQUENCE_WINDOW_IMAGE,
    DRAW_QUERY,
    CAMERA_MOVE_X,
    MOVIE_CONTROL,
    MOVIE_SEEK,
    SCREEN_COORDINATES,
    INPUT_PRESS,
    MGV_PLAYER,
    DISPLAY_COUNT,
    FULLSCREEN_MONITOR,
    KEYED_ARCHIVES,
    FONT_CELLS,
    PARTICLES,
    PARTICLES_X87,
    RAIN,
    BACKGROUND_AUDIO,
    SE_RESTART,
    AUDIO_PRELOAD,
    POLAR_POINT,
    TEXT_ALPHA,
    OBJECT_QUEUE,
    LIST_SELECT,
    FALLING_BILLBOARDS,
    FALLING_BILLBOARDS_X87,
    COLLISION_PARTICLES,
    FADING_PARTICLES,
    FRAGMENTS,
];

#[cfg(test)]
mod index_tests {
    use super::*;

    #[test]
    fn indexes_match_linear_scans_for_every_command() {
        for command in 0..=u16::MAX {
            let linear = ROOT.iter().find(|e| e.command == command);
            assert_eq!(
                root_entry(command).map(|e| e as *const Entry),
                linear.map(|e| e as *const Entry),
                "{command:#x}"
            );
            let all: Vec<_> = ROOT
                .iter()
                .chain(VARIANT_TABLES.iter().flat_map(|table| table.iter()))
                .filter(|e| e.command == command)
                .map(|e| e as *const Entry)
                .collect();
            let indexed: Vec<_> = entries_any_table(command)
                .iter()
                .map(|&e| e as *const Entry)
                .collect();
            assert_eq!(indexed, all, "{command:#x}");
        }
    }
}
