//! Check results, verdict levels and the on-disk report.
use crate::json::Json;
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

/// What a passing check proves. Ordered: a higher level includes the lower ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// No error, no gate stop, no stall.
    L0,
    /// A round trip or invariant held within this run.
    L1,
    /// Matched reference data captured from the original executable.
    L2,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Self::L0 => "L0",
            Self::L1 => "L1",
            Self::L2 => "L2",
        }
    }
}

/// A command gate stop; the probe reports it instead of relaxing the policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    /// Full system opcode, 0x2000-based.
    pub command: u16,
    pub script: String,
    pub pc: usize,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass(Level),
    Fail(String),
    Skipped(String),
    Blocked(Blocked),
    Inconclusive(String),
}

impl Outcome {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Pass(_) => "pass",
            Self::Fail(_) => "fail",
            Self::Skipped(_) => "skipped",
            Self::Blocked(_) => "blocked",
            Self::Inconclusive(_) => "inconclusive",
        }
    }
    fn json(&self) -> Vec<(String, Json)> {
        let mut fields = vec![("result".to_owned(), Json::from(self.name()))];
        match self {
            Self::Pass(level) => fields.push(("level".into(), level.name().into())),
            Self::Fail(why) | Self::Skipped(why) | Self::Inconclusive(why) => {
                fields.push(("reason".into(), why.as_str().into()));
            }
            Self::Blocked(b) => fields.push((
                "blocked".into(),
                Json::obj([
                    ("command", Json::hex(b.command.into())),
                    ("script", b.script.as_str().into()),
                    ("pc", Json::hex(b.pc as u64)),
                    ("kind", b.kind.as_str().into()),
                ]),
            )),
        }
        fields
    }
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Stable across games: the column of the cross-game matrix.
    pub id: &'static str,
    pub title: &'static str,
    pub outcome: Outcome,
    pub details: Json,
}

impl CheckResult {
    pub fn new(id: &'static str, title: &'static str, outcome: Outcome) -> Self {
        Self {
            id,
            title,
            outcome,
            details: Json::Null,
        }
    }
    pub fn with(mut self, details: Json) -> Self {
        self.details = details;
        self
    }
    pub fn json(&self) -> Json {
        let mut fields = vec![
            ("id".to_owned(), Json::from(self.id)),
            ("title".to_owned(), Json::from(self.title)),
        ];
        fields.extend(self.outcome.json());
        if self.details != Json::Null {
            fields.push(("details".into(), self.details.clone()));
        }
        Json::Obj(fields)
    }
}

/// Writes `summary.json` and `events.jsonl` into the session output directory; existing
/// files are never overwritten, like the other tools.
pub fn write(
    directory: &Path,
    header: Json,
    results: &[CheckResult],
    events: &[Json],
) -> io::Result<()> {
    let mut summary = header;
    summary.set("checks", Json::arr(results.iter().map(CheckResult::json)));
    let mut file = File::create_new(directory.join("summary.json"))?;
    writeln!(file, "{summary}")?;
    let mut log = BufWriter::new(File::create_new(directory.join("events.jsonl"))?);
    for event in events {
        writeln!(log, "{event}")?;
    }
    log.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_fields_name_the_achieved_level_or_reason() {
        let pass = CheckResult::new("D1", "t", Outcome::Pass(Level::L1));
        assert_eq!(
            pass.json().to_string(),
            r#"{"id":"D1","title":"t","result":"pass","level":"L1"}"#
        );
        let blocked = CheckResult::new(
            "R1",
            "t",
            Outcome::Blocked(Blocked {
                command: 0x22d0,
                script: "x".into(),
                pc: 0x10,
                kind: "unreviewed".into(),
            }),
        )
        .with(Json::obj([("steps", 5u8.into())]));
        assert_eq!(
            blocked.json().to_string(),
            r#"{"id":"R1","title":"t","result":"blocked","blocked":{"command":"0x22d0","script":"x","pc":"0x10","kind":"unreviewed"},"details":{"steps":5}}"#
        );
    }
}
