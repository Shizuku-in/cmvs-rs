//! Software presentation of verified static 2D objects, their children, full-screen color
//! fades, mask transitions and camera projections and strip wipes. Unsupported transforms,
//! blend modes and unverified solid-color paths fail explicitly.
use crate::{
    FormatError, MAX_PAYLOAD,
    camera::CameraHost,
    message::RESEARCH_SCREEN,
    raster::RgbaImage,
    scene::{DrawState, MaskTransition, ObjectImage, SceneHost, SceneObject},
};

enum Layer<'a> {
    Image {
        image: &'a ObjectImage,
        /// Source x/y/width/height.
        rect: [i64; 4],
        /// Top-left destination corner: position minus origin.
        corner: [i64; 2],
        alpha: u32,
        /// The mask transition shading this slot object (not its children).
        mask: Option<&'a MaskTransition>,
        screen_mask: Option<&'a ObjectImage>,
        blend: Blend,
    },
    /// A projected or scaled 2D image, sampled over a device-space quad.
    Projected {
        image: &'a ObjectImage,
        /// Source x/y/width/height.
        rect: [i64; 4],
        /// Left/top/right/bottom in device coordinates, whose pixel centers are integers.
        quad: [f32; 4],
        alpha: u32,
        blend: Blend,
    },
    /// A rotated 2D image: device-space corners in strip order top-left, top-right,
    /// bottom-left, bottom-right of the source rectangle.
    Rotated {
        image: &'a ObjectImage,
        /// Source x/y/width/height.
        rect: [i64; 4],
        corners: [[f32; 2]; 4],
        alpha: u32,
        blend: Blend,
    },
    Solid {
        rect: [i64; 4],
        color: [u8; 3],
        alpha: u32,
        blend: Blend,
    },
    Fade(&'a crate::fade::Fade),
}

/// Device blend state chosen by 0x140020340 through 0x14001bc20.
#[derive(Clone, Copy)]
enum Blend {
    /// Device mode 0: blending disabled, the texel color replaces the target.
    Replace,
    /// Device mode 1: SRCALPHA / INVSRCALPHA.
    Alpha,
    /// Device mode 2: SRCALPHA / ONE, saturated by the 8-bit target.
    Additive,
    BlackReplace,
    BlackAlpha,
    BlackAdditive,
}

impl Blend {
    fn subtract_white(self) -> Self {
        match self {
            Self::Replace | Self::BlackReplace => Self::BlackReplace,
            Self::Alpha | Self::BlackAlpha => Self::BlackAlpha,
            Self::Additive | Self::BlackAdditive => Self::BlackAdditive,
        }
    }
    fn alpha(self) -> Self {
        match self {
            Self::BlackReplace | Self::BlackAlpha | Self::BlackAdditive => Self::BlackAlpha,
            _ => Self::Alpha,
        }
    }
}

/// The verified textured paths supply white diffuse RGB (0x1400238a0). Stage 0
/// COLORARG1=TEXTURE, COLORARG2=DIFFUSE, so SUBTRACT saturates RGB to black while
/// leaving the independent alpha operation intact. Solid primitives use no texture.
fn texture_operation(mut layer: Layer<'_>, enabled: bool) -> Layer<'_> {
    if enabled {
        match &mut layer {
            // A transition's pixel shader (0x140028af0) replaces fixed-function
            // texture stages, so COLOROP has no effect while that shader is bound.
            Layer::Image {
                blend, mask: None, ..
            }
            | Layer::Projected { blend, .. }
            | Layer::Rotated { blend, .. } => *blend = blend.subtract_white(),
            _ => {}
        }
    }
    layer
}

/// Draw word 1 (0x204b): 1 alpha, 2 additive. 0x140020340 falls back to device mode 0 when
/// the image has no alpha channel (image +1152) and the effective opacity is at least 255.
/// Modes 0 and 3 (INVDESTCOLOR / ONE) are unused by this title and rejected by `check`.
/// A masked object keeps the verified alpha path; other modes with a mask are not verified.
fn blend(
    words: &[u32; 22],
    image: &ObjectImage,
    alpha: i64,
    masked: bool,
) -> Result<Blend, FormatError> {
    if masked {
        if words[1] != 1 {
            return Err(FormatError(
                "mask transition on an additive object is not verified",
            ));
        }
        return Ok(Blend::Alpha);
    }
    Ok(if !image.alpha && alpha >= 255 {
        Blend::Replace
    } else if words[1] == 2 {
        Blend::Additive
    } else {
        Blend::Alpha
    })
}

/// A main queue entry and the children 0x14003de80 attaches to its sublist (flag bit 1).
struct Entry<'a> {
    order: f32,
    layer: Option<Layer<'a>>,
    children: Vec<(f32, Layer<'a>)>,
}

/// Only blend modes 1/2, crop, origin, position, order and opacity may differ from the
/// constructor values; a projected object (coordinate mode 2/3) also keeps its world XYZ
/// (words 11..14) and projection inputs (15/16). Both paths accept finite nonnegative
/// XY scale and a rotation (word 20); fade factors
/// in 0..256 are used by image replacement. A 2D object's color word also selects
/// the solid rectangle color and whether its children inherit the parent's image.
fn check(words: &[u32; 22], projected: bool) -> Result<(), FormatError> {
    let mut allowed = DrawState::default().words;
    if matches!(words[1], 1 | 2) {
        allowed[1] = words[1];
    }
    allowed[2..11].copy_from_slice(&words[2..11]);
    if projected {
        allowed[0] = words[0];
        allowed[11..14].copy_from_slice(&words[11..14]);
        allowed[15..17].copy_from_slice(&words[15..17]);
    }
    for i in 18..20 {
        let scale = f32::from_bits(words[i]);
        if scale.is_finite() && scale >= 0.0 {
            allowed[i] = words[i];
        }
    }
    if !projected {
        allowed[14] = words[14];
    }
    allowed[17] = words[17];
    allowed[20] = words[20];
    if words[21] <= 256 {
        allowed[21] = words[21];
    }
    if *words != allowed {
        return Err(FormatError("unsupported scene transform or blend mode"));
    }
    Ok(())
}

/// Order key as inserted by 0x14003ddd0: the layer word as i32, converted to f32.
fn order(words: &[u32; 22]) -> f32 {
    words[10] as i32 as f32
}

/// Builds an image layer, or None when the object draws nothing (empty rect, zero alpha).
fn image_layer<'a>(
    image: &'a ObjectImage,
    words: &[u32; 22],
    position: [i64; 2],
    alpha: i64,
    mask: Option<&'a MaskTransition>,
    inherited: Option<([f32; 2], [f32; 2])>,
) -> Result<Option<Layer<'a>>, FormatError> {
    let [sx, sy, w, h] = [2, 3, 4, 5].map(|i| i64::from(words[i] as i16));
    if w <= 0 || h <= 0 || alpha <= 0 {
        return Ok(None);
    }
    let size = [image.pixels.width(), image.pixels.height()].map(i64::from);
    if sx < 0 || sy < 0 || sx + w > size[0] || sy + h > size[1] {
        return Err(FormatError("scene source rectangle outside image"));
    }
    // The mask is the object's second texture; only the case where it covers the whole,
    // equally sized image texel for texel is modeled.
    if let Some(mask) = mask {
        let mask_size = [mask.mask.pixels.width(), mask.mask.pixels.height()].map(i64::from);
        if [sx, sy, w, h] != [0, 0, size[0], size[1]] || mask_size != size {
            return Err(FormatError(
                "mask transition on a cropped or differently sized image is not verified",
            ));
        }
    }
    let scale = [words[18], words[19]].map(f32::from_bits);
    // 0x140022500 divides the signed hundredths by 100.0f; only a zero angle skips rotation.
    let degrees = words[20] as i32 as f32 / 100.0;
    if degrees != 0.0 {
        if mask.is_some() {
            return Err(FormatError("rotated 2D mask rendering is not verified"));
        }
        let (position, scale) = inherited.unwrap_or((position.map(|v| v as f32), scale));
        if scale.contains(&0.0) {
            return Ok(None);
        }
        let origin = [words[6] as i16, words[7] as i16].map(i64::from);
        let corners = rotated_corners(position, scale, origin, [w, h], degrees);
        if !corners.as_flattened().iter().all(|v| v.is_finite()) {
            return Err(FormatError("rotated 2D image extent is not finite"));
        }
        return Ok(Some(Layer::Rotated {
            image,
            rect: [sx, sy, w, h],
            corners,
            alpha: alpha.min(255) as u32,
            blend: blend(words, image, alpha, false)?,
        }));
    }
    if inherited.is_some() || scale != [1.0; 2] {
        if mask.is_some() {
            return Err(FormatError("scaled 2D mask rendering is not verified"));
        }
        let (position, scale) = inherited.unwrap_or((position.map(|v| v as f32), scale));
        if scale.contains(&0.0) {
            return Ok(None);
        }
        let origin = [words[6] as i16, words[7] as i16].map(f32::from);
        let quad = [
            position[0] - origin[0] * scale[0] - 0.5,
            position[1] - origin[1] * scale[1] - 0.5,
            position[0] + (w as f32 - origin[0]) * scale[0] - 0.5,
            position[1] + (h as f32 - origin[1]) * scale[1] - 0.5,
        ];
        if !quad.iter().all(|v| v.is_finite()) {
            return Err(FormatError("scaled 2D image extent is not finite"));
        }
        return Ok(Some(Layer::Projected {
            image,
            rect: [sx, sy, w, h],
            quad,
            alpha: alpha.min(255) as u32,
            blend: blend(words, image, alpha, false)?,
        }));
    }
    Ok(Some(Layer::Image {
        image,
        rect: [sx, sy, w, h],
        corner: [
            position[0] - i64::from(words[6] as i16),
            position[1] - i64::from(words[7] as i16),
        ],
        alpha: alpha.min(255) as u32,
        mask,
        screen_mask: None,
        blend: blend(words, image, alpha, mask.is_some())?,
    }))
}

