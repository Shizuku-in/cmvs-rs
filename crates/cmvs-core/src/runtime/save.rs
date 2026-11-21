//! Gameplay saves for the researched layout: the engine snapshot (0x140069270) taken at
//! 0x2080 and 0x22b2, slot writes 0x22b6 (0x140080b20), loads 0x22b5 (0x140081960 then
//! 0x140069fd0), thumbnails 0x22b8 (0x14006b4a0) and the slot queries 0x22b7 / 0x22bd.
use super::{Runtime, SystemOutput};
use crate::filesystem::{HostFs, OpenOptions};
use crate::{
    audio::{SavedAudio, SavedCvnsAudio},
    dispatch::RuntimeCall,
    evaluate::error,
    message::{MESSAGE_WINDOWS, TEXT_OBJECTS},
    platform::PlatformBackend,
    raster::RgbaImage,
    save::{Csv2Save, HEADER_SIZE},
    save_menu::{SaveFamily, SaveHeader},
    scene::{ClearState, ObjectImage, SceneObject},
    snapshot::{
        CapturedSave, CvnsLocalVariables, CvnsSnapshot, EngineSnapshot, LocalVariables, SaveImage,
        SavedScene, SavedWindow, SnapshotLayout, cvns::native,
    },
};
use std::{collections::BTreeMap, error::Error, io::Write, path::Path, sync::Arc};

/// Local wall-clock fields for save labels: year, month, day, hour, minute, second.
pub type SaveTime = [u16; 6];

