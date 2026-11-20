//! Cross-platform CMVS session bootstrap and runtime configuration over [`HostFs`].
use crate::{
    EncodingPolicy, GpuFrameSource, GpuTarget, InputState, compose_cpu_frame_with_capture,
    compose_gpu_frame, resolve_profile_encoding_with_fs, with_gpu_target,
};
use cmvs_core::{
    FormatError, TextEncoding,
    format::{mgv::DecoderFactory, ps3::Script},
    host::{
        filesystem::HostFs, image::DecodeThreads, input::KeyBindings, settings::DEFAULT_FONT_FACE,
    },
    profile::{Game, Registry},
    render::raster::RgbaImage,
    runtime::{
        FrameDriver, Runtime, RuntimeOptions, SaveStorage, SnapshotPolicy, load_start_with_fs,
        utc_save_time,
    },
};
use cmvs_font::font::FontRenderer;
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
};

/// How `key.cfg` bindings are loaded for the selected engine profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyBindingPolicy {
    /// Require `root/key.cfg` when `profile.uses_key_cfg()` is true.
    Required,
    /// Load `root/key.cfg` when present, or fall back to built-in defaults when absent.
    #[default]
    Optional,
    /// Skip reading `key.cfg` and use [`KeyBindings::default`].
    DefaultOnly,
}

/// Ogg/Theora MGV decoder and scene-prefetch configuration.
pub enum MovieConfig {
    /// Synchronous pure-Rust Theora decoding without worker threads (used on `wasm32` and tests).
    Synchronous,
    /// Worker-thread lookahead decoding + scene prefetch via [`cmvs_video::PrefetchTheoraFactory`].
    Prefetch {
        budget_bytes: usize,
        max_frames: usize,
        on_thread_start: fn(),
    },
    /// Custom [`DecoderFactory`] and optional scene-prefetch worker hook.
    Custom {
        factory: Box<dyn DecoderFactory>,
        scene_prefetch: Option<fn()>,
    },
}

#[allow(clippy::derivable_impls)]
impl Default for MovieConfig {
    fn default() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            Self::Synchronous
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self::Prefetch {
                budget_bytes: 64 << 20,
                max_frames: 48,
                on_thread_start: || {},
            }
        }
    }
}

/// Configuration for initializing a [`Runtime`] or [`HostSession`].
pub struct SessionConfig {
    /// Optional explicit profile identifier (`None` auto-identifies the game build).
    pub profile_id: Option<String>,
    /// Policy for resolving [`TextEncoding`] when [`HostSession::open_with_fs`] loads bootstrap.
    pub encoding: EncodingPolicy,
    /// Save-file storage policy.
    pub saves: SaveStorage,
    /// Optional writable user directory for persistent Unicode font-face handles (`font-faces/`).
    pub user_font_dir: Option<PathBuf>,
    /// Gameplay snapshot policy at main entry and scenario points.
    pub snapshot_policy: SnapshotPolicy,
    /// Stop on unreviewed system commands.
    pub block_unreviewed: bool,
    /// Stop on legacy presumed system commands.
    pub block_presumed: bool,
    /// Clear the startup auto-resume flag in memory after loading `system.dat`.
    pub skip_auto_resume: bool,
    /// Decode full-screen CMV video frames.
    pub decode_cmv_frames: bool,
    /// Record glyph layout and reveal timing in `MessageHost`.
    pub record_text: bool,
    /// Use host-supplied frame readback for deferred screen captures.
    pub host_capture: bool,
    /// Install [`utc_save_time`] as the save label clock.
    pub save_clock: bool,
    /// Explicit CRT PRNG seed (`None` leaves the engine's initial seed unchanged).
    pub random_seed: Option<u32>,
    /// MGV movie decoder and scene prefetch policy.
    pub movie: MovieConfig,
    /// Parallel image plane decode worker configuration.
    pub image_decode_threads: Option<DecodeThreads>,
    /// Optional compressed image archive cache budget in bytes.
    pub raw_image_cache: Option<usize>,
    /// Optional decoded RGBA image cache budget in bytes.
    pub decoded_image_cache: Option<usize>,
    /// How `key.cfg` bindings are loaded.
    pub key_bindings: KeyBindingPolicy,
}

