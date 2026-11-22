use crate::Result;
use cmvs_core::format::digest::{sha256, to_hex};
use cmvs_font::font::FontRenderer;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

const MAX_FONT: usize = 32 << 20;
pub const MAX_REFERENCE: u64 = 4096;

pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_FONT as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FONT {
        return Err("font exceeds 32 MiB research limit".into());
    }
    Ok(bytes)
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn is_reference(name: &str) -> bool {
    name.strip_prefix("CMVS-R-")
        .is_some_and(|id| is_hex(id, 24))
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Source {
    System { postscript: String },
    Imported { hash: String, index: u32 },
}

#[derive(Serialize, Deserialize)]
pub(super) struct Reference {
    pub source: Source,
    pub name: String,
}

impl Reference {
    fn key(&self) -> Result<String> {
        let hash = to_hex(&sha256(&serde_json::to_vec(&self.source)?));
        Ok(format!("CMVS-R-{}", &hash[..24]))
    }

    fn validate(&self) -> Result<()> {
        let valid_name = |name: &str| {
            !name.is_empty() && name.len() <= 512 && !name.chars().any(char::is_control)
        };
        let valid_source = match &self.source {
            Source::System { postscript } => valid_name(postscript),
            Source::Imported { hash, .. } => is_hex(hash, 64),
        };
        if valid_name(&self.name) && valid_source {
            Ok(())
        } else {
            Err("invalid font reference".into())
        }
    }

    pub fn read(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_REFERENCE + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_REFERENCE {
            return Err("font reference is too large".into());
        }
        let reference: Self = serde_json::from_slice(&bytes)?;
        reference.validate()?;
        if path.file_name().and_then(|name| name.to_str()) != Some(reference.key()?.as_str()) {
            return Err("font reference does not match its identifier".into());
        }
        Ok(reference)
    }

    pub fn write(&self, directory: &Path) -> Result<String> {
        self.validate()?;
        let key = self.key()?;
        match fs::create_dir(directory) {
            Err(e)
                if e.kind() == io::ErrorKind::AlreadyExists
                    && fs::symlink_metadata(directory)?.is_dir() => {}
            result => result?,
        }
        let path = directory.join(&key);
        let mut temp = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(temp.as_file_mut(), self)?;
        temp.as_file().sync_all()?;
        match temp.persist_noclobber(&path) {
            Ok(_) => {}
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {
                if !fs::symlink_metadata(&path)?.is_file()
                    || Self::read(&path)?.source != self.source
                {
                    return Err("stored font reference conflict".into());
                }
            }
            Err(e) => return Err(e.into()),
        }
        Ok(key)
    }
}

#[derive(Clone, Deserialize)]
pub struct Choice {
    id: String,
    name: String,
    path: PathBuf,
    index: u32,
    system: Option<String>,
}

pub struct Fonts {
    directory: PathBuf,
    choices: Vec<Choice>,
    hashes: HashMap<PathBuf, Option<String>>,
    labels: HashMap<String, (String, Option<String>)>,
}

fn file_hash<'a>(path: &Path, cache: &'a mut HashMap<PathBuf, Option<String>>) -> Option<&'a str> {
    cache
        .entry(path.to_owned())
        .or_insert_with(|| read_file(path).ok().map(|bytes| to_hex(&sha256(&bytes))))
        .as_deref()
}

impl Fonts {
    pub fn open(directory: &Path, choices: Vec<Choice>, renderer: &FontRenderer) -> Result<Self> {
        let directory = directory.join("font-faces");
        let mut fonts = Self {
            directory,
            choices,
            hashes: HashMap::new(),
            labels: HashMap::new(),
        };
        if fs::symlink_metadata(&fonts.directory).is_ok_and(|metadata| metadata.is_dir())
            && let Ok(entries) = fs::read_dir(&fonts.directory)
        {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if let Some(key) = name.to_str().filter(|name| is_reference(name))
                    && entry.file_type().is_ok_and(|kind| kind.is_file())
                    && let Ok(reference) = Reference::read(&entry.path())
                {
                    let choice = fonts.locate(&reference.source).cloned();
                    fonts.bind(renderer, key, reference, choice)?;
                }
            }
        }
        Ok(fonts)
    }

    fn locate(&mut self, source: &Source) -> Option<&Choice> {
        self.choices.iter().find(|choice| match source {
            Source::System { postscript } => choice.system.as_ref() == Some(postscript),
            Source::Imported { hash, index } => {
                choice.system.is_none()
                    && choice.index == *index
                    && choice
                        .path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .is_none_or(|stem| !is_hex(stem, 64) || stem == hash)
                    && file_hash(&choice.path, &mut self.hashes) == Some(hash.as_str())
            }
        })
    }

    fn bind(
        &mut self,
        renderer: &FontRenderer,
        key: &str,
        reference: Reference,
        choice: Option<Choice>,
    ) -> Result<()> {
        let (name, id, file) = match choice {
            Some(choice) => (
                choice.name,
                Some(choice.id),
                Some((choice.path, choice.index)),
            ),
            None => (reference.name, None, None),
        };
        renderer.bind_face_reference(key.as_bytes().to_vec(), name.clone(), file)?;
        self.labels.insert(key.to_owned(), (name, id));
        Ok(())
    }

    pub fn select(&mut self, renderer: &FontRenderer, id: &str) -> Result<String> {
        let choice = self
            .choices
            .iter()
            .find(|choice| choice.id == id)
            .ok_or("font is unavailable")?;
        let source = if let Some(postscript) = &choice.system {
            Source::System {
                postscript: postscript.clone(),
            }
        } else {
            let hash =
                file_hash(&choice.path, &mut self.hashes).ok_or("cannot read imported font")?;
            Source::Imported {
                hash: hash.to_owned(),
                index: choice.index,
            }
        };
        let reference = Reference {
            source,
            name: choice.name.clone(),
        };
        let key = reference.write(&self.directory)?;
        self.bind(renderer, &key, reference, Some(choice.clone()))?;
        Ok(key)
    }

    pub fn label(&self, handle: &str) -> Option<(&str, Option<&str>)> {
        self.labels
            .get(handle)
            .map(|(name, id)| (name.as_str(), id.as_deref()))
    }
}
