//! 0x228c / 0x140068dd0 and 0x228e / 0x1400690b0, restricted to the locked user directory.
use super::Startup;
use crate::{evaluate::error, vm::SystemResult};
use std::{error::Error, fs, io, path::PathBuf};

enum Operation {
    Remove(String),
    Rename(String, String),
}

/// Keep the successful prefix across an I/O failure. Retrying a partially completed
/// swap must resume its next rename rather than swap the already moved files again.
pub(super) struct SlotEdit {
    identity: (usize, u16, u32, u32),
    directory: PathBuf,
    operations: Vec<Operation>,
    next: usize,
    refresh: bool,
}

impl SlotEdit {
    fn prepare(directory: PathBuf, identity: (usize, u16, u32, u32)) -> io::Result<Self> {
        let (_, command, first, second) = identity;
        let a = format!("save{first:03}.dat");
        let b = format!("save{second:03}.dat");
        let a_exists = directory.join(&a).try_exists()?;
        let b_exists = directory.join(&b).try_exists()?;
        let scratch = if command == 0x28c {
            "savetemp.dat"
        } else {
            "savedel.dat"
        }
        .to_owned();
        let mut operations = vec![Operation::Remove(scratch.clone())];
        if command == 0x28c {
            if first != second {
                match (a_exists, b_exists) {
                    (true, true) => operations.extend([
                        Operation::Rename(a.clone(), scratch.clone()),
                        Operation::Rename(b.clone(), a),
                        Operation::Rename(scratch, b),
                    ]),
                    (true, false) => operations.push(Operation::Rename(a, b)),
                    (false, true) => operations.push(Operation::Rename(b, a)),
                    _ => {}
                }
            }
        } else if b_exists {
            if a_exists {
                operations.push(Operation::Rename(a.clone(), scratch));
            }
            // With identical slots the original's second rename fails after the
            // target was moved to savedel.dat: the slot is now absent, backup retained.
            if first != second {
                operations.push(Operation::Rename(b, a));
            }
        }
        Ok(Self {
            identity,
            directory,
            operations,
            next: 0,
            refresh: first < 10 && second < 10 && (command == 0x28c || b_exists),
        })
    }

    fn run(&mut self) -> io::Result<()> {
        while let Some(operation) = self.operations.get(self.next) {
            match operation {
                Operation::Remove(name) => match fs::remove_file(self.directory.join(name)) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    result => result?,
                },
                Operation::Rename(from, to) => {
                    // The original uses MoveFileA, which rejects an occupied target.
                    // The profile lock excludes competing engine sessions; also reject
                    // unexpected destinations instead of deliberately replacing them.
                    if self.directory.join(to).try_exists()? {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "slot rename target already exists",
                        ));
                    }
                    fs::rename(self.directory.join(from), self.directory.join(to))?
                }
            }
            self.next += 1;
        }
        Ok(())
    }
}

impl Startup<'_> {
    pub(super) fn try_edit_slots(&mut self) -> Result<bool, Box<dyn Error>> {
        let Some((pc, command @ (0x28c | 0x28e))) = self.machine.pending_system() else {
            return Ok(false);
        };
        let arg = |distance| {
            self.machine
                .stack()
                .peek_u32(distance)
                .map_err(|e| error(pc, command + 0x2000, e.0))
        };
        let (first, second) = (arg(4)?, arg(8)?);
        if first <= 999 && second <= 999 {
            let identity = (pc, command, first, second);
            if self.slot_edit.is_none() {
                self.slot_edit = Some(SlotEdit::prepare(self.slot_directory()?, identity)?);
            }
            let edit = self.slot_edit.as_mut().expect("prepared");
            if edit.identity != identity {
                return Err("another slot edit remains unfinished".into());
            }
            edit.run()?;
            if edit.refresh {
                self.save_menu.refresh_slots(&mut self.saves)?;
            }
        }
        self.machine.complete_system(SystemResult(0x4008))?;
        self.slot_edit = None;
        Ok(true)
    }
}
