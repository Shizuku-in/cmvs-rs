//! Engine +4736: a timed action sequence (0x614-byte buffer from 0x140014270: a five-word
//! header plus 16 entries of 96 bytes) that the frame update 0x140015900 runs through
//! 0x140014b10; addresses refer to the documented binary hash. Wait entries and their
//! start/stop/finish lifecycle, image replacement and XYZ motion entries are modeled; other actions stay unhandled.
use crate::{
    camera::CameraHost, dispatch::Call, evaluate::error, image::ImageSource, machine::Machine,
    scene::SceneHost,
};
use std::error::Error;

pub const SEQUENCE_ENTRIES: usize = 16;

/// Semantics of the supported 96-byte record type. This is not a binary save image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceEntry {
    /// Type 4, 0x2287: camera 0 XYZ; finish/zero duration writes immediately.
    CameraMotion {
        xyz: [u32; 3],
        mode: u32,
        duration: u32,
        pc: usize,
    },
    /// Type 3/subtype 1 or 2; both start XYZ tracks, differing on fast finish.
    WorldMotion {
        /// Subtype 1 (0x2289) snaps to XYZ on fast finish; subtype 2 skips it.
        snap_on_finish: bool,
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

#[derive(Debug, Clone, Default)]
pub struct SequenceHost {
    header: SequenceHeader,
    entries: [Option<SequenceEntry>; SEQUENCE_ENTRIES],
    /// Explicit wrapping millisecond clock, independent of callback registration ticks.
    clock: u32,
}

/// Optional execution context for finishing scene actions; absence preserves the
/// headless sequence API's explicit action-specific errors.
pub struct SequenceContext<'a> {
    pub scene: &'a mut SceneHost,
    pub images: &'a mut dyn ImageSource,
    pub camera: Option<&'a mut CameraHost>,
}

impl SequenceHost {
    /// 0x140014320 clears only the five header words, retaining entry storage and clock.
    pub(crate) fn reset_header(&mut self) {
        self.header = SequenceHeader::default();
    }
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

    pub fn advance_frame_with_hosts(
        &mut self,
        delta: u32,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
        camera: &mut CameraHost,
    ) -> Result<(), Box<dyn Error>> {
        self.clock = self.clock.wrapping_add(delta);
        self.run_with_camera(false, scene, images, Some(camera))
    }

    fn run_images(
        &mut self,
        finish: bool,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
    ) -> Result<(), Box<dyn Error>> {
        self.run_with_camera(finish, scene, images, None)
    }

    fn run_with_camera(
        &mut self,
        finish: bool,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
        mut camera: Option<&mut CameraHost>,
    ) -> Result<(), Box<dyn Error>> {
        self.run(finish, |entry, finish| {
            match entry {
                SequenceEntry::CameraMotion {
                    xyz,
                    mode,
                    duration,
                    pc,
                } => {
                    let camera = camera
                        .as_deref_mut()
                        .ok_or_else(|| error(*pc, 0x2287, "camera sequence needs a camera host"))?;
                    camera.sequence_move(*xyz, *mode, *duration, finish, *pc)?;
                }
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
                    snap_on_finish,
                    slot: raw,
                    xyz,
                    mode,
                    duration,
                    pc,
                } => {
                    let at = (*pc, if *snap_on_finish { 0x2289 } else { 0x228a });
                    if finish && *snap_on_finish {
                        scene.snap_world_position(*raw, *xyz, at)?;
                    } else if finish {
                        return Ok(());
                    } else {
                        scene.replace_world_motion(*raw, *xyz, *mode, *duration, at)?;
                    }
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

    /// Resource-aware finish runs image actions with duration zero, snaps subtype 1
    /// motion and skips subtype 2 motion.
    pub fn try_handle_with_images(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, command @ 0x280..=0x28a)) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle(
            &mut Call::new(vm, pc, command),
            Some(SequenceContext {
                scene,
                images,
                camera: None,
            }),
        )?;
        Ok(true)
    }

    pub fn try_handle_with_hosts(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        images: &mut dyn ImageSource,
        camera: &mut CameraHost,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, command @ 0x280..=0x28a)) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle(
            &mut Call::new(vm, pc, command),
            Some(SequenceContext {
                scene,
                images,
                camera: Some(camera),
            }),
        )?;
        Ok(true)
    }
    /// 0x2280: the dispatcher calls 0x140014320 inline, zeroing the five header words, and
    /// returns 0x4000 (nothing popped). Entries keep their bytes but are no longer run.
    pub fn try_handle(&mut self, vm: &mut Machine<'_>) -> Result<bool, Box<dyn Error>> {
        let Some((pc, command @ 0x280..=0x28a)) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle(&mut Call::new(vm, pc, command), None)?;
        Ok(true)
    }
    pub(crate) fn handle(
        &mut self,
        call: &mut Call<'_, '_>,
        context: Option<SequenceContext<'_>>,
    ) -> Result<(), Box<dyn Error>> {
        let (pc, command) = (call.pc, call.command);
        if command == 0x287 {
            let mut args = [0; 5];
            for (i, arg) in args.iter_mut().enumerate() {
                *arg = call.argument((i + 1) * 4, argument_names(command)[i])?;
            }
            call.complete(0x4014)?;
            if self.header.count < SEQUENCE_ENTRIES as u32 {
                self.entries[self.header.count as usize] = Some(SequenceEntry::CameraMotion {
                    mode: args[0],
                    xyz: [args[1], args[2], args[3]],
                    duration: args[4],
                    pc,
                });
                self.header.count += 1;
            }
            return Ok(());
        }
        if matches!(command, 0x289 | 0x28a) {
            let mut args = [0; 6];
            for (i, arg) in args.iter_mut().enumerate() {
                *arg = call.argument((i + 1) * 4, argument_names(command)[i])?;
            }
            call.complete(0x4018)?;
            if self.header.count < SEQUENCE_ENTRIES as u32 {
                self.entries[self.header.count as usize] = Some(SequenceEntry::WorldMotion {
                    snap_on_finish: command == 0x289,
                    slot: args[0],
                    xyz: [args[1], args[2], args[3]],
                    mode: args[4],
                    duration: args[5],
                    pc,
                });
                self.header.count += 1;
            }
            return Ok(());
        }
        if command == 0x288 {
            let mut args = [0; 3];
            for (i, arg) in args.iter_mut().enumerate() {
                *arg = call.argument((i + 1) * 4, argument_names(command)[i])?;
            }
            let name = call.vm.resolve_string(args[1])?;
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
            call.complete(0x400c)?;
            if let Some(entry) = entry {
                self.entries[self.header.count as usize] = Some(entry);
                self.header.count += 1;
            }
            return Ok(());
        }
        if command == 0x286 {
            let duration = call.argument(4, "duration")?;
            call.complete(0x4004)?;
            // 0x1400143e0: ignore an append at capacity but still complete the command.
            if self.header.count < SEQUENCE_ENTRIES as u32 {
                self.entries[self.header.count as usize] = Some(SequenceEntry::Wait { duration });
                self.header.count += 1;
            }
            return Ok(());
        }
        if command == 0x284 {
            if let Some(context) = context {
                self.run_with_camera(true, context.scene, context.images, context.camera)?;
            } else {
                self.run(true, missing_scene)?;
            }
        }
        call.complete(0x4000)?;
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
            call.vm
                .state_mut()
                .set_system_integer(0, value)
                .expect("slot 0 exists");
        }
        Ok(())
    }
}

