//! Engine +4024: four effect objects, independent of the 256 scene slots.
use crate::{bytecode::DecodeError, dispatch::Call, machine::Machine};
mod falling;
mod particles;
pub use falling::{FallingEffect, FallingParticle};
pub use particles::{Particle, ParticleEffect};

/// 0x22a0 / 0x14007dbe0: SP-4 slot 0..3 destroys and frees that object (0x140040820) when
/// present; other values are ignored. Returns 0x4004 without a system result.
/// VM-only compatibility helper for an empty object host. Stateful callers use dispatch.
pub fn try_handle(vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
    let Some((pc, 0x2a0)) = vm.pending_system() else {
        return Ok(false);
    };
    crate::scene::SceneHost::default().release_effect(&mut Call::new(vm, pc, 0x2a0))?;
    Ok(true)
}

impl crate::scene::SceneHost {
    pub(crate) fn release_effect(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
        let slot = call.argument(4, "slot")?;
        call.complete(0x4004)?;
        if slot < 4 {
            self.particle_effects.remove(&(slot as u8));
            self.falling_effects.remove(&(slot as u8));
        }
        Ok(())
    }
}
