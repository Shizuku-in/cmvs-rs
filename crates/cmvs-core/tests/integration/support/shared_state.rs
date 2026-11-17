use cmvs_core::{
    format::snapshot::SaveImage, format::snapshot::SaveInfo, format::snapshot::SaveTitle,
    host::gesture::PointerGesture, host::random::CrtRandom, host::settings::SettingsHost,
    profile::Semantics, render::camera::CameraHost, render::fade::FadeHost,
    render::scene::SceneHost, runtime::HistoryHost, runtime::MessageFlags, runtime::VmTimers,
    runtime::save_menu::SaveMenuHost, vm::dispatch, vm::dispatch::DispatchError,
    vm::dispatch::Flow, vm::dispatch::Hosts, vm::dispatch::SnapshotHosts, vm::machine::Machine,
};

pub trait SharedState {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError>;
}

fn run_done(vm: &mut Machine, hosts: &mut Hosts<'_>) -> Result<bool, DispatchError> {
    Ok(match dispatch::run(Semantics::Root, vm, hosts)? {
        Some(Flow::Done) => true,
        None | Some(Flow::Unavailable) => false,
        other => panic!("unexpected state flow: {other:?}"),
    })
}

impl SharedState for SceneHost {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                scene: Some(self),
                ..Default::default()
            },
        )
    }
}

impl SharedState for FadeHost {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        let mut scene = SceneHost::default();
        std::mem::swap(scene.fade_mut(), self);
        let result = scene.shared_state(vm);
        std::mem::swap(scene.fade_mut(), self);
        result
    }
}

impl SharedState for CameraHost {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                camera: Some(self),
                ..Default::default()
            },
        )
    }
}

impl SharedState for PointerGesture {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                gesture: Some(self),
                ..Default::default()
            },
        )
    }
}

impl SharedState for HistoryHost {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                history: Some(self),
                ..Default::default()
            },
        )
    }
}

impl SharedState for CrtRandom {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                random: Some(self),
                ..Default::default()
            },
        )
    }
}

impl SharedState for MessageFlags {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                flags: Some(self),
                ..Default::default()
            },
        )
    }
}

impl SharedState for VmTimers {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                timers: Some(self),
                ..Default::default()
            },
        )
    }
}

impl SharedState for SettingsHost {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                settings: Some(self),
                ..Default::default()
            },
        )
    }
}

impl SharedState for SaveMenuHost {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                popup: Some(self),
                ..Default::default()
            },
        )
    }
}

impl SharedState for SaveTitle {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                save_title: Some(self),
                ..Default::default()
            },
        )
    }
}

impl SharedState for SaveInfo {
    fn shared_state(&mut self, vm: &mut Machine) -> Result<bool, DispatchError> {
        run_done(
            vm,
            &mut Hosts {
                save_info: Some(self),
                ..Default::default()
            },
        )
    }
}

pub trait SharedSnapshotUnavailable {
    fn shared_snapshot_unavailable(
        &mut self,
        vm: &mut Machine,
        continue_without_image: bool,
    ) -> Result<bool, DispatchError>;
}

impl SharedSnapshotUnavailable for SaveImage {
    fn shared_snapshot_unavailable(
        &mut self,
        vm: &mut Machine,
        continue_without_image: bool,
    ) -> Result<bool, DispatchError> {
        if !continue_without_image {
            return Ok(false);
        }
        run_done(
            vm,
            &mut Hosts {
                snapshot: Some(SnapshotHosts {
                    image: self,
                    capture: false,
                    continue_without_image: true,
                }),
                ..Default::default()
            },
        )
    }
}

pub fn shared_effects(vm: &mut Machine, effects: &mut bool) -> Result<bool, DispatchError> {
    run_done(
        vm,
        &mut Hosts {
            effects: Some(effects),
            ..Default::default()
        },
    )
}

pub fn shared_math(vm: &mut Machine) -> Result<bool, DispatchError> {
    run_done(
        vm,
        &mut Hosts {
            math: true,
            ..Default::default()
        },
    )
}

pub fn shared_strings(vm: &mut Machine) -> Result<bool, DispatchError> {
    run_done(
        vm,
        &mut Hosts {
            strings: true,
            ..Default::default()
        },
    )
}
