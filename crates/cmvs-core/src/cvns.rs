//! CVNS (the 2004-2009 predecessor engine) runtime glue. CVNS system commands are numbered
//! differently from CMVS; each reviewed one is translated to the runtime command whose
//! semantics it shares, so the gate, dispatch and runner keep working in CMVS numbering.

/// First runtime command for CVNS-only semantics: `CVNS_ONLY + n`. CMVS switches stop at
/// 992 cases (0x3e0), and VM system opcodes reach 0x27ff, so CMVS profiles never have these.
pub const CVNS_ONLY: u16 = 0x400;

/// Highest runtime command: VM system opcodes are 0x2000..=0x27ff.
pub const LAST_COMMAND: u16 = 0x7ff;

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
