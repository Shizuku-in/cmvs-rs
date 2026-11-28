//! Line-oriented profile and game-record files; `#` starts a comment line.
use super::{
    CommandStatus, Family, GameRecord, MachineProof, ProfileError, RecordFile, ResearchBinary,
    ResearchProfile, VariantBinding, VariantId, model::lookup_sorted,
};
use crate::exe::{Arch, Case};
use cmvs_core::format::digest::{parse_hex, to_hex};
use std::fmt::Write;

pub(super) fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Non-comment lines split into words, with their 1-based line numbers.
pub(super) struct Lines<'a> {
    lines: std::iter::Enumerate<std::str::Lines<'a>>,
    peeked_line: usize,
    peeked: Vec<&'a str>,
    current: Vec<&'a str>,
}

impl<'a> Lines<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        let mut this = Self {
            lines: text.lines().enumerate(),
            peeked_line: 0,
            peeked: Vec::new(),
            current: Vec::new(),
        };
        this.fill_peeked();
        this
    }
    fn fill_peeked(&mut self) {
        self.peeked.clear();
        for (i, line) in self.lines.by_ref() {
            self.peeked.extend(line.split_whitespace());
            if self.peeked.first().is_some_and(|w| !w.starts_with('#')) {
                self.peeked_line = i + 1;
                return;
            }
            self.peeked.clear();
        }
    }
    pub(super) fn next(&mut self) -> Option<(usize, &[&'a str])> {
        if self.peeked.is_empty() {
            return None;
        }
        let line = self.peeked_line;
        std::mem::swap(&mut self.current, &mut self.peeked);
        self.fill_peeked();
        Some((line, &self.current))
    }
    fn peek_key(&self) -> Option<&'a str> {
        self.peeked.first().copied()
    }
    /// The next line, which must start with `key` and hold exactly `count` values.
    pub(super) fn expect(
        &mut self,
        key: &str,
        count: usize,
    ) -> Result<(usize, &[&'a str]), ProfileError> {
        match self.next() {
            Some((line, words)) if words[0] == key && words.len() == count + 1 => {
                Ok((line, &words[1..]))
            }
            Some((line, _)) => Err(at(line, format!("expected `{key}` with {count} values"))),
            None => Err(ProfileError::new(format!("missing `{key}`"))),
        }
    }
}

pub(super) fn at(line: usize, message: impl Into<String>) -> ProfileError {
    ProfileError {
        line: Some(line),
        message: message.into(),
    }
}

