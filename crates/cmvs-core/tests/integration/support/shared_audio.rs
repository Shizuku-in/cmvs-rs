use cmvs_core::{
    host::audio::AudioBackend, host::audio::AudioHost, profile::Semantics, vm::dispatch,
    vm::dispatch::AudioHosts, vm::dispatch::Flow, vm::dispatch::Hosts, vm::machine::Machine,
};

pub trait SharedAudio {
    fn shared_handle(
        &mut self,
        vm: &mut Machine,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
}

impl SharedAudio for AudioHost {
    fn shared_handle(
        &mut self,
        vm: &mut Machine,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
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
