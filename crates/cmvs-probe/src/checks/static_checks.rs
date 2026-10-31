//! Family S: checks that never run the VM.
use crate::{
    json::Json,
    report::{CheckResult, Level, Outcome},
    target::{Target, cap},
};
use cmvs_core::{
    MAX_PAYLOAD,
    bytecode::{self, InstructionKind},
    cpz::Cpz6Index,
    profile::CommandStatus,
    ps3::Script,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
};

/// Static script structure, shared by the decode check, the gate classification and
/// the coverage table.
#[derive(Debug, Default)]
pub struct ScriptInventory {
    pub scripts: usize,
    pub instructions: usize,
    pub failures: Vec<(String, String)>,
    /// Full system opcode (0x2000-based) to (static sites, scripts containing it).
    pub commands: BTreeMap<u16, (usize, BTreeSet<String>)>,
    /// Sorted instruction start addresses by the script name the VM reports.
    pub instruction_starts: BTreeMap<String, Vec<u32>>,
}

impl ScriptInventory {
    fn add(&mut self, name: String, raw: Result<Vec<u8>, String>) {
        self.scripts += 1;
        let checked = raw.and_then(|raw| {
            let script = Script::decode(&raw).map_err(|e| e.to_string())?;
            bytecode::decode_layout(script.code(), script.header.entry_pc, &script.labels())
                .map_err(|e| e.to_string())
        });
        match checked {
            Ok(instructions) => {
                self.instructions += instructions.len();
                self.instruction_starts.insert(
                    name.clone(),
                    instructions.iter().map(|i| i.pc as u32).collect(),
                );
                for instruction in instructions {
                    if let InstructionKind::System { command } = instruction.kind {
                        let entry = self.commands.entry(0x2000 + command).or_default();
                        entry.0 += 1;
                        entry.1.insert(name.clone());
                    }
                }
            }
            Err(e) => self.failures.push((name, e)),
        }
    }
}

pub fn identify(target: &Target) -> CheckResult {
    const TITLE: &str = "engine profile, file hashes and capabilities";
    let outcome = match (&target.game, &target.identify_error) {
        (Some(_), _) => Outcome::Pass(Level::L0),
        (None, Some(e)) => Outcome::Fail(format!("unsupported-profile: {e}")),
        (None, None) => Outcome::Fail("unsupported-profile".into()),
    };
    CheckResult::new("S1", TITLE, outcome).with(target.identity_json())
}

/// Linear decode of the start script and every script in the archive. Structural only:
/// a decodable script says nothing about runtime support of its commands.
pub fn scripts(target: &Target) -> (CheckResult, Option<ScriptInventory>) {
    const TITLE: &str = "all scripts decode with valid labels and jump targets";
    if let Some(missing) = target.missing(&[cap::FILES, cap::CPZ6]) {
        let skipped = Outcome::Skipped(format!("missing capability {missing}"));
        return (CheckResult::new("S2", TITLE, skipped), None);
    }
    let files = target.files.as_ref().expect("files capability");
    let mut inventory = ScriptInventory::default();
    let start = File::open(target.root.join(&files.start_script))
        .and_then(|f| {
            let mut raw = Vec::new();
            f.take(MAX_PAYLOAD as u64 + 49).read_to_end(&mut raw)?;
            Ok(raw)
        })
        .map_err(|e| e.to_string());
    // The external start script is the one that runs; key it by the name the VM reports
    // and skip an archive entry of the same name.
    let start_name = files.start_name();
    inventory.add(start_name.escape_ascii().to_string(), start);
    let archive = File::open(target.root.join(&files.script_archive)).and_then(|mut file| {
        let index = Cpz6Index::read(&mut file).map_err(std::io::Error::other)?;
        Ok((file, index))
    });
    match archive {
        Ok((mut file, index)) => {
            for (i, entry) in index.entries().iter().enumerate() {
                let lower = entry.name.to_ascii_lowercase();
                if !(lower.ends_with(b".ps3") || lower.ends_with(b".ps2"))
                    || entry.name.eq_ignore_ascii_case(&start_name)
                {
                    continue;
                }
                let raw = index
                    .read_entry_verified(&mut file, i)
                    .map_err(|e| e.to_string());
                inventory.add(entry.name.escape_ascii().to_string(), raw);
            }
        }
        Err(e) => inventory.failures.push(("<archive>".into(), e.to_string())),
    }
    let outcome = if inventory.failures.is_empty() {
        Outcome::Pass(Level::L0)
    } else {
        Outcome::Fail(format!(
            "{} scripts failed to decode",
            inventory.failures.len()
        ))
    };
    let details = Json::obj([
        ("scripts", Json::from(inventory.scripts)),
        ("instructions", inventory.instructions.into()),
        ("distinct_system_commands", inventory.commands.len().into()),
        (
            "failures",
            Json::arr(inventory.failures.iter().take(32).map(|(name, e)| {
                Json::obj([
                    ("script", Json::from(name.as_str())),
                    ("error", e.as_str().into()),
                ])
            })),
        ),
    ]);
    (
        CheckResult::new("S2", TITLE, outcome).with(details),
        Some(inventory),
    )
}

