//! Minimal JSON values for probe reports; the workspace keeps report output dependency-free.
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Arr(Vec<Json>),
    /// Field order is kept so reports diff cleanly between runs.
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn obj<K: Into<String>>(fields: impl IntoIterator<Item = (K, Json)>) -> Self {
        Self::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
    pub fn arr(items: impl IntoIterator<Item = Json>) -> Self {
        Self::Arr(items.into_iter().collect())
    }
    /// Script names and other engine byte strings, ASCII-escaped like the other tools.
    pub fn bytes(bytes: &[u8]) -> Self {
        Self::Str(bytes.escape_ascii().to_string())
    }
    pub fn hex(value: u64) -> Self {
        Self::Str(format!("0x{value:x}"))
    }
    /// Adds or replaces a field of an object; other values are left unchanged.
    pub fn set(&mut self, key: &str, value: Json) {
        if let Self::Obj(fields) = self {
            match fields.iter_mut().find(|(k, _)| k == key) {
                Some((_, v)) => *v = value,
                None => fields.push((key.into(), value)),
            }
        }
    }
}

macro_rules! from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Json {
            fn from(v: $t) -> Self {
                // Report counters stay far below i64::MAX; saturate instead of wrapping.
                Self::Int(i64::try_from(v).unwrap_or(i64::MAX))
            }
        }
    )*};
}
from_int!(u8, u16, u32, u64, usize, i32, i64);

impl From<bool> for Json {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl From<&str> for Json {
    fn from(v: &str) -> Self {
        Self::Str(v.into())
    }
}
impl From<String> for Json {
    fn from(v: String) -> Self {
        Self::Str(v)
    }
}
impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(v: Option<T>) -> Self {
        v.map_or(Self::Null, Into::into)
    }
}

fn write_str(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if u32::from(c) < 0x20 => write!(f, "\\u{:04x}", u32::from(c))?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::Int(v) => write!(f, "{v}"),
            Self::Str(s) => write_str(f, s),
            Self::Arr(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Self::Obj(fields) => {
                f.write_str("{")?;
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write_str(f, k)?;
                    write!(f, ":{v}")?;
                }
                f.write_str("}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_strings_and_keeps_field_order() {
        let mut value = Json::obj([("b", Json::from("a\"\\\n\u{1}")), ("a", Json::Null)]);
        value.set("c", Json::arr([1u8.into(), true.into()]));
        value.set("b", 2u32.into());
        assert_eq!(value.to_string(), r#"{"b":2,"a":null,"c":[1,true]}"#);
        assert_eq!(Json::from("q\"\\\n\u{1}").to_string(), r#""q\"\\\n\u0001""#);
        assert_eq!(Json::bytes(b"a\x82.bin").to_string(), r#""a\\x82.bin""#);
    }
}
