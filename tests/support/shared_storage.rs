use cmvs_rs::{
    dispatch::{self, Capability, Flow, Hosts, ScriptHosts, SystemReadHosts, SystemWriteHosts},
    machine::Machine,
    profile::Semantics,
    random::CrtRandom,
    save_menu::{SaveMenuHost, SaveSource},
    script_control::{ControlEvent, ScriptControl, ScriptSource},
    system_save::{SystemSaveEvent, SystemSaveHost, SystemSaveSink, SystemSaveSource},
};
use std::error::Error;
fn accepts(vm: &Machine<'_>, cap: Capability) -> bool {
    vm.pending_system().is_some_and(|(_, c)| {
        dispatch::ROOT
            .iter()
            .any(|e| e.command == c && e.capability == cap)
    })
}
#[allow(dead_code)]
pub trait SharedControl {
    fn shared_control(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn ScriptSource,
    ) -> Result<Option<ControlEvent>, Box<dyn Error>>;
}
impl SharedControl for ScriptControl {
    fn shared_control(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn ScriptSource,
    ) -> Result<Option<ControlEvent>, Box<dyn Error>> {
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
#[allow(dead_code)]
pub trait SharedSystem {
    fn shared_read(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn SystemSaveSource,
        game_id: Option<u32>,
    ) -> Result<Option<SystemSaveEvent>, Box<dyn Error>>;
    fn shared_write(
        &mut self,
        vm: &mut Machine<'_>,
        sink: &mut dyn SystemSaveSink,
        random: &mut CrtRandom,
        game_id: Option<u32>,
    ) -> Result<bool, Box<dyn Error>>;
}
impl SharedSystem for SystemSaveHost {
    fn shared_read(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn SystemSaveSource,
        game_id: Option<u32>,
    ) -> Result<Option<SystemSaveEvent>, Box<dyn Error>> {
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
        vm: &mut Machine<'_>,
        sink: &mut dyn SystemSaveSink,
        random: &mut CrtRandom,
        game_id: Option<u32>,
    ) -> Result<bool, Box<dyn Error>> {
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
#[allow(dead_code)]
pub trait SharedSaveMenu {
    fn shared_menu(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn SaveSource,
    ) -> Result<bool, Box<dyn Error>>;
}
impl SharedSaveMenu for SaveMenuHost {
    fn shared_menu(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut dyn SaveSource,
    ) -> Result<bool, Box<dyn Error>> {
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
