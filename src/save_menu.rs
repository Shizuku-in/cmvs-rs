//! Read-only gameplay-save header scanning and platform-neutral menu state.
//! Header recognition does not validate or restore the saved engine snapshot.
use crate::{
    FormatError, bytecode::DecodeError, evaluate::error, machine::Machine, vm::SystemResult,
};
use std::{
    error::Error,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub const HEADER_READ_LIMIT: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveHeader {
    /// ASCII version digit, currently accepted by the original menu: '2'..='9'.
    pub version: u8,
    /// Original bytes at offset 16, excluding the terminating NUL.
    pub label: Vec<u8>,
    /// Original ordering key; only the last two year digits and even seconds count.
    pub ordering_key: u32,
}

impl SaveHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let bytes = &bytes[..bytes.len().min(HEADER_READ_LIMIT)];
        if bytes.len() < 4 || &bytes[..3] != b"CSV" || !(b'2'..=b'9').contains(&bytes[3]) {
            return Err(FormatError("unsupported gameplay-save signature"));
        }
        let label = bytes
            .get(16..)
            .and_then(|b| b.iter().position(|v| *v == 0).map(|end| &b[..end]))
            .ok_or(FormatError("save label is truncated or not NUL terminated"))?;
        // Explicit malformed-input policy: no stale read-buffer bytes or unbounded C strings.
        if label.len() < 19
            || label[4] != b'-'
            || label[7] != b'-'
            || label[10] != b' '
            || label[13] != b':'
            || label[16] != b':'
            || !label[..19]
                .iter()
                .enumerate()
                .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16) || b.is_ascii_digit())
        {
            return Err(FormatError("save label lacks YYYY-MM-DD HH:MM:SS prefix"));
        }
        let pair = |i: usize| u32::from(label[i] - b'0') * 10 + u32::from(label[i + 1] - b'0');
        let ordering_key = pair(2).wrapping_shl(25)
            | ((pair(5) & 15) << 21)
            | ((pair(8) & 31) << 16)
            | ((pair(11) & 31) << 11)
            | ((pair(14) & 63) << 5)
            | ((pair(17) / 2) & 31);
        Ok(Self {
            version: bytes[3],
            label: label.to_vec(),
            ordering_key,
        })
    }
}

/// A source returns at most the first 512 bytes; None means that slot is absent.
/// I/O failures are distinct from missing or malformed saves and stop the command.
pub trait SaveSource {
    fn read_header(&mut self, slot: u16) -> io::Result<Option<Vec<u8>>>;
}

/// An explicitly selected existing directory; no writes or game-relative path inference.
pub struct DirectorySaves {
    root: PathBuf,
}

impl DirectorySaves {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "save root is not a directory",
            ));
        }
        Ok(Self { root })
    }
}

impl SaveSource for DirectorySaves {
    fn read_header(&mut self, slot: u16) -> io::Result<Option<Vec<u8>>> {
        if slot >= 256 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "save slot outside 0..255",
            ));
        }
        let file = match File::open(self.root.join(format!("save{slot:03}.dat"))) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut bytes = Vec::new();
        file.take(HEADER_READ_LIMIT as u64)
            .read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotStatus {
    Missing,
    Invalid(FormatError),
    Recognized(SaveHeader),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveMenu {
    /// Index is the zero-based slot. Recognized rows enable the load menu item.
    pub slots: Vec<SlotStatus>,
    /// Checked row in both menus. Strict greater-than comparison keeps the first tie.
    pub latest: Option<u16>,
}

impl SaveMenu {
    pub fn scan(source: &mut impl SaveSource, requested: i32) -> io::Result<Self> {
        let count = if requested < 0 {
            10
        } else {
            requested.min(256)
        };
        let mut slots = Vec::with_capacity(count as usize);
        let mut latest = None;
        let mut newest_key = 0;
        for slot in 0..count as u16 {
            let status = match source.read_header(slot)? {
                None => SlotStatus::Missing,
                Some(bytes) => match SaveHeader::parse(&bytes) {
                    Ok(header) => {
                        if header.ordering_key > newest_key {
                            newest_key = header.ordering_key;
                            latest = Some(slot);
                        }
                        SlotStatus::Recognized(header)
                    }
                    Err(e) => SlotStatus::Invalid(e),
                },
            };
            slots.push(status);
        }
        Ok(Self { slots, latest })
    }
}

#[derive(Debug)]
pub struct SaveMenuHost {
    menu: Option<SaveMenu>,
    /// 0x22b4 enables position 0 unconditionally and sets position 1 to this value.
    second_group_enabled: Option<bool>,
    /// +12092: the main loop may open the popup menu (0x140018850 -> TrackPopupMenu).
    popup_enabled: bool,
}

impl Default for SaveMenuHost {
    fn default() -> Self {
        Self {
            menu: None,
            second_group_enabled: None,
            // Constructor 0x140016e20, reset 0x140080700 and load 0x140081960 all set 1.
            popup_enabled: true,
        }
    }
}

impl SaveMenuHost {
    pub fn menu(&self) -> Option<&SaveMenu> {
        self.menu.as_ref()
    }
    pub fn second_group_enabled(&self) -> Option<bool> {
        self.second_group_enabled
    }
    pub fn popup_enabled(&self) -> bool {
        self.popup_enabled
    }
    /// 0x22cd stores SP-4 != 0 in +12092; 0x22ce copies +12092 to system result 0.
    /// Neither needs a save directory or creates native UI.
    pub fn try_handle_popup(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        match command {
            0x2cd => {
                let value = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x22cd, e.0))?;
                vm.complete_system(SystemResult(0x4004))?;
                self.popup_enabled = value != 0;
            }
            0x2ce => {
                vm.complete_system(SystemResult(0x4000))?;
                vm.state_mut()
                    .set_system_integer(0, u32::from(self.popup_enabled))
                    .expect("slot 0 exists");
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    /// Returns true only when a verified command completed. No native UI is created.
    pub fn try_handle(
        &mut self,
        vm: &mut Machine<'_>,
        source: &mut impl SaveSource,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        if !matches!(command, 0x2b3 | 0x2b4) {
            return Ok(false);
        }
        let argument = vm
            .stack()
            .peek_u32(4)
            .map_err(|e| error(pc, command + 0x2000, e.0))?;
        if command == 0x2b3 {
            let menu = SaveMenu::scan(source, argument as i32)?;
            vm.complete_system(SystemResult(0x4004))?;
            self.menu = Some(menu);
        } else {
            vm.complete_system(SystemResult(0x4004))?;
            self.second_group_enabled = Some(argument != 0);
        }
        Ok(true)
    }
}
