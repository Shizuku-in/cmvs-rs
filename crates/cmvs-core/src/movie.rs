//! Movie sources, shared host and scene-player persistence.
//!
//! Full-screen CMV and MGV player commands/timing live in `cmv_player` and `mgv_player`.
//! Scene segment playback lives in `playback`.
//!
//! Engine +4480 holds three scene players. 0x2308 creates a stopped CMV6/CMV9 player and
//! initializes its bound scene object; 0x230a destroys the player, retaining that image.
//! 0x2309 starts default-mode segment playback and 0x231a queries whether a player is
//! active; 0x230b/0x230c resume/pause while retaining the frame. Other slot controls,
//! playback modes, audio-bearing slots and the other creation
//! variant remain unhandled.
use crate::filesystem::File;
use crate::{
    audio::MovieAudioClock, bytecode::DecodeError, cmv::CmvIndex, dispatch::Call, evaluate::error,
    machine::Machine, platform::resolve_under,
};
use std::{error::Error, io, path::Path};

pub const MOVIE_SLOTS: usize = 3;
mod cmv_player;
mod mgv_player;
mod playback;

pub use cmv_player::Movie;
use mgv_player::MgvMovie;
pub use playback::{MOVIE_SEGMENTS, MoviePlayback, MovieSegment};

/// The game-relative path the soundtrack request names and the parsed container.
pub type OpenedMovie = (Vec<u8>, CmvIndex);
pub type OpenedMgv = (Vec<u8>, crate::mgv::MgvIndex, Box<dyn crate::mgv::ReadSeek>);
type OpenedMovieFile = (Vec<u8>, File);

/// Opens movies by the name scripts pass to 0x20c0.
pub trait MovieSource {
    fn open_mgv(&mut self, _name: &[u8]) -> Result<Option<OpenedMgv>, Box<dyn Error>> {
        Err("MGV source unavailable".into())
    }
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
    fn open_mgv(&mut self, name: &[u8]) -> Result<Option<OpenedMgv>, Box<dyn Error>> {
        let Some((path, mut file)) = self.open_file(name)? else {
            return Ok(None);
        };
        let index = crate::mgv::MgvIndex::read(&mut file)?;
        Ok(Some((path, index, Box::new(file))))
    }
    fn frame(
        &mut self,
        path: &[u8],
        index: &CmvIndex,
        frame: u32,
    ) -> Result<crate::raster::RgbaImage, Box<dyn Error>> {
        index.decode_frame(&mut File::open(resolve_under(self.root, path)?)?, frame)
    }
    fn open(&mut self, name: &[u8]) -> Result<Option<OpenedMovie>, Box<dyn Error>> {
        let Some((path, mut file)) = self.open_file(name)? else {
            return Ok(None);
        };
        Ok(Some((path, CmvIndex::read(&mut file)?)))
    }
}
impl DirectoryMovies<'_> {
    fn open_file(&self, name: &[u8]) -> Result<Option<OpenedMovieFile>, Box<dyn Error>> {
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
        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(Some((relative, file)))
    }
}

/// 0x1400383c0 / 0x1400390b0: a scene player, initially stopped, bound to a separate object.
#[derive(Debug)]
pub struct SceneMovie {
    pub object_slot: u8,
    pub name: Vec<u8>,
    pub frame: u32,
    pub index: SceneMovieIndex,
    data: Vec<u8>,
    base: Option<crate::raster::RgbaImage>,
    pub playback: MoviePlayback,
    load_mode: u32,
}

