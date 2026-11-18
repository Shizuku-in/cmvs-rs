use cmvs_core::{
    host::gesture::PointerGesture, host::input::ActionInput, profile::Semantics,
    render::scene::SceneHost, runtime::ButtonHost, vm::dispatch, vm::dispatch::Capability,
    vm::dispatch::DispatchError, vm::dispatch::Flow, vm::dispatch::FunctionKeys,
    vm::dispatch::Hosts, vm::dispatch::InputHosts, vm::machine::Machine,
};

pub fn input(
    vm: &mut Machine,
    input: &mut ActionInput,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    if vm.pending_system().is_some_and(|(_, c)| c == 0x34d) {
        return Ok(matches!(
            dispatch::run(
                Semantics::Root,
                vm,
                &mut Hosts {
                    function_keys: Some(FunctionKeys::Input(input)),
                    ..Default::default()
                }
            )?,
            Some(Flow::Done)
        ));
    }
    Ok(matches!(
        dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                input: Some(InputHosts {
                    input,
                    skip: false,
                    clock: None
                }),
                ..Default::default()
            }
        )?,
        Some(Flow::Done)
    ))
}
pub fn skipping(vm: &mut Machine, input: &ActionInput, skip: bool) -> Result<bool, DispatchError> {
    let mut input = input.clone();
    Ok(matches!(
        dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                input: Some(InputHosts {
                    input: &mut input,
                    skip,
                    clock: None
                }),
                ..Default::default()
            }
        )?,
        Some(Flow::Done)
    ))
}

pub trait SharedControls {
    fn shared_buttons(
        &mut self,
        vm: &mut Machine,
        scene: &mut SceneHost,
    ) -> Result<bool, DispatchError>;
    fn shared_button_input(
        &mut self,
        vm: &mut Machine,
        scene: &mut SceneHost,
        input: &mut ActionInput,
    ) -> Result<bool, DispatchError>;
    fn shared_list_input(
        &mut self,
        vm: &mut Machine,
        scene: &mut SceneHost,
        input: &mut ActionInput,
        gesture: &PointerGesture,
    ) -> Result<bool, DispatchError>;
}
fn accepts(vm: &Machine, capability: Capability) -> bool {
    vm.pending_system().is_some_and(|(_, c)| {
        dispatch::entry(Semantics::Root, c).is_some_and(|e| e.capability == capability)
    })
}
impl SharedControls for ButtonHost {
    fn shared_buttons(
        &mut self,
        vm: &mut Machine,
        scene: &mut SceneHost,
    ) -> Result<bool, DispatchError> {
        if !accepts(vm, Capability::Buttons) {
            return Ok(false);
        }
        Ok(matches!(
            dispatch::run(
                Semantics::Root,
                vm,
                &mut Hosts {
                    buttons: Some(self),
                    scene: Some(scene),
                    ..Default::default()
                }
            )?,
            Some(Flow::Done)
        ))
    }
    fn shared_button_input(
        &mut self,
        vm: &mut Machine,
        scene: &mut SceneHost,
        input: &mut ActionInput,
    ) -> Result<bool, DispatchError> {
        if !accepts(vm, Capability::ButtonInput) {
            return Ok(false);
        }
        Ok(matches!(
            dispatch::run(
                Semantics::Root,
                vm,
                &mut Hosts {
                    buttons: Some(self),
                    scene: Some(scene),
                    input: Some(InputHosts {
                        input,
                        skip: false,
                        clock: None
                    }),
                    ..Default::default()
                }
            )?,
            Some(Flow::Done)
        ))
    }
    fn shared_list_input(
        &mut self,
        vm: &mut Machine,
        scene: &mut SceneHost,
        input: &mut ActionInput,
        gesture: &PointerGesture,
    ) -> Result<bool, DispatchError> {
        if !accepts(vm, Capability::ListInput) {
            return Ok(false);
        }
        let mut gesture = gesture.clone();
        Ok(matches!(
            dispatch::run(
                Semantics::Root,
                vm,
                &mut Hosts {
                    buttons: Some(self),
                    scene: Some(scene),
                    input: Some(InputHosts {
                        input,
                        skip: false,
                        clock: None
                    }),
                    gesture: Some(&mut gesture),
                    list_input: true,
                    ..Default::default()
                }
            )?,
            Some(Flow::Done)
        ))
    }
}
