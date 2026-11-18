use cmvs_core::{
    host::image::ImageSource, host::input::ActionInput, profile::Semantics,
    render::message::MessageHost, vm::dispatch, vm::dispatch::Capability, vm::dispatch::Flow,
    vm::dispatch::Hosts, vm::dispatch::InputHosts, vm::dispatch::TextHosts, vm::machine::Machine,
};
fn accepts(vm: &Machine, capability: Capability) -> bool {
    vm.pending_system().is_some_and(|(_, c)| {
        dispatch::entry(Semantics::Root, c).is_some_and(|e| e.capability == capability)
    })
}
pub trait SharedWindows {
    fn shared_message(
        &mut self,
        vm: &mut Machine,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
    fn shared_auxiliary_scene(
        &mut self,
        vm: &mut Machine,
        scene: &mut cmvs_core::render::scene::SceneHost,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
    fn shared_window_pointer(
        &mut self,
        vm: &mut Machine,
        input: &ActionInput,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
    fn shared_window_buttons(
        &mut self,
        vm: &mut Machine,
        input: &mut ActionInput,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
    fn shared_window_image(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
}
impl SharedWindows for MessageHost {
    fn shared_message(
        &mut self,
        vm: &mut Machine,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        if accepts(vm, Capability::Text) {
            let mut input = ActionInput::default();
            return Ok(matches!(
                dispatch::run(
                    Semantics::Root,
                    vm,
                    &mut Hosts {
                        text: Some(TextHosts {
                            messages: self,
                            input: &mut input,
                            flags: Default::default(),
                        }),
                        ..Default::default()
                    },
                )?,
                Some(Flow::Done)
            ));
        }
        if accepts(vm, Capability::AuxiliaryScene) {
            return match dispatch::run(
                Semantics::Root,
                vm,
                &mut Hosts {
                    messages: Some(self),
                    ..Default::default()
                },
            )? {
                Some(Flow::Done) => Ok(true),
                Some(Flow::Unavailable) => {
                    Err("bound auxiliary position requires scene context".into())
                }
                _ => Ok(false),
            };
        }
        if !accepts(vm, Capability::Windows) && !accepts(vm, Capability::TextState) {
            return Ok(false);
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
        vm: &mut Machine,
        scene: &mut cmvs_core::render::scene::SceneHost,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
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
        vm: &mut Machine,
        input: &ActionInput,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
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
        vm: &mut Machine,
        input: &mut ActionInput,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
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
        vm: &mut Machine,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
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
