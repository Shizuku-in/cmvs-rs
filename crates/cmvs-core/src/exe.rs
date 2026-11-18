//! Executable architecture recorded with a build identity.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Arch {
    X86,
    X64,
}

impl Arch {
    pub fn name(self) -> &'static str {
        match self {
            Self::X86 => "x86",
            Self::X64 => "x64",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "x86" => Some(Self::X86),
            "x64" => Some(Self::X64),
            _ => None,
        }
    }
}
