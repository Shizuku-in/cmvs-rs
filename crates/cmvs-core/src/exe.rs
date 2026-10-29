//! Read-only PE inspection that binds research evidence to a CMVS executable.
//! Only the headers and the system-command switch are located; nothing is executed or patched.
use crate::FormatError;

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

const SCN_MEM_EXECUTE: u32 = 0x2000_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    virtual_address: u32,
    raw_offset: u32,
    raw_size: u32,
    characteristics: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeImage {
    pub arch: Arch,
    pub timestamp: u32,
    pub linker: (u8, u8),
    pub image_base: u64,
    sections: Vec<Section>,
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, FormatError> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or(FormatError("PE header truncated"))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32, FormatError> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().expect("four bytes")))
        .ok_or(FormatError("PE header truncated"))
}

impl PeImage {
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        if bytes.get(..2) != Some(b"MZ") {
            return Err(FormatError("expected MZ executable"));
        }
        let pe = u32_at(bytes, 0x3c)? as usize;
        if bytes.get(pe..pe.saturating_add(4)) != Some(b"PE\0\0") {
            return Err(FormatError("expected PE signature"));
        }
        let arch = match u16_at(bytes, pe + 4)? {
            0x14c => Arch::X86,
            0x8664 => Arch::X64,
            _ => return Err(FormatError("unsupported PE machine")),
        };
        let count = u16_at(bytes, pe + 6)? as usize;
        let timestamp = u32_at(bytes, pe + 8)?;
        let optional = u16_at(bytes, pe + 20)? as usize;
        let opt = pe + 24;
        let image_base = match (u16_at(bytes, opt)?, arch) {
            (0x10b, Arch::X86) => u32_at(bytes, opt + 28)? as u64,
            (0x20b, Arch::X64) => {
                u32_at(bytes, opt + 24)? as u64 | (u32_at(bytes, opt + 28)? as u64) << 32
            }
            _ => return Err(FormatError("PE optional header does not match machine")),
        };
        let linker = (
            *bytes
                .get(opt + 2)
                .ok_or(FormatError("PE header truncated"))?,
            *bytes
                .get(opt + 3)
                .ok_or(FormatError("PE header truncated"))?,
        );
        let table = opt + optional;
        let sections = (0..count)
            .map(|i| {
                let at = table + 40 * i;
                Ok(Section {
                    virtual_address: u32_at(bytes, at + 12)?,
                    raw_size: u32_at(bytes, at + 16)?,
                    raw_offset: u32_at(bytes, at + 20)?,
                    characteristics: u32_at(bytes, at + 36)?,
                })
            })
            .collect::<Result<_, FormatError>>()?;
        Ok(Self {
            arch,
            timestamp,
            linker,
            image_base,
            sections,
        })
    }
    /// File offset of `len` bytes at `rva`, only when they lie inside one section's raw data.
    pub fn offset(&self, rva: u32, len: usize) -> Option<usize> {
        self.sections.iter().find_map(|s| {
            let delta = rva.checked_sub(s.virtual_address)?;
            let end = (delta as usize).checked_add(len)?;
            (end <= s.raw_size as usize).then(|| s.raw_offset as usize + delta as usize)
        })
    }
    fn rva_of(&self, offset: usize) -> Option<u32> {
        self.sections.iter().find_map(|s| {
            let delta = offset.checked_sub(s.raw_offset as usize)?;
            (delta < s.raw_size as usize).then(|| s.virtual_address + delta as u32)
        })
    }
}

/// Where one switch case leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    /// The `ja` target, which returns 0; also reached by every out-of-range command.
    Default,
    /// The shared stub that returns 0x4000 without any effect or parameter recycling.
    NoOp(u32),
    /// Any other entry RVA.
    Handler(u32),
}

/// The system-command switch: case `n` handles VM opcode `0x2000 + n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    /// RVA of the `cmp eax, imm32` bounding the switch.
    pub compare_rva: u32,
    pub table_rva: u32,
    pub default_rva: u32,
    pub cases: Vec<Case>,
}

impl Dispatch {
    pub fn count(&self, kind: fn(&Case) -> bool) -> usize {
        self.cases.iter().filter(|c| kind(c)).count()
    }
    pub fn handlers(&self) -> usize {
        self.count(|c| matches!(c, Case::Handler(_)))
    }
    pub fn noops(&self) -> usize {
        self.count(|c| matches!(c, Case::NoOp(_)))
    }
}

/// `mov eax, 0x4000` plus the dispatcher epilogue, as compiled in each surveyed era.
const X86_NOOPS: [&[u8]; 4] = [
    // 2007: mov ax, 0x4000; ret 4
    &[0x66, 0xb8, 0x00, 0x40, 0xc2, 0x04, 0x00],
    // 2008-2012: ret 4
    &[0xb8, 0x00, 0x40, 0x00, 0x00, 0xc2, 0x04, 0x00],
    // 2013-2017: pop ebx; pop ebp; ret 4
    &[0xb8, 0x00, 0x40, 0x00, 0x00, 0x5b, 0x5d, 0xc2, 0x04, 0x00],
    // 2019-2023: pop esi; pop ebp; ret 4
    &[0xb8, 0x00, 0x40, 0x00, 0x00, 0x5e, 0x5d, 0xc2, 0x04, 0x00],
];
// add rsp, 0x20; pop rbx; ret
const X64_NOOP: &[u8] = &[
    0xb8, 0x00, 0x40, 0x00, 0x00, 0x48, 0x83, 0xc4, 0x20, 0x5b, 0xc3,
];

