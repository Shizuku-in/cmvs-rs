//! Stateful integer and float evaluation with explicit unknown values and transactional errors.
use crate::{
    bytecode::{DecodeError, Expression, Term},
    vm::{ParameterStack, StackWord},
    vm_core::frame_offset,
};
use std::{collections::BTreeMap, fmt};

/// Float variables flt_14012df00..0x14012ff00 hold 2048 entries.
const FLOAT_LIMIT: u32 = 2048;

pub(crate) fn error(pc: usize, opcode: u16, reason: &'static str) -> DecodeError {
    DecodeError {
        pc,
        opcode: Some(opcode),
        reason,
    }
}

#[derive(Debug)]
enum Undo {
    Integer(u32, Option<i32>),
    Bit(u32, Option<bool>),
    String(u32, Option<Vec<u8>>),
    Buffer(Vec<u8>),
    Result(Option<u32>),
    Condition(Option<bool>),
    Float(u32, Option<u32>),
    SystemInteger(usize, Option<u32>),
    SystemFloat(usize, Option<u32>),
    Stack(usize, StackWord),
    Data(usize, [u8; 4]),
}

/// Previous values recorded before each write, so a failed evaluation is undone
/// without cloning the whole State.
#[derive(Debug, Default)]
pub(crate) struct Journal(Vec<Undo>);

impl Journal {
    pub(crate) fn clear(&mut self, scratch: &mut EvalScratch) {
        for undo in self.0.drain(..) {
            if let Undo::Buffer(buf) = undo {
                scratch.put_string_buffer(buf);
            }
        }
    }
}

/// Reusable operand stacks and string accumulator kept across VM evaluations.
#[derive(Debug, Default)]
pub(crate) struct EvalScratch {
    int_stack: Vec<Cell>,
    float_stack: Vec<FloatCell>,
    string_buffer: Vec<u8>,
}

impl EvalScratch {
    pub(crate) fn take_string_buffer(&mut self) -> Vec<u8> {
        let mut buf = std::mem::take(&mut self.string_buffer);
        buf.clear();
        buf
    }

    pub(crate) fn put_string_buffer(&mut self, mut buf: Vec<u8>) {
        buf.clear();
        if buf.capacity() >= self.string_buffer.capacity() {
            self.string_buffer = buf;
        }
    }
}

struct StackFrame<'a, T> {
    stack: &'a mut Vec<T>,
    base: usize,
}

impl<'a, T> StackFrame<'a, T> {
    fn new(stack: &'a mut Vec<T>) -> Self {
        let base = stack.len();
        Self { stack, base }
    }

    fn push(&mut self, value: T) {
        self.stack.push(value);
    }

    fn pop(&mut self) -> Option<T> {
        (self.stack.len() > self.base)
            .then(|| self.stack.pop())
            .flatten()
    }

    fn single(&self) -> Option<T>
    where
        T: Copy,
    {
        (self.stack.len() - self.base == 1).then(|| self.stack[self.base])
    }
}

impl<T> Drop for StackFrame<'_, T> {
    fn drop(&mut self) {
        self.stack.truncate(self.base);
    }
}

/// Iterates `(word_index, bit_mask)` covering bit indices `start..end`.
fn bit_words(start: usize, end: usize) -> impl Iterator<Item = (usize, u64)> {
    let first_word = start >> 6;
    let last_word = (end - 1) >> 6;
    let start_bit = start & 63;
    let end_bit = end & 63;
    (first_word..=last_word).map(move |word| {
        let low = if word == first_word {
            u64::MAX << start_bit
        } else {
            u64::MAX
        };
        let high = if word == last_word && end_bit != 0 {
            (1u64 << end_bit) - 1
        } else {
            u64::MAX
        };
        (word, low & high)
    })
}

fn known_indices(words: &[u64]) -> impl Iterator<Item = u32> + Clone {
    words.iter().enumerate().flat_map(|(word, &known)| {
        let mut remaining = known;
        std::iter::from_fn(move || {
            if remaining == 0 {
                return None;
            }
            let bit = remaining.trailing_zeros();
            remaining &= remaining - 1;
            Some(((word as u32) << 6) + bit)
        })
    })
}

#[derive(Debug, Clone, Default)]
struct BitBank {
    values: Vec<u64>,
    known: Vec<u64>,
    known_count: usize,
}

impl BitBank {
    fn get(&self, index: u32) -> Option<bool> {
        let idx = index as usize;
        let word = idx >> 6;
        let mask = 1u64 << (idx & 63);
        if self.known.get(word).copied().unwrap_or(0) & mask != 0 {
            Some(self.values[word] & mask != 0)
        } else {
            None
        }
    }

    fn set(&mut self, index: u32, value: bool) -> Option<bool> {
        let idx = index as usize;
        let word = idx >> 6;
        let mask = 1u64 << (idx & 63);
        if word >= self.known.len() {
            self.known.resize(word + 1, 0);
            self.values.resize(word + 1, 0);
        }
        let old = if self.known[word] & mask != 0 {
            Some(self.values[word] & mask != 0)
        } else {
            self.known[word] |= mask;
            self.known_count += 1;
            None
        };
        if value {
            self.values[word] |= mask;
        } else {
            self.values[word] &= !mask;
        }
        old
    }

    fn restore(&mut self, index: u32, old: Option<bool>) {
        match old {
            Some(value) => {
                self.set(index, value);
            }
            None => {
                let idx = index as usize;
                let word = idx >> 6;
                let mask = 1u64 << (idx & 63);
                if let Some(known) = self.known.get_mut(word)
                    && *known & mask != 0
                {
                    *known &= !mask;
                    self.values[word] &= !mask;
                    self.known_count -= 1;
                }
            }
        }
    }

    fn fill(&mut self, start: u32, count: u32, value: bool) {
        if count == 0 {
            return;
        }
        let start = start as usize;
        let end = start + count as usize;
        let words = end.div_ceil(64);
        if words > self.known.len() {
            self.known.resize(words, 0);
            self.values.resize(words, 0);
        }
        for (word, mask) in bit_words(start, end) {
            self.known_count += (mask & !self.known[word]).count_ones() as usize;
            self.known[word] |= mask;
            if value {
                self.values[word] |= mask;
            } else {
                self.values[word] &= !mask;
            }
        }
    }

    fn iter(&self) -> impl Iterator<Item = (u32, bool)> + Clone {
        known_indices(&self.known).map(|index| {
            (
                index,
                self.values[index as usize >> 6] & (1 << (index & 63)) != 0,
            )
        })
    }

    fn to_btree_map(&self) -> BTreeMap<u32, bool> {
        self.iter().collect()
    }
}

#[derive(Debug, Clone)]
struct WordBank<T> {
    values: Vec<T>,
    known: Vec<u64>,
    known_count: usize,
}

impl<T> Default for WordBank<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            known: Vec::new(),
            known_count: 0,
        }
    }
}