fn argument_names(command: u16) -> &'static [&'static str] {
    match command {
        0x287 => &["mode", "x", "y", "z", "duration"],
        0x288 => &["object slot", "image name", "duration"],
        0x289 | 0x28a => &["object slot", "x", "y", "z", "mode", "duration"],
        _ => unreachable!("registered sequence argument schema"),
    }
}

fn missing_scene(entry: &SequenceEntry, finish: bool) -> Result<(), Box<dyn Error>> {
    match entry {
        SequenceEntry::CameraMotion { pc, .. } => {
            Err(error(*pc, 0x2287, "camera sequence needs a camera host").into())
        }
        SequenceEntry::WorldMotion {
            snap_on_finish: false,
            ..
        } if finish => Ok(()),
        SequenceEntry::WorldMotion {
            pc, snap_on_finish, ..
        } => Err(error(
            *pc,
            if *snap_on_finish { 0x2289 } else { 0x228a },
            "motion sequence needs scene",
        )
        .into()),
        SequenceEntry::ReplaceImage { pc, .. } => {
            Err(error(*pc, 0x2288, "image sequence needs scene and image source").into())
        }
        SequenceEntry::Wait { .. } => unreachable!("wait handled by sequence"),
    }
}

/// Record 0x6e0 body length: u16 version 1 and the 0x614-byte buffer (0x1400142b0).
pub const SAVED_SEQUENCE: usize = 2 + 20 + SEQUENCE_ENTRIES * 96;

