//! Explicit silent preview: commands keep the recording backend's failure results.
//! Each call uses a fresh recorder so long sessions cannot fill a research log.
use cmvs_core::audio::{
    AudioBackend, AudioRequest, RecordingAudio, SE_CHANNELS, SePlayback, VoiceDuckingSettings,
};
use std::error::Error;
pub struct SilentAudio;
macro_rules! forward {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) -> $result:ty;)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) -> $result {
            RecordingAudio::default().$name($($arg),*)
        })*
    };
}
impl AudioBackend for SilentAudio {
    forward! {
        fn set_output_enabled(enabled: bool) -> Result<(), Box<dyn Error>>;
        fn configure_voice_ducking(settings: VoiceDuckingSettings) -> Result<(), Box<dyn Error>>;
        fn configure_se(channels: &[Option<SePlayback>; SE_CHANNELS]) -> Result<(), Box<dyn Error>>;
        fn set_volumes(channels: &[Option<u32>; 15]) -> Result<(), Box<dyn Error>>;
        fn play(request: &AudioRequest) -> Result<bool, Box<dyn Error>>;
        fn start(channel: u8, looped: bool) -> Result<(), Box<dyn Error>>;
        fn stop(channel: u8) -> Result<(), Box<dyn Error>>;
        fn stop_channel15() -> Result<(), Box<dyn Error>>;
        fn fade_out_se(channel: u8, fade_ms: u32) -> Result<(), Box<dyn Error>>;
        fn stop_voice(channel: u8) -> Result<(), Box<dyn Error>>;
        fn fade_out_voice(channel: u8, fade_ms: u32) -> Result<(), Box<dyn Error>>;
        fn play_voice(request: &AudioRequest) -> Result<bool, Box<dyn Error>>;
        fn play_bgm(request: &AudioRequest) -> Result<(), Box<dyn Error>>;
        fn fade_out_bgm(channel: u8, fade_ms: u32) -> Result<(), Box<dyn Error>>;
        fn stop_bgm() -> Result<(), Box<dyn Error>>;
        fn play_movie_audio(request: &AudioRequest) -> Result<(), Box<dyn Error>>;
        fn stop_movie_audio() -> Result<(), Box<dyn Error>>;
    }
    fn se_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        Ok(None)
    }
    fn voice_position(&self, _channel: u8) -> Result<Option<u32>, Box<dyn Error>> {
        Ok(None)
    }
}