/// Container-specific scene frame decoding; timing and object ownership are shared.
#[derive(Debug)]
pub enum SceneMovieIndex {
    Cmv6(CmvIndex),
    /// 32-bit JBPE frames with alpha (0x1400390b0 type 4).
    Cmv8(CmvIndex),
    Cmv9(crate::cmv9::Cmv9Index),
}
impl SceneMovieIndex {
    pub fn max_sequence(&self) -> u32 {
        match self {
            Self::Cmv6(index) | Self::Cmv8(index) => index.header.max_sequence,
            Self::Cmv9(index) => index.max_sequence,
        }
    }
    pub fn rate(&self) -> u32 {
        match self {
            Self::Cmv6(index) | Self::Cmv8(index) => index.header.rate,
            Self::Cmv9(index) => index.rate,
        }
    }
    pub fn rate_time(&self) -> u32 {
        match self {
            Self::Cmv6(index) | Self::Cmv8(index) => index.header.rate_time,
            Self::Cmv9(index) => index.rate_time,
        }
    }
    fn decode(
        &self,
        data: &[u8],
        frame: u32,
        base: Option<&crate::raster::RgbaImage>,
    ) -> Result<crate::raster::RgbaImage, Box<dyn Error>> {
        match self {
            Self::Cmv6(index) | Self::Cmv8(index) => {
                index.decode_frame(&mut io::Cursor::new(data), frame)
            }
            Self::Cmv9(index) => Ok(index.decode_with_base(
                data,
                frame,
                base.ok_or("CMV9 scene base image unavailable")?,
            )?),
        }
    }
}
impl SceneMovieIndex {
    /// The bound object's image: a surface, with alpha for type 4 (CMV8) as 0x14003c020's
    /// fifth argument gives it. `name` tags the binding in Rust only.
    fn frame_image(
        &self,
        name: &[u8],
        pixels: crate::raster::RgbaImage,
    ) -> crate::scene::ObjectImage {
        crate::scene::ObjectImage {
            filter: None,
            name: name.to_vec(),
            pixels,
            alpha: matches!(self, Self::Cmv8(_)),
            surface: true,
        }
    }
}
impl SceneMovie {
    pub fn encoded(&self) -> &[u8] {
        &self.data
    }
}

/// 0x1400390b0's container setup, shared by 0x2308 and the loader.
fn open_scene(
    data: &[u8],
) -> Result<(SceneMovieIndex, Option<crate::raster::RgbaImage>), Box<dyn Error>> {
    if matches!(data.get(..4), Some(b"CMV6" | b"CMV8")) {
        let index = CmvIndex::read_scene(&mut io::Cursor::new(data))?;
        // 0x140039c90 loads a nonempty final audio entry when +40 != 0;
        // 0x1400390b0 hands it to device channel slot+2. Do not drop audio.
        if index.header.audio_flag != 0
            && index
                .entries
                .last()
                .is_some_and(|entry| entry.stored_size != 0)
        {
            return Err("CMV6/CMV8 scene movie soundtrack is not implemented".into());
        }
        Ok(if index.alpha {
            (SceneMovieIndex::Cmv8(index), None)
        } else {
            (SceneMovieIndex::Cmv6(index), None)
        })
    } else {
        let index = crate::cmv9::Cmv9Index::parse(data)?;
        let base = index.decode_base(data)?;
        Ok((SceneMovieIndex::Cmv9(index), Some(base)))
    }
}

/// Record 0x8e8 body (0x140038420): u16 version 4, +268 player slot, +272 object slot,
/// the 64-byte name at +300, the 208-byte playback block +60..+268 (current and pending
/// segment, active, paused, sixteen looping/start/end segments), +280 frame, then +48,
/// +52 (load mode) and +56.
const SCENE_RECORD: usize = 298;
/// 0x2308 passes load mode 2; the reader reads 3 as 2.
const SCENE_MODE: u32 = 2;

#[derive(Debug, Default)]
pub struct MovieHost {
    /// The constructor (0x140014fc2..0x140014fd0) stores null in all three pointers.
    slots: [Option<SceneMovie>; MOVIE_SLOTS],
    player: Option<Movie>,
    mgv_player: Option<MgvMovie>,
    mgv_factory: Option<Box<dyn crate::mgv::DecoderFactory>>,
    decode_frames: bool,
    scene_clock: u32,
}

