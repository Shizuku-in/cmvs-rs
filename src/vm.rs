//! Verified VM value/stack conventions. The bounded executor lives in `machine`.
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

/// Frame operand address: the original adds or subtracts `(int)raw` to `(int)frame[depth]`
/// (0x1400652e0/0x1400657b0), so raw values with bit 31 set reach the other side of the base.
pub(crate) fn frame_offset(base: usize, raw: u32, below_base: bool) -> Option<usize> {
    let (base, raw) = (i64::try_from(base).ok()?, i64::from(raw as i32));
    let offset = if below_base { base - raw } else { base + raw };
    usize::try_from(offset).ok()
}

/// Previous contents of a stack word; None marks a byte that was never written.
pub type StackWord = [Option<u8>; 4];

/// Explicit byte stack: return flags encode byte counts, not counts of values.
/// 0x0440/0x442 reserve bytes without writing them; the original leaves stale buffer
/// contents there, which this model keeps unknown instead of guessing.
#[derive(Debug, Clone)]
pub struct ParameterStack {
    bytes: Vec<Option<u8>>,
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
        self.reserve(value, 4)
    }
    /// 0x0440/0x442 (0x140067930): write `value` at SP, then advance SP by `count` bytes.
    /// Only the bytes below the new SP are kept; bytes past the first word stay unknown.
    pub fn reserve(&mut self, value: u32, count: usize) -> Result<(), FormatError> {
        if self.limit.saturating_sub(self.bytes.len()) < count {
            return Err(FormatError("parameter stack limit"));
        }
        let value = value.to_le_bytes();
        self.bytes.extend((0..count).map(|i| value.get(i).copied()));
        Ok(())
    }
    fn word(&self, start: usize) -> Result<u32, FormatError> {
        let mut word = [0; 4];
        for (byte, stored) in word.iter_mut().zip(&self.bytes[start..start + 4]) {
            *byte = stored.ok_or(FormatError(
                "read of stack bytes reserved but never written",
            ))?;
        }
        Ok(u32::from_le_bytes(word))
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
        self.word(start)
    }
    fn live(&self, offset: usize) -> Result<(), FormatError> {
        match offset.checked_add(4) {
            Some(end) if end <= self.bytes.len() => Ok(()),
            _ => Err(FormatError("stack access outside live bytes")),
        }
    }
    /// Absolute byte offset from the stack base, as used by frame-relative operands.
    pub fn read_u32_at(&self, offset: usize) -> Result<u32, FormatError> {
        self.live(offset)?;
        self.word(offset)
    }
    /// Overwrites a live word in place, including never-written bytes, and returns the
    /// previous contents so a failed transaction can restore them exactly.
    pub fn write_u32_at(&mut self, offset: usize, value: u32) -> Result<StackWord, FormatError> {
        self.live(offset)?;
        let old = [0, 1, 2, 3].map(|i| self.bytes[offset + i]);
        for (stored, byte) in self.bytes[offset..offset + 4]
            .iter_mut()
            .zip(value.to_le_bytes())
        {
            *stored = Some(byte);
        }
        Ok(old)
    }
    /// Undo of `write_u32_at`, restoring unknown bytes as unknown.
    pub(crate) fn restore_at(&mut self, offset: usize, old: StackWord) {
        self.bytes[offset..offset + 4].copy_from_slice(&old);
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