impl SessionConfig {
    /// Portable single-threaded defaults suitable for `wasm32` browsers and deterministic tests
    /// (no OS wall clock, no worker threads, synchronous Theora decoder, 64 MiB image cache).
    pub fn single_threaded() -> Self {
        Self {
            profile_id: None,
            encoding: EncodingPolicy::AutoDefaultShiftJis,
            saves: SaveStorage::ReadOnly,
            user_font_dir: None,
            snapshot_policy: SnapshotPolicy::Capture,
            block_unreviewed: false,
            block_presumed: false,
            skip_auto_resume: false,
            decode_cmv_frames: true,
            record_text: true,
            host_capture: true,
            save_clock: false,
            random_seed: None,
            movie: MovieConfig::Synchronous,
            image_decode_threads: None,
            raw_image_cache: None,
            decoded_image_cache: Some(64 << 20),
            key_bindings: KeyBindingPolicy::Optional,
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            Self::single_threaded()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                save_clock: true,
                random_seed: Some(cmvs_core::host::random::host_time_seed()),
                movie: MovieConfig::default(),
                image_decode_threads: Some(DecodeThreads::host()),
                decoded_image_cache: Some(256 << 20),
                ..Self::single_threaded()
            }
        }
    }
}

/// Binds an optional user font directory, installs [`FontRenderer::metrics`] on `runtime`,
/// and resets an unavailable configured font face to [`DEFAULT_FONT_FACE`].
///
/// Returns `Ok(true)` if the configured font face was missing and fell back to the default.
pub fn bind_font(
    runtime: &mut Runtime,
    font: &FontRenderer,
    user_dir: Option<&Path>,
) -> Result<bool, Box<dyn Error + Send + Sync>> {
    if let Some(directory) = user_dir {
        font.set_user_directory(directory)?;
    }
    runtime.set_glyph_metrics(font.metrics());
    if !font.has_face(runtime.settings().font_face()) {
        runtime.set_font_face(DEFAULT_FONT_FACE)?;
        return Ok(true);
    }
    Ok(false)
}

/// Detailed result of [`GameBootstrap::open_runtime_detailed`].
pub struct RuntimeOpenReport {
    pub runtime: Runtime,
    pub bindings: KeyBindings,
    /// Whether an unavailable configured font face was reset to [`DEFAULT_FONT_FACE`].
    pub font_face_reset: bool,
}

/// Identified game build, decoded startup script, and resolved [`TextEncoding`].
///
/// Hosts that derive user save directories from `game.selection.record` or choose system
/// fonts based on `encoding` can call [`Self::load`] first, then finish building a
/// [`HostSession`] or `(Runtime, KeyBindings)`.
pub struct GameBootstrap {
    pub fs: Arc<dyn HostFs>,
    pub root: PathBuf,
    pub game: Game<'static>,
    pub script: Script,
    pub encoding: TextEncoding,
}

impl GameBootstrap {
    /// Identifies the game under `root` via `fs`, loads its startup script, and resolves
    /// its text encoding according to `encoding_policy`.
    pub fn load(
        fs: Arc<dyn HostFs>,
        root: &Path,
        profile_id: Option<&str>,
        encoding_policy: EncodingPolicy,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let game = Registry::builtin().identify_game_with_fs(&*fs, root, profile_id)?;
        let script = load_start_with_fs(&*fs, root, &game.files)?;
        let encoding = resolve_profile_encoding_with_fs(
            &*fs,
            root,
            &game.files,
            &script,
            encoding_policy,
            Some(game.selection.profile),
        )?;
        Ok(Self {
            fs,
            root: root.to_path_buf(),
            game,
            script,
            encoding,
        })
    }

    /// Configures and opens a [`Runtime`] and [`KeyBindings`] from this bootstrap state.
    pub fn open_runtime_detailed(
        self,
        font: Option<&FontRenderer>,
        config: SessionConfig,
    ) -> Result<RuntimeOpenReport, Box<dyn Error + Send + Sync>> {
        let bindings = match config.key_bindings {
            KeyBindingPolicy::Required => KeyBindings::load_for_profile_with_fs(
                &*self.fs,
                &self.root,
                self.game.selection.profile,
            )?,
            KeyBindingPolicy::Optional => KeyBindings::load_optional_for_profile_with_fs(
                &*self.fs,
                &self.root,
                self.game.selection.profile,
            )?,
            KeyBindingPolicy::DefaultOnly => KeyBindings::default(),
        };
        let mut runtime = Runtime::with_options(
            self.script,
            &self.root,
            &self.game,
            RuntimeOptions {
                encoding: self.encoding,
                saves: config.saves,
                snapshot_policy: config.snapshot_policy,
                block_unreviewed: config.block_unreviewed,
                block_presumed: config.block_presumed,
                skip_auto_resume: config.skip_auto_resume,
                decode_cmv_frames: config.decode_cmv_frames,
                fs: Some(Arc::clone(&self.fs)),
                ..Default::default()
            },
        )?;
        runtime.enable_in_game_dialogs(font.map(|font| {
            Box::new(font.clone()) as Box<dyn cmvs_core::render::message::GlyphRasterizer + Send>
        }));
        match config.movie {
            MovieConfig::Synchronous => {
                runtime.set_mgv_factory(Box::new(cmvs_video::TheoraFactory));
            }
            MovieConfig::Prefetch {
                budget_bytes,
                max_frames,
                on_thread_start,
            } => {
                runtime.set_mgv_factory(Box::new(
                    cmvs_video::PrefetchTheoraFactory { on_thread_start }
                        .with_lookahead(budget_bytes, max_frames),
                ));
                runtime.set_scene_prefetch(on_thread_start);
            }
            MovieConfig::Custom {
                factory,
                scene_prefetch,
            } => {
                runtime.set_mgv_factory(factory);
                if let Some(on_start) = scene_prefetch {
                    runtime.set_scene_prefetch(on_start);
                }
            }
        }
        if let Some(threads) = config.image_decode_threads {
            runtime.set_image_decode_threads(threads);
        }
        if let Some(bytes) = config.raw_image_cache {
            runtime.set_raw_image_cache(bytes);
        }
        if let Some(bytes) = config.decoded_image_cache {
            runtime.set_decoded_image_cache(bytes);
        }
        if config.save_clock {
            runtime.set_save_clock(Box::new(utc_save_time));
        }
        if let Some(seed) = config.random_seed {
            runtime.set_random_seed(seed);
        }
        runtime.set_record_text(config.record_text);
        runtime.set_host_capture(config.host_capture);
        let font_face_reset = match font {
            Some(font) => bind_font(&mut runtime, font, config.user_font_dir.as_deref())?,
            None => false,
        };
        Ok(RuntimeOpenReport {
            runtime,
            bindings,
            font_face_reset,
        })
    }

