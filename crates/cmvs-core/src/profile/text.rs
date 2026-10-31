//! Line-oriented profile and game-record files; `#` starts a comment line.
use super::{
    Binary, CommandStatus, EngineProfile, Family, GameRecord, MachineProof, ProfileError,
    RecordFile,
};
use crate::{
    digest::{parse_hex, to_hex},
    exe::{Arch, Case},
};
use std::fmt::Write;

pub(super) fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Non-comment lines split into words, with their 1-based line numbers.
struct Lines<'a> {
    lines: Vec<(usize, Vec<&'a str>)>,
    next: usize,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let lines = text
            .lines()
            .enumerate()
            .map(|(i, line)| (i + 1, line.split_whitespace().collect::<Vec<_>>()))
            .filter(|(_, words)| words.first().is_some_and(|w| !w.starts_with('#')))
            .collect();
        Self { lines, next: 0 }
    }
    fn next(&mut self) -> Option<(usize, Vec<&'a str>)> {
        let line = self.lines.get(self.next).cloned();
        self.next += 1;
        line
    }
    fn peek_key(&self) -> Option<&'a str> {
        self.lines.get(self.next).map(|(_, words)| words[0])
    }
    /// The next line, which must start with `key` and hold exactly `count` values.
    fn expect(&mut self, key: &str, count: usize) -> Result<(usize, Vec<&'a str>), ProfileError> {
        match self.next() {
            Some((line, words)) if words[0] == key && words.len() == count + 1 => {
                Ok((line, words[1..].to_vec()))
            }
            Some((line, _)) => Err(at(line, format!("expected `{key}` with {count} values"))),
            None => Err(ProfileError::new(format!("missing `{key}`"))),
        }
    }
}

fn at(line: usize, message: impl Into<String>) -> ProfileError {
    ProfileError {
        line: Some(line),
        message: message.into(),
    }
}

fn hex_number(line: usize, text: &str) -> Result<u64, ProfileError> {
    text.strip_prefix("0x")
        .filter(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|h| u64::from_str_radix(h, 16).ok())
        .ok_or_else(|| {
            at(
                line,
                format!("expected 0x-prefixed hexadecimal, found `{text}`"),
            )
        })
}
fn hex32(line: usize, text: &str) -> Result<u32, ProfileError> {
    u32::try_from(hex_number(line, text)?).map_err(|_| at(line, "value exceeds 32 bits"))
}
fn sha(line: usize, text: &str) -> Result<[u8; 32], ProfileError> {
    parse_hex(text).ok_or_else(|| at(line, "expected 64 lowercase hexadecimal digits"))
}
fn sha_field(line: usize, text: &str) -> Result<[u8; 32], ProfileError> {
    sha(
        line,
        text.strip_prefix("sha256=")
            .ok_or_else(|| at(line, "expected sha256=<digest>"))?,
    )
}
fn identifier(line: usize, text: &str) -> Result<String, ProfileError> {
    is_identifier(text)
        .then(|| text.to_owned())
        .ok_or_else(|| at(line, "expected a lowercase ASCII identifier"))
}

/// `key=value` fields in a fixed order, so the files stay diffable.
fn fields<'a>(line: usize, words: &[&'a str], keys: &[&str]) -> Result<Vec<&'a str>, ProfileError> {
    if words.len() != keys.len() {
        return Err(at(line, format!("expected fields {}", keys.join(" "))));
    }
    words
        .iter()
        .zip(keys)
        .map(|(word, key)| {
            word.strip_prefix(key)
                .and_then(|rest| rest.strip_prefix('='))
                .ok_or_else(|| at(line, format!("expected `{key}=`")))
        })
        .collect()
}

const BINARY_KEYS: [&str; 8] = [
    "file",
    "sha256",
    "timestamp",
    "linker",
    "image-base",
    "table",
    "default",
    "cases",
];

/// A review record: a relative `.md` path under `docs/`, plain ASCII, without `..`.
fn is_review_path(path: &str) -> bool {
    path.ends_with(".md")
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b))
}

