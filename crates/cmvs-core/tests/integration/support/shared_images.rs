use cmvs_core::{
    dispatch::{self, Capability, Flow, Hosts, ImageHosts},
    image::ImageSource,
    machine::Machine,
    profile::Semantics,
    scene::SceneHost,
};

pub trait SharedImages {
    fn shared_image(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error>>;
    fn shared_mask(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error>>;
    fn shared_change(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error>>;
}

fn run(
    scene: &mut SceneHost,
    vm: &mut Machine<'_>,
    source: &mut dyn ImageSource,
    capability: Capability,
) -> Result<bool, Box<dyn std::error::Error>> {
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
        vm: &mut Machine<'_>,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        run(self, vm, source, Capability::SceneImages)
    }
    fn shared_mask(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        run(self, vm, source, Capability::MaskImages)
    }
    fn shared_change(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        run(self, vm, source, Capability::ImageChange)
    }
}
