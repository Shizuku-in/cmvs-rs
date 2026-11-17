use std::{
    ffi::OsStr,
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub fn unique_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// RAII guard for an eagerly created temporary directory.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let seq = unique_id();
        let path =
            std::env::temp_dir().join(format!("cmvs-cli-{tag}-{}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<OsStr> for TempDir {
    fn as_ref(&self) -> &OsStr {
        self.0.as_os_str()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub mod bytecode {
    #[cfg(feature = "desktop")]
    use cmvs_rs::{format::ps3::Script, vm::machine::Machine, vm::machine::Step};

    #[cfg(feature = "desktop")]
    pub fn script(code: &[u8]) -> Script {
        let mut payload = 0u32.to_le_bytes().to_vec();
        payload.extend(code);
        let mut raw = b"PS2A".to_vec();
        for n in [
            48,
            0,
            0,
            1,
            code.len() as u32,
            0,
            0,
            0,
            0,
            payload.len() as u32,
            0,
        ] {
            raw.extend(n.to_le_bytes());
        }
        raw.extend(payload.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        Script::decode(&raw).unwrap()
    }

    pub fn command(code: &mut Vec<u8>, opcode: u16, args: &[u32]) {
        for &arg in args.iter().rev() {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(arg.to_le_bytes());
        }
        code.extend(opcode.to_le_bytes());
    }

    #[cfg(feature = "desktop")]
    pub fn pending(vm: &mut Machine) {
        while !matches!(vm.step().unwrap(), Step::System { .. }) {}
    }
}

pub mod command_trace {
    //! Opt-in metadata trace for external UI replays; no game text or resource bytes.
    use std::{
        fs::File,
        io::Write,
        sync::{Mutex, OnceLock},
    };

    pub fn newly_implemented(op: u16) -> bool {
        matches!(
            op,
            0x203e | 0x2078 | 0x2154 | 0x2157 | 0x2158 | 0x2260
                ..=0x2264 | 0x2287 | 0x2289 | 0x228c | 0x228e | 0x22c9 | 0x2322 | 0x2329
        )
    }

    fn output() -> &'static Option<Mutex<File>> {
        static OUTPUT: OnceLock<Option<Mutex<File>>> = OnceLock::new();
        OUTPUT.get_or_init(|| {
            std::env::var_os("CMVS_RUNTIME_COMMAND_TRACE").map(|path| {
                let mut file = File::create_new(path).expect("fresh command trace path");
                writeln!(
                    file,
                    "# newly implemented commands; successful service, not branch coverage"
                )
                .unwrap();
                Mutex::new(file)
            })
        })
    }

    pub fn begin() {
        let _ = output();
    }

    pub fn record(script: &[u8], pc: usize, op: u16, complete: bool) {
        if let Some(file) = output() {
            writeln!(
                file.lock().unwrap(),
                "command\t{}\t{pc:x}\t0x{op:04x}\tcomplete={complete}",
                script.escape_ascii()
            )
            .unwrap();
        }
    }
}
