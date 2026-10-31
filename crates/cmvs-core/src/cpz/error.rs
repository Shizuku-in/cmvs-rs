use super::ArchiveVersion;
use crate::FormatError;
use std::{fmt, io};

#[derive(Debug)]
pub enum ArchiveError {
    Io(io::Error),
    Format(FormatError),
    UnknownSignature([u8; 4]),
    UnsupportedVersion(ArchiveVersion),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Format(error) => error.fmt(f),
            Self::UnknownSignature(signature) => {
                write!(f, "unknown CPZ signature: {}", signature.escape_ascii())
            }
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported CPZ archive version: {version:?}")
            }
        }
    }
}

impl std::error::Error for ArchiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Format(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ArchiveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FormatError> for ArchiveError {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
}
