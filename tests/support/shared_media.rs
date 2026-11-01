use cmvs_rs::{
    audio::{AudioBackend, AudioSettings},
    camera::CameraHost,
    dispatch::{
        self, Capability, DispatchError, Flow, Hosts, MoviePlayerHosts, MovieSceneHosts,
        SequenceHosts,
    },
    image::ImageSource,
    machine::Machine,
    movie::{MovieHost, MovieSource},
    profile::Semantics,
    resources::ResourceSource,
    scene::SceneHost,
    sequence::{SequenceContext, SequenceHost},
};
fn run(
    vm: &mut Machine<'_>,
    hosts: &mut Hosts<'_>,
    capability: Capability,
) -> Result<bool, DispatchError> {
    if !vm.pending_system().is_some_and(|(_, c)| {
        dispatch::ROOT
            .iter()
            .any(|e| e.command == c && e.capability == capability)
    }) {
        return Ok(false);
    }
    Ok(dispatch::run(Semantics::Root, vm, hosts)? == Some(Flow::Done))
}
#[allow(dead_code)]
pub trait SharedMovies {
    fn shared_movie(&mut self, vm: &mut Machine<'_>) -> Result<bool, DispatchError>;
    fn shared_create_scene(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        source: &mut dyn ResourceSource,
    ) -> Result<bool, DispatchError>;
    fn shared_start_scene(&mut self, vm: &mut Machine<'_>) -> Result<bool, DispatchError>;
    fn shared_player(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn MovieSource,
        settings: AudioSettings,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, DispatchError>;
}
impl SharedMovies for MovieHost {
    fn shared_movie(&mut self, vm: &mut Machine<'_>) -> Result<bool, DispatchError> {
        run(
            vm,
            &mut Hosts {
                movies: Some(self),
                ..Default::default()
            },
            Capability::MovieState,
        )
    }
    fn shared_create_scene(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        source: &mut dyn ResourceSource,
    ) -> Result<bool, DispatchError> {
        run(
            vm,
            &mut Hosts {
                movies: Some(self),
                movie_scene: Some(MovieSceneHosts { scene, source }),
                ..Default::default()
            },
            Capability::MovieScene,
        )
    }
    fn shared_start_scene(&mut self, vm: &mut Machine<'_>) -> Result<bool, DispatchError> {
        run(
            vm,
            &mut Hosts {
                movies: Some(self),
                scene_movie_playback: true,
                ..Default::default()
            },
            Capability::MovieStart,
        )
    }
    fn shared_player(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn MovieSource,
        settings: AudioSettings,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, DispatchError> {
        run(
            vm,
            &mut Hosts {
                movies: Some(self),
                movie_player: Some(MoviePlayerHosts {
                    source,
                    settings,
                    backend,
                }),
                ..Default::default()
            },
            Capability::MoviePlayer,
        )
    }
}
#[allow(dead_code)]
pub trait SharedSequence {
    fn shared_sequence(&mut self, vm: &mut Machine<'_>) -> Result<bool, DispatchError>;
    fn shared_sequence_images(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
    ) -> Result<bool, DispatchError>;
    fn shared_sequence_hosts(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
        camera: &mut CameraHost,
    ) -> Result<bool, DispatchError>;
}
impl SharedSequence for SequenceHost {
    fn shared_sequence(&mut self, vm: &mut Machine<'_>) -> Result<bool, DispatchError> {
        run(
            vm,
            &mut Hosts {
                sequence: Some(SequenceHosts {
                    host: self,
                    execution: None,
                }),
                ..Default::default()
            },
            Capability::Sequence,
        )
    }
    fn shared_sequence_images(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
    ) -> Result<bool, DispatchError> {
        run(
            vm,
            &mut Hosts {
                sequence: Some(SequenceHosts {
                    host: self,
                    execution: Some(SequenceContext {
                        scene,
                        images,
                        camera: None,
                    }),
                }),
                ..Default::default()
            },
            Capability::Sequence,
        )
    }
    fn shared_sequence_hosts(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
        camera: &mut CameraHost,
    ) -> Result<bool, DispatchError> {
        run(
            vm,
            &mut Hosts {
                sequence: Some(SequenceHosts {
                    host: self,
                    execution: Some(SequenceContext {
                        scene,
                        images,
                        camera: Some(camera),
                    }),
                }),
                ..Default::default()
            },
            Capability::Sequence,
        )
    }
}
