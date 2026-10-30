//! Byte-preserving INI updates; original game files are never written.
use crate::FormatError;
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

pub const MAX_CONFIGURATION: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayPreferences {
    pub fullscreen: bool,
    pub aspect_mode: u32,
    /// Requested device +112 mode before the backend capability fallback.
    pub render_mode: u32,
}
impl Default for DisplayPreferences {
    fn default() -> Self {
        Self {
            fullscreen: false,
            aspect_mode: 1,
            render_mode: 1,
        }
    }
}
impl DisplayPreferences {
    pub fn from_ini(bytes: &[u8]) -> Self {
        let get =
            |key, default| crate::save_menu::profile_int(bytes, "CMVS_RS_DESKTOP", key, default);
        Self {
            fullscreen: get("FULLSCREEN", 0) != 0,
            aspect_mode: match get("ASPECT_MODE", 1) as u32 {
                n @ 0..=3 => n,
                _ => 1,
            },
            render_mode: render_mode_from_ini(crate::save_menu::profile_int(
                bytes,
                "GRAPHIC_CONFIG",
                "ASPECT_RATIO_FILTER",
                0,
            )),
        }
    }
}
/// 0x140013b9d: the stored word swaps 0 and 1; negative or >= 4 becomes 0.
fn render_mode_from_ini(raw: i32) -> u32 {
    match raw {
        0 | 1 => (1 - raw) as u32,
        2 | 3 => raw as u32,
        _ => 0,
    }
}
/// 0x14000cd63 writes the same swap for modes 0 and 1.
pub(crate) fn render_mode_to_ini(mode: u32) -> u32 {
    match mode {
        0 | 1 => 1 - mode,
        _ => mode,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configuration(Vec<u8>);
impl Configuration {
    pub fn new(bytes: Vec<u8>) -> Result<Self, FormatError> {
        if bytes.len() > MAX_CONFIGURATION || bytes.contains(&0) {
            return Err(FormatError("invalid or oversized settings configuration"));
        }
        Ok(Self(bytes))
    }
    pub fn read(path: &Path) -> io::Result<Option<Self>> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut bytes = Vec::new();
        file.take(MAX_CONFIGURATION as u64 + 1)
            .read_to_end(&mut bytes)?;
        Self::new(bytes).map(Some).map_err(io::Error::other)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn set_integer(&mut self, section: &str, key: &str, value: u32) -> Result<(), FormatError> {
        Self::check_name(section, key)?;
        // Signed formatting preserves raw u32 words through the original profile-int reader.
        self.set(section, key, (value as i32).to_string().as_bytes())
    }
    /// Raw bytes in the game's encoding, as WritePrivateProfileStringA stores them.
    pub fn set_string(
        &mut self,
        section: &str,
        key: &str,
        value: &[u8],
    ) -> Result<(), FormatError> {
        Self::check_name(section, key)?;
        if value.iter().any(|b| matches!(b, 0 | b'\r' | b'\n')) {
            return Err(FormatError("invalid settings string value"));
        }
        self.set(section, key, value)
    }
    fn check_name(section: &str, key: &str) -> Result<(), FormatError> {
        if section.is_empty()
            || key.is_empty()
            || !section.is_ascii()
            || !key.is_ascii()
            || section
                .bytes()
                .chain(key.bytes())
                .any(|b| matches!(b, 0 | b'\r' | b'\n' | b'[' | b']' | b'='))
        {
            return Err(FormatError("invalid settings section or key"));
        }
        Ok(())
    }
    fn set(&mut self, section: &str, key: &str, value: &[u8]) -> Result<(), FormatError> {
        let mut active = false;
        let mut insertion = None;
        let mut first_section = false;
        let mut replacements = Vec::new();
        let mut offset = 0;
        for line in self.0.split_inclusive(|b| *b == b'\n') {
            let trimmed = line.trim_ascii();
            if let Some(name) = trimmed
                .strip_prefix(b"[")
                .and_then(|s| s.strip_suffix(b"]"))
            {
                if active && insertion.is_none() && first_section {
                    insertion = Some(offset);
                }
                active = name.trim_ascii().eq_ignore_ascii_case(section.as_bytes());
                if active {
                    first_section = true;
                }
            } else if active
                && let Some(eq) = trimmed.iter().position(|b| *b == b'=')
                && trimmed[..eq]
                    .trim_ascii()
                    .eq_ignore_ascii_case(key.as_bytes())
            {
                let ending = if line.ends_with(b"\r\n") {
                    b"\r\n".as_slice()
                } else if line.ends_with(b"\n") {
                    b"\n".as_slice()
                } else {
                    b"".as_slice()
                };
                let mut replacement = key.as_bytes().to_vec();
                replacement.push(b'=');
                replacement.extend(value);
                replacement.extend(ending);
                replacements.push((offset..offset + line.len(), replacement));
            }
            offset += line.len();
        }
        let mut candidate = self.0.clone();
        if !replacements.is_empty() {
            for (range, replacement) in replacements.into_iter().rev() {
                candidate.splice(range, replacement);
            }
        } else {
            let at = insertion.unwrap_or(candidate.len());
            let mut addition = Vec::new();
            if at > 0 && candidate[at - 1] != b'\n' {
                addition.extend(b"\r\n");
            }
            if !first_section {
                addition.extend(format!("[{section}]\r\n").as_bytes());
            }
            addition.extend(key.as_bytes());
            addition.push(b'=');
            addition.extend(value);
            addition.extend(b"\r\n");
            candidate.splice(at..at, addition);
        }
        *self = Self::new(candidate)?;
        Ok(())
    }
}