pub(super) fn parse_profile(text: &str) -> Result<EngineProfile, ProfileError> {
    let mut lines = Lines::new(text);
    let (line, words) = lines.expect("profile", 1)?;
    let id = identifier(line, words[0])?;
    let (line, words) = lines.expect("family", 1)?;
    let family = match words[0] {
        "cmvs" => Family::Cmvs,
        _ => return Err(at(line, "unknown family")),
    };
    let (line, words) = lines.expect("base", 1)?;
    let base = match words[0] {
        "-" => None,
        other => Some(identifier(line, other)?),
    };
    let mut binaries = Vec::new();
    let mut declared = Vec::new();
    while lines.peek_key() == Some("binary") {
        let (line, words) = lines.next().expect("peeked");
        if words.len() != 2 + BINARY_KEYS.len() {
            return Err(at(line, "malformed binary line"));
        }
        let arch = Arch::parse(words[1]).ok_or_else(|| at(line, "unknown architecture"))?;
        let v = fields(line, &words[2..], &BINARY_KEYS)?;
        let linker = v[3]
            .split_once('.')
            .and_then(|(major, minor)| Some((major.parse().ok()?, minor.parse().ok()?)))
            .ok_or_else(|| at(line, "linker must be <major>.<minor>"))?;
        let cases: usize = v[7]
            .parse()
            .ok()
            .filter(|&n| (1..=0x800).contains(&n))
            .ok_or_else(|| at(line, "cases must be 1..=2048"))?;
        if v[0].is_empty() || !v[0].is_ascii() {
            return Err(at(line, "file name must be ASCII"));
        }
        declared.push(cases);
        binaries.push(Binary {
            arch,
            file: v[0].to_owned(),
            sha256: sha(line, v[1])?,
            timestamp: hex32(line, v[2])?,
            linker,
            image_base: hex_number(line, v[4])?,
            table_rva: hex32(line, v[5])?,
            default_rva: hex32(line, v[6])?,
            cases: Vec::with_capacity(cases),
        });
    }
    // One `compare <arch> base-sha256=` line per proof architecture: every machine proof
    // of that architecture binds this root exe and the profile's own exe of the same arch.
    let mut proof_bases: Vec<(Arch, [u8; 32], usize)> = Vec::new();
    while lines.peek_key() == Some("compare") {
        let (line, words) = lines.next().expect("peeked");
        if words.len() != 3 {
            return Err(at(line, "malformed compare line"));
        }
        let arch = Arch::parse(words[1]).ok_or_else(|| at(line, "unknown proof architecture"))?;
        if !binaries.iter().any(|b| b.arch == arch) {
            return Err(at(line, "compare architecture has no binary"));
        }
        if proof_bases.iter().any(|&(a, ..)| a == arch) {
            return Err(at(line, "duplicate compare architecture"));
        }
        let v = fields(line, &words[2..], &["base-sha256"])?;
        proof_bases.push((arch, sha(line, v[0])?, line));
    }
    let mut statuses = Vec::new();
    let mut proofs = std::collections::BTreeMap::new();
    let mut reviews = std::collections::BTreeMap::new();
    while let Some((line, words)) = lines.next() {
        let proof_key = match words.get(1) {
            Some(&"inherited") => Some("inherit="),
            Some(&"presumed") => Some("presume="),
            _ => None,
        };
        let reviewed = words.get(1) == Some(&"reviewed");
        if words.len() != 2 + binaries.len() + usize::from(proof_key.is_some() || reviewed) {
            return Err(at(
                line,
                "command row needs status and one address per binary",
            ));
        }
        if hex_number(line, words[0])? != 0x2000 + statuses.len() as u64 {
            return Err(at(line, "command rows must be consecutive from 0x2000"));
        }
        let status = match words[1] {
            "absent" => CommandStatus::Absent,
            "noop" => CommandStatus::NoOp,
            "base" => CommandStatus::Base,
            "inherited" => CommandStatus::Inherited,
            "reviewed" => CommandStatus::Reviewed,
            "presumed" => CommandStatus::Presumed,
            "unreviewed" => CommandStatus::Unreviewed,
            _ => return Err(at(line, "unknown command status")),
        };
        if let Some(key) = proof_key {
            let (arch, digest) = words
                .last()
                .expect("row length checked")
                .strip_prefix(key)
                .and_then(|proof| proof.split_once(':'))
                .ok_or_else(|| at(line, format!("expected {key}<arch>:<digest>")))?;
            let arch = Arch::parse(arch).ok_or_else(|| at(line, "unknown proof architecture"))?;
            let &(_, base_sha256, _) = proof_bases
                .iter()
                .find(|&&(a, ..)| a == arch)
                .ok_or_else(|| at(line, "proof architecture lacks a compare line"))?;
            let derived = binaries.iter().find(|b| b.arch == arch);
            proofs.insert(
                statuses.len() as u16,
                MachineProof {
                    arch,
                    base_sha256,
                    derived_sha256: derived.expect("compare line checked").sha256,
                    digest: sha(line, digest)?,
                },
            );
        }
        if reviewed {
            let path = words
                .last()
                .expect("row length checked")
                .strip_prefix("review=")
                .filter(|path| is_review_path(path))
                .ok_or_else(|| at(line, "expected review=<docs-relative .md path>"))?;
            reviews.insert(statuses.len() as u16, path.to_owned());
        }
        statuses.push(status);
        for (binary, word) in binaries.iter_mut().zip(&words[2..]) {
            binary.cases.push(match (*word, status) {
                ("-", CommandStatus::Absent) => Case::Default,
                ("-", _) => return Err(at(line, "only absent commands lack an address")),
                (_, CommandStatus::Absent) => {
                    return Err(at(line, "absent commands have no address"));
                }
                (rva, CommandStatus::NoOp) => Case::NoOp(hex32(line, rva)?),
                (rva, _) => Case::Handler(hex32(line, rva)?),
            });
        }
    }
    if binaries.is_empty()
        || binaries
            .iter()
            .zip(&declared)
            .any(|(b, &n)| b.cases.len() != n)
    {
        return Err(ProfileError::new(
            "command rows do not match the declared cases",
        ));
    }
    let unused = |&&(arch, ..): &&(Arch, [u8; 32], usize)| !proofs.values().any(|p| p.arch == arch);
    if let Some(&(.., line)) = proof_bases.iter().find(unused) {
        return Err(at(
            line,
            "compare line without inherited or presumed commands",
        ));
    }
    let profile = EngineProfile {
        id,
        family,
        base,
        binaries,
        statuses,
        proofs,
        reviews,
    };
    profile.validate()?;
    Ok(profile)
}

