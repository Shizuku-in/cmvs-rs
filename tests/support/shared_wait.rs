use cmvs_rs::{
    dispatch::DispatchError,
    dispatch::{self, Flow, Hosts, WaitHosts},
    fade::SkipGate,
    input::ActionInput,
    machine::{Machine, Step},
    profile::Semantics,
    scene::SceneHost,
};

pub trait SharedSceneWait {
    fn shared_wait(
        &mut self,
        vm: &mut Machine<'_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Option<Step>, DispatchError>;
}

impl SharedSceneWait for SceneHost {
    fn shared_wait(
        &mut self,
        vm: &mut Machine<'_>,
        gate: SkipGate,
        input: &mut ActionInput,
    ) -> Result<Option<Step>, DispatchError> {
        if !vm
            .pending_system()
            .is_some_and(|(_, command)| dispatch::is_wait(command))
        {
            return Ok(None);
        }
        Ok(
            match dispatch::run(
                Semantics::Root,
                vm,
                &mut Hosts {
                    waits: Some(WaitHosts {
                        scene: Some(self),
                        camera: None,
                        input,
                        gate,
                        change_images: true,
                    }),
                    ..Default::default()
                },
            )? {
                Some(Flow::Done) => Some(Step::Continue),
                Some(Flow::Wait | Flow::Yield) => Some(Step::Yield),
                None => None,
                other => panic!("expected a supported scene wait, got {other:?}"),
            },
        )
    }
}
