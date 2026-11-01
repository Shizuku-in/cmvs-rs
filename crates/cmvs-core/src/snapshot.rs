//! Engine save snapshot (0x140069270): record framing and its verified live-state effect.
//! Captures use the supported record layouts; skipped captures are tracked as unavailable.
use crate::{
    FormatError, bytecode::DecodeError, dispatch::Call, evaluate::error, machine::Machine,
    vm::SystemResult,
};
use std::ops::Range;

mod engine;
mod variables;
mod visual;
mod vm_state;
pub use engine::EngineSnapshot;
pub use variables::LocalVariables;
pub(crate) use visual::RESTORE_PIXELS;
pub use visual::{SavedImage, SavedScene, SavedWindow, VisualPreview, VisualSnapshot};
pub use vm_state::VmSnapshot;

/// Supported snapshot text layouts. Special image/effect records remain bounded by
/// their own decoders; a text layout does not grant support for unknown records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotLayout {
    TextV2,
    TextV3,
}
impl SnapshotLayout {
    pub(crate) fn text_version(self) -> u16 {
        match self {
            Self::TextV2 => 2,
            Self::TextV3 => 3,
        }
    }
}

fn unique_body<'a>(
    bytes: &'a [u8],
    records: &[SnapshotRecord],
    tag: u16,
) -> Result<&'a [u8], FormatError> {
    let mut found = records.iter().filter(|r| r.tag == tag);
    let record = found
        .next()
        .ok_or(FormatError("missing required snapshot record"))?;
    if found.next().is_some() {
        return Err(FormatError("duplicate snapshot record"));
    }
    Ok(&bytes[record.body.clone()])
}

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

/// The engine snapshot buffer +11176 with the CSV2 header it prepared at +11204 and the
/// main script memory image (+16592) the slot writer packs as section 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedSave {
    pub header: [u8; crate::save::HEADER_SIZE],
    pub snapshot: Vec<u8>,
    pub script_image: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SaveImage {
    /// Original buffer is still null; its save writer returns without writing.
    #[default]
    Absent,
    /// The original captured an image here; this research host did not build it.
    /// Save writers must fail explicitly rather than write a fabricated snapshot.
    Unavailable { script: Vec<u8>, pc: usize },
    /// A complete snapshot, built at 0x2080/0x22b2 or loaded from a slot.
    Captured(Box<CapturedSave>),
}

impl SaveImage {
    /// 0x22b2 / 0x14007c9a0 captures the next PC with repeat count zero.
    /// Only an explicit research policy may proceed without the serialized image.
    pub fn try_handle(
        &mut self,
        vm: &mut Machine<'_>,
        continue_without_image: bool,
    ) -> Result<bool, DecodeError> {
        let Some((pc, 0x2b2)) = vm.pending_system() else {
            return Ok(false);
        };
        if !continue_without_image {
            return Ok(false);
        }
        // Validate/apply the only fallible live-state change before completing the
        // verified no-argument command. Completion advances PC and resets repeat.
        vm.state_mut()
            .set_bit(0, false)
            .map_err(|why| error(pc, 0x22b2, why))?;
        vm.complete_system(SystemResult(0x4000))?;
        *self = Self::Unavailable {
            script: vm.current_name().to_vec(),
            pc: vm.pc(),
        };
        Ok(true)
    }

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
    /// Record 0x100.
    pub(crate) fn restore(&mut self, title: Vec<u8>) {
        self.0 = title;
    }
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, 0x162)) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle(&mut Call::new(vm, pc, 0x162))?;
        Ok(true)
    }

    pub(crate) fn handle(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
        let pc = call.pc;
        let raw = call.argument(4, "title")?;
        let title = call.vm.resolve_string(raw)?;
        if title.len() >= 128 {
            // The original skips the copy and sets +12056 bit 8, which is not modeled.
            return Err(error(
                pc,
                0x2162,
                "save title of 128+ bytes sets an unmodeled flag",
            ));
        }
        call.complete(0x4004)?;
        self.0 = title;
        Ok(())
    }
}

/// +11828/+11832/+11836: three words the snapshot writer copies into CSV2 header offsets
/// 528/532/536, save loading (0x140081960) restores, and the save slot query 0x22bd
/// (0x14007cf10) returns as system results 4..6 for menu scripts. No constructor or reset
/// write was found, so they stay unknown until set. 0x22ba (+11832/+11836) is verified but
/// not yet met on the real path.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SaveInfo([Option<u32>; 3]);

impl SaveInfo {
    pub fn values(&self) -> [Option<u32>; 3] {
        self.0
    }
    /// 0x22b9, inline in the dispatcher: +11828 = SP-4, pops 4 bytes. 0x22ba (inline):
    /// +11832 = SP-4 and +11836 = SP-8, pops 8.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x2b9 | 0x2ba))) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle(&mut Call::new(vm, pc, command))?;
        Ok(true)
    }

    pub(crate) fn handle(&mut self, call: &mut Call<'_, '_>) -> Result<(), DecodeError> {
        let command = call.command;
        let first = call.argument(4, "first word")?;
        if command == 0x2b9 {
            call.complete(0x4004)?;
            self.0[0] = Some(first);
        } else {
            let second = call.argument(8, "second word")?;
            call.complete(0x4008)?;
            self.0[1] = Some(first);
            self.0[2] = Some(second);
        }
        Ok(())
    }
    /// Loader 0x140081960: header 528/532/536, then +11832 cleared.
    pub(crate) fn restore(&mut self, words: [u32; 3]) {
        self.0 = words.map(Some);
    }
}
