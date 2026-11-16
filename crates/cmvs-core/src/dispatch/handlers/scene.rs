//! Scene objects, images, camera motion, effects, and their waits.
use crate::dispatch::AudioHosts;
use crate::dispatch::{Call, Flow, Hosts};
use crate::{audio::AudioBackend, sequence::SequenceContext};
use std::error::Error;

pub(in crate::dispatch) fn root_effects(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    crate::fade::handle_effects(
        call,
        hosts
            .effects
            .as_deref_mut()
            .expect("effects capability checked"),
    )?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_effect_objects(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("effect objects capability checked")
        .release_effect(call, audio_backend(&mut hosts.audio))?;
    Ok(Flow::Done)
}

/// Effect slot replacement runs the old object's destructor, which releases prepared
/// sounds; callers without audio can still replace slots that own none.
fn audio_backend<'h>(audio: &'h mut Option<AudioHosts<'_>>) -> Option<&'h mut dyn AudioBackend> {
    audio
        .as_mut()
        .map(|audio| &mut *audio.backend as &mut dyn AudioBackend)
}

pub(in crate::dispatch) fn root_wait_fade(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.scene
        .as_deref_mut()
        .expect("scene wait capability checked")
        .fade_mut()
        .handle_wait(call, h.gate, h.input)?)
}

pub(in crate::dispatch) fn root_wait_mask(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.scene
        .as_deref_mut()
        .expect("scene wait capability checked")
        .handle_wait_mask(call, h.gate, h.input)?)
}

pub(in crate::dispatch) fn root_wait_quake(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.scene
        .as_deref_mut()
        .expect("scene wait capability checked")
        .handle_wait_quake(call)?)
}

pub(in crate::dispatch) fn root_wait_strip(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.scene
        .as_deref_mut()
        .expect("scene wait capability checked")
        .handle_wait_strip(call, h.gate, h.input)?)
}

pub(in crate::dispatch) fn root_wait_change(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.scene
        .as_deref_mut()
        .expect("scene wait capability checked")
        .handle_wait_change(call, h.gate, h.input)?)
}

pub(in crate::dispatch) fn root_wait_camera(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.camera
        .as_deref_mut()
        .expect("camera wait capability checked")
        .handle_wait_move(call, h.gate, h.input)?)
}

pub(in crate::dispatch) fn root_wait_view(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.waits.as_mut().expect("wait context checked");
    Ok(h.camera
        .as_deref_mut()
        .expect("camera wait capability checked")
        .handle_wait_view(call, h.gate, h.input)?)
}

