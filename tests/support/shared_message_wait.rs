use cmvs_rs::{
    audio::RecordingAudio,
    dispatch::{self, Capability, Flow, Hosts, MessageWaitHosts},
    input::ActionInput,
    machine::{Machine, Step},
    message::{AutoAdvanceContext, MessageHost},
    message_flags::MessageFlags,
    profile::Semantics,
};
use std::error::Error;
#[allow(dead_code)]
pub trait SharedMessageWait {
    fn shared_wait(
        &mut self,
        vm: &mut Machine<'_>,
        input: &mut ActionInput,
        flags: MessageFlags,
    ) -> Result<Option<Step>, Box<dyn Error>>;
    fn shared_wait_auto(
        &mut self,
        vm: &mut Machine<'_>,
        input: &mut ActionInput,
        flags: MessageFlags,
        context: AutoAdvanceContext<'_>,
    ) -> Result<Option<Step>, Box<dyn Error>>;
}
impl SharedMessageWait for MessageHost {
    fn shared_wait(
        &mut self,
        vm: &mut Machine<'_>,
        input: &mut ActionInput,
        flags: MessageFlags,
    ) -> Result<Option<Step>, Box<dyn Error>> {
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
        vm: &mut Machine<'_>,
        input: &mut ActionInput,
        flags: MessageFlags,
        context: AutoAdvanceContext<'_>,
    ) -> Result<Option<Step>, Box<dyn Error>> {
        if !vm.pending_system().is_some_and(|(_, c)| {
            dispatch::ROOT
                .iter()
                .any(|e| e.command == c && e.capability == Capability::MessageWait)
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
