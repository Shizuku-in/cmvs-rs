//! Cross-platform CMVS host session layer, embedded `Player`, and versioned C ABI.
//!
//! `cmvs-host` provides two layers:
//! - Portable session building blocks ([`HostSession`], [`GameBootstrap`], [`SessionConfig`],
//!   [`InputState`], [`EncodingPolicy`], [`GpuTarget`], [`compose_cpu_frame`], and
//!   [`with_gpu_target`]) shared across Desktop, Android, Web (`wasm32`), Switch, PS Vita,
//!   and embedded players.
//! - With the default `native-audio` feature enabled, a complete single-threaded `Player`
//!   and C interface (`cmvs_host_*`) with OS file storage and `cmvs-audio` device output.
mod backend;
mod comment_input;
mod encoding;
mod error;
#[cfg(feature = "native-audio")]
#[allow(unsafe_code)]
mod ffi;
mod frame;
mod input;
#[cfg(all(test, unix, feature = "native-audio"))]
mod savedata_replay;
mod session;
#[cfg(feature = "std-fs")]
mod storage;

pub use backend::HostBackend;
pub use comment_input::{CommentInput, CommentInputRequest};
pub use encoding::{
    EncodingPolicy, conversion_encoding_with_fs, resolve_encoding_with_fs,
    resolve_profile_encoding_with_fs,
};
pub use error::PlayerError;
pub use frame::{
    GpuFrameSource, GpuTarget, compose_cpu_frame, compose_cpu_frame_with_capture,
    compose_gpu_frame, with_gpu_target,
};
pub use input::InputState;
pub use session::{
    GameBootstrap, HostSession, KeyBindingPolicy, MovieConfig, RuntimeOpenReport, SessionConfig,
    bind_font,
};
#[cfg(all(target_os = "android", feature = "std-fs"))]
pub use storage::AndroidHostFs;
#[cfg(feature = "std-fs")]
pub use storage::{default_host_fs, game_savedata};

/// Result of one `Player::tick` slice. A budget slice does not publish a partial frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// Resume the same VM run in a later slice; no new frame is available.
    BudgetExhausted,
    /// A complete frame is available through `Player::frame`.
    FrameReady,
    /// The game requested shutdown.
    Quit,
    /// The player is paused or suspended by the game's focus policy.
    Inactive,
}

#[cfg(feature = "native-audio")]
mod player {
    use crate::{
        EncodingPolicy, GameBootstrap, HostBackend, HostSession, KeyBindingPolicy, PlayerError,
        SessionConfig, TickOutcome, default_host_fs, game_savedata,
    };
    use cmvs_core::{
        format::system_save::ProfileSystemSave,
        render::raster::RgbaImage,
        runtime::{FrameBudget, FrameOutcome, RuntimeEvent, SaveStorage},
    };
    use cmvs_font::font::FontRenderer;
    use std::{error::Error, path::Path};

    /// An embedded player that configures fonts, audio, movies, and frame composition.
    pub struct Player {
        pub(crate) session: HostSession,
        pub(crate) backend: HostBackend,
        paused: bool,
        serial: u64,
        audio: Option<cmvs_audio::HostAudioControl>,
    }

    impl Player {
        pub fn open(
            root: &Path,
            user: &Path,
            font_path: &Path,
            native_audio: bool,
        ) -> Result<Self, PlayerError> {
            Self::open_with_profile(
                root,
                || {
                    Ok(ProfileSystemSave::open_with_fs(
                        user,
                        root,
                        default_host_fs(),
                    )?)
                },
                font_path,
                native_audio,
            )
            .map_err(PlayerError::Initialization)
        }

        /// Prefer `game/savedata`, importing old external saves without replacing
        /// existing files. Read-only media retains the external per-game profile.
        pub fn open_game_savedata(
            root: &Path,
            legacy: &Path,
            font_path: &Path,
            native_audio: bool,
        ) -> Result<Self, PlayerError> {
            Self::open_with_profile(
                root,
                || Ok(game_savedata(root, legacy)?),
                font_path,
                native_audio,
            )
            .map_err(PlayerError::Initialization)
        }

        fn open_with_profile(
            root: &Path,
            open_profile: impl FnOnce() -> Result<ProfileSystemSave, Box<dyn Error + Send + Sync>>,
            font_path: &Path,
            native_audio: bool,
        ) -> Result<Self, Box<dyn Error + Send + Sync>> {
            #[cfg(target_os = "android")]
            if native_audio && !crate::ffi::android_initialized() {
                return Err(
                    "initialize the CMVS Android application context before native audio".into(),
                );
            }
            let fs = default_host_fs();
            let bootstrap =
                GameBootstrap::load(fs, root, None, EncodingPolicy::AutoDefaultShiftJis)?;
            let font = FontRenderer::read(font_path, 0)?.with_text_encoding(bootstrap.encoding);
            let profile = open_profile()?;
            let user_font_dir = profile.directory().to_path_buf();
            let mut session = bootstrap.into_session(
                font,
                SessionConfig {
                    saves: SaveStorage::Profile(profile),
                    user_font_dir: Some(user_font_dir),
                    key_bindings: KeyBindingPolicy::Optional,
                    ..SessionConfig::default()
                },
            )?;
            let mut audio_control = None;
            if native_audio {
                let audio = cmvs_audio::NativeAudio::new(root)?;
                audio_control = Some(audio.host_control());
                session.runtime.set_audio_backend(Box::new(audio))?;
            }
            let mut backend = HostBackend::new(session.runtime.screen_size());
            backend.encoding = session.runtime.machine().text_encoding();
            let preferences = session.runtime.display_preferences();
            backend.fullscreen = preferences.fullscreen;
            backend.aspect = preferences.aspect_mode;
            backend.scaling = preferences.aspect_scaling;
            backend.render = cmvs_core::host::platform::supported_render_mode(
                &backend,
                preferences.render_mode,
            )?;
            Ok(Self {
                session,
                backend,
                paused: false,
                serial: 0,
                audio: audio_control,
            })
        }