/// UTC save label fields for `seconds` since 1970-01-01 (proleptic Gregorian calendar).
pub fn save_time_from_unix(seconds: u64) -> SaveTime {
    let (days, rest) = ((seconds / 86_400) as i64, seconds % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    [
        year.clamp(0, i64::from(u16::MAX)) as u16,
        month as u16,
        day as u16,
        (rest / 3600) as u16,
        (rest / 60 % 60) as u16,
        (rest % 60) as u16,
    ]
}

/// Host save clock in UTC: the standard library has no local time zone without platform
/// calls, so every host labels saves identically. A clock before 1970 reads as the epoch.
pub fn utc_save_time() -> SaveTime {
    save_time_from_unix(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
}

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

/// Command inheritance does not establish the wire format of gameplay snapshots.
/// The independently inspected 20101211/20110525/20120309/20130131/20140217/20150405 subsets use text v2;
/// 20171114 uses v3, and 20190305/20190405/20211006/20221105 use text v3 with scene v4
/// (docs/games/{aoitori,realive,seishunf,kunado,hapymaher_re}). Effect configurations have an independent
/// build binding; unknown image, collision-mesh and effect records remain rejected.
pub fn snapshot_layout_supported(profile: &crate::profile::EngineProfile) -> bool {
    snapshot_layout(profile).is_some()
}

/// The gameplay snapshot layout of an inspected build, for decoding its saves offline.
pub fn snapshot_layout_for(profile: &crate::profile::EngineProfile) -> Option<SnapshotLayout> {
    snapshot_layout(profile)
}

fn snapshot_layout(profile: &crate::profile::EngineProfile) -> Option<SnapshotLayout> {
    use crate::profile::InspectedBuild;

    match profile.inspected_build() {
        Some(InspectedBuild::Cmvs20070119) => Some(SnapshotLayout::TextV1SceneLegacy),
        Some(InspectedBuild::Cmvs20071117) => Some(SnapshotLayout::TextV1SceneV1),
        Some(InspectedBuild::Cmvs20090815) => Some(SnapshotLayout::TextV1SceneV2Collision),
        Some(InspectedBuild::Cmvs20100314) => Some(SnapshotLayout::TextV2WindowV2),
        Some(InspectedBuild::Cmvs20080719 | InspectedBuild::Cmvs20080903) => {
            Some(SnapshotLayout::TextV1SceneV2)
        }
        Some(
            InspectedBuild::Cmvs20101211
            | InspectedBuild::Cmvs20110525
            | InspectedBuild::Cmvs20120309
            | InspectedBuild::Cmvs20130131
            | InspectedBuild::Cmvs20140217
            | InspectedBuild::Cmvs20150405,
        ) => Some(SnapshotLayout::TextV2),
        Some(
            InspectedBuild::Cmvs20190305
            | InspectedBuild::Cmvs20190405
            | InspectedBuild::Cmvs20211006
            | InspectedBuild::Cmvs20221105,
        ) => Some(SnapshotLayout::TextV3SceneV4),
        Some(InspectedBuild::Cmvs20160201 | InspectedBuild::Cmvs20171114) => {
            Some(SnapshotLayout::TextV3)
        }
        Some(
            InspectedBuild::Cvns20040515
            | InspectedBuild::Cvns20040730
            | InspectedBuild::Cvns20050216
            | InspectedBuild::Cvns20050727
            | InspectedBuild::Cvns20090203,
        ) => Some(SnapshotLayout::CvnsCsp1),
        None if profile.base.is_none() => Some(SnapshotLayout::TextV3),
        None => None,
    }
}

fn effect_layout_supported(profile: &crate::profile::EngineProfile) -> bool {
    use crate::profile::InspectedBuild;

    matches!(
        profile.inspected_build(),
        Some(
            InspectedBuild::Cmvs20080719
                | InspectedBuild::Cmvs20080903
                | InspectedBuild::Cmvs20090815
                | InspectedBuild::Cmvs20100314
                | InspectedBuild::Cmvs20110525
                | InspectedBuild::Cmvs20120309
                | InspectedBuild::Cmvs20160201
                | InspectedBuild::Cmvs20171114
                | InspectedBuild::Cmvs20211006
                | InspectedBuild::Cmvs20221105
        )
    )
}

fn base_effect_layout_supported(profile: &crate::profile::EngineProfile) -> bool {
    profile.inspected_build() == Some(crate::profile::InspectedBuild::Cmvs20160201)
}

fn effect_record_supported(profile: &crate::profile::EngineProfile, body: &[u8]) -> bool {
    use crate::profile::InspectedBuild;

    match profile.inspected_build() {
        Some(InspectedBuild::Cmvs20070119) => {
            body.len() == 52 && body.get(..4) == Some(&[1, 0, 3, 0])
                || body.len() == 60 && matches!(body.get(..4), Some([1, 0, 1 | 2, 0]))
                || body.len() == 4
                    && body[..2] == [1, 0]
                    && (0x50..=0x57).contains(&body[2])
                    && body[3] == 0
        }
        // Writer 0x425740 (NATSUUTA 0x427530 is the same code) / reader 0x425850: types 6 and 7 share the 48-byte template;
        // type 2 stores six words from object +12.
        Some(InspectedBuild::Cmvs20090815 | InspectedBuild::Cmvs20100314) => {
            body.len() == 52 && matches!(body.get(..4), Some([1, 0, 6 | 7, 0]))
                || body.len() == 60 && body.get(..4) == Some(&[1, 0, 2, 0])
        }
        _ if body.get(..4) == Some(&[1, 0, 3, 0]) => false,
        Some(InspectedBuild::Cmvs20160201) => {
            body.get(..4) == Some(&[1, 0, 2, 0]) || body == [1, 0, 8, 0]
        }
        Some(InspectedBuild::Cmvs20080903) => body.get(..4) == Some(&[1, 0, 4, 0]),
        _ if body.get(..4) == Some(&[1, 0, 4, 0]) => false,
        // The native type-8 writer emits only this header; its reader discards it.
        Some(InspectedBuild::Cmvs20221105) => {
            body == [1, 0, 8, 0] || body.get(..4) == Some(&[1, 0, 5, 0])
        }
        // These builds' type-5 writer/readers are independently inspected. Other
        // configurations still need their own runtime and snapshot evidence.
        Some(
            InspectedBuild::Cmvs20080719
            | InspectedBuild::Cmvs20110525
            | InspectedBuild::Cmvs20120309
            | InspectedBuild::Cmvs20211006,
        ) => body.get(..4) == Some(&[1, 0, 5, 0]),
        Some(InspectedBuild::Cmvs20171114) => true,
        _ => false,
    }
}

fn effect_state_supported(
    profile: &crate::profile::EngineProfile,
    scene: &crate::scene::SceneHost,
) -> bool {
    if profile.inspected_build() == Some(crate::profile::InspectedBuild::Cmvs20070119) {
        return scene
            .saved_effect_pools_with_legacy(true)
            .is_ok_and(|records| {
                records
                    .iter()
                    .all(|(_, body)| effect_record_supported(profile, body))
            });
    }
    effect_layout_supported(profile)
        && (!base_effect_layout_supported(profile)
            || scene.saved_effect_pools().is_ok_and(|records| {
                records
                    .iter()
                    .all(|(_, body)| effect_record_supported(profile, body))
            }))
}

/// Writers 20110525 0x1400332e0, 20120309 0x140033e60, 20130131 0x1400330b0
/// and 20140217 0x1400375d0 visit 512 children.
/// HASSAKU 0x140033000 visits 256. Button-list records are absent or not enabled
/// for these builds.
fn validate_child_slots_without_button_lists<'a>(
    scenes: impl Iterator<Item = &'a SavedScene>,
    has_button_lists: bool,
    capacity: u16,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    fn scene_supported(scene: &SavedScene, capacity: u16) -> bool {
        scene
            .children
            .iter()
            .all(|(&slot, child)| slot < capacity && scene_supported(child, capacity))
    }
    if has_button_lists {
        return Err("button-list snapshot records are not supported for this build".into());
    }
    if !scenes
        .into_iter()
        .all(|scene| scene_supported(scene, capacity))
    {
        return Err(
            format!("snapshot child slot exceeds this build's {capacity}-entry table").into(),
        );
    }
    Ok(())
}

impl Runtime {
    pub(super) fn system_save_identifier(&self) -> Option<u32> {
        self.gate
            .profile()
            .css1_game_id(self.bootstrap.settings().get(&0x201d).copied())
    }
    fn validate_build_snapshot(
        &self,
        snapshot: &EngineSnapshot,
        layout: SnapshotLayout,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        use crate::profile::InspectedBuild;

        if layout.old_tables() {
            if snapshot.effects[2].is_some() {
                return Err("old mask snapshot records are not implemented".into());
            }
            if let Some(&(tag, _)) = snapshot.raw_objects.keys().find(|&&(tag, _)| {
                tag == 0x8c8 || tag == 0x800 && layout == SnapshotLayout::TextV1SceneV1
            }) {
                return Err(format!("old snapshot record {tag:#x} is not implemented").into());
            }
        }
        if let Some(capacity) = match self.gate.profile().inspected_build() {
            Some(InspectedBuild::Cmvs20101211) => Some(256),
            Some(
                InspectedBuild::Cmvs20110525
                | InspectedBuild::Cmvs20120309
                | InspectedBuild::Cmvs20130131
                | InspectedBuild::Cmvs20140217,
            ) => Some(512),
            _ => None,
        } {
            validate_child_slots_without_button_lists(
                snapshot
                    .objects
                    .values()
                    .chain(snapshot.windows.iter().flatten().map(|w| &w.scene)),
                snapshot.raw_objects.keys().any(|&(tag, _)| tag == 0x8c8),
                capacity,
            )?;
        }
        Ok(())
    }
    fn require_snapshot_layout(&self) -> Result<SnapshotLayout, Box<dyn Error + Send + Sync>> {
        let profile = self.gate.profile();
        snapshot_layout(profile).ok_or_else(|| {
            format!(
                "gameplay snapshot layout is not implemented for engine profile {}",
                profile.id
            )
            .into()
        })
    }
    /// Wall-clock source for labels; the default is a fixed research time so runs repeat.
    pub fn set_save_clock(&mut self, clock: Box<dyn FnMut() -> SaveTime + Send>) {
        self.save_clock = clock;
    }
    /// Research inspection: the snapshot the writer would build from the current hosts,
    /// without the live bit-0 change or replacing the held image.
    pub fn inspect_snapshot(&self) -> Result<EngineSnapshot, Box<dyn Error + Send + Sync>> {
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
    ) -> Result<(EngineSnapshot, Vec<u8>), Box<dyn Error + Send + Sync>> {
        let layout = self.require_snapshot_layout()?;
        if self.scene.has_effect_pools()
            && !effect_state_supported(self.gate.profile(), &self.scene)
        {
            return Err("saving active particle effects is not implemented".into());
        }
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
        for (slot, record) in self
            .scene
            .saved_effect_pools_with_legacy(layout == SnapshotLayout::TextV1SceneLegacy)?
        {
            if !effect_record_supported(self.gate.profile(), &record) {
                return Err(
                    "saved particle effect type is not implemented for this profile".into(),
                );
            }
            raw_objects.insert((0x800, slot), record);
        }
        raw_objects.insert((0x540, 0), self.history.saved());
        for (slot, record) in self.buttons.saved_groups_with_layout(layout) {
            raw_objects.insert((0x8c0, slot), record);
        }
        for (slot, record) in self.buttons.saved_lists() {
            raw_objects.insert((0x8c8, slot), record);
        }
        for (slot, record) in self.movies.saved_scene_players_with_layout(layout) {
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
            record_6e0: if layout.old_tables() {
                if self.sequence.saved()? != crate::sequence::SequenceHost::default().saved()? {
                    return Err("sequence state has no v1 snapshot representation".into());
                }
                Vec::new()
            } else {
                self.sequence.saved()?
            },
            cameras: [
                self.camera.saved(0),
                if layout == SnapshotLayout::TextV1SceneLegacy {
                    Vec::new()
                } else {
                    self.camera.saved(1)
                },
                self.camera.saved_view(0),
                if layout == SnapshotLayout::TextV1SceneLegacy {
                    Vec::new()
                } else {
                    self.camera.saved_view(1)
                },
            ],
            script_data: vm.main_data.clone(),
            bgm: self.audio.saved_bgm(),
            sound_channels: self.audio.saved_channels(),
            sound_device: vec![1, 0, 0, 0, 0, 0],
        };
        self.validate_build_snapshot(&snapshot, layout)?;
        Ok((snapshot, vm.main_image))
    }

    pub(super) fn save_family(&self) -> SaveFamily {
        self.gate.profile().save_family()
    }

    /// The 600-byte header 0x140069270 (or CVNS 0x44ebf0) prepares at +11204 / +22816.
    fn snapshot_header(&mut self) -> Result<[u8; HEADER_SIZE], Box<dyn Error + Send + Sync>> {
        let mut header = [0u8; HEADER_SIZE];
        let (signature, word8, version) = match self.save_family() {
            SaveFamily::Cmvs => (*b"CSV2", HEADER_WORD_8, 4u16),
            SaveFamily::Cvns => (*b"CSP1", 0u32, 3u16),
        };
        header[..4].copy_from_slice(&signature);
        header[4..8].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        header[8..12].copy_from_slice(&word8.to_le_bytes());
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
        header[544..546].copy_from_slice(&version.to_le_bytes());
        let thumbnail = self.platform.thumbnail().is_some_and(|t| t.enabled);
        header[548..550].copy_from_slice(&u16::from(thumbnail).to_le_bytes());
        Ok(header)
    }

    fn capture_cvns_snapshot(
        &self,
        vm: crate::machine::VmImage,
    ) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error + Send + Sync>> {
        let starts = self.bootstrap.cvns_persistent_starts();
        let variables = CvnsLocalVariables::capture(self.machine.state(), starts)?;
        let clear = self.scene.clear();
        let default_scene = crate::scene::cvns::CvnsScene::default();
        let cvns_scene = self.scene.cvns().unwrap_or(&default_scene);
        let (_, popup) = self.save_menu.saved_flags();
        let snapshot = CvnsSnapshot::from_scene_and_messages(
            crate::snapshot::cvns::CvnsSceneCapture {
                scene: cvns_scene,
                msg_host: self.messages.cvns(),
                texts: self.messages.saved_texts(),
                clear_color: clear.color,
                clear_depth: clear.depth,
                backdrop_disabled: self.scene.cvns_backdrop_disabled(),
                layer_fade_alpha: self.scene.cvns_layer_fade_alpha(),
            },
            crate::snapshot::cvns::CvnsVmCapture {
                save_title: self.save_title.bytes().to_vec(),
                scene_title: self.platform.scene_title().to_vec(),
                script_names: vm.script_names,
                script_sizes: vm.script_sizes,
                active_slot: vm.active_slot,
                pc: vm.pc,
                repeat_count: vm.repeat_count,
                timers: *self.timers.values(),
                popup,
                callbacks: self.control.saved_callbacks(),
                stack_pointer: vm.stack_pointer,
                frame_depth: vm.frame_depth,
                frames: vm.frames,
                stack: vm.stack,
                system_results: vm.system_results,
                script_data: vm.main_data,
                variables,
                audio: self.audio.saved_cvns(),
                history: self.history.saved(),
            },
        )?;
        Ok((snapshot.encode()?, vm.main_image))
    }

    /// 0x2080 / 0x22b2: the image is built before any live change, so a state without a
    /// record format leaves the VM, bit 0 and the held image as they were.
    pub(super) fn capture_snapshot(
        &mut self,
        vm: crate::machine::VmImage,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let layout = self.require_snapshot_layout()?;
        let (bytes, script_image) = if layout == SnapshotLayout::CvnsCsp1 {
            self.capture_cvns_snapshot(vm)?
        } else {
            let (snapshot, script_image) = self.engine_snapshot_with(vm)?;
            (snapshot.encode_with_layout(layout)?, script_image)
        };
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

    /// The directory and backend gameplay slots are written to: only an independent user directory.
    pub(super) fn slot_target(
        &self,
    ) -> Result<(std::path::PathBuf, Arc<dyn HostFs>), Box<dyn Error + Send + Sync>> {
        match &self.system_output {
            Some(SystemOutput::Profile(profile)) => {
                Ok((profile.directory().to_path_buf(), Arc::clone(profile.fs())))
            }
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
    fn write_slot(
        &mut self,
        slot: u16,
        before_load: bool,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        if slot == 0 && self.save_family() == SaveFamily::Cvns {
            return Ok(false);
        }
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
        let (directory, slot_fs) = self.slot_target()?;
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
        let file = Csv2Save::encode_for(
            self.save_family(),
            &header,
            &captured.snapshot,
            &thumbnail,
            &captured.script_image,
        )?;
        // 0x140068b50: keep the previous version of slots below 950 as slot 999.
        if slot < 950
            && let Some(previous) = self.saves.read_slot(slot)?
        {
            replace_file(
                &*slot_fs,
                &directory,
                &format!("save{BACKUP_SLOT:03}.dat"),
                &previous,
            )?;
        }
        // 0x1400179e0: the system save is written with every slot.
        let id = self.system_save_identifier();
        let mut system_path = None;
        if let Some(output) = &mut self.system_output {
            self.system_host
                .write_now(self.machine.state(), output, &mut self.random, id)?;
            system_path = Some((output.path(), Arc::clone(output.fs())));
        }
        if let Some((path, fs)) = system_path {
            self.system_source = self.open_system_source(path, fs);
        }
        replace_file(&*slot_fs, &directory, &format!("save{slot:03}.dat"), &file)?;
        Ok(true)
    }

    /// The write 0x22b6 performs, from any point between VM steps. False when no snapshot
    /// has been taken yet or the slot is reserved for the CVNS system save.
    pub fn save_slot(&mut self, slot: u16) -> Result<bool, Box<dyn Error + Send + Sync>> {
        if slot > 999 {
            return Err("save slot outside 0..999".into());
        }
        self.write_slot(slot, false)
    }

    /// MIRANOSU 0x22be -> 0x140070780: change only CSV header words 532/536.
    /// Missing or unrecognized slots report zero; I/O failures leave the VM pending.
    pub(super) fn handle_set_slot_info(
        &mut self,
        request: &RuntimeCall,
        slot: u32,
        values: [u32; 2],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let changed = if slot > 999 {
            false
        } else {
            let (directory, slot_fs) = self.slot_target()?;
            let name = format!("save{slot:03}.dat");
            match self.saves.read_slot(slot as u16)? {
                None => false,
                Some(mut bytes) => {
                    let recognized = bytes.get(..3) == Some(b"CSV")
                        && bytes.get(3).is_some_and(|v| (b'2'..=b'9').contains(v));
                    if !recognized {
                        false
                    } else {
                        if bytes.len() < HEADER_SIZE {
                            return Err(
                                "slot metadata needs a complete 600-byte save header".into()
                            );
                        }
                        bytes[532..536].copy_from_slice(&values[0].to_le_bytes());
                        bytes[536..540].copy_from_slice(&values[1].to_le_bytes());
                        replace_file(&*slot_fs, &directory, &name, &bytes)?;
                        true
                    }
                }
            }
        };
        request.complete(&mut self.machine, 0x400c)?;
        self.machine
            .state_mut()
            .set_system_integer(0, u32::from(changed))?;
        Ok(())
    }

    /// 0x22b6 / 0x1400812f0 (and CVNS 0x21a4 / 0x467bf0): SP-4 slot; slots above 999
    /// (and slot 0 / <= 0 in CVNS) are ignored. Returns 0x4004.
    pub(super) fn handle_save_slot(
        &mut self,
        request: &RuntimeCall,
        slot: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if slot <= 999 {
            self.write_slot(slot as u16, false)?;
        }
        request.complete(&mut self.machine, 0x4004)?;
        Ok(())
    }

    /// 0x22b5 / 0x140082340 (and CVNS 0x21a3 / 0x467b80): SP-4 slot. The load replaces the VM,
    /// so the result flags are applied by hand: 0x2000 (repeat + 1) when the restored repeat
    /// count is nonzero, else 0 (repeat reset, nothing popped, the restored PC runs next).
    pub(super) fn handle_load_slot(
        &mut self,
        request: &RuntimeCall,
        slot: u32,
        backend: &mut impl PlatformBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if slot > 999 || (self.save_family() == SaveFamily::Cvns && slot == 0) {
            request.complete(&mut self.machine, 0x4004)?;
            return Ok(());
        }
        self.load_slot(slot as u16, backend)?;
        let repeat = self.machine.repeat_count();
        self.machine
            .set_repeat_count(if repeat != 0 { repeat + 1 } else { 0 });
        Ok(())
    }

    /// The load 0x22b5 performs, from any point between VM steps: the slot replaces the VM
    /// and every saved host, so where the script stood before does not matter. Slots below
    /// 950 first save the current snapshot to slot 998 when one exists (0x1400818e0).
    pub fn load_slot(
        &mut self,
        slot: u16,
        backend: &mut impl PlatformBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let layout = self.require_snapshot_layout()?;
        if self.scene.has_effect_pools()
            && !effect_state_supported(self.gate.profile(), &self.scene)
        {
            return Err("loading over active particle effects is not implemented".into());
        }
        if self.slot_edit.is_some() {
            return Err("finish the pending slot edit before loading a gameplay save".into());
        }
        let bytes = self
            .saves
            .read_slot(slot)?
            .ok_or("save slot to load does not exist")?;
        let family = self.save_family();
        let save = Csv2Save::decode_family(&bytes, family)?;
        if layout == SnapshotLayout::CvnsCsp1 {
            let snapshot = if save.snapshot.starts_with(&0x100u16.to_le_bytes()) {
                if self.gate.profile().inspected_build()
                    != Some(crate::profile::InspectedBuild::Cvns20090203)
                    || &bytes[..4] != b"CSP1"
                    || bytes[544..546] != 3u16.to_le_bytes()
                {
                    return Err("unsupported original CVNS save build or container version".into());
                }
                let snapshot = native::decode(
                    &save,
                    self.bootstrap.cvns_persistent_starts(),
                    self.messages
                        .cvns()
                        .ok_or("CVNS message host is unavailable")?,
                    self.messages.screen_size().map(|value| value as i32),
                )?;
                let common_size = self.machine.loaded_script(1).map_or(0, |script| {
                    crate::ps3::HEADER_SIZE + script.header().decoded_size as usize
                });
                if snapshot.script_sizes[1] as usize != common_size {
                    return Err("original CVNS save uses a different common script size".into());
                }
                snapshot
            } else {
                CvnsSnapshot::decode(&save.snapshot)?
            };
            return self.restore_cvns_snapshot(
                slot,
                &save,
                &bytes[..HEADER_SIZE],
                &snapshot,
                backend,
            );
        }
        let snapshot = EngineSnapshot::decode_with_layout(&save.snapshot, layout)?;
        self.validate_build_snapshot(&snapshot, layout)?;
        self.restore_snapshot(
            slot,
            &save,
            &bytes[..HEADER_SIZE],
            &snapshot,
            layout,
            backend,
        )
    }

    fn restore_cvns_snapshot(
        &mut self,
        slot: u16,
        save: &Csv2Save,
        header: &[u8],
        snapshot: &CvnsSnapshot,
        backend: &mut impl PlatformBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let audio = SavedCvnsAudio::decode(&snapshot.audio)?;
        let mut control = self.control.clone();
        control.restore_callbacks(&snapshot.callbacks)?;
        let callback = control
            .callbacks()
            .get(&LOAD_CALLBACK)
            .filter(|c| c.enabled)
            .map(|c| (c.script_slot, c.target as usize));
        let image = crate::machine::VmImage {
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
        let mut vm = self.machine.prepare_restore(&image, callback)?;
        snapshot
            .variables
            .apply_to_state(&mut vm.state, self.bootstrap.cvns_persistent_starts())?;
        let mut budget = crate::snapshot::RESTORE_PIXELS;
        let mut image_src = self.images.with_paths(self.bootstrap.paths());
        let mut scene_images = vec![None; crate::scene::cvns::CVNS_IMAGES];
        for (&idx, saved_buf) in &snapshot.images {
            scene_images[usize::from(idx)] = Some(saved_buf.load(&mut image_src, &mut budget)?);
        }
        let mask_transition = snapshot
            .mask_transition
            .as_ref()
            .map(|m| m.restore(&mut budget))
            .transpose()?;
        let mut scene_sprites = vec![None; crate::scene::cvns::CVNS_SPRITES];
        for (&idx, sp) in &snapshot.sprites {
            scene_sprites[usize::from(idx)] = Some(sp.clone());
        }
        let cvns_scene = crate::scene::cvns::CvnsScene {
            images: scene_images,
            sprites: scene_sprites,
            fade: snapshot.fade,
            mask_transition,
            quake: snapshot.quake,
            cg_seen: snapshot.cg_seen.clone(),
        };
        let mut cvns_windows: [Option<crate::message::cvns::CvnsWindow>;
            crate::message::cvns::CVNS_WINDOWS] = Default::default();
        for (&idx, win) in &snapshot.windows {
            cvns_windows[usize::from(idx)] = Some(win.clone());
        }
        let mut cvns_messages: [Option<crate::message::cvns::CvnsMessage>;
            crate::message::cvns::CVNS_MESSAGES] = Default::default();
        for (&idx, msg) in &snapshot.messages {
            cvns_messages[usize::from(idx)] = Some(msg.load(&mut image_src, &mut budget)?);
        }
        let cvns_msg_host = crate::message::cvns::CvnsMessageHost {
            windows: cvns_windows,
            messages: cvns_messages,
            text_shadow: snapshot.text_shadow,
            read_skip: snapshot.read_skip,
            auto_skip: snapshot.auto_skip,
            auto_wait_elapsed_ms: snapshot.auto_wait_elapsed_ms,
            click_wait_elapsed_ms: snapshot.click_wait_elapsed_ms,
        };
        let mut texts: [Option<crate::message::TextObject>; TEXT_OBJECTS] = Default::default();
        for (&idx, obj) in &snapshot.texts {
            texts[usize::from(idx)] = Some(obj.clone());
        }
        let mut history = self.history.clone();
        history.restore(&snapshot.history)?;
        self.platform
            .restored_title(&snapshot.scene_title, self.machine.text_encoding())?;
        if slot < 950
            && matches!(self.save_image, SaveImage::Captured(_))
            && matches!(self.system_output, Some(SystemOutput::Profile(_)))
        {
            self.write_slot(LOAD_BACKUP_SLOT, true)?;
        }
        self.machine.install_restore(vm);
        self.discard_dialog(backend);
        self.history = history;
        self.sequence = Default::default();
        self.camera = Default::default();
        self.control = control;
        self.timers.restore_values(snapshot.timers);
        self.save_title.restore(snapshot.save_title.clone());
        self.save_menu.restore_flags(snapshot.popup);
        self.message_flags.clear_for_load();
        self.buttons = Default::default();
        if self.movies.restore(Default::default()) {
            self.audio_backend.stop_movie_audio()?;
        }
        self.scene.restore_cvns(
            ClearState {
                color: snapshot.clear_color,
                depth: snapshot.clear_depth,
            },
            snapshot.backdrop_disabled,
            snapshot.layer_fade_alpha,
            cvns_scene,
        );
        self.messages.restore_cvns(texts, cvns_msg_host);
        let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
        self.save_info.restore([word(528), word(532), word(536)]);
        if !save.auxiliary.is_empty() {
            self.thumbnail = Some(save.auxiliary.clone());
        }
        self.save_image = SaveImage::Captured(Box::new(CapturedSave {
            header: header.try_into().expect("600-byte header"),
            snapshot: save.snapshot.clone(),
            script_image: save.script_image.clone(),
        }));
        self.audio
            .restore_cvns(audio, self.audio_backend.as_mut())?;
        self.platform.restore_scene_title(
            snapshot.scene_title.clone(),
            self.machine.text_encoding(),
            backend,
        )?;
        Ok(())
    }

    /// Loader 0x140081960 and reader 0x140069fd0 over the Rust hosts. Everything fallible
    /// that does not touch a device (decoding, script layout, images) runs before the first
    /// host changes; audio and window-title requests then go to the backends.
    fn restore_snapshot(
        &mut self,
        slot: u16,
        save: &Csv2Save,
        header: &[u8],
        snapshot: &EngineSnapshot,
        layout: SnapshotLayout,
        backend: &mut impl PlatformBackend,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if snapshot.word_11144 != 1 {
            return Err("saved +11144 value is not modeled".into());
        }
        let quake = snapshot.effects[0]
            .as_deref()
            .map(crate::scene::Quake::restore)
            .transpose()?;
        if quake.as_ref().is_some_and(|q| q.mode & 0xf00 == 0x100)
            && !self.scene.config.allow_triangle_quake
        {
            return Err("triangle quake restore requires the inspected build".into());
        }
        let fade = snapshot.effects[1]
            .as_deref()
            .map(crate::fade::Fade::restore)
            .transpose()?;
        let strip = match snapshot.effects[3].as_deref() {
            Some(record) => crate::scene::StripWipe::restore(record, self.screen_size())?,
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
            .any(|&(tag, _)| !matches!(tag, 0x6a0 | 0x800 | 0x540 | 0x8c0 | 0x8c8 | 0x8e8))
        {
            return Err("saved camera-sequence objects are not implemented".into());
        }
        if snapshot.sound_device != [1, 0, 0, 0, 0, 0] {
            return Err("saved sound device buffers are not implemented".into());
        }
        let audio = SavedAudio::decode(&snapshot.bgm, &snapshot.sound_channels)?;
        let mut control = self.control.clone();
        control.restore_callbacks(&snapshot.callbacks)?;
        let callback = control
            .callbacks()
            .get(&LOAD_CALLBACK)
            .filter(|c| c.enabled)
            .map(|c| (c.script_slot, c.target as usize));
        let image = crate::machine::VmImage {
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
        let mut vm = self.machine.prepare_restore(&image, callback)?;
        snapshot.variables.apply_to_state(&mut vm.state)?;
        if slot < 950 && matches!(self.save_image, SaveImage::Captured(_)) {
            self.write_slot(LOAD_BACKUP_SLOT, true)?;
        }
        // Candidate hosts first: images, windows and texts must all rebuild.
        let mut budget = crate::snapshot::RESTORE_PIXELS;
        let mut images = self.images.with_paths(self.bootstrap.paths());
        let effect_records: Vec<_> = snapshot
            .raw_objects
            .iter()
            .filter(|((tag, _), _)| *tag == 0x800)
            .map(|(&(_, slot), body)| (slot, body.as_slice()))
            .collect();
        if effect_records
            .iter()
            .any(|(_, body)| !effect_record_supported(self.gate.profile(), body))
        {
            return Err("saved particle record layout is not implemented for this profile".into());
        }
        let mut restored_random = self.random.clone();
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
        // Native 0x4179f0 discards the bodyless old scene effects on load.
        let effect_records: Vec<_> = effect_records
            .into_iter()
            .filter(|(_, body)| {
                !(layout == SnapshotLayout::TextV1SceneLegacy && body.len() == 4 && body[2] >= 0x50)
            })
            .collect();
        let effects = crate::scene::SceneHost::load_effect_pools(
            &effect_records,
            &mut images,
            &mut restored_random,
            &mut budget,
            matches!(
                self.gate.profile().inspected_build(),
                Some(
                    crate::profile::InspectedBuild::Cmvs20080719
                        | crate::profile::InspectedBuild::Cmvs20090815
                        | crate::profile::InspectedBuild::Cmvs20100314
                )
            ),
        )?;
        // Records 0x8e8 follow 0x700 and bind to the objects just rebuilt.
        let movie_records: Vec<_> = snapshot
            .raw_objects
            .iter()
            .filter(|((tag, _), _)| *tag == 0x8e8)
            .map(|(&(_, slot), body)| (slot, body.as_slice()))
            .collect();
        let players = self.movies.load_scene_players_with_layout(
            &movie_records,
            &mut objects,
            &mut self.scene_movies.with_paths(
                self.bootstrap.paths(),
                crate::resources::ResourceKind::SceneMovie,
            ),
            layout,
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
        buttons.restore_with_layout(&records(0x8c0), &records(0x8c8), layout)?;
        let mut texts: [Option<_>; TEXT_OBJECTS] = snapshot.texts.clone();
        if layout.text_version() == 1 {
            for text in texts.iter_mut().flatten() {
                text.logical_size = self.messages.screen_size().map(|v| v as i32);
            }
        }
        let mut history = self.history.clone();
        history.restore(&snapshot.raw_objects[&(0x540, 0)])?;
        let mut sequence = self.sequence.clone();
        if layout.old_tables() {
            sequence = crate::sequence::SequenceHost::default();
        } else {
            sequence.restore(&snapshot.record_6e0)?;
        }
        let mut camera_host = self.camera.clone();
        camera_host.restore(0, &snapshot.cameras[0])?;
        if layout != SnapshotLayout::TextV1SceneLegacy {
            camera_host.restore(1, &snapshot.cameras[1])?;
        }
        camera_host.restore_view(0, &snapshot.cameras[2])?;
        if layout != SnapshotLayout::TextV1SceneLegacy {
            camera_host.restore_view(1, &snapshot.cameras[3])?;
        }
        self.platform
            .restored_title(&snapshot.scene_title, self.machine.text_encoding())?;
        self.machine.install_restore(vm);
        // Commit the hosts.
        self.discard_dialog(backend);
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
        self.scene.legacy_flakes = effects.legacy_flakes;
        self.scene.rain_effects = effects.rain_effects;
        self.scene.falling_effects = effects.falling_effects;
        self.scene.particle_effects = effects.particle_effects;
        self.scene.collision_effects = effects.collision_effects;
        // Legacy slots are never saved (saving them is rejected), so a load destroys
        // them, releasing their prepared sounds.
        self.scene
            .release_legacy_effects(self.audio_backend.as_mut())?;
        self.random = restored_random;
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
        self.audio.restore(audio, self.audio_backend.as_mut())?;
        self.platform.restore_scene_title(
            snapshot.scene_title.clone(),
            self.machine.text_encoding(),
            backend,
        )?;
        Ok(())
    }

    /// 0x22b8: 0x14006b4a0 builds the thumbnail from the screen capture when one exists,
    /// otherwise from the frame; returns 0x4000. Text glyphs drawn by the desktop font
    /// backend are not part of the core frame.
    pub(super) fn handle_thumbnail(
        &mut self,
        request: &RuntimeCall,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let config = self
            .platform
            .thumbnail()
            .ok_or("thumbnail size (0x22b0) was never configured")?;
        let frame = match self.capture.image() {
            Some(image) => image.clone(),
            None => self.render_frame()?,
        };
        let bmp = thumbnail_bmp(&frame, config.width, config.height);
        request.complete(&mut self.machine, 0x4000)?;
        self.thumbnail = Some(bmp);
        Ok(())
    }

    /// 0x22b7 / 0x14007caf0: SP-4 first slot, SP-8 count. System result 0 is the slot in
    /// range whose label orders latest (0x140093490), or -1; out of range also gives -1.
    pub(super) fn handle_latest_slot(
        &mut self,
        request: &RuntimeCall,
        start: i32,
        count: i32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let family = self.save_family();
        let mut latest = u32::MAX;
        let in_range = match family {
            SaveFamily::Cmvs => start >= 0 && count.wrapping_add(start) <= 999,
            SaveFamily::Cvns => start > 0 && count >= 0 && start.saturating_add(count) <= 257,
        };
        if in_range {
            let mut best = 0;
            for slot in start..start.wrapping_add(count) {
                if let Some(bytes) =
                    crate::save_menu::SaveSource::read_header(&mut self.saves, slot as u16)?
                    && let Ok(header) = SaveHeader::parse_family(&bytes, family)
                    && header.ordering_key > best
                {
                    best = header.ordering_key;
                    latest = slot as u32;
                }
            }
        }
        request.complete(&mut self.machine, 0x4008)?;
        self.machine
            .state_mut()
            .set_system_integer(0, latest)
            .expect("slot 0 exists");
        Ok(())
    }

    /// Old 0x22bc / CVNS 0x25ab (0x21ab) patches only the 256-byte comment field in the 600-byte header.
    pub(super) fn handle_set_slot_comment(
        &mut self,
        request: &RuntimeCall,
        slot: u32,
        text: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let comment = self.machine.resolve_string(text)?;
        if comment.len() >= 256 {
            return Err(error(
                request.pc(),
                request.command() + 0x2000,
                "save comment exceeds native capacity",
            )
            .into());
        }
        let family = self.save_family();
        let mut changed = false;
        if slot <= 999
            && (family != SaveFamily::Cvns || slot != 0)
            && let Some(mut bytes) = self.saves.read_slot(slot as u16)?
            && bytes.len() >= HEADER_SIZE
            && family.accepts(&bytes)
        {
            bytes[272..272 + comment.len()].copy_from_slice(&comment);
            bytes[272 + comment.len()] = 0;
            let (directory, slot_fs) = self.slot_target()?;
            replace_file(&*slot_fs, &directory, &format!("save{slot:03}.dat"), &bytes)?;
            changed = true;
        }
        request.complete(&mut self.machine, 0x4008)?;
        self.machine
            .state_mut()
            .set_system_integer(0, u32::from(changed))
            .expect("slot zero exists");
        Ok(())
    }

    /// CVNS 0x21ac (0x5ac / 0x467ff0): SP-4 slot, SP-8 CVNS image slot (0..128), SP-12/SP-16
    /// string destinations for label (+16) and comment (+272). Loads BMP thumbnail directly into
    /// `scene.cvns_mut().images[img_slot]`. Returns 0x4010.
    pub(super) fn handle_cvns_slot_info(
        &mut self,
        request: &RuntimeCall,
        slot: i32,
        img_slot: u32,
        label_slot: u32,
        text_slot: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let pc = request.pc();
        let fail = |why: &'static str| error(pc, 0x21ac, why);
        if slot <= 0 || slot > 999 || img_slot >= crate::scene::cvns::CVNS_IMAGES as u32 {
            request.complete(&mut self.machine, 0x4010)?;
            self.machine
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0 exists");
            return Ok(());
        }
        let Some(bytes) = self.saves.read_slot(slot as u16)? else {
            request.complete(&mut self.machine, 0x4010)?;
            self.machine
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0 exists");
            return Ok(());
        };
        if bytes.len() < HEADER_SIZE || !SaveFamily::Cvns.accepts(&bytes) {
            request.complete(&mut self.machine, 0x4010)?;
            self.machine
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0 exists");
            return Ok(());
        }
        let (header, auxiliary) = Csv2Save::decode_auxiliary(&bytes, SaveFamily::Cvns)?;
        let c_string = |field: &[u8]| {
            field[..field.iter().position(|&b| b == 0).unwrap_or(field.len())].to_vec()
        };
        let label = c_string(&bytes[16..272]);
        let text = c_string(&bytes[272..528]);
        if label.len() >= 1024 || text.len() >= 1024 {
            return Err(fail("save label overflows its string slot").into());
        }
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let image = if u16::from_le_bytes([bytes[548], bytes[549]]) != 0 {
            if auxiliary.is_empty() {
                None
            } else {
                let pixels = decode_bmp(&auxiliary)?;
                let mut layer = crate::scene::cvns::CvnsImageLayer::from_image(
                    Vec::new(),
                    Vec::new(),
                    pixels,
                    0,
                    0,
                    false,
                );
                layer.captured = true;
                Some(crate::scene::cvns::CvnsImageBuffer::with_base(layer))
            }
        } else {
            None
        };
        request.complete(&mut self.machine, 0x4010)?;
        if let Some(buf) = image {
            self.scene.cvns_mut().images[img_slot as usize] = Some(buf);
        }
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
        Ok(())
    }

    /// 0x22bd / 0x14007cf10: SP-4 slot, SP-8 object, SP-12 child (negative = the object
    /// itself, recreated), SP-16/SP-20 string destinations for the label (+16) and the
    /// second text (+272). Results 0 = found, 1 = label order key, 4..6 = header 528..536;
    /// the thumbnail becomes the target image (a blank opaque black one when the header
    /// enables thumbnails but the file has none). Returns 0x4014.
    pub(super) fn handle_slot_info(
        &mut self,
        request: &RuntimeCall,
        slot: i32,
        object: u32,
        child: i32,
        label_slot: u32,
        text_slot: u32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let pc = request.pc();
        let fail = |why: &'static str| error(pc, 0x22bd, why);
        if slot < 0 || object > 255 || slot > 999 {
            request.complete(&mut self.machine, 0x4014)?;
            self.machine
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0 exists");
            return Ok(());
        }
        let Some(bytes) = self.saves.read_slot(slot as u16)? else {
            request.complete(&mut self.machine, 0x4014)?;
            self.machine
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0 exists");
            return Ok(());
        };
        if bytes.len() < HEADER_SIZE || &bytes[..3] != b"CSV" || !(b'2'..=b'9').contains(&bytes[3])
        {
            request.complete(&mut self.machine, 0x4014)?;
            self.machine
                .state_mut()
                .set_system_integer(0, 0)
                .expect("slot 0 exists");
            return Ok(());
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
                crate::save::Csv2Layout::parse(&bytes, bytes.len() as u64)?;
                let (_, auxiliary) = Csv2Save::decode_auxiliary(&bytes, SaveFamily::Cmvs)?;
                Some(ObjectImage {
                    filter: None,
                    name: Vec::new(),
                    pixels: decode_bmp(&auxiliary)?,
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
        request.complete(&mut self.machine, 0x4014)?;
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
        Ok(())
    }
}

/// Replaces a slot file through a fresh temporary file in the same directory.
fn replace_file(
    fs: &dyn HostFs,
    directory: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let temporary = directory.join(format!(".{name}.tmp"));
    let _ = fs.remove_file(&temporary);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open_with(fs, &temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(e) = fs.replace(&temporary, &directory.join(name)) {
        let _ = fs.remove_file(&temporary);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn unix_seconds_map_to_utc_civil_fields() {
        assert_eq!(save_time_from_unix(0), [1970, 1, 1, 0, 0, 0]);
        // Leap day and the last second of a century leap year.
        assert_eq!(save_time_from_unix(951_782_400), [2000, 2, 29, 0, 0, 0]);
        assert_eq!(save_time_from_unix(978_307_199), [2000, 12, 31, 23, 59, 59]);
        assert_eq!(save_time_from_unix(1_793_912_645), [2026, 11, 5, 21, 4, 5]);
    }

    #[test]
    fn snapshot_layout_requires_an_independently_inspected_build() {
        let registry = crate::profile::Registry::builtin();
        for (id, layout) in [
            ("cmvs-20070119", SnapshotLayout::TextV1SceneLegacy),
            ("cmvs-20071117", SnapshotLayout::TextV1SceneV1),
            ("cmvs-20080719", SnapshotLayout::TextV1SceneV2),
            ("cmvs-20080903", SnapshotLayout::TextV1SceneV2),
            ("cmvs-20090815", SnapshotLayout::TextV1SceneV2Collision),
            ("cmvs-20100314", SnapshotLayout::TextV2WindowV2),
            ("cmvs-20101211", SnapshotLayout::TextV2),
            ("cmvs-20110525", SnapshotLayout::TextV2),
            ("cmvs-20120309", SnapshotLayout::TextV2),
            ("cmvs-20130131", SnapshotLayout::TextV2),
            ("cmvs-20140217", SnapshotLayout::TextV2),
            ("cmvs-20150405", SnapshotLayout::TextV2),
            ("cmvs-20171114", SnapshotLayout::TextV3),
            ("cmvs-20190305", SnapshotLayout::TextV3SceneV4),
            ("cmvs-20190405", SnapshotLayout::TextV3SceneV4),
            ("cmvs-20211006", SnapshotLayout::TextV3SceneV4),
            ("cmvs-20221105", SnapshotLayout::TextV3SceneV4),
            ("cmvs-20160201", SnapshotLayout::TextV3),
            ("cvns-20040515", SnapshotLayout::CvnsCsp1),
            ("cvns-20040730", SnapshotLayout::CvnsCsp1),
            ("cvns-20050216", SnapshotLayout::CvnsCsp1),
            ("cvns-20050727", SnapshotLayout::CvnsCsp1),
            ("cvns-20090203", SnapshotLayout::CvnsCsp1),
        ] {
            assert_eq!(snapshot_layout(registry.profile(id).unwrap()), Some(layout));
            assert_eq!(
                effect_layout_supported(registry.profile(id).unwrap()),
                matches!(
                    id,
                    "cmvs-20090815"
                        | "cmvs-20100314"
                        | "cmvs-20080719"
                        | "cmvs-20080903"
                        | "cmvs-20110525"
                        | "cmvs-20120309"
                        | "cmvs-20171114"
                        | "cmvs-20160201"
                        | "cmvs-20211006"
                        | "cmvs-20221105"
                )
            );
        }
        let mut profile = registry.profile("cmvs-20171114").unwrap().clone();
        profile.id = "uninspected-derived".into();
        assert_eq!(snapshot_layout(&profile), None);
        assert!(!effect_layout_supported(&profile));
        profile.id = "cmvs-20171114".into();
        profile.base = Some("uninspected-base".into());
        assert_eq!(snapshot_layout(&profile), None);
        assert!(!effect_layout_supported(&profile));
    }

    #[test]
    fn restricted_effect_layout_only_accepts_the_inspected_type_five() {
        let registry = crate::profile::Registry::builtin();
        for id in [
            "cmvs-20080719",
            "cmvs-20080903",
            "cmvs-20110525",
            "cmvs-20120309",
            "cmvs-20211006",
            "cmvs-20221105",
        ] {
            let profile = registry.profile(id).unwrap();
            for kind in [2, 4, 5, 6, 8] {
                assert_eq!(
                    effect_record_supported(profile, &[1, 0, kind, 0]),
                    (id != "cmvs-20080903" && kind == 5)
                        || (id == "cmvs-20221105" && kind == 8)
                        || (id == "cmvs-20080903" && kind == 4)
                );
            }
            assert!(!effect_record_supported(profile, &[1, 0]));
            assert!(!effect_record_supported(profile, &[2, 0, 5, 0]));
            assert!(!effect_record_supported(profile, &[1, 0, 8, 0, 0]));
        }
        for id in [
            "cmvs-20070119",
            "cmvs-20071117",
            "cmvs-20080719",
            "cmvs-20080903",
            "cmvs-20101211",
            "cmvs-20120309",
            "cmvs-20211006",
            "cmvs-20221105",
        ] {
            let mut profile = registry.profile(id).unwrap().clone();
            for binary in &mut profile.binaries {
                binary.sha256 = [0; 32];
            }
            assert_eq!(snapshot_layout(&profile), None);
            assert!(!effect_record_supported(&profile, &[1, 0, 5, 0]));
        }
    }

    #[test]
    fn snapshot_child_capacity_rejects_button_lists_and_out_of_range_nested_children() {
        for capacity in [256, 512] {
            let leaf = SavedScene::from_object(&SceneObject::default()).unwrap();
            let mut parent = leaf.clone();
            parent.children.insert(capacity - 1, leaf.clone());
            assert!(
                validate_child_slots_without_button_lists(
                    std::iter::once(&parent),
                    false,
                    capacity
                )
                .is_ok()
            );
            assert!(
                validate_child_slots_without_button_lists(std::iter::once(&parent), true, capacity)
                    .is_err()
            );
            let mut nested = leaf.clone();
            nested.children.insert(capacity, leaf);
            parent.children.insert(0, nested);
            assert!(
                validate_child_slots_without_button_lists(
                    std::iter::once(&parent),
                    false,
                    capacity
                )
                .is_err()
            );
        }
    }

    #[test]
    fn memoria_collision_template_requires_exact_length_and_identity() {
        let mut profile = crate::profile::Registry::builtin()
            .profile("cmvs-20090815")
            .unwrap()
            .clone();
        let mut body = vec![0; 52];
        body[..4].copy_from_slice(&[1, 0, 6, 0]);
        assert!(effect_record_supported(&profile, &body));
        assert!(!effect_record_supported(&profile, &body[..51]));
        body.push(0);
        assert!(!effect_record_supported(&profile, &body));
        body.pop();
        profile.binaries[0].sha256 = [0; 32];
        assert!(!effect_record_supported(&profile, &body));
        assert_eq!(snapshot_layout(&profile), None);
    }

    #[test]
    fn legacy_flakes_records_require_the_inspected_identity_and_exact_length() {
        let registry = crate::profile::Registry::builtin();
        let mut body = vec![0; 52];
        body[..4].copy_from_slice(&[1, 0, 3, 0]);
        for profile in registry.profiles() {
            assert_eq!(
                effect_record_supported(profile, &body),
                profile.id == "cmvs-20070119",
                "{}",
                profile.id
            );
        }
        let mut profile = registry.profile("cmvs-20070119").unwrap().clone();
        assert!(!effect_record_supported(&profile, &body[..51]));
        body.push(0);
        assert!(!effect_record_supported(&profile, &body));
        body.pop();
        for binary in &mut profile.binaries {
            binary.sha256 = [0; 32];
        }
        assert!(!effect_record_supported(&profile, &body));
    }

    #[test]
    fn base_effect_records_require_identity_and_only_accept_falling_and_fragments() {
        let registry = crate::profile::Registry::builtin();
        let base = registry.profile("cmvs-20160201").unwrap();
        assert!(effect_record_supported(base, &[1, 0, 8, 0]));
        assert!(effect_record_supported(base, &[1, 0, 2, 0]));
        assert!(!effect_record_supported(base, &[1, 0, 5, 0]));
        assert!(!effect_record_supported(base, &[1, 0, 8, 0, 0]));
        let mut changed = base.clone();
        for binary in &mut changed.binaries {
            binary.sha256 = [0; 32];
        }
        assert!(!effect_layout_supported(&changed));
        assert!(!effect_record_supported(&changed, &[1, 0, 8, 0]));
    }

    #[test]
    fn csp1_container_encode_decode_and_menu_semantics() {
        let mut header = [0u8; HEADER_SIZE];
        header[..4].copy_from_slice(b"CSP1");
        header[4..8].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        header[16..37].copy_from_slice(b"2026-11-16 12:00:00 A");
        header[272..276].copy_from_slice(b"Note");
        header[528..532].copy_from_slice(&11u32.to_le_bytes());
        header[532..536].copy_from_slice(&22u32.to_le_bytes());
        header[536..540].copy_from_slice(&33u32.to_le_bytes());
        header[544..546].copy_from_slice(&3u16.to_le_bytes());
        header[548..550].copy_from_slice(&1u16.to_le_bytes());

        let bytes = Csv2Save::encode_for(
            SaveFamily::Cvns,
            &header,
            b"snap-body",
            b"BMthumb",
            b"script-img",
        )
        .unwrap();
        assert_eq!(&bytes[..4], b"CSP1");
        assert!(SaveFamily::Cvns.accepts(&bytes));
        assert!(!SaveFamily::Cmvs.accepts(&bytes));
        assert!(Csv2Save::decode_family(&bytes, SaveFamily::Cmvs).is_err());

        let decoded = Csv2Save::decode_family(&bytes, SaveFamily::Cvns).unwrap();
        assert_eq!(decoded.snapshot, b"snap-body");
        assert_eq!(decoded.auxiliary, b"BMthumb");
        assert_eq!(decoded.script_image, b"script-img");
        let (aux_hdr, aux_bytes) = Csv2Save::decode_auxiliary(&bytes, SaveFamily::Cvns).unwrap();
        assert_eq!(aux_hdr.version, b'1');
        assert_eq!(aux_bytes, b"BMthumb");

        struct MapSaves(std::collections::BTreeMap<u16, Vec<u8>>);
        impl crate::save_menu::SaveSource for MapSaves {
            fn read_header(&mut self, slot: u16) -> std::io::Result<Option<Vec<u8>>> {
                Ok(self.0.get(&slot).cloned())
            }
        }
        let mut files = MapSaves(std::collections::BTreeMap::from([(1, bytes.clone())]));
        let menu = crate::save_menu::SaveMenu::scan_cvns(&mut files, 3).unwrap();
        assert_eq!(menu.slots.len(), 3);
        assert!(matches!(
            menu.slots[0],
            crate::save_menu::SlotStatus::Recognized(_)
        ));
        assert_eq!(menu.latest, Some(0));

        // 0x2bb on CVNS must reject slot 0 (reserved for CSS0 save000.dat) and accept slot 1.
        let mut host = crate::save_menu::SaveMenuHost::for_profile(
            crate::profile::Registry::builtin()
                .profile("cvns-20090203")
                .unwrap(),
        );
        let mut code = Vec::new();
        for arg in [0u32, 1u32] {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(arg.to_le_bytes());
            code.extend(0x22bbu16.to_le_bytes());
        }
        let mut raw = b"PS2A".to_vec();
        for word in [
            48u32,
            0,
            0,
            0,
            code.len() as u32,
            0,
            0,
            0,
            0,
            code.len() as u32,
            0,
        ] {
            raw.extend(word.to_le_bytes());
        }
        raw.extend(code.into_iter().map(|b| b.rotate_left(1).wrapping_add(124)));
        let dummy_script = crate::ps3::Script::decode(&raw).unwrap();
        let mut vm = crate::machine::Machine::new(&dummy_script).unwrap();
        vm.state_mut().set_bit(0, false).unwrap();
        for expected in [0u32, 1u32] {
            while vm.pending_system().is_none() {
                vm.step().unwrap();
            }
            let (pc, command) = vm.pending_system().unwrap();
            host.handle(
                &mut crate::dispatch::Call::new(&mut vm, pc, command),
                &mut files,
            )
            .unwrap();
            assert_eq!(vm.state().system_integer(0), Some(expected));
        }
    }
}
