//! cmvs-20070119 effect slots of types 0x50.. (engine +2928, four slots shared with the
//! particle pools); evidence and addresses are in games/puriring/research.md.
use crate::{
    audio::{AudioBackend, AudioSettings},
    camera::CameraHost,
    dispatch::Call,
    image::{self, ImageSource},
    scene::{ObjectImage, SceneHost},
    scene_render::Blend,
};
use std::error::Error;

/// A textured world-space quad of the old 3D queues (mesh FVF 0x142, triangle strip).
pub struct WorldQuad<'a> {
    pub image: &'a ObjectImage,
    /// Normalized texture rectangle (left, top, right, bottom).
    pub uv: [f32; 4],
    /// Strip order: top-left, top-right, bottom-left, bottom-right of the texture.
    pub corners: [[f32; 3]; 4],
    /// Object position; the queue key is its distance from the view (0x422f20).
    pub position: [f32; 3],
    pub alpha: u32,
    /// 0x411630: no blending while mesh +52 is clear and the alpha is 255.
    pub blend: Blend,
    /// Mesh +56 (0x409ba0) selects queue 5 instead of queue 1 (0x417350).
    pub late: bool,
}

/// One 4-vertex mesh object of 0x416c10 (white diffuse, texture rectangle in strip
/// order). The world matrix of 0x410c30 is scale, then Y, X and Z rotations (degrees,
/// skipped while zero), then the translation.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyMesh {
    /// Index into the effect's images; the mesh child draws with its parent's image.
    pub image: usize,
    /// Normalized texture rectangle, defaulting to the full image.
    pub uv: [f32; 4],
    pub corners: [[f32; 3]; 4],
    /// +12..+20 position and +24/+28/+32 X/Y/Z rotation.
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    /// +36..+44 (uniform here) and +4 alpha 0..255.
    pub scale: f32,
    pub alpha: u32,
    /// +52 forces blending, +56 selects queue 5; +8 selects additive blending.
    pub blended: bool,
    pub late: bool,
    pub additive: bool,
}
impl Eq for LegacyMesh {}

impl LegacyMesh {
    pub(super) fn new(image: usize, corners: [[f32; 3]; 4], position: [f32; 3]) -> Self {
        Self {
            image,
            uv: [0.0, 0.0, 1.0, 1.0],
            corners,
            position,
            rotation: [0.0; 3],
            scale: 1.0,
            alpha: 255,
            blended: false,
            late: false,
            additive: false,
        }
    }
    pub(super) fn blended(mut self, late: bool) -> Self {
        self.blended = true;
        self.late = late;
        self
    }
    fn world(&self) -> [[f32; 3]; 4] {
        let rotate = |p: [f32; 3]| {
            let mut p = p.map(|v| v * self.scale);
            for (axis, degrees) in [
                (1, self.rotation[1]),
                (0, self.rotation[0]),
                (2, self.rotation[2]),
            ] {
                if degrees == 0.0 {
                    continue;
                }
                let r = f64::from(degrees) * 0.01745329238474369;
                let (s, c) = (r.sin() as f32, r.cos() as f32);
                let [x, y, z] = p;
                // D3DX row-vector rotations (left-handed).
                p = match axis {
                    0 => [x, y * c - z * s, y * s + z * c],
                    1 => [x * c + z * s, y, -x * s + z * c],
                    _ => [x * c - y * s, x * s + y * c, z],
                };
            }
            std::array::from_fn(|i| p[i] + self.position[i])
        };
        self.corners.map(rotate)
    }
}

/// Meshes and the images their objects loaded.
#[derive(Debug, PartialEq, Eq)]
pub struct MeshScene {
    pub images: Vec<ObjectImage>,
    pub meshes: Vec<LegacyMesh>,
}
impl MeshScene {
    pub fn quads(&self) -> Vec<WorldQuad<'_>> {
        self.meshes
            .iter()
            .filter(|mesh| mesh.alpha > 0)
            .map(|mesh| WorldQuad {
                image: &self.images[mesh.image],
                uv: mesh.uv,
                corners: mesh.world(),
                position: mesh.position,
                alpha: mesh.alpha,
                blend: match (mesh.blended || mesh.alpha < 255, mesh.additive) {
                    (false, _) => Blend::Replace,
                    (true, false) => Blend::Alpha,
                    (true, true) => Blend::Additive,
                },
                late: mesh.late,
            })
            .collect()
    }
}

/// Horizontal and vertical quads used by the scripted scenes, in strip order.
fn flat(half: f32, y: f32) -> [[f32; 3]; 4] {
    [
        [-half, y, half],
        [half, y, half],
        [-half, y, -half],
        [half, y, -half],
    ]
}
fn upright(half: f32, height: f32) -> [[f32; 3]; 4] {
    [
        [-half, height, 0.0],
        [half, height, 0.0],
        [-half, 0.0, 0.0],
        [half, 0.0, 0.0],
    ]
}

