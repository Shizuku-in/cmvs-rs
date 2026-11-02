//! MGV full-screen player state, commands and stateful decoding.
use super::{MovieHost, MovieSource};
use crate::{
    audio::{AudioBackend, AudioRequest, AudioSettings, MovieAudioClock},
    dispatch::Call,
};
use std::error::Error;

#[derive(Debug)]
pub(super) struct MgvMovie {
    path: Vec<u8>,
    index: crate::mgv::MgvIndex,
    decoder: Box<dyn crate::mgv::Decoder>,
    frame: u32,
    elapsed: u32,
    pub(super) image: crate::raster::RgbaImage,
}

impl MovieHost {
    pub(crate) fn handle_mgv(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn MovieSource,
        settings: AudioSettings,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        match call.command {
            0xc4 => {
                let raw = call.argument(4, "MGV movie name")?;
                let name = call.vm.resolve_string(raw)?;
                let opened = source.open_mgv(&name)?;
                let candidate = if let Some((path, index, reader)) = opened {
                    let factory = self
                        .mgv_factory
                        .as_ref()
                        .ok_or("MGV decoder backend unavailable")?;
                    let mut decoder = factory.open(reader, &index)?;
                    let image = decoder.frame(0)?;
                    Some(MgvMovie {
                        path,
                        index,
                        decoder,
                        image,
                        frame: 0,
                        elapsed: 0,
                    })
                } else {
                    None
                };
                if let Some(movie) = &candidate {
                    if movie.index.audio_size == 0 {
                        return Err(
                            "MGV without an Ogg soundtrack is not supported for playback".into(),
                        );
                    }
                    backend.play_movie_audio(&AudioRequest::PlayMovieAudio {
                        path: movie.path.clone(),
                        offset: movie.index.audio_offset,
                        size: movie.index.audio_size,
                        volume: settings.master.wrapping_mul(settings.movie) >> 8,
                    })?;
                } else {
                    backend.stop_movie_audio()?;
                }
                call.complete(0x4004)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(candidate.is_some()))
                    .expect("slot 0 exists");
                self.mgv_player = candidate;
            }
            0xc5 => {
                if self.mgv_player.is_some() {
                    backend.stop_movie_audio()?;
                }
                call.complete(0x4000)?;
                self.mgv_player = None;
            }
            0xc6 => {
                let frame = self.mgv_player.as_ref().map_or(u32::MAX, |m| m.frame);
                call.complete(0x4000)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, frame)
                    .expect("slot 0 exists");
            }
            _ => unreachable!("MGV player dispatch"),
        }
        Ok(())
    }
    pub(crate) fn advance_mgv(
        &mut self,
        delta_ms: u32,
        clock: Option<MovieAudioClock>,
    ) -> Result<(), Box<dyn Error>> {
        let Some(movie) = &mut self.mgv_player else {
            return Ok(());
        };
        if clock == Some(MovieAudioClock::Ended) {
            self.mgv_player = None;
            return Ok(());
        }
        let elapsed = match clock {
            Some(MovieAudioClock::Playing { position_ms }) => position_ms.max(movie.elapsed),
            _ => movie.elapsed.saturating_add(delta_ms),
        };
        let frame = if movie.index.rate < 100 {
            elapsed.wrapping_mul(movie.index.rate) / 1000 / movie.index.rate_time
        } else {
            (f64::from(movie.index.rate) / 1000.0 / f64::from(movie.index.rate_time)
                * f64::from(elapsed)) as u32
        };
        if frame as usize >= movie.index.frames.len() {
            movie.decoder.finish()?;
            self.mgv_player = None;
        } else if frame > movie.frame {
            let image = movie.decoder.frame(frame)?;
            movie.frame = frame;
            movie.elapsed = elapsed;
            movie.image = image;
        } else {
            movie.elapsed = elapsed;
        }
        Ok(())
    }
}
