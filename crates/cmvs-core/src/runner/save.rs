//! Gameplay saves for the researched layout: the engine snapshot (0x140069270) taken at
//! 0x2080 and 0x22b2, slot writes 0x22b6 (0x140080b20), loads 0x22b5 (0x140081960 then
//! 0x140069fd0), thumbnails 0x22b8 (0x14006b4a0) and the slot queries 0x22b7 / 0x22bd.
use super::{Startup, SystemOutput};
use crate::{
    evaluate::error,
    message::{MESSAGE_WINDOWS, TEXT_OBJECTS},
    platform::PlatformBackend,
    raster::RgbaImage,
    save::{Csv2Save, HEADER_SIZE},
    save_menu::SaveHeader,
    scene::{ClearState, ObjectImage, SceneObject},
    snapshot::{CapturedSave, EngineSnapshot, LocalVariables, SaveImage, SavedScene, SavedWindow},
    text::bank_slot,
    vm::SystemResult,
};
use std::{collections::BTreeMap, error::Error, fs, io::Write, path::Path};

/// Local wall-clock fields for save labels: year, month, day, hour, minute, second.
pub type SaveTime = [u16; 6];

/// Constructor value of +12080, written to header offset 8; only an unimplemented
/// command changes it.
const HEADER_WORD_8: u32 = 0x1_0000;
/// Slot 999 keeps the previous version of an overwritten slot (0x140068b50, slots < 950).
const BACKUP_SLOT: u16 = 999;
/// The loader first saves the current state to slot 998 (0x1400818e0, loads of slots < 950).
const LOAD_BACKUP_SLOT: u16 = 998;
/// Callbacks 29 and 30 run after loads and saves (0x140068770).
const LOAD_CALLBACK: u8 = 29;
const SAVE_CALLBACK: u8 = 30;

fn label_time(time: SaveTime) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        time[0], time[1], time[2], time[3], time[4], time[5]
    )
}

/// 0x14006b4a0: a 24-bit bottom-up BMP of `width` x `height`. Each pixel averages the
/// integer box `(source << 10) / target >> 10` from the 10-bit fixed-point source position,
/// skipping samples past the source edge; channels divide as integers.
pub fn thumbnail_bmp(frame: &RgbaImage, width: u32, height: u32) -> Vec<u8> {
    let (src_w, src_h) = (frame.width(), frame.height());
    let row = ((3 * width + 3) & !3) as usize;
    let size = 54 + row * height as usize;
    let mut out = vec![0u8; size];
    out[..2].copy_from_slice(b"BM");
    out[2..6].copy_from_slice(&(size as u32).to_le_bytes());
    out[10..14].copy_from_slice(&54u32.to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&width.to_le_bytes());
    out[22..26].copy_from_slice(&height.to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&24u16.to_le_bytes());
    out[34..38].copy_from_slice(&((row * height as usize) as u32).to_le_bytes());
    let step_x = (src_w << 10) / width.max(1);
    let step_y = (src_h << 10) / height.max(1);
    let (box_w, box_h) = (step_x >> 10, step_y >> 10);
    let pixels = frame.pixels();
    for y in 0..height {
        let source_y = (y * step_y) >> 10;
        let line = 54 + row * (height - 1 - y) as usize;
        for x in 0..width {
            let source_x = (x * step_x) >> 10;
            let (mut sum, mut count) = ([0u32; 3], 0u32);
            for dy in 0..box_h {
                for dx in 0..box_w {
                    let (sx, sy) = (source_x + dx, source_y + dy);
                    if sx < src_w && sy < src_h {
                        let at = 4 * (sy * src_w + sx) as usize;
                        sum[0] += u32::from(pixels[at + 2]);
                        sum[1] += u32::from(pixels[at + 1]);
                        sum[2] += u32::from(pixels[at]);
                        count += 1;
                    }
                }
            }
            if let Some(count) = std::num::NonZeroU32::new(count) {
                let at = line + 3 * x as usize;
                for (i, channel) in sum.iter().enumerate() {
                    out[at + i] = (*channel / count) as u8;
                }
            }
        }
    }
    out
}

