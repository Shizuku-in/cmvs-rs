//! Movies; addresses refer to the documented binary hash.
//!
//! The full-screen player at engine +4472 (0x38-byte object, constructor 0x1400375e0) is
//! driven by 0x20c0 (open and play), 0x20c1 (stop) and 0x20c2 (current frame). It opens a
//! CMV6 file below the 0x2015 video directory, starts the Ogg soundtrack on audio device
//! channel 2 and advances one frame index per decoded frame. Desktop enables JBPD decoding;
//! timing-only research remains explicit. The clock still stands in for soundtrack position.
//!
//! Engine +4480 holds three scene players. 0x2308 creates a stopped CMV9 player and
//! initializes its bound scene object; 0x230a destroys the player, retaining that image.
//! Playback, audio-bearing slots and the other creation variant remain unhandled.
use crate::{
    audio::{AudioBackend, AudioRequest, AudioSettings, MovieAudioClock},
    bytecode::DecodeError,
    cmv::CmvIndex,
    evaluate::error,
    machine::Machine,
    platform::resolve_under,
    vm::SystemResult,
};
use std::{error::Error, fs::File, io, path::Path};

pub const MOVIE_SLOTS: usize = 3;

/// The game-relative path the soundtrack request names and the parsed container.
pub type OpenedMovie = (Vec<u8>, CmvIndex);

/// Opens movies by the name scripts pass to 0x20c0.
pub trait MovieSource {
    /// Ok(None) when the file does not exist (0x1400379f0 then fails quietly).
    fn open(&mut self, name: &[u8]) -> Result<Option<OpenedMovie>, Box<dyn Error>>;
    fn frame(
        &mut self,
        _path: &[u8],
        _index: &CmvIndex,
        _frame: u32,
    ) -> Result<crate::raster::RgbaImage, Box<dyn Error>> {
        Err("movie frame source unavailable".into())
    }
}

/// Files below the game root and the 0x2015 directory (engine +4832 = root + entry).
pub struct DirectoryMovies<'a> {
    pub root: &'a Path,
    pub directory: Option<&'a [u8]>,
}

impl MovieSource for DirectoryMovies<'_> {
    fn frame(
        &mut self,
        path: &[u8],
        index: &CmvIndex,
        frame: u32,
    ) -> Result<crate::raster::RgbaImage, Box<dyn Error>> {
        index.decode_frame(&mut File::open(resolve_under(self.root, path)?)?, frame)
    }
    fn open(&mut self, name: &[u8]) -> Result<Option<OpenedMovie>, Box<dyn Error>> {
        let directory = self
            .directory
            .ok_or("movie directory 0x2015 was not registered")?;
        let mut relative = directory.to_vec();
        relative.extend_from_slice(name);
        let path = match resolve_under(self.root, &relative) {
            Ok(path) => path,
            Err(e)
                if e.downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::NotFound) =>
            {
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(Some((relative, CmvIndex::read(&mut file)?)))
    }
}

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

/// 0x1400383c0 / 0x1400390b0: a stopped player bound to a separately owned scene object.
#[derive(Debug)]
pub struct SceneMovie {
    pub object_slot: u8,
    pub name: Vec<u8>,
    pub frame: u32,
    pub index: crate::cmv9::Cmv9Index,
    data: Vec<u8>,
}
impl SceneMovie {
    pub fn encoded(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Debug, Default)]
pub struct MovieHost {
    /// The constructor (0x140014fc2..0x140014fd0) stores null in all three pointers.
    slots: [Option<SceneMovie>; MOVIE_SLOTS],
    player: Option<Movie>,
    decode_frames: bool,
}

