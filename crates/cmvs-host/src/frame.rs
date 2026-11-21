//! Dual CPU [`RgbaImage`] and GPU [`RenderFrame`] frame composition helpers.
use cmvs_core::{
    FormatError,
    host::platform::RenderMode,
    render::{
        raster::{RgbaImage, ScaleFilter, movie_canvas},
        scene_render::RenderFrame,
    },
    runtime::Runtime,
};
use cmvs_font::{composition::SceneComposition, font::FontRenderer};
use std::error::Error;

/// Ready-to-render GPU frame target: either an active full-screen movie frame or a
/// prepared layered [`RenderFrame`] (including rasterized glyphs and message windows).
pub enum GpuTarget<'a> {
    /// Active full-screen movie image and its logical-canvas presentation mode.
    Movie {
        image: &'a RgbaImage,
        size: [u32; 2],
        render_mode: RenderMode,
    },
    /// Prepared layered scene queue ready for `cmvs-wgpu` or `cmvs-console::gpu::Compositor`.
    Scene(RenderFrame<'a>),
}

/// Intermediate owner holding rasterized glyph objects while [`GpuTarget::Scene`] borrows them.
pub enum GpuFrameSource<'a> {
    /// Active full-screen movie frame.
    Movie {
        image: &'a RgbaImage,
        size: [u32; 2],
        render_mode: RenderMode,
    },
    /// Scene + camera + message windows + rasterized glyphs.
    WithFont(SceneComposition<'a>),
    /// Scene + camera without glyph rendering (e.g. `--record-text` research mode).
    WithoutFont(&'a Runtime),
}

impl GpuFrameSource<'_> {
    /// Builds the [`GpuTarget`] borrowing from this source's owned glyph objects and runtime state.
    pub fn prepare(&self) -> Result<GpuTarget<'_>, FormatError> {
        match self {
            Self::Movie {
                image,
                size,
                render_mode,
            } => Ok(GpuTarget::Movie {
                image,
                size: *size,
                render_mode: *render_mode,
            }),
            Self::WithFont(composition) => Ok(GpuTarget::Scene(composition.prepare()?)),
            Self::WithoutFont(runtime) => Ok(GpuTarget::Scene(runtime.prepare_scene_frame()?)),
        }
    }
}

/// Composes the completed VM frame on the CPU: active full-screen movie if playing,
/// otherwise the software scene with optional [`FontRenderer`] glyphs and message windows.
pub fn compose_cpu_frame(
    runtime: &Runtime,
    font: Option<&mut FontRenderer>,
    filter: impl Into<ScaleFilter>,
) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
    if let Some(image) = runtime.movie_frame() {
        return Ok(movie_canvas(image, runtime.screen_size(), filter)?);
    }
    if let Some(font) = font {
        return Ok(font.render(runtime.scene(), runtime.camera(), runtime.messages())?);
    }
    Ok(runtime.render_frame()?)
}

/// Composes the completed VM frame on the CPU and optionally satisfies [`Runtime::capture_due`].
pub fn compose_cpu_frame_with_capture(
    runtime: &mut Runtime,
    font: Option<&mut FontRenderer>,
    filter: impl Into<ScaleFilter>,
    supply_capture: bool,
) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
    let frame = compose_cpu_frame(runtime, font, filter)?;
    if supply_capture && runtime.capture_due() {
        runtime.supply_capture(frame.clone())?;
    }
    Ok(frame)
}

/// Prepares the intermediate [`GpuFrameSource`] for GPU presentation without duplicating
/// movie vs glyph-scene dispatch.
pub fn compose_gpu_frame<'a>(
    runtime: &'a Runtime,
    font: Option<&mut FontRenderer>,
    render_mode: impl Into<RenderMode>,
) -> Result<GpuFrameSource<'a>, FormatError> {
    if let Some(image) = runtime.movie_frame() {
        return Ok(GpuFrameSource::Movie {
            image,
            size: runtime.screen_size(),
            render_mode: render_mode.into(),
        });
    }
    if let Some(font) = font {
        return Ok(GpuFrameSource::WithFont(font.compose(
            runtime.scene(),
            runtime.camera(),
            runtime.messages(),
        )?));
    }
    Ok(GpuFrameSource::WithoutFont(runtime))
}

/// Prepares a [`GpuTarget`] (either active movie or [`RenderFrame`]) and passes it to `f`.
pub fn with_gpu_target<T, E>(
    runtime: &Runtime,
    font: Option<&mut FontRenderer>,
    render_mode: impl Into<RenderMode>,
    f: impl FnOnce(GpuTarget<'_>) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<FormatError>,
{
    let source = compose_gpu_frame(runtime, font, render_mode)?;
    let target = source.prepare()?;
    f(target)
}