impl SequenceHost {
    /// Record 0x6e0: the five header words, then each 96-byte entry as 0x1400143e0 /
    /// 0x140014540 / 0x140014720 write it (type, subtype, slot, 64-byte name, duration,
    /// three motion words, mode). Rust does not keep the bytes an earlier, longer name
    /// leaves behind its terminator; they are written as zero.
    pub(crate) fn saved(&self) -> Result<Vec<u8>, &'static str> {
        let mut out = 1u16.to_le_bytes().to_vec();
        let h = self.header;
        for word in [h.active, h.count, h.index, h.start, h.waited] {
            out.extend(word.to_le_bytes());
        }
        for entry in &self.entries {
            let mut raw = [0u8; 96];
            let mut put =
                |at: usize, value: u32| raw[at..at + 4].copy_from_slice(&value.to_le_bytes());
            match entry {
                Some(SequenceEntry::CameraMotion {
                    xyz,
                    mode,
                    duration,
                    ..
                }) => {
                    put(0, 4);
                    put(4, *mode);
                    put(76, *duration);
                    put(80, xyz[0]);
                    put(84, xyz[1]);
                    put(88, xyz[2]);
                }
                None => {}
                Some(SequenceEntry::Wait { duration }) => {
                    put(0, 1);
                    put(76, *duration);
                }
                Some(SequenceEntry::ReplaceImage {
                    slot,
                    name,
                    duration,
                    ..
                }) => {
                    if name.len() >= 64 {
                        return Err("sequence image name exceeds its field");
                    }
                    put(0, 3);
                    put(8, *slot);
                    put(76, *duration);
                    raw[12..12 + name.len()].copy_from_slice(name);
                }
                Some(SequenceEntry::WorldMotion {
                    snap_on_finish,
                    slot,
                    xyz,
                    mode,
                    duration,
                    ..
                }) => {
                    put(0, 3);
                    put(4, if *snap_on_finish { 1 } else { 2 });
                    put(8, *slot);
                    put(76, *duration);
                    put(80, xyz[0]);
                    put(84, xyz[1]);
                    put(88, xyz[2]);
                    put(92, *mode);
                }
            }
            out.extend(raw);
        }
        Ok(out)
    }

    /// Reader 0x1400142e0: copies the buffer, and for an active non-empty sequence moves
    /// the start time so the wait already consumed stays consumed (start = now - waited).
    pub(crate) fn restore(&mut self, record: &[u8]) -> Result<(), &'static str> {
        if record.len() != SAVED_SEQUENCE || record[..2] != [1, 0] {
            return Err("unsupported sequence record");
        }
        let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
        let mut header = SequenceHeader {
            active: word(2),
            count: word(6),
            index: word(10),
            start: word(14),
            waited: word(18),
        };
        if header.count > SEQUENCE_ENTRIES as u32 {
            return Err("saved sequence count exceeds its entries");
        }
        let mut entries: [Option<SequenceEntry>; SEQUENCE_ENTRIES] = Default::default();
        for (i, entry) in entries.iter_mut().enumerate() {
            let raw = &record[22 + 96 * i..22 + 96 * (i + 1)];
            let at =
                |offset: usize| u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap());
            let name = &raw[12..76];
            let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(64)];
            *entry = match (at(0), at(4)) {
                (0, _) if raw.iter().all(|&b| b == 0) => None,
                (1, 0) => Some(SequenceEntry::Wait { duration: at(76) }),
                (3, 0) => Some(SequenceEntry::ReplaceImage {
                    slot: at(8),
                    name: name.to_vec(),
                    duration: at(76),
                    pc: 0,
                }),
                (3, subtype @ (1 | 2)) => Some(SequenceEntry::WorldMotion {
                    snap_on_finish: subtype == 1,
                    slot: at(8),
                    xyz: [at(80), at(84), at(88)],
                    mode: at(92),
                    duration: at(76),
                    pc: 0,
                }),
                (4, mode) => Some(SequenceEntry::CameraMotion {
                    mode,
                    xyz: [at(80), at(84), at(88)],
                    duration: at(76),
                    pc: 0,
                }),
                _ => return Err("saved sequence entry type is not modeled"),
            };
        }
        if header.active != 0 && header.count != 0 {
            header.start = self.clock.wrapping_sub(header.waited);
        }
        self.header = header;
        self.entries = entries;
        Ok(())
    }
}

#[cfg(test)]
mod motion_snapshot_tests {
    use super::*;
    #[test]
    fn camera_sequence_entry_round_trips_raw_parameters() {
        let mut host = SequenceHost::default();
        host.header.count = 1;
        host.entries[0] = Some(SequenceEntry::CameraMotion {
            xyz: [0x8000_0000, 0x7fc0_1234, 0x7f80_0000],
            mode: u32::MAX,
            duration: u32::MAX,
            pc: 0,
        });
        let bytes = host.saved().unwrap();
        assert_eq!(u32::from_le_bytes(bytes[22..26].try_into().unwrap()), 4);
        let mut restored = SequenceHost::default();
        restored.restore(&bytes).unwrap();
        assert_eq!(restored.entries, host.entries);
        assert_eq!(restored.saved().unwrap(), bytes);
    }
    #[test]
    fn both_motion_subtypes_survive_sequence_snapshot_round_trip() {
        for snap_on_finish in [false, true] {
            let mut host = SequenceHost::default();
            host.header.count = 1;
            host.entries[0] = Some(SequenceEntry::WorldMotion {
                snap_on_finish,
                slot: 255,
                xyz: [0x8000_0000, 0x7fc0_1234, 0x7f80_0000],
                mode: u32::MAX,
                duration: u32::MAX,
                pc: 0,
            });
            let bytes = host.saved().unwrap();
            assert_eq!(
                u32::from_le_bytes(bytes[26..30].try_into().unwrap()),
                if snap_on_finish { 1 } else { 2 }
            );
            let mut restored = SequenceHost::default();
            restored.restore(&bytes).unwrap();
            assert_eq!(restored.entries, host.entries);
            assert_eq!(restored.saved().unwrap(), bytes);
        }
    }
}