        /// Take the initial text and length limit for a newly requested platform keyboard.
        pub fn take_comment_input(&mut self) -> Option<crate::CommentInputRequest> {
            self.backend.comment_input.take_request()
        }

        /// Return the keyboard's draft, or `None` on keyboard cancellation.
        pub fn submit_comment_input(&mut self, id: u32, text: Option<String>) {
            self.backend.comment_input.submit(id, text);
        }

        pub fn set_key_state(&mut self, code: usize, pressed: bool) -> Result<(), &'static str> {
            self.session.input.set_key(code, pressed)
        }

        pub fn set_pointer_position(&mut self, x: i32, y: i32) {
            self.session.input.set_pointer(x, y);
        }

        pub fn set_paused(&mut self, paused: bool) {
            self.paused = paused;
            if let Some(audio) = &self.audio {
                audio.set_active(!paused);
            }
            if paused {
                self.session.driver.suspend();
                self.session.input.clear(self.session.runtime.input_mut());
            }
        }

        pub fn size(&self) -> [u32; 2] {
            self.backend.size
        }

        pub fn frame_serial(&self) -> u64 {
            self.serial
        }

        /// Compatibility name for [`Self::set_key_state`].
        #[doc(hidden)]
        pub fn key(&mut self, code: usize, pressed: bool) -> Result<(), &'static str> {
            self.set_key_state(code, pressed)
        }

        /// Compatibility name for [`Self::set_pointer_position`].
        #[doc(hidden)]
        pub fn pointer(&mut self, x: i32, y: i32) {
            self.set_pointer_position(x, y);
        }

        /// Compatibility name for [`Self::set_paused`].
        #[doc(hidden)]
        pub fn pause(&mut self, paused: bool) {
            self.set_paused(paused);
        }

        /// Compatibility name for [`Self::frame_serial`].
        #[doc(hidden)]
        pub fn serial(&self) -> u64 {
            self.frame_serial()
        }

        pub fn frame(&self) -> Option<&RgbaImage> {
            self.backend.frame.as_ref()
        }

        pub fn save_preferences(&mut self) -> Result<bool, PlayerError> {
            let display = self.backend.preferences();
            self.session
                .runtime
                .save_preferences(display)
                .map_err(PlayerError::Persistence)
        }

        /// Run a VM budget slice, publishing a frame only when the VM yields.
        ///
        /// `delta_ms` is consumed on a completed frame, capped at 250 ms. When the budget
        /// expires, the driver retains that delta and input until the frame completes;
        /// delta values on continuation calls are ignored.
        /// Unsupported commands and barriers retain their structured [`RuntimeEvent`]
        /// in [`PlayerError::Stopped`].
        pub fn tick(&mut self, delta_ms: u32) -> Result<TickOutcome, PlayerError> {
            if self.paused {
                return Ok(TickOutcome::Inactive);
            }
            self.session
                .runtime
                .poll_audio_output()
                .map_err(PlayerError::Execution)?;
            self.session.driver.latch_delta(delta_ms.min(250));
            let bindings = &self.session.bindings;
            let input_state = &mut self.session.input;
            self.session
                .driver
                .sample_input(self.session.runtime.input_mut(), |input| {
                    input_state.sample(bindings, input);
                });
            match self
                .session
                .driver
                .run_slice(
                    &mut self.session.runtime,
                    &mut FrameBudget::steps(4000),
                    |runtime| runtime.tick(&mut self.backend),
                )
                .map_err(|e| PlayerError::Execution(e.into()))?
            {
                FrameOutcome::Yield => {
                    self.session
                        .driver
                        .finish_frame(&mut self.session.runtime, 0)
                        .map_err(PlayerError::Execution)?;
                    let frame = self
                        .session
                        .compose_cpu_frame(self.backend.render == 1, true)
                        .map_err(PlayerError::Execution)?;
                    self.backend.frame = Some(frame);
                    self.serial += 1;
                    Ok(TickOutcome::FrameReady)
                }
                FrameOutcome::Stopped(RuntimeEvent::Quit) => Ok(TickOutcome::Quit),
                FrameOutcome::Inactive => Ok(TickOutcome::Inactive),
                FrameOutcome::Stopped(event) => Err(PlayerError::Stopped(event)),
                FrameOutcome::BudgetExhausted => Ok(TickOutcome::BudgetExhausted),
            }
        }
    }
}

