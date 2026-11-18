//! Command lookup, host provisioning queries, and variant metadata.
mod cvns;
mod root;
mod variants;

use super::Entry;
use crate::profile::{EngineProfile, Semantics, VariantId};

pub use cvns::CVNS;
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

fn default_entry(command: u16) -> Option<&'static Entry> {
    if command >= crate::cvns::CVNS_ONLY {
        cvns_entry(command)
    } else {
        root_entry(command)
    }
}

pub fn entry(semantics: Semantics, command: u16) -> Option<&'static Entry> {
    resolve_entry(semantics, command)
}

pub(crate) fn resolve_entry(semantics: Semantics, command: u16) -> Option<&'static Entry> {
    match semantics {
        Semantics::Root => default_entry(command),
        Semantics::Variant(id) => variant_entry(id, command),
    }
}

pub fn uses_message_state(command: u16) -> bool {
    command == 0x2de
        || entries_any_table(command)
            .iter()
            .copied()
            .any(Entry::uses_message_state)
}

pub fn uses_windows(command: u16) -> bool {
    entries_any_table(command)
        .iter()
        .copied()
        .any(Entry::uses_windows)
}

pub fn uses_window_images(command: u16) -> bool {
    entries_any_table(command)
        .iter()
        .copied()
        .any(Entry::uses_window_images)
}

pub fn uses_input(command: u16) -> bool {
    uses_input_for(Semantics::Root, command)
}

pub fn uses_input_for(semantics: Semantics, command: u16) -> bool {
    resolve_entry(semantics, command).is_some_and(Entry::uses_input)
}

pub fn is_message_wait(command: u16) -> bool {
    default_entry(command).is_some_and(Entry::is_message_wait)
}

pub fn is_lifecycle(command: u16) -> bool {
    default_entry(command).is_some_and(Entry::is_lifecycle)
}

/// Platform and capture backends must be borrowed independently of image sources.
pub fn uses_bootstrap(command: u16) -> bool {
    uses_bootstrap_for(Semantics::Root, command)
}

pub fn uses_bootstrap_for(semantics: Semantics, command: u16) -> bool {
    resolve_entry(semantics, command).is_some_and(Entry::uses_bootstrap)
}

pub fn uses_platform(command: u16) -> bool {
    uses_platform_for(Semantics::Root, command)
}

pub fn uses_platform_for(semantics: Semantics, command: u16) -> bool {
    resolve_entry(semantics, command).is_some_and(Entry::uses_platform)
}

pub fn uses_device_frame_for(semantics: Semantics, command: u16) -> bool {
    resolve_entry(semantics, command).is_some_and(Entry::uses_device_frame)
}

pub fn uses_device_frame(command: u16) -> bool {
    uses_device_frame_for(Semantics::Root, command)
}

pub fn uses_sequence(command: u16) -> bool {
    entries_any_table(command)
        .iter()
        .copied()
        .any(Entry::uses_sequence)
}

pub fn uses_movie_scene(command: u16) -> bool {
    uses_movie_scene_for(Semantics::Root, command)
}

pub fn uses_movie_scene_for(semantics: Semantics, command: u16) -> bool {
    resolve_entry(semantics, command).is_some_and(Entry::uses_movie_scene)
}

pub fn uses_movie_player(command: u16) -> bool {
    uses_movie_player_for(Semantics::Root, command)
}

pub fn uses_movie_player_for(semantics: Semantics, command: u16) -> bool {
    resolve_entry(semantics, command).is_some_and(Entry::uses_movie_player)
}

pub fn uses_images(command: u16) -> bool {
    entries_any_table(command)
        .iter()
        .copied()
        .any(Entry::uses_images)
}

pub fn is_wait(command: u16) -> bool {
    is_wait_for(Semantics::Root, command)
}

/// Waits of the selected semantics, including profile-bound variants.
pub fn is_wait_for(semantics: Semantics, command: u16) -> bool {
    resolve_entry(semantics, command).is_some_and(Entry::is_wait)
}

fn indexed_entry(
    table: &'static [Entry],
    cache: &std::sync::OnceLock<Vec<u16>>,
    command: u16,
) -> Option<&'static Entry> {
    let index = cache.get_or_init(|| {
        let size = table
            .iter()
            .map(|e| usize::from(e.command) + 1)
            .max()
            .unwrap_or(0);
        let mut index = vec![u16::MAX; size];
        for (at, entry) in table.iter().enumerate().rev() {
            index[usize::from(entry.command)] = at as u16;
        }
        index
    });
    match index.get(usize::from(command)) {
        Some(&at) if at != u16::MAX => Some(&table[usize::from(at)]),
        _ => None,
    }
}

/// The ROOT entry for `command`, through an index built once: system commands are
/// classified many times per VM step, and ROOT holds each command at most once
/// (`root_commands_are_unique`).
pub(crate) fn root_entry(command: u16) -> Option<&'static Entry> {
    static INDEX: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    indexed_entry(ROOT, &INDEX, command)
}

/// The CVNS-only entry for `command` (`>= CVNS_ONLY`), indexed once like [`root_entry`].
pub(crate) fn cvns_entry(command: u16) -> Option<&'static Entry> {
    static INDEX: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    indexed_entry(CVNS, &INDEX, command)
}

