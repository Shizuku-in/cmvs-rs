use cmvs_rs::{
    audio::{AudioBackend, AudioHost},
    dispatch::{self, AudioHosts, Flow, Hosts},
    machine::Machine,
    profile::Semantics,
};

pub trait SharedAudio {
    fn shared_handle(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn std::error::Error>>;
}

impl SharedAudio for AudioHost {
    fn shared_handle(
        &mut self,
        vm: &mut Machine<'_>,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(
            match dispatch::run(
                Semantics::Root,
                vm,
                &mut Hosts {
                    audio: Some(AudioHosts {
                        audio: self,
                        backend,
                    }),
                    ..Default::default()
                },
            )? {
                Some(Flow::Done) => true,
                None => false,
                other => panic!("expected an audio completion, got {other:?}"),
            },
        )
    }
}
