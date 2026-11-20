//! Command-line argument handling for the offline tools.
use crate::Result;
use crate::table::Encoding;
use cmvs_core::{
    format::ps3::{Container, Script},
    profile::{EngineProfile, Registry},
};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::Path,
};

pub fn wants_help(args: &[OsString]) -> bool {
    args.iter()
        .take_while(|a| *a != "--")
        .any(|a| a == "--help" || a == "-h")
}

pub struct Arguments<'a> {
    pub paths: Vec<&'a OsString>,
    options: BTreeMap<&'a str, &'a OsStr>,
}

impl<'a> Arguments<'a> {
    pub fn parse(args: &'a [OsString], values: &[&str], flags: &[&str]) -> Result<Self> {
        let mut paths = Vec::new();
        let mut options = BTreeMap::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if arg == "--" {
                paths.extend(args);
                break;
            }
            if !arg.as_encoded_bytes().starts_with(b"-") {
                paths.push(arg);
                continue;
            }
            let key = arg.to_str().ok_or("invalid option")?;
            let value = if values.contains(&key) {
                args.next()
                    .ok_or_else(|| format!("expected value for {key}"))?
                    .as_os_str()
            } else if flags.contains(&key) {
                OsStr::new("")
            } else {
                return Err(format!("unknown option {key}").into());
            };
            if options.insert(key, value).is_some() {
                return Err(format!("duplicate option {key}").into());
            }
        }
        Ok(Self { paths, options })
    }
    pub fn option(&self, name: &str) -> Option<&OsStr> {
        self.options.get(name).copied()
    }
    pub fn encoding(&self, name: &str) -> Result<Encoding> {
        Encoding::parse(
            self.option(name)
                .and_then(OsStr::to_str)
                .ok_or_else(|| format!("expected {name} <encoding>"))?,
        )
    }
    pub fn profile(&self) -> Result<Option<&'static EngineProfile>> {
        self.option("--profile")
            .map(|value| {
                let id = value.to_str().ok_or("invalid profile name")?;
                Registry::builtin()
                    .resolve_profile(id)
                    .ok_or_else(|| format!("unknown profile {id}").into())
            })
            .transpose()
    }
    pub fn read_script(&self, path: &Path) -> Result<Script> {
        let script = Script::decode(&crate::read_bounded(
            path,
            cmvs_core::MAX_PAYLOAD + cmvs_core::format::ps3::HEADER_SIZE,
        )?)?;
        if let Some(profile) = self.profile()?
            && profile.is_cvns() != (script.header().container == Container::Pss0)
        {
            return Err("script container differs from selected profile family".into());
        }
        Ok(script)
    }
}