/// Byte prefixes observed before `cmp eax, imm32` in the surveyed CMVS builds (2007-2023).
const X86_PREFIXES: [&[u8]; 3] = [
    // movzx eax, word [esp+4]
    &[0x0f, 0xb7, 0x44, 0x24, 0x04],
    // push ebp; mov ebp, esp; movzx eax, word [ebp+8]; push ebx
    &[0x55, 0x8b, 0xec, 0x0f, 0xb7, 0x45, 0x08, 0x53],
    // push ebp; mov ebp, esp; movzx eax, word [ebp+8]; push esi; mov esi, ecx
    &[0x55, 0x8b, 0xec, 0x0f, 0xb7, 0x45, 0x08, 0x56, 0x8b, 0xf1],
];
// movzx eax, dx; mov rbx, rcx
const X64_PREFIX: &[u8] = &[0x0f, 0xb7, 0xc2, 0x48, 0x8b, 0xd9];

struct Candidate {
    compare: usize,
    limit: u32,
    default: usize,
    table_rva: u32,
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn match_at(image: &PeImage, bytes: &[u8], at: usize) -> Option<Candidate> {
    let prefixes: &[&[u8]] = match image.arch {
        Arch::X86 => &X86_PREFIXES,
        Arch::X64 => &[X64_PREFIX],
    };
    let compare = prefixes
        .iter()
        .find(|p| bytes.get(at..at + p.len()) == Some(p))
        .map(|p| at + p.len())?;
    // cmp eax, imm32; ja rel8
    if bytes.get(compare) != Some(&0x3d) || bytes.get(compare + 5) != Some(&0x77) {
        return None;
    }
    let limit = le32(bytes, compare + 1)?;
    let after_ja = compare + 7;
    let default = after_ja.checked_add_signed(*bytes.get(compare + 6)? as i8 as isize)?;
    let table_rva = match image.arch {
        Arch::X86 => {
            // jmp dword [eax*4 + table]
            if bytes.get(after_ja..after_ja + 3) != Some(&[0xff, 0x24, 0x85]) {
                return None;
            }
            let va = le32(bytes, after_ja + 3)? as u64;
            u32::try_from(va.checked_sub(image.image_base)?).ok()?
        }
        Arch::X64 => {
            // lea rcx, [rip + image base]; mov edx, [rcx + rax*4 + table]; add rdx, rcx; jmp rdx
            if bytes.get(after_ja..after_ja + 3) != Some(&[0x48, 0x8d, 0x0d])
                || bytes.get(after_ja + 7..after_ja + 10) != Some(&[0x8b, 0x94, 0x81])
                || bytes.get(after_ja + 14..after_ja + 19) != Some(&[0x48, 0x03, 0xd1, 0xff, 0xe2])
            {
                return None;
            }
            let next = image.rva_of(after_ja + 7)? as i64;
            if next + le32(bytes, after_ja + 3)? as i32 as i64 != 0 {
                return None;
            }
            le32(bytes, after_ja + 10)?
        }
    };
    Some(Candidate {
        compare,
        limit,
        default,
        table_rva,
    })
}

/// Locates the unique system-command switch; any other shape is an explicit error.
pub fn find_dispatch(image: &PeImage, bytes: &[u8]) -> Result<Dispatch, FormatError> {
    let mut found = None;
    for s in image
        .sections
        .iter()
        .filter(|s| s.characteristics & SCN_MEM_EXECUTE != 0)
    {
        let start = s.raw_offset as usize;
        let end = start
            .checked_add(s.raw_size as usize)
            .filter(|&end| end <= bytes.len())
            .ok_or(FormatError("PE section outside file"))?;
        for at in start..end {
            if let Some(candidate) = match_at(image, bytes, at) {
                if found.is_some() {
                    return Err(FormatError("several system-command switch candidates"));
                }
                found = Some(candidate);
            }
        }
    }
    let c = found.ok_or(FormatError("system-command switch not found"))?;
    // VM system opcodes are 0x2000..=0x27ff, so a switch beyond 0x800 cases is not this one.
    if c.limit >= 0x800 {
        return Err(FormatError(
            "system-command switch exceeds the opcode range",
        ));
    }
    let compare_rva = image.rva_of(c.compare).expect("matched inside a section");
    let default_rva = image
        .rva_of(c.default)
        .ok_or(FormatError("switch default outside sections"))?;
    let count = c.limit as usize + 1;
    let table = image
        .offset(c.table_rva, count * 4)
        .ok_or(FormatError("switch table outside sections"))?;
    let cases = (0..count)
        .map(|i| {
            let raw = le32(bytes, table + 4 * i).expect("table bounds checked");
            let rva = match image.arch {
                Arch::X86 => u32::try_from((raw as u64).wrapping_sub(image.image_base))
                    .map_err(|_| FormatError("switch entry below image base"))?,
                Arch::X64 => raw,
            };
            if rva == default_rva {
                return Ok(Case::Default);
            }
            let stubs: &[&[u8]] = match image.arch {
                Arch::X86 => &X86_NOOPS,
                Arch::X64 => &[X64_NOOP],
            };
            let noop = stubs.iter().any(|stub| {
                image
                    .offset(rva, stub.len())
                    .is_some_and(|at| bytes.get(at..at + stub.len()) == Some(stub))
            });
            Ok(if noop {
                Case::NoOp(rva)
            } else {
                Case::Handler(rva)
            })
        })
        .collect::<Result<_, FormatError>>()?;
    Ok(Dispatch {
        compare_rva,
        table_rva: c.table_rva,
        default_rva,
        cases,
    })
}
