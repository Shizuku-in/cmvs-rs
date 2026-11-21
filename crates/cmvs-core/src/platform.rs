//! Host services for windows, displays, dialogs, and frame readback.
//!
//! Implement [`PlatformBackend`] for a real host. [`PlatformHost`] translates script
//! commands into validated [`PlatformRequest`] values and backend queries.
//! [`RecordingBackend`] records requests for headless use; it does not open a window.
//! [`PlatformAssets`] supplies bounded cursor and icon resource bytes.
use crate::filesystem::HostFs;
use crate::{
    FormatError, MAX_PAYLOAD, TextEncoding, bytecode::DecodeError, cursor::CursorBitmap,
    dispatch::Call, evaluate::error, raster::ScaleFilter,
};
use std::{
    error::Error,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Maximum cursor or icon asset size accepted by [`DirectoryAssets`], in bytes.
pub const MAX_PLATFORM_ASSET: usize = 4 * 1024 * 1024;
/// Read platform assets such as cursors and window icons by game-relative name.
pub trait PlatformAssets {
    /// Return the asset bytes, or an error when missing, invalid, or inaccessible.
    fn read(&mut self, name: &[u8]) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>;
}
/// Platform assets confined to an existing canonical game directory.
pub struct DirectoryAssets {
    fs: Arc<dyn HostFs>,
    root: PathBuf,
}
impl DirectoryAssets {
    /// Resolve `root` with the given [`HostFs`] and require it to be a directory.
    pub fn with_fs(
        root: impl AsRef<Path>,
        fs: Arc<dyn HostFs>,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let root = fs.canonicalize(root.as_ref())?;
        if !fs.is_dir(&root) {
            return Err("platform asset root is not a directory".into());
        }
        Ok(Self { fs, root })
    }

    /// Resolve `root` and require it to be a directory.
    #[cfg(feature = "std-fs")]
    pub fn new(root: impl AsRef<Path>) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Self::with_fs(root, crate::filesystem::default_fs())
    }
}
impl PlatformAssets for DirectoryAssets {
    fn read(&mut self, name: &[u8]) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        let file = self
            .fs
            .open_read(&crate::filesystem::resolve_under_with_fs(
                &*self.fs, &self.root, name,
            )?)?;
        let mut bytes = Vec::new();
        file.take(MAX_PLATFORM_ASSET as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_PLATFORM_ASSET {
            return Err("platform asset exceeds research limit".into());
        }
        Ok(bytes)
    }
}

/// Numbered CMVS display modes exposed to the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum RenderMode {
    Point = 0,
    Linear = 1,
    HighQuality = 2,
    HighQuality2 = 3,
}

impl RenderMode {
    /// The scaling algorithm used to present this mode.
    pub fn filter(self) -> ScaleFilter {
        match self {
            Self::Point => ScaleFilter::Nearest,
            Self::Linear => ScaleFilter::Bilinear,
            Self::HighQuality => ScaleFilter::Keys,
            Self::HighQuality2 => ScaleFilter::Lanczos4,
        }
    }
}

impl From<RenderMode> for ScaleFilter {
    fn from(mode: RenderMode) -> Self {
        mode.filter()
    }
}

impl TryFrom<u32> for RenderMode {
    type Error = FormatError;

    fn try_from(mode: u32) -> Result<Self, Self::Error> {
        match mode {
            0 => Ok(Self::Point),
            1 => Ok(Self::Linear),
            2 => Ok(Self::HighQuality),
            3 => Ok(Self::HighQuality2),
            _ => Err(FormatError("invalid render mode")),
        }
    }
}

impl From<bool> for RenderMode {
    fn from(linear: bool) -> Self {
        if linear { Self::Linear } else { Self::Point }
    }
}

/// A validated change the host must apply to its window or presentation state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformRequest {
    Fullscreen(bool),
    /// Fullscreen display preference: 0 primary, 1..4 explicit, 5 follows window.
    FullscreenMonitor(u32),
    AspectMode(u32),
    /// Active display's scaling enable (+172). In fullscreen, refresh presentation
    /// resources before acknowledging; keep the saved windowed client size intact.
    AspectScaling(bool),
    /// Device +112 presentation filter: point, linear, Keys, or Lanczos4 (0 through 3).
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
/// A text-entry dialog whose byte strings use the selected game's encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDialogRequest {
    pub title: Vec<u8>,
    pub prompt: Vec<u8>,
    pub initial: Vec<u8>,
    /// ANSI edit limit: 100 bytes allows 50 double-byte characters in the game encoding.
    pub max_bytes: u32,
}
impl TextDialogRequest {
    /// Maximum encoded bytes, leaving room for the save field's terminator.
    pub fn byte_limit(&self) -> u32 {
        match self.max_bytes {
            0 => 255,
            limit => limit.min(255),
        }
    }