/// `cvttss2si`: out-of-range and NaN inputs give the integer indefinite value.
fn truncate(v: f32) -> i32 {
    if v.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&v) {
        i32::MIN
    } else {
        v as i32
    }
}

/// The single-texture rotated branch of 0x140022500 (`0x14002263b..0x140022a52`): the
/// rotated local corners move by the position and the device's half-pixel offset (device
/// factor 1). Rotation is about the object's own position.
fn rotated_corners(
    position: [f32; 2],
    scale: [f32; 2],
    origin: [i64; 2],
    size: [i64; 2],
    degrees: f32,
) -> [[f32; 2]; 4] {
    rotated_local(scale, origin, size, degrees)
        .map(|[x, y]| [x + position[0] - 0.5, y + position[1] - 0.5])
}

/// Local corners top-left, top-right, bottom-left, bottom-right: the scaled offsets from
/// the origin, in single precision. With θ = (360 − degrees) · 0.017453292 they become
/// x' = s·y + c·x and y' = c·y − x·s, each truncated toward zero to hundredths. Shared by
/// 0x140022500 (2D) and 0x140024020 (`0x1400240b2..0x140024223`, projected local layout).
/// The binary's static CRT cosf/sinf are approximated by rounding f64 results.
fn rotated_local(
    scale: [f32; 2],
    origin: [i64; 2],
    [w, h]: [i64; 2],
    degrees: f32,
) -> [[f32; 2]; 4] {
    let [left, right] = [-origin[0], w - origin[0]].map(|v| v as f32 * scale[0]);
    let [top, bottom] = [-origin[1], h - origin[1]].map(|v| v as f32 * scale[1]);
    let theta = (360.0 - degrees) * 0.017_453_292;
    let (sin, cos) = (f64::from(theta).sin() as f32, f64::from(theta).cos() as f32);
    let hundredths = |v: f32| truncate(v * 100.0) as f32 / 100.0;
    [[left, top], [right, top], [left, bottom], [right, bottom]]
        .map(|[x, y]| [hundredths(sin * y + cos * x), hundredths(cos * y - x * sin)])
}

/// Untextured 2D branch of 0x140020340 / 0x1400233e0. The color's alpha byte
/// selects child image inheritance, but vertex alpha comes from opacity and fade.
fn solid_layer<'a>(
    words: &[u32; 22],
    position: [i64; 2],
    alpha: i64,
) -> Result<Option<Layer<'a>>, FormatError> {
    let [w, h] = [4, 5].map(|i| i64::from(words[i] as i16));
    if w <= 0 || h <= 0 || alpha <= 0 {
        return Ok(None);
    }
    if words[18..20] != [1.0f32.to_bits(); 2] {
        return Err(FormatError(
            "scaled solid rectangle rendering is not verified",
        ));
    }
    if words[20] != 0 {
        return Err(FormatError(
            "rotated solid rectangle rendering is not verified",
        ));
    }
    let [b, g, r, _] = words[14].to_le_bytes();
    Ok(Some(Layer::Solid {
        rect: [
            position[0] - i64::from(words[6] as i16),
            position[1] - i64::from(words[7] as i16),
            w,
            h,
        ],
        color: [r, g, b],
        alpha: alpha.min(255) as u32,
        blend: if alpha >= 255 {
            Blend::Replace
        } else if words[1] == 2 {
            Blend::Additive
        } else {
            Blend::Alpha
        },
    }))
}

