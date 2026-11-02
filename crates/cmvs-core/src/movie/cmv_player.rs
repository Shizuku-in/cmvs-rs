//! CMV full-screen player state, commands and frame timing.
//!
//! The full-screen player at engine +4472 (0x38-byte object, constructor 0x1400375e0) is
//! driven by 0x20c0 (open and play), 0x20c1 (stop) and 0x20c2 (current frame). It opens a
//! CMV6 file below the 0x2015 video directory, starts the Ogg soundtrack on audio device
//! channel 2 and advances one frame index per decoded frame. Desktop enables JBPD decoding;
//! timing-only research remains explicit. The clock still stands in for soundtrack position.
//!
use super::{MovieHost, MovieSource};
use crate::{
    audio::{AudioBackend, AudioRequest, AudioSettings, MovieAudioClock},
    cmv::CmvIndex,
    dispatch::Call,
    evaluate::error,
    machine::Machine,
};
use std::error::Error;

/// The +4472 player while a movie runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Movie {
    pub path: Vec<u8>,
    pub index: CmvIndex,
    /// +44: the frame index 0x20c2 reports; frame 0 is decoded when opening.
    pub frame: u32,
    /// Milliseconds since opening, standing in for the soundtrack position.
    pub elapsed: u32,
    pub image: Option<crate::raster::RgbaImage>,
}

impl Movie {
    /// 0x140037bb0 maps a soundtrack position in ms to a frame; rates of 100 or more use
    /// doubles. Decode mode +4 = (+1636 = 0) % 3 skips no frames.
    fn target(&self) -> u32 {
        self.target_at(self.elapsed)
    }
    fn target_at(&self, elapsed: u32) -> u32 {
        let (rate, time) = (self.index.header.rate, self.index.header.rate_time);
        if rate < 100 {
            elapsed.wrapping_mul(rate) / 1000 / time
        } else {
            (f64::from(rate as i32) / 1000.0 / f64::from(time as i32) * f64::from(elapsed as i32))
                as i32 as u32
        }
    }
}

impl MovieHost {
    /// Real audio time is absolute and never supplemented by wall time. Ended releases
    /// the player, including when audio duration differs from the container's frame limit.
    pub(super) fn advance_cmv_with_clock(
        &mut self,
        delta_ms: u32,
        clock: Option<MovieAudioClock>,
        source: &mut dyn MovieSource,
    ) -> Result<(), Box<dyn Error>> {
        let Some(movie) = &self.player else {
            return Ok(());
        };
        if clock == Some(MovieAudioClock::Ended) {
            self.player = None;
            return Ok(());
        }
        let elapsed = match clock {
            Some(MovieAudioClock::Playing { position_ms }) => position_ms.max(movie.elapsed),
            _ => movie.elapsed.saturating_add(delta_ms),
        };
        let target = movie.target_at(elapsed);
        let image = if target > movie.frame
            && target < movie.index.header.max_sequence
            && self.decode_frames
        {
            Some(source.frame(&movie.path, &movie.index, target)?)
        } else {
            None
        };
        if target >= movie.index.header.max_sequence {
            self.player = None;
            return Ok(());
        }
        let movie = self.player.as_mut().expect("checked active player");
        movie.elapsed = elapsed;
        if target > movie.frame {
            movie.frame = target;
            movie.image = None;
        }
        if let Some(image) = image {
            movie.image = Some(image);
        }
        Ok(())
    }
    /// 0x20c0 / 0x140071500: SP-4 name; destroys any player, opens root + 0x2015 entry +
    /// name, starts the soundtrack at (+1612 * +1628) >> 8 and sets system result 0 to
    /// whether the file opened; returns 0x4004. 0x20c1 / 0x140071680 destroys the player
    /// (stopping channel 2) and returns 0x4000. 0x20c2 / 0x1400716d0 sets system result 0
    /// to the frame index, or -1 without a player, and returns 0x4000.
    pub fn try_handle_player(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn MovieSource,
        settings: AudioSettings,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, command @ 0xc0..=0xc2)) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle_player(&mut Call::new(vm, pc, command), source, settings, backend)?;
        Ok(true)
    }

    pub(crate) fn handle_player(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn MovieSource,
        settings: AudioSettings,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        let (pc, command) = (call.pc, call.command);
        let op = command + 0x2000;
        match command {
            0xc0 => {
                let raw = call.argument(4, "movie name")?;
                let name = call.vm.resolve_string(raw)?;
                let opened = source.open(&name)?;
                let image = if self.decode_frames {
                    opened
                        .as_ref()
                        .map(|(path, index)| source.frame(path, index, 0))
                        .transpose()?
                } else {
                    None
                };
                if let Some((path, index)) = &opened {
                    // 0x1400397e0/0x1400397b0 fetch the soundtrack; a CMV without one is the
                    // MOG_INputMemory crash RxCMVS notes, so it is rejected here.
                    let audio = index
                        .audio()
                        .ok_or_else(|| error(pc, op, "CMV without an Ogg soundtrack"))?;
                    backend.play_movie_audio(&AudioRequest::PlayMovieAudio {
                        path: path.clone(),
                        offset: audio.file_offset(&index.header),
                        size: audio.stored_size,
                        volume: settings.master.wrapping_mul(settings.movie) >> 8,
                    })?;
                } else {
                    // The failed player is destroyed at once; its destructor stops channel 2
                    // (the open stored the device pointer first).
                    backend.stop_movie_audio()?;
                }
                call.complete(0x4004)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(opened.is_some()))
                    .expect("slot 0 exists");
                self.player = opened.map(|(path, index)| Movie {
                    path,
                    index,
                    frame: 0,
                    elapsed: 0,
                    image,
                });
            }
            0xc1 => {
                if self.player.is_some() {
                    backend.stop_movie_audio()?;
                }
                call.complete(0x4000)?;
                self.player = None;
            }
            _ => {
                let frame = self.player.as_ref().map_or(u32::MAX, |movie| movie.frame);
                call.complete(0x4000)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, frame)
                    .expect("slot 0 exists");
            }
        }
        Ok(())
    }

    /// Frame update (main loop 0x14000e4a4 -> 0x140037bb0): a later frame index is decoded
    /// and becomes current; an index at or past `max_sequence` fails to decode and the main
    /// loop destroys the player. That happens when the soundtrack has ended, so no stop
    /// request is needed here.
    pub fn advance(&mut self, delta_ms: u32) {
        let Some(movie) = &mut self.player else {
            return;
        };
        movie.elapsed = movie.elapsed.saturating_add(delta_ms);
        let target = movie.target();
        if target <= movie.frame {
            return;
        }
        if target >= movie.index.header.max_sequence {
            self.player = None;
        } else {
            movie.frame = target;
            movie.image = None;
        }
    }
}
