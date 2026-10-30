//! Screen cameras at engine +1968 -> +16 and +24 (0xa0 bytes each, constructor 0x14004fab0);
//! addresses refer to the documented binary hash. 0x2058..0x205f/0x2066/0x2067/0x209c set
//! fields of the camera selected by SP-4 (0x140027090), 0x2060..0x2065 the same fields of camera 0
//! (0x140027070). Fields stay raw words named by offset. The original renderer uses them
//! for coordinate modes 2/3 (0x140020340 -> 0x14004ef40); the software renderer supports
//! cameras of type 3 without rotation (`project_object`). 0x2056 queries projected
//! integer coordinates without rendering. Camera 0 motion supports linear and quadratic
//! ease-out branches. 0x2126 queries the screen device size at +1968 -> +8.
use crate::{
    bytecode::DecodeError,
    evaluate::{error, truncate},
    fade::SkipGate,
    input::ActionInput,
    machine::{Machine, Step},
    message::RESEARCH_SCREEN,
    vm::SystemResult,
};
mod view;
pub use view::DeviceView;

pub const CAMERAS: usize = 2;
pub const CAMERA_WORDS: usize = 40;

/// Word index of +84, set while an eased move runs (0x14004f5b0..0x14004f9a0).
const MOVING: usize = 21;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Camera {
    pub words: [u32; CAMERA_WORDS],
}

impl Default for Camera {
    /// 0x14004fab0 writes every word; unlisted ones are zero.
    fn default() -> Self {
        let mut words = [0; CAMERA_WORDS];
        for (index, value) in [
            (2, 3.0f32),
            (6, 10.0),
            (7, 10.0),
            (8, 6.0),
            (9, 1024.0),
            (10, 612.0),
            (11, 24.0),
            (15, 512.0),
            (16, 306.0),
            (17, 1.0),
            (18, 1.0),
        ] {
            words[index] = value.to_bits();
        }
        words[0] = 1;
        Self { words }
    }
}

impl Camera {
    pub fn float(&self, index: usize) -> f32 {
        f32::from_bits(self.words[index])
    }
    fn set(&mut self, index: usize, value: f32) {
        self.words[index] = value.to_bits();
    }

    /// 0x14004f550: cancel, optionally copying the stored targets without quantization.
    fn stop_move(&mut self, finish: bool) {
        if self.words[MOVING] == 0 {
            return;
        }
        self.words[MOVING] = 0;
        if finish {
            self.words.copy_within(23..26, 1);
            for (bit, target, field) in [
                (0x80000000, 26, 12),
                (0x40000000, 27, 13),
                (0x20000000, 28, 14),
            ] {
                if self.words[22] & bit != 0 {
                    self.words[field] = self.words[target];
                }
            }
        }
    }

    /// 0x14004fb60: unbounded frame delta (unlike object tracks), unsigned elapsed/time,
    /// f32 linear arithmetic or f32 differences with f64 quadratic ease-out.
    fn advance_move(&mut self, delta: u32) {
        if self.words[MOVING] == 0 {
            return;
        }
        self.words[36] = self.words[36].wrapping_add(delta);
        let elapsed = self.words[36];
        let duration = self.words[29];
        if elapsed >= duration {
            self.stop_move(true);
            return;
        }
        let time = elapsed as f32;
        let total = duration as f32;
        for (axis, shift) in [(0, 16), (1, 8), (2, 0)] {
            let target = self.float(23 + axis);
            let base = self.float(30 + axis);
            if target == base {
                continue;
            }
            let value = match (self.words[22] >> shift) & 0xff {
                0 => hundredths(((target - base) * time) / total) + base,
                1 => {
                    let remaining = 1.0 - f64::from(time) / f64::from(total);
                    let value = ((remaining * remaining) * f64::from(target - base)) * 100.0;
                    let integer = if value.is_nan()
                        || !(-2_147_483_648.0..2_147_483_648.0).contains(&value)
                    {
                        i32::MIN
                    } else {
                        value as i32
                    };
                    target - (integer as f32 / 100.0)
                }
                _ => unreachable!("only validated camera move modes may start"),
            };
            self.set(1 + axis, value);
        }
    }

