//! Read and update CPZ resource archives.
//!
//! Use [`ArchiveIndex`] to open any supported archive version. [`ArchiveLayout`]
//! reads headers without decrypting the index. All six versions support entry replacement.
//!
//! # Read an entry
//!
//! ```no_run
//! use std::fs::File;
//! use cmvs_core::format::cpz::ArchiveIndex;
//!
//! let mut file = File::open("data/pack/script.cpz")?;
//! let index = ArchiveIndex::read(&mut file)?;
//! if let Some(entry) = index.lookup(b"main.ps3")? {
//!     let bytes = index.read_entry_verified(&mut file, entry)?;
//!     println!("script bytes: {}", bytes.len());
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Entry names are byte strings interpreted by the archive's lookup rules.
//! CPZ7 may require explicit [`Cpz7Parameters`]; CPZ5 supports
//! explicit [`Cpz5Mode`] selection when its packing mode is known.

mod cpz1;
mod cpz2;
mod cpz3;
mod cpz5;
mod cpz6;
mod cpz7;
mod crypto;
mod entry;
mod error;
mod flat;
mod write;

use crate::FormatError;
use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Seek, SeekFrom, Write},
};

/// Fold ASCII `A..=Z` to lowercase while preserving Shift-JIS two-byte sequences.
pub(super) fn fold_into(name: &[u8], out: &mut [u8]) {
    out.copy_from_slice(name);
    let mut at = 0;
    while at < out.len() {
        if matches!(out[at], 0x81..=0x9f | 0xe0..=0xfc) {
            at += 2;
        } else {
            out[at] = out[at].to_ascii_lowercase();
            at += 1;
        }
    }
}

pub(super) fn fold_vec(name: &[u8]) -> Vec<u8> {
    let mut out = vec![0; name.len()];
    fold_into(name, &mut out);
    out
}

/// Precomputed $O(1)$ exact and ASCII-case-folded lookup tables built once on open.
#[derive(Debug, Default)]
pub(super) struct LookupTable {
    exact: HashMap<Box<[u8]>, Option<usize>>,
    folded: HashMap<Box<[u8]>, usize>,
}

impl LookupTable {
    /// Only CPZ1/2 fold stored names. All versions fold the query at lookup.
    pub(super) fn from_entries(entries: &[Entry], lookup_key: fn(&[u8]) -> Vec<u8>) -> Self {
        let mut exact = HashMap::with_capacity(entries.len());
        let mut folded = HashMap::with_capacity(entries.len());
        for (i, entry) in entries.iter().enumerate() {
            exact
                .entry(entry.name.clone().into_boxed_slice())
                .and_modify(|slot| *slot = None)
                .or_insert(Some(i));
            folded
                .entry(lookup_key(&entry.name).into_boxed_slice())
                .or_insert(i);
        }
        Self { exact, folded }
    }

    pub(super) fn find(
        &self,
        name: &[u8],
        not_found: &'static str,
        ambiguous: &'static str,
    ) -> Result<usize, FormatError> {
        match self.exact.get(name) {
            Some(Some(index)) => Ok(*index),
            Some(None) => Err(FormatError(ambiguous)),
            None => Err(FormatError(not_found)),
        }
    }

    pub(super) fn lookup(&self, name: &[u8]) -> Option<usize> {
        if name.len() <= 64 {
            let mut buf = [0u8; 64];
            let key = &mut buf[..name.len()];
            fold_into(name, key);
            self.folded.get(key).copied()
        } else {
            let key = fold_vec(name);
            self.folded.get(key.as_slice()).copied()
        }
    }
}

pub use cpz1::{CPZ1_HEADER_SIZE, Cpz1Index, Cpz1Layout};
pub use cpz2::{CPZ2_HEADER_SIZE, Cpz2Index, Cpz2Layout};
pub use cpz3::{CPZ3_HEADER_SIZE, Cpz3Index, Cpz3Layout, cpz3_checksum};
pub use cpz5::{CPZ5_HEADER_SIZE, Cpz5Index, Cpz5Layout, Cpz5Mode};
pub use cpz6::{CPZ6_HEADER_SIZE, Cpz6Index, Cpz6Layout, lookup_key};
pub use cpz7::{CPZ7_HEADER_SIZE, Cpz7Index, Cpz7Layout, Cpz7Parameters};
pub use crypto::data_checksum;
pub use entry::Entry;
pub use error::ArchiveError;
pub use write::{ArchiveDirectory, ArchiveFile, DirectoryFormat, DirectoryNames};

