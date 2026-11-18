//! Read-only gameplay-save header scanning, signature queries and menu state.
//! Header recognition does not validate or restore the saved engine snapshot.
use crate::filesystem::{File, HostFs, resolve_under_with_fs};
use crate::{FormatError, bytecode::DecodeError, dispatch::Call};
use std::{
    error::Error,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

pub use crate::save::{HEADER_READ_LIMIT, SaveFamily, SaveHeader};

/// Slots 0..999; a source returns at most the first 512 bytes, None for absent files.
/// I/O failures are distinct from missing or malformed saves and stop the command.
pub trait SaveSource {
    fn read_header(&mut self, slot: u16) -> io::Result<Option<Vec<u8>>>;
}

/// A read-only slot directory. A missing directory represents an empty set of saves.
pub struct DirectorySaves {
    fs: Arc<dyn HostFs>,
    directory: SlotDirectory,
}

enum SlotDirectory {
    Fixed(PathBuf),
    Registered { game_root: PathBuf, prefix: Vec<u8> },
}

impl DirectorySaves {
    pub fn with_fs(root: impl AsRef<Path>, fs: Arc<dyn HostFs>) -> io::Result<Self> {
        let directory = match fs.canonicalize(root.as_ref()) {
            Ok(root) if fs.is_dir(&root) => root,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "save root is not a directory",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => fs.absolute(root.as_ref())?,
            Err(e) => return Err(e),
        };
        Ok(Self {
            fs,
            directory: SlotDirectory::Fixed(directory),
        })
    }

    pub(crate) fn set_registered_directory(&mut self, game_root: &Path, directory: &[u8]) {
        let mut prefix = directory.to_vec();
        if !prefix.is_empty() && !prefix.ends_with(b"/") && !prefix.ends_with(b"\\") {
            prefix.push(b'/');
        }
        self.directory = SlotDirectory::Registered {
            game_root: game_root.to_owned(),
            prefix,
        };
    }

    #[cfg(feature = "std-fs")]
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        Self::with_fs(root, crate::filesystem::default_fs())
    }
}

