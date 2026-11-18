use cmvs_core::{
    host::audio::RecordingAudio, host::input::ActionInput, profile::Semantics,
    render::message::AutoAdvanceContext, render::message::MessageHost, runtime::MessageFlags,
    vm::dispatch, vm::dispatch::Capability, vm::dispatch::Flow, vm::dispatch::Hosts,
    vm::dispatch::MessageWaitHosts, vm::machine::Machine, vm::machine::Step,
};
use std::error::Error;
pub trait SharedMessageWait {
    fn shared_wait(
        &mut self,
        vm: &mut Machine,
        input: &mut ActionInput,
        flags: MessageFlags,
    ) -> Result<Option<Step>, Box<dyn Error + Send + Sync>>;
    fn shared_wait_auto(
        &mut self,
        vm: &mut Machine,
        input: &mut ActionInput,
        flags: MessageFlags,
        context: AutoAdvanceContext<'_>,
    ) -> Result<Option<Step>, Box<dyn Error + Send + Sync>>;
}
impl SharedMessageWait for MessageHost {
    fn shared_wait(
        &mut self,
        vm: &mut Machine,
        input: &mut ActionInput,
        flags: MessageFlags,
    ) -> Result<Option<Step>, Box<dyn Error + Send + Sync>> {
        self.shared_wait_auto(
            vm,
            input,
            flags,
            AutoAdvanceContext {
                settings: Default::default(),
                delta_ms: 0,
                gesture_result: false,
                read_eligible: true,
                audio: &RecordingAudio::default(),
            },
        )
    }
    fn shared_wait_auto(
        &mut self,
        vm: &mut Machine,
        input: &mut ActionInput,
        flags: MessageFlags,
        context: AutoAdvanceContext<'_>,
    ) -> Result<Option<Step>, Box<dyn Error + Send + Sync>> {
        if !vm.pending_system().is_some_and(|(_, c)| {
            dispatch::entry(Semantics::Root, c)
                .is_some_and(|e| e.capability == Capability::MessageWait)
        }) {
            return Ok(None);
        }
        match dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                message_wait: Some(MessageWaitHosts {
                    messages: self,
                    input,
                    flags,
                    context,
                }),
                ..Default::default()
            },
        )? {
            Some(Flow::Done) => Ok(Some(Step::Continue)),
            Some(Flow::Wait | Flow::Yield) => Ok(Some(Step::Yield)),
            other => panic!("unexpected message wait {other:?}"),
        }
    }
}
