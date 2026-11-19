//! Services supplied by an executor and the capabilities they make available.
use crate::{
    audio::{AudioBackend, AudioHost},
    bootstrap::Bootstrap,
    buttons::ButtonHost,
    camera::CameraHost,
    capture::ScreenCapture,
    fade::SkipGate,
    gesture::PointerGesture,
    history::HistoryHost,
    image::ImageSource,
    input::ActionInput,
    message::MessageHost,
    message_flags::MessageFlags,
    movie::{MovieHost, MovieSource},
    platform::{PlatformAssets, PlatformBackend, PlatformHost},
    random::CrtRandom,
    resources::ResourceSource,
    save_menu::{SaveMenuHost, SaveSource},
    scene::SceneHost,
    script_control::{ScriptControl, ScriptSource},
    sequence::{SequenceContext, SequenceHost},
    settings::SettingsHost,
    snapshot::{SaveInfo, SaveTitle},
    system_save::{SystemSaveHost, SystemSaveSink, SystemSaveSource},
    timers::VmTimers,
};

pub struct InputHosts<'a> {
    pub input: &'a mut ActionInput,
    pub skip: bool,
    /// Current host clock for drag initiation; absence keeps that capability unavailable.
    pub clock: Option<u32>,
}

pub struct ImageHosts<'a> {
    pub scene: &'a mut SceneHost,
    pub source: &'a mut dyn ImageSource,
    pub change_images: bool,
}

pub struct AudioHosts<'a> {
    pub audio: &'a mut AudioHost,
    pub backend: &'a mut dyn AudioBackend,
}

pub struct WaitHosts<'a> {
    pub scene: Option<&'a mut SceneHost>,
    pub camera: Option<&'a mut CameraHost>,
    pub input: &'a mut ActionInput,
    pub gate: SkipGate,
    pub change_images: bool,
}

pub struct SnapshotHosts<'a> {
    pub image: &'a mut crate::snapshot::SaveImage,
    pub capture: bool,
    pub continue_without_image: bool,
}

pub struct MessageWaitHosts<'a> {
    pub messages: &'a mut MessageHost,
    pub input: &'a mut ActionInput,
    pub flags: MessageFlags,
    pub context: crate::message::AutoAdvanceContext<'a>,
}

pub struct ScriptHosts<'a> {
    pub host: &'a mut ScriptControl,
    pub source: &'a mut dyn ScriptSource,
}

pub struct SystemReadHosts<'a> {
    pub host: &'a mut SystemSaveHost,
    pub source: &'a mut dyn SystemSaveSource,
    pub game_id: Option<u32>,
    /// CVNS persistent ranges, from [`Bootstrap::cvns_persistent_starts`].
    pub persistent: crate::system_save::PersistentStarts,
}

pub struct SystemWriteHosts<'a> {
    pub host: &'a mut SystemSaveHost,
    pub sink: &'a mut dyn SystemSaveSink,
    pub random: &'a mut CrtRandom,
    pub game_id: Option<u32>,
}

pub struct PlatformHosts<'a> {
    pub host: &'a mut PlatformHost,
    pub backend: &'a mut dyn PlatformBackend,
    pub assets: &'a mut dyn PlatformAssets,
    pub cursors: &'a [Vec<u8>],
}

/// The existing startup setting and input-reporting modes have different effects.
/// Callers choose explicitly; handler order never chooses the function-key policy.
pub enum FunctionKeys<'a> {
    Runtime(&'a mut Bootstrap),
    Input(&'a mut ActionInput),
}

pub struct SequenceHosts<'a> {
    pub host: &'a mut SequenceHost,
    pub execution: Option<SequenceContext<'a>>,
}

pub struct MovieSceneHosts<'a> {
    pub scene: &'a mut SceneHost,
    pub source: &'a mut dyn ResourceSource,
}

pub struct MoviePlayerHosts<'a> {
    pub source: &'a mut dyn MovieSource,
    pub settings: crate::audio::AudioSettings,
    pub backend: &'a mut dyn AudioBackend,
}

pub struct TextHosts<'a> {
    pub messages: &'a mut MessageHost,
    pub input: &'a mut ActionInput,
    pub flags: MessageFlags,
}