/// A 24-bit bottom-up BMP as 0x140032880 loads a thumbnail, into opaque RGBA.
fn decode_bmp(bytes: &[u8]) -> Result<RgbaImage, &'static str> {
    let word = |at: usize| -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(
            bytes
                .get(at..at + 4)
                .ok_or("truncated BMP")?
                .try_into()
                .unwrap(),
        ))
    };
    if bytes.get(..2) != Some(b"BM") || bytes.get(28..30) != Some(&[24, 0]) || word(30)? != 0 {
        return Err("thumbnail is not an uncompressed 24-bit BMP");
    }
    let (offset, width, height) = (word(10)? as usize, word(18)?, word(22)?);
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("thumbnail BMP size is out of range");
    }
    let row = ((3 * width + 3) & !3) as usize;
    let data = bytes
        .get(offset..offset + row * height as usize)
        .ok_or("truncated BMP pixels")?;
    let mut rgba = Vec::with_capacity(4 * (width * height) as usize);
    for y in 0..height as usize {
        let line = &data[row * (height as usize - 1 - y)..];
        for x in 0..width as usize {
            rgba.extend([line[3 * x + 2], line[3 * x + 1], line[3 * x], 255]);
        }
    }
    RgbaImage::new(width, height, rgba).map_err(|e| e.0)
}