impl<T: Copy + Default> WordBank<T> {
    fn get(&self, index: u32) -> Option<T> {
        let idx = index as usize;
        let word = idx >> 6;
        let mask = 1u64 << (idx & 63);
        if self.known.get(word).copied().unwrap_or(0) & mask != 0 {
            Some(self.values[idx])
        } else {
            None
        }
    }

    fn set(&mut self, index: u32, value: T) -> Option<T> {
        let idx = index as usize;
        let word = idx >> 6;
        let mask = 1u64 << (idx & 63);
        if word >= self.known.len() {
            self.known.resize(word + 1, 0);
        }
        if idx >= self.values.len() {
            self.values.resize(idx + 1, T::default());
        }
        let old = if self.known[word] & mask != 0 {
            Some(self.values[idx])
        } else {
            self.known[word] |= mask;
            self.known_count += 1;
            None
        };
        self.values[idx] = value;
        old
    }

    fn restore(&mut self, index: u32, old: Option<T>) {
        match old {
            Some(value) => {
                self.set(index, value);
            }
            None => {
                let idx = index as usize;
                let word = idx >> 6;
                let mask = 1u64 << (idx & 63);
                if let Some(known) = self.known.get_mut(word)
                    && *known & mask != 0
                {
                    *known &= !mask;
                    self.known_count -= 1;
                }
            }
        }
    }

    fn fill(&mut self, start: u32, count: u32, value: T) {
        if count == 0 {
            return;
        }
        let start = start as usize;
        let end = start + count as usize;
        let words = end.div_ceil(64);
        if words > self.known.len() {
            self.known.resize(words, 0);
        }
        if end > self.values.len() {
            self.values.resize(end, T::default());
        }
        self.values[start..end].fill(value);
        for (word, mask) in bit_words(start, end) {
            self.known_count += (mask & !self.known[word]).count_ones() as usize;
            self.known[word] |= mask;
        }
    }

    fn iter(&self) -> impl Iterator<Item = (u32, T)> + Clone {
        known_indices(&self.known).map(|index| (index, self.values[index as usize]))
    }

    fn to_btree_map(&self) -> BTreeMap<u32, T> {
        self.iter().collect()
    }
}

/// Stack bytes addressed by frame operands; `base` is frame[depth], zero at depth 0.
pub(crate) struct Frame<'s> {
    pub(crate) stack: &'s mut ParameterStack,
    pub(crate) base: usize,
}

/// Memory outside State that operands may address.
pub(crate) struct Env<'a, 's> {
    pub(crate) labels: &'a [u32],
    pub(crate) frame: Option<Frame<'s>>,
    pub(crate) data: Option<&'s mut [u8]>,
}

impl Env<'_, '_> {
    fn data_offset(&self, raw: u32, pc: usize, tag: u16) -> Result<usize, DecodeError> {
        let data = self
            .data
            .as_deref()
            .ok_or_else(|| error(pc, tag, "script data operand needs a machine image"))?;
        let offset = usize::try_from(raw as i32)
            .map_err(|_| error(pc, tag, "negative script data offset"))?;
        if offset.checked_add(4).is_none_or(|end| end > data.len()) {
            return Err(error(pc, tag, "script data word outside data segment"));
        }
        Ok(offset)
    }
    fn load_data(&self, offset: usize) -> i32 {
        i32::from_le_bytes(
            self.data.as_deref().expect("checked data")[offset..offset + 4]
                .try_into()
                .expect("checked word"),
        )
    }
    fn store_data(&mut self, offset: usize, value: i32, journal: &mut Journal) {
        let data = self.data.as_deref_mut().expect("checked data");
        let old = data[offset..offset + 4].try_into().expect("checked word");
        journal.0.push(Undo::Data(offset, old));
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    /// Tags 0x103/0x129 address base-raw (arguments); 0x108/0x12c address base+raw (locals).
    fn offset(&self, tag: u16, raw: u32, pc: usize) -> Result<usize, DecodeError> {
        let frame = self
            .frame
            .as_ref()
            .ok_or_else(|| error(pc, tag, "frame operand needs the machine stack"))?;
        frame_offset(frame.base, raw, matches!(tag, 0x103 | 0x129))
            .ok_or_else(|| error(pc, tag, "frame operand outside the stack"))
    }
    fn load(&self, offset: usize, pc: usize, op: u16) -> Result<u32, DecodeError> {
        let frame = self.frame.as_ref().expect("offset checked the frame");
        frame
            .stack
            .read_u32_at(offset)
            .map_err(|e| error(pc, op, e.0))
    }
    fn store(
        &mut self,
        offset: usize,
        value: u32,
        journal: &mut Journal,
        pc: usize,
        op: u16,
    ) -> Result<(), DecodeError> {
        let frame = self.frame.as_mut().expect("offset checked the frame");
        let old = frame
            .stack
            .write_u32_at(offset, value)
            .map_err(|e| error(pc, op, e.0))?;
        journal.0.push(Undo::Stack(offset, old));
        Ok(())
    }
}

/// cvttss2si truncation, including the x86 indefinite value for NaN and out-of-range input.
pub(crate) fn truncate(value: f32) -> i32 {
    if value.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

/// Bounded research state. Missing variables are unknown, not implicitly zero.
#[derive(Clone)]
pub struct State {
    integers: WordBank<i32>,
    bits: BitBank,
    floats: WordBank<u32>,
    strings: Vec<Option<Vec<u8>>>,
    system_integers: [Option<u32>; 7],
    /// Float system result slots 7..10 (+82844..+82856) as raw f32 bits.
    system_float_bits: [Option<u32>; 4],
    integer_limit: u32,
    bit_limit: u32,
    /// String variable slots: 128 in CMVS, 256 in CVNS (cvns-20090203 0x469cd0/0x469d00).
    string_slots: u32,
    string_buffer: Vec<u8>,
    pub result: Option<u32>,
    pub condition: Option<bool>,
}

struct DebugMap<I>(I);

impl<K: fmt::Debug, V: fmt::Debug, I: Iterator<Item = (K, V)> + Clone> fmt::Debug for DebugMap<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.0.clone()).finish()
    }
}

// Probe hashes this representation: omit unknown slots and retained scratch storage.
impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let strings = self
            .strings
            .iter()
            .enumerate()
            .filter_map(|(index, value)| value.as_ref().map(|value| (index, value)));
        f.debug_struct("State")
            .field("integers", &DebugMap(self.integers.iter()))
            .field("bits", &DebugMap(self.bits.iter()))
            .field("floats", &DebugMap(self.floats.iter()))
            .field("strings", &DebugMap(strings))
            .field("system_integers", &self.system_integers)
            .field("system_float_bits", &self.system_float_bits)
            .field("integer_limit", &self.integer_limit)
            .field("bit_limit", &self.bit_limit)
            .field("string_slots", &self.string_slots)
            .field("string_buffer", &self.string_buffer)
            .field("result", &self.result)
            .field("condition", &self.condition)
            .finish()
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new(8192, 262144)
    }
}

