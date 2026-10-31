//! Explicit platform requests. The recording backend is a research model, not a window.
use crate::{
    FormatError, MAX_PAYLOAD, cursor::CursorBitmap, evaluate::error, machine::Machine,
    vm::SystemResult,
};
use std::{
    error::Error,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const MAX_PLATFORM_ASSET: usize = 4 * 1024 * 1024;
pub trait PlatformAssets {
    fn read(&mut self, name: &[u8]) -> Result<Vec<u8>, Box<dyn Error>>;
}
pub struct DirectoryAssets {
    root: PathBuf,
}
impl DirectoryAssets {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err("platform asset root is not a directory".into());
        }
        Ok(Self { root })
    }
}
/// A bounded ASCII game-relative path, split on `/` or `\`, that must resolve inside the
/// canonical `root`. A missing file surfaces as the canonicalize `io::Error`.
pub fn resolve_under(root: &Path, name: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
    if name.is_empty()
        || name.len() >= 2048
        || !name.is_ascii()
        || name.contains(&0)
        || name.contains(&b':')
    {
        return Err("game path must be a bounded ASCII relative path".into());
    }
    let mut path = root.to_path_buf();
    for part in name.split(|b| matches!(b, b'/' | b'\\')) {
        if part.is_empty() || part == b"." || part == b".." {
            return Err("invalid game path component".into());
        }
        path.push(std::str::from_utf8(part)?);
    }
    let path = path.canonicalize()?;
    if !path.starts_with(root) {
        return Err("game path resolves outside root".into());
    }
    Ok(path)
}

