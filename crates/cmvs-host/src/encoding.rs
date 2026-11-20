//! Game text encoding resolution over [`HostFs`], including `encoding.txt`,
//! fan-patch `conversion.json` manifests, and bounded script-text detection.
use cmvs_core::{TextEncoding, format::ps3::Script, host::filesystem::HostFs, profile::GameFiles};
use cmvs_font::text_codec::{self, DetectError};
use std::{
    error::Error,
    io::{ErrorKind, Read as _},
    path::Path,
};

const MAX_CONVERSION_JSON: usize = 256 * 1024;
const MAX_JSON_DEPTH: usize = 32;

/// Policy for resolving a game's [`TextEncoding`] during session bootstrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncodingPolicy {
    /// Use an explicit encoding directly without inspecting `encoding.txt`,
    /// `conversion.json`, or game scripts.
    Explicit(TextEncoding),
    /// Check `encoding.txt`, then `conversion.json`, then script detection, falling back to
    /// [`TextEncoding::ShiftJis`] only when script text is purely ASCII/uncertain.
    #[default]
    AutoDefaultShiftJis,
    /// Check `encoding.txt`, then `conversion.json`, then script detection, returning
    /// the supplied error message if script text is uncertain.
    AutoStrict(&'static str),
}

/// Resolves a game's [`TextEncoding`] through `fs` according to `policy`.
///
/// For automatic policies, precedence is:
/// 1. `root/encoding.txt`
/// 2. `root/conversion.json` top-level `"encoding"` field (written by `tools/fan_patch`)
/// 3. Bounded startup and archived script text detection ([`text_codec::select_or_default_with_fs`]
///    or [`text_codec::detect_game_with_fs`])
pub fn resolve_encoding_with_fs(
    fs: &dyn HostFs,
    root: &Path,
    files: &GameFiles,
    startup: &Script,
    policy: EncodingPolicy,
) -> Result<TextEncoding, Box<dyn Error + Send + Sync>> {
    resolve_profile_encoding_with_fs(fs, root, files, startup, policy, None)
}
pub fn resolve_profile_encoding_with_fs(
    fs: &dyn HostFs,
    root: &Path,
    files: &GameFiles,
    startup: &Script,
    policy: EncodingPolicy,
    profile: Option<&cmvs_core::profile::EngineProfile>,
) -> Result<TextEncoding, Box<dyn Error + Send + Sync>> {
    if let EncodingPolicy::Explicit(encoding) = policy {
        return Ok(encoding);
    }
    if let Some(encoding) = text_codec::encoding_txt_with_fs(fs, root)? {
        return Ok(encoding);
    }
    if let Some(encoding) = conversion_encoding_with_fs(fs, root)? {
        return Ok(encoding);
    }
    match text_codec::detect_game_for_profile(fs, root, files, startup, profile) {
        Ok(encoding) => Ok(encoding),
        Err(DetectError::Uncertain) => match policy {
            EncodingPolicy::AutoDefaultShiftJis => Ok(TextEncoding::ShiftJis),
            EncodingPolicy::AutoStrict(message) => Err(message.into()),
            EncodingPolicy::Explicit(_) => unreachable!(),
        },
        Err(error) => Err(Box::new(error)),
    }
}

/// Reads the top-level `"encoding"` setting from `root/conversion.json` if that file exists.
pub fn conversion_encoding_with_fs(
    fs: &dyn HostFs,
    root: &Path,
) -> Result<Option<TextEncoding>, Box<dyn Error + Send + Sync>> {
    let file = match fs.open_read(&root.join("conversion.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Box::new(error)),
    };
    let mut raw = Vec::new();
    file.take(MAX_CONVERSION_JSON as u64 + 1)
        .read_to_end(&mut raw)?;
    if raw.len() > MAX_CONVERSION_JSON {
        return Err("conversion.json exceeds 256 KiB limit".into());
    }
    let text = String::from_utf8(raw)?;
    parse_conversion_json_encoding(&text)
}

fn parse_conversion_json_encoding(
    text: &str,
) -> Result<Option<TextEncoding>, Box<dyn Error + Send + Sync>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut parser = JsonScanner::new(text);
    let encoding = parser.parse_top_object()?;
    parser.skip_ws();
    if !parser.is_eof() {
        return Err("conversion.json has trailing data".into());
    }
    let Some(raw) = encoding else {
        return Ok(None);
    };
    let normalized = if raw == "utf8" { "utf-8" } else { raw.as_str() };
    text_codec::parse(normalized).map(Some).map_err(|_| {
        format!("conversion.json encoding must be shift-jis, gbk or utf-8, not {raw:?}").into()
    })
}