/// 0x14003b420/0x140020340 for a child of a top-level object. Without its own image the
/// child uses the parent's only when its +56 color has a zero alpha byte.
/// Flag bit 0 inherits the parent's position, scale and opacity.
fn child_layer<'a>(
    parent: &'a SceneObject,
    parent_words: &[u32; 22],
    child: &'a SceneObject,
    words: &[u32; 22],
) -> Result<Option<Layer<'a>>, FormatError> {
    let image = child.image.as_ref().or_else(|| {
        parent
            .image
            .as_ref()
            .filter(|_| words[14] & 0xff00_0000 == 0)
    });
    let own = [words[8] as i32, words[9] as i32].map(i64::from);
    let (position, alpha) = if child.flags & 1 != 0 {
        let (opacity, parent_opacity) = (words[17] as i32, parent_words[17] as i32);
        if opacity < 0 || parent_opacity < 0 {
            return Err(FormatError(
                "negative opacity with parent inheritance is not verified",
            ));
        }
        let base = [parent_words[8] as i32, parent_words[9] as i32].map(i64::from);
        // Fade factors are the constructor's 256, so (256 * 256 * (c * p / 255)) >> 16.
        (
            [base[0] + own[0], base[1] + own[1]],
            i64::from(
                (words[21]
                    .wrapping_mul(parent_words[21])
                    .wrapping_mul((opacity * parent_opacity / 255) as u32) as i32)
                    >> 16,
            ),
        )
    } else {
        (
            own,
            i64::from((words[17].wrapping_mul(words[21]) as i32) >> 8),
        )
    };
    // 0x140020d75..0x140020dcc: parent X scale applies to both child position
    // components; local corners use the product of the corresponding XY scales.
    let inherited = if child.flags & 1 != 0 && parent_words[18..20] != [1.0f32.to_bits(); 2] {
        let parent_scale = [parent_words[18], parent_words[19]].map(f32::from_bits);
        Some((
            [0, 1].map(|i| own[i] as f32 * parent_scale[0] + parent_words[8 + i] as i32 as f32),
            [0, 1].map(|i| f32::from_bits(words[18 + i]) * parent_scale[i]),
        ))
    } else {
        None
    };
    match image {
        Some(image) => image_layer(image, words, position, alpha, None, inherited),
        None if inherited.is_some() && words[4] as i16 > 0 && words[5] as i16 > 0 => {
            Err(FormatError("scaled solid child rendering is not verified"))
        }
        None => solid_layer(words, position, alpha),
    }
}

/// 0x14003b350 scales drawing by the device width (+24) over the logical width (+6256), or
/// over the screen width (+8) while that is zero. Both device widths are the research
/// viewport, so only another logical width would change the factor; that is not modeled.
fn unit_screen_scale(object: &SceneObject) -> Result<(), FormatError> {
    match object.logical_size[0] {
        0 => Ok(()),
        width if width == RESEARCH_SCREEN[0] => Ok(()),
        _ => Err(FormatError("logical screen scaling is not modeled")),
    }
}

