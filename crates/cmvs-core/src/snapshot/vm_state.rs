//! Read-only VM-domain inspection. Host records remain explicitly uninterpreted.
use super::{LocalVariables, SnapshotRecord, parse_records, unique_body};
use crate::{FormatError, bytecode, ps3::Script, save::Csv2Save};

fn words<const N: usize>(bytes: &[u8]) -> Result<[u32; N], FormatError> {
    if bytes.len() != N * 4 {
        return Err(FormatError("snapshot VM field has unexpected length"));
    }
    Ok(std::array::from_fn(|i| {
        u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
    }))
}

/// Decoded fields are not a restored Machine or engine. In particular, callback,
/// timer, object, message, camera and audio records still need independent loaders.
#[derive(Debug)]
pub struct VmSnapshot<'a> {
    pub script_names: [&'a [u8]; 4],
    pub script_sizes: [u32; 4],
    pub active_slot: u8,
    pub pc: u32,
    pub repeat_count: u32,
    pub stack_pointer: usize,
    pub frame_depth: usize,
    pub frames: [u32; 64],
    /// All 64 KiB, including bytes beyond SP; no claim they are live stack data.
    pub stack: &'a [u8],
    /// Preserve all eleven saved words; the current executor exposes only 0..6.
    pub system_results: [u32; 11],
    pub variables: LocalVariables,
    /// Section 2 decoded without applying PS2A decryption a second time, with
    /// record 0x900 overlaid on the mutable data segment as the original loader does.
    pub main_script: Script,
    /// True only when the active script is the main image and its PC was checked.
    pub active_pc_verified: bool,
    pub uninterpreted: Vec<SnapshotRecord>,
}

impl<'a> VmSnapshot<'a> {
    pub fn decode(save: &'a Csv2Save) -> Result<Self, FormatError> {
        let bytes = &save.snapshot;
        let records = parse_records(bytes)?;
        let field = |tag| unique_body(bytes, &records, tag);
        let mut names = [&[][..]; 4];
        let mut sizes = [0; 4];
        for i in 0..4 {
            let name = field(0x102 + i as u16)?;
            if name.is_empty()
                || name.len() > 32
                || name.last() != Some(&0)
                || name[..name.len() - 1].contains(&0)
            {
                return Err(FormatError(
                    "snapshot script name is not a bounded C string",
                ));
            }
            names[i] = &name[..name.len() - 1];
            sizes[i] = words::<1>(field(0x112 + i as u16)?)?[0];
            if names[i].is_empty() != (sizes[i] == 0) {
                return Err(FormatError("snapshot script name and image size disagree"));
            }
        }
        let slot = words::<1>(field(0x106)?)?[0];
        if slot > 3 || names[slot as usize].is_empty() {
            return Err(FormatError(
                "snapshot active script slot is absent or out of range",
            ));
        }
        let pc = words::<1>(field(0x107)?)?[0];
        if pc >= sizes[slot as usize] {
            return Err(FormatError("snapshot PC exceeds its script memory image"));
        }
        let repeat_count = words::<1>(field(0x108)?)?[0];
        let stack_pointer = words::<1>(field(0x240)?)?[0] as usize;
        let frame_depth = words::<1>(field(0x241)?)?[0] as usize;
        let frames = words::<64>(field(0x242)?)?;
        let stack = field(0x243)?;
        if stack.len() != 65536 || stack_pointer > stack.len() || frame_depth > 63 {
            return Err(FormatError(
                "snapshot stack pointer or frame depth exceeds bounds",
            ));
        }
        if frames[0] != 0
            || frames[..=frame_depth]
                .iter()
                .any(|&base| base as usize > stack_pointer)
        {
            return Err(FormatError(
                "snapshot active call frame lies outside live stack",
            ));
        }
        let system_results = words::<11>(field(0x248)?)?;
        let variables = LocalVariables::from_records(bytes, &records)?;
        if save.script_image.len() as u64 != u64::from(sizes[0]) {
            return Err(FormatError(
                "snapshot main script size differs from CSV2 section 2",
            ));
        }
        let mut main_script = Script::from_memory_image(&save.script_image)?;
        main_script.apply_snapshot_data(field(0x900)?)?;
        let active_pc_verified = slot == 0;
        if active_pc_verified {
            let instructions = bytecode::decode_layout(
                main_script.code(),
                main_script.header.entry_pc,
                &main_script.labels(),
            )
            .map_err(|_| FormatError("snapshot main script bytecode layout is invalid"))?;
            if !instructions.iter().any(|i| i.pc == pc as usize) {
                return Err(FormatError(
                    "snapshot PC is not a main script instruction boundary",
                ));
            }
        }
        let uninterpreted = records
            .into_iter()
            .filter(|r| {
                !matches!(r.tag,
                    0x102..=0x108 | 0x112..=0x115 | 0x240..=0x243 | 0x248 | 0x280..=0x283 | 0x900
                )
            })
            .collect();
        Ok(Self {
            script_names: names,
            script_sizes: sizes,
            active_slot: slot as u8,
            pc,
            repeat_count,
            stack_pointer,
            frame_depth,
            frames,
            stack,
            system_results,
            variables,
            main_script,
            active_pc_verified,
            uninterpreted,
        })
    }
}