    /// 0x14004ee00: mode 3 projects a point, other camera modes leave it unchanged.
    /// Preserve the scalar SSE operation order; +52 is a horizontal projection offset,
    /// not the angle at +56. Query mode 1 selects the alternate position during a move.
    fn project(&self, mode: u32, point: [f32; 3], offset: f32, reference: f32) -> [f32; 3] {
        if self.words[0] != 3 {
            return point;
        }
        let base = if mode == 1 && self.words[MOVING] != 0 {
            23
        } else {
            1
        };
        let [x, y, z] = point;
        let depth = z - self.float(base + 2);
        let ratio = reference / depth;
        let shift = if self.float(13) != 0.0 {
            ((self.float(7) * 0.5) * (self.float(13) / 45.0)) * (depth / self.float(6))
        } else {
            0.0
        };
        let relative = depth / self.float(6);
        let vertical = ((self.float(base + 1) - y) * ratio) + offset;
        let horizontal = (x - (shift + self.float(base))) * ratio;
        let scale_y = (self.float(10) / (relative * self.float(8))) / self.float(18);
        let scale_x = (self.float(9) / (relative * self.float(7))) / self.float(17);
        [
            (scale_x * horizontal) + self.float(15),
            ((scale_y * vertical) + self.float(16)) + ((reference - depth) * self.float(11)),
            0.0,
        ]
    }
}

/// 0x14004ef40's result for a drawn object (camera type 3; see `Camera::project_object`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectProjection {
    /// Screen position of the object's origin (output +0/+4).
    pub position: [f32; 2],
    /// z − camera z (+8); the queue key adds the draw state's depth bias (`queue_key`).
    pub depth: f32,
    /// (reference / depth)², times the input scale 1.0 of a top-level object (+12/+16).
    pub scale: f32,
    /// 255, or (int)((depth − 9) × 256) below depth 10; scales the opacity by /255 (+20).
    pub fade: i32,
}

impl Camera {
    /// +148: camera 1's queue 6 is drawn only while set (0x14004eca0).
    pub fn enabled(&self) -> bool {
        self.words[37] != 0
    }
    /// +152 (0x209c): nonzero draws the camera's queue into an effect target first
    /// (0x14004ecc0 in 0x140015fc0).
    pub fn effect(&self) -> u32 {
        self.words[38]
    }
    /// 0x14003de80 keys a projected object by 0x14004ef30 applied to 0x14001f360:
    /// (draw +108 + z) − camera z (+12), for any camera type. Only keys above 0 (not NaN)
    /// are enqueued; children of such objects are skipped with their parent.
    pub fn queue_key(&self, z: f32, bias: f32) -> f32 {
        (bias + z) - self.float(3)
    }
    /// 0x14004ef40 for camera type 3 (+0): the x/y terms equal the 0x2056 projection of
    /// the current position, the offset being draw +64 and the reference depth draw +60.
    /// Objects at depth ≤ 9 (also NaN) are not drawn. Types 1 and 2 are not verified.
    pub fn project_object(
        &self,
        world: [f32; 3],
        offset: f32,
        reference: f32,
    ) -> Result<Option<ObjectProjection>, &'static str> {
        if self.words[0] != 3 {
            return Err("camera projection types other than 3 are not verified");
        }
        let depth = world[2] - self.float(3);
        if depth.is_nan() || depth <= 9.0 {
            return Ok(None);
        }
        let [x, y, _] = self.project(0, world, offset, reference);
        let ratio = reference / depth;
        let fade = if depth >= 10.0 {
            255
        } else {
            truncate((depth - 9.0) * 256.0)
        };
        Ok(Some(ObjectProjection {
            position: [x, y],
            depth,
            scale: ratio * ratio,
            fade,
        }))
    }
}

/// `(float)(int)(v * 100) / 100` as 0x14004ed10 computes it with cvttss2si.
fn hundredths(value: f32) -> f32 {
    truncate(value * 100.0) as f32 / 100.0
}

#[derive(Debug, Clone)]
pub struct CameraHost {
    cameras: [Camera; CAMERAS],
    views: [DeviceView; CAMERAS],
}

impl Default for CameraHost {
    fn default() -> Self {
        Self {
            cameras: Default::default(),
            views: std::array::from_fn(DeviceView::new),
        }
    }
}

impl CameraHost {
    /// 0x14004e760 resets the camera parameters and motion, leaving padding alone.
    pub(crate) fn reset(&mut self) {
        for view in &mut self.views {
            view.reset();
        }
        for camera in &mut self.cameras {
            let retained = [19, 20, 37, 38, 39].map(|i| camera.words[i]);
            *camera = Camera::default();
            for (i, value) in [19, 20, 37, 38, 39].into_iter().zip(retained) {
                camera.words[i] = value;
            }
        }
    }
    pub fn camera(&self, index: usize) -> Option<&Camera> {
        self.cameras.get(index)
    }
    /// Camera 0 +84: a move is running (0x206a).
    pub fn moving(&self) -> bool {
        self.cameras[0].words[MOVING] != 0
    }
    /// Camera updates are the last part of 0x140015900, camera 0 then camera 1.
    pub fn advance_frame(&mut self, delta: u32) {
        for camera in &mut self.cameras {
            camera.advance_move(delta);
        }
    }