impl PlatformAssets for DirectoryAssets {
    fn read(&mut self, name: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
        let file = File::open(resolve_under(&self.root, name)?)?;
        let mut bytes = Vec::new();
        file.take(MAX_PLATFORM_ASSET as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_PLATFORM_ASSET {
            return Err("platform asset exceeds research limit".into());
        }
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformRequest {
    Fullscreen(bool),
    AspectMode(u32),
    /// Active display's scaling enable (+172). In fullscreen, refresh presentation
    /// resources before acknowledging; keep the saved windowed client size intact.
    AspectScaling(bool),
    /// Device +112 presentation filter: 0 point, 1 linear. Shader modes 2/3 are not modeled.
    RenderMode(u32),
    /// Windowed client size. In fullscreen, record it for the next windowed mode.
    WindowSize([u32; 2]),
    Title(Vec<u8>),
    Cursor {
        id: u32,
        bitmap: Option<Arc<CursorBitmap>>,
    },
    /// Validated ICO directory and opaque payloads; no raster/icon size selection yet.
    Icon {
        name: Vec<u8>,
        bytes: Arc<Vec<u8>>,
    },
}
pub trait PlatformBackend {
    /// Read the latest completed engine frame in device/script pixels, including
    /// fonts and movies. Do not redraw current VM state or reuse a held 0x2304 capture.
    /// Absence of a completed frame is an error, not a black/empty fabricated image.
    fn read_device_frame(&mut self) -> Result<crate::raster::RgbaImage, Box<dyn Error>> {
        Err("platform backend does not implement device frame readback".into())
    }
    fn window_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        Err("platform backend does not implement window size queries".into())
    }
    fn windowed_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        if self.fullscreen()? {
            return Err("platform backend does not track the future windowed size".into());
        }
        self.window_size()
    }
    fn reset_window_size(&mut self) -> Result<bool, Box<dyn Error>> {
        if !self.resizable()? {
            return Ok(false);
        }
        if self.windowed_size()? == [1280, 720] {
            return Ok(false);
        }
        let fullscreen = self.fullscreen()?;
        self.apply(&PlatformRequest::WindowSize([1280, 720]))?;
        Ok(!fullscreen)
    }
    /// 0x2163 queries the device's current display mode, not the startup preference.
    fn fullscreen(&self) -> Result<bool, Box<dyn Error>> {
        Err("platform backend does not implement display mode queries".into())
    }
    /// Native pixel shader capability; software backends report zero. This does
    /// not grant support for individual VM effects in the core renderer.
    fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error>> {
        Err("platform backend does not implement pixel shader capability queries".into())
    }
    fn render_mode(&self) -> Result<u32, Box<dyn Error>> {
        Err("platform backend does not implement render mode queries".into())
    }
    fn resizable(&self) -> Result<bool, Box<dyn Error>> {
        Err("platform backend does not implement window resizing queries".into())
    }
    /// StretchRect-equivalent point or linear enlargement in the selected backend.
    fn stretch_filter_supported(&self) -> Result<bool, Box<dyn Error>> {
        Err("platform backend does not implement stretch filter queries".into())
    }
    /// D3DCAPS9.StretchRectFilterCaps MAGFLINEAR alone; render mode 1 requires it.
    fn linear_stretch_supported(&self) -> Result<bool, Box<dyn Error>> {
        Err("platform backend does not implement linear stretch queries".into())
    }
    /// 0x213d's modal font dialog, seeded with the global face in the game encoding.
    /// Ok(None) is a cancelled dialog; Ok(Some) the chosen face in the game encoding.
    fn choose_font(&mut self, _current: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        Err("platform backend does not implement the font dialog".into())
    }
    /// 0=unscaled when it fits, 1=contain, 2=cover, 3=stretch (0x14001a360).
    fn aspect_mode(&self) -> Result<u32, Box<dyn Error>> {
        Err("platform backend does not implement aspect mode queries".into())
    }
    fn aspect_scaling(&self) -> Result<bool, Box<dyn Error>> {
        Err("platform backend does not implement aspect scaling queries".into())
    }
    /// Must commit the request on Ok; on Err leave backend state unchanged for retry.
    /// This is not a queued acknowledgement. Native event-loop implementations must honor this contract.
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error>>;
}
/// 0x14001cfd0: a request the device cannot honor leaves render mode 0 (point).
/// Modes 2/3 also need device +124, whose source is not modeled, so a capable
/// backend reports an error instead of guessing.
pub fn supported_render_mode(
    backend: &(impl PlatformBackend + ?Sized),
    requested: u32,
) -> Result<u32, Box<dyn Error>> {
    Ok(match requested {
        1 if backend.linear_stretch_supported()? => 1,
        2 | 3 if u32::from(backend.pixel_shader_major()?) >= requested => {
            return Err("shader presentation filters need unmodeled device flag +124".into());
        }
        _ => 0,
    })
}
#[derive(Default)]
pub struct RecordingBackend {
    requests: Vec<PlatformRequest>,
    bytes: usize,
    fullscreen: bool,
    aspect_mode: Option<u32>,
    aspect_scaling: Option<bool>,
    device_frame: Option<crate::raster::RgbaImage>,
    render_mode: u32,
    window_size: Option<[u32; 2]>,
    font_answers: std::collections::VecDeque<Option<Vec<u8>>>,
    font_dialogs: Vec<Vec<u8>>,
}
impl RecordingBackend {
    /// Explicit software device draw boundary. Frame producers must publish the
    /// complete engine canvas after drawing; VM commands do not update it.
    pub fn present_frame(&mut self, frame: crate::raster::RgbaImage) {
        self.device_frame = Some(frame);
    }
    pub fn requests(&self) -> &[PlatformRequest] {
        &self.requests
    }
    /// Scripted result for the next font dialog; without one the dialog is an error.
    pub fn answer_font_dialog(&mut self, answer: Option<Vec<u8>>) {
        self.font_answers.push_back(answer);
    }
    /// The global face each font dialog was opened with.
    pub fn font_dialogs(&self) -> &[Vec<u8>] {
        &self.font_dialogs
    }
}
impl PlatformBackend for RecordingBackend {
    fn read_device_frame(&mut self) -> Result<crate::raster::RgbaImage, Box<dyn Error>> {
        self.device_frame
            .clone()
            .ok_or_else(|| "recording device has no completed frame".into())
    }
    fn windowed_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        self.window_size()
    }
    fn window_size(&self) -> Result<[u32; 2], Box<dyn Error>> {
        Ok(self.window_size.unwrap_or([960, 540]))
    }
    fn aspect_mode(&self) -> Result<u32, Box<dyn Error>> {
        Ok(self.aspect_mode.unwrap_or(1))
    }
    fn aspect_scaling(&self) -> Result<bool, Box<dyn Error>> {
        Ok(self.aspect_scaling.unwrap_or(true))
    }
    fn stretch_filter_supported(&self) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
    fn linear_stretch_supported(&self) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
    fn resizable(&self) -> Result<bool, Box<dyn Error>> {
        Ok(true)
    }
    fn render_mode(&self) -> Result<u32, Box<dyn Error>> {
        Ok(self.render_mode)
    }
    fn choose_font(&mut self, current: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        if self.font_dialogs.len() >= 4096 {
            return Err("recorded font dialogs exceed research limits".into());
        }
        let answer = self
            .font_answers
            .pop_front()
            .ok_or("recording backend has no scripted font dialog answer")?;
        self.font_dialogs.push(current.to_vec());
        Ok(answer)
    }
    fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error>> {
        Ok(0)
    }
    fn fullscreen(&self) -> Result<bool, Box<dyn Error>> {
        Ok(self.fullscreen)
    }
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error>> {
        if matches!(request, PlatformRequest::WindowSize(size) if *size != [1280, 720]) {
            return Err("unsupported window size request".into());
        }
        if matches!(request, PlatformRequest::AspectMode(mode) if *mode > 3) {
            return Err("invalid aspect mode".into());
        }
        if matches!(request, PlatformRequest::RenderMode(mode) if *mode > 1) {
            return Err("unsupported render mode".into());
        }
        let size = match request {
            PlatformRequest::Fullscreen(_)
            | PlatformRequest::AspectMode(_)
            | PlatformRequest::AspectScaling(_)
            | PlatformRequest::RenderMode(_)
            | PlatformRequest::WindowSize(_) => 0,
            PlatformRequest::Title(bytes) => bytes.len(),
            PlatformRequest::Cursor { bitmap, .. } => {
                bitmap.as_ref().map_or(0, |b| b.width() * b.height() * 4)
            }
            PlatformRequest::Icon { bytes, .. } => bytes.len(),
        };
        if self.requests.len() >= 4096 || size > MAX_PAYLOAD - self.bytes {
            return Err("recorded platform requests exceed research limits".into());
        }
        self.requests.push(request.clone());
        if let PlatformRequest::Fullscreen(value) = request {
            self.fullscreen = *value;
        }
        if let PlatformRequest::AspectMode(value) = request {
            self.aspect_mode = Some(*value);
        }
        if let PlatformRequest::AspectScaling(value) = request {
            self.aspect_scaling = Some(*value);
        }
        if let PlatformRequest::RenderMode(value) = request {
            self.render_mode = *value;
        }
        if let PlatformRequest::WindowSize(value) = request {
            self.window_size = Some(*value);
        }
        self.bytes += size;
        Ok(())
    }
}