/// Queues 2/6 of 0x14003de80/0x140015fc0 for a top-level mode 2/3 object, keyed by its
/// biased depth (`Camera::queue_key`); objects with keys not above 0 are not enqueued.
/// 0x140020340 projects it with camera 0/1 (0x14004ef40), lays out the
/// source rectangle around the origin (0x140024020) and places it at the projected position
/// and scale (0x1400238a0, device factor 1 on the 1280x720 viewport) minus the device's
/// half-pixel offset. The object's own angle rotates its local corners before projection
/// scaling. Inherited attached children reuse the parent projection and origin; masks,
/// camera rotation, rotated parents of children and effect targets are not verified.
fn projected_layer<'a>(
    scene: &SceneHost,
    cameras: Option<&CameraHost>,
    slot: Option<u8>,
    object: &'a SceneObject,
    draw: &DrawState,
    parent: Option<&'a SceneObject>,
) -> Result<Option<(f32, Layer<'a>)>, FormatError> {
    if object.secondary_image.is_some() {
        return Err(FormatError(
            "projected second texture rendering is not verified",
        ));
    }
    let cameras = cameras.ok_or(FormatError("camera projection needs the camera state"))?;
    let words = &draw.words;
    let camera = cameras
        .camera((words[0] - 2) as usize)
        .expect("camera exists");
    let inherited_draw = parent
        .filter(|_| object.flags & 1 != 0)
        .and_then(|p| p.draw.as_ref());
    let key_draw = inherited_draw.unwrap_or(draw);
    let key = camera.queue_key(
        f32::from_bits(key_draw.words[13]),
        f32::from_bits(key_draw.depth_bias),
    );
    if key.is_nan() || key <= 0.0 {
        return Ok(None);
    }
    check(words, true)?;
    if slot.is_some_and(|slot| scene.mask_on(slot).is_some()) {
        return Err(FormatError(
            "mask transition on a projected object is not verified",
        ));
    }
    if words[0] == 3 && !camera.enabled() {
        return Ok(None);
    }
    if camera.effect() != 0 {
        return Err(FormatError(
            "camera effect targets (0x209c) are not modeled",
        ));
    }
    // +48, +52 (also a horizontal shift) and +56 (screen roll) feed rotations.
    if camera.words[12..15].iter().any(|&word| word != 0) {
        return Err(FormatError("camera rotation is not modeled"));
    }
    let inherited = parent
        .filter(|_| object.flags & 1 != 0)
        .and_then(|p| p.draw.as_ref())
        .map(|d| &d.words);
    let image = object.image.as_ref().or_else(|| {
        parent
            .and_then(|p| p.image.as_ref())
            .filter(|_| words[14] & 0xff00_0000 == 0)
    });
    let Some(image) = image else {
        return Ok(None);
    };
    let projection_words = inherited.unwrap_or(words);
    let world = [11, 12, 13].map(|i| f32::from_bits(projection_words[i]));
    let Some(projection) = camera
        .project_object(
            world,
            f32::from_bits(projection_words[16]),
            f32::from_bits(projection_words[15]),
        )
        .map_err(FormatError)?
    else {
        return Ok(None);
    };
    // Opacity times the constructor fade factor 256, >> 8; then the depth fade, /255.
    let opacity = if let Some(p) = inherited {
        let (own, parent) = (words[17] as i32, p[17] as i32);
        if own < 0 || parent < 0 {
            return Err(FormatError(
                "negative opacity with parent inheritance is not verified",
            ));
        }
        (words[21]
            .wrapping_mul(p[21])
            .wrapping_mul((own.wrapping_mul(parent) / 255) as u32) as i32)
            >> 16
    } else {
        (words[17].wrapping_mul(words[21]) as i32) >> 8
    };
    let alpha = projection.fade.wrapping_mul(opacity) / 255;
    let [sx, sy, w, h] = [2, 3, 4, 5].map(|i| i64::from(words[i] as i16));
    // 0x140020340 skips v249/v250 <= 0.0; a NaN scale reaches the extent check below.
    if alpha <= 0 || projection.scale <= 0.0 || w <= 0 || h <= 0 {
        return Ok(None);
    }
    let size = [image.pixels.width(), image.pixels.height()].map(i64::from);
    if sx < 0 || sy < 0 || sx + w > size[0] || sy + h > size[1] {
        return Err(FormatError("scene source rectangle outside image"));
    }
    let [px, py] = projection.position;
    // 0x140024020 scales the child's local corners about its own origin first.
    // 0x140020708 and 0x1400238a0 then subtract the parent origin/child position
    // offset and apply the projection scale (which includes the parent's XY scale).
    let scale =
        [0, 1].map(|i| projection.scale * inherited.map_or(1.0, |p| f32::from_bits(p[18 + i])));
    if scale.iter().any(|v| *v <= 0.0) {
        return Ok(None);
    }
    let offset = [0, 1].map(|i| {
        if let Some(p) = inherited {
            // 0x140020708/0x14002070c: signed 16-bit parent origin minus child position.
            f32::from((p[6 + i] as i16).wrapping_sub(words[8 + i] as i16))
        } else {
            0.0
        }
    });
    let origin = [0, 1].map(|i| 0.0 - f32::from(words[6 + i] as i16));
    let local = [0, 1].map(|i| f32::from_bits(words[18 + i]));
    // 0x140024020 rotates the scaled local corners by the object's own angle; 0x1400238a0
    // then subtracts the offset, applies the projection scale, rotates by its +52 argument
    // (the parent's angle for an inherited child, 0 otherwise) and adds the position.
    if inherited.is_some_and(|p| p[20] != 0) {
        return Err(FormatError(
            "children of a rotated projected parent are not verified",
        ));
    }
    let degrees = words[20] as i32 as f32 / 100.0;
    if degrees != 0.0 {
        let origin = [words[6] as i16, words[7] as i16].map(i64::from);
        let corners = rotated_local(local, origin, [w, h], degrees)
            .map(|c| [0, 1].map(|i| ((c[i] - offset[i]) * scale[i] + [px, py][i]) - 0.5));
        if !corners.as_flattened().iter().all(|v| v.is_finite()) {
            return Err(FormatError("projected object has a non-finite extent"));
        }
        let blend = blend(words, image, i64::from(alpha), false)?;
        let blend = if draw.texture_subtract != 0 {
            blend.subtract_white()
        } else {
            blend
        };
        return Ok(Some((
            key,
            Layer::Rotated {
                image,
                rect: [sx, sy, w, h],
                corners,
                alpha: alpha.min(255) as u32,
                blend,
            },
        )));
    }
    let quad = [
        ((origin[0] * local[0] - offset[0]) * scale[0] + px) - 0.5,
        ((origin[1] * local[1] - offset[1]) * scale[1] + py) - 0.5,
        (((w as f32 + origin[0]) * local[0] - offset[0]) * scale[0] + px) - 0.5,
        (((h as f32 + origin[1]) * local[1] - offset[1]) * scale[1] + py) - 0.5,
    ];
    if quad.iter().any(|v| !v.is_finite()) {
        return Err(FormatError("projected object has a non-finite extent"));
    }
    let blend = blend(words, image, i64::from(alpha), false)?;
    let blend = if draw.texture_subtract != 0 {
        blend.subtract_white()
    } else {
        blend
    };
    Ok(Some((
        key,
        Layer::Projected {
            image,
            rect: [sx, sy, w, h],
            quad,
            alpha: alpha.min(255) as u32,
            blend,
        },
    )))
}

