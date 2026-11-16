//! Profile-bound command registrations; selected variants never fall back to base semantics.
use crate::dispatch::handlers::input::{select_list, variant_input_press};
use crate::dispatch::handlers::media::{
    background_audio, legacy_movie_control, legacy_movie_seek, mgv_player, mixer_gain,
    preload_audio, root_audio, se_gain,
};
use crate::dispatch::handlers::platform::{root_platform, variant_text_dialog, window_full_move};
use crate::dispatch::handlers::scene::{
    collision_mesh, create_collision_particles, create_falling_billboards,
    create_falling_billboards_x87, create_fragments, create_legacy_flakes, create_particles,
    create_particles_x87, create_rain, draw_query, legacy_circle, legacy_curtain,
    legacy_floating_planes, legacy_magic, legacy_query, legacy_set, legacy_snow_field,
    legacy_sprites, legacy_vortex, legacy_walk, query_fragments, root_camera, root_effect_objects,
    root_images, root_scene, root_sequence, root_wait_view, screen_coordinates,
};
use crate::dispatch::handlers::services::{
    activation_pass, polar_point, root_bootstrap, root_settings,
};
use crate::dispatch::handlers::storage::root_runtime;
use crate::dispatch::handlers::text::{
    auxiliary_measure, font_append_window, font_auxiliary_face, font_auxiliary_line_break,
    font_auxiliary_text, font_direct_text, font_direct_window, font_measure_text, font_poll_window,
    font_set_mode, root_text_state, root_window_images, root_windows,
};
use crate::dispatch::{Capability, Entry};