impl MovieHost {
    /// Full-screen timing-only mode is the default; desktop enables decoding.
    /// Scene-slot creation always decodes its initial image before committing.
    pub fn set_decode_frames(&mut self, enabled: bool) {
        self.decode_frames = enabled;
    }
    /// Decode before committing the new frame/time. Errors preserve the prior player.
    pub fn advance_decoded(
        &mut self,
        delta_ms: u32,
        source: &mut dyn MovieSource,
    ) -> Result<(), Box<dyn Error>> {
        self.advance_with_clock(delta_ms, None, source)
    }
    /// Real audio time is absolute and never supplemented by wall time. Ended releases
    /// the player, including when audio duration differs from the container's frame limit.
    pub fn advance_with_clock(
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
    pub fn occupied(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(Option::is_some)
    }
    pub fn player(&self) -> Option<&Movie> {
        self.player.as_ref()
    }
    pub fn scene_player(&self, slot: usize) -> Option<&SceneMovie> {
        self.slots.get(slot).and_then(Option::as_ref)
    }

    /// 0x2308 / 0x14007ef20: SP-4 player, SP-8 object, SP-12 name, SP-20 initial frame.
    /// SP-16 is not read; return 0x4014 without writing a system result. Decode before
    /// committing so an I/O/format error retains the pending command and old objects.
    pub fn try_create_scene(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut crate::scene::SceneHost,
        source: &mut dyn crate::resources::ResourceSource,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, 0x308)) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, 0x2308, why);
        let arg = |at| vm.stack().peek_u32(at).map_err(|e| fail(e.0));
        let (slot, object, name, frame) = (arg(4)?, arg(8)?, arg(12)?, arg(20)?);
        let name = vm.resolve_string(name)?;
        if slot as usize >= MOVIE_SLOTS {
            return Err(fail("movie slot outside 0..2 sets an unmodeled flag").into());
        }
        let object_slot =
            u8::try_from(object).map_err(|_| fail("scene movie object outside 0..255"))?;
        if name.len() >= 64 || name.contains(&0) {
            return Err(fail("scene movie name exceeds the 63-byte research limit").into());
        }
        if self.slots.iter().enumerate().any(|(i, p)| {
            i != slot as usize && p.as_ref().is_some_and(|p| p.object_slot == object_slot)
        }) {
            return Err(
                fail("replacing an object bound to another movie player is not modeled").into(),
            );
        }
        let data = source
            .read(&name)?
            .ok_or_else(|| fail("missing scene movie leaves an unmodeled failed player"))?;
        let index = crate::cmv9::Cmv9Index::parse(&data)?;
        let pixels = index.decode_frame(&data, frame)?;
        let mut draw = crate::scene::DrawState::default();
        draw.words[4] = pixels.width();
        draw.words[5] = pixels.height();
        let object = crate::scene::SceneObject {
            image: Some(crate::scene::ObjectImage {
                name: name.clone(),
                pixels,
                alpha: false,
            }),
            draw: Some(draw),
            ..crate::scene::SceneObject::default()
        };
        vm.complete_system(SystemResult(0x4014))?;
        scene.replace_movie_object(object_slot, object);
        self.slots[slot as usize] = Some(SceneMovie {
            object_slot,
            name,
            frame,
            index,
            data,
        });
        Ok(true)
    }

    /// 0x230a / 0x14007f390: SP-4 slot compared unsigned against 2; destroys and clears the
    /// slot's player if present and returns 0x4004. Out-of-range slots only set +12056 bit
    /// 0x1000000 in the original, which is not modeled.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, 0x30a)) = vm.pending_system() else {
            return Ok(false);
        };
        let slot = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x230a, e.0))? as usize;
        if slot >= MOVIE_SLOTS {
            return Err(error(
                pc,
                0x230a,
                "movie slot outside 0..2 sets an unmodeled flag",
            ));
        }
        vm.complete_system(SystemResult(0x4004))?;
        self.slots[slot] = None;
        Ok(true)
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
        let op = command + 0x2000;
        match command {
            0xc0 => {
                let raw = vm.stack().peek_u32(4).map_err(|e| error(pc, op, e.0))?;
                let name = vm.resolve_string(raw)?;
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
                vm.complete_system(SystemResult(0x4004))?;
                vm.state_mut()
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
                vm.complete_system(SystemResult(0x4000))?;
                self.player = None;
            }
            _ => {
                let frame = self.player.as_ref().map_or(u32::MAX, |movie| movie.frame);
                vm.complete_system(SystemResult(0x4000))?;
                vm.state_mut()
                    .set_system_integer(0, frame)
                    .expect("slot 0 exists");
            }
        }
        Ok(true)
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