/// Moves an image layer by a quake offset.
fn shifted(layer: Layer<'_>, by: [i64; 2]) -> Layer<'_> {
    match layer {
        Layer::Image {
            image,
            rect,
            corner,
            alpha,
            mask,
            screen_mask,
            blend,
        } => Layer::Image {
            image,
            rect,
            corner: [corner[0] + by[0], corner[1] + by[1]],
            alpha,
            mask,
            screen_mask,
            blend,
        },
        Layer::Projected {
            image,
            rect,
            quad,
            alpha,
            blend,
        } => {
            let [x, y] = by.map(|v| v as f32);
            Layer::Projected {
                image,
                rect,
                quad: [quad[0] + x, quad[1] + y, quad[2] + x, quad[3] + y],
                alpha,
                blend,
            }
        }
        Layer::Rotated {
            image,
            rect,
            corners,
            alpha,
            blend,
        } => {
            let [x, y] = by.map(|v| v as f32);
            Layer::Rotated {
                image,
                rect,
                corners: corners.map(|[cx, cy]| [cx + x, cy + y]),
                alpha,
                blend,
            }
        }
        Layer::Solid {
            rect: [x, y, w, h],
            color,
            alpha,
            blend,
        } => Layer::Solid {
            rect: [x + by[0], y + by[1], w, h],
            color,
            alpha,
            blend,
        },
        fade => fade,
    }
}

/// Render in script coordinates. Window aspect fitting is a separate desktop policy.
/// Without camera state, projected objects fail explicitly.
pub fn render(scene: &SceneHost, width: u32, height: u32) -> Result<RgbaImage, FormatError> {
    render_scene(scene, None, &[], width, height)
}

/// `render` with additional objects; see `render_scene`.
pub fn render_with_objects(
    scene: &SceneHost,
    additional: &[&SceneObject],
    width: u32,
    height: u32,
) -> Result<RgbaImage, FormatError> {
    render_scene(scene, None, additional, width, height)
}

/// Additional objects (e.g. a backend's glyph images and message windows) join the
/// same stable layer queue before the fade. Their order breaks ties after slot objects.
/// 0x140015fc0 draws camera 0/1's queues 2/6 before queue 3, which holds the 2D
/// objects, windows, glyphs and the fade, so projected objects lie beneath all of them.
pub fn render_scene(
    scene: &SceneHost,
    cameras: Option<&CameraHost>,
    additional: &[&SceneObject],
    width: u32,
    height: u32,
) -> Result<RgbaImage, FormatError> {
    let size = (u64::from(width) * u64::from(height))
        .checked_mul(4)
        .ok_or(FormatError("scene surface size overflow"))?;
    if width == 0 || height == 0 || size > MAX_PAYLOAD as u64 {
        return Err(FormatError("invalid scene surface dimensions"));
    }
    // Validate the entire scene before rasterizing. Slot order is the stable tie-breaker.
    let mut entries = Vec::new();
    let mut projected = [Vec::new(), Vec::new()];
    let slots = scene
        .objects()
        .iter()
        .map(|(&slot, object)| (Some(slot), object));
    // Default forward queue traversal: common, previous, then current. Equal keys keep
    // this insertion order (0x14003de80 / 0x14003ddd0). Retained objects have no slot mask.
    let mut objects = Vec::new();
    for (slot, object) in slots.chain(additional.iter().map(|&object| (None, object))) {
        if object.shown
            && let Some(change) = &object.change
        {
            objects.push((None, change.common.as_ref(), slot.is_none()));
            objects.push((None, change.previous.as_ref(), slot.is_none()));
        }
        objects.push((slot, object, slot.is_none()));
    }
    for (slot, object, additional_object) in objects {
        // 0x14003de80 returns early for +6244 == 0; no draw state means +6240 == 0.
        let Some(draw) = object.draw.as_ref().filter(|_| object.shown) else {
            continue;
        };
        unit_screen_scale(object)?;
        if object.secondary_image.is_some() && matches!(draw.words[0], 2 | 3) {
            return Err(FormatError(
                "projected second texture rendering is not verified",
            ));
        }
        let d = &draw.words;
        // 0x1400526b0 offsets slot objects as queue layer 0 and the backend's windows and
        // glyphs as layer 1; 0x14003b420 applies it only with the top-level object's +6248
        // bit 0x100, children included.
        let by = match scene.quake() {
            Some(quake) if object.flags & 0x100 != 0 => {
                quake.offset(additional_object).map(i64::from)
            }
            _ => [0, 0],
        };
        match d[0] {
            2 | 3 => {
                let cameras =
                    cameras.ok_or(FormatError("camera projection needs the camera state"))?;
                let camera = cameras.camera((d[0] - 2) as usize).expect("camera exists");
                let key = camera.queue_key(f32::from_bits(d[13]), f32::from_bits(draw.depth_bias));
                if key.is_nan() || key <= 0.0 || d[0] == 3 && !camera.enabled() {
                    continue;
                }
                let layer = projected_layer(scene, Some(cameras), slot, object, draw, None)?
                    .map(|(_, l)| shifted(l, by));
                let mut entry = Entry {
                    order: key,
                    layer,
                    children: Vec::new(),
                };
                if object.flags & 0x8000_0000 != 0 {
                    for child in object.children.values().filter(|c| c.shown) {
                        let child_draw = child
                            .draw
                            .as_ref()
                            .ok_or(FormatError("shown child object has no draw state"))?;
                        if child_draw.words[0] != d[0] {
                            continue;
                        }
                        if child.change.is_some() || child.children.values().any(|c| c.shown) {
                            return Err(FormatError(
                                "nested projected children or child image replacement is not verified",
                            ));
                        }
                        unit_screen_scale(child)?;
                        // Bound this implementation to the observed inherited, attached image path.
                        if child.flags & 3 != 3 {
                            return Err(FormatError(
                                "independent projected children are not verified",
                            ));
                        }
                        let child_depth = f32::from_bits(child_draw.depth_bias)
                            + f32::from_bits(child_draw.words[13]);
                        let child_key = key + child_depth;
                        if !child_key.is_finite() {
                            return Err(FormatError("projected child has a non-finite queue key"));
                        }
                        if let Some((_, layer)) = projected_layer(
                            scene,
                            Some(cameras),
                            None,
                            child,
                            child_draw,
                            Some(object),
                        )? {
                            entry.children.push((child_key, shifted(layer, by)));
                        }
                    }
                }
                projected[(d[0] - 2) as usize].push(entry);
                continue;
            }
            _ => {}
        }
        check(d, false)?;
        let position = [d[8] as i32, d[9] as i32].map(i64::from);
        let mask = slot.and_then(|slot| scene.mask_on(slot));
        let layer = match &object.image {
            Some(image) => image_layer(
                image,
                d,
                position,
                i64::from((d[17].wrapping_mul(d[21]) as i32) >> 8),
                mask,
                None,
            )?,
            None => solid_layer(
                d,
                position,
                i64::from((d[17].wrapping_mul(d[21]) as i32) >> 8),
            )?,
        }
        .map(|layer| texture_operation(layer, draw.texture_subtract != 0))
        .map(|layer| screen_mask(layer, object, None))
        .transpose()?
        .map(|layer| shifted(layer, by));
        let mut entry = Entry {
            order: order(d),
            layer,
            children: Vec::new(),
        };
        let mut independent = Vec::new();
        // Children are visited only once 0x2022 has set the parent's bit 31.
        if object.flags & 0x8000_0000 != 0 {
            for child in object.children.values().filter(|child| child.shown) {
                if child.change.is_some() {
                    return Err(FormatError(
                        "image replacement transition rendering is not implemented",
                    ));
                }
                let words = &child
                    .draw
                    .as_ref()
                    .ok_or(FormatError("shown child object has no draw state"))?
                    .words;
                if words[0] != d[0] || order(words) < 0.0 {
                    continue;
                }
                unit_screen_scale(child)?;
                check(words, false)?;
                let Some(layer) = child_layer(object, d, child, words)? else {
                    continue;
                };
                let layer = texture_operation(
                    layer,
                    child.draw.as_ref().expect("validated").texture_subtract != 0,
                );
                let layer = shifted(screen_mask(layer, child, Some(object))?, by);
                if child.flags & 2 != 0 {
                    entry.children.push((order(words), layer));
                } else {
                    independent.push(Entry {
                        order: order(words),
                        layer: Some(layer),
                        children: Vec::new(),
                    });
                }
            }
        }
        entries.push(entry);
        entries.extend(independent);
    }
    // Attached strips follow ordinary slots, then the fade (0x140015da0).
    if let Some(wipe) = scene.strip_wipe() {
        for object in std::iter::once(&wipe.object).chain(wipe.strips.iter().map(|s| &s.object)) {
            if !object.shown {
                continue;
            }
            let Some(draw) = &object.draw else {
                continue;
            };
            let d = &draw.words;
            let mut plain = *d;
            plain[14] = 0;
            check(&plain, false)?;
            if d[0] != 0 || d[1] != 1 || d[18..20] != [1.0f32.to_bits(); 2] || d[20] != 0 {
                return Err(FormatError("unsupported strip transform or blend mode"));
            }
            let [w, h] = [d[4] as i16, d[5] as i16].map(i64::from);
            if w <= 0 || h <= 0 {
                continue;
            }
            let [left, top] = [8, 9].map(|i| i64::from(d[i] as i32));
            let [b, g, r, a] = d[14].to_le_bytes();
            entries.push(Entry {
                order: order(d),
                layer: Some(Layer::Solid {
                    rect: [left, top, w, h],
                    color: [r, g, b],
                    alpha: u32::from(a),
                    blend: Blend::Alpha,
                }),
                children: Vec::new(),
            });
        }
    }
    // The engine enqueues the fade after ordinary object slots (0x140015da0).
    if let Some(fade) = scene.fade().current() {
        // The overlay is a layer 0 queue item; its translated extent is not verified.
        if scene
            .quake()
            .is_some_and(|quake| quake.offset(false) != [0, 0])
        {
            return Err(FormatError(
                "fade overlay under a quake offset is not verified",
            ));
        }
        entries.push(Entry {
            order: fade.order as f32,
            layer: Some(Layer::Fade(fade)),
            children: Vec::new(),
        });
    }
    // 0x14003ddd0 inserts after equal keys, with larger orders first; the traversal
    // 0x14003dc40 drains an entry's sublist right after the entry itself.
    entries.sort_by(|a, b| b.order.total_cmp(&a.order));
    for queue in &mut projected {
        queue.sort_by(|a, b| b.order.total_cmp(&a.order));
    }
    let mut pixels = vec![0; size as usize];
    for p in pixels.chunks_exact_mut(4) {
        p.copy_from_slice(&scene.clear().rgba());
    }
    for mut entry in projected.into_iter().flatten().chain(entries) {
        entry.children.sort_by(|a, b| b.0.total_cmp(&a.0));
        let layers = entry
            .layer
            .into_iter()
            .chain(entry.children.into_iter().map(|(_, layer)| layer));
        for layer in layers {
            draw(&mut pixels, width, height, layer);
        }
    }
    RgbaImage::new(width, height, pixels)
}

/// One target pixel under the device blend state, with straight 8-bit source values.
fn put(target: &mut [u8], source: [u8; 3], alpha: u32, blend: Blend) {
    let source = if matches!(
        blend,
        Blend::BlackReplace | Blend::BlackAlpha | Blend::BlackAdditive
    ) {
        [0; 3]
    } else {
        source
    };
    for c in 0..3 {
        let (s, t) = (u32::from(source[c]), u32::from(target[c]));
        target[c] = match blend {
            Blend::Replace | Blend::BlackReplace => source[c],
            Blend::Alpha | Blend::BlackAlpha => ((s * alpha + t * (255 - alpha) + 127) / 255) as u8,
            Blend::Additive | Blend::BlackAdditive => (t + (s * alpha + 127) / 255).min(255) as u8,
        };
    }
}

/// Bilinear sampling (cmvs.cfg TEXTURE_MIN/MAG_FILTER = 1 selects D3DTEXF_LINEAR in
/// 0x1400220e0) with clamped addressing over the whole image, modulated by the vertex alpha.
/// Pixels whose centers lie in [left, right) × [top, bottom) are covered. Texture tiling
/// seams, power-of-two padding and the device's sub-texel precision are not modeled, so
/// scaled output approximates the original; at unit scale on whole pixels it copies texels.
fn draw_projected(
    pixels: &mut [u8],
    [width, height]: [u32; 2],
    image: &ObjectImage,
    [sx, sy, w, h]: [i64; 4],
    [left, top, right, bottom]: [f32; 4],
    opacity: u32,
    blend: Blend,
) {
    let step = [w as f32 / (right - left), h as f32 / (bottom - top)];
    let span = |from: f32, to: f32, limit: u32| {
        let limit = limit as f32;
        (
            from.ceil().clamp(0.0, limit) as i64,
            to.ceil().clamp(0.0, limit) as i64,
        )
    };
    let (x0, x1) = span(left, right, width);
    let (y0, y1) = span(top, bottom, height);
    for y in y0..y1 {
        let ty = sy as f32 + (y as f32 - top) * step[1] - 0.5;
        for x in x0..x1 {
            let tx = sx as f32 + (x as f32 - left) * step[0] - 0.5;
            let texel = bilinear(image, tx, ty);
            let alpha = (u32::from(texel[3]) * opacity + 127) / 255;
            let dst = ((y * i64::from(width) + x) * 4) as usize;
            put(
                &mut pixels[dst..dst + 3],
                [texel[0], texel[1], texel[2]],
                alpha,
                blend,
            );
        }
    }
}

/// Clamped bilinear sample at texel-space coordinates whose texel centers are integers.
fn bilinear(image: &ObjectImage, tx: f32, ty: f32) -> [u8; 4] {
    let size = [image.pixels.width(), image.pixels.height()].map(i64::from);
    let texels = image.pixels.pixels();
    let [wx, wy] = [tx - tx.floor(), ty - ty.floor()];
    let rows = [ty.floor() as i64, ty.floor() as i64 + 1].map(|r| r.clamp(0, size[1] - 1));
    let cols = [tx.floor() as i64, tx.floor() as i64 + 1].map(|c| c.clamp(0, size[0] - 1));
    let texel =
        |row: i64, col: i64, c: usize| f32::from(texels[((row * size[0] + col) * 4) as usize + c]);
    std::array::from_fn(|c| {
        let upper = texel(rows[0], cols[0], c) * (1.0 - wx) + texel(rows[0], cols[1], c) * wx;
        let lower = texel(rows[1], cols[0], c) * (1.0 - wx) + texel(rows[1], cols[1], c) * wx;
        (upper * (1.0 - wy) + lower * wy).round().clamp(0.0, 255.0) as u8
    })
}

/// A rotated quad as the strip triangles (0, 1, 2) and (1, 3, 2) with affine texture
/// coordinates and bilinear sampling. Pixel centers on an edge follow a top-left rule, so
/// the shared diagonal is covered once and an axis-aligned quad covers the same pixels as
/// `draw_projected`. Device rasterization precision is not modeled.
fn draw_rotated(
    pixels: &mut [u8],
    [width, height]: [u32; 2],
    image: &ObjectImage,
    [sx, sy, w, h]: [i64; 4],
    corners: [[f32; 2]; 4],
    opacity: u32,
    blend: Blend,
) {
    let [u0, v0, u1, v1] = [sx, sy, sx + w, sy + h].map(|v| v as f64);
    let texture = [[u0, v0], [u1, v0], [u0, v1], [u1, v1]];
    let point = corners.map(|[x, y]| [f64::from(x), f64::from(y)]);
    for triangle in [[0, 1, 2], [1, 3, 2]] {
        let mut p = triangle.map(|i| point[i]);
        let mut t = triangle.map(|i| texture[i]);
        let edge = |a: [f64; 2], b: [f64; 2], q: [f64; 2]| {
            (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0])
        };
        let mut area = edge(p[0], p[1], p[2]);
        if area == 0.0 {
            continue;
        }
        if area < 0.0 {
            p.swap(1, 2);
            t.swap(1, 2);
            area = -area;
        }
        // With y down and a positive area, top edges point right and left edges point up.
        let top_left = |a: [f64; 2], b: [f64; 2]| b[1] < a[1] || (b[1] == a[1] && b[0] > a[0]);
        let edges = [(1, 2), (2, 0), (0, 1)];
        let bound = |axis: usize, limit: u32| {
            let values = p.map(|v| v[axis]);
            let low = values.iter().copied().fold(f64::INFINITY, f64::min);
            let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            (
                low.floor().clamp(0.0, f64::from(limit)) as i64,
                (high.ceil() + 1.0).clamp(0.0, f64::from(limit)) as i64,
            )
        };
        let (x0, x1) = bound(0, width);
        let (y0, y1) = bound(1, height);
        for y in y0..y1 {
            for x in x0..x1 {
                let q = [x as f64, y as f64];
                let weights = edges.map(|(a, b)| edge(p[a], p[b], q));
                let inside = edges
                    .iter()
                    .zip(weights)
                    .all(|(&(a, b), e)| e > 0.0 || (e == 0.0 && top_left(p[a], p[b])));
                if !inside {
                    continue;
                }
                // weights[i] is opposite vertex i.
                let [tx, ty] = [0, 1].map(|axis| {
                    let sum: f64 = (0..3).map(|i| weights[i] * t[i][axis]).sum();
                    (sum / area) as f32 - 0.5
                });
                let texel = bilinear(image, tx, ty);
                let alpha = (u32::from(texel[3]) * opacity + 127) / 255;
                let dst = ((y * i64::from(width) + x) * 4) as usize;
                put(
                    &mut pixels[dst..dst + 3],
                    [texel[0], texel[1], texel[2]],
                    alpha,
                    blend,
                );
            }
        }
    }
}