pub(super) fn hex_number(line: usize, text: &str) -> Result<u64, ProfileError> {
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
pub(super) fn hex32(line: usize, text: &str) -> Result<u32, ProfileError> {
    u32::try_from(hex_number(line, text)?).map_err(|_| at(line, "value exceeds 32 bits"))
}
pub(super) fn sha(line: usize, text: &str) -> Result<[u8; 32], ProfileError> {
    parse_hex(text).ok_or_else(|| at(line, "expected 64 lowercase hexadecimal digits"))
}
fn sha_field(line: usize, text: &str) -> Result<[u8; 32], ProfileError> {
    sha(
        line,
        text.strip_prefix("sha256=")
            .ok_or_else(|| at(line, "expected sha256=<digest>"))?,
    )
}
pub(super) fn identifier(line: usize, text: &str) -> Result<String, ProfileError> {
    is_identifier(text)
        .then(|| text.to_owned())
        .ok_or_else(|| at(line, "expected a lowercase ASCII identifier"))
}

/// `key=value` fields in a fixed order, so the files stay diffable.
pub(super) fn fields<'a, const N: usize>(
    line: usize,
    words: &[&'a str],
    keys: &[&str; N],
) -> Result<[&'a str; N], ProfileError> {
    if words.len() != N {
        return Err(at(line, format!("expected fields {}", keys.join(" "))));
    }
    let mut out = [""; N];
    for ((slot, word), key) in out.iter_mut().zip(words).zip(keys) {
        *slot = word
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
            .ok_or_else(|| at(line, format!("expected `{key}=`")))?;
    }
    Ok(out)
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

pub(crate) fn parse_profile(text: &str) -> Result<ResearchProfile, ProfileError> {
    let mut lines = Lines::new(text);
    let (line, words) = lines.expect("profile", 1)?;
    let id = identifier(line, words[0])?;
    let (line, words) = lines.expect("family", 1)?;
    let family = match words[0] {
        "cmvs" => Family::Cmvs,
        "cvns" => Family::Cvns,
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
        binaries.push(ResearchBinary {
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
    let mut proofs = Vec::new();
    let mut reviews = Vec::new();
    let mut contracts = Vec::new();
    let mut variant = Vec::new();
    let mut maps = Vec::new();
    while let Some((line, words)) = lines.next() {
        let proof_key = match words.get(1) {
            Some(&"inherited") => Some("inherit="),
            Some(&"presumed") => Some("presume="),
            _ => None,
        };
        let reviewed = words.get(1) == Some(&"reviewed");
        let contracted = words.get(1) == Some(&"contract-equivalent");
        let has_variant = words.iter().any(|word| word.starts_with("variant="));
        // `map=` sits between the addresses and `review=`.
        let map = words
            .get(2 + binaries.len())
            .and_then(|word| word.strip_prefix("map="));
        if map.is_some() && (!reviewed || family != Family::Cvns) {
            return Err(at(line, "only reviewed CVNS commands carry a map"));
        }
        if words.len()
            != 2 + binaries.len()
                + usize::from(proof_key.is_some() || reviewed || contracted)
                + usize::from(map.is_some())
                + 2 * usize::from(has_variant)
        {
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
            "contract-equivalent" => CommandStatus::ContractEquivalent,
            "presumed" => CommandStatus::Presumed,
            "unreviewed" => CommandStatus::Unreviewed,
            _ => return Err(at(line, "unknown command status")),
        };
        if has_variant {
            let fields = &words[2 + binaries.len()..];
            if status != CommandStatus::Unreviewed || fields.len() != 2 {
                return Err(at(line, "variant override requires an unreviewed command"));
            }
            let (name, binding) = fields[0]
                .strip_prefix("variant=")
                .and_then(|value| value.split_once(':'))
                .ok_or_else(|| at(line, "expected variant=<implementation>:<arch>:<sha256>"))?;
            let (arch, binary_hash) = binding.split_once(':').ok_or_else(|| {
                at(
                    line,
                    "variant binding needs architecture and binary SHA-256",
                )
            })?;
            let id =
                VariantId::parse(name).ok_or_else(|| at(line, "unknown variant implementation"))?;
            let arch = Arch::parse(arch).ok_or_else(|| at(line, "unknown variant architecture"))?;
            let binary_sha256 = sha(line, binary_hash)?;
            let _binary = binaries
                .iter()
                .find(|binary| binary.arch == arch)
                .ok_or_else(|| at(line, "variant architecture has no binary"))?;
            let evidence = fields[1]
                .strip_prefix("evidence=")
                .filter(|path| is_review_path(path))
                .ok_or_else(|| at(line, "expected evidence=<docs-relative .md path>"))?;
            variant.push((
                statuses.len() as u16,
                VariantBinding {
                    id,
                    arch,
                    binary_sha256,
                    evidence: evidence.to_owned(),
                },
            ));
        }
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
            proofs.push((
                statuses.len() as u16,
                MachineProof {
                    arch,
                    base_sha256,
                    derived_sha256: derived.expect("compare line checked").sha256,
                    digest: sha(line, digest)?,
                },
            ));
        }
        if let Some(target) = map {
            let target = hex_number(line, target)?
                .checked_sub(0x2000)
                .and_then(|t| u16::try_from(t).ok())
                .ok_or_else(|| at(line, "map target must be a 0x2000-based system opcode"))?;
            maps.push((statuses.len() as u16, target));
        }
        if reviewed {
            let path = words
                .last()
                .expect("row length checked")
                .strip_prefix("review=")
                .filter(|path| is_review_path(path))
                .ok_or_else(|| at(line, "expected review=<docs-relative .md path>"))?;
            reviews.push((statuses.len() as u16, path.to_owned()));
        }
        if contracted {
            let id = words
                .last()
                .expect("row length checked")
                .strip_prefix("contract=")
                .ok_or_else(|| at(line, "expected contract=<certificate id>"))?;
            contracts.push((statuses.len() as u16, sha(line, id)?));
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
    let unused =
        |&&(arch, ..): &&(Arch, [u8; 32], usize)| !proofs.iter().any(|(_, p)| p.arch == arch);
    if let Some(&(.., line)) = proof_bases.iter().find(unused) {
        return Err(at(
            line,
            "compare line without inherited or presumed commands",
        ));
    }
    Ok(ResearchProfile {
        id,
        family,
        base,
        binaries,
        statuses,
        proofs,
        contracts,
        reviews,
        variant,
        maps,
    })
}

impl ResearchProfile {
    /// The canonical file form of a source profile.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "profile {}", self.id);
        let _ = writeln!(
            out,
            "family {}",
            match self.family {
                Family::Cmvs => "cmvs",
                Family::Cvns => "cvns",
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
            if let Some(proof) = self
                .proofs
                .iter()
                .find_map(|(_, p)| (p.arch == b.arch).then_some(p))
            {
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
            if let Some(&target) = lookup_sorted(&self.maps, i as u16) {
                let _ = write!(out, " map=0x{:04x}", 0x2000 + u32::from(target));
            }
            if let Some(path) = lookup_sorted(&self.reviews, i as u16) {
                let _ = write!(out, " review={path}");
            }
            if let Some(id) = lookup_sorted(&self.contracts, i as u16) {
                let _ = write!(out, " contract={}", to_hex(id));
            }
            if let Some(proof) = lookup_sorted(&self.proofs, i as u16) {
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
            if let Some(variant) = lookup_sorted(&self.variant, i as u16) {
                let _ = write!(
                    out,
                    " variant={}:{}:{} evidence={}",
                    variant.id.name(),
                    variant.arch.name(),
                    to_hex(&variant.binary_sha256),
                    variant.evidence
                );
            }
            out.push('\n');
        }
        out
    }
}

pub(crate) fn parse_games(text: &str) -> Result<Vec<GameRecord>, ProfileError> {
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
        let mut start_scripts = vec![file(&mut lines, "start")?];
        while lines.peek_key() == Some("start") {
            start_scripts.push(file(&mut lines, "start")?);
        }
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
            start_scripts,
            script_archive,
            game_id,
            executables,
            skip_auto_resume,
        });
    }
    Ok(games)
}
