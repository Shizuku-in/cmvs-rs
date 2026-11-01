use cmvs_rs::{
    dispatch::{self, Capability, Flow, Hosts, ScriptHosts, SystemReadHosts, SystemWriteHosts},
    machine::Machine,
    profile::Semantics,
    ps3::Script,
    random::CrtRandom,
    save_menu::{SaveMenuHost, SaveSource},
    script_control::{ControlEvent, ScriptControl, ScriptSource},
    system_save::{Css1Save, SystemSaveEvent, SystemSaveHost, SystemSaveSink, SystemSaveSource},
};
use std::{error::Error, io};
#[path = "support/css1_fixture.rs"]
mod css1_fixture;
#[path = "support/storage_cases.rs"]
mod storage_cases;
#[allow(dead_code)]
mod support;
use storage_cases::cases;
use support::bytecode::{command, pending};
fn bytes(code: &[u8]) -> Vec<u8> {
    let text = b"main.ps3\0common.ps3\0broken\0";
    let mut raw = b"PS2A".to_vec();
    for word in [
        48u32,
        0,
        0,
        0,
        code.len() as u32,
        0,
        text.len() as u32,
        0,
        0,
        (code.len() + text.len()) as u32,
        0,
    ] {
        raw.extend(word.to_le_bytes());
    }
    raw.extend(
        code.iter()
            .chain(text)
            .map(|b| b.rotate_left(1).wrapping_add(124)),
    );
    raw
}
fn script(code: &[u8]) -> Script {
    Script::decode(&bytes(code)).unwrap()
}

