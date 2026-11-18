use cmvs_core::{
    host::image::ImageSource, profile::Semantics, render::scene::SceneHost, vm::dispatch,
    vm::dispatch::Capability, vm::dispatch::Flow, vm::dispatch::Hosts, vm::dispatch::ImageHosts,
    vm::machine::Machine,
};

pub trait SharedImages {
    fn shared_image(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
    fn shared_mask(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
    fn shared_change(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
}

fn run(
    scene: &mut SceneHost,
    vm: &mut Machine,
    source: &mut dyn ImageSource,
    capability: Capability,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    if !vm.pending_system().is_some_and(|(_, command)| {
        dispatch::entry(Semantics::Root, command)
            .is_some_and(|entry| entry.capability == capability)
    }) {
        return Ok(false);
    }
    Ok(
        match dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                images: Some(ImageHosts {
                    scene,
                    source,
                    change_images: true,
                }),
                ..Default::default()
            },
        )? {
            Some(Flow::Done) => true,
            other => panic!("expected image completion, got {other:?}"),
        },
    )
}
impl SharedImages for SceneHost {
    fn shared_image(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        run(self, vm, source, Capability::SceneImages)
    }
    fn shared_mask(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        run(self, vm, source, Capability::MaskImages)
    }
    fn shared_change(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        run(self, vm, source, Capability::ImageChange)
    }
}
