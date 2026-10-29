//! The complete record stream of the engine snapshot, in the order the writer 0x140069270
//! emits it. Records with a typed model use it; the others keep their payload bytes until
//! their subsystem is modelled. `decode` accepts only the canonical order and `encode`
//! reproduces it, so a decoded original snapshot re-encodes to the same bytes.
use super::{
    Framing, LocalVariables, SavedScene, SavedWindow, framing, parse_records,
    visual::{text, window},
};
use crate::{FormatError, message::TextObject};
use std::collections::BTreeMap;

/// Object records written only while the object exists, as (tag, table size), in order.
const OBJECT_TABLES: [(u16, u16); 9] = [
    (0x6a0, 2),
    (0x380, 12),
    (0x400, 8),
    (0x700, 256),
    (0x800, 4),
    (0x8c0, 6),
    (0x8c8, 8),
    (0x540, 1),
    (0x8e8, 3),
];
/// Single optional effect records after the object tables: +4624, +4616, +4632, +4640.
const EFFECTS: [u16; 4] = [0x600, 0x610, 0x620, 0x630];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineSnapshot {
    /// 0x100: +11840 save title (0x2162).
    pub save_title: Vec<u8>,
    /// 0x101: +14336 scene title (0x2161).
    pub scene_title: Vec<u8>,
    /// 0x102..0x105: main and common script names.
    pub script_names: [Vec<u8>; 4],
    /// 0x112..0x115: memory image sizes; the loader keeps only 0x112.
    pub script_sizes: [u32; 4],
    /// 0x106..0x108: active slot, PC and repeat count.
    pub active_slot: u32,
    pub pc: u32,
    pub repeat_count: u32,
    /// 0x109: +14748, ten words.
    pub timers: [u32; 10],
    /// 0x10a: +16584.
    pub word_16584: u32,
    /// 0x10b: +14788, 1796 bytes.
    pub callbacks: Vec<u8>,
    /// 0x200..0x203.
    pub clear_color: u32,
    pub word_11144: u32,
    pub word_12092: u32,
    pub depth_clear: u32,
    /// 0x240..0x243 and 0x248.
    pub stack_pointer: u32,
    pub frame_depth: u32,
    pub frames: [u32; 64],
    pub stack: Vec<u8>,
    pub system_results: [u32; 11],
    /// 0x280..0x283.
    pub variables: LocalVariables,
    pub texts: [Option<TextObject>; 12],
    pub windows: [Option<SavedWindow>; 8],
    pub objects: BTreeMap<u8, SavedScene>,
    /// Other object records (0x6a0, 0x800, 0x8c0, 0x8c8, 0x540, 0x8e8) by (tag, index).
    pub raw_objects: BTreeMap<(u16, u16), Vec<u8>>,
    /// 0x600, 0x610, 0x620, 0x630 when present.
    pub effects: [Option<Vec<u8>>; 4],
    /// 0x6e0: +4736.
    pub record_6e0: Vec<u8>,
    /// 0x880..0x883.
    pub cameras: [Vec<u8>; 4],
    /// 0x900: the main image's saved data segment.
    pub script_data: Vec<u8>,
    /// 0x304: BGM names at +4464 and +4464+64.
    pub bgm: [Vec<u8>; 2],
    /// 0x310: +1656, 312 bytes.
    pub sound_channels: Vec<u8>,
    /// 0x318: +4456.
    pub sound_device: Vec<u8>,
}

fn c_string(body: &[u8]) -> Result<Vec<u8>, FormatError> {
    match body.split_last() {
        Some((0, text)) if !text.contains(&0) => Ok(text.to_vec()),
        _ => Err(FormatError("snapshot string record is not one C string")),
    }
}
fn word(body: &[u8]) -> Result<u32, FormatError> {
    Ok(words::<1>(body)?[0])
}
fn words<const N: usize>(body: &[u8]) -> Result<[u32; N], FormatError> {
    if body.len() != N * 4 {
        return Err(FormatError("snapshot word record has unexpected length"));
    }
    Ok(std::array::from_fn(|i| {
        u32::from_le_bytes(body[i * 4..i * 4 + 4].try_into().unwrap())
    }))
}

/// The fixed-order global records before the object tables, with their tags.
const HEAD: [u16; 29] = [
    0x100, 0x101, 0x102, 0x103, 0x104, 0x105, 0x112, 0x113, 0x114, 0x115, 0x106, 0x107, 0x108,
    0x109, 0x10a, 0x10b, 0x200, 0x201, 0x202, 0x203, 0x240, 0x241, 0x242, 0x243, 0x248, 0x280,
    0x281, 0x282, 0x283,
];
const TAIL: [u16; 8] = [0x6e0, 0x880, 0x881, 0x882, 0x883, 0x900, 0x304, 0x310];

