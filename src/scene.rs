//! Platform-independent display state set by verified scene commands.
//! Renderers read it each frame; this module draws nothing and owns no device.
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    image::{self, ImageSource},
    machine::Machine,
    raster::RgbaImage,
    vm::SystemResult,
};
use std::{collections::BTreeMap, error::Error};

/// Engine +1976 holds 256 object pointers (snapshot records 0x700).
pub const OBJECT_SLOTS: usize = 256;

/// CD3DDevice +84/+80; constructor 0x14001d5d0 and reset 0x140080700 both zero them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClearState {
    /// 0x22cb keeps the raw u32; frames clear to opaque 0x00RRGGBB from its low 24 bits.
    pub color: u32,
    /// 0x2250: also clear depth to 1.0, not only the target.
    pub depth: bool,
}
impl ClearState {
    pub fn rgba(&self) -> [u8; 4] {
        let [b, g, r, _] = self.color.to_le_bytes();
        [r, g, b, 255]
    }
}

/// Object +6176 after a successful 0x2030 (image object built by 0x14002fd00).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectImage {
    /// File name as given to 0x2030 (image +1044).
    pub name: Vec<u8>,
    pub pixels: RgbaImage,
    /// Image +1152: the source has 32 bits per pixel.
    pub alpha: bool,
}

/// A 0x38c8-byte object freshly built by 0x14003a3c0, including 768 child pointers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneObject {
    /// None until 0x2030 loads an image; a quiet decode failure also leaves it empty.
    pub image: Option<ObjectImage>,
    /// 768 child pointers at +8, addressed by 0x2022/0x2038/0x2050.
    pub children: BTreeMap<u16, SceneObject>,
    /// First 88 bytes of +6192, initialized by 0x14001ee40.
    pub draw: Option<DrawState>,
    /// +6248: inheritance flags; bit 31 records child creation.
    pub flags: u32,
}
impl Default for SceneObject {
    fn default() -> Self {
        Self {
            image: None,
            children: BTreeMap::new(),
            draw: None,
            flags: 0x7fff_ffff,
        }
    }
}

/// Draw fields retained as original words (offset / 4). Unimplemented setters stay pending.
/// 0=coordinate mode, 1=blend, 2..6=source x/y/width/height, 6..8=origin,
/// 8..10=position, 10=order, 17=opacity, 18/19=scale bits, 20=rotation, 21=fade factor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawState {
    pub words: [u32; 22],
}
impl Default for DrawState {
    fn default() -> Self {
        let mut words = [0; 22];
        words[1] = 1;
        words[17] = 255;
        words[18] = 1.0f32.to_bits();
        words[19] = 1.0f32.to_bits();
        words[21] = 256;
        Self { words }
    }
}

#[derive(Debug, Default)]
pub struct SceneHost {
    clear: ClearState,
    objects: BTreeMap<u8, SceneObject>,
}
impl SceneHost {
    pub fn clear(&self) -> ClearState {
        self.clear
    }
    pub fn object(&self, slot: u8) -> Option<&SceneObject> {
        self.objects.get(&slot)
    }
    pub fn objects(&self) -> &BTreeMap<u8, SceneObject> {
        &self.objects
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        if matches!(command, 0x22 | 0x40 | 0x50 | 0x44..=0x48 | 0x4b) {
            return self.handle_object(vm, pc, command);
        }
        if !matches!(command, 0x2cb | 0x250 | 0x20 | 0x21) {
            return Ok(false);
        }
        let op = command + 0x2000;
        let value = vm.stack().peek_u32(4).map_err(|e| error(pc, op, e.0))?;
        let slot = if matches!(command, 0x20 | 0x21) {
            // 0x14006bb00/0x14006bbc0 compare signed against 256: larger values only set
            // +12056 bit 0x10 (unmodeled) and negative values would index before the table.
            match u8::try_from(value as i32) {
                Ok(slot) => Some(slot),
                Err(_) => return Err(error(pc, op, "object slot outside 0..255")),
            }
        } else {
            None
        };
        vm.complete_system(SystemResult(0x4004))?;
        match (command, slot) {
            (0x2cb, _) => self.clear.color = value,
            (0x250, _) => self.clear.depth = value != 0,
            // 0x2020 destroys any existing object in the slot before building a new one.
            (0x20, Some(slot)) => {
                self.objects.insert(slot, SceneObject::default());
            }
            (_, Some(slot)) => {
                self.objects.remove(&slot);
            }
            _ => unreachable!("slot is parsed for object commands"),
        }
        Ok(true)
    }