impl State {
    /// Limits are research policy, not assertions about all CMVS versions.
    pub fn new(integer_limit: u32, bit_limit: u32) -> Self {
        Self {
            integers: WordBank::default(),
            bits: BitBank::default(),
            floats: WordBank::default(),
            strings: Vec::new(),
            system_integers: [None; 7],
            system_float_bits: [None; 4],
            integer_limit,
            bit_limit,
            string_slots: 128,
            string_buffer: Vec::new(),
            result: None,
            condition: None,
        }
    }
    pub fn integer(&self, index: u32) -> Option<i32> {
        self.integers.get(index)
    }
    /// Integer system result slots +82816..+82840, distinct from expression result +82812.
    pub fn system_integer(&self, index: u32) -> Option<u32> {
        self.system_integers.get(index as usize).copied().flatten()
    }
    pub fn set_system_integer(&mut self, index: u32, value: u32) -> Result<(), &'static str> {
        let slot = self
            .system_integers
            .get_mut(index as usize)
            .ok_or("integer system result slot must be 0..6")?;
        *slot = Some(value);
        Ok(())
    }
    /// Float system result slot 7..10 as raw f32 bits.
    pub fn system_float_bits(&self, index: u32) -> Option<u32> {
        self.system_float_bits
            .get((index as usize).wrapping_sub(7))
            .copied()
            .flatten()
    }
    pub fn set_system_float_bits(&mut self, index: u32, bits: u32) -> Result<(), &'static str> {
        let slot = self
            .system_float_bits
            .get_mut((index as usize).wrapping_sub(7))
            .ok_or("float system result slot must be 7..10")?;
        *slot = Some(bits);
        Ok(())
    }
    /// Marks a system result the original writes with a value this model does not know, so
    /// that reading it fails explicitly.
    pub(crate) fn forget_system_integer(&mut self, index: u32) -> Result<(), &'static str> {
        let slot = self
            .system_integers
            .get_mut(index as usize)
            .ok_or("integer system result slot must be 0..6")?;
        *slot = None;
        Ok(())
    }
    pub fn bit(&self, index: u32) -> Option<bool> {
        self.bits.get(index)
    }
    pub fn known_integer_count(&self) -> usize {
        self.integers.known_count
    }
    pub fn known_bit_count(&self) -> usize {
        self.bits.known_count
    }
    pub fn integers(&self) -> BTreeMap<u32, i32> {
        self.integers.to_btree_map()
    }
    pub fn bits(&self) -> BTreeMap<u32, bool> {
        self.bits.to_btree_map()
    }
    pub fn float_bits(&self, index: u32) -> Option<u32> {
        self.floats.get(index)
    }
    pub fn string_slots(&self) -> u32 {
        self.string_slots
    }
    /// The engine's string bank size; slots at or past it are never written.
    pub fn set_string_slots(&mut self, slots: u32) {
        self.string_slots = slots;
    }
    pub fn string(&self, index: u32) -> Option<&[u8]> {
        self.strings.get(index as usize).and_then(Option::as_deref)
    }
    /// The 1024-byte accumulator at +82864, cleared by every 0x0201 evaluation.
    pub fn string_buffer(&self) -> &[u8] {
        &self.string_buffer
    }
    pub(crate) fn set_string_buffer_journaled(&mut self, bytes: Vec<u8>, journal: &mut Journal) {
        let old = std::mem::replace(&mut self.string_buffer, bytes);
        journal.0.push(Undo::Buffer(old));
    }
    pub(crate) fn set_result_journaled(&mut self, value: Option<u32>, journal: &mut Journal) {
        journal.0.push(Undo::Result(self.result));
        self.result = value;
    }
    pub(crate) fn set_string_journaled(
        &mut self,
        index: u32,
        bytes: &[u8],
        journal: &mut Journal,
    ) -> Result<(), &'static str> {
        Self::range(index, 1, self.string_slots)?;
        if bytes.len() >= 1024 || bytes.contains(&0) {
            return Err("invalid bounded string slot value");
        }
        let idx = index as usize;
        if idx >= self.strings.len() {
            self.strings.resize_with(idx + 1, || None);
        }
        let old = self.strings[idx].replace(bytes.to_vec());
        journal.0.push(Undo::String(index, old));
        Ok(())
    }
    fn set_integer_journaled(
        &mut self,
        index: u32,
        value: i32,
        journal: &mut Journal,
    ) -> Result<(), &'static str> {
        if index >= self.integer_limit {
            return Err("integer variable outside research limit");
        }
        let old = self.integers.set(index, value);
        journal.0.push(Undo::Integer(index, old));
        Ok(())
    }
    fn set_bit_journaled(
        &mut self,
        index: u32,
        value: bool,
        journal: &mut Journal,
    ) -> Result<(), &'static str> {
        if index >= self.bit_limit {
            return Err("bit variable outside research limit");
        }
        let old = self.bits.set(index, value);
        journal.0.push(Undo::Bit(index, old));
        Ok(())
    }
    fn set_float_journaled(
        &mut self,
        index: u32,
        bits: u32,
        journal: &mut Journal,
    ) -> Result<(), &'static str> {
        if index >= FLOAT_LIMIT {
            return Err("float variable outside research limit");
        }
        let old = self.floats.set(index, bits);
        journal.0.push(Undo::Float(index, old));
        Ok(())
    }
    /// Restores every journaled write in reverse order; stack words need the same stack.
    pub(crate) fn rollback(
        &mut self,
        journal: &mut Journal,
        mut stack: Option<&mut ParameterStack>,
        mut data: Option<&mut [u8]>,
    ) {
        for undo in journal.0.drain(..).rev() {
            match undo {
                Undo::Integer(index, old) => self.integers.restore(index, old),
                Undo::Bit(index, old) => self.bits.restore(index, old),
                Undo::String(index, old) => {
                    if (index as usize) < self.strings.len() {
                        self.strings[index as usize] = old;
                    }
                }
                Undo::Float(index, old) => self.floats.restore(index, old),
                Undo::SystemInteger(index, old) => self.system_integers[index] = old,
                Undo::SystemFloat(index, old) => self.system_float_bits[index] = old,
                Undo::Stack(offset, old) => {
                    let stack = stack.as_deref_mut().expect("stack writes need their stack");
                    stack.restore_at(offset, old);
                }
                Undo::Data(offset, old) => {
                    data.as_deref_mut().expect("data writes need their image")[offset..offset + 4]
                        .copy_from_slice(&old)
                }
                Undo::Buffer(old) => self.string_buffer = old,
                Undo::Result(old) => self.result = old,
                Undo::Condition(old) => self.condition = old,
            }
        }
    }
    pub fn set_string(&mut self, index: u32, bytes: &[u8]) -> Result<(), &'static str> {
        self.fill_strings(index, 1, bytes)
    }
    fn range(start: u32, count: u32, limit: u32) -> Result<std::ops::Range<u32>, &'static str> {
        let end = start.checked_add(count).ok_or("variable range overflow")?;
        if end > limit {
            return Err("variable range outside research limit");
        }
        Ok(start..end)
    }
    pub fn fill_integers(
        &mut self,
        start: u32,
        count: u32,
        value: i32,
    ) -> Result<(), &'static str> {
        Self::range(start, count, self.integer_limit)?;
        self.integers.fill(start, count, value);
        Ok(())
    }
    pub fn fill_bits(&mut self, start: u32, count: u32, value: bool) -> Result<(), &'static str> {
        Self::range(start, count, self.bit_limit)?;
        self.bits.fill(start, count, value);
        Ok(())
    }
    pub fn fill_floats(&mut self, start: u32, count: u32, bits: u32) -> Result<(), &'static str> {
        // Research capacity; the model preserves raw f32 bits, including NaN payloads.
        Self::range(start, count, FLOAT_LIMIT)?;
        self.floats.fill(start, count, bits);
        Ok(())
    }
    pub fn fill_strings(
        &mut self,
        start: u32,
        count: u32,
        bytes: &[u8],
    ) -> Result<(), &'static str> {
        if bytes.len() >= 1024 || bytes.contains(&0) {
            return Err("invalid bounded string slot value");
        }
        let range = Self::range(start, count, self.string_slots)?;
        if !range.is_empty() && range.end as usize > self.strings.len() {
            self.strings.resize_with(range.end as usize, || None);
        }
        for index in range {
            match &mut self.strings[index as usize] {
                Some(existing) => {
                    existing.clear();
                    existing.extend_from_slice(bytes);
                }
                slot @ None => *slot = Some(bytes.to_vec()),
            }
        }
        Ok(())
    }
    pub fn set_integer(&mut self, index: u32, value: i32) -> Result<(), &'static str> {
        if index >= self.integer_limit {
            return Err("integer variable outside research limit");
        }
        self.integers.set(index, value);
        Ok(())
    }
    pub fn set_bit(&mut self, index: u32, value: bool) -> Result<(), &'static str> {
        if index >= self.bit_limit {
            return Err("bit variable outside research limit");
        }
        self.bits.set(index, value);
        Ok(())
    }
    /// Commit all nested expression writes only if the complete expression succeeds.
    pub fn evaluate_integer(&mut self, expr: &Expression) -> Result<i32, DecodeError> {
        self.evaluate_integer_with_labels(expr, &[])
    }
    pub fn evaluate_integer_with_labels(
        &mut self,
        expr: &Expression,
        labels: &[u32],
    ) -> Result<i32, DecodeError> {
        let mut journal = Journal::default();
        let mut scratch = EvalScratch::default();
        let mut env = Env {
            labels,
            frame: None,
            data: None,
        };
        let outcome = self.evaluate_in(expr, &mut env, &mut scratch, &mut journal);
        if outcome.is_err() {
            self.rollback(&mut journal, None, None);
        }
        outcome
    }
    /// Evaluation inside a larger transaction; the caller owns rollback.
    pub(crate) fn evaluate_in(
        &mut self,
        expr: &Expression,
        env: &mut Env,
        scratch: &mut EvalScratch,
        journal: &mut Journal,
    ) -> Result<i32, DecodeError> {
        self.evaluate(expr, 0, env, &mut scratch.int_stack, journal)
    }
    fn read(&self, cell: Cell, env: &Env, pc: usize, op: u16) -> Result<i32, DecodeError> {
        match cell {
            Cell::Value(value) | Cell::Literal(value) => Ok(value),
            Cell::Frame(offset) => Ok(env.load(offset, pc, op)? as i32),
            Cell::Data(offset) => Ok(env.load_data(offset)),
            Cell::Integer(index) => self
                .integer(index)
                .ok_or_else(|| error(pc, op, "read of unknown integer variable")),
            Cell::Bit(index) => self
                .bit(index)
                .map(i32::from)
                .ok_or_else(|| error(pc, op, "read of unknown bit variable")),
        }
    }
    fn write(
        &mut self,
        cell: Cell,
        value: i32,
        env: &mut Env,
        pc: usize,
        op: u16,
        journal: &mut Journal,
    ) -> Result<(), DecodeError> {
        match cell {
            Cell::Data(offset) => {
                env.store_data(offset, value, journal);
                return Ok(());
            }
            Cell::Frame(offset) => return env.store(offset, value as u32, journal, pc, op),
            Cell::Integer(index) => self.set_integer_journaled(index, value, journal),
            Cell::Bit(index) => self.set_bit_journaled(index, value != 0, journal),
            // The native integer writer has no 0x100 case: discard the write,
            // while the evaluator still returns the computed assignment value.
            Cell::Literal(_) => Ok(()),
            Cell::Value(_) => Err("assignment requires a writable variable"),
        }
        .map_err(|why| error(pc, op, why))
    }
    fn finish_integer_eval(
        &mut self,
        expr: &Expression,
        value: i32,
        journal: &mut Journal,
    ) -> Result<i32, DecodeError> {
        // The original integer evaluator writes its result to global integer 4096.
        self.set_integer_journaled(4096, value, journal)
            .map_err(|why| error(expr.pc, expr.kind, why))?;
        self.set_result_journaled(Some(value as u32), journal);
        journal.0.push(Undo::Condition(self.condition));
        self.condition = Some(value != 0);
        Ok(value)
    }

    fn evaluate(
        &mut self,
        expr: &Expression,
        depth: usize,
        env: &mut Env,
        int_stack: &mut Vec<Cell>,
        journal: &mut Journal,
    ) -> Result<i32, DecodeError> {
        if depth >= 64 {
            return Err(error(
                expr.pc,
                expr.kind,
                "expression evaluation depth limit",
            ));
        }
        if expr.kind != 0x200 {
            return Err(error(expr.pc, expr.kind, "not an integer expression"));
        }
        if expr.terms.is_empty() {
            // CVNS sub_425810 skips the term loop and computes default result 0 for empty 0x200 0x20f.
            return self.finish_integer_eval(expr, 0, journal);
        }
        let mut stack = StackFrame::new(int_stack);
        for term in &expr.terms {
            match term {
                Term::Scalar {
                    tag: 0x100, raw, ..
                } => stack.push(Cell::Literal(*raw as i32)),
                Term::Scalar {
                    pc,
                    tag: 0x10e,
                    raw,
                } => {
                    let target = env
                        .labels
                        .get(*raw as usize)
                        .ok_or_else(|| error(*pc, 0x10e, "label operand index out of bounds"))?;
                    stack.push(Cell::Value(*target as i32));
                }
                Term::Scalar {
                    pc,
                    tag: 0x10f,
                    raw,
                } => {
                    let value = match *raw {
                        0..=6 => self
                            .system_integer(*raw)
                            .ok_or_else(|| error(*pc, 0x10f, "unknown integer system result"))?,
                        7..=10 => {
                            return Err(error(
                                *pc,
                                0x10f,
                                "floating system result conversion not implemented",
                            ));
                        }
                        _ => 0, // Original operand reader returns zero outside slots 0..10.
                    };
                    stack.push(Cell::Value(value as i32));
                }
                Term::Scalar {
                    pc,
                    tag: tag @ (0x103 | 0x108 | 0x10a),
                    raw,
                } => stack.push(Cell::Frame(env.offset(*tag, *raw, *pc)?)),
                Term::Scalar {
                    pc,
                    tag: tag @ (0x104 | 0x106),
                    raw,
                } => {
                    stack.push(Cell::Data(env.data_offset(*raw, *pc, *tag)?));
                }
                Term::Indexed {
                    pc,
                    tag: tag @ (0x105 | 0x107 | 0x109 | 0x10b | 0x110 | 0x111 | 0x112 | 0x113),
                    base,
                    stride,
                    indices,
                } => {
                    // 0x140066200 / CVNS 0x447420 rewrite array operands with base + stride * idx0
                    // (+ 4 * idx1 for 2D tags 0x111/0x113). Data tags yield 0x104/0x106/0x107; frame
                    // tags yield 0x108/0x10a/0x10b.
                    let two_d = matches!(tag, 0x111 | 0x113);
                    let raw = match indices.as_slice() {
                        [first] if !two_d => {
                            let idx = self.evaluate(first, depth + 1, env, stack.stack, journal)?;
                            base.wrapping_add((idx as u32).wrapping_mul(u32::from(*stride)))
                        }
                        [first, second] if two_d => {
                            let idx0 =
                                self.evaluate(first, depth + 1, env, stack.stack, journal)?;
                            let idx1 =
                                self.evaluate(second, depth + 1, env, stack.stack, journal)?;
                            base.wrapping_add((idx0 as u32).wrapping_mul(u32::from(*stride)))
                                .wrapping_add((idx1 as u32).wrapping_mul(4))
                        }
                        _ => {
                            return Err(error(
                                *pc,
                                *tag,
                                if two_d {
                                    "2D array operand needs two indices"
                                } else {
                                    "array operand needs one index"
                                },
                            ));
                        }
                    };
                    let (is_frame, resolved_tag) = match tag {
                        0x105 => (false, 0x104),
                        0x110 => (false, 0x106),
                        0x107 | 0x111 => (false, 0x107),
                        0x109 => (true, 0x108),
                        0x112 => (true, 0x10a),
                        _ => (true, 0x10b),
                    };
                    stack.push(if is_frame {
                        Cell::Frame(env.offset(resolved_tag, raw, *pc)?)
                    } else {
                        Cell::Data(env.data_offset(raw, *pc, resolved_tag)?)
                    });
                }
                Term::Nested {
                    pc,
                    tag,
                    expression,
                } if matches!(tag, 0x200 | 0x101 | 0x102) => {
                    let value = self.evaluate(expression, depth + 1, env, stack.stack, journal)?;
                    let cell = match tag {
                        0x200 => Cell::Value(value),
                        0x101 if value >= 0 && (value as u32) < self.integer_limit => {
                            Cell::Integer(value as u32)
                        }
                        0x102 if value >= 0 && (value as u32) < self.bit_limit => {
                            Cell::Bit(value as u32)
                        }
                        _ => return Err(error(*pc, *tag, "variable index outside research limit")),
                    };
                    stack.push(cell);
                }
                Term::Operator {
                    pc,
                    opcode: op @ (0x173 | 0x174),
                } => {
                    let cell = stack
                        .pop()
                        .ok_or_else(|| error(*pc, *op, "expression stack underflow"))?;
                    let old = self.read(cell, env, *pc, *op)?;
                    let value = if *op == 0x173 {
                        old.wrapping_add(1)
                    } else {
                        old.wrapping_sub(1)
                    };
                    self.write(cell, value, env, *pc, *op, journal)?;
                    stack.push(Cell::Value(value));
                }
                Term::Operator { pc, opcode } => {
                    let right_cell = stack
                        .pop()
                        .ok_or_else(|| error(*pc, *opcode, "expression stack underflow"))?;
                    let left_cell = stack
                        .pop()
                        .ok_or_else(|| error(*pc, *opcode, "expression stack underflow"))?;
                    let value = if *opcode == 0x170 {
                        let right = self.read(right_cell, env, *pc, *opcode)?;
                        self.write(left_cell, right, env, *pc, *opcode, journal)?;
                        right
                    } else {
                        // Preserve descriptor laziness: nested writes may change either source.
                        let left = self.read(left_cell, env, *pc, *opcode)?;
                        let right = self.read(right_cell, env, *pc, *opcode)?;
                        let binary = match opcode {
                            0x175 => 0x163,
                            0x176 => 0x164,
                            0x177 => 0x160,
                            0x178 => 0x161,
                            0x179 => 0x165,
                            0x17a => 0x166,
                            0x17b => 0x162,
                            0x17c => 0x167,
                            0x17d => 0x168,
                            0x17e => 0x169,
                            other => *other,
                        };
                        let value = binary_value(*pc, binary, left, right)?;
                        if (0x175..=0x17e).contains(opcode) {
                            self.write(left_cell, value, env, *pc, *opcode, journal)?;
                        }
                        value
                    };
                    stack.push(Cell::Value(value));
                }
                Term::Scalar { pc, tag, .. }
                | Term::Nested { pc, tag, .. }
                | Term::Indexed { pc, tag, .. } => {
                    return Err(error(*pc, *tag, "operand storage/type is not implemented"));
                }
            }
        }
        let Some(top) = stack.single() else {
            return Err(error(
                expr.pc,
                expr.kind,
                "expression does not produce exactly one value",
            ));
        };
        let value = self.read(top, env, expr.pc, expr.kind)?;
        self.finish_integer_eval(expr, value, journal)
    }

    pub fn evaluate_float_with_labels(
        &mut self,
        expr: &Expression,
        labels: &[u32],
    ) -> Result<f32, DecodeError> {
        let mut journal = Journal::default();
        let mut scratch = EvalScratch::default();
        let mut env = Env {
            labels,
            frame: None,
            data: None,
        };
        let outcome = self.evaluate_float_in(expr, &mut env, &mut scratch, &mut journal);
        if outcome.is_err() {
            self.rollback(&mut journal, None, None);
        }
        outcome
    }
    /// Operand reader 0x1400659b0: integer storage converts to float.
    fn read_float(
        &self,
        cell: FloatCell,
        env: &Env,
        pc: usize,
        op: u16,
    ) -> Result<f32, DecodeError> {
        Ok(match cell {
            FloatCell::Value(value) => value,
            FloatCell::Integer(index) => self
                .integer(index)
                .ok_or_else(|| error(pc, op, "read of unknown integer variable"))?
                as f32,
            FloatCell::Bit(index) => f32::from(u8::from(
                self.bit(index)
                    .ok_or_else(|| error(pc, op, "read of unknown bit variable"))?,
            )),
            FloatCell::Float(index) => f32::from_bits(
                self.float_bits(index)
                    .ok_or_else(|| error(pc, op, "read of unknown float variable"))?,
            ),
            FloatCell::FrameInt(offset) => env.load(offset, pc, op)? as i32 as f32,
            FloatCell::FrameFloat(offset) => f32::from_bits(env.load(offset, pc, op)?),
            FloatCell::System(index) => match index {
                0..=6 => self
                    .system_integer(index)
                    .ok_or_else(|| error(pc, op, "unknown integer system result"))?
                    as i32 as f32,
                7..=10 => f32::from_bits(
                    self.system_float_bits(index)
                        .ok_or_else(|| error(pc, op, "unknown float system result"))?,
                ),
                _ => 0.0,
            },
        })
    }
    /// Operand writer 0x140065f00: integer storage receives the truncated value.
    fn write_float(
        &mut self,
        cell: FloatCell,
        value: f32,
        env: &mut Env,
        pc: usize,
        op: u16,
        journal: &mut Journal,
    ) -> Result<(), DecodeError> {
        match cell {
            FloatCell::Integer(index) => {
                self.set_integer_journaled(index, truncate(value), journal)
            }
            FloatCell::Bit(index) => self.set_bit_journaled(index, truncate(value) != 0, journal),
            FloatCell::Float(index) => self.set_float_journaled(index, value.to_bits(), journal),
            FloatCell::System(index) => {
                match index {
                    0..=6 => {
                        let slot = &mut self.system_integers[index as usize];
                        journal.0.push(Undo::SystemInteger(index as usize, *slot));
                        *slot = Some(truncate(value) as u32);
                    }
                    7..=10 => {
                        let index = (index - 7) as usize;
                        let slot = &mut self.system_float_bits[index];
                        journal.0.push(Undo::SystemFloat(index, *slot));
                        *slot = Some(value.to_bits());
                    }
                    _ => {}
                }
                Ok(())
            }
            FloatCell::FrameInt(offset) => {
                return env.store(offset, truncate(value) as u32, journal, pc, op);
            }
            FloatCell::FrameFloat(offset) => {
                return env.store(offset, value.to_bits(), journal, pc, op);
            }
            FloatCell::Value(_) => Err("assignment requires a writable variable"),
        }
        .map_err(|why| error(pc, op, why))
    }
    /// 0x0202 / 0x140066e60: float postfix evaluation; result bits go to +82812 and the
    /// condition bit to `result != 0.0`. Integer 4096 changes only through nested integers.
    pub(crate) fn evaluate_float_in(
        &mut self,
        expr: &Expression,
        env: &mut Env,
        scratch: &mut EvalScratch,
        journal: &mut Journal,
    ) -> Result<f32, DecodeError> {
        if expr.kind != 0x202 {
            return Err(error(expr.pc, expr.kind, "not a float expression"));
        }
        let mut stack = StackFrame::new(&mut scratch.float_stack);
        for term in &expr.terms {
            match term {
                Term::Scalar {
                    tag: 0x100, raw, ..
                } => stack.push(FloatCell::Value(*raw as i32 as f32)),
                Term::Scalar {
                    tag: 0x12a, raw, ..
                } => stack.push(FloatCell::Value(f32::from_bits(*raw))),
                Term::Scalar {
                    tag: 0x10f, raw, ..
                } => stack.push(FloatCell::System(*raw)),
                Term::Scalar {
                    pc,
                    tag: tag @ (0x103 | 0x108 | 0x129 | 0x12c),
                    raw,
                } => {
                    let offset = env.offset(*tag, *raw, *pc)?;
                    stack.push(if matches!(tag, 0x103 | 0x108) {
                        FloatCell::FrameInt(offset)
                    } else {
                        FloatCell::FrameFloat(offset)
                    });
                }
                Term::Nested {
                    pc,
                    tag,
                    expression,
                } if matches!(tag, 0x200 | 0x101 | 0x102 | 0x12b) => {
                    let value =
                        self.evaluate(expression, 1, env, &mut scratch.int_stack, journal)?;
                    let index = value as u32;
                    stack.push(match tag {
                        0x200 => FloatCell::Value(value as f32),
                        0x101 if value >= 0 && index < self.integer_limit => {
                            FloatCell::Integer(index)
                        }
                        0x102 if value >= 0 && index < self.bit_limit => FloatCell::Bit(index),
                        0x12b if value >= 0 && index < FLOAT_LIMIT => FloatCell::Float(index),
                        _ => return Err(error(*pc, *tag, "variable index outside research limit")),
                    });
                }
                Term::Operator {
                    pc,
                    opcode: op @ (0x173 | 0x174),
                } => {
                    let cell = stack
                        .pop()
                        .ok_or_else(|| error(*pc, *op, "expression stack underflow"))?;
                    let old = self.read_float(cell, env, *pc, *op)?;
                    let value = if *op == 0x173 { old + 1.0 } else { old - 1.0 };
                    self.write_float(cell, value, env, *pc, *op, journal)?;
                    stack.push(FloatCell::Value(value));
                }
                Term::Operator { pc, opcode } => {
                    let right_cell = stack
                        .pop()
                        .ok_or_else(|| error(*pc, *opcode, "expression stack underflow"))?;
                    let left_cell = stack
                        .pop()
                        .ok_or_else(|| error(*pc, *opcode, "expression stack underflow"))?;
                    let value = if *opcode == 0x170 {
                        let right = self.read_float(right_cell, env, *pc, *opcode)?;
                        self.write_float(left_cell, right, env, *pc, *opcode, journal)?;
                        right
                    } else if (0x175..=0x17e).contains(opcode) {
                        // The compound path reads the right operand first.
                        let right = self.read_float(right_cell, env, *pc, *opcode)?;
                        let left = self.read_float(left_cell, env, *pc, *opcode)?;
                        let binary = match opcode {
                            0x175 => 0x163,
                            0x176 => 0x164,
                            0x177 => 0x160,
                            0x178 => 0x161,
                            0x179 => 0x165,
                            0x17a => 0x166,
                            0x17b => 0x162,
                            0x17c => 0x167,
                            0x17d => 0x168,
                            _ => 0x169,
                        };
                        let value = float_binary(*pc, binary, left, right)?;
                        self.write_float(left_cell, value, env, *pc, *opcode, journal)?;
                        value
                    } else {
                        let left = self.read_float(left_cell, env, *pc, *opcode)?;
                        let right = self.read_float(right_cell, env, *pc, *opcode)?;
                        float_binary(*pc, *opcode, left, right)?
                    };
                    stack.push(FloatCell::Value(value));
                }
                Term::Scalar { pc, tag, .. }
                | Term::Nested { pc, tag, .. }
                | Term::Indexed { pc, tag, .. } => {
                    return Err(error(
                        *pc,
                        *tag,
                        "float operand storage/type is not implemented",
                    ));
                }
            }
        }
        let Some(top) = stack.single() else {
            return Err(error(
                expr.pc,
                expr.kind,
                "expression does not produce exactly one value",
            ));
        };
        let value = self.read_float(top, env, expr.pc, expr.kind)?;
        self.set_result_journaled(Some(value.to_bits()), journal);
        journal.0.push(Undo::Condition(self.condition));
        self.condition = Some(value != 0.0);
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy)]
enum FloatCell {
    Value(f32),
    Integer(u32),
    Bit(u32),
    Float(u32),
    System(u32),
    FrameInt(usize),
    FrameFloat(usize),
}