    /// Native input limit. Legacy dialogs budget two bytes per character.
    pub fn character_limit(&self, encoding: TextEncoding) -> u32 {
        let bytes = self.byte_limit();
        if encoding == TextEncoding::Utf8 {
            bytes.min(127)
        } else {
            (bytes / 2).max(1)
        }
    }
}
pub enum TextDialogResult {
    Pending,
    Cancelled,
    Accepted(Vec<u8>),
}

pub enum FontDialogResult {
    Pending,
    Cancelled,
    Selected(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDialogKind {
    /// Acknowledge-only alert (`MessageBoxA` `MB_ICONEXCLAMATION | MB_OK`).
    Alert,
    /// Two-way confirmation (`0x21b0` exit confirmation or `0x21b1` mode >= 1).
    Confirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDialogRequest {
    pub kind: MessageDialogKind,
    pub caption: Vec<u8>,
    pub text: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDialogResult {
    Pending,
    Cancelled,
    Accepted,
}

/// Outcome of polling the host for the latest completed logical device frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceFrameResult {
    /// Asynchronous device readback is in progress; the VM keeps the instruction pending.
    Pending,
    /// Completed logical engine canvas in RGBA order.
    Ready(crate::raster::RgbaImage),
}

/// Host implementation of window, presentation, and modal-dialog services.
///
/// Implement [`Self::apply`] and the queries needed by the games your host supports.
/// Most optional methods return an error by default. Queries should describe the
/// current host state. Text and font dialogs may remain pending across VM ticks
/// while the host processes events.
///
/// Logical dimensions are engine/script pixels; window dimensions are host client
/// pixels. Completed-frame readback must include the fonts and movies actually drawn.
pub trait PlatformBackend {
    /// Select a fullscreen target; return whether the selected display changed.
    /// Mode 5 must return true even when the current target is unchanged.
    fn select_fullscreen_monitor(
        &mut self,
        _mode: u32,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement fullscreen monitor selection".into())
    }

    /// Number of available host displays; recording backends expose one virtual display.
    fn display_count(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement display count queries".into())
    }
    fn logical_size(&self) -> [u32; 2] {
        crate::viewport::ScreenSize::default().pixels()
    }
    /// Read the latest completed engine frame in device/script pixels, including
    /// fonts and movies. Do not redraw current VM state or reuse a held 0x2304 capture.
    /// Absence of a completed frame is an error, not a black/empty fabricated image.
    fn read_device_frame(
        &mut self,
    ) -> Result<crate::raster::RgbaImage, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement device frame readback".into())
    }
    /// Poll for the latest completed engine frame, allowing asynchronous GPU backends to
    /// report [`DeviceFrameResult::Pending`] while a readback is in flight.
    /// Synchronous hosts only need to implement [`Self::read_device_frame`].
    fn poll_device_frame(&mut self) -> Result<DeviceFrameResult, Box<dyn Error + Send + Sync>> {
        self.read_device_frame().map(DeviceFrameResult::Ready)
    }
    fn window_size(&self) -> Result<[u32; 2], Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement window size queries".into())
    }
    fn windowed_size(&self) -> Result<[u32; 2], Box<dyn Error + Send + Sync>> {
        if self.fullscreen()? {
            return Err("platform backend does not track the future windowed size".into());
        }
        self.window_size()
    }
    fn reset_window_size(&mut self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        if !self.resizable()? {
            return Ok(false);
        }
        if self.windowed_size()? == self.logical_size() {
            return Ok(false);
        }
        let fullscreen = self.fullscreen()?;
        self.apply(&PlatformRequest::WindowSize(self.logical_size()))?;
        Ok(!fullscreen)
    }
    /// 0x2163 queries the device's current display mode, not the startup preference.
    fn fullscreen(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement display mode queries".into())
    }
    /// Native pixel shader capability; software backends report zero. This does
    /// not grant support for individual VM effects in the core renderer.
    fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement pixel shader capability queries".into())
    }
    /// Whether the host implements both high-quality presentation filters.
    fn high_quality_scaling_supported(&self) -> bool {
        false
    }
    fn render_mode(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement render mode queries".into())
    }
    fn resizable(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement window resizing queries".into())
    }
    /// StretchRect-equivalent point or linear enlargement in the selected backend.
    fn stretch_filter_supported(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement stretch filter queries".into())
    }
    /// D3DCAPS9.StretchRectFilterCaps MAGFLINEAR alone; render mode 1 requires it.
    fn linear_stretch_supported(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement linear stretch queries".into())
    }
    /// The host feeds keyboard and IME edits directly into the canvas field.
    fn inline_comment_input(&self) -> bool {
        false
    }
    fn poll_text_dialog(
        &mut self,
        _request: &TextDialogRequest,
    ) -> Result<TextDialogResult, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement the text dialog".into())
    }
    /// Discard a pending keyboard request and its reply when a restore replaces the dialog.
    fn cancel_text_dialog(&mut self) {}
    /// Fallback for CVNS `0x21b7` when the in-game picker is disabled.
    /// The two swatches use `0xRRGGBB` order. The default response is cancellation.
    fn choose_color(
        &mut self,
        _swatches: [u32; 2],
    ) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
        Ok(None)
    }
    /// 0x213d's modal font dialog, seeded with the global face in the game encoding.
    /// Ok(None) is a cancelled dialog; Ok(Some) the chosen face in the game encoding.
    fn choose_font(
        &mut self,
        _current: &[u8],
    ) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement the font dialog".into())
    }
    /// Mobile dialogs complete asynchronously while the host processes lifecycle
    /// events. Synchronous desktop dialogs keep the existing choose_font path.
    fn poll_font_dialog(
        &mut self,
        current: &[u8],
    ) -> Result<FontDialogResult, Box<dyn Error + Send + Sync>> {
        Ok(match self.choose_font(current)? {
            Some(face) => FontDialogResult::Selected(face),
            None => FontDialogResult::Cancelled,
        })
    }
    /// CVNS modal message/confirmation dialog (`0x21b0` / `0x21b1`).
    fn poll_message_dialog(
        &mut self,
        _request: &MessageDialogRequest,
    ) -> Result<MessageDialogResult, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement message dialogs".into())
    }
    /// 0=unscaled when it fits, 1=contain, 2=cover, 3=stretch (0x14001a360).
    fn aspect_mode(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement aspect mode queries".into())
    }
    fn aspect_scaling(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Err("platform backend does not implement aspect scaling queries".into())
    }
    /// Must commit the request on Ok; on Err leave backend state unchanged for retry.
    /// This is not a queued acknowledgement. Native event-loop implementations must honor this contract.
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error + Send + Sync>>;
}
/// 0x14001cfd0: a request the device cannot honor leaves render mode 0 (point).
/// For a legacy backend reporting only a D3D shader version, modes 2/3 also
/// require the unmodeled device flag at +124.
pub fn supported_render_mode(
    backend: &(impl PlatformBackend + ?Sized),
    requested: u32,
) -> Result<u32, Box<dyn Error + Send + Sync>> {
    Ok(match requested {
        1 if backend.linear_stretch_supported()? => 1,
        2 | 3 if backend.high_quality_scaling_supported() => requested,
        2 | 3 if u32::from(backend.pixel_shader_major()?) >= requested => {
            return Err("shader presentation filters need unmodeled device flag +124".into());
        }
        _ => 0,
    })
}
/// Headless backend that records requests and accepts explicit dialog answers and frames.
#[derive(Default)]
pub struct RecordingBackend {
    screen_size: crate::viewport::ScreenSize,
    requests: Vec<PlatformRequest>,
    bytes: usize,
    fullscreen: bool,
    aspect_mode: Option<u32>,
    aspect_scaling: Option<bool>,
    device_frame: Option<crate::raster::RgbaImage>,
    render_mode: u32,
    high_quality_scaling: bool,
    window_size: Option<[u32; 2]>,
    font_answers: std::collections::VecDeque<Option<Vec<u8>>>,
    font_dialogs: Vec<Vec<u8>>,
    text_answers: std::collections::VecDeque<Option<Vec<u8>>>,
    text_dialogs: Vec<TextDialogRequest>,
    message_answers: std::collections::VecDeque<bool>,
    default_message_answer: Option<bool>,
    message_dialogs: Vec<MessageDialogRequest>,
}
impl RecordingBackend {
    pub fn set_screen_size(&mut self, size: crate::viewport::ScreenSize) {
        self.screen_size = size;
    }
    /// Simulate a caller that can present the high-quality draw modes.
    pub fn with_high_quality_scaling(mut self) -> Self {
        self.high_quality_scaling = true;
        self
    }
    /// Explicit software device draw boundary. Frame producers must publish the
    /// complete engine canvas after drawing; VM commands do not update it.
    pub fn present_frame(&mut self, frame: crate::raster::RgbaImage) {
        self.device_frame = Some(frame);
    }
    pub fn requests(&self) -> &[PlatformRequest] {
        &self.requests
    }
    pub fn answer_text_dialog(&mut self, answer: Option<Vec<u8>>) {
        self.text_answers.push_back(answer);
    }
    pub fn text_dialogs(&self) -> &[TextDialogRequest] {
        &self.text_dialogs
    }
    /// Scripted result for the next font dialog; without one the dialog is an error.
    pub fn answer_font_dialog(&mut self, answer: Option<Vec<u8>>) {
        self.font_answers.push_back(answer);
    }
    /// The global face each font dialog was opened with.
    pub fn font_dialogs(&self) -> &[Vec<u8>] {
        &self.font_dialogs
    }
    pub fn answer_message_dialog(&mut self, accepted: bool) {
        self.message_answers.push_back(accepted);
    }
    pub fn set_default_message_answer(&mut self, accepted: Option<bool>) {
        self.default_message_answer = accepted;
    }
    pub fn message_dialogs(&self) -> &[MessageDialogRequest] {
        &self.message_dialogs
    }
}
impl PlatformBackend for RecordingBackend {
    fn poll_message_dialog(
        &mut self,
        request: &MessageDialogRequest,
    ) -> Result<MessageDialogResult, Box<dyn Error + Send + Sync>> {
        if self.message_dialogs.len() >= 4096 {
            return Err("recorded message dialogs exceed research limits".into());
        }
        let accepted = self
            .message_answers
            .pop_front()
            .or(self.default_message_answer)
            .ok_or("recording backend has no scripted message dialog answer")?;
        self.message_dialogs.push(request.clone());
        Ok(if accepted {
            MessageDialogResult::Accepted
        } else {
            MessageDialogResult::Cancelled
        })
    }
    fn poll_text_dialog(
        &mut self,
        request: &TextDialogRequest,
    ) -> Result<TextDialogResult, Box<dyn Error + Send + Sync>> {
        if self.text_dialogs.len() >= 4096 {
            return Err("recorded text dialogs exceed research limits".into());
        }
        let answer = self
            .text_answers
            .pop_front()
            .ok_or("recording backend has no scripted text dialog answer")?;
        self.text_dialogs.push(request.clone());
        Ok(match answer {
            Some(text) => TextDialogResult::Accepted(text),
            None => TextDialogResult::Cancelled,
        })
    }