fn validate_icon(bytes: &[u8]) -> Result<(), FormatError> {
    if bytes.len() < 6 || bytes[..4] != [0, 0, 1, 0] {
        return Err(FormatError("expected ICO directory"));
    }
    let count = usize::from(u16::from_le_bytes([bytes[4], bytes[5]]));
    let end = 6 + 16 * count;
    if count == 0 || count > 256 || end > bytes.len() {
        return Err(FormatError("ICO directory count exceeds bounds"));
    }
    for entry in bytes[6..end].as_chunks::<16>().0 {
        let size = u64::from(u32::from_le_bytes(
            entry[8..12].try_into().expect("16-byte entry"),
        ));
        let offset = u64::from(u32::from_le_bytes(
            entry[12..16].try_into().expect("16-byte entry"),
        ));
        if entry[3] != 0 || size == 0 || offset < end as u64 || offset + size > bytes.len() as u64 {
            return Err(FormatError("ICO image range exceeds file"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailConfig {
    pub width: u32,
    pub height: u32,
    pub enabled: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformEvent {
    Yield,
    Applied,
    CursorUnchanged,
    Configured,
}
/// Separator 0x1400de070 between base title and scene title: Shift-JIS full-width space.
const TITLE_SEPARATOR: [u8; 2] = [0x81, 0x40];

#[derive(Default)]
pub struct PlatformHost {
    /// Base window title +340, set by 0x2160.
    title: Option<Vec<u8>>,
    /// Scene title +14336; constructor 0x140068580 zeroes it, 0x2161 replaces it.
    scene_title: Vec<u8>,
    // Original constructor 0x14008d540 sets selection to zero.
    cursor: u32,
    icon: Option<Vec<u8>>,
    thumbnail: Option<ThumbnailConfig>,
    /// CHRONOCLOCK device +144, initialized from WINDOW_RESOLVING.
    others_display_use: bool,
}
impl PlatformHost {
    pub fn configure_display_selection(&mut self, ini: &[u8]) {
        self.others_display_use =
            crate::save_menu::profile_int(ini, "WINDOW_RESOLVING", "OTHERS_DISPLAY_USE", 0) != 0;
    }
    pub fn title(&self) -> Option<&[u8]> {
        self.title.as_deref()
    }
    pub fn scene_title(&self) -> &[u8] {
        &self.scene_title
    }
    /// The window title 0x140069fd0 sets after a load: +340, the separator and the saved
    /// +14336, as 0x2161 composes it.
    pub(crate) fn restored_title(&self, title: &[u8]) -> Result<Vec<u8>, &'static str> {
        let mut text = self
            .title
            .clone()
            .ok_or("base window title +340 is unknown at load")?;
        text.extend(TITLE_SEPARATOR);
        text.extend(title);
        if text.len() >= 128 {
            return Err("restored window title exceeds 127 bytes");
        }
        Ok(text)
    }
    /// Record 0x101 and the window title. The backend applies the title before the scene
    /// title commits.
    pub(crate) fn restore_scene_title(
        &mut self,
        title: Vec<u8>,
        backend: &mut impl PlatformBackend,
    ) -> Result<(), Box<dyn Error>> {
        let text = self.restored_title(&title)?;
        backend.apply(&PlatformRequest::Title(text))?;
        self.scene_title = title;
        Ok(())
    }
    pub fn cursor(&self) -> u32 {
        self.cursor
    }
    pub fn icon(&self) -> Option<&[u8]> {
        self.icon.as_deref()
    }
    pub fn thumbnail(&self) -> Option<ThumbnailConfig> {
        self.thumbnail
    }
    pub fn try_handle(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut impl PlatformBackend,
        assets: &mut impl PlatformAssets,
        cursors: &[Vec<u8>],
    ) -> Result<Option<PlatformEvent>, Box<dyn Error>> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(None);
        };
        if command == 0x2ff {
            // 0x140078400 -> device +144: preference, not monitor availability.
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, u32::from(self.others_display_use))
                .expect("slot 0 exists");
            return Ok(Some(PlatformEvent::Configured));
        }
        if command == 0x164 {
            let raw = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2164, e.0))?;
            backend.apply(&PlatformRequest::Fullscreen(raw != 0))?;
            vm.complete_system(SystemResult(0xc004))?;
            return Ok(Some(PlatformEvent::Yield));
        }
        if command == 0x16e {
            let changed = backend.reset_window_size()?;
            vm.complete_system(SystemResult(if changed { 0xc000 } else { 0x4000 }))?;
            return Ok(Some(if changed {
                PlatformEvent::Yield
            } else {
                PlatformEvent::Configured
            }));
        }
        if command == 0x16d {
            // 0x140075630: negative or >= 4 selects 0; no yield or system result.
            let raw = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x216d, e.0))? as i32;
            let requested = if (0..4).contains(&raw) { raw as u32 } else { 0 };
            let mode = supported_render_mode(backend, requested)?;
            if mode != backend.render_mode()? {
                backend.apply(&PlatformRequest::RenderMode(mode))?;
            }
            vm.complete_system(SystemResult(0x4004))?;
            return Ok(Some(PlatformEvent::Configured));
        }
        if command == 0x2fd {
            let raw = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x22fd, e.0))?;
            if raw != backend.aspect_mode()? {
                backend.apply(&PlatformRequest::AspectMode(if raw > 3 { 1 } else { raw }))?;
            }
            vm.complete_system(SystemResult(0x4004))?;
            return Ok(Some(PlatformEvent::Configured));
        }
        if command == 0x2c9 {
            let raw = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x22c9, e.0))?;
            let requested = raw != 0;
            // Compare before capability fallback. An unsupported enable still
            // refreshes fullscreen presentation, even though the stored value stays 0.
            let changed = requested != backend.aspect_scaling()?;
            let fullscreen = if changed {
                let supported = backend.stretch_filter_supported()?;
                let enabled = requested && supported;
                let fullscreen = backend.fullscreen()?;
                backend.apply(&PlatformRequest::AspectScaling(enabled))?;
                fullscreen
            } else {
                false
            };
            vm.complete_system(SystemResult(if fullscreen { 0xc004 } else { 0x4004 }))?;
            return Ok(Some(if fullscreen {
                PlatformEvent::Yield
            } else {
                PlatformEvent::Configured
            }));
        }
        if command == 0x2ca {
            let value = backend.aspect_scaling()?;
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, u32::from(value))
                .expect("slot 0 exists");
            return Ok(Some(PlatformEvent::Configured));
        }
        if command == 0x2fc {
            let value = backend.aspect_mode()?;
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
            return Ok(Some(PlatformEvent::Configured));
        }
        if command == 0x2c8 {
            let value = backend.stretch_filter_supported()?;
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, u32::from(value))
                .expect("slot 0 exists");
            return Ok(Some(PlatformEvent::Configured));
        }
        if command == 0x16f {
            // Screen viewport +152; 0x140018260 selects WS_THICKFRAME with it.
            let value = backend.resizable()?;
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, u32::from(value))
                .expect("slot 0 exists");
            return Ok(Some(PlatformEvent::Configured));
        }
        if command == 0x16c {
            let mode = backend.render_mode()?;
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, mode)
                .expect("slot 0 exists");
            return Ok(Some(PlatformEvent::Configured));
        }
        if command == 0x16b {
            // 0x140075570: D3DCAPS9.PixelShaderVersion major byte.
            let major = backend.pixel_shader_major()?;
            let value = 3 | if major >= 2 { 4 } else { 0 } | if major >= 3 { 8 } else { 0 };
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
            return Ok(Some(PlatformEvent::Configured));
        }
        if command == 0x163 {
            // 0x1400753d0 -> 0x140027040 -> device +56 (zero is fullscreen).
            let fullscreen = backend.fullscreen()?;
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, u32::from(fullscreen))
                .expect("slot 0 exists");
            return Ok(Some(PlatformEvent::Configured));
        }
        if !matches!(command, 0x160 | 0x161 | 0x99 | 0x8f | 0x2b0) {
            return Ok(None);
        }
        let arg = vm
            .stack()
            .peek_u32(4)
            .map_err(|e| error(pc, command + 0x2000, e.0))?;
        if command == 0x2b0 {
            let height = vm.stack().peek_u32(8).map_err(|e| error(pc, 0x22b0, e.0))?;
            vm.complete_system(SystemResult(0x4008))?;
            self.thumbnail = Some(ThumbnailConfig {
                width: arg,
                height,
                enabled: arg != 0 && height != 0,
            });
            return Ok(Some(PlatformEvent::Configured));
        }
        if command == 0x99 && arg == self.cursor {
            vm.complete_system(SystemResult(0x4004))?;
            return Ok(Some(PlatformEvent::CursorUnchanged));
        }
        let mut scene = None;
        let request = match command {
            0x161 => {
                let name = vm.resolve_string(arg)?;
                let base = self
                    .title
                    .as_deref()
                    .ok_or_else(|| error(pc, 0x2161, "base window title +340 is unknown"))?;
                let mut text = base.to_vec();
                text.extend(TITLE_SEPARATOR);
                text.extend(&name);
                // The original composes into a 128-byte stack buffer without a bound check.
                if text.len() >= 128 {
                    return Err(error(pc, 0x2161, "composed title exceeds 127 bytes").into());
                }
                scene = Some(name);
                PlatformRequest::Title(text)
            }
            0x160 => {
                let title = vm.resolve_string(arg)?;
                if title.len() >= 256 {
                    return Err(error(pc, 0x2160, "title exceeds research byte limit").into());
                }
                PlatformRequest::Title(title)
            }
            0x99 => {
                let bitmap = if arg == 0 {
                    None
                } else {
                    let name = cursors
                        .get((arg - 1) as usize)
                        .ok_or_else(|| error(pc, 0x2099, "cursor ID not registered"))?;
                    let bytes = assets.read(name)?;
                    if bytes.len() > MAX_PLATFORM_ASSET {
                        return Err("cursor file exceeds research limit".into());
                    }
                    Some(Arc::new(CursorBitmap::decode(&bytes)?))
                };
                PlatformRequest::Cursor { id: arg, bitmap }
            }
            0x8f => {
                let name = vm.resolve_string(arg)?;
                let bytes = assets.read(&name)?;
                if bytes.len() > MAX_PLATFORM_ASSET {
                    return Err("icon file exceeds research limit".into());
                }
                validate_icon(&bytes)?;
                PlatformRequest::Icon {
                    name,
                    bytes: Arc::new(bytes),
                }
            }
            _ => unreachable!(),
        };
        backend.apply(&request)?;
        // All parameter sizes and references have been validated before invoking the backend.
        vm.complete_system(SystemResult(0x4004))?;
        match request {
            PlatformRequest::Title(_) if command == 0x161 => {
                self.scene_title = scene.expect("set for 0x2161");
            }
            PlatformRequest::Title(bytes) => self.title = Some(bytes),
            PlatformRequest::Cursor { id, .. } => self.cursor = id,
            PlatformRequest::Icon { name, .. } => self.icon = Some(name),
            PlatformRequest::Fullscreen(_)
            | PlatformRequest::AspectMode(_)
            | PlatformRequest::AspectScaling(_)
            | PlatformRequest::RenderMode(_)
            | PlatformRequest::WindowSize(_) => {
                unreachable!("handled before string requests")
            }
        }
        Ok(Some(PlatformEvent::Applied))
    }
}