impl MovieHost {
    pub(crate) fn saved_scene_players_with_layout(
        &self,
        layout: crate::snapshot::SnapshotLayout,
    ) -> Vec<(u16, Vec<u8>)> {
        let mut records = self.saved_scene_players();
        if layout == crate::snapshot::SnapshotLayout::TextV1SceneV1 {
            // 0x41f1a0: v2 omits v4's three trailing control/mode words.
            for (_, body) in &mut records {
                body[..2].copy_from_slice(&2u16.to_le_bytes());
                body.truncate(286);
            }
        }
        records
    }

    pub(crate) fn load_scene_players_with_layout(
        &self,
        records: &[(u16, &[u8])],
        objects: &mut std::collections::BTreeMap<u8, crate::scene::SceneObject>,
        source: &mut dyn crate::resources::ResourceSource,
        layout: crate::snapshot::SnapshotLayout,
    ) -> Result<[Option<SceneMovie>; MOVIE_SLOTS], Box<dyn Error>> {
        if layout != crate::snapshot::SnapshotLayout::TextV1SceneV1 {
            return self.load_scene_players(records, objects, source);
        }
        let mut converted = Vec::new();
        for &(slot, body) in records {
            if body.len() != 286 || body[..2] != [2, 0] {
                return Err("unsupported scene movie v2 record".into());
            }
            let mut bytes = body.to_vec();
            bytes[..2].copy_from_slice(&4u16.to_le_bytes());
            for word in [0, SCENE_MODE, 0] {
                bytes.extend(word.to_le_bytes());
            }
            converted.push((slot, bytes));
        }
        let records: Vec<_> = converted.iter().map(|(s, b)| (*s, b.as_slice())).collect();
        self.load_scene_players(&records, objects, source)
    }