impl EngineSnapshot {
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        let records = parse_records(bytes)?;
        let mut at = 0;
        let mut next = |tag: u16| -> Result<&[u8], FormatError> {
            let record = records
                .get(at)
                .filter(|r| r.tag == tag)
                .ok_or(FormatError("snapshot records are not in writer order"))?;
            at += 1;
            Ok(&bytes[record.body.clone()])
        };
        let mut head = Vec::with_capacity(HEAD.len());
        for tag in HEAD {
            head.push(next(tag)?);
        }
        let strings: Vec<Vec<u8>> = head[..6]
            .iter()
            .map(|b| c_string(b))
            .collect::<Result<_, _>>()?;
        let stack = head[23].to_vec();
        if head[15].len() != 1796 || stack.len() != 0x10000 {
            return Err(FormatError(
                "snapshot fixed-size record has unexpected length",
            ));
        }
        let mut variable_records = Vec::new();
        for (i, tag) in (0x280u16..=0x283).enumerate() {
            let body = head[25 + i];
            variable_records.extend(tag.to_le_bytes());
            variable_records.extend((body.len() as u32).to_le_bytes());
            variable_records.extend(body);
        }
        let mut out = Self {
            save_title: strings[0].clone(),
            scene_title: strings[1].clone(),
            script_names: std::array::from_fn(|i| strings[2 + i].clone()),
            script_sizes: [
                word(head[6])?,
                word(head[7])?,
                word(head[8])?,
                word(head[9])?,
            ],
            active_slot: word(head[10])?,
            pc: word(head[11])?,
            repeat_count: word(head[12])?,
            timers: words(head[13])?,
            word_16584: word(head[14])?,
            callbacks: head[15].to_vec(),
            clear_color: word(head[16])?,
            word_11144: word(head[17])?,
            word_12092: word(head[18])?,
            depth_clear: word(head[19])?,
            stack_pointer: word(head[20])?,
            frame_depth: word(head[21])?,
            frames: words(head[22])?,
            stack,
            system_results: words(head[24])?,
            variables: LocalVariables::decode(&variable_records)?,
            texts: std::array::from_fn(|_| None),
            windows: std::array::from_fn(|_| None),
            objects: BTreeMap::new(),
            raw_objects: BTreeMap::new(),
            effects: std::array::from_fn(|_| None),
            record_6e0: Vec::new(),
            cameras: std::array::from_fn(|_| Vec::new()),
            script_data: Vec::new(),
            bgm: std::array::from_fn(|_| Vec::new()),
            sound_channels: Vec::new(),
            sound_device: Vec::new(),
        };
        // Object tables in writer order; within a table, ascending unique indices.
        let mut last = None;
        let mut nodes = 0;
        while let Some(record) = records.get(at) {
            let Some(position) = OBJECT_TABLES
                .iter()
                .position(|&(tag, _)| tag == record.tag)
                .or_else(|| {
                    EFFECTS
                        .iter()
                        .position(|&tag| tag == record.tag)
                        .map(|i| OBJECT_TABLES.len() + i)
                })
            else {
                break;
            };
            let index = record.index.unwrap_or(0);
            if last.is_some_and(|previous| (position, index) <= previous) {
                return Err(FormatError(
                    "snapshot object records are not in writer order",
                ));
            }
            last = Some((position, index));
            let body = &bytes[record.body.clone()];
            match record.tag {
                0x380 if index < 12 => out.texts[index as usize] = Some(text::decode(body)?),
                0x400 if index < 8 => {
                    out.windows[index as usize] = Some(window::decode(body, &mut nodes)?)
                }
                0x700 if index < 256 => {
                    out.objects
                        .insert(index as u8, SavedScene::decode(body, 0, &mut nodes)?);
                }
                0x600 | 0x610 | 0x620 | 0x630 => {
                    out.effects[usize::from((record.tag - 0x600) / 0x10)] = Some(body.to_vec());
                }
                tag => {
                    let limit = OBJECT_TABLES[position].1;
                    if index >= limit {
                        return Err(FormatError("snapshot object index exceeds its table"));
                    }
                    out.raw_objects.insert((tag, index), body.to_vec());
                }
            }
            at += 1;
        }
        for window in out.windows.iter().flatten() {
            if out.texts[window.state.text].is_none() {
                return Err(FormatError("snapshot window references absent text"));
            }
        }
        let mut next = |tag: u16| -> Result<&[u8], FormatError> {
            let record = records
                .get(at)
                .filter(|r| r.tag == tag)
                .ok_or(FormatError("snapshot records are not in writer order"))?;
            at += 1;
            Ok(&bytes[record.body.clone()])
        };
        let mut tail = Vec::with_capacity(TAIL.len());
        for tag in TAIL {
            tail.push(next(tag)?);
        }
        out.sound_device = next(0x318)?.to_vec();
        if at != records.len() {
            return Err(FormatError("unexpected snapshot records after 0x318"));
        }
        out.record_6e0 = tail[0].to_vec();
        out.cameras = std::array::from_fn(|i| tail[1 + i].to_vec());
        out.script_data = tail[5].to_vec();
        let pair = tail[6];
        let first = usize::from(u16::from_le_bytes([pair[0], pair[1]]));
        out.bgm = [
            c_string(&pair[2..2 + first])?,
            c_string(&pair[4 + first..])?,
        ];
        out.sound_channels = tail[7].to_vec();
        if out.sound_channels.len() != 312 {
            return Err(FormatError(
                "snapshot sound channel record has unexpected length",
            ));
        }
        Ok(out)
    }

    pub fn encode(&self) -> Result<Vec<u8>, FormatError> {
        let mut out = Writer::default();
        out.string(0x100, &self.save_title)?;
        out.string(0x101, &self.scene_title)?;
        for (i, name) in self.script_names.iter().enumerate() {
            out.string(0x102 + i as u16, name)?;
        }
        for (i, size) in self.script_sizes.iter().enumerate() {
            out.words(0x112 + i as u16, &[*size])?;
        }
        out.words(0x106, &[self.active_slot])?;
        out.words(0x107, &[self.pc])?;
        out.words(0x108, &[self.repeat_count])?;
        out.words(0x109, &self.timers)?;
        out.words(0x10a, &[self.word_16584])?;
        out.record(0x10b, None, &self.callbacks)?;
        out.words(0x200, &[self.clear_color])?;
        out.words(0x201, &[self.word_11144])?;
        out.words(0x202, &[self.word_12092])?;
        out.words(0x203, &[self.depth_clear])?;
        out.words(0x240, &[self.stack_pointer])?;
        out.words(0x241, &[self.frame_depth])?;
        out.words(0x242, &self.frames)?;
        out.record(0x243, None, &self.stack)?;
        out.words(0x248, &self.system_results)?;
        out.0.extend(self.variables.encode_records());
        for &(tag, limit) in &OBJECT_TABLES {
            for index in 0..limit {
                let body = match tag {
                    0x380 => self.texts[index as usize].as_ref().map(text::encode),
                    0x400 => self.windows[index as usize].as_ref().map(window::encode),
                    0x700 => self.objects.get(&(index as u8)).map(SavedScene::encode),
                    _ => self.raw_objects.get(&(tag, index)).cloned().map(Ok),
                };
                if let Some(body) = body {
                    out.record(tag, Some(index), &body?)?;
                }
            }
        }
        for (tag, body) in EFFECTS.iter().zip(&self.effects) {
            if let Some(body) = body {
                out.record(*tag, None, body)?;
            }
        }
        out.record(0x6e0, None, &self.record_6e0)?;
        for (i, camera) in self.cameras.iter().enumerate() {
            out.record(0x880 + i as u16, None, camera)?;
        }
        out.record(0x900, None, &self.script_data)?;
        out.0.extend(0x304u16.to_le_bytes());
        for name in &self.bgm {
            out.short(name.len() + 1)?;
            out.0.extend(name);
            out.0.push(0);
        }
        out.record(0x310, None, &self.sound_channels)?;
        out.record(0x318, None, &self.sound_device)?;
        Ok(out.0)
    }
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn short(&mut self, len: usize) -> Result<(), FormatError> {
        let len = u16::try_from(len).map_err(|_| FormatError("snapshot record too long"))?;
        self.0.extend(len.to_le_bytes());
        Ok(())
    }
    fn record(&mut self, tag: u16, index: Option<u16>, body: &[u8]) -> Result<(), FormatError> {
        self.0.extend(tag.to_le_bytes());
        match framing(tag).expect("writer emits known tags") {
            Framing::Short => self.short(body.len())?,
            Framing::Long | Framing::Indexed => {
                if framing(tag) == Some(Framing::Indexed) {
                    let index = index.ok_or(FormatError("indexed snapshot record lacks index"))?;
                    self.0.extend(index.to_le_bytes());
                }
                let len = u32::try_from(body.len())
                    .map_err(|_| FormatError("snapshot record too long"))?;
                self.0.extend(len.to_le_bytes());
            }
            Framing::StringPair => unreachable!("0x304 is written separately"),
        }
        self.0.extend(body);
        Ok(())
    }
    fn string(&mut self, tag: u16, text: &[u8]) -> Result<(), FormatError> {
        if text.contains(&0) {
            return Err(FormatError("snapshot string contains NUL"));
        }
        let mut body = text.to_vec();
        body.push(0);
        self.record(tag, None, &body)
    }
    fn words(&mut self, tag: u16, words: &[u32]) -> Result<(), FormatError> {
        let body: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.record(tag, None, &body)
    }
}