/// Float operators of 0x140066e60; bitwise, remainder and shifts truncate both sides first.
fn float_binary(pc: usize, opcode: u16, left: f32, right: f32) -> Result<f32, DecodeError> {
    let flag = |value: bool| f32::from(u8::from(value));
    if matches!(opcode, 0x16a..=0x16f | 0x171 | 0x172) && (left.is_nan() || right.is_nan()) {
        return Err(error(pc, opcode, "NaN comparison semantics not verified"));
    }
    Ok(match opcode {
        0x160 => left * right,
        0x161 => left / right,
        0x163 => left + right,
        0x164 => left - right,
        0x162 | 0x165..=0x169 => binary_value(pc, opcode, truncate(left), truncate(right))? as f32,
        0x16a => flag(left > right),
        0x16b => flag(left >= right),
        0x16c => flag(left < right),
        0x16d => flag(left <= right),
        0x16e => flag(left != 0.0 && right != 0.0),
        0x16f => flag(left != 0.0 || right != 0.0),
        0x171 => flag(left == right),
        0x172 => flag(left != right),
        _ => return Err(error(pc, opcode, "unsupported float operator")),
    })
}

#[derive(Debug, Clone, Copy)]
enum Cell {
    Value(i32),
    Literal(i32),
    Integer(u32),
    Bit(u32),
    /// Absolute stack byte offset of a frame word.
    Frame(usize),
    Data(usize),
}