    fn select_fullscreen_monitor(
        &mut self,
        mode: u32,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        self.apply(&PlatformRequest::FullscreenMonitor(mode))?;
        // One virtual display: every supported mode resolves to the same target.
        Ok(mode == 5)
    }

    fn display_count(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Ok(1)
    }
    fn logical_size(&self) -> [u32; 2] {
        self.screen_size.pixels()
    }
    fn read_device_frame(
        &mut self,
    ) -> Result<crate::raster::RgbaImage, Box<dyn Error + Send + Sync>> {
        self.device_frame
            .clone()
            .ok_or_else(|| "recording device has no completed frame".into())
    }
    fn windowed_size(&self) -> Result<[u32; 2], Box<dyn Error + Send + Sync>> {
        self.window_size()
    }
    fn window_size(&self) -> Result<[u32; 2], Box<dyn Error + Send + Sync>> {
        Ok(self.window_size.unwrap_or([960, 540]))
    }
    fn aspect_mode(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Ok(self.aspect_mode.unwrap_or(1))
    }
    fn aspect_scaling(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(self.aspect_scaling.unwrap_or(true))
    }
    fn stretch_filter_supported(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    fn linear_stretch_supported(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    fn resizable(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(true)
    }
    fn render_mode(&self) -> Result<u32, Box<dyn Error + Send + Sync>> {
        Ok(self.render_mode)
    }
    fn choose_font(
        &mut self,
        current: &[u8],
    ) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>> {
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
    fn pixel_shader_major(&self) -> Result<u8, Box<dyn Error + Send + Sync>> {
        Ok(0)
    }
    fn high_quality_scaling_supported(&self) -> bool {
        self.high_quality_scaling
    }
    fn fullscreen(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        Ok(self.fullscreen)
    }
    fn apply(&mut self, request: &PlatformRequest) -> Result<(), Box<dyn Error + Send + Sync>> {
        if matches!(request, PlatformRequest::FullscreenMonitor(mode) if *mode > 5) {
            return Err("unsupported fullscreen monitor mode".into());
        }
        if let PlatformRequest::WindowSize(size) = request {
            crate::viewport::ScreenSize::new(*size)?;
        }
        if matches!(request, PlatformRequest::AspectMode(mode) if *mode > 3) {
            return Err("invalid aspect mode".into());
        }
        let max_render_mode = if self.high_quality_scaling_supported() {
            3
        } else {
            1
        };
        if matches!(request, PlatformRequest::RenderMode(mode) if *mode > max_render_mode) {
            return Err("unsupported render mode".into());
        }
        let size = match request {
            PlatformRequest::FullscreenMonitor(_)
            | PlatformRequest::Fullscreen(_)
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
    fullscreen_monitor_mode: u32,
    fullscreen_monitor_initialized: bool,
    window_full_move: bool,
    /// CVNS display +14 keep-aspect flag (`0x222d` / `0x222e`).
    cvns_keep_aspect: bool,
}
impl PlatformHost {
    pub fn cvns_keep_aspect(&self) -> bool {
        self.cvns_keep_aspect
    }
    /// Legacy device +56: omit the centered origin in unscaled fullscreen mode.
    pub fn window_full_move(&self) -> bool {
        self.window_full_move
    }
    /// Called only for builds whose legacy display setting has been inspected.
    pub fn configure_window_full_move(&mut self, ini: &[u8]) {
        self.window_full_move =
            crate::save_menu::profile_int(ini, "SYSTEM_INFO", "WINDOW_FULL_MOVE_USE", 0) != 0;
    }
    /// cmvs-20070119 0x43bae0: read/write boolean, always consumes two words.
    pub(crate) fn handle_window_full_move(
        &mut self,
        call: &mut Call<'_>,
    ) -> Result<(), DecodeError> {
        let mode = call.argument(4, "mode")?;
        let value = if mode != 0 {
            Some(call.argument(8, "value")? != 0)
        } else {
            call.ignore_argument(8, "unused value")?;
            None
        };
        call.complete(0x4008)?;
        if let Some(value) = value {
            self.window_full_move = value;
        } else {
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(self.window_full_move))
                .expect("slot 0 exists");
        }
        Ok(())
    }

    pub fn fullscreen_monitor_mode(&self) -> u32 {
        self.fullscreen_monitor_mode
    }

    pub fn configure_display_selection(&mut self, ini: &[u8]) {
        self.others_display_use =
            crate::save_menu::profile_int(ini, "WINDOW_RESOLVING", "OTHERS_DISPLAY_USE", 0) != 0;
        self.fullscreen_monitor_mode =
            crate::save_menu::profile_int(ini, "WINDOW_RESOLVING", "FULL_SCREEN_MONITOR_MODE", 0)
                as u32;
        self.fullscreen_monitor_initialized = false;
    }
    pub fn title(&self) -> Option<&[u8]> {
        self.title.as_deref()
    }
    pub fn scene_title(&self) -> &[u8] {
        &self.scene_title
    }
    /// The window title 0x140069fd0 sets after a load: +340, the separator and the saved
    /// +14336, as 0x2161 composes it.
    pub(crate) fn restored_title(
        &self,
        title: &[u8],
        encoding: TextEncoding,
    ) -> Result<Vec<u8>, &'static str> {
        let mut text = self
            .title
            .clone()
            .ok_or("base window title +340 is unknown at load")?;
        // Host-generated text uses the same encoding as the script's title strings.
        text.extend(encoding.title_separator());
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
        encoding: TextEncoding,
        backend: &mut impl PlatformBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let text = self.restored_title(&title, encoding)?;
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
    pub(crate) fn handle(
        &mut self,
        call: &mut Call<'_>,
        backend: &mut dyn PlatformBackend,
        assets: &mut dyn PlatformAssets,
        cursors: &[Vec<u8>],
    ) -> Result<PlatformEvent, Box<dyn Error + Send + Sync>> {
        let (pc, command) = (call.pc, call.command);
        if command == 0x3c6 {
            let raw = call.argument(4, "fullscreen monitor mode")?;
            if raw > 5 {
                return Err("unsupported fullscreen monitor mode".into());
            }
            let mode = if (1..=4).contains(&raw) && backend.display_count()? < raw {
                0
            } else {
                raw
            };
            // Read fallible state before applying the request, so retry keeps host and VM intact.
            let fullscreen = backend.fullscreen()?;
            let changed = backend.select_fullscreen_monitor(mode)?;
            let yield_frame = changed && fullscreen;
            call.complete(if yield_frame { 0xc004 } else { 0x4004 })?;
            self.fullscreen_monitor_mode = mode;
            self.fullscreen_monitor_initialized = true;
            return Ok(if yield_frame {
                PlatformEvent::Yield
            } else {
                PlatformEvent::Configured
            });
        }
        if command == 0x3c5 {
            // REALIVE device +148: mode 0=primary, 1..4=explicit, 5=window monitor.
            let mode = self.fullscreen_monitor_mode;
            if mode > 5 {
                return Err("unsupported fullscreen monitor mode".into());
            }
            let mode = if !self.fullscreen_monitor_initialized
                && (1..=4).contains(&mode)
                && backend.display_count()? < mode
            {
                0
            } else {
                mode
            };
            call.complete(0x4000)?;
            self.fullscreen_monitor_mode = mode;
            self.fullscreen_monitor_initialized = true;
            call.vm
                .state_mut()
                .set_system_integer(0, mode)
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x3c7 {
            let count = backend.display_count()?;
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, count)
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x2ff {
            // 0x140078400 -> device +144: preference, not monitor availability.
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(self.others_display_use))
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x164 {
            let raw = call.argument(4, argument_names(command)[0])?;
            backend.apply(&PlatformRequest::Fullscreen(raw != 0))?;
            call.complete(0xc004)?;
            return Ok(PlatformEvent::Yield);
        }
        if command == 0x16e {
            let changed = backend.reset_window_size()?;
            call.complete(if changed { 0xc000 } else { 0x4000 })?;
            return Ok(if changed {
                PlatformEvent::Yield
            } else {
                PlatformEvent::Configured
            });
        }
        if command == 0x16d {
            // 0x140075630: negative or >= 4 selects 0; no yield or system result.
            let raw = call.argument(4, argument_names(command)[0])? as i32;
            let requested = if (0..4).contains(&raw) { raw as u32 } else { 0 };
            let mode = supported_render_mode(backend, requested)?;
            if mode != backend.render_mode()? {
                backend.apply(&PlatformRequest::RenderMode(mode))?;
            }
            call.complete(0x4004)?;
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x2fd {
            let raw = call.argument(4, argument_names(command)[0])?;
            if raw != backend.aspect_mode()? {
                backend.apply(&PlatformRequest::AspectMode(if raw > 3 { 1 } else { raw }))?;
            }
            call.complete(0x4004)?;
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x629 {
            let raw = call.argument(4, argument_names(command)[0])?;
            let supported = backend.stretch_filter_supported()?;
            let fullscreen = backend.fullscreen()?;
            backend.apply(&PlatformRequest::AspectScaling(raw != 0 && supported))?;
            call.complete(if fullscreen { 0xc004 } else { 0x4004 })?;
            return Ok(if fullscreen {
                PlatformEvent::Yield
            } else {
                PlatformEvent::Configured
            });
        }
        if command == 0x62b {
            let raw = call.argument(4, argument_names(command)[0])?;
            let requested = if raw > 2 { 0 } else { raw };
            let current = backend.aspect_mode()? % 3;
            let yields = if requested != current {
                let fullscreen = backend.fullscreen()?;
                backend.apply(&PlatformRequest::AspectMode(requested))?;
                fullscreen && !(matches!(current, 1 | 2) && matches!(requested, 1 | 2))
            } else {
                false
            };
            call.complete(if yields { 0xc004 } else { 0x4004 })?;
            return Ok(if yields {
                PlatformEvent::Yield
            } else {
                PlatformEvent::Configured
            });
        }
        if command == 0x62c {
            let value = backend.aspect_mode()? % 3;
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x62d {
            let raw = call.argument(4, argument_names(command)[0])?;
            let fullscreen = backend.fullscreen()?;
            let aspect = backend.aspect_mode()? % 3;
            let yields = fullscreen && aspect == 0;
            call.complete(if yields { 0xc004 } else { 0x4004 })?;
            self.cvns_keep_aspect = raw != 0;
            return Ok(if yields {
                PlatformEvent::Yield
            } else {
                PlatformEvent::Configured
            });
        }
        if command == 0x62e {
            let value = u32::from(self.cvns_keep_aspect);
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x2c9 {
            let raw = call.argument(4, argument_names(command)[0])?;
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
            call.complete(if fullscreen { 0xc004 } else { 0x4004 })?;
            return Ok(if fullscreen {
                PlatformEvent::Yield
            } else {
                PlatformEvent::Configured
            });
        }
        if command == 0x2ca {
            let value = backend.aspect_scaling()?;
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(value))
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x2fc {
            let value = backend.aspect_mode()?;
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x2c8 {
            let value = backend.stretch_filter_supported()?;
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(value))
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x16f {
            // Screen viewport +152; 0x140018260 selects WS_THICKFRAME with it.
            let value = backend.resizable()?;
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(value))
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x16c {
            let mode = backend.render_mode()?;
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, mode)
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x16b {
            let value = if backend.high_quality_scaling_supported() {
                15
            } else {
                // 0x140075570: legacy capability flags from D3DCAPS9.PixelShaderVersion.
                let major = backend.pixel_shader_major()?;
                3 | if major >= 2 { 4 } else { 0 } | if major >= 3 { 8 } else { 0 }
            };
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x163 {
            // 0x1400753d0 -> 0x140027040 -> device +56 (zero is fullscreen).
            let fullscreen = backend.fullscreen()?;
            call.complete(0x4000)?;
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(fullscreen))
                .expect("slot 0 exists");
            return Ok(PlatformEvent::Configured);
        }
        let arg = call.argument(4, argument_names(command)[0])?;
        if command == 0x2b0 {
            let height = call.argument(8, argument_names(command)[1])?;
            call.complete(0x4008)?;
            self.thumbnail = Some(ThumbnailConfig {
                width: arg,
                height,
                enabled: arg != 0 && height != 0,
            });
            return Ok(PlatformEvent::Configured);
        }
        if command == 0x99 && arg == self.cursor {
            call.complete(0x4004)?;
            return Ok(PlatformEvent::CursorUnchanged);
        }
        let mut scene = None;
        let request = match command {
            0x161 => {
                let name = call.vm.resolve_string(arg)?;
                let base = self
                    .title
                    .as_deref()
                    .ok_or_else(|| error(pc, 0x2161, "base window title +340 is unknown"))?;
                let mut text = base.to_vec();
                text.extend(call.vm.text_encoding().title_separator());
                text.extend(&name);
                // The original composes into a 128-byte stack buffer without a bound check.
                if text.len() >= 128 {
                    return Err(error(pc, 0x2161, "composed title exceeds 127 bytes").into());
                }
                scene = Some(name);
                PlatformRequest::Title(text)
            }
            0x160 => {
                let title = call.vm.resolve_string(arg)?;
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
                let name = call.vm.resolve_string(arg)?;
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
        call.complete(0x4004)?;
        match request {
            PlatformRequest::Title(_) if command == 0x161 => {
                self.scene_title = scene.expect("set for 0x2161");
            }
            PlatformRequest::Title(bytes) => self.title = Some(bytes),
            PlatformRequest::Cursor { id, .. } => self.cursor = id,
            PlatformRequest::Icon { name, .. } => self.icon = Some(name),
            PlatformRequest::FullscreenMonitor(_)
            | PlatformRequest::Fullscreen(_)
            | PlatformRequest::AspectMode(_)
            | PlatformRequest::AspectScaling(_)
            | PlatformRequest::RenderMode(_)
            | PlatformRequest::WindowSize(_) => {
                unreachable!("handled before string requests")
            }
        }
        Ok(PlatformEvent::Applied)
    }
}

fn argument_names(command: u16) -> &'static [&'static str] {
    match command {
        0x8f => &["icon name"],
        0x99 => &["cursor ID"],
        0x160 => &["window title"],
        0x161 => &["scene title"],
        0x164 => &["fullscreen"],
        0x16d => &["render mode"],
        0x2fd | 0x62b => &["aspect mode"],
        0x2c9 | 0x629 => &["aspect scaling"],
        0x62d => &["keep aspect"],
        0x2b0 => &["thumbnail width", "thumbnail height"],
        _ => unreachable!("registered platform arguments"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_scene_title_uses_game_encoding() {
        for (encoding, separator) in [
            (TextEncoding::ShiftJis, &[0x81, 0x40][..]),
            (TextEncoding::Gbk, &[0xa1, 0xa1]),
            (TextEncoding::Utf8, "\u{3000}".as_bytes()),
        ] {
            let mut host = PlatformHost {
                title: Some(b"Game".to_vec()),
                ..Default::default()
            };
            let mut backend = RecordingBackend::default();
            host.restore_scene_title(b"- Opening -".to_vec(), encoding, &mut backend)
                .unwrap();
            let mut expected = b"Game".to_vec();
            expected.extend(separator);
            expected.extend(b"- Opening -");
            assert_eq!(
                backend.requests().last(),
                Some(&PlatformRequest::Title(expected))
            );
            assert_eq!(host.scene_title(), b"- Opening -");
        }
    }

    #[test]
    fn cvns_display_aspect_scaling_keep_aspect_and_dialogs_follow_cvns_rules() {
        struct NoAssets;
        impl PlatformAssets for NoAssets {
            fn read(&mut self, _: &[u8]) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
                unreachable!("dialog/display tests do not read assets")
            }
        }
        fn run_root_cmd(
            cmd: u16,
            args: &[u32],
            strings: &[&[u8]],
            hosts_fn: impl FnOnce(&mut crate::machine::Machine, usize),
        ) {
            let mut code = Vec::new();
            for &value in args.iter().rev() {
                code.extend(0x440u16.to_le_bytes());
                code.extend(4u16.to_le_bytes());
                code.extend(value.to_le_bytes());
            }
            code.extend((cmd + 0x2000).to_le_bytes());
            let mut payload = 0u32.to_le_bytes().to_vec();
            payload.extend(&code);
            let mut raw = b"PS2A".to_vec();
            for n in [
                48,
                0,
                0,
                1,
                code.len() as u32,
                0,
                0,
                0,
                0,
                payload.len() as u32,
                0,
            ] {
                raw.extend(n.to_le_bytes());
            }
            raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
            let script = Box::leak(Box::new(crate::ps3::Script::decode(&raw).unwrap()));
            let mut vm = crate::machine::Machine::new(script).unwrap();
            vm.state_mut().set_string_slots(256);
            vm.state_mut().fill_strings(0, 256, b"").unwrap();
            for (i, s) in strings.iter().enumerate() {
                vm.state_mut().set_string(i as u32, s).unwrap();
            }
            while !matches!(vm.step().unwrap(), crate::machine::Step::System { .. }) {}
            let pc = vm.pc();
            hosts_fn(&mut vm, pc);
        }

        let mut host = PlatformHost::default();
        let mut backend = RecordingBackend::default();
        let mut assets = NoAssets;
        backend.apply(&PlatformRequest::Fullscreen(true)).unwrap();
        backend.apply(&PlatformRequest::AspectMode(1)).unwrap();

        // 0x629 always yields in fullscreen.
        run_root_cmd(0x629, &[1], &[], |vm, pc| {
            let ev = host
                .handle(
                    &mut Call::new(vm, pc, 0x629),
                    &mut backend,
                    &mut assets,
                    &[],
                )
                .unwrap();
            assert_eq!(ev, PlatformEvent::Yield);
            assert!(backend.aspect_scaling().unwrap());
        });

        // 0x62b between modes 1 and 2 does not yield even in fullscreen; transition to 0 yields.
        run_root_cmd(0x62b, &[2], &[], |vm, pc| {
            let ev = host
                .handle(
                    &mut Call::new(vm, pc, 0x62b),
                    &mut backend,
                    &mut assets,
                    &[],
                )
                .unwrap();
            assert_eq!(ev, PlatformEvent::Configured);
        });
        run_root_cmd(0x62b, &[9], &[], |vm, pc| {
            let ev = host
                .handle(
                    &mut Call::new(vm, pc, 0x62b),
                    &mut backend,
                    &mut assets,
                    &[],
                )
                .unwrap();
            assert_eq!(ev, PlatformEvent::Yield);
            assert_eq!(backend.aspect_mode().unwrap(), 0);
        });
        run_root_cmd(0x62c, &[], &[], |vm, pc| {
            host.handle(
                &mut Call::new(vm, pc, 0x62c),
                &mut backend,
                &mut assets,
                &[],
            )
            .unwrap();
            assert_eq!(vm.state().system_integer(0), Some(0));
        });

        // 0x62d yields in fullscreen iff aspect_mode == 0; 0x62e reads it back.
        run_root_cmd(0x62d, &[1], &[], |vm, pc| {
            let ev = host
                .handle(
                    &mut Call::new(vm, pc, 0x62d),
                    &mut backend,
                    &mut assets,
                    &[],
                )
                .unwrap();
            assert_eq!(ev, PlatformEvent::Yield);
            assert!(host.cvns_keep_aspect());
        });
        run_root_cmd(0x62e, &[], &[], |vm, pc| {
            host.handle(
                &mut Call::new(vm, pc, 0x62e),
                &mut backend,
                &mut assets,
                &[],
            )
            .unwrap();
            assert_eq!(vm.state().system_integer(0), Some(1));
        });

        // Dispatch checks for 0x5b0, 0x5b1, 0x5b2 (>=128 string slot), 0x417, 0x42e.
        let sem = crate::profile::Semantics::Root;
        backend.answer_message_dialog(false);
        backend.answer_message_dialog(true);
        backend.answer_text_dialog(Some(vec![0x82, 0xa0])); // Shift-JIS "あ"
        run_root_cmd(0x5b0, &[], &[], |vm, _| {
            let flow = crate::dispatch::run(
                sem,
                vm,
                &mut crate::dispatch::Hosts {
                    device_frame: Some(&mut backend),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(flow, Some(crate::dispatch::Flow::Done));
            assert_eq!(vm.state().system_integer(0), Some(0));
        });
        assert_eq!(backend.message_dialogs().len(), 1);
        assert_eq!(
            backend.message_dialogs()[0].kind,
            MessageDialogKind::Confirm
        );

        run_root_cmd(
            0x5b1,
            &[0, 0x8000_0000, 0x8000_0001],
            &[b"Cap", b"Msg"],
            |vm, _| {
                crate::dispatch::run(
                    sem,
                    vm,
                    &mut crate::dispatch::Hosts {
                        device_frame: Some(&mut backend),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(vm.state().system_integer(0), Some(1));
            },
        );
        assert_eq!(backend.message_dialogs()[1].kind, MessageDialogKind::Alert);
        assert_eq!(backend.message_dialogs()[1].caption, b"Cap");
        assert_eq!(backend.message_dialogs()[1].text, b"Msg");

        // Slot 200 (>= 128) is valid on CVNS's 256-slot string bank.
        run_root_cmd(
            0x5b2,
            &[
                0x8000_0000,
                0x8000_0001,
                0x8000_0002,
                32,
                0x8000_0000 + 200,
                1,
            ],
            &[b"T", b"P", b"Init"],
            |vm, _| {
                crate::dispatch::run(
                    sem,
                    vm,
                    &mut crate::dispatch::Hosts {
                        device_frame: Some(&mut backend),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(vm.state().system_integer(0), Some(1));
                assert_eq!(vm.state().string(200), Some([0x82, 0xa0].as_slice()));
            },
        );

        for (cmd, mode, expected) in [
            (0x417, 1, 0),
            (0x417, 2, 1),
            (0x417, 3, 1),
            (0x417, 5, 1),
            (0x417, 6, 1),
            (0x42e, 1, 1),
            (0x42e, 4, 0),
            (0x42e, 6, 1),
        ] {
            run_root_cmd(cmd, &[mode], &[], |vm, _| {
                crate::dispatch::run(
                    sem,
                    vm,
                    &mut crate::dispatch::Hosts {
                        math: true,
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(vm.state().system_integer(0), Some(expected));
            });
        }
    }
}
