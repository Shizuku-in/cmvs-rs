//! Scene composition shared by software and GPU hosts.
use cmvs_core::{
    FormatError, render::camera::CameraHost, render::message::MessageHost,
    render::scene::SceneHost, render::scene::SceneObject, render::scene_render::RenderFrame,
    render::scene_render::prepare_scene,
};

/// Holds generated glyph objects while the draw queue borrows their images.
/// Created by [`crate::font::FontRenderer::compose`]; scene and window state stay borrowed.
pub struct SceneComposition<'a> {
    pub(crate) scene: &'a SceneHost,
    pub(crate) cameras: &'a CameraHost,
    pub(crate) messages: &'a MessageHost,
    pub(crate) glyphs: Vec<SceneObject>,
}
impl SceneComposition<'_> {
    /// Prepare the shared layer queue at the message system's logical canvas size.
    /// Glyphs precede visible windows on equal layer keys, matching software rendering.
    pub fn prepare(&self) -> Result<RenderFrame<'_>, FormatError> {
        let mut additional: Vec<_> = self.glyphs.iter().collect();
        additional.extend(self.messages.visible_window_objects());
        let [width, height] = self.messages.screen_size();
        prepare_scene(self.scene, Some(self.cameras), &additional, width, height)
    }
}