/// Type 0x50 (0x2290 -> 0x4184c0, 0x4205f0/0x4206b0, frame 0x420530): three 10x10 floor
/// tiles textured by the first image follow the view along +Z; a 30x30 blended plane
/// textured by the second image stays under the view.
#[derive(Debug, PartialEq, Eq)]
pub struct SnowField {
    pub scene: MeshScene,
    /// +20: the tile moved next, and +24: the Z distance already covered.
    pub next: usize,
    pub covered: u32,
}

impl SnowField {
    fn new(floor: ObjectImage, cover: ObjectImage) -> Self {
        let mut meshes: Vec<_> = [0.0, 10.0, 20.0]
            .into_iter()
            .map(|z| LegacyMesh::new(0, flat(5.0, -0.1), [0.0, 0.0, z]))
            .collect();
        let mut cover_mesh = LegacyMesh::new(1, flat(15.0, 0.0), [0.0; 3]);
        cover_mesh.blended = true;
        meshes.push(cover_mesh);
        Self {
            scene: MeshScene {
                images: vec![floor, cover],
                meshes,
            },
            next: 0,
            covered: 0,
        }
    }

    fn advance(&mut self, view: [f32; 3]) {
        self.scene.meshes[3].position = [view[0], 0.0, view[2]];
        let covered = f32::from_bits(self.covered);
        if view[2] > covered + 10.0 {
            self.scene.meshes[self.next].position = [0.0, 0.0, covered + 30.0];
            self.next = (self.next + 1) % 3;
            self.covered = (covered + 10.0).to_bits();
        }
    }
}

/// Type 0x56: 0x41e8e0 geometry and 0x41e740 frame update.
#[derive(Debug, PartialEq, Eq)]
pub struct FloatingPlanes {
    pub scene: MeshScene,
    pub clock: u32,
}
impl FloatingPlanes {
    fn new(images: Vec<ObjectImage>) -> Self {
        let mut meshes = vec![LegacyMesh::new(0, upright(7.5, 9.0), [0.0, -3.0, 8.7])];
        for (position, x, z) in [
            ([-0.86, 0.6, 3.0], 5.0, -10.0),
            ([0.8, 1.4, 4.0], -15.0, 10.0),
            ([-0.6, 2.2, 5.0], -15.0, -10.0),
            ([0.6, 3.0, 6.0], -18.0, 10.0),
            ([-0.4, 3.8, 7.0], -18.0, -10.0),
            ([0.4, 4.6, 8.0], -18.0, 10.0),
            ([-0.2, 5.4, 9.0], -18.0, -10.0),
        ] {
            let mut mesh = LegacyMesh::new(1, flat(1.0, 0.0), position).blended(true);
            mesh.rotation = [x, 0.0, z];
            meshes.push(mesh);
        }
        Self {
            scene: MeshScene { images, meshes },
            clock: 0,
        }
    }
    fn advance(&mut self, delta: u32) {
        self.clock = self.clock.wrapping_add(delta.min(200));
        let angle = self.clock.wrapping_mul(3600) / 120000 % 3600;
        let phase = self.clock % 2000;
        let alpha = if phase < 1000 {
            255 - 63 * phase / 1000
        } else {
            192 + 63 * (phase - 1000) / 1000
        };
        for (mesh, offset) in self.scene.meshes[1..]
            .iter_mut()
            .zip([720, 0, 1720, 480, 1500, 640, 1280])
        {
            mesh.rotation[1] = ((angle + offset) % 3600) as f32 / 10.0;
            mesh.alpha = alpha;
        }
    }
}

/// Type 0x52 (0x2292 -> 0x418630, 0x41b3f0/0x41b5f0/0x41c5b0, frame 0x41c370): a wall, a
/// stand and 24 rotating 6x6 rings over nine fixed images, with a rising pillar.
#[derive(Debug, PartialEq, Eq)]
pub struct Circle {
    pub scene: MeshScene,
    /// +144 state, +148 rotation clock, +152 state clock, +156 sound played, +160 request.
    pub state: u32,
    pub clock: u32,
    pub elapsed: u32,
    pub played: bool,
    pub requested: i32,
}

const CIRCLE_IMAGES: [&[u8]; 9] = [
    b"try002a1.pb3",
    b"try002a2.pb3",
    b"try002c1.pb3",
    b"try002c2.pb3",
    b"try002c3.pb3",
    b"try002c4.pb3",
    b"try002c5.pb3",
    b"try002c6.pb3",
    b"try002d.pb3",
];
const CIRCLE_SOUNDS: [(u8, &[u8]); 2] = [(16, b"SE006.ogg"), (17, b"SE006c.wav")];
/// Mesh indices: wall, stand, six base rings, eighteen upper rings (tier-major), pillar.
const CIRCLE_RINGS: usize = 8;
const CIRCLE_PILLAR: usize = 26;
/// 0x41bc20 groups over the upper rings: first ring and count.
const CIRCLE_GROUPS: [(usize, usize); 12] = [
    (0, 1),
    (1, 2),
    (3, 2),
    (5, 1),
    (6, 1),
    (7, 2),
    (9, 2),
    (11, 1),
    (12, 1),
    (13, 2),
    (15, 2),
    (17, 1),
];