/// Recognized signatures; recognition alone does not imply decoding support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveVersion {
    Cpz1,
    Cpz2,
    Cpz3,
    Cpz5,
    Cpz6,
    Cpz7,
}

impl ArchiveVersion {
    pub fn is_flat(self) -> bool {
        matches!(self, Self::Cpz1 | Self::Cpz2 | Self::Cpz3)
    }

    pub fn detect(bytes: &[u8]) -> Result<Self, ArchiveError> {
        let signature: [u8; 4] = bytes
            .get(..4)
            .ok_or(FormatError("truncated CPZ signature"))?
            .try_into()
            .expect("four-byte signature");
        match &signature {
            b"CPZ1" => Ok(Self::Cpz1),
            b"CPZ2" => Ok(Self::Cpz2),
            b"CPZ3" => Ok(Self::Cpz3),
            b"CPZ5" => Ok(Self::Cpz5),
            b"CPZ6" => Ok(Self::Cpz6),
            b"CPZ7" => Ok(Self::Cpz7),
            _ => Err(ArchiveError::UnknownSignature(signature)),
        }
    }
}

pub(crate) fn read_version<R: Read + Seek>(reader: &mut R) -> Result<ArchiveVersion, ArchiveError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut signature = [0; 4];
    reader.read_exact(&mut signature)?;
    ArchiveVersion::detect(&signature)
}

/// Header-only inspection, without decrypting or authenticating the index.
#[derive(Debug)]
pub enum ArchiveLayout {
    Cpz1(Cpz1Layout),
    Cpz2(Cpz2Layout),
    Cpz3(Cpz3Layout),
    Cpz5(Cpz5Layout),
    Cpz6(Cpz6Layout),
    Cpz7(Cpz7Layout),
}

impl ArchiveLayout {
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, ArchiveError> {
        match read_version(reader)? {
            ArchiveVersion::Cpz3 => {
                let length = reader.seek(SeekFrom::End(0))?;
                reader.seek(SeekFrom::Start(0))?;
                let mut header = [0; CPZ3_HEADER_SIZE];
                reader.read_exact(&mut header)?;
                Ok(Self::Cpz3(Cpz3Layout::parse(&header, length)?))
            }
            ArchiveVersion::Cpz5 => {
                let length = reader.seek(SeekFrom::End(0))?;
                reader.seek(SeekFrom::Start(0))?;
                let mut header = [0; CPZ5_HEADER_SIZE];
                reader.read_exact(&mut header)?;
                Ok(Self::Cpz5(Cpz5Layout::parse(&header, length)?))
            }
            ArchiveVersion::Cpz6 => {
                let length = reader.seek(SeekFrom::End(0))?;
                reader.seek(SeekFrom::Start(0))?;
                let mut header = [0; CPZ6_HEADER_SIZE];
                reader.read_exact(&mut header)?;
                Ok(Self::Cpz6(Cpz6Layout::parse(&header, length)?))
            }
            ArchiveVersion::Cpz7 => {
                let length = reader.seek(SeekFrom::End(0))?;
                reader.seek(SeekFrom::Start(0))?;
                let mut header = [0; CPZ7_HEADER_SIZE];
                reader.read_exact(&mut header)?;
                Ok(Self::Cpz7(Cpz7Layout::parse(&header, length)?))
            }
            ArchiveVersion::Cpz1 => {
                let length = reader.seek(SeekFrom::End(0))?;
                reader.seek(SeekFrom::Start(0))?;
                let mut header = [0; CPZ1_HEADER_SIZE];
                reader.read_exact(&mut header)?;
                Ok(Self::Cpz1(Cpz1Layout::parse(&header, length)?))
            }
            ArchiveVersion::Cpz2 => {
                let length = reader.seek(SeekFrom::End(0))?;
                reader.seek(SeekFrom::Start(0))?;
                let mut header = [0; CPZ2_HEADER_SIZE];
                reader.read_exact(&mut header)?;
                Ok(Self::Cpz2(Cpz2Layout::parse(&header, length)?))
            }
        }
    }
}

/// A decoded archive index for entry lookup and extraction.
///
/// Reuse the same archive reader for entry reads. Prefer [`Self::read_entry_verified`]
/// to check payloads during extraction. Reading the index does not read every payload.
#[derive(Debug)]
pub enum ArchiveIndex {
    Cpz1(Cpz1Index),
    Cpz2(Cpz2Index),
    Cpz3(Cpz3Index),
    Cpz5(Cpz5Index),
    Cpz6(Cpz6Index),
    Cpz7(Cpz7Index),
}