    fn handle_object(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        command: u16,
    ) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, command + 0x2000, why);
        let count = match command {
            0x40 => 1,
            0x22 | 0x50 => 2,
            0x44 => 6,
            0x45 | 0x46 => 4,
            0x47 | 0x48 | 0x4b => 3,
            _ => unreachable!(),
        };
        let mut args = Vec::with_capacity(count);
        for i in 1..=count {
            args.push(vm.stack().peek_u32(i * 4).map_err(|e| fail(e.0))?);
        }
        let slot = u8::try_from(args[0]).map_err(|_| fail("object slot outside 0..255"))?;
        let parent = self
            .objects
            .get(&slot)
            .ok_or_else(|| fail("object slot does not exist"))?;
        let child =
            if command == 0x40 || (matches!(command, 0x44..=0x48 | 0x4b) && (args[1] as i32) < 0) {
                None
            } else {
                Some(
                    u16::try_from(args[1])
                        .ok()
                        .filter(|v| *v < 768)
                        .ok_or_else(|| fail("child slot outside 0..767"))?,
                )
            };
        if command != 0x22 {
            let object = match child {
                Some(id) => parent
                    .children
                    .get(&id)
                    .ok_or_else(|| fail("child object does not exist"))?,
                None => parent,
            };
            if matches!(command, 0x44..=0x48 | 0x4b) && object.draw.is_none() {
                return Err(fail("object has no draw state"));
            }
        }
        vm.complete_system(SystemResult(0x4000 | (count * 4) as u16))?;
        let parent = self.objects.get_mut(&slot).expect("validated");
        if command == 0x22 {
            let flags = if parent.draw.is_some() {
                0x7fff_ffff
            } else {
                0
            };
            parent.children.insert(
                child.expect("child"),
                SceneObject {
                    flags,
                    ..SceneObject::default()
                },
            );
            parent.flags |= 0x8000_0000;
        } else {
            let object = match child {
                Some(id) => parent.children.get_mut(&id).expect("validated"),
                None => parent,
            };
            match command {
                0x40 | 0x50 => object.draw = Some(DrawState::default()),
                0x44 => object.draw.as_mut().expect("validated").words[2..6]
                    .copy_from_slice(&args[2..6]),
                0x46 => object.draw.as_mut().expect("validated").words[6..8]
                    .copy_from_slice(&args[2..4]),
                0x45 => object.draw.as_mut().expect("validated").words[8..10]
                    .copy_from_slice(&args[2..4]),
                0x47 => object.draw.as_mut().expect("validated").words[10] = args[2],
                0x48 => {
                    object.draw.as_mut().expect("validated").words[17] =
                        (args[2] as i32).min(255) as u32
                }
                0x4b => {
                    if args[2] <= 3 {
                        object.draw.as_mut().expect("validated").words[1] = args[2];
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok(true)
    }

    /// 0x2030 / 0x14006c220: SP-4 object slot, SP-8 file name. Rebuilds the object image
    /// (0x14003bb10) and sets system result 0 to whether loading succeeded; returns 0x4008.
    /// 0x2038 uses SP-8 child slot and SP-12 name, and returns 0x400c.
    /// +12032 is set only by 0x212a, which stays unhandled, so its 0x140018bc0 call never runs.
    pub fn try_handle_image(
        &mut self,
        vm: &mut Machine<'_>,
        images: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, command @ (0x30 | 0x38))) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, command + 0x2000, why);
        let raw_slot = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let raw_name = vm
            .stack()
            .peek_u32(if command == 0x38 { 12 } else { 8 })
            .map_err(|e| fail(e.0))?;
        // Slots >= 256 or without an object only set +12056 bit 0x10 (unmodeled);
        // negative slots would index before the table.
        let slot = u8::try_from(raw_slot as i32)
            .ok()
            .filter(|slot| self.objects.contains_key(slot))
            .ok_or_else(|| fail("image load needs an existing object in slots 0..255"))?;
        let child = if command == 0x38 {
            let id = vm.stack().peek_u32(8).map_err(|e| fail(e.0))?;
            Some(
                u16::try_from(id)
                    .ok()
                    .filter(|id| *id < 768 && self.objects[&slot].children.contains_key(id))
                    .ok_or_else(|| fail("image load needs an existing child in slots 0..767"))?,
            )
        } else {
            None
        };
        let name = vm.resolve_string(raw_name)?;
        if name.len() >= 64 {
            return Err(fail("image name exceeds the original 64-byte buffers").into());
        }
        let bytes = images.read(&name)?.ok_or_else(|| {
            fail("image not found in any 0x2011 entry; the original shows a modal MessageBox")
        })?;
        let decoded = image::decode(&bytes)
            .map_err(|e| format!("0x{:04x} {}: {e}", command + 0x2000, name.escape_ascii()))?;
        vm.complete_system(SystemResult(if child.is_some() { 0x400c } else { 0x4008 }))?;
        vm.state_mut()
            .set_system_integer(0, u32::from(decoded.is_some()))
            .expect("slot 0 exists");
        let parent = self.objects.get_mut(&slot).expect("checked above");
        let object = match child {
            Some(id) => parent.children.get_mut(&id).expect("checked above"),
            None => parent,
        };
        object.image = decoded.map(|decoded| ObjectImage {
            name,
            pixels: decoded.pixels,
            alpha: decoded.alpha,
        });
        Ok(true)
    }
}
