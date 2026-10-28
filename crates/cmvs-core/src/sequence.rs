//! Engine +4736: a timed action sequence (0x614-byte buffer from 0x140014270: a five-word
//! header plus 16 entries of 96 bytes) that the frame update 0x140015900 runs through
//! 0x140014b10; addresses refer to the documented binary hash. Wait entries and their
//! start/stop/finish lifecycle, image replacement and XYZ motion entries are modeled; other actions stay unhandled.
use crate::{
    evaluate::error, image::ImageSource, machine::Machine, scene::SceneHost, vm::SystemResult,
};
use std::error::Error;

pub const SEQUENCE_ENTRIES: usize = 16;

/// Semantics of the supported 96-byte record type. This is not a binary save image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceEntry {
    /// Type 3/subtype 2; fast finish skips this entry without touching the object.
    WorldMotion {
        slot: u32,
        xyz: [u32; 3],
        mode: u32,
        duration: u32,
        pc: usize,
    },
    Wait {
        duration: u32,
    },
    /// Type 3/subtype 0; the owned name is captured at append, not at execution.
    ReplaceImage {
        slot: u32,
        name: Vec<u8>,
        duration: u32,
        pc: usize,
    },
}

/// Header words as 0x140014b10 reads them; the constructor zeroes the whole buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SequenceHeader {
    /// Word 0: the frame update returns at once while it is zero.
    pub active: u32,
    /// Word 1: entry count.
    pub count: u32,
    /// Word 2: next entry; the update stops once it reaches `count`.
    pub index: u32,
    /// Word 3: start time for wait entries.
    pub start: u32,
    /// Word 4: time already consumed by earlier wait entries.
    pub waited: u32,
}

impl SequenceHeader {
    /// 0x1400143d0: any nonzero active word exposes the raw index, otherwise -1.
    /// Does not compare the index to count or modify the sequence.
    pub fn current_index(self) -> u32 {
        if self.active != 0 {
            self.index
        } else {
            u32::MAX
        }
    }
}

#[derive(Debug, Default)]
pub struct SequenceHost {
    header: SequenceHeader,
    entries: [Option<SequenceEntry>; SEQUENCE_ENTRIES],
    /// Explicit wrapping millisecond clock, independent of callback registration ticks.
    clock: u32,
}

impl SequenceHost {
    pub fn with_tick(clock: u32) -> Self {
        Self {
            clock,
            ..Self::default()
        }
    }
    pub fn header(&self) -> SequenceHeader {
        self.header
    }
    /// Includes inactive records retained by 0x2280/0x2284, unlike the active count.
    pub fn entries(&self) -> &[Option<SequenceEntry>; SEQUENCE_ENTRIES] {
        &self.entries
    }
    pub fn clock(&self) -> u32 {
        self.clock
    }

    /// 0x140015900 calls 0x140014b10 before updating scene objects. Waits use absolute
    /// clock - start - already-waited (u32 wrapping), allowing consecutive waits to
    /// consume leftover time within the same update. No 200 ms frame clamp.
    pub fn advance_frame(&mut self, delta: u32) -> Result<(), Box<dyn Error>> {
        self.clock = self.clock.wrapping_add(delta);
        self.run(false, missing_scene)
    }

    pub fn advance_frame_with_images(
        &mut self,
        delta: u32,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
    ) -> Result<(), Box<dyn Error>> {
        self.clock = self.clock.wrapping_add(delta);
        self.run_images(false, scene, images)
    }

    fn run_images(
        &mut self,
        finish: bool,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
    ) -> Result<(), Box<dyn Error>> {
        self.run(finish, |entry, finish| {
            match entry {
                SequenceEntry::ReplaceImage {
                    slot: raw,
                    name,
                    duration,
                    pc,
                } => {
                    let slot = u8::try_from(*raw)
                        .map_err(|_| error(*pc, 0x2288, "replacement slot outside 0..255"))?;
                    if let Some(prepared) = scene.prepare_image_change(
                        slot,
                        name,
                        if finish { 0 } else { *duration },
                        images,
                        *pc,
                        0x2288,
                    )? {
                        scene.commit_image_change(slot, prepared);
                    }
                }
                SequenceEntry::WorldMotion {
                    slot: raw,
                    xyz,
                    mode,
                    duration,
                    pc,
                } => {
                    if finish {
                        return Ok(());
                    }
                    scene.replace_world_motion(*raw, *xyz, *mode, *duration, *pc)?;
                }
                SequenceEntry::Wait { .. } => unreachable!("wait handled by sequence"),
            }
            Ok(())
        })
    }

    fn run(
        &mut self,
        finish: bool,
        mut apply: impl FnMut(&SequenceEntry, bool) -> Result<(), Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        if self.header.active == 0 || self.header.index >= self.header.count {
            return Ok(());
        }
        while self.header.index < self.header.count {
            match self.entries[self.header.index as usize]
                .as_ref()
                .expect("validated entry")
            {
                SequenceEntry::Wait { duration } => {
                    if !finish {
                        let elapsed = self
                            .clock
                            .wrapping_sub(self.header.start)
                            .wrapping_sub(self.header.waited);
                        if elapsed < *duration {
                            return Ok(());
                        }
                        self.header.waited = self.header.waited.wrapping_add(*duration);
                    }
                }
                entry => apply(entry, finish)?,
            }
            // Resource failures retain this entry; earlier successful entries stay committed.
            self.header.index += 1;
        }
        self.header.active = 0;
        Ok(())
    }