pub(in crate::dispatch) fn root_sequence(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .sequence
        .as_mut()
        .expect("sequence capability checked");
    let context = h.execution.as_mut().map(|c| SequenceContext {
        scene: &mut *c.scene,
        images: &mut *c.images,
        camera: c.camera.as_deref_mut(),
        messages: c.messages.as_deref_mut(),
    });
    h.host.handle(call, context)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_quake(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked")
        .handle_quake(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_strip(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked")
        .handle_strip(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_scene(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_camera(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .camera
        .as_deref_mut()
        .expect("camera capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_images(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.images.as_mut().expect("image capability checked");
    h.scene.handle_image(call, h.source)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_mask_images(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.images.as_mut().expect("image capability checked");
    h.scene.handle_mask(call, h.source)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_change_image(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.images.as_mut().expect("image capability checked");
    h.scene.handle_change_image(call, h.source)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn collision_mesh(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked")
        .handle_collision_mesh(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_snow_field(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.images.as_mut().expect("image capability checked");
    h.scene
        .create_snow_field(call, h.source, audio_backend(&mut hosts.audio))?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_floating_planes(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let (Some(images), Some(audio)) = (hosts.images.as_mut(), hosts.audio.as_mut()) else {
        return Ok(Flow::Unavailable);
    };
    images
        .scene
        .create_floating_planes(call, images.source, audio.backend)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_magic(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let (Some(images), Some(audio), Some(camera), Some(random)) = (
        hosts.images.as_mut(),
        hosts.audio.as_mut(),
        hosts.camera.as_deref_mut(),
        hosts.random.as_deref_mut(),
    ) else {
        return Ok(Flow::Unavailable);
    };
    images
        .scene
        .create_magic(call, images.source, audio.backend, camera, random)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_vortex(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let (Some(images), Some(audio), Some(camera), Some(random)) = (
        hosts.images.as_mut(),
        hosts.audio.as_mut(),
        hosts.camera.as_deref_mut(),
        hosts.random.as_deref_mut(),
    ) else {
        return Ok(Flow::Unavailable);
    };
    images
        .scene
        .create_vortex(call, images.source, audio.backend, camera, random)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_curtain(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let (Some(images), Some(audio), Some(camera)) = (
        hosts.images.as_mut(),
        hosts.audio.as_mut(),
        hosts.camera.as_deref_mut(),
    ) else {
        return Ok(Flow::Unavailable);
    };
    images
        .scene
        .create_curtain(call, images.source, audio.backend, camera)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_sprites(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let (Some(images), Some(audio)) = (hosts.images.as_mut(), hosts.audio.as_mut()) else {
        return Ok(Flow::Unavailable);
    };
    images
        .scene
        .create_sprites(call, images.source, audio.backend)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_circle(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let (Some(images), Some(audio)) = (hosts.images.as_mut(), hosts.audio.as_mut()) else {
        return Ok(Flow::Unavailable);
    };
    images
        .scene
        .create_circle(call, images.source, audio.backend)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_walk(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let (Some(scene), Some(camera), Some(audio)) = (
        hosts.scene.as_deref_mut(),
        hosts.camera.as_deref_mut(),
        hosts.audio.as_mut(),
    ) else {
        return Ok(Flow::Unavailable);
    };
    scene.create_walk(call, camera, audio.backend)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_query(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let scene = hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked");
    let slot = call.argument(4, "effect slot")?;
    match (slot < 4)
        .then(|| scene.query_legacy_effect(slot as u8))
        .flatten()
    {
        Some(value) => {
            call.complete(0x4004)?;
            call.vm
                .state_mut()
                .set_system_integer(0, value)
                .expect("result slot exists");
        }
        None => scene.query_fragments(call)?,
    }
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_set(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let scene = hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked");
    scene.set_legacy_effect(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn create_rain(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .images
        .as_mut()
        .expect("particle image capability checked");
    h.scene
        .create_rain(call, h.source, audio_backend(&mut hosts.audio))?;
    Ok(Flow::Done)
}

fn particles(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
    x87: bool,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .images
        .as_mut()
        .expect("particle image capability checked");
    h.scene.create_particles(
        call,
        h.source,
        hosts
            .random
            .as_deref_mut()
            .expect("particle random capability checked"),
        x87,
        audio_backend(&mut hosts.audio),
    )?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn create_particles(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    particles(hosts, call, false)
}

pub(in crate::dispatch) fn create_particles_x87(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    particles(hosts, call, true)
}

fn falling_billboards(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
    x87: bool,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .images
        .as_mut()
        .expect("particle image capability checked");
    h.scene.create_falling_effect(
        call,
        h.source,
        hosts
            .random
            .as_deref_mut()
            .expect("particle random capability checked"),
        x87,
        audio_backend(&mut hosts.audio),
    )?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn create_falling_billboards(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    falling_billboards(hosts, call, false)
}

pub(in crate::dispatch) fn create_falling_billboards_x87(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    falling_billboards(hosts, call, true)
}

pub(in crate::dispatch) fn create_fragments(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("scene random capability checked")
        .create_fragments(
            call,
            hosts
                .random
                .as_deref_mut()
                .expect("scene random capability checked"),
            audio_backend(&mut hosts.audio),
        )?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn query_fragments(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref()
        .expect("scene capability checked")
        .query_fragments(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn create_collision_particles(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .images
        .as_mut()
        .expect("particle image capability checked");
    h.scene.create_collision_effect(
        call,
        h.source,
        hosts
            .random
            .as_deref_mut()
            .expect("particle random capability checked"),
        audio_backend(&mut hosts.audio),
    )?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn screen_coordinates(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked")
        .screen_coordinates(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn create_legacy_flakes(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .images
        .as_mut()
        .expect("particle image capability checked");
    h.scene.create_legacy_flakes(
        call,
        h.source,
        hosts
            .random
            .as_deref_mut()
            .expect("particle random capability checked"),
        audio_backend(&mut hosts.audio),
    )?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn draw_query(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .scene
        .as_deref_mut()
        .expect("scene capability checked")
        .draw_query(call)?;
    Ok(Flow::Done)
}