impl ArchiveIndex {
    /// Replace entries by index, retaining the archive version, names, directories,
    /// keys and packing/encryption modes. Other payloads are copied as verified
    /// ciphertext. `source` must match this index and `output` must be empty;
    /// failures may leave a partial output. No entries are added or removed.
    pub fn write_replacements<R: Read + Seek, W: Write + Seek>(
        &self,
        source: &mut R,
        output: &mut W,
        replacements: &BTreeMap<usize, Vec<u8>>,
    ) -> Result<(), ArchiveError> {
        write::rewrite(self, source, output, replacements)
    }

    /// Read an archive index, detecting its version and using default CPZ7 parameters.
    ///
    /// Seeks from the start of `reader`. Returns an error for unsupported or invalid
    /// archive data and I/O failures. Use [`Self::read_with_cpz7_parameters`] when
    /// the archive requires non-default keys.
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, ArchiveError> {
        Self::read_with_cpz7_parameters(reader, Cpz7Parameters::default())
    }

    /// Reopen an archive of the same version, using this index's packing mode or keys.
    pub fn read_with_same_settings<R: Read + Seek>(
        &self,
        reader: &mut R,
    ) -> Result<Self, ArchiveError> {
        let index = match self {
            Self::Cpz5(index) => Self::read_with_cpz5_mode(reader, index.mode)?,
            Self::Cpz7(index) => Self::read_with_cpz7_parameters(reader, index.parameters)?,
            _ => Self::read(reader)?,
        };
        if index.version() != self.version() {
            return Err(FormatError("archive version changed").into());
        }
        Ok(index)
    }

    /// Read an index with explicit CPZ7 keys; other archive versions ignore them.
    pub fn read_with_cpz7_parameters<R: Read + Seek>(
        reader: &mut R,
        parameters: Cpz7Parameters,
    ) -> Result<Self, ArchiveError> {
        match read_version(reader)? {
            ArchiveVersion::Cpz1 | ArchiveVersion::Cpz2
                if parameters != Cpz7Parameters::default() =>
            {
                Err(FormatError("CPZ7 parameters cannot be used with CPZ1/CPZ2").into())
            }
            ArchiveVersion::Cpz3 | ArchiveVersion::Cpz5 | ArchiveVersion::Cpz6
                if parameters != Cpz7Parameters::default() =>
            {
                Err(FormatError("CPZ7 parameters cannot be used with CPZ3/CPZ5/CPZ6").into())
            }
            ArchiveVersion::Cpz1 => Ok(Self::Cpz1(Cpz1Index::read(reader)?)),
            ArchiveVersion::Cpz2 => Ok(Self::Cpz2(Cpz2Index::read(reader)?)),
            ArchiveVersion::Cpz3 => Ok(Self::Cpz3(Cpz3Index::read(reader)?)),
            ArchiveVersion::Cpz5 => Ok(Self::Cpz5(Cpz5Index::read(reader)?)),
            ArchiveVersion::Cpz6 => Ok(Self::Cpz6(Cpz6Index::read(reader)?)),
            ArchiveVersion::Cpz7 => Ok(Self::Cpz7(Cpz7Index::read_with_parameters(
                reader, parameters,
            )?)),
        }
    }

    /// Read a CPZ5 index using a known packing mode; reject other archive versions.
    pub fn read_with_cpz5_mode<R: Read + Seek>(
        reader: &mut R,
        mode: Cpz5Mode,
    ) -> Result<Self, ArchiveError> {
        if read_version(reader)? != ArchiveVersion::Cpz5 {
            return Err(FormatError("CPZ5 packing mode requires a CPZ5 archive").into());
        }
        Ok(Self::Cpz5(Cpz5Index::read_with_mode(reader, mode)?))
    }

    /// Container version detected when this index was read.
    pub fn version(&self) -> ArchiveVersion {
        match self {
            Self::Cpz1(_) => ArchiveVersion::Cpz1,
            Self::Cpz2(_) => ArchiveVersion::Cpz2,
            Self::Cpz3(_) => ArchiveVersion::Cpz3,
            Self::Cpz5(_) => ArchiveVersion::Cpz5,
            Self::Cpz6(_) => ArchiveVersion::Cpz6,
            Self::Cpz7(_) => ArchiveVersion::Cpz7,
        }
    }

    /// Ordered directory records, including empty or repeated directories.
    pub fn directories(&self) -> &[(std::sync::Arc<[u8]>, u32)] {
        match self {
            Self::Cpz5(i) => &i.directories,
            Self::Cpz6(i) => &i.directories,
            Self::Cpz7(i) => &i.directories,
            _ => &[],
        }
    }