impl Startup<'_> {
    /// Command inheritance does not establish the wire format of gameplay snapshots.
    /// The 20150405 writer uses text record v2, eight common-image slots and no effect 9;
    /// the root writer uses text v3, sixteen slots and effect 9. Keep derived layouts
    /// blocked until their encoders and restorers have independent evidence.
    fn require_snapshot_layout(&self) -> Result<(), Box<dyn Error>> {
        let profile = self.gate.selection().profile;
        if profile.base.is_some() {
            return Err(format!(
                "gameplay snapshot layout is not implemented for engine profile {}",
                profile.id
            )
            .into());
        }
        Ok(())
    }
    /// Wall-clock source for labels; the default is a fixed research time so runs repeat.
    pub fn set_save_clock(&mut self, clock: Box<dyn FnMut() -> SaveTime>) {
        self.save_clock = clock;
    }
    /// Research inspection: the snapshot the writer would build from the current hosts,
    /// without the live bit-0 change or replacing the held image.
    pub fn inspect_snapshot(&self) -> Result<EngineSnapshot, Box<dyn Error>> {
        Ok(self.engine_snapshot_with(self.machine.capture_vm()?)?.0)
    }
    /// The held snapshot image (built, loaded, or none).
    pub fn held_snapshot(&self) -> &SaveImage {
        &self.save_image
    }
    /// Engine +11192: the last 0x22b8 thumbnail or the one a loaded slot carried.
    pub fn thumbnail(&self) -> Option<&[u8]> {
        self.thumbnail.as_deref()
    }

    /// The engine snapshot writer 0x140069270 over the Rust hosts, with `vm` as the writer
    /// sees it. Fails when live state has no record format here.
    fn engine_snapshot_with(
        &self,
        vm: crate::machine::VmImage,
    ) -> Result<(EngineSnapshot, Vec<u8>), Box<dyn Error>> {
        self.require_snapshot_layout()?;
        let mut variables = LocalVariables::capture(self.machine.state());
        // The captured copy holds bit 0 = 1; the live bit was cleared by the caller.
        variables.set_bit0();
        let texts = self.messages.saved_texts().clone();
        let mut windows: [Option<SavedWindow>; MESSAGE_WINDOWS] = Default::default();
        for (slot, window) in self.messages.saved_windows().iter().enumerate() {
            if let Some(window) = window {
                let text = texts[window.text]
                    .as_ref()
                    .ok_or("window text object is missing")?;
                let size = text.logical_size.map(|v| v as u32);
                windows[slot] = Some(SavedWindow::from_window(window, size)?);
            }
        }
        let objects = self
            .scene
            .objects()
            .iter()
            .map(|(&slot, object)| Ok((slot, SavedScene::from_object(object)?)))
            .collect::<Result<BTreeMap<_, _>, crate::FormatError>>()?;
        let mut raw_objects = BTreeMap::new();
        raw_objects.insert((0x540, 0), self.history.saved());
        for (slot, record) in self.buttons.saved_groups() {
            raw_objects.insert((0x8c0, slot), record);
        }
        for (slot, record) in self.buttons.saved_lists() {
            raw_objects.insert((0x8c8, slot), record);
        }
        for (slot, record) in self.movies.saved_scene_players() {
            raw_objects.insert((0x8e8, slot), record);
        }
        let (second_group, popup) = self.save_menu.saved_flags();
        let clear = self.scene.clear();
        let snapshot = EngineSnapshot {
            save_title: self.save_title.bytes().to_vec(),
            scene_title: self.platform.scene_title().to_vec(),
            script_names: vm.script_names.clone(),
            script_sizes: vm.script_sizes,
            active_slot: vm.active_slot,
            pc: vm.pc,
            repeat_count: vm.repeat_count,
            timers: *self.timers.values(),
            word_16584: second_group,
            callbacks: self.control.saved_callbacks(),
            clear_color: clear.color,
            word_11144: 1,
            word_12092: popup,
            depth_clear: u32::from(clear.depth),
            stack_pointer: vm.stack_pointer,
            frame_depth: vm.frame_depth,
            frames: vm.frames,
            stack: vm.stack.clone(),
            system_results: vm.system_results,
            variables,
            texts,
            windows,
            objects,
            raw_objects,
            effects: [
                self.scene.saved_quake(),
                self.scene.saved_fade(),
                self.scene.saved_mask(),
                self.scene.saved_strip(),
            ],
            record_6e0: self.sequence.saved()?,
            cameras: [
                self.camera.saved(0),
                self.camera.saved(1),
                self.camera.saved_view(0),
                self.camera.saved_view(1),
            ],
            script_data: vm.main_data.clone(),
            bgm: self.audio.saved_bgm(),
            sound_channels: self.audio.saved_channels(),
            sound_device: vec![1, 0, 0, 0, 0, 0],
        };
        Ok((snapshot, vm.main_image))
    }

    /// The 600-byte header 0x140069270 prepares at +11204.
    fn snapshot_header(&mut self) -> Result<[u8; HEADER_SIZE], Box<dyn Error>> {
        let mut header = [0u8; HEADER_SIZE];
        header[..4].copy_from_slice(b"CSV2");
        header[4..8].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        header[8..12].copy_from_slice(&HEADER_WORD_8.to_le_bytes());
        let time = (self.save_clock)();
        let mut label = format!("{} ", label_time(time)).into_bytes();
        label.extend(self.save_title.bytes());
        if label.len() >= 256 {
            return Err("save label exceeds its 256-byte field".into());
        }
        header[16..16 + label.len()].copy_from_slice(&label);
        for (i, value) in self.save_info.values().iter().enumerate() {
            header[528 + 4 * i..532 + 4 * i].copy_from_slice(&value.unwrap_or(0).to_le_bytes());
        }
        header[544..546].copy_from_slice(&4u16.to_le_bytes());
        let thumbnail = self.platform.thumbnail().is_some_and(|t| t.enabled);
        header[548..550].copy_from_slice(&u16::from(thumbnail).to_le_bytes());
        Ok(header)
    }

    /// 0x2080 / 0x22b2: the image is built before any live change, so a state without a
    /// record format leaves the VM, bit 0 and the held image as they were.
    pub(super) fn capture_snapshot(
        &mut self,
        vm: crate::machine::VmImage,
    ) -> Result<(), Box<dyn Error>> {
        let (snapshot, script_image) = self.engine_snapshot_with(vm)?;
        let bytes = snapshot.encode()?;
        if bytes.len() > crate::snapshot::MAX_SNAPSHOT {
            return Err("snapshot exceeds the original 3 MiB buffer".into());
        }
        let header = self.snapshot_header()?;
        // 0x140069270 sets bit 0 only for the copy; the live bit ends clear.
        self.machine.state_mut().set_bit(0, false)?;
        self.save_image = SaveImage::Captured(Box::new(CapturedSave {
            header,
            snapshot: bytes,
            script_image,
        }));
        Ok(())
    }

    /// The directory gameplay slots are written to: only an independent user directory.
    pub(super) fn slot_directory(&self) -> Result<std::path::PathBuf, Box<dyn Error>> {
        match &self.system_output {
            Some(SystemOutput::Profile(profile)) => Ok(profile.directory().to_path_buf()),
            _ => Err(
                "gameplay saves require --user-data-dir; the game directory stays read-only".into(),
            ),
        }
    }

    /// 0x140080b20 without its native overwrite dialog. Returns false when no snapshot was
    /// ever taken, which the original also treats as nothing to write. After a write the
    /// original enters callback 30 (0x140068770) when registered; this title never registers
    /// it, and the command flags applied afterwards are not modeled, so an enabled callback
    /// is an error except for the pre-load backup, whose context the load replaces.
    fn write_slot(&mut self, slot: u16, before_load: bool) -> Result<bool, Box<dyn Error>> {
        if self.slot_edit.is_some() {
            return Err("finish the pending slot edit before writing a gameplay save".into());
        }
        let captured = match &self.save_image {
            SaveImage::Absent => return Ok(false),
            SaveImage::Unavailable { .. } => {
                return Err("no snapshot image was built (--continue-without-snapshot)".into());
            }
            SaveImage::Captured(captured) => captured.clone(),
        };
        if !before_load
            && self
                .control
                .callbacks()
                .get(&SAVE_CALLBACK)
                .is_some_and(|c| c.enabled)
        {
            return Err("entering the save callback 30 after a write is not modeled".into());
        }
        let directory = self.slot_directory()?;
        let path = directory.join(format!("save{slot:03}.dat"));
        let mut header = captured.header;
        let now = label_time((self.save_clock)()).into_bytes();
        for i in [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
            header[16 + i] = now[i];
        }
        let thumbnail = match (
            u16::from_le_bytes([header[548], header[549]]),
            &self.thumbnail,
        ) {
            (0, _) | (_, None) => Vec::new(),
            (_, Some(bmp)) => bmp.clone(),
        };
        let file = Csv2Save::encode(
            &header,
            &captured.snapshot,
            &thumbnail,
            &captured.script_image,
        )?;
        // 0x140068b50: keep the previous version of slots below 950 as slot 999.
        if slot < 950 && path.try_exists()? {
            let previous = fs::read(&path)?;
            replace_file(&directory, &format!("save{BACKUP_SLOT:03}.dat"), &previous)?;
        }
        // 0x1400179e0: the system save is written with every slot.
        let id = self.bootstrap.settings().get(&0x201d).copied();
        if let Some(output) = &mut self.system_output {
            self.system_host
                .write_now(self.machine.state(), output, &mut self.random, id)?;
            self.system_source = crate::system_save::FileSystemSave::new(output.path());
        }
        replace_file(&directory, &format!("save{slot:03}.dat"), &file)?;
        Ok(true)
    }

    /// The write 0x22b6 performs, from any point between VM steps. False when no snapshot
    /// has been taken yet, as in the original.
    pub fn save_slot(&mut self, slot: u16) -> Result<bool, Box<dyn Error>> {
        if slot > 999 {
            return Err("save slot outside 0..999".into());
        }
        self.write_slot(slot, false)
    }

    /// 0x22b6 / 0x1400812f0: SP-4 slot; slots above 999 are ignored. Returns 0x4004.
    pub(super) fn try_save_slot(&mut self) -> Result<bool, Box<dyn Error>> {
        let Some((pc, 0x2b6)) = self.machine.pending_system() else {
            return Ok(false);
        };
        let slot = self
            .machine
            .stack()
            .peek_u32(4)
            .map_err(|e| error(pc, 0x22b6, e.0))?;
        if slot <= 999 {
            self.write_slot(slot as u16, false)?;
        }
        self.machine.complete_system(SystemResult(0x4004))?;
        Ok(true)
    }

    /// 0x22b5 / 0x140082340: SP-4 slot. The load replaces the VM, so the result flags are
    /// applied by hand: 0x2000 (repeat + 1) when the restored repeat count is nonzero, else
    /// 0 (repeat reset, nothing popped, the restored PC runs next).
    pub(super) fn try_load_slot(
        &mut self,
        backend: &mut impl PlatformBackend,
    ) -> Result<bool, Box<dyn Error>> {
        let Some((pc, 0x2b5)) = self.machine.pending_system() else {
            return Ok(false);
        };
        let slot = self
            .machine
            .stack()
            .peek_u32(4)
            .map_err(|e| error(pc, 0x22b5, e.0))?;
        if slot > 999 {
            self.machine.complete_system(SystemResult(0x4004))?;
            return Ok(true);
        }
        self.load_slot(slot as u16, backend)?;
        let repeat = self.machine.repeat_count();
        self.machine
            .set_repeat_count(if repeat != 0 { repeat + 1 } else { 0 });
        Ok(true)
    }

    /// The load 0x22b5 performs, from any point between VM steps: the slot replaces the VM
    /// and every saved host, so where the script stood before does not matter. Slots below
    /// 950 first save the current snapshot to slot 998 when one exists (0x1400818e0).
    pub fn load_slot(
        &mut self,
        slot: u16,
        backend: &mut impl PlatformBackend,
    ) -> Result<(), Box<dyn Error>> {
        self.require_snapshot_layout()?;
        if self.slot_edit.is_some() {
            return Err("finish the pending slot edit before loading a gameplay save".into());
        }
        let bytes = self
            .saves
            .read_slot(slot)?
            .ok_or("save slot to load does not exist")?;
        let save = Csv2Save::decode(&bytes)?;
        let snapshot = EngineSnapshot::decode(&save.snapshot)?;
        if slot < 950 && matches!(self.save_image, SaveImage::Captured(_)) {
            self.write_slot(LOAD_BACKUP_SLOT, true)?;
        }
        self.restore_snapshot(&save, &bytes[..HEADER_SIZE], &snapshot, backend)
    }

    /// Loader 0x140081960 and reader 0x140069fd0 over the Rust hosts. Everything fallible
    /// that does not touch a device (decoding, script layout, images) runs before the first
    /// host changes; audio and window-title requests then go to the backends.
    fn restore_snapshot(
        &mut self,
        save: &Csv2Save,
        header: &[u8],
        snapshot: &EngineSnapshot,
        backend: &mut impl PlatformBackend,
    ) -> Result<(), Box<dyn Error>> {
        if snapshot.word_11144 != 1 {
            return Err("saved +11144 value is not modeled".into());
        }
        let quake = snapshot.effects[0]
            .as_deref()
            .map(crate::scene::Quake::restore)
            .transpose()?;
        let fade = snapshot.effects[1]
            .as_deref()
            .map(crate::fade::Fade::restore)
            .transpose()?;
        let strip = match snapshot.effects[3].as_deref() {
            Some(record) => crate::scene::StripWipe::restore(record)?,
            None => None,
        };
        // 0x6a0 holds D3D render-target allocation metadata, not overlay pixels.
        // 0x14001dc70 -> 0x140026dd0 ignores the saved dimensions and rebuilds
        // square targets from the current viewport. The software backend needs
        // no D3D allocation; validate the record rather than inventing scene pixels.
        for (&(tag, _), body) in &snapshot.raw_objects {
            if tag == 0x6a0 && (body.len() != 10 || body[..2] != [0, 0]) {
                return Err("unsupported saved render-target record".into());
            }
        }
        if snapshot
            .raw_objects
            .keys()
            .any(|&(tag, _)| !matches!(tag, 0x6a0 | 0x540 | 0x8c0 | 0x8c8 | 0x8e8))
        {
            return Err("saved camera-sequence objects are not implemented".into());
        }
        if snapshot.sound_device != [1, 0, 0, 0, 0, 0] {
            return Err("saved sound device buffers are not implemented".into());
        }
        let vm = crate::machine::VmImage {
            script_names: snapshot.script_names.clone(),
            script_sizes: snapshot.script_sizes,
            active_slot: snapshot.active_slot,
            pc: snapshot.pc,
            repeat_count: snapshot.repeat_count,
            stack_pointer: snapshot.stack_pointer,
            frame_depth: snapshot.frame_depth,
            frames: snapshot.frames,
            stack: snapshot.stack.clone(),
            system_results: snapshot.system_results,
            main_image: save.script_image.clone(),
            main_data: snapshot.script_data.clone(),
        };
        // Candidate hosts first: images, windows and texts must all rebuild.
        let mut budget = crate::snapshot::RESTORE_PIXELS;
        let mut images = self.images.with_paths(self.bootstrap.paths());
        let mut objects = BTreeMap::new();
        for (&slot, saved) in &snapshot.objects {
            objects.insert(slot, saved.load(&mut images, &mut budget)?);
        }
        // Record 0x620 follows the objects it shades.
        let mask = snapshot.effects[2]
            .as_deref()
            .map(|record| crate::scene::MaskTransition::restore(record, &objects, &mut images))
            .transpose()?;
        let mut windows: [Option<crate::message::MessageWindow>; MESSAGE_WINDOWS] =
            Default::default();
        for (slot, saved) in snapshot.windows.iter().enumerate() {
            if let Some(saved) = saved {
                windows[slot] = Some(saved.load(&mut images, &mut budget)?);
            }
        }
        // Records 0x8e8 follow 0x700 and bind to the objects just rebuilt.
        let movie_records: Vec<_> = snapshot
            .raw_objects
            .iter()
            .filter(|((tag, _), _)| *tag == 0x8e8)
            .map(|(&(_, slot), body)| (slot, body.as_slice()))
            .collect();
        let players = self.movies.load_scene_players(
            &movie_records,
            &mut objects,
            &mut self.scene_movies.with_paths(
                self.bootstrap.paths(),
                crate::resources::ResourceKind::SceneMovie,
            ),
        )?;
        let records = |tag: u16| -> Vec<(u16, &[u8])> {
            snapshot
                .raw_objects
                .iter()
                .filter(|((t, _), _)| *t == tag)
                .map(|(&(_, slot), body)| (slot, body.as_slice()))
                .collect()
        };
        let mut buttons = self.buttons.clone();
        buttons.restore(&records(0x8c0), &records(0x8c8))?;
        let texts: [Option<_>; TEXT_OBJECTS] = snapshot.texts.clone();
        let mut history = self.history.clone();
        history.restore(&snapshot.raw_objects[&(0x540, 0)])?;
        let mut sequence = self.sequence.clone();
        sequence.restore(&snapshot.record_6e0)?;
        let mut camera_host = self.camera.clone();
        camera_host.restore(0, &snapshot.cameras[0])?;
        camera_host.restore(1, &snapshot.cameras[1])?;
        camera_host.restore_view(0, &snapshot.cameras[2])?;
        camera_host.restore_view(1, &snapshot.cameras[3])?;
        let mut control = self.control.clone();
        control.restore_callbacks(&snapshot.callbacks)?;
        self.platform.restored_title(&snapshot.scene_title)?;
        // The local banks are validated against a copy before the VM changes.
        snapshot
            .variables
            .apply_to_state(&mut self.machine.state().clone())?;
        self.machine.restore_vm(&vm)?;
        snapshot
            .variables
            .apply_to_state(self.machine.state_mut())?;
        // Commit the hosts.
        self.history = history;
        self.sequence = sequence;
        self.camera = camera_host;
        self.control = control;
        self.timers.restore_values(snapshot.timers);
        self.save_title.restore(snapshot.save_title.clone());
        self.save_menu.restore_flags(snapshot.word_12092);
        self.message_flags.clear_for_load();
        self.buttons = buttons;
        if self.movies.restore(players) {
            self.audio_backend.stop_movie_audio()?;
        }
        self.scene.restore(
            ClearState {
                color: snapshot.clear_color,
                depth: snapshot.depth_clear != 0,
            },
            objects,
            quake,
            fade,
            mask,
            strip,
        );
        self.messages.restore(texts, windows);
        let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
        self.save_info.restore([word(528), 0, word(536)]);
        if !save.auxiliary.is_empty() {
            self.thumbnail = Some(save.auxiliary.clone());
        }
        self.save_image = SaveImage::Captured(Box::new(CapturedSave {
            header: header.try_into().expect("600-byte header"),
            snapshot: save.snapshot.clone(),
            script_image: save.script_image.clone(),
        }));
        self.audio.restore(
            &snapshot.bgm,
            &snapshot.sound_channels,
            self.audio_backend.as_mut(),
        )?;
        self.platform
            .restore_scene_title(snapshot.scene_title.clone(), backend)?;
        if let Some(callback) = self
            .control
            .callbacks()
            .get(&LOAD_CALLBACK)
            .filter(|c| c.enabled)
        {
            let (slot, target) = (callback.script_slot, callback.target as usize);
            self.machine.enter_callback(slot, target)?;
        }
        Ok(())
    }

    /// 0x22b8: 0x14006b4a0 builds the thumbnail from the screen capture when one exists,
    /// otherwise from the frame; returns 0x4000. Text glyphs drawn by the desktop font
    /// backend are not part of the core frame.
    pub(super) fn try_thumbnail(&mut self) -> Result<bool, Box<dyn Error>> {
        if !matches!(self.machine.pending_system(), Some((_, 0x2b8))) {
            return Ok(false);
        }
        let config = self
            .platform
            .thumbnail()
            .ok_or("thumbnail size (0x22b0) was never configured")?;
        let frame = match self.capture.image() {
            Some(image) => image.clone(),
            None => self.render_frame()?,
        };
        let bmp = thumbnail_bmp(&frame, config.width, config.height);
        self.machine.complete_system(SystemResult(0x4000))?;
        self.thumbnail = Some(bmp);
        Ok(true)
    }

    /// 0x22b7 / 0x14007caf0: SP-4 first slot, SP-8 count. System result 0 is the slot in
    /// range whose label orders latest (0x140093490), or -1; out of range also gives -1.
    pub(super) fn try_latest_slot(&mut self) -> Result<bool, Box<dyn Error>> {
        let Some((pc, 0x2b7)) = self.machine.pending_system() else {
            return Ok(false);
        };
        let arg = |d| {
            self.machine
                .stack()
                .peek_u32(d)
                .map_err(|e| error(pc, 0x22b7, e.0))
        };
        let (start, count) = (arg(4)? as i32, arg(8)? as i32);
        let mut latest = u32::MAX;
        if start >= 0 && count.wrapping_add(start) <= 999 {
            let mut best = 0;
            for slot in start..start.wrapping_add(count) {
                if let Some(bytes) =
                    crate::save_menu::SaveSource::read_header(&mut self.saves, slot as u16)?
                    && let Ok(header) = SaveHeader::parse(&bytes)
                    && header.ordering_key > best
                {
                    best = header.ordering_key;
                    latest = slot as u32;
                }
            }
        }
        self.machine.complete_system(SystemResult(0x4008))?;
        self.machine
            .state_mut()
            .set_system_integer(0, latest)
            .expect("slot 0 exists");
        Ok(true)
    }

    /// 0x22bd / 0x14007cf10: SP-4 slot, SP-8 object, SP-12 child (negative = the object
    /// itself, recreated), SP-16/SP-20 string destinations for the label (+16) and the
    /// second text (+272). Results 0 = found, 1 = label order key, 4..6 = header 528..536;
    /// the thumbnail becomes the target image (a blank opaque black one when the header
    /// enables thumbnails but the file has none). Returns 0x4014.
    pub(super) fn try_slot_info(&mut self) -> Result<bool, Box<dyn Error>> {
        let Some((pc, 0x2bd)) = self.machine.pending_system() else {
            return Ok(false);
        };
        let fail = |why: &'static str| error(pc, 0x22bd, why);
        let arg = |d| self.machine.stack().peek_u32(d).map_err(|e| fail(e.0));
        let (slot, object, child) = (arg(4)? as i32, arg(8)?, arg(12)? as i32);
        let label_slot = bank_slot(arg(16)?)
            .ok_or_else(|| fail("label destination outside the string bank is not modeled"))?;
        let text_slot = bank_slot(arg(20)?)
            .ok_or_else(|| fail("text destination outside the string bank is not modeled"))?;
        if slot < 0 || object > 255 || slot > 999 {
            self.machine.complete_system(SystemResult(0x4014))?;
            self.machine
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0 exists");
            return Ok(true);
        }
        let Some(bytes) = self.saves.read_slot(slot as u16)? else {
            self.machine.complete_system(SystemResult(0x4014))?;
            self.machine
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0 exists");
            return Ok(true);
        };
        if bytes.len() < HEADER_SIZE || &bytes[..3] != b"CSV" || !(b'2'..=b'9').contains(&bytes[3])
        {
            self.machine.complete_system(SystemResult(0x4014))?;
            self.machine
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0 exists");
            return Ok(true);
        }
        let header = SaveHeader::parse(&bytes[..HEADER_SIZE])?;
        let c_string = |field: &[u8]| {
            field[..field.iter().position(|&b| b == 0).unwrap_or(field.len())].to_vec()
        };
        let label = c_string(&bytes[16..272]);
        let text = c_string(&bytes[272..528]);
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let image = if u16::from_le_bytes([bytes[548], bytes[549]]) != 0 {
            let config = self
                .platform
                .thumbnail()
                .ok_or_else(|| fail("thumbnail size is unknown"))?;
            if word(564) == 0 {
                let pixels = [0, 0, 0, 255].repeat((config.width * config.height) as usize);
                Some(ObjectImage {
                    filter: None,
                    name: Vec::new(),
                    pixels: RgbaImage::new(config.width, config.height, pixels)?,
                    alpha: true,
                    surface: false,
                })
            } else {
                let save = Csv2Save::decode(&bytes)?;
                Some(ObjectImage {
                    filter: None,
                    name: Vec::new(),
                    pixels: decode_bmp(&save.auxiliary)?,
                    alpha: false,
                    surface: false,
                })
            }
        } else {
            None
        };
        if let Some(image) = image {
            let target = if child < 0 {
                self.scene
                    .replace_object(object as u8, SceneObject::default());
                self.scene.object_mut(object as u8)
            } else {
                self.scene
                    .object_mut(object as u8)
                    .map(|parent| parent.children.entry(child as u16).or_default())
            };
            if let Some(target) = target {
                target.image = Some(image);
            }
        }
        if label.len() >= 1024 || text.len() >= 1024 {
            return Err(fail("save label overflows its string slot").into());
        }
        self.machine.complete_system(SystemResult(0x4014))?;
        let state = self.machine.state_mut();
        state
            .set_string(label_slot, &label)
            .expect("checked length");
        state.set_string(text_slot, &text).expect("checked length");
        for (index, value) in [
            (4, word(528)),
            (5, word(532)),
            (6, word(536)),
            (1, header.ordering_key),
            (0, 1),
        ] {
            state.set_system_integer(index, value).expect("slot exists");
        }
        Ok(true)
    }
}

/// Replaces a slot file through a fresh temporary file in the same directory.
fn replace_file(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let temporary = directory.join(format!(".{name}.tmp"));
    let _ = fs::remove_file(&temporary);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(e) = fs::rename(&temporary, directory.join(name)) {
        let _ = fs::remove_file(&temporary);
        return Err(e.into());
    }
    Ok(())
}
