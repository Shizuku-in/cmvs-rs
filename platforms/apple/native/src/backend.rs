use crate::{Result, renderer::Frame};
use cmvs_core::{
    TextEncoding,
    host::{
        platform::*,
        settings::{DEFAULT_FONT_FACE, DisplayPreferences, font_face_for_encoding},
    },
    render::raster::{RgbaImage, ScaleFilter},
};
use cmvs_font::text_codec;
use cmvs_host::CommentInput;
use serde_json::{Value, json};
use std::collections::VecDeque;

pub struct Backend {
    pub display: DisplayPreferences,
    pub aspect_override: Option<u32>,
    pub scale_filter: Option<ScaleFilter>,
    pub size: [u32; 2],
    pub output: [u32; 2],
    pub windowed: [u32; 2],
    pub resizable: bool,
    pub full_move: bool,
    pub frame: Option<Frame>,
    pub events: VecDeque<Value>,
    pub comment: CommentInput,
    pub font: Option<FontDialogResult>,
    pub message: Option<MessageDialogResult>,
    pub encoding: TextEncoding,
}
impl Backend {
    pub fn viewport(&self) -> Option<cmvs_input::Viewport> {
        cmvs_input::Viewport::for_canvas(self.size, self.output, self.mode())
    }
    pub fn mode(&self) -> u32 {
        self.aspect_override.unwrap_or_else(|| {
            cmvs_core::render::viewport::presentation_mode_with_full_move(
                self.display.fullscreen,
                self.display.aspect_scaling,
                self.display.aspect_mode,
                self.full_move,
            )
        })
    }
    pub fn layer_linear(&self) -> bool {
        self.scale_filter
            .map_or(self.display.render_mode == 1, |filter| {
                filter == ScaleFilter::Bilinear
            })
    }
}
impl PlatformBackend for Backend {
    fn logical_size(&self) -> [u32; 2] {
        self.size
    }
    fn window_size(&self) -> Result<[u32; 2]> {
        Ok(self.output)
    }
    fn windowed_size(&self) -> Result<[u32; 2]> {
        Ok(self.windowed)
    }
    fn select_fullscreen_monitor(&mut self, mode: u32) -> Result<bool> {
        Ok(mode == 5)
    }
    fn display_count(&self) -> Result<u32> {
        Ok(1)
    }
    fn fullscreen(&self) -> Result<bool> {
        Ok(self.display.fullscreen)
    }
    fn aspect_mode(&self) -> Result<u32> {
        Ok(self.display.aspect_mode)
    }
    fn aspect_scaling(&self) -> Result<bool> {
        Ok(self.display.aspect_scaling)
    }
    fn render_mode(&self) -> Result<u32> {
        Ok(self.display.render_mode)
    }
    fn resizable(&self) -> Result<bool> {
        Ok(self.resizable)
    }
    fn pixel_shader_major(&self) -> Result<u8> {
        Ok(0)
    }
    fn stretch_filter_supported(&self) -> Result<bool> {
        Ok(true)
    }
    fn linear_stretch_supported(&self) -> Result<bool> {
        Ok(true)
    }
    fn read_device_frame(&mut self) -> Result<RgbaImage> {
        self.frame.as_ref().ok_or("no completed frame")?.read()
    }
    fn poll_text_dialog(&mut self, req: &TextDialogRequest) -> Result<TextDialogResult> {
        let result = self.comment.poll(req, self.encoding)?;
        if let Some(r) = self.comment.take_request() {
            self.events.push_back(json!({"kind":"comment","id":r.id,"text":r.initial,"limit":r.max_chars,"bytes":req.byte_limit()}));
        }
        Ok(result)
    }
    fn cancel_text_dialog(&mut self) {
        self.comment.cancel();
    }
    fn poll_font_dialog(&mut self, face: &[u8]) -> Result<FontDialogResult> {
        if self.font.is_none() {
            let default = face == DEFAULT_FONT_FACE
                || face == font_face_for_encoding(DEFAULT_FONT_FACE, self.encoding);
            let name = text_codec::decode(self.encoding, face).ok();
            self.events
                .push_back(json!({"kind":"font", "default":default, "face":name}));
        }
        match self.font.replace(FontDialogResult::Pending) {
            None | Some(FontDialogResult::Pending) => Ok(FontDialogResult::Pending),
            Some(answer) => {
                self.font = None;
                Ok(answer)
            }
        }
    }
    fn poll_message_dialog(&mut self, req: &MessageDialogRequest) -> Result<MessageDialogResult> {
        if self.message.is_none() {
            self.events.push_back(json!({"kind":"message","title":text_codec::decode(self.encoding,&req.caption)?,"text":text_codec::decode(self.encoding,&req.text)?,"confirm":req.kind==MessageDialogKind::Confirm}));
        }
        match self.message.replace(MessageDialogResult::Pending) {
            None | Some(MessageDialogResult::Pending) => Ok(MessageDialogResult::Pending),
            Some(answer) => {
                self.message = None;
                Ok(answer)
            }
        }
    }
    fn apply(&mut self, req: &PlatformRequest) -> Result<()> {
        match req {
            PlatformRequest::Fullscreen(v) => self.display.fullscreen = *v,
            PlatformRequest::AspectMode(v) if *v <= 3 => self.display.aspect_mode = *v,
            PlatformRequest::AspectScaling(v) => self.display.aspect_scaling = *v,
            PlatformRequest::RenderMode(v) if *v <= 1 => self.display.render_mode = *v,
            PlatformRequest::Title(bytes) => {
                self.events.push_back(
                    json!({"kind":"title","text":text_codec::decode(self.encoding,bytes)?}),
                );
                return Ok(());
            }
            PlatformRequest::Cursor { .. } if !cfg!(target_os = "macos") => return Ok(()),
            PlatformRequest::Cursor { bitmap, .. } => {
                let cursor=bitmap.as_ref().map(|b| -> Result<_>{Ok(json!({"width":b.width(),"height":b.height(),"hotspot":b.hotspot(),"pixels":b.rgba()?}))}).transpose()?;
                self.events
                    .push_back(json!({"kind":"cursor","cursor":cursor}));
                return Ok(());
            }
            PlatformRequest::FullscreenMonitor(_) | PlatformRequest::Icon { .. } => return Ok(()),
            PlatformRequest::WindowSize(size) if self.resizable => {
                self.windowed = *size;
                self.events
                    .push_back(json!({"kind":"windowSize", "size":size}));
                return Ok(());
            }
            PlatformRequest::WindowSize(size) if *size == self.size => return Ok(()),
            _ => return Err("unsupported Apple display request".into()),
        }
        self.events.push_back(json!({"kind":"display","fullscreen":self.display.fullscreen,"mode":self.mode(),"linear":self.layer_linear()}));
        Ok(())
    }
}
