use cmvs_core::{
    format::system_save::SystemSaveEvent, format::system_save::SystemSaveHost,
    format::system_save::SystemSaveSink, format::system_save::SystemSaveSource,
    host::random::CrtRandom, profile::Semantics, runtime::save_menu::SaveMenuHost,
    runtime::save_menu::SaveSource, runtime::script_control::ControlEvent,
    runtime::script_control::ScriptControl, runtime::script_control::ScriptSource, vm::dispatch,
    vm::dispatch::Capability, vm::dispatch::Flow, vm::dispatch::Hosts, vm::dispatch::ScriptHosts,
    vm::dispatch::SystemReadHosts, vm::dispatch::SystemWriteHosts, vm::machine::Machine,
};
use std::error::Error;
fn accepts(vm: &Machine, cap: Capability) -> bool {
    vm.pending_system().is_some_and(|(_, c)| {
        dispatch::entry(Semantics::Root, c).is_some_and(|e| e.capability == cap)
    })
}
pub trait SharedControl {
    fn shared_control(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn ScriptSource,
    ) -> Result<Option<ControlEvent>, Box<dyn Error + Send + Sync>>;
}
impl SharedControl for ScriptControl {
    fn shared_control(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn ScriptSource,
    ) -> Result<Option<ControlEvent>, Box<dyn Error + Send + Sync>> {
        if !accepts(vm, Capability::ScriptControl) {
            return Ok(None);
        }
        match dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                scripts: Some(ScriptHosts { host: self, source }),
                ..Default::default()
            },
        )? {
            Some(Flow::Control(event)) => Ok(Some(event)),
            other => panic!("unexpected control flow {other:?}"),
        }
    }
}
pub trait SharedSystem {
    fn shared_read(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn SystemSaveSource,
        game_id: Option<u32>,
    ) -> Result<Option<SystemSaveEvent>, Box<dyn Error + Send + Sync>>;
    fn shared_write(
        &mut self,
        vm: &mut Machine,
        sink: &mut dyn SystemSaveSink,
        random: &mut CrtRandom,
        game_id: Option<u32>,
    ) -> Result<bool, Box<dyn Error + Send + Sync>>;
}
impl SharedSystem for SystemSaveHost {
    fn shared_read(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn SystemSaveSource,
        game_id: Option<u32>,
    ) -> Result<Option<SystemSaveEvent>, Box<dyn Error + Send + Sync>> {
        if !accepts(vm, Capability::SystemRead) {
            return Ok(None);
        }
        match dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                system_read: Some(SystemReadHosts {
                    host: self,
                    source,
                    game_id,
                    persistent: Default::default(),
                }),
                ..Default::default()
            },
        )? {
            Some(Flow::SystemSave(event)) => Ok(Some(event)),
            other => panic!("unexpected read flow {other:?}"),
        }
    }
    fn shared_write(
        &mut self,
        vm: &mut Machine,
        sink: &mut dyn SystemSaveSink,
        random: &mut CrtRandom,
        game_id: Option<u32>,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        if !accepts(vm, Capability::SystemWrite) {
            return Ok(false);
        }
        Ok(dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                system_write: Some(SystemWriteHosts {
                    host: self,
                    sink,
                    random,
                    game_id,
                }),
                ..Default::default()
            },
        )? == Some(Flow::SystemWritten))
    }
}
pub trait SharedSaveMenu {
    fn shared_menu(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn SaveSource,
    ) -> Result<bool, Box<dyn Error + Send + Sync>>;
}
impl SharedSaveMenu for SaveMenuHost {
    fn shared_menu(
        &mut self,
        vm: &mut Machine,
        source: &mut dyn SaveSource,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        if !accepts(vm, Capability::SaveMenu) {
            return Ok(false);
        }
        Ok(dispatch::run(
            Semantics::Root,
            vm,
            &mut Hosts {
                popup: Some(self),
                save_source: Some(source),
                ..Default::default()
            },
        )? == Some(Flow::Done))
    }
}
