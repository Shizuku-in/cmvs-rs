//! CPZ archive reading. Supports CPZ6 mode 1 and CPZ7 resource modes 0/1.
//!
//! Version-specific implementations are kept behind this public entry point.

mod cpz6;
mod cpz7;
mod crypto;
mod entry;
mod error;

use crate::FormatError;
use std::io::{Read, Seek, SeekFrom};

pub use cpz6::{CPZ6_HEADER_SIZE, Cpz6Index, Cpz6Layout, lookup_key};
pub use cpz7::{CPZ7_HEADER_SIZE, Cpz7Index, Cpz7Layout, Cpz7Parameters};
pub use crypto::data_checksum;
pub use entry::Entry;
pub use error::ArchiveError;

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

fn read_version<R: Read + Seek>(reader: &mut R) -> Result<ArchiveVersion, ArchiveError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut signature = [0; 4];
    reader.read_exact(&mut signature)?;
    ArchiveVersion::detect(&signature)
}

/// Header-only inspection, without decrypting or authenticating the index.
#[derive(Debug)]
pub enum ArchiveLayout {
    Cpz6(Cpz6Layout),
    Cpz7(Cpz7Layout),
}

impl ArchiveLayout {
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, ArchiveError> {
        match read_version(reader)? {
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
            version => Err(ArchiveError::UnsupportedVersion(version)),
        }
    }
}

/// Version-dispatched archive access. Lookup and verification stay version-specific.
#[derive(Debug)]
pub enum ArchiveIndex {
    Cpz6(Cpz6Index),
    Cpz7(Cpz7Index),
}

impl ArchiveIndex {
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, ArchiveError> {
        Self::read_with_cpz7_parameters(reader, Cpz7Parameters::default())
    }

    pub fn read_with_cpz7_parameters<R: Read + Seek>(
        reader: &mut R,
        parameters: Cpz7Parameters,
    ) -> Result<Self, ArchiveError> {
        match read_version(reader)? {
            ArchiveVersion::Cpz6 if parameters != Cpz7Parameters::default() => {
                Err(FormatError("CPZ7 parameters cannot be used with CPZ6").into())
            }
            ArchiveVersion::Cpz6 => Ok(Self::Cpz6(Cpz6Index::read(reader)?)),
            ArchiveVersion::Cpz7 => Ok(Self::Cpz7(Cpz7Index::read_with_parameters(
                reader, parameters,
            )?)),
            version => Err(ArchiveError::UnsupportedVersion(version)),
        }
    }

    pub fn version(&self) -> ArchiveVersion {
        match self {
            Self::Cpz6(_) => ArchiveVersion::Cpz6,
            Self::Cpz7(_) => ArchiveVersion::Cpz7,
        }
    }

    /// Common archive metadata. Version-specific cipher keys remain private.
    pub fn entries(&self) -> &[Entry] {
        match self {
            Self::Cpz6(index) => index.entries(),
            Self::Cpz7(index) => index.entries(),
        }
    }

    pub fn find(&self, name: &[u8]) -> Result<usize, FormatError> {
        match self {
            Self::Cpz6(index) => index.find(name),
            Self::Cpz7(index) => index.find(name),
        }
    }

    pub fn lookup(&self, name: &[u8]) -> Result<Option<usize>, FormatError> {
        match self {
            Self::Cpz6(index) => index.lookup(name),
            Self::Cpz7(index) => index.lookup(name),
        }
    }

    pub fn read_entry<R: Read + Seek>(
        &self,
        reader: &mut R,
        entry: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        match self {
            Self::Cpz6(index) => index.read_entry(reader, entry),
            Self::Cpz7(index) => index.read_entry(reader, entry),
        }
    }

    /// Apply the selected format's verification before returning plaintext.
    pub fn read_entry_verified<R: Read + Seek>(
        &self,
        reader: &mut R,
        entry: usize,
    ) -> Result<Vec<u8>, ArchiveError> {
        match self {
            Self::Cpz6(index) => index.read_entry_verified(reader, entry),
            Self::Cpz7(index) => index.read_entry_verified(reader, entry),
        }
    }
}