struct JsonScanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonScanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            pos: 0,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\r' | b'\n') = self.peek() {
            self.pos += 1;
        }
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), &'static str> {
        self.skip_ws();
        if self.peek() == Some(expected) {
            self.pos += 1;
            Ok(())
        } else {
            Err("malformed conversion.json")
        }
    }

    fn parse_top_object(&mut self) -> Result<Option<String>, Box<dyn Error + Send + Sync>> {
        self.expect_byte(b'{')?;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(None);
        }
        let mut found = None;
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.expect_byte(b':')?;
            self.skip_ws();
            if key == "encoding" {
                if self.peek() != Some(b'"') {
                    return Err("conversion.json \"encoding\" must be a string".into());
                }
                found = Some(self.parse_string()?);
            } else {
                self.skip_value(0)?;
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(found);
                }
                _ => return Err("malformed conversion.json".into()),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, &'static str> {
        if self.peek() != Some(b'"') {
            return Err("malformed conversion.json string");
        }
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let start = self.pos;
            while let Some(&b) = self.bytes.get(self.pos) {
                if b == b'"' || b == b'\\' {
                    break;
                }
                if b < 0x20 {
                    return Err("unescaped control character in conversion.json");
                }
                self.pos += 1;
            }
            let chunk = std::str::from_utf8(&self.bytes[start..self.pos])
                .map_err(|_| "invalid UTF-8 in conversion.json")?;
            out.push_str(chunk);
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self
                        .peek()
                        .ok_or("unterminated escape in conversion.json")?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex4()?;
                            if (0xd800..=0xdbff).contains(&code) {
                                if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                    return Err("invalid surrogate pair in conversion.json");
                                }
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err("invalid low surrogate in conversion.json");
                                }
                                let scalar = 0x10000
                                    + (((u32::from(code) - 0xd800) << 10)
                                        | (u32::from(low) - 0xdc00));
                                out.push(
                                    char::from_u32(scalar)
                                        .ok_or("invalid Unicode scalar in conversion.json")?,
                                );
                            } else {
                                out.push(
                                    char::from_u32(u32::from(code))
                                        .ok_or("invalid Unicode escape in conversion.json")?,
                                );
                            }
                        }
                        _ => return Err("invalid escape sequence in conversion.json"),
                    }
                }
                _ => break,
            }
        }
        Err("unterminated string in conversion.json")
    }

    fn parse_hex4(&mut self) -> Result<u16, &'static str> {
        let hex = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or("truncated \\u escape in conversion.json")?;
        let text = std::str::from_utf8(hex).map_err(|_| "invalid hex escape in conversion.json")?;
        let value =
            u16::from_str_radix(text, 16).map_err(|_| "invalid hex escape in conversion.json")?;
        self.pos += 4;
        Ok(value)
    }

    fn skip_value(&mut self, depth: usize) -> Result<(), &'static str> {
        if depth > MAX_JSON_DEPTH {
            return Err("conversion.json nesting exceeds limit");
        }
        self.skip_ws();
        match self.peek().ok_or("unexpected EOF in conversion.json")? {
            b'"' => {
                let _ = self.parse_string()?;
                Ok(())
            }
            b'{' => {
                self.pos += 1;
                self.skip_ws();
                if self.peek() == Some(b'}') {
                    self.pos += 1;
                    return Ok(());
                }
                loop {
                    self.skip_ws();
                    let _ = self.parse_string()?;
                    self.expect_byte(b':')?;
                    self.skip_value(depth + 1)?;
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return Ok(());
                        }
                        _ => return Err("malformed object in conversion.json"),
                    }
                }
            }
            b'[' => {
                self.pos += 1;
                self.skip_ws();
                if self.peek() == Some(b']') {
                    self.pos += 1;
                    return Ok(());
                }
                loop {
                    self.skip_value(depth + 1)?;
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(());
                        }
                        _ => return Err("malformed array in conversion.json"),
                    }
                }
            }
            b't' => self.consume_literal(b"true"),
            b'f' => self.consume_literal(b"false"),
            b'n' => self.consume_literal(b"null"),
            b'-' | b'0'..=b'9' => self.skip_number(),
            _ => Err("invalid value in conversion.json"),
        }
    }

    fn consume_literal(&mut self, lit: &[u8]) -> Result<(), &'static str> {
        if self.bytes.get(self.pos..self.pos + lit.len()) == Some(lit) {
            self.pos += lit.len();
            Ok(())
        } else {
            Err("invalid literal in conversion.json")
        }
    }

    fn skip_number(&mut self) -> Result<(), &'static str> {
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err("invalid number in conversion.json"),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err("invalid fraction in conversion.json");
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err("invalid exponent in conversion.json");
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        Ok(())
    }
}