#[derive(Default, Debug)]
struct Store {
    reads: usize,
    writes: Vec<Vec<u8>>,
    fail: bool,
}
impl ScriptSource for Store {
    fn load(&mut self, _: &[u8]) -> Result<Script, Box<dyn Error>> {
        self.reads += 1;
        if self.fail {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "source unavailable").into());
        }
        Ok(script(&[0x00, 0x20, 0x14, 0x04, 0, 0]))
    }
}
impl SystemSaveSource for Store {
    fn read(&mut self) -> io::Result<Option<Vec<u8>>> {
        self.reads += 1;
        if self.fail {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "source unavailable",
            ))
        } else {
            Ok(Some(css1_fixture::css1()))
        }
    }
}
impl SystemSaveSink for Store {
    fn write(&mut self, raw: &[u8]) -> io::Result<()> {
        if self.fail {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "source unavailable",
            ));
        }
        self.writes.push(raw.to_vec());
        Ok(())
    }
}
impl SaveSource for Store {
    fn read_header(&mut self, _: u16) -> io::Result<Option<Vec<u8>>> {
        self.reads += 1;
        if self.fail {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "source unavailable",
            ))
        } else {
            Ok(Some(b"CSV2".to_vec()))
        }
    }
}
struct State {
    control: ScriptControl,
    system: SystemSaveHost,
    menu: SaveMenuHost,
    store: Store,
    random: CrtRandom,
}
impl Default for State {
    fn default() -> Self {
        let save = Css1Save::decode(&css1_fixture::css1()).unwrap();
        Self {
            control: ScriptControl::new(17),
            system: SystemSaveHost {
                loaded: Some(save),
                ..Default::default()
            },
            menu: Default::default(),
            store: Default::default(),
            random: CrtRandom::new(0),
        }
    }
}
impl State {
    fn setup(&mut self, vm: &mut Machine<'_>) {
        *vm.state_mut() = self
            .system
            .loaded
            .as_ref()
            .unwrap()
            .applied(vm.state(), Some(0x12345678))
            .unwrap();
        let mut code = vec![];
        command(&mut code, 0x2088, &[1, 0, 3, 4, 5]);
        let script = script(&code);
        let mut registration = Machine::new(&script).unwrap();
        pending(&mut registration);
        self.control
            .try_handle(&mut registration, &mut self.store)
            .unwrap();
    }
    fn hosts(&mut self, cap: Capability) -> Hosts<'_> {
        match cap {
            Capability::ScriptControl => Hosts {
                scripts: Some(ScriptHosts {
                    host: &mut self.control,
                    source: &mut self.store,
                }),
                ..Default::default()
            },
            Capability::SystemRead => Hosts {
                system_read: Some(SystemReadHosts {
                    host: &mut self.system,
                    source: &mut self.store,
                    game_id: Some(0x12345678),
                }),
                ..Default::default()
            },
            Capability::SystemWrite => Hosts {
                system_write: Some(SystemWriteHosts {
                    host: &mut self.system,
                    sink: &mut self.store,
                    random: &mut self.random,
                    game_id: Some(0x12345678),
                }),
                ..Default::default()
            },
            Capability::SaveMenu => Hosts {
                popup: Some(&mut self.menu),
                save_source: Some(&mut self.store),
                ..Default::default()
            },
            _ => unreachable!(),
        }
    }
    fn snapshot(&self) -> String {
        format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
            self.control.callbacks(),
            self.control.settings(),
            self.system.loaded,
            self.system.lists,
            self.menu.menu(),
            self.menu.second_group_enabled(),
            self.random,
            self.store
        )
    }
}
fn expected(op: u16) -> Flow {
    match op {
        0x2080 => Flow::Control(ControlEvent::MainPreparedNeedsSnapshot),
        0x2081 => Flow::Control(ControlEvent::CommonEntered { slot: 1 }),
        0x2083 | 0x2088..=0x208c => Flow::Control(ControlEvent::Applied),
        0x2128 => Flow::SystemSave(SystemSaveEvent::Applied),
        0x2129 => Flow::SystemWritten,
        _ => Flow::Done,
    }
}
#[test]
fn storage_commands_use_one_owner_and_explicit_events() {
    assert_eq!(cases().len(), 13);
    for (op, args, cap) in cases() {
        let owners: Vec<_> = dispatch::ROOT
            .iter()
            .filter(|e| e.command == op - 0x2000)
            .collect();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].capability, cap);
        let mut code = vec![];
        command(&mut code, op, &args);
        command(&mut code, 0x2000, &[]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut state = State::default();
        state.setup(&mut vm);
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let snapshot = state.snapshot();
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut Hosts::default()).unwrap(),
            Some(Flow::Unavailable)
        );
        assert_eq!(
            dispatch::run(
                Semantics::Research(cmvs_rs::profile::ResearchId::FontCells),
                &mut vm,
                &mut state.hosts(cap)
            )
            .unwrap(),
            Some(Flow::Unhandled)
        );
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(state.snapshot(), snapshot);
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap(),
            Some(expected(op))
        );
        assert!(vm.pending_system().is_none());
        if matches!(op, 0x2081 | 0x208a) {
            assert_eq!(vm.stack().len_bytes(), 12);
        } else {
            assert_eq!(vm.stack().len_bytes(), 0);
        }
        if !matches!(op, 0x2080 | 0x2081 | 0x208a) {
            assert_eq!(vm.pc(), before.0 + 2);
        }
        let snapshot = state.snapshot();
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap(),
            None
        );
        assert_eq!(state.snapshot(), snapshot);
    }
}
#[test]
fn storage_short_arguments_preserve_vm_hosts_and_sources() {
    for (op, args, cap) in cases().into_iter().filter(|(_, a, _)| !a.is_empty()) {
        for len in [0, args.len() - 1] {
            let mut code = vec![];
            command(&mut code, op, &args[..len]);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            let mut state = State::default();
            state.setup(&mut vm);
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let vars = format!("{:?}", vm.state());
            let snapshot = state.snapshot();
            let err = dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap_err();
            assert_eq!((err.pc, err.command), (before.0, op - 0x2000));
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(state.snapshot(), snapshot);
            assert_eq!(format!("{:?}", vm.state()), vars);
        }
    }
}
#[test]
fn storage_io_failures_preserve_typed_cause_and_allow_retry() {
    for (op, args, cap) in cases()
        .into_iter()
        .filter(|(op, _, _)| matches!(op, 0x2080 | 0x2081 | 0x2128 | 0x2129 | 0x22b3 | 0x22bb))
    {
        let mut code = vec![];
        command(&mut code, op, &args);
        command(&mut code, 0x2000, &[]);
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        let mut state = State::default();
        state.setup(&mut vm);
        state.store.fail = true;
        pending(&mut vm);
        let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
        let random = state.random.clone();
        let vars = format!("{:?}", vm.state());
        let err = dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap_err();
        assert_eq!(
            err.source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap()
                .kind(),
            io::ErrorKind::Interrupted
        );
        assert_eq!(err.to_string(), "source unavailable");
        assert_eq!(
            (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
            before
        );
        assert_eq!(state.random, random);
        assert_eq!(format!("{:?}", vm.state()), vars);
        state.store.fail = false;
        assert_eq!(
            dispatch::run(Semantics::Root, &mut vm, &mut state.hosts(cap)).unwrap(),
            Some(expected(op))
        );
    }
}

#[test]
fn trace_preserves_storage_option_gates_and_keeps_writes_unavailable() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("cmvs-cli-storage-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("storage.ps3");
    for (op, args, cap) in cases() {
        for record in [false, true] {
            for saves in [false, true] {
                for system in [false, true] {
                    let mut code = vec![];
                    command(&mut code, op, &args);
                    command(&mut code, 0x2003, &[]);
                    let raw = bytes(&code);
                    fs::write(&path, &raw).unwrap();
                    let mut cli = Command::new(env!("CARGO_BIN_EXE_cmvs-trace"));
                    cli.arg(&path).args(["--profile", "cmvs-20160201"]);
                    if record {
                        cli.arg("--record-platform").arg(&root);
                    }
                    if saves {
                        cli.arg("--save-dir").arg(&root);
                    }
                    if system {
                        cli.arg("--system-save").arg(root.join("absent.dat"));
                    }
                    let out = cli.output().unwrap();
                    let stdout = String::from_utf8(out.stdout).unwrap();
                    assert!(
                        out.status.success(),
                        "{stdout}\n{}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                    let enabled = cap == Capability::SaveMenu && saves
                        || cap == Capability::SystemRead && system;
                    let stop = if enabled { 0x2003 } else { op };
                    assert!(
                        stdout.contains(&format!("# STOP pending system=0x{stop:04x};")),
                        "{stdout}"
                    );
                    assert_eq!(fs::read(&path).unwrap(), raw);
                    assert!(!root.join("absent.dat").exists());
                }
            }
        }
    }
    fs::remove_file(path).unwrap();
    fs::remove_dir(root).unwrap();
}