    /// 0x206b/0x206c/0x206d -> 0x14004f5b0/0x14004f680/0x14004f820.
    /// Camera 0 only; first argument mode, last duration, intervening floats XZ/XYZ/Y.
    fn start_move(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, command + 0x2000, why);
        let count = match command {
            0x6b => 4,
            0x6c => 5,
            _ => 3,
        };
        let mut args = [0; 5];
        for (i, arg) in args.iter_mut().take(count).enumerate() {
            *arg = vm.stack().peek_u32((i + 1) * 4).map_err(|e| fail(e.0))?;
        }
        if args[0] > 1 {
            return Err(fail("camera motion interpolation mode is not implemented"));
        }
        let mut target: [u32; 3] = self.cameras[0].words[1..4].try_into().expect("XYZ");
        let axes: &[usize] = match command {
            0x6b => &[0, 2],
            0x6c => &[0, 1, 2],
            _ => &[1],
        };
        for (i, &axis) in axes.iter().enumerate() {
            target[axis] = hundredths(f32::from_bits(args[i + 1])).to_bits();
        }
        vm.complete_system(SystemResult(0x4000 | (count * 4) as u16))?;
        let camera = &mut self.cameras[0];
        camera.words[22] = if args[0] == 0 { 0 } else { 0x00010101 };
        camera.words[23..26].copy_from_slice(&target);
        camera.words.copy_within(12..15, 26);
        camera.words[29] = args[count - 1];
        camera.words.copy_within(1..4, 30);
        camera.words.copy_within(12..15, 33);
        camera.words[36] = 0;
        camera.words[MOVING] = 1;
        Ok(true)
    }

    /// 0x2069 / 0x14006fd90: wait on camera 0, using 0x4004/0xa000/0xc004.
    pub fn try_wait_move(
        &mut self,
        vm: &mut Machine<'_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Option<Step>, DecodeError> {
        let Some((pc, 0x69)) = vm.pending_system() else {
            return Ok(None);
        };
        let skippable = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2069, e.0))? != 0;
        if !self.moving() {
            let step = vm.complete_system(SystemResult(0x4004))?;
            vm.state_mut()
                .set_system_integer(0, 0)
                .expect("result 0 exists");
            return Ok(Some(step));
        }
        let immediate = !gate.effects || input.skip_held() || gate.skip;
        let released = skippable && !immediate && input.released(0);
        if skippable && (immediate || released) {
            let step = vm.complete_system(SystemResult(0xc004))?;
            if released {
                input.consume(0);
            }
            vm.state_mut()
                .set_system_integer(0, 1)
                .expect("result 0 exists");
            self.cameras[0].stop_move(true);
            return Ok(Some(step));
        }
        let step = vm.yield_pending_system()?;
        vm.state_mut()
            .set_system_integer(0, 0)
            .expect("result 0 exists");
        Ok(Some(step))
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        if let Some((pc, command @ 0x260..=0x264)) = vm.pending_system() {
            return self.set_view(vm, pc, command);
        }
        if let Some((pc, command @ 0x6b..=0x6d)) = vm.pending_system() {
            return self.start_move(vm, pc, command);
        }
        if matches!(vm.pending_system(), Some((_, 0x6a))) {
            // 0x206a / 0x14006fe80 -> camera 0 (0x140027070) -> 0x14004f5a0.
            // No stack parameters: return only the boolean of camera +84.
            let result = u32::from(self.moving());
            vm.complete_system(SystemResult(0x4000))?;
            vm.state_mut()
                .set_system_integer(0, result)
                .expect("result 0 exists");
            return Ok(true);
        }
        if matches!(vm.pending_system(), Some((_, 0x126))) {
            // 0x2126 / 0x1400756f0: no stack parameters; results 1/2 = device +24/+28
            // (0x140027020/0x140027030). Only startup code (0x140010370/0x140011320) sets
            // them from cmvs.cfg, which selects 1280x720 here: the research viewport.
            vm.complete_system(SystemResult(0x4000))?;
            for (slot, value) in [(1, RESEARCH_SCREEN[0]), (2, RESEARCH_SCREEN[1])] {
                vm.state_mut()
                    .set_system_integer(slot, value)
                    .expect("results 1/2 exist");
            }
            return Ok(true);
        }
        if let Some((pc, 0x56)) = vm.pending_system() {
            return self.project_query(vm, pc);
        }
        let Some((pc, command @ (0x58..=0x68 | 0x9c))) = vm.pending_system() else {
            return Ok(false);
        };
        let op = command + 0x2000;
        let fail = |why| error(pc, op, why);
        // Commands 0x2060..0x2065 address camera 0 directly and take one argument less.
        let indexed = !(0x60..=0x65).contains(&command) && command != 0x68;
        let values = match command {
            0x58 | 0x5a | 0x60 | 0x62 => 3,
            0x59 | 0x5d | 0x61 | 0x65 | 0x67 => 2,
            _ => 1,
        };
        let count = values + usize::from(indexed);
        let mut args = Vec::with_capacity(count);
        for i in 1..=count {
            args.push(vm.stack().peek_u32(i * 4).map_err(|e| fail(e.0))?);
        }
        if command == 0x68 {
            // 0x2068 / 0x14006fd50 -> 0x14004f550: zero cancels, nonzero finishes.
            vm.complete_system(SystemResult(0x4004))?;
            self.cameras[0].stop_move(args[0] != 0);
            return Ok(true);
        }
        let camera = if indexed {
            // 0x140027090 indexes the screen object's pointers without a bound; only
            // indices 0 and 1 are cameras.
            usize::try_from(args.remove(0))
                .ok()
                .filter(|index| *index < CAMERAS)
                .ok_or_else(|| fail("camera index outside 0..1"))?
        } else {
            0
        };
        let f = |i: usize| f32::from_bits(args[i]);
        vm.complete_system(SystemResult(0x4000 | (count * 4) as u16))?;
        let target = &mut self.cameras[camera];
        match command {
            // 0x14004ecd0: +28, +32, +24.
            0x58 | 0x60 => {
                target.set(7, f(0));
                target.set(8, f(1));
                target.set(6, f(2));
            }
            // 0x14004ece0: +36, +40 and their halves at +60, +64.
            0x59 | 0x61 => {
                target.set(9, f(0));
                target.set(10, f(1));
                target.set(15, f(0) * 0.5);
                target.set(16, f(1) * 0.5);
            }
            // 0x14004ed10: +4, +8, +12 in hundredths.
            0x5a | 0x62 => {
                for i in 0..3 {
                    target.set(1 + i, hundredths(f(i)));
                }
            }
            // 0x14004ed90: +56 = (|(int)(v * 100)| % 36000) / 100, an angle in degrees.
            0x5b | 0x63 => {
                let centi = truncate(f(0) * 100.0).wrapping_abs();
                target.set(14, (centi % 36000) as f32 / 100.0);
            }
            // 0x14004ede0: +44.
            0x5c | 0x64 => target.set(11, f(0)),
            // 0x14004ed70: +60, +64.
            0x5d | 0x65 => {
                target.set(15, f(0));
                target.set(16, f(1));
            }
            // 0x14002e840 / 0x14004ecb0 / 0x14004edf0: integers at +148, +0, +156.
            0x5e => target.words[37] = args[0],
            // 0x209c / 0x14006f9e0 -> 0x140019f80: integer at +152.
            0x9c => target.words[38] = args[0],
            0x5f => target.words[0] = args[0],
            0x66 => target.words[39] = args[0],
            // 0x14004ed80: +68, +72.
            0x67 => {
                target.set(17, f(0));
                target.set(18, f(1));
            }
            _ => unreachable!("matched above"),
        }
        Ok(true)
    }

    /// 0x2056 / 0x14006fa20: SP-4 camera, SP-8 query mode, SP-12..20 XYZ,
    /// SP-24 vertical offset, SP-28 reference depth. Returns 0x401c and writes only
    /// integer system results 1..3 with cvttss2si (including its indefinite i32 value).
    fn project_query(&self, vm: &mut Machine<'_>, pc: usize) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, 0x2056, why);
        let mut args = [0; 7];
        for (i, arg) in args.iter_mut().enumerate() {
            *arg = vm.stack().peek_u32(4 * (i + 1)).map_err(|e| fail(e.0))?;
        }
        let camera = self
            .cameras
            .get(args[0] as usize)
            .ok_or_else(|| fail("camera index outside 0..1"))?;
        let point = camera.project(
            args[1],
            [args[2], args[3], args[4]].map(f32::from_bits),
            f32::from_bits(args[5]),
            f32::from_bits(args[6]),
        );
        let result = point.map(truncate);
        vm.complete_system(SystemResult(0x401c))?;
        for (index, value) in (1..=3).zip(result) {
            vm.state_mut()
                .set_system_integer(index, value as u32)
                .expect("system results 1..3 exist");
        }
        Ok(true)
    }
}