pub(crate) fn binary_value(
    pc: usize,
    opcode: u16,
    left: i32,
    right: i32,
) -> Result<i32, DecodeError> {
    Ok(match opcode {
        0x160 => left.wrapping_mul(right),
        0x161 => left
            .checked_div(right)
            .ok_or_else(|| error(pc, opcode, "invalid signed division"))?,
        0x162 => left
            .checked_rem(right)
            .ok_or_else(|| error(pc, opcode, "invalid signed remainder"))?,
        0x163 => left.wrapping_add(right),
        0x164 => left.wrapping_sub(right),
        0x165 => left & right,
        0x166 => left | right,
        0x167 => left ^ right,
        0x168 => left.wrapping_shl(right as u32),
        0x169 => left.wrapping_shr(right as u32),
        0x16a => i32::from(left > right),
        0x16b => i32::from(left >= right),
        0x16c => i32::from(left < right),
        0x16d => i32::from(left <= right),
        0x16e => i32::from(left != 0 && right != 0),
        0x16f => i32::from(left != 0 || right != 0),
        0x171 => i32::from(left == right),
        0x172 => i32::from(left != right),
        _ => return Err(error(pc, opcode, "unsupported integer operator")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_bank_fill_set_and_restore_preserve_values_and_counts() {
        let mut bank = BitBank::default();
        assert_eq!(bank.known_count, 0);
        assert_eq!(bank.set(63, true), None);
        assert_eq!(bank.set(64, false), None);
        assert_eq!(bank.known_count, 2);

        // Overlaps existing bit 63..=64 across two u64 word boundaries.
        bank.fill(60, 70, true);
        assert_eq!(bank.known_count, 70);
        assert_eq!(bank.get(59), None);
        assert_eq!(bank.get(60), Some(true));
        assert_eq!(bank.get(64), Some(true));
        assert_eq!(bank.get(129), Some(true));
        assert_eq!(bank.get(130), None);

        // Clearing inside an already-filled span leaves known count unchanged.
        bank.fill(64, 10, false);
        assert_eq!(bank.known_count, 70);
        assert_eq!(bank.get(64), Some(false));
        assert_eq!(bank.get(73), Some(false));
        assert_eq!(bank.get(74), Some(true));

        bank.restore(60, None);
        bank.restore(64, Some(true));
        assert_eq!(bank.known_count, 69);
        assert_eq!(bank.get(60), None);
        assert_eq!(bank.get(64), Some(true));
        assert_eq!(bank.to_btree_map().len(), 69);
    }

    #[test]
    fn word_bank_fill_and_rollback_preserve_counts_and_unknowns() {
        let mut bank = WordBank::default();
        assert_eq!(bank.set(5, 10), None);
        bank.fill(3, 6, -7);
        assert_eq!(bank.known_count, 6);
        assert_eq!(bank.get(2), None);
        assert_eq!(bank.get(5), Some(-7));
        assert_eq!(bank.get(8), Some(-7));

        bank.restore(3, None);
        bank.restore(5, Some(10));
        assert_eq!(bank.known_count, 5);
        assert_eq!(bank.get(3), None);
        assert_eq!(bank.get(5), Some(10));
        assert_eq!(bank.to_btree_map().len(), 5);
    }

    #[test]
    fn debug_preserves_logical_state_after_rollback() {
        for initialized in [false, true] {
            let mut state = State::default();
            if initialized {
                state.set_integer(0, -7).unwrap();
                state.set_bit(0, false).unwrap();
                state.fill_floats(0, 1, 0x7fc0_1234).unwrap();
                state.set_string(0, b"").unwrap();
            }
            let before = format!("{state:?}");
            let mut journal = Journal::default();
            for value in [42, 99] {
                for (integer, bit, float, string) in [(0, 0, 0, 0), (8191, 262143, 2047, 127)] {
                    state
                        .set_integer_journaled(integer, value, &mut journal)
                        .unwrap();
                    state.set_bit_journaled(bit, true, &mut journal).unwrap();
                    state
                        .set_float_journaled(float, value as u32, &mut journal)
                        .unwrap();
                    state
                        .set_string_journaled(string, b"temporary", &mut journal)
                        .unwrap();
                }
                assert_ne!(format!("{state:?}"), before);
                state.rollback(&mut journal, None, None);
                assert_eq!(format!("{state:?}"), before);
            }
        }
    }

    #[test]
    fn scratch_stack_truncates_on_early_return_and_reuses_capacity() {
        let scalar = |raw| Term::Scalar {
            pc: 0,
            tag: 0x100,
            raw,
        };
        let operator = |opcode| Term::Operator { pc: 0, opcode };
        let expression = |terms| Expression {
            pc: 0,
            kind: 0x200,
            end: 0,
            terms,
        };
        let nested = |expression| Term::Nested {
            pc: 0,
            tag: 0x200,
            expression: Box::new(expression),
        };
        for kind in [0x200, 0x202] {
            let mut state = State::default();
            let mut scratch = EvalScratch::default();
            let mut journal = Journal::default();
            let mut env = Env {
                labels: &[],
                frame: None,
                data: None,
            };
            let mut evaluate = |expr: &Expression, scratch: &mut EvalScratch| {
                if kind == 0x200 {
                    state.evaluate_in(expr, &mut env, scratch, &mut journal)
                } else {
                    state
                        .evaluate_float_in(expr, &mut env, scratch, &mut journal)
                        .map(|v| v as i32)
                }
            };
            // Both the outer frame and nested integer frame retain an operand at failure.
            let bad = Expression {
                kind,
                ..expression(vec![
                    scalar(7),
                    nested(expression(vec![
                        scalar(9),
                        scalar(1),
                        scalar(0),
                        operator(0x161),
                    ])),
                ])
            };
            assert_eq!(
                evaluate(&bad, &mut scratch).unwrap_err().reason,
                "invalid signed division"
            );
            assert!(scratch.int_stack.is_empty());
            assert!(scratch.float_stack.is_empty());
            let capacities = (scratch.int_stack.capacity(), scratch.float_stack.capacity());

            let good = Expression {
                kind,
                ..expression(vec![
                    scalar(3),
                    nested(expression(vec![scalar(4)])),
                    operator(0x163),
                ])
            };
            assert_eq!(evaluate(&good, &mut scratch).unwrap(), 7);
            assert!(scratch.int_stack.is_empty());
            assert!(scratch.float_stack.is_empty());
            assert_eq!(
                (scratch.int_stack.capacity(), scratch.float_stack.capacity()),
                capacities
            );
        }
    }

    #[test]
    fn cvns_strided_arrays_and_frame_scalar_read_write() {
        let scalar = |tag, raw| Term::Scalar { pc: 0, tag, raw };
        let indexed = |tag, base, stride, indices| Term::Indexed {
            pc: 0,
            tag,
            base,
            stride,
            indices,
        };
        let operator = |opcode| Term::Operator { pc: 0, opcode };
        let expr = |terms| Expression {
            pc: 0,
            kind: 0x200,
            end: 0,
            terms,
        };

        let mut state = State::default();
        let mut scratch = EvalScratch::default();
        let mut journal = Journal::default();
        let mut stack = ParameterStack::new(256);
        stack.reserve(0, 128).unwrap();
        let mut data = vec![0u8; 128];

        let mut eval = |stack: &mut ParameterStack, data: &mut [u8], terms: Vec<Term>| {
            let mut env = Env {
                labels: &[],
                frame: Some(Frame { stack, base: 16 }),
                data: Some(data),
            };
            state
                .evaluate_in(&expr(terms), &mut env, &mut scratch, &mut journal)
                .unwrap()
        };

        // 0x10a frame scalar at base(16) + 8 = 24.
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![scalar(0x10a, 8), scalar(0x100, 111), operator(0x170)],
            ),
            111
        );
        assert_eq!(eval(&mut stack, &mut data, vec![scalar(0x10a, 8)]), 111);
        assert_eq!(stack.read_u32_at(24).unwrap(), 111);

        // 0x112 frame 1D array (stride 4) at base(16) + 12 + 4 * 2 = 36 (= 0x10a raw 20).
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![
                    indexed(0x112, 12, 4, vec![expr(vec![scalar(0x100, 2)])]),
                    scalar(0x100, 222),
                    operator(0x170),
                ],
            ),
            222
        );
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![indexed(0x112, 12, 4, vec![expr(vec![scalar(0x100, 2)])])],
            ),
            222
        );
        assert_eq!(eval(&mut stack, &mut data, vec![scalar(0x10a, 20)]), 222);
        assert_eq!(stack.read_u32_at(36).unwrap(), 222);

        // 0x10b frame 1D array with non-4 stride: base(16) + 24 + 12 * 3 = 76.
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![
                    indexed(0x10b, 24, 12, vec![expr(vec![scalar(0x100, 3)])]),
                    scalar(0x100, (-333i32) as u32),
                    operator(0x170),
                ],
            ),
            -333
        );
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![indexed(0x10b, 24, 12, vec![expr(vec![scalar(0x100, 3)])])],
            ),
            -333
        );
        assert_eq!(stack.read_u32_at(76).unwrap(), (-333i32) as u32);

        // 0x113 frame 2D array with non-4 stride: base(16) + 24 + 12 * 2 + 4 * 1 = 68.
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![
                    indexed(
                        0x113,
                        24,
                        12,
                        vec![expr(vec![scalar(0x100, 2)]), expr(vec![scalar(0x100, 1)])],
                    ),
                    scalar(0x100, 444),
                    operator(0x170),
                ],
            ),
            444
        );
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![indexed(
                    0x113,
                    24,
                    12,
                    vec![expr(vec![scalar(0x100, 2)]), expr(vec![scalar(0x100, 1)])],
                )],
            ),
            444
        );
        assert_eq!(stack.read_u32_at(68).unwrap(), 444);

        // 0x107 data 1D array with non-4 stride: 8 + 20 * 2 = 48.
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![
                    indexed(0x107, 8, 20, vec![expr(vec![scalar(0x100, 2)])]),
                    scalar(0x100, (-555i32) as u32),
                    operator(0x170),
                ],
            ),
            -555
        );
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![indexed(0x107, 8, 20, vec![expr(vec![scalar(0x100, 2)])])],
            ),
            -555
        );
        assert_eq!(i32::from_le_bytes(data[48..52].try_into().unwrap()), -555);

        // 0x111 data 2D array with non-4 stride: 8 + 20 * 1 + 4 * 3 = 40.
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![
                    indexed(
                        0x111,
                        8,
                        20,
                        vec![expr(vec![scalar(0x100, 1)]), expr(vec![scalar(0x100, 3)])],
                    ),
                    scalar(0x100, 666),
                    operator(0x170),
                ],
            ),
            666
        );
        assert_eq!(
            eval(
                &mut stack,
                &mut data,
                vec![indexed(
                    0x111,
                    8,
                    20,
                    vec![expr(vec![scalar(0x100, 1)]), expr(vec![scalar(0x100, 3)])],
                )],
            ),
            666
        );
        assert_eq!(i32::from_le_bytes(data[40..44].try_into().unwrap()), 666);
    }
}
