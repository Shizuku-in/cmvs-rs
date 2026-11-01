use cmvs_rs::{
    camera::CameraHost,
    dispatch::{self, DispatchError, Flow, Hosts},
    machine::Machine,
    profile::Semantics,
    scene::SceneHost,
};

pub trait SharedState {
    fn shared_state(&mut self, vm: &mut Machine<'_>) -> Result<bool, DispatchError>;
}
impl SharedState for SceneHost {
    fn shared_state(&mut self, vm: &mut Machine<'_>) -> Result<bool, DispatchError> {
        Ok(
            match dispatch::run(
                Semantics::Root,
                vm,
                &mut Hosts {
                    scene: Some(self),
                    ..Default::default()
                },
            )? {
                Some(Flow::Done) => true,
                None | Some(Flow::Unavailable) => false,
                other => panic!("unexpected scene flow: {other:?}"),
            },
        )
    }
}
impl SharedState for CameraHost {
    fn shared_state(&mut self, vm: &mut Machine<'_>) -> Result<bool, DispatchError> {
        Ok(
            match dispatch::run(
                Semantics::Root,
                vm,
                &mut Hosts {
                    camera: Some(self),
                    ..Default::default()
                },
            )? {
                Some(Flow::Done) => true,
                None | Some(Flow::Unavailable) => false,
                other => panic!("unexpected camera flow: {other:?}"),
            },
        )
    }
}