impl Circle {
    fn new(images: Vec<ObjectImage>) -> Self {
        let mut meshes = vec![
            LegacyMesh::new(0, upright(30.0, 36.0), [0.0, -12.0, 39.0]),
            LegacyMesh::new(1, upright(5.0, 6.0), [0.0, 2.0, 7.15]).blended(true),
        ];
        // 0x41b740 / 0x41b8e0: the height grows after every ring except the 2nd and 4th.
        let mut height = 1.25;
        for i in 0..6 {
            meshes.push(LegacyMesh::new(2 + i, flat(3.0, 0.0), [2.0, height, 13.0]).blended(true));
            if i != 1 && i != 3 {
                height += 0.25;
            }
        }
        let mut height = 2.5;
        for _ in 0..3 {
            for i in 0..6 {
                meshes.push(
                    LegacyMesh::new(2 + i, flat(3.0, 0.0), [2.0, height, 13.0]).blended(true),
                );
                if i != 1 && i != 3 {
                    height += 0.75;
                }
            }
        }
        meshes.push(LegacyMesh::new(8, upright(6.0, 20.0), [2.0, -20.0, 12.0]).blended(true));
        Self {
            scene: MeshScene { images, meshes },
            state: 0,
            clock: 0,
            elapsed: 0,
            played: false,
            requested: -1,
        }
    }

    fn group_alpha(&mut self, group: usize, alpha: u32) {
        let (first, count) = CIRCLE_GROUPS[group];
        for ring in first..first + count {
            self.scene.meshes[CIRCLE_RINGS + ring].alpha = alpha.min(255);
        }
    }

