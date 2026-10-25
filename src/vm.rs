//! Verified VM value/stack conventions. No script execution or host handlers yet.
use crate::FormatError;

/// Strings are 32-bit tagged offsets, not native host pointers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringRef {
    Text(u32),
    ScriptData(u32),
    StringBank(u32),
    Frame(u32),
}

impl StringRef {
    pub fn decode(raw: u32) -> Self {
        let offset = raw & 0x3fff_ffff;
        match raw >> 30 {
            0 => Self::Text(offset),
            1 => Self::ScriptData(offset),
            2 => Self::StringBank(offset),
            _ => Self::Frame(offset),
        }
    }
}

/// Return word from the system dispatcher, as consumed by the main VM loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemResult(pub u16);

impl SystemResult {
    pub fn yields(self) -> bool {
        self.0 & 0x8000 != 0
    }
    pub fn advances_pc(self) -> bool {
        self.0 & 0x4000 != 0
    }
    pub fn increments_repeat_counter(self) -> bool {
        self.0 & 0x2000 != 0
    }
    /// The original VM consumes this byte count only when bit 14 is set.
    pub fn cleanup_bytes(self) -> usize {
        if self.advances_pc() {
            usize::from(self.0 as u8)
        } else {
            0
        }
    }
}

/// Explicit byte stack: return flags encode byte counts, not counts of values.
#[derive(Debug)]
pub struct ParameterStack {
    bytes: Vec<u8>,
    limit: usize,
}

impl ParameterStack {
    /// Caller chooses a bounded research policy; this does not infer original capacity.
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
    pub fn len_bytes(&self) -> usize {
        self.bytes.len()
    }
    pub fn push_u32(&mut self, value: u32) -> Result<(), FormatError> {
        if self.limit.saturating_sub(self.bytes.len()) < 4 {
            return Err(FormatError("parameter stack limit"));
        }
        self.bytes.extend(value.to_le_bytes());
        Ok(())
    }
    /// distance=4 reads the last pushed word; distance=8 reads the preceding word.
    pub fn peek_u32(&self, distance: usize) -> Result<u32, FormatError> {
        if distance < 4 {
            return Err(FormatError("invalid stack distance"));
        }
        let start = self
            .bytes
            .len()
            .checked_sub(distance)
            .ok_or(FormatError("parameter stack underflow"))?;
        let word = &self.bytes[start..start + 4];
        Ok(u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
    }
    pub fn discard(&mut self, count: usize) -> Result<(), FormatError> {
        let length = self
            .bytes
            .len()
            .checked_sub(count)
            .ok_or(FormatError("parameter stack underflow"))?;
        self.bytes.truncate(length);
        Ok(())
    }
    pub fn apply_cleanup(&mut self, result: SystemResult) -> Result<(), FormatError> {
        self.discard(result.cleanup_bytes())
    }
}
