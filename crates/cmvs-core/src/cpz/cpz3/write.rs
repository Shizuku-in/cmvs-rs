//! New flat CPZ3 archives use zero cipher keys; rewriting preserves source keys.
use super::*;
use crate::cpz::write::{empty_output, put};
use std::io::Write;

impl Cpz3Index {
    /// Write a new CPZ3 archive to an empty stream. Names are raw filename bytes
    /// (1..=4096 bytes, no NUL); duplicate names retain their order. Cipher keys
    /// default to zero. Failures may leave a partial output.
    pub fn write<W: Write + Seek>(
        output: &mut W,
        entries: &[(Vec<u8>, Vec<u8>)],
    ) -> Result<(), ArchiveError> {
        if entries.len() > 1_000_000 {
            return Err(FormatError("CPZ3 file count exceeds allocation limit").into());
        }
        let mut index_len = 0;
        let mut data_len = 0u64;
        for (name, bytes) in entries {
            if name.is_empty()
                || name.len() > 4096
                || name.contains(&0)
                || bytes.len() > crate::MAX_PAYLOAD
            {
                return Err(FormatError("invalid CPZ3 filename or resource size").into());
            }
            index_len += 25 + name.len();
            if index_len > crate::MAX_PAYLOAD || data_len > u64::from(u32::MAX) {
                return Err(FormatError("CPZ3 index or data offset exceeds bounds").into());
            }
            data_len += bytes.len() as u64;
        }
        empty_output(output)?;
        let mut index = Vec::with_capacity(index_len);
        let mut offset = 0u32;
        for (name, bytes) in entries {
            for value in [
                25 + name.len() as u32,
                bytes.len() as u32,
                offset,
                0,
                0,
                0xc7f5da63,
            ] {
                index.extend(value.to_le_bytes());
            }
            index.extend(name);
            index.push(0);
            // Only the start of each record needs to fit the format's offset word.
            offset = offset.wrapping_add(bytes.len() as u32);
        }
        let mut header = [0; CPZ3_HEADER_SIZE];
        header[..4].copy_from_slice(b"CPZ3");
        put(&mut header, 4, entries.len() as u32 ^ 0x5e9c4f37);
        put(&mut header, 8, index.len() as u32 ^ 0xf32aed17);
        put(&mut header, 16, 0xdddddddd ^ 0x7bf4a539);
        output.write_all(&header)?;
        output.write_all(&index)?;
        let mut at = 0;
        for (name, bytes) in entries {
            let mut bytes = bytes.clone();
            crypt(&mut bytes, 0, true);
            put(&mut index, at + 16, cpz3_checksum(&bytes));
            output.write_all(&bytes)?;
            at += 25 + name.len();
        }
        crypt(&mut index, 0, true);
        put(&mut header, 12, cpz3_checksum(&index));
        output.rewind()?;
        output.write_all(&header)?;
        output.write_all(&index)?;
        Ok(())
    }
}
