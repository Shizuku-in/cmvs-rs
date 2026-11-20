//! Script project assembly and translation-table import.
mod project;
mod table;
use crate::Result;
use cmvs_core::{MAX_PAYLOAD, format::ps3::Script, vm::machine::STRING_CAPACITY};
use cmvs_tooling::script::Key;
pub use cmvs_tooling::{
    script::{ScriptText, Segment},
    table::Encoding,
};
pub use project::assemble;
use std::collections::BTreeMap;
pub use table::import_text;

/// Replacements contain encoded bytes without their terminator. A shared suffix must still
/// be a suffix of its parent, or have an explicit replacement of its own.
pub fn rewrite(
    layout: &ScriptText<'_>,
    edits: &BTreeMap<(Segment, usize), Vec<u8>>,
) -> Result<(Script, usize)> {
    rewrite_with(layout, edits, |key| edits.contains_key(key))
}

fn rewrite_with(
    layout: &ScriptText<'_>,
    edits: &BTreeMap<Key, Vec<u8>>,
    explicit: impl Fn(&Key) -> bool,
) -> Result<(Script, usize)> {
    for (key, bytes) in edits {
        if !layout.entries().contains_key(key) || bytes.contains(&0) {
            return Err(format!("invalid string edit at {key:?}").into());
        }
    }
    let mut text = Vec::new();
    let mut offsets = BTreeMap::new();
    let mut data = layout.source().data().to_vec();
    let mut data_end = 0;
    let mut parent_end = 0;
    let mut parent_value: &[u8] = &[];
    let mut parent_target = 0;
    let limit = MAX_PAYLOAD - (layout.source().payload().len() - layout.source().text().len());
    for (&key, &original) in layout.entries() {
        let (segment, offset) = key;
        let edit = edits.get(&key);
        let value = edit.map(Vec::as_slice).unwrap_or(original);
        if segment == Segment::Data {
            if value.len() != original.len() {
                return Err(format!(
                    "data+{offset:#x}: edit must keep its {} byte length",
                    original.len()
                )
                .into());
            }
            let target = &mut data[offset..offset + value.len()];
            if offset < data_end && target != value {
                return Err(format!("data+{offset:#x}: overlapping string edits disagree").into());
            }
            target.copy_from_slice(value);
            data_end = offset + value.len() + 1;
            continue;
        }
        if offset < parent_end {
            if parent_value.ends_with(value) {
                offsets.insert(key, parent_target + parent_value.len() - value.len());
                continue;
            }
            if !explicit(&key) {
                let pc = layout
                    .references()
                    .iter()
                    .find(|(_, target)| *target == key)
                    .unwrap()
                    .0;
                return Err(format!(
                    "code+{pc:#x}: specify a translation for shared suffix text+{offset:#x}"
                )
                .into());
            }
        } else {
            parent_end = offset + original.len() + 1;
            parent_target = text.len();
            parent_value = value;
        }
        if value.len() >= limit - text.len() {
            return Err("translated script exceeds payload limit".into());
        }
        offsets.insert(key, text.len());
        text.extend_from_slice(value);
        text.push(0);
    }
    check_accumulators(layout, |key| {
        edits.get(key).map_or(layout.entries()[key].len(), Vec::len)
    })?;
    let mut code = layout.source().code().to_vec();
    let mut relocated = 0;
    for &(pc, key @ (segment, offset)) in layout.references() {
        if segment == Segment::Text && offsets[&key] != offset {
            code[pc + 2..pc + 6].copy_from_slice(&(offsets[&key] as u32).to_le_bytes());
            relocated += 1;
        }
    }
    let mut script = layout.source().with_segments(&code, &text)?;
    script.initial_data_mut().copy_from_slice(&data);
    Ok((script, relocated))
}

fn check_accumulators(layout: &ScriptText<'_>, length_of: impl Fn(&Key) -> usize) -> Result<()> {
    for (pc, copies) in layout.accumulators() {
        // Mutable data, slots and frame operands are only known when the VM runs.
        let mut length = 0;
        for key in copies {
            length += length_of(key);
            if length > STRING_CAPACITY {
                return Err(format!(
                    "code+{pc:#x}: string accumulator exceeds {STRING_CAPACITY} bytes"
                )
                .into());
            }
        }
    }
    Ok(())
}