    pub fn directory_format(&self) -> Option<DirectoryFormat> {
        match self {
            Self::Cpz5(i) => Some(DirectoryFormat::Cpz5 {
                mode: i.mode,
                encrypted: i.layout.encryption_mode == 1,
            }),
            Self::Cpz6(_) => Some(DirectoryFormat::Cpz6),
            Self::Cpz7(i) => Some(DirectoryFormat::Cpz7 {
                parameters: i.parameters,
                encrypted: i.layout.encryption_mode == 1,
            }),
            _ => None,
        }
    }

    /// Common archive metadata. Version-specific cipher keys remain private.
    pub fn entries(&self) -> &[Entry] {
        match self {
            Self::Cpz1(index) => index.entries(),
            Self::Cpz2(index) => index.entries(),
            Self::Cpz3(index) => index.entries(),
            Self::Cpz5(index) => index.entries(),
            Self::Cpz6(index) => index.entries(),
            Self::Cpz7(index) => index.entries(),
        }
    }

    /// Look up an entry, treating a missing name as an error.
    ///
    /// Use [`Self::lookup`] when absence is expected.
    pub fn find(&self, name: &[u8]) -> Result<usize, FormatError> {
        match self {
            Self::Cpz1(index) => index.find(name),
            Self::Cpz2(index) => index.find(name),
            Self::Cpz3(index) => index.find(name),
            Self::Cpz5(index) => index.find(name),
            Self::Cpz6(index) => index.find(name),
            Self::Cpz7(index) => index.find(name),
        }
    }

    /// Return the matching entry index, or `None` when the name is absent.
    ///
    /// Invalid lookup names return an error according to the archive's name rules.
    pub fn lookup(&self, name: &[u8]) -> Result<Option<usize>, FormatError> {
        match self {
            Self::Cpz1(index) => index.lookup(name),
            Self::Cpz2(index) => index.lookup(name),
            Self::Cpz3(index) => index.lookup(name),
            Self::Cpz5(index) => index.lookup(name),
            Self::Cpz6(index) => index.lookup(name),
            Self::Cpz7(index) => index.lookup(name),
        }
    }

    /// Read and decrypt an entry without checking its payload checksum.
    ///
    /// `reader` must contain the archive used to construct this index. Prefer
    /// [`Self::read_entry_verified`] unless unchecked extraction is intentional.
    pub fn read_entry<R: Read + Seek>(
        &self,
        reader: &mut R,
        entry: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        match self {
            Self::Cpz1(index) => index.read_entry(reader, entry),
            Self::Cpz2(index) => index.read_entry(reader, entry),
            Self::Cpz3(index) => index.read_entry(reader, entry),
            Self::Cpz5(index) => index.read_entry(reader, entry),
            Self::Cpz6(index) => index.read_entry(reader, entry),
            Self::Cpz7(index) => index.read_entry(reader, entry),
        }
    }

