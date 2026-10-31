//! Engine +11104..+11128: the screen capture requested by 0x2304, taken when the next
//! frame draw ends (0x1400157b0, the last step of 0x140015fc0) and handed to a scene object
//! by 0x203c. 0x203e instead reads the backend's current completed device frame without
//! consuming this held image or its pending request. Addresses bind the documented hash.
use crate::{
    bytecode::DecodeError,
    evaluate::error,
    machine::Machine,
    platform::PlatformBackend,
    raster::RgbaImage,
    scene::{ObjectImage, SceneHost},
    vm::SystemResult,
};

#[derive(Debug, Default)]
pub struct ScreenCapture {
    /// +11112: set by 0x2304, cleared once a frame is taken.
    requested: bool,
    /// +11128 with its size at +11116/+11120.
    image: Option<RgbaImage>,
}

impl ScreenCapture {
    /// 0x14006c6e0 -> 0x14003c180 -> 0x1400330a0. SP-4 existing parent slot,
    /// SP-8 negative; reads the device immediately, result 0 = success, returns 0x4008.
    /// Validate/read first so an unsupported readback can be retried with the old
    /// image intact, rather than reproducing the original's empty partial image.
    pub fn try_immediate(
        &self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        backend: &mut impl PlatformBackend,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let Some((pc, 0x3e)) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, 0x203e, why);
        let slot = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let child = vm.stack().peek_u32(8).map_err(|e| fail(e.0))? as i32;
        let slot = u8::try_from(slot)
            .ok()
            .filter(|_| child < 0)
            .filter(|slot| scene.object(*slot).is_some())
            .ok_or_else(|| fail("device image needs an existing object in slots 0..255 itself"))?;
        let frame = opaque(backend.read_device_frame()?)?;
        vm.complete_system(SystemResult(0x4008))?;
        scene.object_mut(slot).expect("validated").image = Some(ObjectImage {
            filter: None,
            name: Vec::new(),
            pixels: frame,
            alpha: false,
            surface: false,
        });
        vm.state_mut()
            .set_system_integer(0, 1)
            .expect("result 0 exists");
        Ok(true)
    }
    pub fn requested(&self) -> bool {
        self.requested
    }
    pub fn image(&self) -> Option<&RgbaImage> {
        self.image.as_ref()
    }
    /// 0x1400157b0 with SAM_IMAGE_MODE = 0 (+11104): frees an earlier capture, reads the
    /// device-sized frame (0x14001ca00 forces alpha 0xff) and clears the request.
    pub fn take(&mut self, frame: RgbaImage) -> Result<(), crate::FormatError> {
        self.image = Some(opaque(frame)?);
        self.requested = false;
        Ok(())
    }

    /// No stack parameters:
    /// - 0x2304 / 0x1400158f0: request a capture (+11112 = 1).
    /// - 0x2305 / 0x1400158a0: free the capture and clear the request and size.
    /// - 0x2306 / 0x14007fb20 -> 0x1400158e0: system result 0 = whether a capture is held.
    ///
    /// All return 0x4000. 0x203c is described at `give`.
    pub fn try_handle(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
    ) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x3c | 0x304..=0x306))) = vm.pending_system() else {
            return Ok(false);
        };
        if command == 0x3c {
            return self.give(vm, pc, scene);
        }
        vm.complete_system(SystemResult(0x4000))?;
        match command {
            0x304 => self.requested = true,
            0x305 => *self = Self::default(),
            _ => vm
                .state_mut()
                .set_system_integer(0, u32::from(self.image.is_some()))
                .expect("result 0 exists"),
        }
        Ok(true)
    }

    /// 0x203c / 0x14006c580: SP-4 slot (signed < 256 with an object) and SP-8, which must be
    /// negative; otherwise only +12056 bit 0x10 is set. 0x14003c0d0 releases the object
    /// image (+6176) and builds one from the capture (0x140032f80: empty name, +1152 = 0);
    /// result 0 = success, then the capture is reset (0x1400158a0). Pops 8. Without a
    /// capture the original keeps an empty image object, which Rust rejects.
    fn give(
        &mut self,
        vm: &mut Machine<'_>,
        pc: usize,
        scene: &mut SceneHost,
    ) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, 0x203c, why);
        let word = |distance| {
            vm.stack()
                .peek_u32(distance)
                .map(|value| value as i32)
                .map_err(|e| fail(e.0))
        };
        let (slot, child) = (word(4)?, word(8)?);
        let object = u8::try_from(slot)
            .ok()
            .filter(|_| child < 0)
            .and_then(|slot| scene.object_mut(slot))
            .ok_or_else(|| fail("screen image needs an existing object in slots 0..255 itself"))?;
        if self.image.is_none() {
            return Err(fail(
                "screen image without a capture leaves an empty image object",
            ));
        }
        vm.complete_system(SystemResult(0x4008))?;
        object.image = Some(ObjectImage {
            filter: None,
            name: Vec::new(),
            pixels: self.image.take().expect("checked above"),
            alpha: false,
            surface: false,
        });
        *self = Self::default();
        vm.state_mut()
            .set_system_integer(0, 1)
            .expect("result 0 exists");
        Ok(true)
    }
}

/// The software device's 32-bit readback corresponds to A8R8G8B8/X8R8G8B8,
/// which 0x14001ca00 copies with alpha forced to 0xff.
fn opaque(frame: RgbaImage) -> Result<RgbaImage, crate::FormatError> {
    let mut pixels = frame.pixels().to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 0xff;
    }
    RgbaImage::new(frame.width(), frame.height(), pixels)
}