    fn advance(
        &mut self,
        delta: u32,
        backend: &mut dyn AudioBackend,
        volume: u32,
    ) -> Result<(), Box<dyn Error>> {
        self.clock = self.clock.wrapping_add(delta);
        let angle = (self.clock.wrapping_mul(360) / 5000 % 360) as f32;
        let reverse = 360.0 - angle;
        let directions = [angle, angle, reverse, reverse, angle, reverse];
        for (i, mesh) in self.scene.meshes[2..CIRCLE_PILLAR].iter_mut().enumerate() {
            mesh.rotation[1] = directions[i % 6];
        }
        match self.state {
            0 => {
                if self.requested != -1 {
                    self.state = self.requested as u32;
                    self.requested = -1;
                    self.elapsed = 0;
                    self.played = false;
                }
            }
            // 0x41be30: SE006.ogg, then each group fades in over 3 s, 300 ms apart.
            1 => {
                if !self.played {
                    backend.play_preloaded(16, false, volume)?;
                    self.played = true;
                }
                self.elapsed = self.elapsed.wrapping_add(delta.min(200));
                if self.elapsed >= 6500 {
                    for group in 0..12 {
                        self.group_alpha(group, 255);
                    }
                    self.finish();
                } else {
                    for group in 0..12 {
                        let start = 300 * group as u32;
                        if self.elapsed >= start {
                            let t = self.elapsed - start;
                            self.group_alpha(group, if t >= 3000 { 255 } else { 255 * t / 3000 });
                        }
                    }
                }
            }
            // 0x41bf10: SE006c.wav, the pillar rises 32 over 1 s and the groups fade out.
            2 => {
                if !self.played {
                    backend.play_preloaded(17, false, volume)?;
                    self.played = true;
                }
                self.elapsed = self.elapsed.wrapping_add(delta.min(200));
                if self.elapsed >= 7000 {
                    for group in 0..12 {
                        self.group_alpha(group, 0);
                    }
                    self.scene.meshes[CIRCLE_PILLAR].position = [2.0, -20.0, 12.0];
                    backend.stop_preloaded(17)?;
                    self.finish();
                } else {
                    let rise = if self.elapsed >= 1000 {
                        32.0
                    } else {
                        (f64::from(self.elapsed) * 32.0 / 1000.0) as f32
                    };
                    self.scene.meshes[CIRCLE_PILLAR].position = [2.0, rise - 20.0, 12.0];
                    for group in 0..12 {
                        let start = 1500 + 300 * group as u32;
                        if self.elapsed >= start {
                            let t = self.elapsed - start;
                            self.group_alpha(
                                group,
                                if t >= 2000 { 0 } else { 255 - 255 * t / 2000 },
                            );
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn finish(&mut self) {
        self.state = 0;
        self.elapsed = 0;
        self.played = false;
    }
}

/// Type 0x51 (0x2291 -> 0x418580, 0x420aa0, frame 0x4211b0): a scripted walk that drives the
/// device view and plays prepared sounds 16, 17 and 20.
#[derive(Debug, PartialEq, Eq)]
pub struct Walk {
    /// +0: 0 idle, 1 slow step, 2 fast step, 3 final step, 90 pause after a slow step.
    pub state: u32,
    /// +4/+8/+12: base view X/Y/Z bits.
    pub base: [u32; 3],
    /// +16 elapsed milliseconds and +20 whether this step's sound already played.
    pub elapsed: u32,
    pub played: bool,
    /// +24: requested next state, -1 for none.
    pub requested: i32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LegacyEffect {
    Magic(Box<super::magic::Magic>),
    Curtain(Box<super::curtain::Curtain>),
    Sprites(Box<super::sprites::Sprites>),
    SnowField(Box<SnowField>),
    Walk(Walk),
    Circle(Box<Circle>),
    FloatingPlanes(Box<FloatingPlanes>),
    Vortex(Box<super::vortex::Vortex>),
}
impl LegacyEffect {
    pub fn quads(&self) -> Vec<WorldQuad<'_>> {
        match self {
            Self::Magic(magic) => magic.scene.quads(),
            Self::Curtain(curtain) => curtain.quads(),
            Self::SnowField(field) => field.scene.quads(),
            Self::Circle(circle) => circle.scene.quads(),
            Self::FloatingPlanes(planes) => planes.scene.quads(),
            Self::Vortex(vortex) => vortex.scene.quads(),
            Self::Walk(_) | Self::Sprites(_) => Vec::new(),
        }
    }
}

/// Sounds prepared by 0x420aa0 and released, with device channel 10, by 0x4209f0.
const WALK_SOUNDS: [(u8, &[u8]); 3] = [
    (16, b"se001d.wav"),
    (17, b"se001e.wav"),
    (20, b"se003c.wav"),
];
const WALK_RELEASED: [u8; 5] = [16, 17, 18, 19, 20];

fn floats<const N: usize>(words: [u32; N]) -> [f32; N] {
    words.map(f32::from_bits)
}

/// 0x422170: cosine and sine of `degrees` times `length`, each truncated to hundredths.
pub(super) fn quarter(degrees: f32, length: f32) -> (f32, f32) {
    let radians = if degrees == 0.0 {
        0.0
    } else {
        f64::from(degrees) / 180.0 * f64::from(std::f32::consts::PI)
    };
    let hundredths = |value: f64| (((value as f32 as f64) * 100.0) as i32) as f64 / 100.0;
    (
        hundredths(radians.cos() * f64::from(length)) as f32,
        hundredths(radians.sin() * f64::from(length)) as f32,
    )
}

/// Inputs of a walk frame: the view to drive and the prepared-sound volume.
pub struct WalkOutput<'a> {
    pub camera: &'a mut CameraHost,
    pub backend: &'a mut dyn AudioBackend,
    pub volume: u32,
}

impl Walk {
    fn begin(&mut self, state: u32) {
        self.state = state;
        self.requested = -1;
        self.elapsed = 0;
        self.played = false;
    }
    /// Common end of a step: the base Z advances and the view rests there.
    fn finish_step(&mut self, out: &mut WalkOutput<'_>, dz: f32, y: f32, then: u32) {
        let mut base = floats(self.base);
        base[2] += dz;
        base[1] = y;
        self.base = base.map(f32::to_bits);
        out.camera.set_view_position(base);
        self.elapsed = 0;
        self.played = false;
        self.state = then;
        if self.requested != -1 {
            self.state = self.requested as u32;
            self.requested = -1;
        }
    }
    fn play(&mut self, out: &mut WalkOutput<'_>, slot: u8) -> Result<(), Box<dyn Error>> {
        out.backend.play_preloaded(slot, false, out.volume)?;
        self.played = true;
        Ok(())
    }

    /// One frame of 0x4211b0; `delta` is the frame clock, capped at 200 inside the steps.
    fn advance(&mut self, delta: u32, out: &mut WalkOutput<'_>) -> Result<(), Box<dyn Error>> {
        match self.state {
            0 => {
                if self.requested != -1 {
                    self.begin(self.requested as u32);
                }
            }
            90 => {
                self.elapsed = self.elapsed.wrapping_add(delta);
                if self.requested != -1 {
                    self.begin(self.requested as u32);
                } else if self.elapsed > 600 {
                    self.begin(1);
                }
            }
            1 => self.step(delta, out, [1500, 1000, 1250, 250], 0.2, 1.0, 16, 90)?,
            2 => self.step(delta, out, [900, 800, 850, 50], 0.4, 0.8, 17, 2)?,
            3 => self.final_step(delta, out)?,
            _ => {}
        }
        Ok(())
    }

    /// 0x420da0 (slow) / 0x420fb0 (fast): forward over `[total, walk, settle, tail]` ms, a
    /// half-sine head bob of `bob` / 10 before `walk`, then a dip and recovery.
    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        delta: u32,
        out: &mut WalkOutput<'_>,
        [total, walk, settle, tail]: [u32; 4],
        stride: f32,
        bob: f32,
        sound: u8,
        then: u32,
    ) -> Result<(), Box<dyn Error>> {
        self.elapsed = self.elapsed.wrapping_add(delta.min(200));
        let base = floats(self.base);
        if self.elapsed >= total {
            self.finish_step(out, stride, base[1], then);
            return Ok(());
        }
        let t = self.elapsed as f32;
        let forward = if self.elapsed >= walk {
            stride
        } else {
            (f64::from(t) * f64::from(stride) / f64::from(walk)) as f32
        };
        let rise = if self.elapsed < walk {
            let degrees = (f64::from(t) * 180.0 / f64::from(walk)) as f32;
            (f64::from(quarter(degrees, bob).1) / 10.0) as f32
        } else if self.elapsed < settle {
            let dip = t - walk as f32;
            (0.0 - f64::from(dip) * 0.019999999552965164 / f64::from(settle - walk)) as f32
        } else {
            let degrees = (t - settle as f32) * 90.0 / tail as f32;
            (f64::from(quarter(degrees, 0.2).1) / 10.0 - 0.019999999552965164) as f32
        };
        out.camera
            .set_view_position([base[0] + 0.0, base[1] + rise, base[2] + forward]);
        if self.elapsed >= walk && !self.played {
            self.play(out, sound)?;
        }
        Ok(())
    }

    /// 0x420b50: a 2 s final step that rises, tilts the view from 35 to 60 degrees, plays
    /// sound 20 at 1.8 s and ends at 80 degrees, idle, with device channel 10 stopped.
    fn final_step(&mut self, delta: u32, out: &mut WalkOutput<'_>) -> Result<(), Box<dyn Error>> {
        self.elapsed = self.elapsed.wrapping_add(delta.min(200));
        let base = floats(self.base);
        if self.elapsed >= 2000 {
            let z = base[2] + 1.0;
            self.base = [base[0], -0.5, z].map(f32::to_bits);
            out.camera.set_view_position([base[0], -0.5, z]);
            out.camera.set_view_angle(0, 80.0);
            self.state = 0;
            self.elapsed = 0;
            self.played = false;
            out.backend.stop(1)?;
            return Ok(());
        }
        let t = self.elapsed as f64;
        let (forward, rise) = if self.elapsed >= 1000 {
            let later = t - 1000.0;
            let degrees = (90.0 - later * 90.0 / 1000.0) as f32;
            let rise = quarter(degrees, 2.0).1 - 2.0;
            out.camera
                .set_view_angle(0, (later * 25.0 / 1000.0) as f32 + 35.0);
            ((later * 0.5 / 1000.0 + 0.5) as f32, rise)
        } else {
            let degrees = (t * 180.0 / 1000.0) as f32;
            (
                (0.5 * t / 1000.0) as f32,
                (f64::from(quarter(degrees, 1.0).1) / 10.0) as f32,
            )
        };
        out.camera
            .set_view_position([base[0] + 0.0, base[1] + rise, base[2] + forward]);
        if self.elapsed >= 1800 && !self.played {
            self.play(out, 20)?;
            out.backend.stop(1)?;
        }
        Ok(())
    }
}

impl SceneHost {
    pub fn legacy_effects(&self) -> &std::collections::BTreeMap<u8, LegacyEffect> {
        &self.legacy_effects
    }

    /// Creating any effect type replaces whatever the slot held (0x418aa0).
    pub(crate) fn clear_effect_slot(&mut self, slot: u8) {
        self.legacy_flakes.remove(&slot);
        self.rain_effects.remove(&slot);
        self.particle_effects.remove(&slot);
        self.falling_effects.remove(&slot);
        self.collision_effects.remove(&slot);
        self.fragment_effects.remove(&slot);
        self.legacy_effects.remove(&slot);
    }

    /// 0x2290 / 0x43c410: SP-4 slot (unsigned, ignored above 3), SP-8 floor image, SP-12
    /// cover image; returns 0x400c without a result write.
    pub(crate) fn create_snow_field(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        let floor_name = call.argument(8, "floor image")?;
        let cover_name = call.argument(12, "cover image")?;
        if slot > 3 {
            call.complete(0x400c)?;
            return Ok(());
        }
        let mut load = |raw| -> Result<ObjectImage, Box<dyn Error>> {
            let name = call.vm.resolve_string(raw)?;
            let decoded = image::load(source, &name)?
                .ok_or_else(|| call.error("snow field image did not decode"))?;
            Ok(ObjectImage {
                filter: None,
                name,
                pixels: decoded.pixels,
                alpha: decoded.alpha,
                surface: false,
            })
        };
        let floor = load(floor_name)?;
        let cover = load(cover_name)?;
        call.complete(0x400c)?;
        self.clear_effect_slot(slot as u8);
        self.legacy_effects.insert(
            slot as u8,
            LegacyEffect::SnowField(Box::new(SnowField::new(floor, cover))),
        );
        Ok(())
    }

    /// 0x2291 / 0x43c500: SP-4 slot; places the view at (0, 1.5, 0) with X angle 35 and Y
    /// angle 0, and prepares the walk sounds. Returns 0x4004.
    pub(crate) fn create_walk(
        &mut self,
        call: &mut Call<'_, '_>,
        camera: &mut CameraHost,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        if slot > 3 {
            call.complete(0x4004)?;
            return Ok(());
        }
        // 0x43c500 destroys the old slot object (0x418aa0) before preparing new sounds.
        self.release_legacy(slot as u8, backend)?;
        self.clear_effect_slot(slot as u8);
        for (sound, name) in WALK_SOUNDS {
            backend.preload(sound, name)?;
        }
        call.complete(0x4004)?;
        camera.set_view_position([0.0, 1.5, 0.0]);
        camera.set_view_angle(0, 35.0);
        camera.set_view_angle(1, 0.0);
        self.legacy_effects.insert(
            slot as u8,
            LegacyEffect::Walk(Walk {
                state: 0,
                base: [0.0f32, 1.5, 0.0].map(f32::to_bits),
                elapsed: 0,
                played: false,
                requested: -1,
            }),
        );
        Ok(())
    }

    /// 0x2292 / 0x43c5d0: SP-4 slot; loads the nine fixed images from the 0x2011 entries
    /// and prepares sounds 16/17. Returns 0x4004.
    pub(crate) fn create_circle(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        if slot > 3 {
            call.complete(0x4004)?;
            return Ok(());
        }
        let mut images = Vec::new();
        for name in CIRCLE_IMAGES {
            let decoded = image::load(source, name)?
                .ok_or_else(|| call.error("circle effect image did not decode"))?;
            images.push(ObjectImage {
                filter: None,
                name: name.to_vec(),
                pixels: decoded.pixels,
                alpha: decoded.alpha,
                surface: false,
            });
        }
        self.release_legacy(slot as u8, backend)?;
        self.clear_effect_slot(slot as u8);
        for (sound, name) in CIRCLE_SOUNDS {
            backend.preload(sound, name)?;
        }
        call.complete(0x4004)?;
        self.legacy_effects.insert(
            slot as u8,
            LegacyEffect::Circle(Box::new(Circle::new(images))),
        );
        Ok(())
    }

    /// 0x2296 / 0x43c910 reads only the slot but returns 0x4008.
    pub(crate) fn create_floating_planes(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        call.ignore_argument(8, "unused")?;
        if slot > 3 {
            call.complete(0x4008)?;
            return Ok(());
        }
        let mut images = Vec::new();
        for name in [b"po_bg510c.pb3".as_slice(), b"it008d2.pb3"] {
            let decoded = image::load(source, name)?
                .ok_or_else(|| call.error("floating planes image did not decode"))?;
            images.push(ObjectImage {
                filter: None,
                name: name.to_vec(),
                pixels: decoded.pixels,
                alpha: decoded.alpha,
                surface: false,
            });
        }
        self.release_legacy(slot as u8, backend)?;
        call.complete(0x4008)?;
        self.clear_effect_slot(slot as u8);
        self.legacy_effects.insert(
            slot as u8,
            LegacyEffect::FloatingPlanes(Box::new(FloatingPlanes::new(images))),
        );
        Ok(())
    }

    pub(crate) fn create_magic(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
        backend: &mut dyn AudioBackend,
        camera: &mut CameraHost,
        random: &mut crate::random::CrtRandom,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        if slot > 3 {
            call.complete(0x4004)?;
            return Ok(());
        }
        let magic = super::magic::Magic::new(source, random)?;
        self.release_legacy(slot as u8, backend)?;
        backend.preload(16, b"st03_magic1b.wav")?;
        call.complete(0x4004)?;
        self.clear_effect_slot(slot as u8);
        camera.set_view_position([0.0, 1.0, 0.0]);
        camera.set_view_angle(0, 0.0);
        camera.set_view_angle(1, 0.0);
        self.legacy_effects
            .insert(slot as u8, LegacyEffect::Magic(Box::new(magic)));
        Ok(())
    }

    pub(crate) fn create_curtain(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
        backend: &mut dyn AudioBackend,
        camera: &mut CameraHost,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        if slot > 3 {
            call.complete(0x4004)?;
            return Ok(());
        }
        let curtain = super::curtain::Curtain::new(source)?;
        self.release_legacy(slot as u8, backend)?;
        call.complete(0x4004)?;
        self.clear_effect_slot(slot as u8);
        camera.set_view_position([0.0, 3.2, 0.0]);
        camera.set_view_angle(0, 0.0);
        camera.set_view_angle(1, 0.0);
        self.legacy_effects
            .insert(slot as u8, LegacyEffect::Curtain(Box::new(curtain)));
        Ok(())
    }

    pub(crate) fn create_sprites(
        &mut self,
        call: &mut Call<'_, '_>,
        source: &mut dyn ImageSource,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        if slot > 3 {
            call.complete(0x4004)?;
            return Ok(());
        }
        let mut images = Vec::new();
        for name in [b"ci001a.pb3", b"ci001b.pb3", b"ci001c.pb3"] {
            let decoded = image::load(source, name)?
                .ok_or_else(|| call.error("sprite effect image did not decode"))?;
            images.push(ObjectImage {
                filter: None,
                name: name.to_vec(),
                pixels: decoded.pixels,
                alpha: decoded.alpha,
                surface: false,
            });
        }
        self.release_legacy(slot as u8, backend)?;
        self.clear_effect_slot(slot as u8);
        backend.preload(16, b"SE035a.ogg")?;
        backend.preload(17, b"SE037b.ogg")?;
        call.complete(0x4004)?;
        self.legacy_effects.insert(
            slot as u8,
            LegacyEffect::Sprites(Box::new(super::sprites::Sprites::new(images))),
        );
        Ok(())
    }

    /// The destructor side effects of 0x418aa0 for types that own device resources.
    pub(crate) fn release_legacy(
        &mut self,
        slot: u8,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        match self.legacy_effects.get(&slot) {
            Some(LegacyEffect::Walk(_)) => {
                for sound in WALK_RELEASED {
                    backend.release_preloaded(sound)?;
                }
                backend.stop(1)?;
            }
            // 0x41bd20 releases the two prepared sounds.
            Some(LegacyEffect::Magic(_)) => {
                backend.release_preloaded(16)?;
            }
            Some(LegacyEffect::Sprites(_)) => {
                backend.release_preloaded(16)?;
                backend.release_preloaded(17)?;
            }
            Some(LegacyEffect::Circle(_)) => {
                for (sound, _) in CIRCLE_SOUNDS {
                    backend.release_preloaded(sound)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 0x22a0 / 0x43cab0 with the legacy destructors: SP-4 slot 0..3, returns 0x4004.
    pub(crate) fn release_effect_with_audio(
        &mut self,
        call: &mut Call<'_, '_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "slot")?;
        if slot < 4 {
            self.release_legacy(slot as u8, backend)?;
        }
        call.complete(0x4004)?;
        if slot < 4 {
            self.clear_effect_slot(slot as u8);
        }
        Ok(())
    }

    /// 0x22ae / 0x43cb00 -> 0x417c90 for the legacy types: the walk reports its state (or -1
    /// while idle); a snow field reports -1. Other types use the shared query.
    pub(crate) fn query_legacy_effect(&self, slot: u8) -> Option<u32> {
        self.legacy_effects.get(&slot).map(|effect| match effect {
            LegacyEffect::Magic(magic) if magic.requested == -1 => magic.state,
            LegacyEffect::Curtain(curtain) if curtain.requested == -1 => curtain.state,
            LegacyEffect::Sprites(sprites) if sprites.requested == -1 => sprites.state,
            LegacyEffect::Walk(walk) if walk.state != 0 => walk.state,
            // 0x417c90 case 0x52: the state, or -1 while a request is pending.
            LegacyEffect::Circle(circle) if circle.requested == -1 => circle.state,
            LegacyEffect::Vortex(_) => 0,
            _ => u32::MAX,
        })
    }

    /// 0x22af / 0x43cb30 -> 0x417d20: SP-4 slot, SP-8..SP-16 parameters, returns 0x4010.
    /// A walk accepts the next state 1..3 (0x420a60); the snow field ignores the call.
    pub(crate) fn set_legacy_effect(
        &mut self,
        call: &mut Call<'_, '_>,
    ) -> Result<(), Box<dyn Error>> {
        let slot = call.argument(4, "effect slot")?;
        let value = call.argument(8, "effect parameter")?;
        call.argument(12, "effect parameter")?;
        call.argument(16, "effect parameter")?;
        let known = slot > 3
            || self.legacy_effects.contains_key(&(slot as u8))
            || self.has_effect_slot(slot as u8);
        if !known {
            return Err(call
                .error("effect parameter needs an existing effect")
                .into());
        }
        if slot <= 3
            && let Some(LegacyEffect::Sprites(sprites)) = self.legacy_effects.get_mut(&(slot as u8))
        {
            sprites.request(value)?;
        }
        if slot <= 3
            && let Some(LegacyEffect::Curtain(curtain)) = self.legacy_effects.get_mut(&(slot as u8))
            && (1..=5).contains(&value)
        {
            curtain.requested = value as i32;
        }
        if slot <= 3
            && let Some(LegacyEffect::Magic(magic)) = self.legacy_effects.get_mut(&(slot as u8))
        {
            magic.request(value);
        }
        call.complete(0x4010)?;
        match self.legacy_effects.get_mut(&(slot as u8)) {
            Some(LegacyEffect::Vortex(vortex)) if slot <= 3 => vortex.request(value),
            Some(LegacyEffect::Walk(walk)) if slot <= 3 && matches!(value, 1..=3) => {
                walk.requested = value as i32;
            }
            // 0x41b700 accepts states 1 and 2.
            Some(LegacyEffect::Circle(circle)) if slot <= 3 && matches!(value, 1 | 2) => {
                circle.requested = value as i32;
            }
            _ => {}
        }
        Ok(())
    }

    fn has_effect_slot(&self, slot: u8) -> bool {
        self.legacy_flakes.contains_key(&slot)
            || self.rain_effects.contains_key(&slot)
            || self.particle_effects.contains_key(&slot)
            || self.falling_effects.contains_key(&slot)
            || self.collision_effects.contains_key(&slot)
            || self.fragment_effects.contains_key(&slot)
    }

    /// The legacy part of 0x417e70, after the view motion of the same frame.
    pub fn advance_legacy_effects(
        &mut self,
        delta: u32,
        camera: &mut CameraHost,
        settings: AudioSettings,
        backend: &mut dyn AudioBackend,
    ) -> Result<(), Box<dyn Error>> {
        for slot in 0..4 {
            match self.legacy_effects.get_mut(&slot) {
                Some(LegacyEffect::SnowField(field)) => {
                    let view = camera.view(0).expect("view exists").words[..3]
                        .try_into()
                        .map(|w: [u32; 3]| floats(w))
                        .expect("three axes");
                    field.advance(view);
                }
                Some(LegacyEffect::Circle(circle)) => {
                    let volume = settings.master.wrapping_mul(settings.se) >> 8;
                    circle.advance(delta, backend, volume)?;
                }
                Some(LegacyEffect::Walk(walk)) => {
                    // 0x4211b0 states use engine +1552 x +1560 (master x SE), without the SE gate.
                    let volume = settings.master.wrapping_mul(settings.se) >> 8;
                    walk.advance(
                        delta,
                        &mut WalkOutput {
                            camera,
                            backend,
                            volume,
                        },
                    )?;
                }
                Some(LegacyEffect::FloatingPlanes(planes)) => planes.advance(delta),
                Some(LegacyEffect::Vortex(vortex)) => vortex.advance(delta),
                Some(LegacyEffect::Sprites(sprites)) => sprites.advance(
                    delta,
                    backend,
                    settings.master.wrapping_mul(settings.se) >> 8,
                )?,
                Some(LegacyEffect::Curtain(curtain)) => curtain.advance(delta)?,
                Some(LegacyEffect::Magic(magic)) => magic.advance(
                    delta,
                    backend,
                    settings.master.wrapping_mul(settings.se) >> 8,
                )?,
                None => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::RgbaImage;

    fn image(name: &[u8]) -> ObjectImage {
        ObjectImage {
            filter: None,
            name: name.to_vec(),
            pixels: RgbaImage::new(4, 4, vec![255; 64]).unwrap(),
            alpha: false,
            surface: false,
        }
    }

    #[test]
    fn floating_planes_cap_time_and_pulse_without_moving_the_background() {
        let mut planes = FloatingPlanes::new(vec![image(b"wall"), image(b"plane")]);
        let wall = planes.scene.meshes[0].clone();
        assert_eq!(planes.scene.meshes.len(), 8);
        planes.advance(1000);
        assert_eq!(planes.clock, 200);
        assert_eq!(planes.scene.meshes[1].rotation, [5.0, 72.6, -10.0]);
        for _ in 0..4 {
            planes.advance(200);
        }
        assert_eq!(planes.scene.meshes[1].alpha, 192);
        for _ in 0..5 {
            planes.advance(200);
        }
        assert_eq!(planes.scene.meshes[1].alpha, 255);
        assert_eq!(planes.scene.meshes[0], wall);
        assert!(planes.scene.quads()[1].late);
    }

    #[test]
    fn snow_field_tiles_leapfrog_ahead_of_the_view_and_the_cover_follows() {
        let mut field = SnowField::new(image(b"floor"), image(b"cover"));
        field.advance([1.0, 1.5, 9.0]);
        assert_eq!(field.scene.meshes[3].position, [1.0, 0.0, 9.0]);
        assert_eq!(field.scene.meshes[0].position, [0.0, 0.0, 0.0]);
        field.advance([1.0, 1.5, 10.5]);
        assert_eq!(field.scene.meshes[0].position, [0.0, 0.0, 30.0]);
        assert_eq!((field.next, f32::from_bits(field.covered)), (1, 10.0));
        let quads = field.scene.quads();
        assert!(matches!(quads[0].blend, Blend::Replace));
        assert!(matches!(quads[3].blend, Blend::Alpha) && !quads[3].late);
    }

    #[test]
    fn circle_rings_rotate_with_the_clock_and_groups_fade_in() {
        let mut circle = Circle::new((0..9).map(|i| image(&[i])).collect());
        let mut backend = crate::audio::RecordingAudio::default();
        circle.advance(1250, &mut backend, 256).unwrap();
        // 360 * 1250 / 5000 = 90 degrees; rings 3 and 4 of each tier turn backwards.
        assert_eq!(circle.scene.meshes[2].rotation[1], 90.0);
        assert_eq!(circle.scene.meshes[4].rotation[1], 270.0);
        circle.requested = 1;
        circle.advance(0, &mut backend, 256).unwrap();
        assert_eq!((circle.state, circle.requested), (1, -1));
        circle.advance(150, &mut backend, 256).unwrap();
        assert_eq!(circle.scene.meshes[CIRCLE_RINGS].alpha, 255 * 150 / 3000);
        assert_eq!(circle.scene.meshes[CIRCLE_RINGS + 1].alpha, 255);
        for _ in 0..40 {
            circle.advance(200, &mut backend, 256).unwrap();
        }
        assert_eq!(circle.state, 0);
        assert!(
            circle.scene.meshes[CIRCLE_RINGS..CIRCLE_PILLAR]
                .iter()
                .all(|m| m.alpha == 255)
        );
    }

    #[test]
    fn world_quads_rotate_about_y_before_translation() {
        let mut mesh = LegacyMesh::new(0, flat(1.0, 0.0), [2.0, 0.0, 3.0]);
        mesh.rotation[1] = 90.0;
        let world = mesh.world();
        // D3DX RotationY: (x, z) -> (x cos + z sin, -x sin + z cos).
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(world[0][0], 3.0) && close(world[0][2], 4.0));
    }
}