    /// Configures and opens `(Runtime, KeyBindings)` from this bootstrap state.
    pub fn open_runtime(
        self,
        font: Option<&FontRenderer>,
        config: SessionConfig,
    ) -> Result<(Runtime, KeyBindings), Box<dyn Error + Send + Sync>> {
        let report = self.open_runtime_detailed(font, config)?;
        Ok((report.runtime, report.bindings))
    }

    /// Finishes opening a complete [`HostSession`], aligning `font` to [`Self::encoding`].
    pub fn into_session(
        self,
        font: FontRenderer,
        config: SessionConfig,
    ) -> Result<HostSession, Box<dyn Error + Send + Sync>> {
        let encoding = self.encoding;
        let font = font.with_text_encoding(encoding);
        let (runtime, bindings) = self.open_runtime(Some(&font), config)?;
        Ok(HostSession {
            runtime,
            font,
            bindings,
            input: InputState::new(),
            driver: FrameDriver::default(),
            encoding,
        })
    }
}

/// Portable, window-independent CMVS session combining [`Runtime`], [`FontRenderer`],
/// [`KeyBindings`], [`InputState`], and [`FrameDriver`].
pub struct HostSession {
    pub runtime: Runtime,
    pub font: FontRenderer,
    pub bindings: KeyBindings,
    pub input: InputState,
    pub driver: FrameDriver,
    pub encoding: TextEncoding,
}

impl HostSession {
    /// Opens a session from `root` through `fs`, resolving the game and its encoding
    /// according to `config`.
    pub fn open_with_fs(
        fs: Arc<dyn HostFs>,
        root: &Path,
        font: FontRenderer,
        config: SessionConfig,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let bootstrap =
            GameBootstrap::load(fs, root, config.profile_id.as_deref(), config.encoding)?;
        bootstrap.into_session(font, config)
    }

    /// Opens a session using [`crate::default_host_fs`].
    #[cfg(feature = "std-fs")]
    pub fn open(
        root: &Path,
        font: FontRenderer,
        config: SessionConfig,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Self::open_with_fs(crate::default_host_fs(), root, font, config)
    }

    /// Composes the completed VM frame onto a CPU [`RgbaImage`], optionally supplying a
    /// pending engine screen capture.
    pub fn compose_cpu_frame(
        &mut self,
        linear: bool,
        supply_capture: bool,
    ) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
        compose_cpu_frame_with_capture(
            &mut self.runtime,
            Some(&mut self.font),
            linear,
            supply_capture,
        )
    }

    /// Prepares a [`GpuFrameSource`] for rendering on `cmvs-wgpu` or console hardware GPUs.
    pub fn compose_gpu_frame(&mut self, linear: bool) -> Result<GpuFrameSource<'_>, FormatError> {
        compose_gpu_frame(&self.runtime, Some(&mut self.font), linear)
    }

    /// Prepares a [`GpuTarget`] (active movie image or [`cmvs_core::render::scene_render::RenderFrame`])
    /// and invokes `f`.
    pub fn with_gpu_target<T, E>(
        &mut self,
        linear: bool,
        f: impl FnOnce(GpuTarget<'_>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<FormatError>,
    {
        with_gpu_target(&self.runtime, Some(&mut self.font), linear, f)
    }
}
