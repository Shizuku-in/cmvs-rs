use cmvs_rs::{
    dispatch::{self, Capability, Flow, Hosts, InputHosts},
    image::ImageSource,
    input::ActionInput,
    machine::Machine,
    message::MessageHost,
    profile::Semantics,
};
fn accepts(vm: &Machine<'_>, capability: Capability) -> bool {
    vm.pending_system().is_some_and(|(_, c)| {
        dispatch::ROOT
            .iter()
            .any(|e| e.command == c && e.capability == capability)
    })
}
#[allow(dead_code)]
pub trait SharedWindows {
    fn shared_message(&mut self, vm: &mut Machine<'_>) -> Result<bool, Box<dyn std::error::Error>>;
    fn shared_auxiliary_scene(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut cmvs_rs::scene::SceneHost,
    ) -> Result<bool, Box<dyn std::error::Error>>;
    fn shared_window_pointer(
        &mut self,
        vm: &mut Machine<'_>,
        input: &ActionInput,
    ) -> Result<bool, Box<dyn std::error::Error>>;
    fn shared_window_buttons(
        &mut self,
        vm: &mut Machine<'_>,
        input: &mut ActionInput,
    ) -> Result<bool, Box<dyn std::error::Error>>;
    fn shared_window_image(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error>>;
}
impl SharedWindows for MessageHost {
    fn shared_message(&mut self, vm: &mut Machine<'_>) -> Result<bool, Box<dyn std::error::Error>> {
        if !accepts(vm, Capability::Windows) && !accepts(vm, Capability::TextState) {
            return Ok(self.try_handle(vm)?);
        }
        Ok(dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                messages: Some(self),
                ..Default::default()
            },
        )? == Some(Flow::Done))
    }
    fn shared_auxiliary_scene(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut cmvs_rs::scene::SceneHost,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !accepts(vm, Capability::AuxiliaryScene) {
            return Ok(false);
        }
        Ok(dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                messages: Some(self),
                scene: Some(scene),
                ..Default::default()
            },
        )? == Some(Flow::Done))
    }
    fn shared_window_pointer(
        &mut self,
        vm: &mut Machine<'_>,
        input: &ActionInput,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !accepts(vm, Capability::WindowPointer) {
            return Ok(false);
        }
        let mut input = input.clone();
        Ok(dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                messages: Some(self),
                input: Some(InputHosts {
                    input: &mut input,
                    skip: false,
                    clock: None,
                }),
                ..Default::default()
            },
        )? == Some(Flow::Done))
    }
    fn shared_window_buttons(
        &mut self,
        vm: &mut Machine<'_>,
        input: &mut ActionInput,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !accepts(vm, Capability::WindowButtons) {
            return Ok(false);
        }
        Ok(dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                messages: Some(self),
                input: Some(InputHosts {
                    input,
                    skip: false,
                    clock: None,
                }),
                ..Default::default()
            },
        )? == Some(Flow::Done))
    }
    fn shared_window_image(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if !accepts(vm, Capability::WindowImages) {
            return Ok(false);
        }
        Ok(dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                messages: Some(self),
                window_images: Some(source),
                ..Default::default()
            },
        )? == Some(Flow::Done))
    }
}