#[derive(Default)]
pub struct Hosts<'a> {
    pub runtime: bool,
    pub in_game_message_dialogs: bool,
    pub in_game_text_dialogs: bool,
    pub snapshot: Option<SnapshotHosts<'a>>,
    pub message_wait: Option<MessageWaitHosts<'a>>,
    pub scripts: Option<ScriptHosts<'a>>,
    pub system_read: Option<SystemReadHosts<'a>>,
    pub system_write: Option<SystemWriteHosts<'a>>,
    pub save_source: Option<&'a mut dyn SaveSource>,
    pub bootstrap: Option<&'a mut Bootstrap>,
    pub function_keys: Option<FunctionKeys<'a>>,
    pub platform: Option<PlatformHosts<'a>>,
    pub device_frame: Option<&'a mut dyn PlatformBackend>,
    pub capture: Option<&'a mut ScreenCapture>,
    pub sequence: Option<SequenceHosts<'a>>,
    pub movies: Option<&'a mut MovieHost>,
    pub movie_scene: Option<MovieSceneHosts<'a>>,
    pub movie_player: Option<MoviePlayerHosts<'a>>,
    /// Scene playback is unavailable in tools that only expose slot queries.
    pub scene_movie_playback: bool,
    pub messages: Option<&'a mut MessageHost>,
    pub window_images: Option<&'a mut dyn ImageSource>,
    pub input: Option<InputHosts<'a>>,
    pub buttons: Option<&'a mut ButtonHost>,
    /// Explicit support for auxiliary-list interaction beyond list metadata.
    pub list_input: bool,
    pub scene: Option<&'a mut SceneHost>,
    pub camera: Option<&'a mut CameraHost>,
    pub images: Option<ImageHosts<'a>>,
    pub audio: Option<AudioHosts<'a>>,
    pub waits: Option<WaitHosts<'a>>,
    pub text: Option<TextHosts<'a>>,
    pub timers: Option<&'a mut VmTimers>,
    pub flags: Option<&'a mut MessageFlags>,
    pub random: Option<&'a mut CrtRandom>,
    pub settings: Option<&'a mut SettingsHost>,
    pub history: Option<&'a mut HistoryHost>,
    pub gesture: Option<&'a mut PointerGesture>,
    pub popup: Option<&'a mut SaveMenuHost>,
    pub save_title: Option<&'a mut SaveTitle>,
    pub save_info: Option<&'a mut SaveInfo>,
    pub effects: Option<&'a mut bool>,
    /// Availability of stateless services in this host environment.
    pub strings: bool,
    pub math: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Runtime,
    Snapshot,
    MessageWait,
    ScriptControl,
    SystemRead,
    SystemWrite,
    SaveMenu,
    Bootstrap,
    FunctionKeys,
    Platform,
    FontDialog,
    TextDialog,
    ColorDialog,
    Capture,
    CaptureDevice,
    Sequence,
    MovieState,
    MovieScene,
    MovieStart,
    MoviePlayer,
    Quake,
    Strip,
    TextState,
    AuxiliaryScene,
    Windows,
    WindowPointer,
    WindowButtons,
    WindowImages,
    Input,
    Buttons,
    ButtonInput,
    ListInput,
    DragPress,
    Scene,
    Camera,
    SceneImages,
    MaskImages,
    ImageChange,
    Audio,
    Text,
    Timers,
    CvnsTiming,
    Flags,
    Random,
    Settings,
    CvnsConfig,
    History,
    Strings,
    Math,
    Gesture,
    Popup,
    SaveTitle,
    SaveInfo,
    Effects,
    EffectObjects,
    ParticleImages,
    SceneRandom,
    FadeWait,
    MaskWait,
    QuakeWait,
    StripWait,
    ChangeWait,
    CameraWait,
}

impl Capability {
    pub(crate) fn uses_input(self) -> bool {
        matches!(
            self,
            Self::Input
                | Self::ButtonInput
                | Self::ListInput
                | Self::DragPress
                | Self::WindowPointer
                | Self::WindowButtons
        )
    }

    pub(crate) fn uses_windows(self) -> bool {
        matches!(
            self,
            Self::Windows | Self::WindowPointer | Self::WindowButtons | Self::WindowImages
        )
    }

    pub(crate) fn uses_message_state(self, command: u16) -> bool {
        // The font-name query needs the message host's backend to show a readable name.
        matches!(
            command,
            0x2de | crate::cvns::cmd::GET_CONFIG | crate::cvns::cmd::SET_CONFIG
        ) || self.uses_windows()
            || matches!(
                self,
                Self::TextState | Self::AuxiliaryScene | Self::FontDialog
            )
    }

    pub(crate) fn uses_bootstrap(self) -> bool {
        matches!(self, Self::Bootstrap | Self::FunctionKeys)
    }

    pub(crate) fn uses_device_frame(self) -> bool {
        matches!(
            self,
            Self::CaptureDevice | Self::FontDialog | Self::TextDialog | Self::ColorDialog
        )
    }

    pub(crate) fn uses_scene_images(self) -> bool {
        matches!(
            self,
            Self::SceneImages | Self::MaskImages | Self::ImageChange | Self::ParticleImages
        )
    }

