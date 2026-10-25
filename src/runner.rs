//! Bounded real-data startup orchestration for the researched Amatsutsumi layout.
//! It never resumes past the original main-transition snapshot barrier.
use crate::{
    MAX_PAYLOAD,
    bootstrap::Bootstrap,
    machine::{Machine, Step},
    platform::{DirectoryAssets, PlatformBackend, PlatformHost},
    ps3::Script,
    save_menu::{DirectorySaves, SaveMenuHost},
    script_control::{ArchiveScripts, ControlEvent, ScriptControl},
    system_save::{FileSystemSave, SystemSaveHost},
};
use std::{error::Error, fs::File, io::Read, path::Path};

pub fn load_start(root: &Path) -> Result<Script, Box<dyn Error>> {
    let mut bytes = Vec::new();
    File::open(root.join("data/pack/start.ps3"))?
        .take(MAX_PAYLOAD as u64 + 49)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PAYLOAD + 48 {
        return Err("startup script exceeds research limit".into());
    }
    Ok(Script::decode(&bytes)?)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupEvent {
    Progress,
    Yield,
    Unhandled { pc: usize, command: u16 },
    MainPreparedNeedsSnapshot,
}
pub struct Startup<'a> {
    machine: Machine<'a>,
    bootstrap: Bootstrap,
    control: ScriptControl,
    source: ArchiveScripts<File>,
    saves: DirectorySaves,
    save_menu: SaveMenuHost,
    system_source: FileSystemSave,
    system_host: SystemSaveHost,
    assets: DirectoryAssets,
    platform: PlatformHost,
    steps: u32,
    main_barrier: bool,
}
impl<'a> Startup<'a> {
    pub fn new(script: &'a Script, root: &Path, virtual_tick: u32) -> Result<Self, Box<dyn Error>> {
        let mut machine = Machine::new(script)?;
        machine.set_current_name(b"start.ps3".to_vec());
        Ok(Self {
            machine,
            bootstrap: Bootstrap::default(),
            control: ScriptControl::new(virtual_tick),
            source: ArchiveScripts::new(File::open(root.join("data/pack/script.cpz"))?)?,
            saves: DirectorySaves::new(root.join("save"))?,
            save_menu: SaveMenuHost::default(),
            system_source: FileSystemSave::new(root.join("save/system.dat")),
            system_host: SystemSaveHost::default(),
            assets: DirectoryAssets::new(root)?,
            platform: PlatformHost::default(),
            steps: 0,
            main_barrier: false,
        })
    }
    pub fn machine(&self) -> &Machine<'a> {
        &self.machine
    }
    pub fn bootstrap(&self) -> &Bootstrap {
        &self.bootstrap
    }
    pub fn control(&self) -> &ScriptControl {
        &self.control
    }
    pub fn platform(&self) -> &PlatformHost {
        &self.platform
    }
    pub fn steps(&self) -> u32 {
        self.steps
    }
    pub fn tick(
        &mut self,
        backend: &mut impl PlatformBackend,
    ) -> Result<StartupEvent, Box<dyn Error>> {
        if self.main_barrier {
            return Ok(StartupEvent::MainPreparedNeedsSnapshot);
        }
        if self.steps >= 1_000_000 {
            return Err("startup research step budget exhausted".into());
        }
        let event = self.machine.step()?;
        self.steps += 1;
        let Step::System { pc, command } = event else {
            return Ok(if event == Step::Yield {
                StartupEvent::Yield
            } else {
                StartupEvent::Progress
            });
        };
        let id = self.bootstrap.settings().get(&0x201d).copied();
        if self
            .system_host
            .try_handle(&mut self.machine, &mut self.system_source, id)?
            .is_some()
        {
            return Ok(StartupEvent::Progress);
        }
        if self
            .platform
            .try_handle(
                &mut self.machine,
                backend,
                &mut self.assets,
                self.bootstrap.cursor_names(),
            )?
            .is_some()
        {
            return Ok(StartupEvent::Progress);
        }
        if let Some(event) = self
            .control
            .try_handle(&mut self.machine, &mut self.source)?
        {
            if event == ControlEvent::MainPreparedNeedsSnapshot {
                self.main_barrier = true;
                return Ok(StartupEvent::MainPreparedNeedsSnapshot);
            }
            return Ok(StartupEvent::Progress);
        }
        if self
            .save_menu
            .try_handle(&mut self.machine, &mut self.saves)?
            || self.bootstrap.try_handle(&mut self.machine)?.is_some()
        {
            return Ok(StartupEvent::Progress);
        }
        Ok(StartupEvent::Unhandled {
            pc,
            command: command + 0x2000,
        })
    }
}
