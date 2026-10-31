//! The optional per-game manifest: corrections to automatic recognition, written only in
//! engine-level terms. It holds data, never logic, and every entry names its evidence.
//!
//! One directive per line; `#` starts a comment; evidence follows `--` and is required:
//!
//! ```text
//! profile <id>                                  -- <evidence>
//! entry <group> <button>                        -- <evidence>
//! choice-group <script> <slot>                  -- <evidence>
//! label page <script> <slot>x<buttons>[+...] <text...>   -- <evidence>
//! label choice <script> <pc-hex> <text...>      -- <evidence>
//! ```
//!
//! The report lists every entry and whether the run used it; an entry that disagrees with
//! automatic recognition is a finding about the recognition, not a reason to add more.
use crate::json::Json;
use std::{
    cell::RefCell,
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    Profile(String),
    Entry(usize, usize),
    ChoiceGroup {
        script: String,
        slot: usize,
    },
    PageLabel {
        script: String,
        shape: String,
        text: String,
    },
    ChoiceLabel {
        script: String,
        pc: usize,
        text: String,
    },
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub line: usize,
    pub directive: Directive,
    pub evidence: String,
}

#[derive(Debug)]
pub struct Manifest {
    pub path: PathBuf,
    pub sha256: String,
    pub entries: Vec<Entry>,
    used: RefCell<BTreeSet<usize>>,
    /// Places where automatic recognition and the manifest disagreed.
    disagreements: RefCell<Vec<String>>,
}

