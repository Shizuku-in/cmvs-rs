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

/// Static script structure shared by S2 and S3.
#[derive(Debug, Default)]
pub struct ScriptInventory {
    pub scripts: usize,
    pub instructions: usize,
    pub failures: Vec<(String, String)>,
    /// Full system opcode (0x2000-based) to (static sites, scripts containing it).
    pub commands: BTreeMap<u16, (usize, BTreeSet<String>)>,
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
    inventory.add(
        files.start_script.to_string_lossy().replace('\\', "/"),
        start,
    );
    let archive = File::open(target.root.join(&files.script_archive)).and_then(|mut file| {
        let index = Cpz6Index::read(&mut file).map_err(std::io::Error::other)?;
        Ok((file, index))
    });
    match archive {
        Ok((mut file, index)) => {
            for (i, entry) in index.entries().iter().enumerate() {
                let lower = entry.name.to_ascii_lowercase();
                if !(lower.ends_with(b".ps3") || lower.ends_with(b".ps2")) {
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
            "S2 inventory unavailable"
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