/// Statuses the default gate stops at.
fn stops(status: CommandStatus) -> bool {
    matches!(
        status,
        CommandStatus::Absent
            | CommandStatus::NoOp
            | CommandStatus::Presumed
            | CommandStatus::Unreviewed
    )
}

/// Classifies statically present system commands by gate status. Presence is a linear
/// decode, not reachability; a stopping command may sit on a dead path.
pub fn gate_inventory(target: &Target, inventory: Option<&ScriptInventory>) -> CheckResult {
    const TITLE: &str = "system commands in scripts classified by gate status";
    let (Some(game), Some(inventory)) = (&target.game, inventory) else {
        let why = if target.game.is_none() {
            "missing capability profile"
        } else {
            "script inventory unavailable"
        };
        return CheckResult::new("S3", TITLE, Outcome::Skipped(why.into()));
    };
    let profile = game.selection.profile;
    let mut by_status: BTreeMap<&'static str, Vec<Json>> = BTreeMap::new();
    let mut stopping = 0;
    for (&command, (sites, scripts)) in &inventory.commands {
        let status = profile.status(command - 0x2000);
        let entry = by_status.entry(status.name()).or_default();
        if stops(status) {
            stopping += 1;
            entry.push(Json::obj([
                ("command", Json::hex(command.into())),
                ("sites", (*sites).into()),
                ("scripts", scripts.len().into()),
                (
                    "examples",
                    Json::arr(scripts.iter().take(3).map(|s| s.as_str().into())),
                ),
            ]));
        } else {
            entry.push(Json::hex(command.into()));
        }
    }
    let counts = Json::Obj(
        by_status
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Json::from(v.len())))
            .collect(),
    );
    let stopping_detail = Json::Obj(
        by_status
            .into_iter()
            .filter(|(_, v)| v.first().is_some_and(|j| matches!(j, Json::Obj(_))))
            .map(|(k, v)| (k.to_owned(), Json::Arr(v)))
            .collect(),
    );
    let outcome = if stopping == 0 {
        Outcome::Pass(Level::L0)
    } else {
        Outcome::Fail(format!(
            "{stopping} commands present in scripts stop at the default gate"
        ))
    };
    CheckResult::new("S3", TITLE, outcome).with(Json::obj([
        ("profile", Json::from(profile.id.as_str())),
        ("counts", counts),
        ("stopping", stopping_detail),
    ]))
}

/// Resource files under `data/`, at most three directories deep, sorted for stable output.
fn data_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 3 {
                walk(&path, depth + 1, out);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&root.join("data"), 0, &mut files);
    files.sort();
    files
}

fn extension(name: &[u8]) -> String {
    name.rsplit(|&b| b == b'.')
        .next()
        .filter(|e| e.len() < name.len())
        .map(|e| e.to_ascii_lowercase().escape_ascii().to_string())
        .unwrap_or_default()
}

/// Checks the header a resource of this kind must have; other kinds pass on their checksum.
fn resource_header(kind: &str, bytes: &[u8]) -> Result<(), String> {
    match kind {
        "pb3" => cmvs_core::pb3::Header::parse(bytes)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "ogg" if !bytes.starts_with(b"OggS") => Err("missing OggS signature".into()),
        // Scene movies are CMV6 or CMV8; CMV9 has its own container.
        "cmv" => cmvs_core::cmv::CmvIndex::read_scene(&mut std::io::Cursor::new(bytes))
            .map(|_| ())
            .or_else(|first| {
                cmvs_core::cmv9::Cmv9Index::parse(bytes)
                    .map(|_| ())
                    .map_err(|second| format!("{first}; as CMV9: {second}"))
            }),
        _ => Ok(()),
    }
}