impl EngineProfile {
    /// The canonical file form; `parse` of this text yields an equal profile.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "profile {}", self.id);
        let _ = writeln!(
            out,
            "family {}",
            match self.family {
                Family::Cmvs => "cmvs",
            }
        );
        let _ = writeln!(out, "base {}", self.base.as_deref().unwrap_or("-"));
        for b in &self.binaries {
            let _ = writeln!(
                out,
                "binary {} file={} sha256={} timestamp=0x{:08x} linker={}.{} image-base=0x{:x} table=0x{:x} default=0x{:x} cases={}",
                b.arch.name(),
                b.file,
                to_hex(&b.sha256),
                b.timestamp,
                b.linker.0,
                b.linker.1,
                b.image_base,
                b.table_rva,
                b.default_rva,
                b.cases.len()
            );
        }
        for b in &self.binaries {
            if let Some(proof) = self.proofs.values().find(|p| p.arch == b.arch) {
                let _ = writeln!(
                    out,
                    "compare {} base-sha256={}",
                    b.arch.name(),
                    to_hex(&proof.base_sha256)
                );
            }
        }
        let _ = writeln!(out, "# command status {}", {
            let archs: Vec<_> = self.binaries.iter().map(|b| b.arch.name()).collect();
            archs.join(" ")
        });
        for (i, status) in self.statuses.iter().enumerate() {
            let _ = write!(out, "0x{:04x} {}", 0x2000 + i, status.name());
            for b in &self.binaries {
                match b.cases[i] {
                    Case::Handler(rva) | Case::NoOp(rva) => {
                        let _ = write!(out, " 0x{rva:x}");
                    }
                    Case::Default => out.push_str(" -"),
                }
            }
            if let Some(path) = self.review(i as u16) {
                let _ = write!(out, " review={path}");
            }
            if let Some(proof) = self.proof(i as u16) {
                let key = match status {
                    CommandStatus::Presumed => "presume",
                    _ => "inherit",
                };
                let _ = write!(
                    out,
                    " {key}={}:{}",
                    proof.arch.name(),
                    to_hex(&proof.digest)
                );
            }
            out.push('\n');
        }
        out
    }
}

pub(super) fn parse_games(text: &str) -> Result<Vec<GameRecord>, ProfileError> {
    let mut lines = Lines::new(text);
    let mut games = Vec::new();
    while lines.peek_key().is_some() {
        let (line, words) = lines.expect("game", 1)?;
        let game = identifier(line, words[0])?;
        let (line, words) = lines.expect("profile", 1)?;
        let profile = identifier(line, words[0])?;
        let file = |lines: &mut Lines, key| -> Result<RecordFile, ProfileError> {
            let (line, words) = lines.expect(key, 2)?;
            if !words[0].is_ascii() {
                return Err(at(line, "path must be ASCII"));
            }
            Ok(RecordFile {
                path: words[0].to_owned(),
                sha256: sha_field(line, words[1])?,
            })
        };
        let start_script = file(&mut lines, "start")?;
        let script_archive = file(&mut lines, "archive")?;
        let (line, words) = lines.expect("game-id", 1)?;
        let game_id = hex32(line, words[0])?;
        let mut executables = Vec::new();
        while lines.peek_key() == Some("executable") {
            executables.push(file(&mut lines, "executable")?);
        }
        let mut skip_auto_resume = None;
        while lines.peek_key() == Some("policy") {
            let (line, words) = lines.expect("policy", 2)?;
            match (words[0], words[1].strip_prefix("integer=")) {
                ("skip-auto-resume", Some(value)) if skip_auto_resume.is_none() => {
                    skip_auto_resume = Some(
                        value
                            .parse()
                            .map_err(|_| at(line, "integer must be decimal"))?,
                    );
                }
                _ => return Err(at(line, "unknown or repeated policy")),
            }
        }
        lines.expect("end", 0)?;
        games.push(GameRecord {
            game,
            profile,
            start_script,
            script_archive,
            game_id,
            executables,
            skip_auto_resume,
        });
    }
    Ok(games)
}