    pub fn set_mgv_factory(&mut self, factory: Box<dyn crate::mgv::DecoderFactory>) {
        self.mgv_factory = Some(factory);
    }
    pub fn has_player(&self) -> bool {
        self.player.is_some() || self.mgv_player.is_some()
    }
    pub fn player_image(&self) -> Option<&crate::raster::RgbaImage> {
        self.mgv_player
            .as_ref()
            .map(|m| &m.image)
            .or_else(|| self.player.as_ref().and_then(|m| m.image.as_ref()))
    }
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
        self.advance_mgv(delta_ms, clock)?;
        self.advance_cmv_with_clock(delta_ms, clock, source)
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
    /// 0x230e additionally consumes SP-24 as the load mode and returns 0x4018.
    /// Archive modes 2/3 are supported; loose-file modes 0/1 stay explicit barriers.
    pub fn try_create_scene(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut crate::scene::SceneHost,
        source: &mut dyn crate::resources::ResourceSource,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, command @ (0x308 | 0x30e))) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle_create_scene(&mut Call::new(vm, pc, command), scene, source)?;
        Ok(true)
    }

    pub(crate) fn handle_create_scene(
        &mut self,
        call: &mut Call<'_, '_>,
        scene: &mut crate::scene::SceneHost,
        source: &mut dyn crate::resources::ResourceSource,
    ) -> Result<(), Box<dyn Error>> {
        let pc = call.pc;
        let op = call.command + 0x2000;
        let fail = |why| error(pc, op, why);
        let (slot, object, name, frame) = (
            call.argument(4, "player slot")?,
            call.argument(8, "object slot")?,
            call.argument(12, "movie name")?,
            call.argument(20, "initial frame")?,
        );
        let name = call.vm.resolve_string(name)?;
        call.ignore_argument(16, "unused load parameter")?;
        let load_mode = if call.command == 0x30e {
            call.argument(24, "movie load mode")?
        } else {
            SCENE_MODE
        };
        // MIRANOSU 0x1400328a0: 2/3 read the registered archive; 0/1 open a loose
        // filename instead. Streaming mode 3 is decoded from bounded resident bytes.
        if !matches!(load_mode, 2 | 3) {
            return Err(fail("loose-file scene movie load modes are not implemented").into());
        }
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
        if data.len() > crate::MAX_PAYLOAD {
            return Err(fail("scene movie exceeds research limit").into());
        }
        let (index, base) = open_scene(&data)?;
        let pixels = index.decode(&data, frame, base.as_ref())?;
        let playback = MoviePlayback::new(frame, index.max_sequence() - 1);
        let mut draw = crate::scene::DrawState::default();
        draw.words[4] = pixels.width();
        draw.words[5] = pixels.height();
        let object = crate::scene::SceneObject {
            image: Some(index.frame_image(&name, pixels)),
            draw: Some(draw),
            ..crate::scene::SceneObject::default()
        };
        call.complete(if call.command == 0x30e {
            0x4018
        } else {
            0x4014
        })?;
        scene.replace_movie_object(object_slot, object);
        self.slots[slot as usize] = Some(SceneMovie {
            object_slot,
            name,
            frame,
            index,
            data,
            base,
            playback,
            load_mode,
        });
        Ok(())
    }

    /// 0x230a / 0x14007f390: SP-4 slot compared unsigned against 2; destroys and clears the
    /// slot's player if present and returns 0x4004. 0x231a / 0x14007f610 checks the same
    /// slot and sets system result 0 to whether 0x140038590 reads a nonzero +68 active flag,
    /// or 0 for an empty slot; returns 0x4004. A completed segment clears active while
    /// setting paused, so it reports 0. Out-of-range slots only set +12056 bit 0x1000000 in
    /// the original, which is not modeled.
    /// 0x230b / 0x14007f420 resumes (0x140038500), resetting the clock and fractional
    /// time only when active. 0x230c / 0x14007f470 pauses (0x1400384f0). Both preserve
    /// active/frame/segments and system results; empty slots are no-ops.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x30a..=0x30c | 0x31a | 0x31e))) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle(&mut Call::new(vm, pc, command))?;
        Ok(true)
    }

    pub(crate) fn handle(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
        let (pc, command) = (call.pc, call.command);
        let op = command + 0x2000;
        let slot = call.argument(4, "player slot")? as usize;
        if slot >= MOVIE_SLOTS {
            return Err(error(
                pc,
                op,
                "movie slot outside 0..2 sets an unmodeled flag",
            ));
        }
        call.complete(0x4004)?;
        if command == 0x30a {
            self.slots[slot] = None;
        } else if matches!(command, 0x30b | 0x30c) {
            if let Some(movie) = &mut self.slots[slot] {
                if command == 0x30b {
                    if movie.playback.active {
                        movie.playback.last_tick = self.scene_clock;
                        movie.playback.elapsed = 0;
                    }
                    movie.playback.paused = false;
                } else {
                    movie.playback.paused = true;
                }
            }
        } else if command == 0x31e {
            // MIRANOSU 0x140072ee0 -> 0x1400310b0: empty slots return zero.
            let frame = self.slots[slot].as_ref().map_or(0, |movie| movie.frame);
            call.vm
                .state_mut()
                .set_system_integer(0, frame)
                .expect("slot 0 exists");
        } else {
            let active = self.slots[slot].as_ref().is_some_and(|m| m.playback.active);
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(active))
                .expect("slot 0 exists");
        }
        Ok(())
    }
}