impl DirectorySaves {
    fn open_slot(&self, slot: u16) -> io::Result<Option<File>> {
        let filename = format!("save{slot:03}.dat");
        let path = match &self.directory {
            SlotDirectory::Fixed(root) => root.join(filename),
            SlotDirectory::Registered { game_root, prefix } => {
                let mut name = prefix.clone();
                name.extend_from_slice(filename.as_bytes());
                match resolve_under_with_fs(&*self.fs, game_root, &name) {
                    Ok(path) => path,
                    Err(e) if e.is_not_found() => return Ok(None),
                    Err(e) => return Err(io::Error::other(e)),
                }
            }
        };
        match self.fs.open_read(&path) {
            Ok(file) => Ok(Some(file)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A whole slot file for loading (0x1400087c0), bounded by the research payload limit.
    pub(crate) fn read_slot(&self, slot: u16) -> io::Result<Option<Vec<u8>>> {
        let Some(file) = self.open_slot(slot)? else {
            return Ok(None);
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
        let Some(file) = self.open_slot(slot)? else {
            return Ok(None);
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
    pub fn scan(source: &mut (impl SaveSource + ?Sized), requested: i32) -> io::Result<Self> {
        let count = if requested < 0 {
            10
        } else {
            requested.min(256)
        };
        Self::scan_family(source, count as u16, SaveFamily::Cmvs)
    }
    /// CVNS 0x21a1 (0x467a20): the count is unsigned and clamped to 256.
    pub fn scan_cvns(source: &mut (impl SaveSource + ?Sized), requested: u32) -> io::Result<Self> {
        Self::scan_family(source, requested.min(256) as u16, SaveFamily::Cvns)
    }
    fn scan_family(
        source: &mut (impl SaveSource + ?Sized),
        count: u16,
        family: SaveFamily,
    ) -> io::Result<Self> {
        let mut slots = Vec::with_capacity(count as usize);
        let mut latest = None;
        let mut newest_key = 0;
        for slot in 0..count {
            let status = match source.read_header(family.file(slot))? {
                None => SlotStatus::Missing,
                Some(bytes) => match SaveHeader::parse_family(&bytes, family) {
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
    family: SaveFamily,
    menu: Option<SaveMenu>,
    /// 0x22b4 enables position 0 unconditionally and sets position 1 to this value.
    second_group_enabled: Option<bool>,
    /// +12092: the main loop may open the popup menu (0x140018850 -> TrackPopupMenu).
    popup_enabled: bool,
    /// +1496 / +1500: DLG_LOAD / DLG_SAVE, whether the scripts ask before loading / saving.
    confirm: [bool; 2],
    /// CVNS +6173 (`0x206b`): transient UI/menu cancel mark.
    cvns_menu_triggered: bool,
}

impl Default for SaveMenuHost {
    fn default() -> Self {
        Self {
            family: SaveFamily::Cmvs,
            menu: None,
            second_group_enabled: None,
            // Constructor 0x140016e20, reset 0x140080700 and load 0x140081960 all set 1.
            popup_enabled: true,
            confirm: [false; 2],
            cvns_menu_triggered: false,
        }
    }
}

impl SaveMenuHost {
    pub fn for_profile(profile: &crate::profile::EngineProfile) -> Self {
        Self {
            family: profile.save_family(),
            ..Self::default()
        }
    }

    /// Slot exchange/move refreshes the first-ten native menu labels. Preserve the
    /// previously requested scan extent in the portable menu model.
    pub(crate) fn refresh_slots(&mut self, source: &mut impl SaveSource) -> io::Result<()> {
        if let Some(menu) = &self.menu {
            self.menu = Some(SaveMenu::scan_family(
                source,
                menu.slots.len() as u16,
                self.family,
            )?);
        }
        Ok(())
    }
    /// RESET writes only +12092; the menu and confirmation settings survive.
    pub(crate) fn reset_popup(&mut self) {
        self.popup_enabled = true;
        self.cvns_menu_triggered = false;
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

    pub(crate) fn handle_popup(&mut self, call: &mut Call<'_>) -> Result<(), DecodeError> {
        let command = call.command;
        match command {
            0x2d3 | 0x2d4 => {
                let index = usize::from(command == 0x2d4);
                let mode = call.argument(4, "mode")?;
                let value = if mode != 0 {
                    Some(call.argument(8, "value")? != 0)
                } else {
                    call.ignore_argument(8, "unused value")?;
                    None
                };
                call.complete(0x4008)?;
                match value {
                    Some(value) => self.confirm[index] = value,
                    None => call
                        .vm
                        .state_mut()
                        .set_system_integer(0, u32::from(self.confirm[index]))
                        .expect("slot 0 exists"),
                }
            }
            0x2cd => {
                let value = call.argument(4, "enabled")?;
                call.complete(0x4004)?;
                self.popup_enabled = value != 0;
            }
            0x2ce | 0x2ee => {
                let value = if command == 0x2ce {
                    self.popup_enabled
                } else {
                    self.second_group_enabled.unwrap_or(false)
                };
                call.complete(0x4000)?;
                call.vm
                    .state_mut()
                    .set_system_integer(0, u32::from(value))
                    .expect("slot 0 exists");
            }
            0x46b => {
                call.complete(0x4000)?;
                self.cvns_menu_triggered = true;
            }
            _ => return Err(call.error("unregistered popup command")),
        }
        Ok(())
    }
    pub(crate) fn handle(
        &mut self,
        call: &mut Call<'_>,
        source: &mut dyn SaveSource,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let command = call.command;
        let argument = call.argument(
            4,
            if command == 0x2b3 || command == 0x5a1 {
                "slot count"
            } else if command == 0x2b4 {
                "second group enabled"
            } else {
                "save slot"
            },
        )?;
        if command == 0x2bb {
            // 0x14007cb60 reads up to 64 bytes and tests only CSV[2-9]; CVNS 0x21ae
            // (`0x4683c0`) checks `slot != 0` and CSP[1-9]. Unlike the menu scan, both accept
            // a signature without a valid date/label/body.
            let recognized = argument <= 999
                && (self.family != SaveFamily::Cvns || argument != 0)
                && source
                    .read_header(argument as u16)?
                    .is_some_and(|bytes| self.family.accepts(&bytes));
            call.complete(0x4004)?;
            call.vm
                .state_mut()
                .set_system_integer(0, u32::from(recognized))
                .expect("slot 0 exists");
        } else if command == 0x2b3 {
            let menu = SaveMenu::scan(source, argument as i32)?;
            call.complete(0x4004)?;
            self.menu = Some(menu);
        } else if command == crate::cvns::cmd::MENU_SCAN {
            let menu = SaveMenu::scan_cvns(source, argument)?;
            call.complete(0x4004)?;
            self.menu = Some(menu);
        } else {
            call.complete(0x4004)?;
            self.second_group_enabled = Some(argument != 0);
        }
        Ok(())
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