    /// Resource-aware finish runs image actions with duration zero and skips subtype 2 motion.
    pub fn try_handle_with_images(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn Error>> {
        if !matches!(vm.pending_system(), Some((_, 0x284))) {
            return self.try_handle(vm);
        }
        self.run_images(true, scene, images)?;
        vm.complete_system(SystemResult(0x4000))?;
        self.header = SequenceHeader::default();
        Ok(true)
    }
    /// 0x2280: the dispatcher calls 0x140014320 inline, zeroing the five header words, and
    /// returns 0x4000 (nothing popped). Entries keep their bytes but are no longer run.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, Box<dyn Error>> {
        if let Some((pc, 0x28a)) = vm.pending_system() {
            let mut args = [0; 6];
            for (i, arg) in args.iter_mut().enumerate() {
                *arg = vm
                    .stack()
                    .peek_u32((i + 1) * 4)
                    .map_err(|e| error(pc, 0x228a, e.0))?;
            }
            vm.complete_system(SystemResult(0x4018))?;
            if self.header.count < SEQUENCE_ENTRIES as u32 {
                self.entries[self.header.count as usize] = Some(SequenceEntry::WorldMotion {
                    slot: args[0],
                    xyz: [args[1], args[2], args[3]],
                    mode: args[4],
                    duration: args[5],
                    pc,
                });
                self.header.count += 1;
            }
            return Ok(true);
        }
        if let Some((pc, 0x288)) = vm.pending_system() {
            let mut args = [0; 3];
            for (i, arg) in args.iter_mut().enumerate() {
                *arg = vm
                    .stack()
                    .peek_u32((i + 1) * 4)
                    .map_err(|e| error(pc, 0x2288, e.0))?;
            }
            let name = vm.resolve_string(args[1])?;
            let entry = if self.header.count < SEQUENCE_ENTRIES as u32 {
                if name.len() >= 64 {
                    return Err(error(pc, 0x2288, "sequence image name exceeds 63 bytes").into());
                }
                Some(SequenceEntry::ReplaceImage {
                    slot: args[0],
                    name,
                    duration: args[2],
                    pc,
                })
            } else {
                None
            };
            vm.complete_system(SystemResult(0x400c))?;
            if let Some(entry) = entry {
                self.entries[self.header.count as usize] = Some(entry);
                self.header.count += 1;
            }
            return Ok(true);
        }
        let Some((pc, command @ 0x280..=0x286)) = vm.pending_system() else {
            return Ok(false);
        };
        if command == 0x286 {
            let duration = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2286, e.0))?;
            vm.complete_system(SystemResult(0x4004))?;
            // 0x1400143e0: ignore an append at capacity but still complete the command.
            if self.header.count < SEQUENCE_ENTRIES as u32 {
                self.entries[self.header.count as usize] = Some(SequenceEntry::Wait { duration });
                self.header.count += 1;
            }
            return Ok(true);
        }
        if command == 0x284 {
            self.run(true, missing_scene)?;
        }
        vm.complete_system(SystemResult(0x4000))?;
        if command == 0x280 {
            self.header = SequenceHeader::default();
        } else if command == 0x282 {
            // 0x140014350: an active or empty sequence is unchanged; otherwise restart.
            if self.header.active == 0 && self.header.count != 0 {
                self.header.start = self.clock;
                self.header.waited = 0;
                self.header.index = 0;
                self.header.active = 1;
            }
        } else if command == 0x283 {
            // 0x1400143a0 leaves count, index, timing and entries untouched.
            if self.header.active != 0 && self.header.count != 0 {
                self.header.active = 0;
            }
        } else if command == 0x284 {
            // 0x14007f860 executes the remaining active entries in fast mode, then resets.
            self.header = SequenceHeader::default();
        } else {
            // 0x14007f8e0 -> 0x1400143c0 reads header word 1, not active/index.
            // 0x14007f830 -> 0x1400143d0 reads word 2 only when active is nonzero.
            let value = if command == 0x281 {
                self.header.current_index()
            } else {
                self.header.count
            };
            vm.state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
        }
        Ok(true)
    }
}

fn missing_scene(entry: &SequenceEntry, finish: bool) -> Result<(), Box<dyn Error>> {
    match entry {
        SequenceEntry::WorldMotion { .. } if finish => Ok(()),
        SequenceEntry::WorldMotion { pc, .. } => {
            Err(error(*pc, 0x228a, "motion sequence needs scene").into())
        }
        SequenceEntry::ReplaceImage { pc, .. } => {
            Err(error(*pc, 0x2288, "image sequence needs scene and image source").into())
        }
        SequenceEntry::Wait { .. } => unreachable!("wait handled by sequence"),
    }
}