    pub(crate) fn is_wait(self) -> bool {
        matches!(
            self,
            Self::FadeWait
                | Self::MaskWait
                | Self::QuakeWait
                | Self::StripWait
                | Self::ChangeWait
                | Self::CameraWait
        )
    }
}

impl Hosts<'_> {
    pub(super) fn has(&self, capability: Capability) -> bool {
        match capability {
            Capability::Runtime => self.runtime,
            Capability::Snapshot => self.snapshot.is_some(),
            Capability::MessageWait => self.message_wait.is_some(),
            Capability::ScriptControl => self.scripts.is_some(),
            Capability::SystemRead => self.system_read.is_some(),
            Capability::SystemWrite => self.system_write.is_some(),
            Capability::SaveMenu => self.popup.is_some() && self.save_source.is_some(),
            Capability::Bootstrap => self.bootstrap.is_some(),
            Capability::FunctionKeys => self.function_keys.is_some(),
            Capability::Platform => self.platform.is_some(),
            Capability::TextDialog | Capability::ColorDialog => self.device_frame.is_some(),
            Capability::FontDialog => {
                self.device_frame.is_some() && self.settings.is_some() && self.messages.is_some()
            }
            Capability::Capture => self.capture.is_some() && self.scene.is_some(),
            Capability::CaptureDevice => {
                self.capture.is_some() && self.scene.is_some() && self.device_frame.is_some()
            }
            Capability::Sequence => self.sequence.is_some(),
            Capability::MovieState => self.movies.is_some(),
            Capability::MovieScene => self.movies.is_some() && self.movie_scene.is_some(),
            Capability::MovieStart => self.movies.is_some() && self.scene_movie_playback,
            Capability::MoviePlayer => self.movies.is_some() && self.movie_player.is_some(),
            Capability::TextState | Capability::AuxiliaryScene => self.messages.is_some(),
            Capability::Windows => self.messages.is_some(),
            Capability::WindowPointer | Capability::WindowButtons => {
                self.messages.is_some() && self.input.is_some()
            }
            Capability::WindowImages => self.messages.is_some() && self.window_images.is_some(),
            Capability::Input => self.input.is_some(),
            Capability::Buttons => self.buttons.is_some() && self.scene.is_some(),
            Capability::ButtonInput => {
                self.buttons.is_some() && self.scene.is_some() && self.input.is_some()
            }
            Capability::ListInput => {
                self.buttons.is_some()
                    && self.scene.is_some()
                    && self.input.is_some()
                    && self.gesture.is_some()
                    && self.list_input
            }
            Capability::DragPress => {
                self.gesture.is_some() && self.input.as_ref().is_some_and(|h| h.clock.is_some())
            }
            Capability::Scene | Capability::Quake | Capability::Strip => self.scene.is_some(),
            Capability::Camera => self.camera.is_some(),
            Capability::SceneImages | Capability::MaskImages => self.images.is_some(),
            Capability::ImageChange => self.images.as_ref().is_some_and(|h| h.change_images),
            Capability::Audio => self.audio.is_some(),
            Capability::Text => self.text.is_some(),
            Capability::Timers => self.timers.is_some(),
            Capability::CvnsTiming => self.timers.is_some() && self.settings.is_some(),
            Capability::Flags => self.flags.is_some(),
            Capability::Random => self.random.is_some(),
            Capability::Settings => self.settings.is_some(),
            Capability::CvnsConfig => {
                self.settings.is_some()
                    && self.audio.is_some()
                    && self.popup.is_some()
                    && self.flags.is_some()
                    && self.effects.is_some()
            }
            Capability::History => self.history.is_some(),
            Capability::Strings => self.strings,
            Capability::Math => self.math,
            Capability::Gesture => self.gesture.is_some(),
            Capability::Popup => self.popup.is_some(),
            Capability::SaveTitle => self.save_title.is_some(),
            Capability::SaveInfo => self.save_info.is_some(),
            Capability::Effects => self.effects.is_some(),
            Capability::EffectObjects => self.scene.is_some(),
            Capability::ParticleImages => self.images.is_some() && self.random.is_some(),
            Capability::SceneRandom => self.scene.is_some() && self.random.is_some(),
            Capability::FadeWait
            | Capability::MaskWait
            | Capability::QuakeWait
            | Capability::StripWait => self.waits.as_ref().is_some_and(|h| h.scene.is_some()),
            Capability::ChangeWait => self
                .waits
                .as_ref()
                .is_some_and(|h| h.scene.is_some() && h.change_images),
            Capability::CameraWait => self.waits.as_ref().is_some_and(|h| h.camera.is_some()),
        }
    }
}
