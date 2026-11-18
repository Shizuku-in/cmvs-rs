//! CVNS (the 2004-2009 predecessor engine) runtime glue. CVNS system commands are numbered
//! differently from CMVS; each reviewed one is translated to the runtime command whose
//! semantics it shares, so the gate, dispatch and runner keep working in CMVS numbering.

/// First runtime command for CVNS-only semantics: `CVNS_ONLY + n`. CMVS switches stop at
/// 992 cases (0x3e0), and VM system opcodes reach 0x27ff, so CMVS profiles never have these.
pub const CVNS_ONLY: u16 = 0x400;

/// Highest runtime command: VM system opcodes are 0x2000..=0x27ff.
pub const LAST_COMMAND: u16 = 0x7ff;

/// CVNS-only runtime command numbers (`CVNS_ONLY + source`) referenced by shared
/// subsystems outside the dispatch registry table.
pub mod cmd {
    pub const FRAME_INTERVAL: u16 = 0x402;
    pub const SAVE_DIRECTORY: u16 = 0x426;
    pub const CAPTURE_SCREEN: u16 = 0x45a;
    pub const PERSISTENT_BIT_START: u16 = 0x50b;
    pub const PERSISTENT_INT_START: u16 = 0x50c;
    pub const PERSISTENT_STR_START: u16 = 0x50d;
    pub const TEXT_MESSAGE_LOG: u16 = 0x512;
    pub const TEXT_MESSAGE_WAIT: u16 = 0x520;
    pub const HISTORY_APPEND_2: u16 = 0x52c;
    pub const HISTORY_QUERY_2: u16 = 0x52d;
    pub const HISTORY_QUERY: u16 = 0x536;
    pub const HISTORY_APPEND: u16 = 0x537;
    pub const MASK_CAPTURE: u16 = 0x589;
    pub const MENU_SCAN: u16 = 0x5a1;
    pub const SLOT_INFO: u16 = 0x5ac;
    pub const MOVIE_PLAY: u16 = 0x4c0;
    pub const SYSTEM_SAVE_READ: u16 = 0x5c0;
    pub const SYSTEM_SAVE_WRITE: u16 = 0x5c1;
    pub const GET_CONFIG: u16 = 0x5ec;
    pub const SET_CONFIG: u16 = 0x5ed;
}

/// Source command (opcode minus 0x2000) to runtime command. Unmapped commands keep their
/// number; the profile gate stops them before any handler can run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandMap {
    targets: Vec<Option<u16>>,
}

impl CommandMap {
    pub fn new(pairs: impl IntoIterator<Item = (u16, u16)>) -> Self {
        let mut targets = Vec::new();
        for (source, target) in pairs {
            let at = usize::from(source);
            if targets.len() <= at {
                targets.resize(at + 1, None);
            }
            targets[at] = Some(target);
        }
        Self { targets }
    }
    /// The runtime command for `source`, or `None` when it has no reviewed translation.
    pub fn get(&self, source: u16) -> Option<u16> {
        self.targets.get(usize::from(source)).copied().flatten()
    }
    pub fn runtime(&self, source: u16) -> u16 {
        self.get(source).unwrap_or(source)
    }
    pub fn len(&self) -> usize {
        self.targets.iter().flatten().count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmapped_commands_keep_their_number() {
        let map = CommandMap::new([(0x20, 0x10), (0x108, 0x401)]);
        assert_eq!(map.runtime(0x20), 0x10);
        assert_eq!(map.runtime(0x108), 0x401);
        assert_eq!(map.get(0x21), None);
        assert_eq!(map.runtime(0x21), 0x21);
        assert_eq!(map.runtime(0x7ff), 0x7ff);
        assert_eq!(map.len(), 2);
        assert!(CommandMap::new([]).is_empty());
    }
}