/// The parts of record 0x8e8 Rust models. +48 and +56 keep their constructor zero: only
/// unimplemented commands (0x14007f4c0, 0x14007f520) set them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SceneRecord {
    slot: u16,
    object_slot: u8,
    name: Vec<u8>,
    frame: u32,
    playback: MoviePlayback,
    load_mode: u32,
}
impl SceneRecord {
    fn encode(&self) -> Vec<u8> {
        let p = &self.playback;
        let mut body = 4u16.to_le_bytes().to_vec();
        body.extend(u32::from(self.slot).to_le_bytes());
        body.extend(u32::from(self.object_slot).to_le_bytes());
        let mut name = [0u8; 64];
        name[..self.name.len()].copy_from_slice(&self.name);
        body.extend(name);
        let head = [
            p.current as u32,
            p.pending.map_or(u32::MAX, |n| n as u32),
            u32::from(p.active),
            u32::from(p.paused),
        ];
        let segments = p
            .segments
            .iter()
            .flat_map(|s| [u32::from(s.looping), s.start, s.end]);
        for word in head
            .into_iter()
            .chain(segments)
            .chain([self.frame, 0, self.load_mode, 0])
        {
            body.extend(word.to_le_bytes());
        }
        body
    }
    /// The playback clock restarts at `now` with no elapsed time, as the reader sets it.
    fn decode(slot: u16, body: &[u8], now: u32) -> Result<Self, &'static str> {
        if body.len() != SCENE_RECORD || body[..2] != [4, 0] {
            return Err("unsupported scene movie record");
        }
        let word = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
        if slot as usize >= MOVIE_SLOTS || word(2) != u32::from(slot) {
            return Err("scene movie record names another player slot");
        }
        if word(286) != 0 || !matches!(word(290), 2 | 3) || word(294) != 0 {
            return Err("scene movie load mode or control fields are not implemented");
        }
        let object_slot = u8::try_from(word(6)).map_err(|_| "scene movie object outside 0..255")?;
        let name = &body[10..74];
        let end = name
            .iter()
            .position(|&b| b == 0)
            .ok_or("scene movie name lacks its NUL")?;
        let flag = |at: usize| match word(at) {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("scene movie flag is not 0 or 1"),
        };
        let (current, pending) = (word(74) as usize, word(78));
        if current >= MOVIE_SEGMENTS || pending != u32::MAX && pending as usize >= MOVIE_SEGMENTS {
            return Err("scene movie segment outside 0..15");
        }
        let mut segments = [MovieSegment::default(); MOVIE_SEGMENTS];
        for (i, segment) in segments.iter_mut().enumerate() {
            let at = 90 + 12 * i;
            *segment = MovieSegment {
                looping: word(at) != 0,
                start: word(at + 4),
                end: word(at + 8),
            };
        }
        Ok(Self {
            slot,
            object_slot,
            name: name[..end].to_vec(),
            frame: word(282),
            load_mode: SCENE_MODE, // The native reader reopens streaming mode 3 as mode 2.
            playback: MoviePlayback {
                segments,
                current,
                pending: (pending != u32::MAX).then_some(pending as usize),
                active: flag(82)?,
                paused: flag(86)?,
                elapsed: 0,
                last_tick: now,
            },
        })
    }
}

