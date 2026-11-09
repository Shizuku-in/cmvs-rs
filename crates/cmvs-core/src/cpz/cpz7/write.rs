//! Compatibility entry point; all archive versions use the common replacement writer.
use super::*;
use std::{collections::BTreeMap, io::Write};

impl Cpz7Index {
    /// Replace entries in a new, empty output stream using the original CPZ7 keys.
    /// See `ArchiveIndex::write_replacements` for the shared writing contract.
    pub fn write_replacements<R: Read + Seek, W: Write + Seek>(
        source: &mut R,
        output: &mut W,
        parameters: Cpz7Parameters,
        replacements: &BTreeMap<usize, Vec<u8>>,
    ) -> Result<(), ArchiveError> {
        let index = Self::read_with_parameters(source, parameters)?;
        super::super::ArchiveIndex::Cpz7(index).write_replacements(source, output, replacements)
    }
}
