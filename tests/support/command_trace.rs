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