impl CameraHost {
    /// Records 0x880/0x881, writer 0x14004e7f0: version 3, words 37..39, then by the mode in
    /// word 39 the base words 0..14 (mode 0 and 1) and the motion words 21..36 (mode 1),
    /// then words 15..18. A running move saves its targets as the base words.
    pub(crate) fn saved(&self, index: usize) -> Vec<u8> {
        let w = &self.cameras[index].words;
        let mut out = 3u16.to_le_bytes().to_vec();
        let mut put = |words: &[u32]| {
            for word in words {
                out.extend(word.to_le_bytes());
            }
        };
        put(&w[37..40]);
        let mut base: [u32; 15] = w[..15].try_into().unwrap();
        match w[39] {
            0 => {
                if w[MOVING] != 0 {
                    base[1..4].copy_from_slice(&w[23..26]);
                    for (bit, target, source) in [(31, 12, 26), (30, 13, 27), (29, 14, 28)] {
                        if w[22] >> bit & 1 != 0 {
                            base[target] = w[source];
                        }
                    }
                }
                put(&base);
            }
            1 => {
                let mut motion: [u32; 16] = w[21..37].try_into().unwrap();
                if motion[0] != 0 {
                    motion[15] = 0;
                    motion[0] = 1;
                    base[1..4].copy_from_slice(&w[30..33]);
                    for (bit, target, source) in [(31, 12, 33), (30, 13, 34), (29, 14, 35)] {
                        if w[22] >> bit & 1 != 0 {
                            base[target] = w[source];
                        }
                    }
                } else {
                    motion = [0; 16];
                }
                put(&base);
                put(&motion);
            }
            _ => {}
        }
        put(&w[15..19]);
        out
    }

