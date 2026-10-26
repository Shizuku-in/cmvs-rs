//! Engine save snapshot (0x140069270): record framing and its verified live-state effect.
//! Rust never builds an original snapshot image; skipped captures are tracked as unavailable.
use crate::{
    FormatError, bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult,
};
use std::ops::Range;

/// The original fixed buffer at engine +11176; larger streams would overflow it on load.
pub const MAX_SNAPSHOT: usize = 0x30_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// u16 length, payload.
    Short,
    /// u32 length, payload.
    Long,
    /// u16 object index, u32 length, payload.
    Indexed,
    /// Two u16-length strings; only tag 0x304.
    StringPair,
}

/// Framing of every tag emitted by the writer; the loader 0x140069fd0 reads the same layout.
pub fn framing(tag: u16) -> Option<Framing> {
    Some(match tag {
        0x100..=0x10b | 0x112..=0x115 | 0x200..=0x203 | 0x240 | 0x241 | 0x248 => Framing::Short,
        0x242
        | 0x243
        | 0x280..=0x283
        | 0x310
        | 0x318
        | 0x540
        | 0x600
        | 0x610
        | 0x620
        | 0x630
        | 0x6e0
        | 0x880..=0x883
        | 0x900 => Framing::Long,
        0x380 | 0x400 | 0x6a0 | 0x700 | 0x800 | 0x8c0 | 0x8c8 | 0x8e8 => Framing::Indexed,
        0x304 => Framing::StringPair,
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub offset: usize,
    pub tag: u16,
    pub index: Option<u16>,
    /// Payload after the length field; for StringPair, both length-prefixed strings.
    pub body: Range<usize>,
}

fn take(bytes: &[u8], at: &mut usize, n: usize) -> Result<Range<usize>, FormatError> {
    let end = at
        .checked_add(n)
        .filter(|&end| end <= bytes.len())
        .ok_or(FormatError("snapshot record exceeds stream"))?;
    let range = *at..end;
    *at = end;
    Ok(range)
}
fn word(bytes: &[u8], at: &mut usize) -> Result<u16, FormatError> {
    let r = take(bytes, at, 2)?;
    Ok(u16::from_le_bytes(bytes[r].try_into().expect("two bytes")))
}
fn dword(bytes: &[u8], at: &mut usize) -> Result<u32, FormatError> {
    let r = take(bytes, at, 4)?;
    Ok(u32::from_le_bytes(bytes[r].try_into().expect("four bytes")))
}

/// Walks a decoded CSV2 section 0 without interpreting payloads.
/// Unknown tags fail: the original loader would silently stop restoring at them.
pub fn parse_records(bytes: &[u8]) -> Result<Vec<SnapshotRecord>, FormatError> {
    if bytes.len() > MAX_SNAPSHOT {
        return Err(FormatError("snapshot exceeds original 3 MiB buffer"));
    }
    let mut records = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let offset = at;
        let tag = word(bytes, &mut at)?;
        let mut index = None;
        let body = match framing(tag).ok_or(FormatError("unknown snapshot record tag"))? {
            Framing::Short => {
                let n = word(bytes, &mut at)?;
                take(bytes, &mut at, n.into())?
            }
            Framing::Long => {
                let n = dword(bytes, &mut at)?;
                take(bytes, &mut at, n as usize)?
            }
            Framing::Indexed => {
                index = Some(word(bytes, &mut at)?);
                let n = dword(bytes, &mut at)?;
                take(bytes, &mut at, n as usize)?
            }
            Framing::StringPair => {
                let start = at;
                for _ in 0..2 {
                    let n = word(bytes, &mut at)?;
                    take(bytes, &mut at, n.into())?;
                }
                start..at
            }
        };
        records.push(SnapshotRecord {
            offset,
            tag,
            index,
            body,
        });
    }
    Ok(records)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SaveImage {
    /// Original buffer is still null; its save writer returns without writing.
    #[default]
    Absent,
    /// The original captured an image here; this research host did not build it.
    /// Save writers must fail explicitly rather than write a fabricated snapshot.
    Unavailable { script: Vec<u8>, pc: usize },
}

impl SaveImage {
    /// Applies 0x140069270's only VM-visible effect without building the image:
    /// bit 0 is set for the captured copy, then cleared in live state.
    /// The variable change-tracking flag it may raise is not modeled.
    pub fn capture_unavailable(&mut self, vm: &mut Machine<'_>) -> Result<(), &'static str> {
        vm.state_mut().set_bit(0, false)?;
        *self = Self::Unavailable {
            script: vm.current_name().to_vec(),
            pc: vm.pc(),
        };
        Ok(())
    }
}

/// +11840: appended to save labels by the snapshot writer and stored as record 0x100.
/// Constructor 0x140016e20 starts it empty; 0x2162 replaces it.
#[derive(Debug, Default)]
pub struct SaveTitle(Vec<u8>);

impl SaveTitle {
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, 0x162)) = vm.pending_system() else {
            return Ok(false);
        };
        let raw = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2162, e.0))?;
        let title = vm.resolve_string(raw)?;
        if title.len() >= 128 {
            // The original skips the copy and sets +12056 bit 8, which is not modeled.
            return Err(error(
                pc,
                0x2162,
                "save title of 128+ bytes sets an unmodeled flag",
            ));
        }
        vm.complete_system(SystemResult(0x4004))?;
        self.0 = title;
        Ok(true)
    }
}
