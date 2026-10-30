//! Read-only gameplay-save header scanning, signature queries and menu state.
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

/// Slots 0..999; a source returns at most the first 512 bytes, None for absent files.
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

impl DirectorySaves {
    /// A whole slot file for loading (0x1400087c0), bounded by the research payload limit.
    pub(crate) fn read_slot(&self, slot: u16) -> io::Result<Option<Vec<u8>>> {
        let file = match File::open(self.root.join(format!("save{slot:03}.dat"))) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut bytes = Vec::new();
        file.take(crate::MAX_PAYLOAD as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > crate::MAX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "save exceeds research limit",
            ));
        }
        Ok(Some(bytes))
    }
}

impl SaveSource for DirectorySaves {
    fn read_header(&mut self, slot: u16) -> io::Result<Option<Vec<u8>>> {
        if slot > 999 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "save slot outside 0..999",
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
    /// +1496 / +1500: DLG_LOAD / DLG_SAVE, whether the scripts ask before loading / saving.
    confirm: [bool; 2],
}

impl Default for SaveMenuHost {
    fn default() -> Self {
        Self {
            menu: None,
            second_group_enabled: None,
            // Constructor 0x140016e20, reset 0x140080700 and load 0x140081960 all set 1.
            popup_enabled: true,
            confirm: [false; 2],
        }
    }
}

impl SaveMenuHost {
    /// Slot exchange/move refreshes the first-ten native menu labels. Preserve the
    /// previously requested scan extent in the portable menu model.
    pub(crate) fn refresh_slots(&mut self, source: &mut impl SaveSource) -> io::Result<()> {
        if let Some(menu) = &self.menu {
            self.menu = Some(SaveMenu::scan(source, menu.slots.len() as i32)?);
        }
        Ok(())
    }
    /// RESET writes only +12092; the menu and confirmation settings survive.
    pub(crate) fn reset_popup(&mut self) {
        self.popup_enabled = true;
    }
    pub fn menu(&self) -> Option<&SaveMenu> {
        self.menu.as_ref()
    }
    pub fn second_group_enabled(&self) -> Option<bool> {
        self.second_group_enabled
    }
    pub fn popup_enabled(&self) -> bool {
        self.popup_enabled
    }
    /// +1496 / +1500 as the startup loader 0x14000a3c0 sets them: the registry values
    /// SI_DlgLoad / SI_DlgSave when present, else `[SYSTEM_INFO]` DLG_LOAD / DLG_SAVE of
    /// cmvs.cfg (GetPrivateProfileIntA, default 0). The host supplies them; the registry is
    /// not read here.
    pub fn set_confirmations(&mut self, load: bool, save: bool) {
        self.confirm = [load, save];
    }
    pub fn confirmations(&self) -> [bool; 2] {
        self.confirm
    }
    /// Records 0x10a (+16584, constructor 0) and 0x202 (+12092).
    pub(crate) fn saved_flags(&self) -> (u32, u32) {
        (
            u32::from(self.second_group_enabled.unwrap_or(false)),
            u32::from(self.popup_enabled),
        )
    }
    /// Loader 0x140081960: +12092 comes from record 0x202, while +16584 is overwritten
    /// afterwards by 0x140092ea0(.., 1), which enables the second menu group.
    pub(crate) fn restore_flags(&mut self, popup: u32) {
        self.popup_enabled = popup != 0;
        self.second_group_enabled = Some(true);
    }
    /// 0x22d3 / 0x14007be00 and 0x22d4 / 0x14007be70: nonzero SP-4 sets +1496 / +1500 to
    /// bool(SP-8) through 0x140090da0 (menu items 32764 / 32765 are checked accordingly);
    /// zero SP-4 copies the flag to system result 0. Both return 0x4008. The original keeps
    /// changed values for its registry; Rust keeps them for the session only.
    /// 0x22cd stores SP-4 != 0 in +12092; 0x22ce copies +12092 to system result 0. 0x22ee
    /// (inline) sets system result 0 to +1644 != 0, the copy of the second menu group flag
    /// 0x140092ea0 keeps (constructor 0). None needs a save directory or creates native UI.
    pub fn try_handle_popup(&mut self, vm: &mut Machine<'_>) -> Result<bool, DecodeError> {
        let Some((pc, command)) = vm.pending_system() else {
            return Ok(false);
        };
        match command {
            0x2d3 | 0x2d4 => {
                let index = usize::from(command == 0x2d4);
                let arg = |distance| {
                    vm.stack()
                        .peek_u32(distance)
                        .map_err(|e| error(pc, command + 0x2000, e.0))
                };
                let value = if arg(4)? != 0 {
                    Some(arg(8)? != 0)
                } else {
                    None
                };
                vm.complete_system(SystemResult(0x4008))?;
                match value {
                    Some(value) => self.confirm[index] = value,
                    None => vm
                        .state_mut()
                        .set_system_integer(0, u32::from(self.confirm[index]))
                        .expect("slot 0 exists"),
                }
            }
            0x2cd => {
                let value = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x22cd, e.0))?;
                vm.complete_system(SystemResult(0x4004))?;
                self.popup_enabled = value != 0;
            }
            0x2ce | 0x2ee => {
                let value = if command == 0x2ce {
                    self.popup_enabled
                } else {
                    self.second_group_enabled.unwrap_or(false)
                };
                vm.complete_system(SystemResult(0x4000))?;
                vm.state_mut()
                    .set_system_integer(0, u32::from(value))
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
        if !matches!(command, 0x2b3 | 0x2b4 | 0x2bb) {
            return Ok(false);
        }
        let argument = vm
            .stack()
            .peek_u32(4)
            .map_err(|e| error(pc, command + 0x2000, e.0))?;
        if command == 0x2bb {
            // 0x14007cb60 reads up to 64 bytes and tests only CSV[2-9]. Unlike the
            // menu scan, this accepts a signature without a valid date/label/body.
            // Out-of-range u32 values (including negative words) perform no I/O.
            // Short reads use only available bytes, never the original stale buffer.
            let recognized = if argument <= 999 {
                source.read_header(argument as u16)?.is_some_and(|bytes| {
                    bytes.len() >= 4 && &bytes[..3] == b"CSV" && (b'2'..=b'9').contains(&bytes[3])
                })
            } else {
                false
            };
            vm.complete_system(SystemResult(0x4004))?;
            vm.state_mut()
                .set_system_integer(0, u32::from(recognized))
                .expect("slot 0 exists");
        } else if command == 0x2b3 {
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

/// GetPrivateProfileStringA over an INI text with a `size`-byte buffer: the trimmed value
/// of `key` in `[section]`, one pair of enclosing double quotes removed, truncated to
/// `size - 1` bytes; `default` when absent.
pub fn profile_string(
    text: &[u8],
    section: &str,
    key: &str,
    default: &[u8],
    size: usize,
) -> Vec<u8> {
    let mut in_section = false;
    let mut value = default;
    for line in text.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line).trim_ascii();
        if let Some(name) = line.strip_prefix(b"[").and_then(|l| l.strip_suffix(b"]")) {
            in_section = name.trim_ascii().eq_ignore_ascii_case(section.as_bytes());
            continue;
        }
        let Some(eq) = line.iter().position(|&b| b == b'=') else {
            continue;
        };
        if in_section && line[..eq].trim_ascii().eq_ignore_ascii_case(key.as_bytes()) {
            let found = line[eq + 1..].trim_ascii();
            value = found
                .strip_prefix(b"\"")
                .and_then(|v| v.strip_suffix(b"\""))
                .unwrap_or(found);
            break;
        }
    }
    value[..value.len().min(size.saturating_sub(1))].to_vec()
}

/// GetPrivateProfileIntA over an INI text: the integer value of `key` in `[section]` (ASCII,
/// case-insensitive names), with optional sign and leading digits; `default` when absent.
pub fn profile_int(text: &[u8], section: &str, key: &str, default: i32) -> i32 {
    let mut in_section = false;
    for line in text.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line).trim_ascii();
        if let Some(name) = line.strip_prefix(b"[").and_then(|l| l.strip_suffix(b"]")) {
            in_section = name.trim_ascii().eq_ignore_ascii_case(section.as_bytes());
            continue;
        }
        let Some(eq) = line.iter().position(|&b| b == b'=') else {
            continue;
        };
        if !in_section || !line[..eq].trim_ascii().eq_ignore_ascii_case(key.as_bytes()) {
            continue;
        }
        let value = line[eq + 1..].trim_ascii();
        let (negative, digits) = match value.first() {
            Some(b'-') => (true, &value[1..]),
            Some(b'+') => (false, &value[1..]),
            _ => (false, value),
        };
        let mut result = 0i32;
        for &b in digits.iter().take_while(|b| b.is_ascii_digit()) {
            result = result.wrapping_mul(10).wrapping_add(i32::from(b - b'0'));
        }
        return if negative {
            result.wrapping_neg()
        } else {
            result
        };
    }
    default
}
