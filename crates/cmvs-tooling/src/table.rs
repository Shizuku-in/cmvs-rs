//! Encoding and byte escapes for the versioned UTF-8 translation table.
use crate::Result;
use cmvs_core::format::{
    digest::{sha256, to_hex},
    ps3::Script,
};
use encoding_rs::{GBK, SHIFT_JIS, UTF_8};
use std::{borrow::Cow, fmt::Write as _};

pub fn fingerprint(script: &Script) -> Result<String> {
    Ok(to_hex(&sha256(&script.encode()?)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    ShiftJis,
    Gbk,
    Utf8,
}

impl Encoding {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "shift-jis" => Ok(Self::ShiftJis),
            "gbk" => Ok(Self::Gbk),
            "utf-8" | "utf8" => Ok(Self::Utf8),
            _ => Err(format!("unknown encoding {s}; expected shift-jis, gbk or utf-8").into()),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ShiftJis => "shift-jis",
            Self::Gbk => "gbk",
            Self::Utf8 => "utf-8",
        }
    }
    fn codec(self) -> &'static encoding_rs::Encoding {
        match self {
            Self::ShiftJis => SHIFT_JIS,
            Self::Gbk => GBK,
            Self::Utf8 => UTF_8,
        }
    }
    pub fn decode(self, bytes: &[u8]) -> Result<Cow<'_, str>> {
        self.codec()
            .decode_without_bom_handling_and_without_replacement(bytes)
            .ok_or_else(|| format!("invalid {} bytes", self.name()).into())
    }
    pub fn encode(self, s: &str) -> Result<Cow<'_, [u8]>> {
        if s.chars().any(|c| c == '\0' || u32::from(c) > 0xffff) {
            return Err("text contains NUL or a character outside the BMP".into());
        }
        let (out, _, errors) = self.codec().encode(s);
        if errors {
            return Err(format!("text cannot be represented in {}", self.name()).into());
        }
        Ok(out)
    }
}

pub fn escape(bytes: &[u8], encoding: Encoding) -> String {
    let mut result = String::new();
    if let Ok(text) = encoding.decode(bytes)
        && encoding.encode(&text).is_ok_and(|v| v.as_ref() == bytes)
    {
        for c in text.chars() {
            match c {
                '\\' => result.push_str("\\\\"),
                '\t' => result.push_str("\\t"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                c if c.is_control() => {
                    for b in encoding.encode(c.encode_utf8(&mut [0; 4])).unwrap().iter() {
                        write!(result, "\\x{b:02x}").unwrap();
                    }
                }
                c => result.push(c),
            }
        }
    } else {
        for b in bytes {
            write!(result, "\\x{b:02x}").unwrap();
        }
    }
    result
}

pub fn unescape(mut text: &str, target: Encoding) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some((plain, tail)) = text.split_once('\\') {
        out.extend_from_slice(&target.encode(plain)?);
        let mut chars = tail.chars();
        out.push(match chars.next() {
            Some('\\') => b'\\',
            Some('t') => b'\t',
            Some('r') => b'\r',
            Some('n') => b'\n',
            Some('x') => {
                let hex = chars
                    .as_str()
                    .get(..2)
                    .filter(|s| s.bytes().all(|b| b.is_ascii_hexdigit()))
                    .ok_or("invalid hex byte escape")?;
                chars = chars.as_str()[2..].chars();
                u8::from_str_radix(hex, 16)?
            }
            _ => return Err("unknown byte escape".into()),
        });
        text = chars.as_str();
    }
    out.extend_from_slice(&target.encode(text)?);
    Ok(out)
}

pub const FORMAT: &str = "cmvs-text-v1";
