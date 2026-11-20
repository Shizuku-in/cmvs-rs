//! New flat CPZ3 archives use zero cipher keys; rewriting preserves source keys.
use super::*;
use crate::cpz::ArchiveVersion;
use std::io::Write;

impl Cpz3Index {
    /// Write a new CPZ3 archive to an empty stream. Names are raw filename bytes
    /// (1..=4096 bytes, no NUL); duplicate names retain their order. Cipher keys
    /// default to zero. Failures may leave a partial output.
    pub fn write<W: Write + Seek>(
        output: &mut W,
        entries: &[(Vec<u8>, Vec<u8>)],
    ) -> Result<(), ArchiveError> {
        let mut offset = 0u64;
        for (_, bytes) in entries {
            if bytes.len() > crate::MAX_PAYLOAD || offset > u64::from(u32::MAX) {
                return Err(FormatError("CPZ3 resource size or data offset exceeds bounds").into());
            }
            offset += bytes.len() as u64;
        }
        let names: Vec<_> = entries.iter().map(|(name, _)| name.as_slice()).collect();
        ArchiveVersion::Cpz3.write_flat(output, &names, |i| {
            let data = &entries[i].1;
            Ok((data.len() as u32, data.as_slice()))
        })
    }
}
