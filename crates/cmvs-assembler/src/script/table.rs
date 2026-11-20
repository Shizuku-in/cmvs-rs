use super::{Encoding, ScriptText, Segment, rewrite_with};
use cmvs_core::format::ps3::Script;
use cmvs_tooling::{
    Result,
    table::{FORMAT, escape, fingerprint, unescape},
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io::BufRead,
};

pub fn import_text(script: &Script, table: impl BufRead, target: Encoding) -> Result<Script> {
    let mut lines = table.lines();
    let header = lines.next().ok_or("missing translation table header")??;
    let mut fields = header.split('\t');
    if fields.next() != Some(FORMAT) {
        return Err("unknown translation table version".into());
    }
    let source = Encoding::parse(fields.next().ok_or("missing source encoding")?)?;
    if fields.next() != Some(fingerprint(script)?.as_str()) || fields.next().is_some() {
        return Err("translation table source script SHA-256 mismatch".into());
    }

    let layout = ScriptText::new(script)?;
    let mut seen = BTreeSet::new();
    let mut edits = BTreeMap::new();
    for (i, line) in lines.enumerate() {
        let line = line?;
        let number = i + 2;
        let mut f = line.split('\t');
        let segment = match f.next() {
            Some("text") => Segment::Text,
            Some("data") => Segment::Data,
            _ => return Err(format!("line {number}: expected text or data").into()),
        };
        let offset = usize::from_str_radix(f.next().ok_or("missing offset")?, 16)?;
        let key = (segment, offset);
        let bytes = layout
            .entries()
            .get(&key)
            .ok_or_else(|| format!("line {number}: unknown string offset"))?;
        if !seen.insert(key) || f.next() != Some(escape(bytes, source).as_str()) {
            return Err(format!("line {number}: duplicate row or original text mismatch").into());
        }
        if let Some(translation) = f.next() {
            let value = unescape(translation, target).map_err(|e| format!("line {number}: {e}"))?;
            edits.insert(key, value);
        }
        if f.next().is_some() {
            return Err(format!("line {number}: extra columns; escape embedded tabs").into());
        }
    }
    let explicit: BTreeSet<_> = edits.keys().copied().collect();
    if source != target {
        for (segment, offset, original) in layout.strings() {
            let key = (segment, offset);
            if let std::collections::btree_map::Entry::Vacant(entry) = edits.entry(key) {
                let value = source
                    .decode(original)
                    .and_then(|s| target.encode(&s).map(Cow::into_owned))
                    .map_err(|e| {
                        format!("{segment:?}+{offset:#x}: {e}; supply an explicit translation")
                    })?;
                if value != original {
                    entry.insert(value);
                }
            }
        }
    }
    Ok(rewrite_with(&layout, &edits, |key| explicit.contains(key))?.0)
}