#[cfg(feature = "native-audio")]
pub use player::Player;

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_core::{
        TextEncoding,
        format::{
            cpz::{ArchiveDirectory, ArchiveFile, Cpz6Index},
            ps3::Script,
        },
        host::{
            filesystem::{File, HostFile, HostFs, Metadata, OpenOptions},
            input::{ActionInput, KeyBindings},
        },
        profile::GameFiles,
        runtime::{FrameBudget, FrameOutcome, RuntimeEvent, SaveStorage},
    };
    use cmvs_font::font::FontRenderer;
    use std::{
        collections::{BTreeMap, BTreeSet},
        ffi::OsString,
        io::{self, Cursor, Read, Seek, SeekFrom, Write},
        path::{Component, Path, PathBuf},
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    struct MockState {
        files: BTreeMap<String, Vec<u8>>,
        dirs: BTreeSet<String>,
        locks: BTreeSet<String>,
    }

    #[derive(Clone, Default)]
    struct MockHostFs {
        state: Arc<Mutex<MockState>>,
    }

    impl MockHostFs {
        fn new() -> Self {
            let fs = Self::default();
            fs.mkdir("/");
            fs
        }

        fn normalize(path: &Path) -> io::Result<String> {
            let mut parts = Vec::new();
            for comp in path.components() {
                match comp {
                    Component::RootDir | Component::CurDir => {}
                    Component::ParentDir => {
                        parts.pop();
                    }
                    Component::Normal(name) => {
                        parts.push(
                            name.to_str()
                                .ok_or_else(|| io::Error::other("non-utf8 mock path"))?
                                .to_owned(),
                        );
                    }
                    Component::Prefix(_) => {
                        return Err(io::Error::other("unsupported path prefix"));
                    }
                }
            }
            Ok(format!("/{}", parts.join("/")))
        }

        fn mkdir(&self, path: impl AsRef<Path>) {
            let norm = Self::normalize(path.as_ref()).unwrap();
            let mut state = self.state.lock().unwrap();
            let mut cur = String::new();
            state.dirs.insert("/".into());
            for seg in norm.split('/').filter(|s| !s.is_empty()) {
                cur.push('/');
                cur.push_str(seg);
                state.dirs.insert(cur.clone());
            }
        }

        fn write_file(&self, path: impl AsRef<Path>, data: impl Into<Vec<u8>>) {
            let path = path.as_ref();
            if let Some(parent) = path.parent() {
                self.mkdir(parent);
            }
            let norm = Self::normalize(path).unwrap();
            self.state.lock().unwrap().files.insert(norm, data.into());
        }

        fn read_bytes(&self, path: impl AsRef<Path>) -> Option<Vec<u8>> {
            let norm = Self::normalize(path.as_ref()).ok()?;
            self.state.lock().unwrap().files.get(&norm).cloned()
        }

        fn remove(&self, path: impl AsRef<Path>) {
            let norm = Self::normalize(path.as_ref()).unwrap();
            self.state.lock().unwrap().files.remove(&norm);
        }
    }

    struct MockFile {
        state: Arc<Mutex<MockState>>,
        path: String,
        pos: u64,
        readable: bool,
        writable: bool,
        locked: Mutex<bool>,
    }

    impl Drop for MockFile {
        fn drop(&mut self) {
            if *self.locked.lock().unwrap() {
                self.state.lock().unwrap().locks.remove(&self.path);
            }
        }
    }

    impl Read for MockFile {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.readable {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            let state = self.state.lock().unwrap();
            let data = state
                .files
                .get(&self.path)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            let start = (self.pos as usize).min(data.len());
            let len = buf.len().min(data.len() - start);
            buf[..len].copy_from_slice(&data[start..start + len]);
            self.pos += len as u64;
            Ok(len)
        }
    }

    impl Write for MockFile {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if !self.writable {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            let mut state = self.state.lock().unwrap();
            let data = state
                .files
                .get_mut(&self.path)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            let end = self.pos as usize + buf.len();
            if data.len() < end {
                data.resize(end, 0);
            }
            data[self.pos as usize..end].copy_from_slice(buf);
            self.pos = end as u64;
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Seek for MockFile {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            let len = self
                .state
                .lock()
                .unwrap()
                .files
                .get(&self.path)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?
                .len() as i128;
            let next = match pos {
                SeekFrom::Start(v) => i128::from(v),
                SeekFrom::Current(v) => i128::from(self.pos) + i128::from(v),
                SeekFrom::End(v) => len + i128::from(v),
            };
            if next < 0 {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            self.pos = next as u64;
            Ok(self.pos)
        }
    }

    impl HostFile for MockFile {
        fn metadata(&self) -> io::Result<Metadata> {
            let state = self.state.lock().unwrap();
            let data = state
                .files
                .get(&self.path)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            Ok(Metadata {
                len: data.len() as u64,
                is_file: true,
                is_dir: false,
                is_symlink: false,
            })
        }

        fn sync_all(&self) -> io::Result<()> {
            Ok(())
        }

        fn try_lock(&self) -> io::Result<()> {
            let mut locked = self.locked.lock().unwrap();
            if *locked {
                return Ok(());
            }
            let mut state = self.state.lock().unwrap();
            if !state.locks.insert(self.path.clone()) {
                return Err(io::Error::from(io::ErrorKind::WouldBlock));
            }
            *locked = true;
            Ok(())
        }
    }

    impl HostFs for MockHostFs {
        fn open(&self, path: &Path, options: &OpenOptions) -> io::Result<File> {
            let norm = Self::normalize(path)?;
            let mut state = self.state.lock().unwrap();
            if state.dirs.contains(&norm) {
                return Err(io::Error::other("is a directory"));
            }
            let exists = state.files.contains_key(&norm);
            if options.create_new && exists {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            if !exists {
                if !(options.create || options.create_new) {
                    return Err(io::Error::from(io::ErrorKind::NotFound));
                }
                if !options.write {
                    return Err(io::Error::from(io::ErrorKind::PermissionDenied));
                }
                state.files.insert(norm.clone(), Vec::new());
            } else if options.truncate {
                if !options.write {
                    return Err(io::Error::from(io::ErrorKind::PermissionDenied));
                }
                state.files.insert(norm.clone(), Vec::new());
            }
            Ok(File::from_host(Box::new(MockFile {
                state: Arc::clone(&self.state),
                path: norm,
                pos: 0,
                readable: options.read,
                writable: options.write,
                locked: Mutex::new(false),
            })))
        }

        fn metadata(&self, path: &Path) -> io::Result<Metadata> {
            let norm = Self::normalize(path)?;
            let state = self.state.lock().unwrap();
            if let Some(data) = state.files.get(&norm) {
                return Ok(Metadata {
                    len: data.len() as u64,
                    is_file: true,
                    is_dir: false,
                    is_symlink: false,
                });
            }
            if state.dirs.contains(&norm) {
                return Ok(Metadata {
                    len: 0,
                    is_file: false,
                    is_dir: true,
                    is_symlink: false,
                });
            }
            Err(io::Error::from(io::ErrorKind::NotFound))
        }

        fn symlink_metadata(&self, path: &Path) -> io::Result<Metadata> {
            self.metadata(path)
        }

        fn try_exists(&self, path: &Path) -> io::Result<bool> {
            let norm = Self::normalize(path)?;
            let state = self.state.lock().unwrap();
            Ok(state.files.contains_key(&norm) || state.dirs.contains(&norm))
        }

        fn is_file(&self, path: &Path) -> bool {
            Self::normalize(path)
                .ok()
                .is_some_and(|n| self.state.lock().unwrap().files.contains_key(&n))
        }

        fn is_dir(&self, path: &Path) -> bool {
            Self::normalize(path)
                .ok()
                .is_some_and(|n| self.state.lock().unwrap().dirs.contains(&n))
        }

        fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
            let norm = Self::normalize(path)?;
            let state = self.state.lock().unwrap();
            if state.files.contains_key(&norm) || state.dirs.contains(&norm) {
                Ok(PathBuf::from(norm))
            } else {
                Err(io::Error::from(io::ErrorKind::NotFound))
            }
        }

        fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
            Ok(PathBuf::from(Self::normalize(path)?))
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.mkdir(path);
            Ok(())
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            let norm = Self::normalize(path)?;
            self.state
                .lock()
                .unwrap()
                .files
                .remove(&norm)
                .map(|_| ())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            let from = Self::normalize(from)?;
            let to = Self::normalize(to)?;
            let mut state = self.state.lock().unwrap();
            let bytes = state
                .files
                .remove(&from)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            state.files.insert(to, bytes);
            Ok(())
        }

        fn read_dir_names(&self, dir: &Path) -> io::Result<Option<Vec<OsString>>> {
            let prefix = Self::normalize(dir)?;
            let state = self.state.lock().unwrap();
            if !state.dirs.contains(&prefix) {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            }
            let slash = if prefix == "/" {
                "/".to_owned()
            } else {
                format!("{prefix}/")
            };
            let mut names = BTreeSet::new();
            for key in state.files.keys().chain(state.dirs.iter()) {
                if let Some(rest) = key.strip_prefix(&slash)
                    && !rest.is_empty()
                    && !rest.contains('/')
                {
                    names.insert(OsString::from(rest));
                }
            }
            Ok(Some(names.into_iter().collect()))
        }
    }

    /// Minimal self-contained TrueType `sfnt` mapping `'A'` (`U+0041`) and `'中'` (`U+4E2D`)
    /// to a rectangular outline glyph so unit tests run completely offline without host fonts.
    fn minimal_ttf() -> Vec<u8> {
        let mut cmap = Vec::new();
        for v in [0u16, 1, 3, 1] {
            cmap.extend(v.to_be_bytes());
        }
        cmap.extend(12u32.to_be_bytes());
        let delta_a = (1i32 - 0x0041) as u16;
        let delta_cjk = (1i32 - 0x4e2d) as u16;
        for v in [
            4u16, 40, 0, 6, 4, 1, 2, // format 4 header (3 segments)
            0x0041, 0x4e2d, 0xffff, // endCode
            0,      // reservedPad
            0x0041, 0x4e2d, 0xffff, // startCode
            delta_a, delta_cjk, 1, // idDelta
            0, 0, 0, // idRangeOffset
        ] {
            cmap.extend(v.to_be_bytes());
        }

        let mut glyf = Vec::new();
        for v in [1i16, 0, 0, 500, 700] {
            glyf.extend(v.to_be_bytes());
        }
        glyf.extend(3u16.to_be_bytes());
        glyf.extend(0u16.to_be_bytes());
        glyf.extend([0x01u8; 4]);
        for v in [0i16, 500, 0, -500, 0, 0, 700, -700] {
            glyf.extend(v.to_be_bytes());
        }

        let mut head = vec![0u8; 54];
        head[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5f0f_3cf5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[38..40].copy_from_slice(&1000i16.to_be_bytes());
        head[40..42].copy_from_slice(&1000i16.to_be_bytes());
        head[46..48].copy_from_slice(&8u16.to_be_bytes());
        head[48..50].copy_from_slice(&2i16.to_be_bytes());

        let mut hhea = vec![0u8; 36];
        hhea[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[10..12].copy_from_slice(&1000u16.to_be_bytes());
        hhea[34..36].copy_from_slice(&2u16.to_be_bytes());

        let mut hmtx = Vec::new();
        for (adv, lsb) in [(500u16, 0i16), (600u16, 0i16)] {
            hmtx.extend(adv.to_be_bytes());
            hmtx.extend(lsb.to_be_bytes());
        }

        let mut loca = Vec::new();
        for off in [0u16, 0, (glyf.len() / 2) as u16] {
            loca.extend(off.to_be_bytes());
        }

        let mut maxp = vec![0u8; 32];
        maxp[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        maxp[4..6].copy_from_slice(&2u16.to_be_bytes());
        maxp[6..8].copy_from_slice(&4u16.to_be_bytes());
        maxp[8..10].copy_from_slice(&1u16.to_be_bytes());

        let utf16: Vec<u8> = "MockGothic"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        let mut name = Vec::new();
        for v in [0u16, 1, 18, 3, 1, 0x0409, 1, utf16.len() as u16, 0] {
            name.extend(v.to_be_bytes());
        }
        name.extend(utf16);

        let tables: [(&[u8; 4], Vec<u8>); 8] = [
            (b"cmap", cmap),
            (b"glyf", glyf),
            (b"head", head),
            (b"hhea", hhea),
            (b"hmtx", hmtx),
            (b"loca", loca),
            (b"maxp", maxp),
            (b"name", name),
        ];

        let mut sfnt = Vec::new();
        sfnt.extend(0x0001_0000u32.to_be_bytes());
        sfnt.extend((tables.len() as u16).to_be_bytes());
        sfnt.extend(128u16.to_be_bytes());
        sfnt.extend(3u16.to_be_bytes());
        sfnt.extend(0u16.to_be_bytes());

        let mut offset = 12 + 16 * tables.len();
        let mut payload = Vec::new();
        for (tag, data) in &tables {
            sfnt.extend_from_slice(*tag);
            sfnt.extend(0u32.to_be_bytes());
            sfnt.extend((offset as u32).to_be_bytes());
            sfnt.extend((data.len() as u32).to_be_bytes());
            payload.extend_from_slice(data);
            let pad = (4 - data.len() % 4) % 4;
            payload.resize(payload.len() + pad, 0);
            offset += data.len() + pad;
        }
        sfnt.extend(payload);
        sfnt
    }

    fn push_command(words: &mut Vec<u8>, op: u16, args: &[u32]) {
        for &arg in args.iter().rev() {
            words.extend_from_slice(&[0x40, 0x04, 4, 0]);
            words.extend_from_slice(&arg.to_le_bytes());
        }
        words.extend_from_slice(&op.to_le_bytes());
    }

    fn encode_ps2a(code: &[u8], text: &[u8]) -> Vec<u8> {
        let mut raw = b"PS2A".to_vec();
        for word in [
            48u32,
            0,
            0,
            0,
            code.len() as u32,
            0,
            text.len() as u32,
            0,
            0,
            (code.len() + text.len()) as u32,
            0,
        ] {
            raw.extend_from_slice(&word.to_le_bytes());
        }
        raw.extend(
            code.iter()
                .chain(text)
                .map(|b| b.rotate_left(1).wrapping_add(124)),
        );
        raw
    }

    fn mount_synthetic_game(fs: &MockHostFs, message_bytes: &[u8]) {
        let mut text = vec![0u8];
        let msg_offset = text.len() as u32;
        text.extend_from_slice(b"\\wc");
        text.extend_from_slice(message_bytes);
        text.push(0);

        let mut code = Vec::new();
        push_command(&mut code, 0x215d, &[0, 7, 0, 0, 64, 48, 64, 48]);
        push_command(&mut code, 0x2152, &[0, 0xffff_ffff, msg_offset]);
        push_command(&mut code, 0x201d, &[0x1234_5678]);
        push_command(&mut code, 0x2128, &[]);
        push_command(&mut code, 0x2092, &[2400, 1, 96]);
        push_command(&mut code, 0x2129, &[]);
        push_command(&mut code, 0x2000, &[]);
        push_command(&mut code, 0x2001, &[]);

        let start = encode_ps2a(&code, &text);
        let mut cpz = Cursor::new(Vec::new());
        Cpz6Index::write(
            &mut cpz,
            &[ArchiveDirectory {
                name: Vec::new(),
                files: vec![ArchiveFile {
                    name: b"menu.ps3".to_vec(),
                    data: encode_ps2a(&[0x00, 0x20], b"ascii\0"),
                }],
            }],
        )
        .unwrap();

        fs.write_file(
            "/game/cmvs.cfg",
            b"[WINDOW_RESOLVING]\nBASE=64,48\n".as_slice(),
        );
        fs.write_file("/game/data/pack/start.ps3", start);
        fs.write_file("/game/data/pack/script.cpz", cpz.into_inner());
        fs.mkdir("/user");
    }

    #[cfg(feature = "native-audio")]
    #[test]
    fn failed_open_preserves_the_initialization_error_source() {
        use std::error::Error as _;
        let missing =
            std::env::temp_dir().join(format!("cmvs-missing-player-{}", std::process::id()));
        let error = Player::open(&missing, &missing, &missing, false)
            .err()
            .unwrap();
        assert!(matches!(error, PlayerError::Initialization(_)));
        assert!(error.source().is_some());
    }

    #[test]
    fn input_state_latches_pulses_tracks_holds_and_clears_without_release() {
        let mut host = InputState::with_pointer([12, 34]);
        assert_eq!(host.pointer(), [12, 34]);
        let bindings = KeyBindings::parse(
            "[KEY_FUNCTION_01]\nKEY_DATA_01=1\n[KEY_FUNCTION_09]\nKEY_DATA_01=24\n",
        )
        .unwrap();
        let mut input = ActionInput::default();

        // Sub-frame click (down + up before sample) survives for one frame, then releases.
        host.set_key(1, true).unwrap();
        host.set_key(1, false).unwrap();
        host.pulse(4).unwrap();
        host.sample(&bindings, &mut input);
        assert!(input.mouse(0).iter().any(|&x| x));
        assert!(input.mouse(2)[2]); // wheel up pulse
        assert_eq!(input.pointer(), [12, 34]);

        // Next frame observes release of click and wheel pulse.
        host.sample(&bindings, &mut input);
        assert!(input.released(0));
        assert_eq!(input.mouse(2), [true, false, false]);

        // Third frame is completely idle.
        host.sample(&bindings, &mut input);
        assert_eq!(input.mouse(0), [false; 3]);
        assert_eq!(input.mouse(2), [false; 3]);
        assert!(!input.released(0));

        // Direct held codes + focus-loss clear suppresses release edges.
        host.set_held_codes(&[24, 200]);
        host.set_pointer(50, 60);
        host.sample(&bindings, &mut input);
        assert!(input.held(8));
        assert_eq!(input.pointer(), [50, 60]);

        host.clear(&mut input);
        host.sample(&bindings, &mut input);
        assert!(!input.held(8));
        assert!(!input.released(8));

        assert!(host.set_key(128, true).is_err());
        assert!(host.set_held(128, true).is_err());
        assert!(host.pulse(128).is_err());
    }

    #[test]
    fn conversion_json_auto_detects_utf8_and_gbk_with_encoding_txt_precedence() {
        let fs = MockHostFs::new();
        // Single character '中' in UTF-8 is < 8 non-ASCII chars, so script heuristic alone
        // is `Uncertain`; `conversion.json` resolves it deterministically!
        mount_synthetic_game(&fs, "中".as_bytes());
        let files = GameFiles {
            start_script: "data/pack/start.ps3".into(),
            script_archive: "data/pack/script.cpz".into(),
        };
        let startup = Script::decode(&fs.read_bytes("/game/data/pack/start.ps3").unwrap()).unwrap();
        let root = Path::new("/game");

        // Without conversion.json, AutoStrict rejects uncertain short script text.
        assert!(
            resolve_encoding_with_fs(
                &fs,
                root,
                &files,
                &startup,
                EncodingPolicy::AutoStrict("uncertain")
            )
            .is_err()
        );

        // Realistic fan_patch conversion.json with source_encoding + nested structures + UTF-8 BOM.
        let report = "\u{feff}{\n  \"converter\": \"asukimi-ck-gal-v1\",\n  \"source_encoding\": \"gbk\",\n  \"nested\": {\"encoding\": \"shift-jis\", \"repairs\": [1, -2.5e3, true, false, null, \"quote\\\"\\\\\\u266a\"]},\n  \"encoding\": \"utf-8\"\n}\n";
        fs.write_file("/game/conversion.json", report.as_bytes());
        assert_eq!(
            resolve_encoding_with_fs(
                &fs,
                root,
                &files,
                &startup,
                EncodingPolicy::AutoStrict("uncertain")
            )
            .unwrap(),
            TextEncoding::Utf8
        );

        // `"utf8"` alias and `"gbk"` both resolve cleanly.
        fs.write_file(
            "/game/conversion.json",
            br#"{"encoding":"utf8"}"#.as_slice(),
        );
        assert_eq!(
            conversion_encoding_with_fs(&fs, root).unwrap(),
            Some(TextEncoding::Utf8)
        );
        fs.write_file("/game/conversion.json", br#"{"encoding":"gbk"}"#.as_slice());
        assert_eq!(
            resolve_encoding_with_fs(
                &fs,
                root,
                &files,
                &startup,
                EncodingPolicy::AutoDefaultShiftJis
            )
            .unwrap(),
            TextEncoding::Gbk
        );

        // `encoding.txt` overrides `conversion.json`; `Explicit` overrides both.
        fs.write_file("/game/encoding.txt", b"shift-jis\n".as_slice());
        assert_eq!(
            resolve_encoding_with_fs(
                &fs,
                root,
                &files,
                &startup,
                EncodingPolicy::AutoDefaultShiftJis
            )
            .unwrap(),
            TextEncoding::ShiftJis
        );
        assert_eq!(
            resolve_encoding_with_fs(
                &fs,
                root,
                &files,
                &startup,
                EncodingPolicy::Explicit(TextEncoding::Utf8)
            )
            .unwrap(),
            TextEncoding::Utf8
        );

        // Malformed JSON or unsupported encoding in conversion.json returns an error.
        fs.remove("/game/encoding.txt");
        fs.write_file(
            "/game/conversion.json",
            br#"{"encoding":"latin1"}"#.as_slice(),
        );
        assert!(conversion_encoding_with_fs(&fs, root).is_err());
        fs.write_file(
            "/game/conversion.json",
            br#"{"encoding":"utf-8""#.as_slice(),
        );
        assert!(conversion_encoding_with_fs(&fs, root).is_err());
    }

    #[test]
    fn host_session_opens_on_mock_fs_and_composes_matching_cpu_and_gpu_frames() {
        let fs = MockHostFs::new();
        mount_synthetic_game(&fs, "中A".as_bytes());
        fs.write_file(
            "/game/conversion.json",
            br#"{"game":"synthetic","encoding":"utf-8"}"#.as_slice(),
        );

        let font = FontRenderer::from_bytes(minimal_ttf(), 0).unwrap();
        let mut session = HostSession::open_with_fs(
            Arc::new(fs.clone()),
            Path::new("/game"),
            font,
            SessionConfig {
                profile_id: Some("cmvs-20160201".into()),
                saves: SaveStorage::UserDirectory("/user".into()),
                random_seed: Some(42),
                ..SessionConfig::single_threaded()
            },
        )
        .unwrap();
        assert_eq!(session.encoding, TextEncoding::Utf8);
        assert_eq!(session.runtime.screen_size(), [64, 48]);

        let mut backend = HostBackend::new(session.runtime.screen_size());
        session
            .runtime
            .set_save_clock(Box::new(|| [2026, 11, 18, 12, 0, 0]));

        // Run through text yield + system-save write frame.
        for _ in 0..2 {
            let bindings = &session.bindings;
            let input_state = &mut session.input;
            session
                .driver
                .sample_input(session.runtime.input_mut(), |input| {
                    input_state.sample(bindings, input);
                });
            let outcome = session
                .driver
                .run_slice(&mut session.runtime, &mut FrameBudget::steps(500), |rt| {
                    rt.tick(&mut backend)
                })
                .unwrap();
            assert_eq!(outcome, FrameOutcome::Yield);
            session
                .driver
                .finish_frame(&mut session.runtime, 16)
                .unwrap();
        }

        let cpu_frame = session.compose_cpu_frame(false, true).unwrap();
        assert_eq!([cpu_frame.width(), cpu_frame.height()], [64, 48]);
        assert!(
            cpu_frame
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|px| px[..3] != [0, 0, 0]),
            "expected rendered glyph ink on canvas"
        );

        let gpu_rendered = session
            .with_gpu_target(false, |target| match target {
                GpuTarget::Movie { .. } => panic!("expected scene RenderFrame, not movie"),
                GpuTarget::Scene(render_frame) => render_frame.render_cpu(),
            })
            .unwrap();
        assert_eq!(cpu_frame, gpu_rendered);

        // Verify system save and preferences persisted into the mock HostFs.
        assert_eq!(&fs.read_bytes("/user/system.dat").unwrap()[..4], b"CSS1");
        assert!(
            session
                .runtime
                .save_preferences(backend.preferences())
                .unwrap()
        );
        assert!(fs.read_bytes("/user/cmvs.cfg").is_some());

        // Final tick reaches script quit (`0x2001`).
        let outcome = session
            .driver
            .run_slice(&mut session.runtime, &mut FrameBudget::steps(500), |rt| {
                rt.tick(&mut backend)
            })
            .unwrap();
        assert_eq!(outcome, FrameOutcome::Stopped(RuntimeEvent::Quit));
    }

    #[test]
    fn fontless_gpu_frame_includes_visible_cvns_window() {
        use cmvs_core::format::pb3::{EncodeDepth, encode_rgba};
        let fs = MockHostFs::new();
        mount_synthetic_game(&fs, b"");
        let pixel = [23, 42, 61, 255];
        fs.write_file(
            "/game/img/window.pb3",
            encode_rgba(&pixel, 1, 1, EncodeDepth::Rgba32).unwrap(),
        );
        let mut code = Vec::new();
        push_command(&mut code, 0x2021, &[0, 0]); // register image directory
        push_command(&mut code, 0x2110, &[0, 0, 0, 0, 1, 1]); // create window
        push_command(&mut code, 0x2130, &[0, 0, 5, 1]); // load window.pb3
        push_command(&mut code, 0x2138, &[0, 0]); // create local sprite
        push_command(&mut code, 0x213a, &[0, 0, 0, 0, 0, 1, 1]);
        push_command(&mut code, 0x2111, &[0, 1]); // show window
        push_command(&mut code, 0x2000, &[]);
        fs.write_file(
            "/game/data/pack/start.ps3",
            encode_ps2a(&code, b"img\\\0window.pb3\0"),
        );
        let (mut runtime, _) = GameBootstrap::load(
            Arc::new(fs),
            Path::new("/game"),
            Some("cvns-20090203"),
            EncodingPolicy::Explicit(TextEncoding::ShiftJis),
        )
        .unwrap()
        .open_runtime(None, SessionConfig::single_threaded())
        .unwrap();
        let mut backend = HostBackend::new(runtime.screen_size());
        let outcome = cmvs_core::runtime::FrameDriver::default()
            .run_slice(&mut runtime, &mut FrameBudget::steps(100), |rt| {
                rt.tick(&mut backend)
            })
            .unwrap();
        assert_eq!(outcome, FrameOutcome::Yield);
        let cpu = compose_cpu_frame(&runtime, None, false).unwrap();
        assert_eq!(&cpu.pixels()[..4], &pixel);
        let gpu = with_gpu_target(&runtime, None, false, |target| match target {
            GpuTarget::Scene(frame) => frame.render_cpu(),
            GpuTarget::Movie { .. } => panic!("expected scene"),
        })
        .unwrap();
        assert_eq!(gpu, cpu);
    }

    #[test]
    fn fontless_composition_and_deferred_capture_work_through_shared_helpers() {
        let fs = MockHostFs::new();
        // Deferred capture (0x2304), yield, assign captured frame to slot 3 (0x203c), quit.
        let mut code = Vec::new();
        push_command(&mut code, 0x2020, &[3]);
        push_command(&mut code, 0x2304, &[]);
        push_command(&mut code, 0x2000, &[]);
        push_command(&mut code, 0x203c, &[3, 0xffff_ffff]);
        push_command(&mut code, 0x2001, &[]);
        let mut cpz = Cursor::new(Vec::new());
        Cpz6Index::write(
            &mut cpz,
            &[ArchiveDirectory {
                name: Vec::new(),
                files: Vec::new(),
            }],
        )
        .unwrap();
        fs.write_file(
            "/game/cmvs.cfg",
            b"[WINDOW_RESOLVING]\nBASE=32,24\n".as_slice(),
        );
        fs.write_file("/game/data/pack/start.ps3", encode_ps2a(&code, b"\0"));
        fs.write_file("/game/data/pack/script.cpz", cpz.into_inner());

        let bootstrap = GameBootstrap::load(
            Arc::new(fs),
            Path::new("/game"),
            Some("cmvs-20160201"),
            EncodingPolicy::AutoDefaultShiftJis,
        )
        .unwrap();
        let (mut runtime, _) = bootstrap
            .open_runtime(
                None,
                SessionConfig {
                    record_text: false,
                    ..SessionConfig::single_threaded()
                },
            )
            .unwrap();
        let mut backend = HostBackend::new(runtime.screen_size());
        let mut driver = cmvs_core::runtime::FrameDriver::default();
        driver.sample_input(runtime.input_mut(), |_| {});
        assert_eq!(
            driver
                .run_slice(&mut runtime, &mut FrameBudget::steps(100), |rt| {
                    rt.tick(&mut backend)
                })
                .unwrap(),
            FrameOutcome::Yield
        );
        driver.finish_frame(&mut runtime, 16).unwrap();
        assert!(runtime.capture_due());

        let gpu_cpu = with_gpu_target(&runtime, None, false, |target| match target {
            GpuTarget::Movie { .. } => panic!("expected scene"),
            GpuTarget::Scene(frame) => frame.render_cpu(),
        })
        .unwrap();
        let cpu = compose_cpu_frame_with_capture(&mut runtime, None, false, true).unwrap();
        assert!(!runtime.capture_due());
        assert_eq!(cpu, gpu_cpu);
        driver.sample_input(runtime.input_mut(), |_| {});
        assert_eq!(
            driver
                .run_slice(&mut runtime, &mut FrameBudget::steps(100), |rt| {
                    rt.tick(&mut backend)
                })
                .unwrap(),
            FrameOutcome::Stopped(RuntimeEvent::Quit)
        );
        assert_eq!(
            runtime
                .scene()
                .object(3)
                .unwrap()
                .image
                .as_ref()
                .unwrap()
                .pixels,
            cpu
        );
        let movie_src = GpuFrameSource::Movie {
            image: &cpu,
            size: [64, 48],
            linear: true,
        };
        assert!(matches!(
            movie_src.prepare().unwrap(),
            GpuTarget::Movie {
                size: [64, 48],
                linear: true,
                ..
            }
        ));
    }

    #[test]
    fn encoding_txt_and_conversion_json_strip_bom_and_check_size_before_utf8() {
        let fs = MockHostFs::new();
        let root = Path::new("/game");

        fs.write_file(root.join("encoding.txt"), "\u{feff}utf-8\r\n");
        assert_eq!(
            cmvs_font::text_codec::encoding_txt_with_fs(&fs, root).unwrap(),
            Some(TextEncoding::Utf8)
        );

        let mut split_encoding = vec![b' '; 63];
        split_encoding.extend_from_slice("あ".as_bytes());
        assert_eq!(split_encoding.len(), 66);
        fs.write_file(root.join("encoding.txt"), split_encoding);
        let encoding_err = cmvs_font::text_codec::encoding_txt_with_fs(&fs, root)
            .unwrap_err()
            .to_string();
        assert!(encoding_err.contains("64 bytes"), "{encoding_err}");

        fs.write_file(
            root.join("conversion.json"),
            "\u{feff}{\"encoding\":\"gbk\"}",
        );
        assert_eq!(
            conversion_encoding_with_fs(&fs, root).unwrap(),
            Some(TextEncoding::Gbk)
        );

        let mut split_json = vec![b' '; 256 * 1024 - 1];
        split_json.extend_from_slice("あ".as_bytes());
        assert_eq!(split_json.len(), 256 * 1024 + 2);
        fs.write_file(root.join("conversion.json"), split_json);
        let json_err = conversion_encoding_with_fs(&fs, root)
            .unwrap_err()
            .to_string();
        assert!(json_err.contains("256 KiB"), "{json_err}");
    }
}