fn number(text: &str, line: usize) -> Result<usize, String> {
    let parsed = match text.strip_prefix("0x") {
        Some(hex) => usize::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.map_err(|_| format!("line {line}: bad number {text}"))
}

/// A page's group shape as written in labels: `0x4+1x49` for slot 0 with 4 buttons and
/// slot 1 with 49.
pub fn shape_text(groups: &[(usize, usize)]) -> String {
    groups
        .iter()
        .map(|(slot, buttons)| format!("{slot}x{buttons}"))
        .collect::<Vec<_>>()
        .join("+")
}

impl Manifest {
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let text =
            String::from_utf8(bytes.clone()).map_err(|_| "manifest must be UTF-8".to_owned())?;
        let mut manifest = Self::parse(&text)?;
        manifest.path = path.to_path_buf();
        manifest.sha256 = cmvs_core::digest::to_hex(&cmvs_core::digest::sha256(&bytes));
        Ok(manifest)
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let content = raw.split('#').next().unwrap_or("").trim();
            if content.is_empty() {
                continue;
            }
            let (body, evidence) = content
                .split_once("--")
                .ok_or_else(|| format!("line {line}: evidence after `--` is required"))?;
            let evidence = evidence.trim().to_owned();
            if evidence.is_empty() {
                return Err(format!("line {line}: evidence after `--` is required"));
            }
            let words: Vec<&str> = body.split_whitespace().collect();
            let directive = match words.as_slice() {
                ["profile", id] => Directive::Profile((*id).to_owned()),
                ["entry", group, button] => {
                    Directive::Entry(number(group, line)?, number(button, line)?)
                }
                ["choice-group", script, slot] => Directive::ChoiceGroup {
                    script: (*script).to_owned(),
                    slot: number(slot, line)?,
                },
                ["label", "page", script, shape, text @ ..] if !text.is_empty() => {
                    Directive::PageLabel {
                        script: (*script).to_owned(),
                        shape: (*shape).to_owned(),
                        text: text.join(" "),
                    }
                }
                ["label", "choice", script, pc, text @ ..] if !text.is_empty() => {
                    Directive::ChoiceLabel {
                        script: (*script).to_owned(),
                        pc: number(pc, line)?,
                        text: text.join(" "),
                    }
                }
                _ => return Err(format!("line {line}: unknown or incomplete directive")),
            };
            entries.push(Entry {
                line,
                directive,
                evidence,
            });
        }
        Ok(Self {
            path: PathBuf::new(),
            sha256: String::new(),
            entries,
            used: RefCell::default(),
            disagreements: RefCell::default(),
        })
    }
    fn find(&self, matches: impl Fn(&Directive) -> bool) -> Option<&Entry> {
        let entry = self.entries.iter().find(|e| matches(&e.directive))?;
        self.used.borrow_mut().insert(entry.line);
        Some(entry)
    }
    /// The profile the manifest was written for; it must match the selected one.
    pub fn profile(&self) -> Option<&str> {
        self.entries.iter().find_map(|e| match &e.directive {
            Directive::Profile(id) => Some(id.as_str()),
            _ => None,
        })
    }
    pub fn mark_profile_used(&self) {
        let _ = self.find(|d| matches!(d, Directive::Profile(_)));
    }
    pub fn entry(&self) -> Option<(usize, usize)> {
        self.find(|d| matches!(d, Directive::Entry(..)))
            .map(|e| match e.directive {
                Directive::Entry(g, b) => (g, b),
                _ => unreachable!(),
            })
    }
    pub fn choice_group(&self, script: &str) -> Option<usize> {
        self.find(|d| matches!(d, Directive::ChoiceGroup { script: s, .. } if s == script))
            .map(|e| match &e.directive {
                Directive::ChoiceGroup { slot, .. } => *slot,
                _ => unreachable!(),
            })
    }
    pub fn page_label(&self, script: &str, shape: &str) -> Option<String> {
        self.find(|d| {
            matches!(d, Directive::PageLabel { script: s, shape: p, .. } if s == script && p == shape)
        })
        .map(|e| match &e.directive {
            Directive::PageLabel { text, .. } => text.clone(),
            _ => unreachable!(),
        })
    }
    pub fn choice_label(&self, script: &str, pc: usize) -> Option<String> {
        self.find(|d| matches!(d, Directive::ChoiceLabel { script: s, pc: p, .. } if s == script && *p == pc))
            .map(|e| match &e.directive {
                Directive::ChoiceLabel { text, .. } => text.clone(),
                _ => unreachable!(),
            })
    }
    pub fn disagree(&self, note: String) {
        let mut notes = self.disagreements.borrow_mut();
        if !notes.contains(&note) {
            notes.push(note);
        }
    }
    pub fn json(&self) -> Json {
        let used = self.used.borrow();
        Json::obj([
            ("path", Json::from(self.path.display().to_string())),
            ("sha256", self.sha256.as_str().into()),
            (
                "entries",
                Json::arr(self.entries.iter().map(|e| {
                    Json::obj([
                        ("line", Json::from(e.line)),
                        ("directive", format!("{:?}", e.directive).into()),
                        ("evidence", e.evidence.as_str().into()),
                        ("used", used.contains(&e.line).into()),
                    ])
                })),
            ),
            (
                "disagreements",
                Json::arr(
                    self.disagreements
                        .borrow()
                        .iter()
                        .map(|n| Json::from(n.as_str())),
                ),
            ),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_directives_and_requires_evidence() {
        let manifest = Manifest::parse(
            "# comment\nprofile cmvs-x -- record\nentry 0 1 -- survey\n\
             choice-group script-a 2 -- two groups at the first choice\n\
             label page script-m 0x4+1x49 config page -- seen\n\
             label choice script-a 0x10 go left -- seen\n",
        )
        .unwrap();
        assert_eq!(manifest.profile(), Some("cmvs-x"));
        assert_eq!(manifest.entry(), Some((0, 1)));
        assert_eq!(manifest.choice_group("script-a"), Some(2));
        assert_eq!(manifest.choice_group("script-b"), None);
        assert_eq!(
            manifest.page_label("script-m", "0x4+1x49").as_deref(),
            Some("config page")
        );
        assert_eq!(
            manifest.choice_label("script-a", 0x10).as_deref(),
            Some("go left")
        );
        assert_eq!(shape_text(&[(0, 4), (1, 49)]), "0x4+1x49");
        assert!(Manifest::parse("entry 0 1\n").is_err());
        assert!(Manifest::parse("entry 0 1 --\n").is_err());
        assert!(Manifest::parse("jump 3 -- x\n").is_err());
        let json = manifest.json().to_string();
        assert!(json.contains("\"used\":true"));
    }
}