/// Build-specific variants, selected only through an explicit profile binding.
/// Implementation selection does not upgrade the command's evidence status.
pub static KEYED_ARCHIVES: &[Entry] = &[
    Entry {
        command: 0x268,
        name: "register keyed script archive",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x269,
        name: "register keyed image archive",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x26a,
        name: "register keyed sound archive",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x26b,
        name: "register keyed music archive",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x26c,
        name: "register keyed voice archive",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x26d,
        name: "register keyed movie archive",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x26e,
        name: "register keyed scene movie archive",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
];

pub static MGV_PLAYER: &[Entry] = &[
    Entry {
        command: 0xc4,
        name: "open MGV player",
        capability: Capability::MoviePlayer,
        handler: mgv_player,
    },
    Entry {
        command: 0xc5,
        name: "stop MGV player",
        capability: Capability::MoviePlayer,
        handler: mgv_player,
    },
    Entry {
        command: 0xc6,
        name: "MGV player frame",
        capability: Capability::MoviePlayer,
        handler: mgv_player,
    },
];

pub static DISPLAY_COUNT: &[Entry] = &[Entry {
    command: 0x3c7,
    name: "available display count",
    capability: Capability::Platform,
    handler: root_platform,
}];

pub static FULLSCREEN_MONITOR: &[Entry] = &[
    Entry {
        command: 0x3c5,
        name: "fullscreen monitor selection mode",
        capability: Capability::Platform,
        handler: root_platform,
    },
    Entry {
        command: 0x3c6,
        name: "select fullscreen monitor",
        capability: Capability::Platform,
        handler: root_platform,
    },
];

pub static FONT_CELLS: &[Entry] = &[
    Entry {
        command: 0x10b,
        name: "font-cell direct text",
        capability: Capability::Text,
        handler: font_direct_text,
    },
    Entry {
        command: 0x11d,
        name: "pixel text measurement",
        capability: Capability::Text,
        handler: font_measure_text,
    },
    Entry {
        command: 0x152,
        name: "font-cell append window",
        capability: Capability::Text,
        handler: font_append_window,
    },
    Entry {
        command: 0x154,
        name: "font-cell direct window",
        capability: Capability::Text,
        handler: font_direct_window,
    },
    Entry {
        command: 0x157,
        name: "font-cell append window",
        capability: Capability::Text,
        handler: font_append_window,
    },
    Entry {
        command: 0x159,
        name: "font-cell poll window",
        capability: Capability::Text,
        handler: font_poll_window,
    },
    Entry {
        command: 0x388,
        name: "auxiliary font face",
        capability: Capability::Text,
        handler: font_auxiliary_face,
    },
    Entry {
        command: 0x38b,
        name: "font-cell auxiliary text",
        capability: Capability::Text,
        handler: font_auxiliary_text,
    },
    Entry {
        command: 0x3ac,
        name: "font-cell mode",
        capability: Capability::Text,
        handler: font_set_mode,
    },
    Entry {
        command: 0x3b8,
        name: "auxiliary line-break rules",
        capability: Capability::Text,
        handler: font_auxiliary_line_break,
    },
    Entry {
        command: 0x3b9,
        name: "auxiliary byte measurement",
        capability: Capability::Text,
        handler: auxiliary_measure,
    },
    Entry {
        command: 0x3bb,
        name: "auxiliary pixel measurement",
        capability: Capability::Text,
        handler: auxiliary_measure,
    },
];

pub static RAIN: &[Entry] = &[Entry {
    command: 0x2a4,
    name: "create rain streak effect",
    capability: Capability::ParticleImages,
    handler: create_rain,
}];

pub static PARTICLES: &[Entry] = &[Entry {
    command: 0x2a5,
    name: "create particle effect",
    capability: Capability::ParticleImages,
    handler: create_particles,
}];

pub static PARTICLES_X87: &[Entry] = &[Entry {
    command: 0x2a5,
    name: "create x87 particle effect",
    capability: Capability::ParticleImages,
    handler: create_particles_x87,
}];

pub static MOVIE_CONTROL: &[Entry] = &[
    Entry {
        command: 0x31d,
        name: "scene movie stop frame",
        capability: Capability::MovieState,
        handler: legacy_movie_control,
    },
    Entry {
        command: 0x30d,
        name: "scene movie playback mode",
        capability: Capability::MovieState,
        handler: legacy_movie_control,
    },
    Entry {
        command: 0x30f,
        name: "scene movie segment and frame",
        capability: Capability::MovieState,
        handler: legacy_movie_control,
    },
];

pub static MOVIE_SEEK: &[Entry] = &[
    Entry {
        command: 0x31c,
        name: "apply queued movie segment",
        capability: Capability::MovieScene,
        handler: legacy_movie_seek,
    },
    Entry {
        command: 0x31b,
        name: "seek scene movie frame",
        capability: Capability::MovieScene,
        handler: legacy_movie_seek,
    },
];

pub static FALLING_BILLBOARDS: &[Entry] = &[Entry {
    command: 0x2a2,
    name: "create falling billboard effect",
    capability: Capability::ParticleImages,
    handler: create_falling_billboards,
}];

/// MEMORIA 0x22a2 / 0x4539a0: the same type-2 effect with x87 arithmetic.
pub static FALLING_BILLBOARDS_X87: &[Entry] = &[Entry {
    command: 0x2a2,
    name: "create x87 falling billboard effect",
    capability: Capability::ParticleImages,
    handler: create_falling_billboards_x87,
}];

pub static FRAGMENTS: &[Entry] = &[
    Entry {
        command: 0x2a8,
        name: "create image fragment effect",
        capability: Capability::SceneRandom,
        handler: create_fragments,
    },
    Entry {
        command: 0x2ae,
        name: "query image fragment completion",
        capability: Capability::Scene,
        handler: query_fragments,
    },
];

pub static COLLISION_PARTICLES: &[Entry] = &[
    Entry {
        command: 0x2a6,
        name: "create collision particle effect",
        capability: Capability::ParticleImages,
        handler: create_collision_particles,
    },
    Entry {
        command: 0x330,
        name: "create collision mesh",
        capability: Capability::Scene,
        handler: collision_mesh,
    },
    Entry {
        command: 0x331,
        name: "release collision mesh",
        capability: Capability::Scene,
        handler: collision_mesh,
    },
    Entry {
        command: 0x332,
        name: "append collision vertex",
        capability: Capability::Scene,
        handler: collision_mesh,
    },
];

/// MEMORIA 0x22a7 / 0x453d20: type 7 takes the 0x22a6 arguments and pool, without collision.
pub static FADING_PARTICLES: &[Entry] = &[Entry {
    command: 0x2a7,
    name: "create fading particle effect",
    capability: Capability::ParticleImages,
    handler: create_collision_particles,
}];

pub static LIST_SELECT: &[Entry] = &[Entry {
    command: 0x37a,
    name: "select list button",
    capability: Capability::ButtonInput,
    handler: select_list,
}];

pub static OBJECT_QUEUE: &[Entry] = &[Entry {
    command: 0x25,
    name: "detach child queue",
    capability: Capability::Scene,
    handler: root_scene,
}];

pub static TEXT_ALPHA: &[Entry] = &[Entry {
    command: 0x2f9,
    name: "text alpha revision",
    capability: Capability::Settings,
    handler: root_settings,
}];

pub static POLAR_POINT: &[Entry] = &[
    Entry {
        command: 0xd8,
        name: "integer polar point",
        capability: Capability::Math,
        handler: polar_point,
    },
    Entry {
        command: 0xd9,
        name: "hundredth-degree polar point",
        capability: Capability::Math,
        handler: polar_point,
    },
];

pub static AUDIO_PRELOAD: &[Entry] = &[
    Entry {
        command: 0x248,
        name: "preload audio",
        capability: Capability::Audio,
        handler: preload_audio,
    },
    Entry {
        command: 0x249,
        name: "release preloaded audio",
        capability: Capability::Audio,
        handler: preload_audio,
    },
    Entry {
        command: 0x24a,
        name: "play preloaded audio",
        capability: Capability::Audio,
        handler: preload_audio,
    },
    Entry {
        command: 0x24b,
        name: "stop preloaded audio",
        capability: Capability::Audio,
        handler: preload_audio,
    },
    Entry {
        command: 0x24f,
        name: "preloaded audio volume",
        capability: Capability::Audio,
        handler: preload_audio,
    },
];

pub static SCREEN_COORDINATES: &[Entry] = &[Entry {
    command: 0x4e,
    name: "screen coordinate mode",
    capability: Capability::Scene,
    handler: screen_coordinates,
}];

pub static SE_RESTART: &[Entry] = &[Entry {
    command: 0xb4,
    name: "restart SE channel",
    capability: Capability::Audio,
    handler: root_audio,
}];

pub static SE_GAIN: &[Entry] = &[Entry {
    command: 0xb8,
    name: "SE gain multiplier",
    capability: Capability::Audio,
    handler: se_gain,
}];

pub static MIXER_GAIN: &[Entry] = &[Entry {
    command: 0xa5,
    name: "device mixer gain",
    capability: Capability::Audio,
    handler: mixer_gain,
}];

pub static ACTIVATION_PASS: &[Entry] = &[
    Entry {
        command: 0x2c0,
        name: "activation check (reported as passed)",
        capability: Capability::Math,
        handler: activation_pass,
    },
    Entry {
        command: 0x2c1,
        name: "activation check (reported as passed)",
        capability: Capability::Math,
        handler: activation_pass,
    },
];

/// cmvs-20070119 effect slots of types 0x50.. ; see effect_objects::legacy.
pub static LEGACY_EFFECTS: &[Entry] = &[
    Entry {
        command: 0x2a3,
        name: "create legacy falling flakes",
        capability: Capability::ParticleImages,
        handler: create_legacy_flakes,
    },
    Entry {
        command: 0x2a1,
        name: "create legacy rain streak effect",
        capability: Capability::ParticleImages,
        handler: create_rain,
    },
    Entry {
        command: 0x294,
        name: "create magic effect",
        capability: Capability::SceneImages,
        handler: legacy_magic,
    },
    Entry {
        command: 0x293,
        name: "create curtain effect",
        capability: Capability::SceneImages,
        handler: legacy_curtain,
    },
    Entry {
        command: 0x295,
        name: "create sprite effect",
        capability: Capability::SceneImages,
        handler: legacy_sprites,
    },
    Entry {
        command: 0x296,
        name: "create floating planes effect",
        capability: Capability::SceneImages,
        handler: legacy_floating_planes,
    },
    Entry {
        command: 0x297,
        name: "create vortex effect",
        capability: Capability::SceneImages,
        handler: legacy_vortex,
    },
    Entry {
        command: 0x290,
        name: "create snow field effect",
        capability: Capability::SceneImages,
        handler: legacy_snow_field,
    },
    Entry {
        command: 0x292,
        name: "create circle effect",
        capability: Capability::SceneImages,
        handler: legacy_circle,
    },
    Entry {
        command: 0x291,
        name: "create walk effect",
        capability: Capability::Scene,
        handler: legacy_walk,
    },
    Entry {
        command: 0x2a0,
        name: "release effect object",
        capability: Capability::Scene,
        handler: root_effect_objects,
    },
    Entry {
        command: 0x2ae,
        name: "query effect object",
        capability: Capability::Scene,
        handler: legacy_query,
    },
    Entry {
        command: 0x2af,
        name: "set effect object parameter",
        capability: Capability::Scene,
        handler: legacy_set,
    },
];

pub static CAMERA_MOVE_X: &[Entry] = &[Entry {
    command: 0x6e,
    name: "move camera X",
    capability: Capability::Camera,
    handler: root_camera,
}];

pub static DRAW_QUERY: &[Entry] = &[Entry {
    command: 0x4f,
    name: "object draw query",
    capability: Capability::Scene,
    handler: draw_query,
}];

pub static SEQUENCE_WINDOW_IMAGE: &[Entry] = &[Entry {
    command: 0x28b,
    name: "append window image replacement",
    capability: Capability::Sequence,
    handler: root_sequence,
}];

pub static WINDOW_DRAW_SCALE: &[Entry] = &[Entry {
    command: 0x18b,
    name: "window draw scale",
    capability: Capability::Windows,
    handler: root_windows,
}];

pub static WINDOW_CHILD_IMAGE: &[Entry] = &[Entry {
    command: 0x176,
    name: "window child image",
    capability: Capability::WindowImages,
    handler: root_window_images,
}];

pub static VIEW_MOTION: &[Entry] = &[
    Entry {
        command: 0x270,
        name: "stop view motion",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x271,
        name: "wait view motion",
        capability: Capability::CameraWait,
        handler: root_wait_view,
    },
    Entry {
        command: 0x272,
        name: "view moving",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x273,
        name: "move view position",
        capability: Capability::Camera,
        handler: root_camera,
    },
    Entry {
        command: 0x274,
        name: "move view angles",
        capability: Capability::Camera,
        handler: root_camera,
    },
];

pub static WINDOW_SCALE_MOTION: &[Entry] = &[
    Entry {
        command: 0x1c0,
        name: "create window scale motion",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x1c1,
        name: "set key window scale motion",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x1c2,
        name: "start window scale motion",
        capability: Capability::Windows,
        handler: root_windows,
    },
];

pub static WINDOW_POSITION_MOTION: &[Entry] = &[
    Entry {
        command: 0x1d4,
        name: "create window position motion",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x1d5,
        name: "set key window position motion",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x1d6,
        name: "start window position motion",
        capability: Capability::Windows,
        handler: root_windows,
    },
];

pub static WINDOW_OPACITY_TRACK: &[Entry] = &[
    Entry {
        command: 0x1ca,
        name: "create window opacity track",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x1cb,
        name: "set key window opacity track",
        capability: Capability::Windows,
        handler: root_windows,
    },
    Entry {
        command: 0x1cc,
        name: "start window opacity track",
        capability: Capability::Windows,
        handler: root_windows,
    },
];

pub static LOGICAL_TEXT: &[Entry] = &[Entry {
    command: 0x11c,
    name: "create text object with logical size",
    capability: Capability::TextState,
    handler: root_text_state,
}];

pub static IMAGE_TABLES: &[Entry] = &[
    Entry {
        command: 0x17,
        name: "register stand image archive",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x18,
        name: "register auxiliary image archive",
        capability: Capability::Bootstrap,
        handler: root_bootstrap,
    },
    Entry {
        command: 0x32,
        name: "stand scene image",
        capability: Capability::SceneImages,
        handler: root_images,
    },
];

pub static WINDOW_FULL_MOVE: &[Entry] = &[Entry {
    command: 0x2ec,
    name: "window full move",
    capability: Capability::Platform,
    handler: window_full_move,
}];

pub static BACKGROUND_AUDIO: &[Entry] = &[Entry {
    command: 0x2ed,
    name: "background audio output",
    capability: Capability::Audio,
    handler: background_audio,
}];

pub static INPUT_PRESS: &[Entry] = &[
    Entry {
        command: 0x19c,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x19e,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1a0,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1a2,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1a4,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1a6,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1a8,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1aa,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1ac,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1ae,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1b0,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1b2,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1b4,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1b6,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1b8,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1ba,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x1bc,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x2f0,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x2f1,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x2f2,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x2f3,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x2f4,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
    Entry {
        command: 0x2f5,
        name: "input down edge",
        capability: Capability::Input,
        handler: variant_input_press,
    },
];

/// Static variant tables used when provisioning shared host capabilities.
pub static TEXT_DIALOG: &[Entry] = &[
    Entry {
        command: 0x2bc,
        name: "save comment",
        capability: Capability::Runtime,
        handler: root_runtime,
    },
    Entry {
        command: 0x13c,
        name: "text input dialog",
        capability: Capability::TextDialog,
        handler: variant_text_dialog,
    },
];