    /// Read plaintext and apply the selected format's payload verification.
    ///
    /// `entry` is an index into [`Self::entries`]; `reader` must contain the same
    /// archive. Invalid indices, checksums, payloads, and I/O failures return errors.
    /// CPZ1 has no payload checksum; for that version this is equivalent to [`Self::read_entry`].
    pub fn read_entry_verified<R: Read + Seek>(
        &self,
        reader: &mut R,
        entry: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        match self {
            // CPZ1 stores no checksum to verify against.
            Self::Cpz1(index) => index.read_entry(reader, entry),
            Self::Cpz2(index) => index.read_entry_verified(reader, entry),
            Self::Cpz3(index) => index.read_entry_verified(reader, entry),
            Self::Cpz5(index) => index.read_entry_verified(reader, entry),
            Self::Cpz6(index) => index.read_entry_verified(reader, entry),
            Self::Cpz7(index) => index.read_entry_verified(reader, entry),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn dummy_entry(name: &[u8]) -> Entry {
        Entry {
            directory: [].into(),
            name: name.to_vec(),
            size: 0,
            offset: 0,
            checksum_raw: 0,
        }
    }

    #[test]
    fn lookup_table_preserves_case_folding_shift_jis_pairs_and_first_match_order() {
        let long_mixed = [b"Long_Resource_Name_".as_slice(), &[b'A'; 60], b".pb3"].concat();
        let long_upper = [b"LONG_RESOURCE_NAME_".as_slice(), &[b'a'; 60], b".PB3"].concat();
        let entries = vec![
            dummy_entry(b"Start.ps3"),
            dummy_entry(b"START.PS3"),
            dummy_entry(b"start.ps3"),
            // 0x83 0x45: trail byte 0x45 is ASCII 'E' by value, but must NOT fold to 0x65.
            dummy_entry(b"\x83\x45Bg01.pb3"),
            dummy_entry(b"\x83\x65Bg01.pb3"),
            dummy_entry(b"unique.bin"),
            dummy_entry(b"dup.bin"),
            dummy_entry(b"dup.bin"),
            dummy_entry(&long_mixed),
        ];
        let table = LookupTable::from_entries(&entries, fold_vec);

        // Case-insensitive lookup returns the FIRST matching index in archive order.
        assert_eq!(table.lookup(b"start.ps3"), Some(0));
        assert_eq!(table.lookup(b"START.PS3"), Some(0));
        assert_eq!(table.lookup(b"StArT.pS3"), Some(0));

        // Shift-JIS trail bytes stay distinct while trailing ASCII still folds.
        assert_eq!(table.lookup(b"\x83\x45bg01.PB3"), Some(3));
        assert_eq!(table.lookup(b"\x83\x65BG01.pb3"), Some(4));

        // Names longer than the 64-byte stack fast-path fold identically.
        assert_eq!(table.lookup(&long_upper), Some(8));
        assert_eq!(table.lookup(b"missing.ps3"), None);

        // Exact `find` distinguishes case and flags exact duplicates as ambiguous.
        assert_eq!(table.find(b"Start.ps3", "missing", "ambiguous"), Ok(0));
        assert_eq!(table.find(b"START.PS3", "missing", "ambiguous"), Ok(1));
        assert_eq!(table.find(b"start.ps3", "missing", "ambiguous"), Ok(2));
        assert_eq!(table.find(b"unique.bin", "missing", "ambiguous"), Ok(5));
        assert_eq!(
            table.find(b"dup.bin", "missing", "ambiguous"),
            Err(FormatError("ambiguous"))
        );
        assert_eq!(
            table.find(b"absent.bin", "missing", "ambiguous"),
            Err(FormatError("missing"))
        );

        // End-to-end verification across CPZ3, CPZ5, and CPZ6 archive indices with duplicates.
        let mut cpz3_buf = Cursor::new(Vec::new());
        Cpz3Index::write(
            &mut cpz3_buf,
            &[
                (b"FirstOnly.ps3".to_vec(), b"one".to_vec()),
                (b"firstonly.ps3".to_vec(), b"two".to_vec()),
                (b"ExactDup.bin".to_vec(), b"a".to_vec()),
                (b"ExactDup.bin".to_vec(), b"b".to_vec()),
            ],
        )
        .unwrap();
        let cpz3 = ArchiveIndex::read(&mut Cursor::new(cpz3_buf.into_inner())).unwrap();
        assert_eq!(cpz3.lookup(b"FIRSTONLY.PS3").unwrap(), Some(1));
        assert_eq!(cpz3.find(b"firstonly.ps3").unwrap(), 1);
        assert!(cpz3.find(b"ExactDup.bin").is_err());
        assert_eq!(cpz3.lookup(b"ExactDup.bin").unwrap(), None);

        let dirs = [
            ArchiveDirectory {
                name: b"dir_a".to_vec(),
                files: vec![ArchiveFile {
                    name: b"Scene01.ps3".to_vec(),
                    data: b"dir_a_payload".to_vec(),
                }],
            },
            ArchiveDirectory {
                name: b"dir_b".to_vec(),
                files: vec![ArchiveFile {
                    name: b"scene01.ps3".to_vec(),
                    data: b"dir_b_payload".to_vec(),
                }],
            },
        ];
        for cpz6 in [false, true] {
            let mut out = Cursor::new(Vec::new());
            if cpz6 {
                Cpz6Index::write(&mut out, &dirs).unwrap();
            } else {
                Cpz5Index::write(&mut out, Cpz5Mode::Mode9, &dirs).unwrap();
            }
            let bytes = out.into_inner();
            let index = ArchiveIndex::read(&mut Cursor::new(&bytes)).unwrap();
            assert_eq!(index.lookup(b"scene01.ps3").unwrap(), Some(1));
            assert_eq!(index.lookup(b"SCENE01.PS3").unwrap(), Some(1));
            assert_eq!(index.find(b"Scene01.ps3").unwrap(), 0);
            assert_eq!(index.find(b"scene01.ps3").unwrap(), 1);
        }
    }
}
