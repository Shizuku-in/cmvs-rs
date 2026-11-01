use cmvs_rs::{
    bootstrap::Bootstrap,
    capture::ScreenCapture,
    dispatch::{self, Capability, DispatchError, Flow, FunctionKeys, Hosts, PlatformHosts},
    machine::{Machine, Step},
    platform::{PlatformAssets, PlatformBackend, PlatformEvent, PlatformHost},
    profile::Semantics,
    scene::SceneHost,
};
fn accepts(vm: &Machine<'_>, capability: Capability) -> bool {
    vm.pending_system().is_some_and(|(_, c)| {
        dispatch::ROOT
            .iter()
            .any(|e| e.command == c && e.capability == capability)
    })
}
#[allow(dead_code)]
pub trait SharedPlatform {
    fn shared_platform(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn PlatformBackend,
        assets: &mut dyn PlatformAssets,
        cursors: &[Vec<u8>],
    ) -> Result<Option<PlatformEvent>, DispatchError>;
}
impl SharedPlatform for PlatformHost {
    fn shared_platform(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn PlatformBackend,
        assets: &mut dyn PlatformAssets,
        cursors: &[Vec<u8>],
    ) -> Result<Option<PlatformEvent>, DispatchError> {
        if !accepts(vm, Capability::Platform) {
            return Ok(None);
        }
        match dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                platform: Some(PlatformHosts {
                    host: self,
                    backend,
                    assets,
                    cursors,
                }),
                ..Default::default()
            },
        )? {
            Some(Flow::Platform(event)) => Ok(Some(event)),
            other => panic!("unexpected platform flow {other:?}"),
        }
    }
}
#[allow(dead_code)]
pub trait SharedCapture {
    fn shared_capture(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
    ) -> Result<bool, DispatchError>;
    fn shared_immediate(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        backend: &mut dyn PlatformBackend,
    ) -> Result<bool, DispatchError>;
}
impl SharedCapture for ScreenCapture {
    fn shared_capture(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
    ) -> Result<bool, DispatchError> {
        if !accepts(vm, Capability::Capture) {
            return Ok(false);
        }
        Ok(dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                capture: Some(self),
                scene: Some(scene),
                ..Default::default()
            },
        )? == Some(Flow::Done))
    }
    fn shared_immediate(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        backend: &mut dyn PlatformBackend,
    ) -> Result<bool, DispatchError> {
        if !accepts(vm, Capability::CaptureDevice) {
            return Ok(false);
        }
        Ok(dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                capture: Some(self),
                scene: Some(scene),
                device_frame: Some(backend),
                ..Default::default()
            },
        )? == Some(Flow::Done))
    }
}
#[allow(dead_code)]
pub trait SharedBootstrap {
    fn shared_bootstrap(&mut self, vm: &mut Machine<'_>) -> Result<Option<Step>, DispatchError>;
}
impl SharedBootstrap for Bootstrap {
    fn shared_bootstrap(&mut self, vm: &mut Machine<'_>) -> Result<Option<Step>, DispatchError> {
        if !accepts(vm, Capability::Bootstrap) && !accepts(vm, Capability::FunctionKeys) {
            return Ok(None);
        }
        let mut hosts = Hosts::default();
        if vm.pending_system().unwrap().1 == 0x34d {
            hosts.function_keys = Some(FunctionKeys::Startup(self));
        } else {
            hosts.bootstrap = Some(self);
        }
        match dispatch::run(Semantics::Root, vm, &mut hosts)? {
            Some(Flow::Done) => Ok(Some(Step::Continue)),
            other => panic!("unexpected bootstrap flow {other:?}"),
        }
    }
}