impl MovieHost {
    /// Records 0x8e8 for the +4480 scene players; the +4472 player is never saved.
    pub(crate) fn saved_scene_players(&self) -> Vec<(u16, Vec<u8>)> {
        let mut out = Vec::new();
        for (slot, movie) in self.slots.iter().enumerate() {
            let Some(movie) = movie else { continue };
            let record = SceneRecord {
                slot: slot as u16,
                object_slot: movie.object_slot,
                name: movie.name.clone(),
                frame: movie.frame,
                playback: movie.playback.clone(),
                load_mode: movie.load_mode,
            };
            out.push((record.slot, record.encode()));
        }
        out
    }
    /// Reader 0x140039630 over objects already rebuilt from records 0x700: reopen each
    /// movie, recreate the bound object's surface (0x14003c020) with the saved frame, then
    /// restore the playback block. Elapsed time restarts at the current tick (+284/+288).
    pub(crate) fn load_scene_players(
        &self,
        records: &[(u16, &[u8])],
        objects: &mut std::collections::BTreeMap<u8, crate::scene::SceneObject>,
        source: &mut dyn crate::resources::ResourceSource,
    ) -> Result<[Option<SceneMovie>; MOVIE_SLOTS], Box<dyn Error>> {
        let mut players: [Option<SceneMovie>; MOVIE_SLOTS] = Default::default();
        for &(slot, body) in records {
            let record = SceneRecord::decode(slot, body, self.scene_clock)?;
            if players
                .iter()
                .flatten()
                .any(|p| p.object_slot == record.object_slot)
            {
                return Err("two scene movies are bound to one object".into());
            }
            let data = source
                .read(&record.name)?
                .ok_or("saved scene movie is missing")?;
            if data.len() > crate::MAX_PAYLOAD {
                return Err("scene movie exceeds research limit".into());
            }
            let (index, base) = open_scene(&data)?;
            if record.frame >= index.max_sequence() {
                return Err("saved scene movie frame exceeds the movie".into());
            }
            let pixels = index.decode(&data, record.frame, base.as_ref())?;
            let object = objects
                .get_mut(&record.object_slot)
                .filter(|o| o.image.as_ref().is_some_and(|i| i.surface))
                .ok_or("scene movie object was not saved with a surface image")?;
            if object.draw.is_none() {
                let mut draw = crate::scene::DrawState::default();
                draw.words[4] = pixels.width();
                draw.words[5] = pixels.height();
                object.draw = Some(draw);
            }
            object.image = Some(index.frame_image(&record.name, pixels));
            players[slot as usize] = Some(SceneMovie {
                load_mode: record.load_mode,
                object_slot: record.object_slot,
                name: record.name,
                frame: record.frame,
                index,
                data,
                base,
                playback: record.playback,
            });
        }
        Ok(players)
    }
    /// Loader 0x140081960 destroys the full-screen player (+4472) and the scene players,
    /// then installs the players its records rebuilt.
    pub(crate) fn restore(&mut self, players: [Option<SceneMovie>; MOVIE_SLOTS]) -> bool {
        let had_player = self.has_player();
        self.player = None;
        self.mgv_player = None;
        self.slots = players;
        had_player
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> SceneRecord {
        let mut playback = MoviePlayback::new(3, 40);
        playback.segments[5] = MovieSegment {
            looping: true,
            start: u32::MAX,
            end: 12,
        };
        playback.current = 5;
        playback.pending = Some(15);
        playback.active = true;
        SceneRecord {
            slot: 2,
            object_slot: 255,
            name: b"synthetic.cmv".to_vec(),
            frame: 9,
            load_mode: SCENE_MODE,
            playback,
        }
    }

    #[test]
    fn scene_records_round_trip_and_restart_the_clock() {
        let record = record();
        let body = record.encode();
        assert_eq!(body.len(), SCENE_RECORD);
        assert_eq!(&body[10..24], b"synthetic.cmv\0");
        // +52 load mode 2 between the zero +48 and +56.
        assert_eq!(body[286..298], [0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
        let mut expected = record.clone();
        expected.playback.last_tick = 77;
        assert_eq!(SceneRecord::decode(2, &body, 77).unwrap(), expected);
        let mut streaming = record.clone();
        streaming.load_mode = 3;
        assert_eq!(streaming.encode()[290..294], 3u32.to_le_bytes());
        assert_eq!(
            SceneRecord::decode(2, &streaming.encode(), 77).unwrap(),
            expected
        );
        let mut idle = record;
        idle.playback.pending = None;
        let body = idle.encode();
        assert_eq!(body[78..82], [0xff; 4]);
        assert_eq!(
            SceneRecord::decode(2, &body, 0).unwrap().playback.pending,
            None
        );
    }

    #[test]
    fn scene_records_reject_unmodeled_fields() {
        let body = record().encode();
        assert!(SceneRecord::decode(1, &body, 0).is_err());
        for (at, value) in [
            (0, 3),   // version
            (6, 256), // object slot
            (74, 16), // current segment
            (78, 16), // pending segment
            (82, 2),  // active flag
            (286, 1), // +48
            (290, 1), // loose-file load mode
            (294, 1), // +56
        ] {
            let mut bad = body.clone();
            bad[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
            if at == 0 {
                bad[2..4].copy_from_slice(&body[2..4]);
            }
            assert!(SceneRecord::decode(2, &bad, 0).is_err(), "offset {at}");
        }
        let mut unterminated = body.clone();
        unterminated[10..74].fill(b'a');
        assert!(SceneRecord::decode(2, &unterminated, 0).is_err());
        assert!(SceneRecord::decode(2, &body[..297], 0).is_err());
    }
}