/// S4: every archive entry under `data/` passes its checksum and the header of its kind;
/// loose audio and movie files have valid headers.
pub fn resources(target: &Target) -> CheckResult {
    const TITLE: &str = "resource archives verify and resource headers decode";
    if let Some(missing) = target.missing(&[cap::FILES, cap::CPZ6]) {
        return CheckResult::new(
            "S4",
            TITLE,
            Outcome::Skipped(format!("missing capability {missing}")),
        );
    }
    let files = target.files.as_ref().expect("files capability");
    let script_archive = target.root.join(&files.script_archive);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures: Vec<Json> = Vec::new();
    let mut failed = 0usize;
    let mut fail = |what: String, error: String| {
        failed += 1;
        if failures.len() < 32 {
            failures.push(Json::obj([
                ("resource", Json::from(what)),
                ("error", error.into()),
            ]));
        }
    };
    let mut archives = 0;
    for path in data_files(&target.root) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kind = extension(name.as_bytes());
        if kind == "cpz" {
            if path == script_archive {
                continue;
            }
            archives += 1;
            let opened = File::open(&path).and_then(|mut file| {
                let index = Cpz6Index::read(&mut file).map_err(std::io::Error::other)?;
                Ok((file, index))
            });
            let (mut file, index) = match opened {
                Ok(opened) => opened,
                Err(e) => {
                    fail(name, e.to_string());
                    continue;
                }
            };
            for (i, entry) in index.entries().iter().enumerate() {
                let entry_kind = extension(&entry.name);
                *kinds.entry(entry_kind.clone()).or_default() += 1;
                let label = format!("{name}/{}", entry.name.escape_ascii());
                match index.read_entry_verified(&mut file, i) {
                    Ok(bytes) => {
                        if let Err(e) = resource_header(&entry_kind, &bytes) {
                            fail(label, e);
                        }
                    }
                    Err(e) => fail(label, e.to_string()),
                }
            }
        } else if matches!(kind.as_str(), "ogg" | "cmv") {
            *kinds.entry(kind.clone()).or_default() += 1;
            // Loose movies can be large: parse the index from the file itself.
            let checked = if kind == "cmv" {
                File::open(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|mut f| {
                        cmvs_core::cmv::CmvIndex::read_scene(&mut f)
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    })
            } else {
                let mut head = Vec::new();
                File::open(&path)
                    .and_then(|f| f.take(4).read_to_end(&mut head))
                    .map_err(|e| e.to_string())
                    .and_then(|_| resource_header(&kind, &head))
            };
            if let Err(e) = checked {
                fail(name, e);
            }
        }
    }
    let total: usize = kinds.values().sum();
    let outcome = if failed > 0 {
        Outcome::Fail(format!("{failed} of {total} resources failed"))
    } else if total == 0 {
        Outcome::Inconclusive("no resource archive or file was found under data/".into())
    } else {
        Outcome::Pass(Level::L0)
    };
    CheckResult::new("S4", TITLE, outcome).with(Json::obj([
        ("archives", Json::from(archives)),
        ("resources", total.into()),
        (
            "kinds",
            Json::Obj(kinds.into_iter().map(|(k, v)| (k, Json::from(v))).collect()),
        ),
        ("failures", Json::Arr(failures)),
    ]))
}

/// S5: saves shipped in the game's own `save/` directory decode with the save formats.
pub fn bundled_saves(target: &Target) -> CheckResult {
    const TITLE: &str = "saves shipped with the game decode";
    let dir = target.root.join("save");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return CheckResult::new(
            "S5",
            TITLE,
            Outcome::Skipped("the game has no save directory".into()),
        );
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();
    let mut rows = Vec::new();
    let mut failed = 0;
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let lower = name.to_ascii_lowercase();
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let result = if lower == "system.dat" {
            cmvs_core::system_save::Css1Save::decode(&bytes)
                .map(|_| "CSS1 system save".to_owned())
                .map_err(|e| e.to_string())
        } else if lower.starts_with("save") && lower.ends_with(".dat") {
            cmvs_core::save::Csv2Save::decode(&bytes)
                .map_err(|e| e.to_string())
                .and_then(|save| {
                    cmvs_core::snapshot::EngineSnapshot::decode(&save.snapshot)
                        .map(|_| "CSV2 gameplay save".to_owned())
                        .map_err(|e| e.to_string())
                })
        } else {
            continue;
        };
        failed += usize::from(result.is_err());
        rows.push(Json::obj([
            ("file", Json::from(name)),
            (
                "result",
                result.unwrap_or_else(|e| format!("error: {e}")).into(),
            ),
        ]));
    }
    let outcome = if rows.is_empty() {
        Outcome::Skipped("the save directory holds no system or gameplay save".into())
    } else if failed == 0 {
        Outcome::Pass(Level::L0)
    } else {
        Outcome::Fail(format!("{failed} saves failed to decode"))
    };
    CheckResult::new("S5", TITLE, outcome).with(Json::obj([("saves", Json::Arr(rows))]))
}
