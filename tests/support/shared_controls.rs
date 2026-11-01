use cmvs_rs::{
    buttons::ButtonHost,
    dispatch::{self, Capability, DispatchError, Flow, Hosts, InputHosts},
    gesture::PointerGesture,
    input::ActionInput,
    machine::Machine,
    profile::Semantics,
    scene::SceneHost,
};

pub fn input(
    vm: &mut Machine<'_>,
    input: &mut ActionInput,
) -> Result<bool, Box<dyn std::error::Error>> {
    if vm.pending_system().is_some_and(|(_, c)| c == 0x34d) {
        return Ok(cmvs_rs::input::try_handle(vm, input)?);
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
pub fn skipping(
    vm: &mut Machine<'_>,
    input: &ActionInput,
    skip: bool,
) -> Result<bool, DispatchError> {
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

#[allow(dead_code)]
pub trait SharedControls {
    fn shared_buttons(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
    ) -> Result<bool, DispatchError>;
    fn shared_button_input(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        input: &mut ActionInput,
    ) -> Result<bool, DispatchError>;
    fn shared_list_input(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        input: &mut ActionInput,
        gesture: &PointerGesture,
    ) -> Result<bool, DispatchError>;
}
fn accepts(vm: &Machine<'_>, capability: Capability) -> bool {
    vm.pending_system().is_some_and(|(_, c)| {
        dispatch::ROOT
            .iter()
            .any(|e| e.command == c && e.capability == capability)
    })
}
impl SharedControls for ButtonHost {
    fn shared_buttons(
        &mut self,
        vm: &mut Machine<'_>,
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
        vm: &mut Machine<'_>,
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
        vm: &mut Machine<'_>,
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