/// 0x140020340 mode 4 maps the second texture over the device viewport.
/// 0x140030a70 copies its green channel into texture alpha; RGB stays primary.
fn screen_mask<'a>(
    mut layer: Layer<'a>,
    object: &'a SceneObject,
    parent: Option<&'a SceneObject>,
) -> Result<Layer<'a>, FormatError> {
    let inherited = parent.filter(|_| object.flags & 1 != 0);
    let draw = inherited
        .and_then(|p| p.draw.as_ref())
        .or(object.draw.as_ref());
    let texture = object.secondary_image.as_ref().or_else(|| {
        inherited
            .filter(|_| {
                object
                    .draw
                    .as_ref()
                    .is_some_and(|d| d.words[14] & 0xff000000 == 0)
                    && draw.is_some_and(|d| d.texture_mode == 4)
            })
            .and_then(|p| p.secondary_image.as_ref())
    });
    if let Some(texture) = texture {
        if draw.is_none_or(|d| d.texture_mode != 4) {
            return Err(FormatError("second texture mapping mode is not verified"));
        }
        match &mut layer {
            Layer::Image {
                screen_mask,
                mask,
                blend,
                ..
            } if mask.is_none() => {
                *screen_mask = Some(texture);
                *blend = blend.alpha();
            }
            _ => {
                return Err(FormatError(
                    "second texture on transformed or masked object is not verified",
                ));
            }
        }
    }
    Ok(layer)
}