/// Every entry for `command` in ROOT, CVNS, and then [`VARIANT_TABLES`], in table order.
fn entries_any_table(command: u16) -> &'static [&'static Entry] {
    static INDEX: std::sync::OnceLock<Vec<Vec<&'static Entry>>> = std::sync::OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let all = || {
            ROOT.iter()
                .chain(CVNS.iter())
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

macro_rules! define_variant_registry {
    ($(($variant:ident, $table:ident, $label:literal)),* $(,)?) => {
        pub fn variant_entry(id: VariantId, command: u16) -> Option<&'static Entry> {
            let entries = match id {
                $(VariantId::$variant => $table,)*
            };
            entries.iter().find(|entry| entry.command == command)
        }

        pub fn semantics_name(profile: &EngineProfile, command: u16) -> &'static str {
            match profile.semantics(command) {
                $(Semantics::Variant(VariantId::$variant) => $label,)*
                Semantics::Root => "base semantics",
            }
        }

        pub static VARIANT_TABLES: &[&[Entry]] = &[$($table,)*];
    };
}

#[rustfmt::skip]
define_variant_registry!(
    (TextDialog, TEXT_DIALOG, "variant text dialog"),
    (SeGain, SE_GAIN, "variant SE gain multiplier"),
    (MixerGain, MIXER_GAIN, "variant device mixer gain"),
    (ImageTables, IMAGE_TABLES, "variant separate image tables"),
    (ActivationPass, ACTIVATION_PASS, "variant activation check reported as passed"),
    (WindowScaleMotion, WINDOW_SCALE_MOTION, "variant window scale motion"),
    (WindowPositionMotion, WINDOW_POSITION_MOTION, "variant window position motion"),
    (WindowOpacityTrack, WINDOW_OPACITY_TRACK, "variant window opacity track"),
    (LogicalText, LOGICAL_TEXT, "variant text with logical size"),
    (ViewMotion, VIEW_MOTION, "variant device view motion"),
    (WindowFullMove, WINDOW_FULL_MOVE, "variant unscaled fullscreen origin"),
    (LegacyEffects, LEGACY_EFFECTS, "variant legacy effect slots"),
    (WindowChildImage, WINDOW_CHILD_IMAGE, "variant window child image"),
    (WindowDrawScale, WINDOW_DRAW_SCALE, "variant window draw scale"),
    (SequenceWindowImage, SEQUENCE_WINDOW_IMAGE, "variant sequence window image"),
    (DrawQuery, DRAW_QUERY, "variant object draw query"),
    (CameraMoveX, CAMERA_MOVE_X, "variant camera X move"),
    (MovieControl, MOVIE_CONTROL, "variant scene movie control"),
    (MovieSeek, MOVIE_SEEK, "variant scene movie seek"),
    (ScreenCoordinates, SCREEN_COORDINATES, "variant screen coordinates"),
    (InputPress, INPUT_PRESS, "variant input down edge"),
    (MgvPlayer, MGV_PLAYER, "variant MGV player"),
    (DisplayCount, DISPLAY_COUNT, "variant display count"),
    (FullscreenMonitor, FULLSCREEN_MONITOR, "variant fullscreen monitor"),
    (KeyedArchives, KEYED_ARCHIVES, "variant keyed archives"),
    (ListSelect, LIST_SELECT, "variant list selection"),
    (ObjectQueue, OBJECT_QUEUE, "variant object queue"),
    (TextAlpha, TEXT_ALPHA, "variant text alpha revision"),
    (FontCells, FONT_CELLS, "variant text override"),
    (Particles, PARTICLES, "variant particle override"),
    (ParticlesX87, PARTICLES_X87, "variant x87 particle override"),
    (Rain, RAIN, "variant rain streaks"),
    (BackgroundAudio, BACKGROUND_AUDIO, "variant background audio override"),
    (SeRestart, SE_RESTART, "variant SE restart"),
    (AudioPreload, AUDIO_PRELOAD, "variant audio preload"),
    (PolarPoint, POLAR_POINT, "variant integer polar point"),
    (FallingBillboards, FALLING_BILLBOARDS, "variant falling billboard override"),
    (FallingBillboardsX87, FALLING_BILLBOARDS_X87, "variant x87 falling billboard override"),
    (CollisionParticles, COLLISION_PARTICLES, "variant collision particle override"),
    (FadingParticles, FADING_PARTICLES, "variant fading particle override"),
    (Fragments, FRAGMENTS, "variant image fragment override"),
);

#[cfg(test)]
mod index_tests {
    use super::*;

    #[test]
    fn indexes_match_linear_scans_for_every_command() {
        for command in 0..=u16::MAX {
            let linear_root = ROOT.iter().find(|e| e.command == command);
            assert_eq!(
                root_entry(command).map(|e| e as *const Entry),
                linear_root.map(|e| e as *const Entry),
                "{command:#x}"
            );
            let linear_cvns = CVNS.iter().find(|e| e.command == command);
            assert_eq!(
                cvns_entry(command).map(|e| e as *const Entry),
                linear_cvns.map(|e| e as *const Entry),
                "{command:#x}"
            );
            let all: Vec<_> = ROOT
                .iter()
                .chain(CVNS.iter())
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