    /// Reader 0x14004ea90 for version 3; words 19/20 and 40.. keep their current values.
    pub(crate) fn restore(&mut self, index: usize, record: &[u8]) -> Result<(), &'static str> {
        let words: Vec<u32> = record
            .get(2..)
            .filter(|_| record.len() % 4 == 2 && record[..2] == [3, 0])
            .ok_or("unsupported camera record version")?
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let (body, expected) = match words.get(2) {
            Some(0) => (15, 22),
            Some(1) => (31, 38),
            Some(_) => (0, 7),
            None => return Err("truncated camera record"),
        };
        if words.len() != expected {
            return Err("camera record length disagrees with its mode");
        }
        let camera = &mut self.cameras[index].words;
        camera[37..40].copy_from_slice(&words[..3]);
        if body != 0 {
            camera[..15].copy_from_slice(&words[3..18]);
            if body == 31 {
                camera[21..37].copy_from_slice(&words[18..34]);
            } else {
                camera[21..37].fill(0);
            }
        }
        camera[15..19].copy_from_slice(&words[3 + body..7 + body]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_uses_alternate_position_only_for_query_mode_one_during_move() {
        let mut camera = Camera::default();
        camera.words[0] = 3;
        for (i, value) in [
            (6, 10.0),
            (7, 8.0),
            (8, 6.0),
            (9, 800.0),
            (10, 600.0),
            (11, 1.5),
            (13, 45.0),
            (15, 400.0),
            (16, 300.0),
            (17, 2.0),
            (18, 0.5),
            (23, 2.0),
            (24, 3.0),
            (25, 4.0),
        ] {
            camera.set(i, value);
        }
        let point = [10.0, -1.0, 24.0];
        let original = camera.project(0, point, 2.0, 40.0);
        assert_eq!(camera.project(1, point, 2.0, 40.0), original);
        camera.words[MOVING] = 1;
        assert_eq!(camera.project(0, point, 2.0, 40.0), original);
        assert_eq!(camera.project(2, point, 2.0, 40.0), original);
        // Horizontal shift is 8, so x matches camera X + shift; Y is unchanged.
        assert_eq!(camera.project(1, point, 2.0, 40.0), [400.0, 1330.0, 0.0]);
    }
}