fn draw(pixels: &mut [u8], width: u32, height: u32, layer: Layer<'_>) {
    let (image, [sx, sy, w, h], [left, top], opacity, mask, screen_mask, blend) = match layer {
        Layer::Image {
            image,
            rect,
            corner,
            alpha,
            mask,
            screen_mask,
            blend,
        } => (image, rect, corner, alpha, mask, screen_mask, blend),
        Layer::Projected {
            image,
            rect,
            quad,
            alpha,
            blend,
        } => {
            draw_projected(pixels, [width, height], image, rect, quad, alpha, blend);
            return;
        }
        Layer::Rotated {
            image,
            rect,
            corners,
            alpha,
            blend,
        } => {
            draw_rotated(pixels, [width, height], image, rect, corners, alpha, blend);
            return;
        }
        Layer::Solid {
            rect: [left, top, w, h],
            color,
            alpha,
            blend,
        } => {
            for y in top.max(0)..(top + h).min(i64::from(height)) {
                for x in left.max(0)..(left + w).min(i64::from(width)) {
                    let dst = ((y * i64::from(width) + x) * 4) as usize;
                    put(&mut pixels[dst..dst + 3], color, alpha, blend);
                }
            }
            return;
        }
        Layer::Fade(fade) => {
            let [b, g, r, _] = fade.color.to_le_bytes();
            let alpha = fade.opacity.clamp(0, 255) as u32;
            for p in pixels.chunks_exact_mut(4) {
                for (c, source) in [r, g, b].iter().enumerate() {
                    p[c] = ((u32::from(*source) * alpha + u32::from(p[c]) * (255 - alpha) + 127)
                        / 255) as u8;
                }
            }
            return;
        }
    };
    // Shader factor per mask green byte; the float result is rounded into the 8-bit alpha,
    // so pixels approximate rather than match the device's blending.
    let shade = mask.map(|mask| {
        let coverage: [f32; 256] = std::array::from_fn(|green| mask.coverage(green as u8));
        (mask.mask.pixels.pixels(), coverage)
    });
    for y in top.max(0)..(top + h).min(i64::from(height)) {
        for x in left.max(0)..(left + w).min(i64::from(width)) {
            let src =
                (((sy + y - top) * i64::from(image.pixels.width()) + sx + x - left) * 4) as usize;
            let dst = ((y * i64::from(width) + x) * 4) as usize;
            let p = &image.pixels.pixels()[src..src + 4];
            let mut alpha = (u32::from(p[3]) * opacity + 127) / 255;
            if let Some((texture, coverage)) = &shade {
                // Same size and coordinates as the image, so the same texel.
                alpha = (alpha as f32 * coverage[usize::from(texture[src + 1])]).round() as u32;
            }
            if let Some(texture) = screen_mask {
                let tx = x as f32 * texture.pixels.width() as f32 / width as f32 - 0.5;
                let ty = y as f32 * texture.pixels.height() as f32 / height as f32 - 0.5;
                alpha = (alpha * u32::from(bilinear(texture, tx, ty)[1]) + 127) / 255;
            }
            put(&mut pixels[dst..dst + 3], [p[0], p[1], p[2]], alpha, blend);
        }
    }
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    fn image() -> ObjectImage {
        ObjectImage {
            name: Vec::new(),
            filter: None,
            alpha: true,
            pixels: RgbaImage::new(2, 2, [255, 0, 0, 255].repeat(4)).unwrap(),
            surface: false,
        }
    }

    #[test]
    fn scaled_root_uses_origin_and_half_pixel_quad_and_zero_scale_draws_nothing() {
        let image = image();
        let mut words = DrawState::default().words;
        words[4..8].copy_from_slice(&[2, 2, 1, 1]);
        words[18..20].copy_from_slice(&[1.5f32.to_bits(), 2.0f32.to_bits()]);
        let layer = image_layer(&image, &words, [4, 5], 255, None, None)
            .unwrap()
            .unwrap();
        let Layer::Projected { quad, .. } = &layer else {
            panic!("scaled image quad")
        };
        assert_eq!(*quad, [2.0, 2.5, 5.0, 6.5]);
        let mut pixels = vec![0; 8 * 8 * 4];
        draw(&mut pixels, 8, 8, layer);
        assert_eq!(&pixels[(3 * 8 + 2) * 4..(3 * 8 + 2) * 4 + 3], &[255, 0, 0]);
        assert_eq!(&pixels[(3 * 8 + 5) * 4..(3 * 8 + 5) * 4 + 3], &[0, 0, 0]);
        words[18] = 0;
        assert!(
            image_layer(&image, &words, [4, 5], 255, None, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn inherited_scale_keeps_fractional_child_positions_and_axis_products() {
        let mut p = DrawState::default().words;
        p[8..10].copy_from_slice(&[10, 20]);
        p[18..20].copy_from_slice(&[1.5f32.to_bits(), 2.0f32.to_bits()]);
        let mut c = DrawState::default().words;
        c[4..10].copy_from_slice(&[2, 2, 0, 0, 1, 3]);
        c[18] = 2.0f32.to_bits();
        let child = SceneObject {
            image: Some(image()),
            ..SceneObject::default()
        };
        let parent = SceneObject::default();
        let Layer::Projected { quad, .. } = child_layer(&parent, &p, &child, &c).unwrap().unwrap()
        else {
            panic!("inherited quad")
        };
        // Both positions use parent's X scale; each extent uses its own axis product.
        assert_eq!(quad, [11.0, 24.0, 17.0, 28.0]);
    }

    #[test]
    fn scale_validation_preserves_unverified_solid_and_rotation_barriers() {
        let mut words = DrawState::default().words;
        for scale in [-1.0, f32::NAN, f32::INFINITY] {
            words[18] = scale.to_bits();
            assert!(check(&words, false).is_err());
        }
        words[18] = 1.5f32.to_bits();
        words[4..6].fill(2);
        assert!(check(&words, false).is_ok());
        assert!(solid_layer(&words, [0, 0], 255).is_err());
        words[20] = 1;
        assert!(check(&words, false).is_ok());
        assert!(check(&words, true).is_ok());
        words[18] = 1.0f32.to_bits();
        assert!(solid_layer(&words, [0, 0], 255).is_err());
    }

    #[test]
    fn rotation_truncates_corners_to_hundredths_and_samples_the_rotated_quad() {
        // Red then green, rotated 90 degrees about the position (4, 5) with origin (0, 0).
        let image = ObjectImage {
            pixels: RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap(),
            ..image()
        };
        let mut words = DrawState::default().words;
        words[4..6].copy_from_slice(&[2, 1]);
        words[20] = 9000;
        let layer = image_layer(&image, &words, [4, 5], 255, None, None)
            .unwrap()
            .unwrap();
        let Layer::Rotated { corners, .. } = &layer else {
            panic!("rotated quad")
        };
        // cos(270°) ≈ -4.4e-8 truncates away; +x maps to +y on the y-down screen.
        assert_eq!(*corners, [[3.5, 4.5], [3.5, 6.5], [2.5, 4.5], [2.5, 6.5]]);
        let mut pixels = vec![0; 8 * 8 * 4];
        draw(&mut pixels, 8, 8, layer);
        let at = |x: usize, y: usize| &pixels[(y * 8 + x) * 4..(y * 8 + x) * 4 + 3];
        assert_eq!(at(3, 5), &[255, 0, 0]);
        assert_eq!(at(3, 6), &[0, 255, 0]);
        let covered = pixels
            .chunks_exact(4)
            .filter(|p| p[..3] != [0, 0, 0])
            .count();
        assert_eq!(covered, 2);
    }

    #[test]
    fn rotated_mask_and_out_of_range_truncation_stay_explicit() {
        assert_eq!(truncate(f32::NAN), i32::MIN);
        assert_eq!(truncate(3.0e9), i32::MIN);
        assert_eq!(truncate(-1.99), -1);
        let image = image();
        let mut words = DrawState::default().words;
        words[4..6].copy_from_slice(&[2, 2]);
        words[20] = 6000;
        let mask = MaskTransition {
            put: true,
            slot: 0,
            ramp: 0,
            width: 0,
            elapsed: 0,
            mask: image.clone(),
        };
        assert!(image_layer(&image, &words, [0, 0], 255, Some(&mask), None).is_err());
        words[18] = 0;
        assert!(
            image_layer(&image, &words, [0, 0], 255, None, None)
                .unwrap()
                .is_none()
        );
    }
}
