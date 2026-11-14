//! Effect objects maintained separately from ordinary scene slots.
//!
//! [`crate::scene::SceneHost`] owns their state. [`crate::runtime::Runtime`] advances
//! the effects during frame updates, and [`crate::scene_render`] prepares them for drawing.

// Engine +4024: four effect objects, independent of the 256 scene slots.
use crate::{bytecode::DecodeError, dispatch::Call, machine::Machine};
mod collision;
mod falling;
mod fragments;
pub use collision::{CollisionEffect, CollisionParticle, ParticleFade};
pub use fragments::{Fragment, FragmentEffect};
mod curtain;
mod legacy;
mod magic;
mod sprites;
mod vortex;
pub use legacy::{LegacyEffect, SnowField, Walk, WorldQuad};
mod legacy_flakes;
pub(crate) use legacy_flakes::LegacyFlakes;
mod rain;
pub(crate) use rain::RainEffect;
mod particles;
mod snapshot;
pub use falling::{FallingEffect, FallingParticle};
pub use particles::{Particle, ParticleEffect};

/// 0x22a0 / 0x14007dbe0: SP-4 slot 0..3 destroys and frees that object (0x140040820) when
/// present; other values are ignored. Returns 0x4004 without a system result.
/// VM-only compatibility helper for an empty object host, which has no effect to
/// destroy. Stateful callers use dispatch.
pub fn try_handle(vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
    let Some((pc, 0x2a0)) = vm.pending_system() else {
        return Ok(false);
    };
    let mut call = Call::new(vm, pc, 0x2a0);
    call.argument(4, "slot")?;
    call.complete(0x4004)?;
    Ok(true)
}

impl crate::scene::SceneHost {
    /// 0x22a0 (0x43cab0 in cmvs-20070119): SP-4 slot 0..3, returns 0x4004.
    pub(crate) fn release_effect(
        &mut self,
        call: &mut Call<'_, '_>,
        backend: Option<&mut dyn crate::audio::AudioBackend>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let slot = call.argument(4, "slot")?;
        if slot < 4 {
            self.replace_effect_slot(slot as u8, backend)?;
        }
        call.complete(0x4004)?;
        Ok(())
    }
}

impl crate::scene::SceneHost {
    /// Pool updates follow the four original slots so mixed types share CRT draws in order.
    pub fn advance_effect_pools(
        &mut self,
        delta: u32,
        random: &mut crate::random::CrtRandom,
        cameras: &crate::camera::CameraHost,
    ) -> Result<(), crate::FormatError> {
        // Type 7 shares the map but never reads the collision planes.
        let triangles = if self
            .collision_effects
            .values()
            .all(CollisionEffect::is_fading)
        {
            Vec::new()
        } else {
            self.collision_triangles().map_err(crate::FormatError)?
        };
        let view = cameras.view(0).expect("view exists");
        let position = [view.words[0], view.words[1], view.words[2]].map(f32::from_bits);
        for slot in 0..4 {
            if let Some(effect) = self.legacy_flakes.get_mut(&slot) {
                effect.advance(delta);
            }
            if let Some(effect) = self.rain_effects.get_mut(&slot) {
                effect.advance(delta, random);
            }
            if let Some(effect) = self.particle_effects.get_mut(&slot) {
                effect.advance(delta, random);
            }
            if let Some(effect) = self.falling_effects.get_mut(&slot) {
                effect.advance(delta, position);
            }
            if let Some(effect) = self.collision_effects.get_mut(&slot) {
                effect.advance(delta, &triangles, random);
            }
            if let Some(effect) = self.fragment_effects.get_mut(&slot) {
                effect.advance(delta);
            }
        }
        Ok(())
    }
}
