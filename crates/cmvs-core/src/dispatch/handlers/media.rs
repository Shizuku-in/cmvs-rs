//! Audio and movie playback.
use crate::dispatch::{Call, Flow, Hosts};
use std::error::Error;

pub(in crate::dispatch) fn root_audio(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.audio.as_mut().expect("audio capability checked");
    if call.command >= 0x400 {
        h.audio.handle_cvns(call, h.backend)?;
    } else {
        h.audio.handle(call, h.backend)?;
    }
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_movie_state(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .handle(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_movie_start(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .handle_start_scene(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_movie_scene(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .movie_scene
        .as_mut()
        .expect("scene movie capability checked");
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .handle_create_scene(call, h.scene, h.source)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn root_movie_player(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .movie_player
        .as_mut()
        .expect("movie player capability checked");
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .handle_player(call, h.source, h.settings, h.backend)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn mgv_player(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .movie_player
        .as_mut()
        .expect("movie player capability checked");
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .handle_mgv(call, h.source, h.settings, h.backend)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_movie_control(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .legacy_control(call)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn legacy_movie_seek(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts
        .movie_scene
        .as_mut()
        .expect("movie scene capability checked");
    hosts
        .movies
        .as_deref_mut()
        .expect("movie capability checked")
        .seek_scene(call, h.scene)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn preload_audio(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.audio.as_mut().expect("audio capability checked");
    h.audio.handle_preload(call, h.backend)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn se_gain(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.audio.as_mut().expect("audio capability checked");
    h.audio.set_se_gain(call, h.backend)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn mixer_gain(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.audio.as_mut().expect("audio capability checked");
    h.audio.set_mixer_gain(call, h.backend)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn background_audio(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.audio.as_mut().expect("audio capability checked");
    h.audio.handle_background_output(call, h.backend)?;
    Ok(Flow::Done)
}

pub(in crate::dispatch) fn cvns_master_volume(
    hosts: &mut Hosts<'_>,
    call: &mut Call<'_, '_>,
) -> Result<Flow, Box<dyn Error>> {
    let h = hosts.audio.as_mut().expect("audio capability checked");
    h.audio.set_master_percent(call, h.backend)?;
    Ok(Flow::Done)
}
