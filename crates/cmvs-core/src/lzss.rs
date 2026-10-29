//! CMVS 2048-byte ring LZSS. See docs/references.md for provenance.
//! The encoder is the binary-tree family shared by CSV2 (0x140068210 / 0x140067ef0 /
//! 0x1400680b0) and CSS1 (0x14008db90), reimplemented from the disassembly.
use crate::{FormatError, MAX_PAYLOAD};

pub fn decode(input: &[u8], expected: usize) -> Result<Vec<u8>, FormatError> {
    if expected > MAX_PAYLOAD {
        return Err(FormatError(
            "decoded size exceeds research allocation limit",
        ));
    }
    let mut ring = [0u8; 2048];
    let mut write = 0x7df;
    let mut cursor = 0;
    let mut flags = 0u16;
    let mut output = Vec::with_capacity(expected);
    while cursor < input.len() {
        flags >>= 1;
        if flags & 0x100 == 0 {
            flags = u16::from(input[cursor]) | 0xff00;
            cursor += 1;
        }
        if flags & 1 != 0 {
            let value = *input
                .get(cursor)
                .ok_or(FormatError("truncated LZSS literal"))?;
            cursor += 1;
            if output.len() == expected {
                return Err(FormatError("LZSS output exceeds declared size"));
            }
            output.push(value);
            ring[write] = value;
            write = (write + 1) & 0x7ff;
        } else {
            let pair = input
                .get(cursor..cursor + 2)
                .ok_or(FormatError("truncated LZSS reference"))?;
            cursor += 2;
            let offset = usize::from(pair[0]) | (usize::from(pair[1] & 0xe0) << 3);
            let length = usize::from(pair[1] & 31) + 2;
            if length > expected - output.len() {
                return Err(FormatError("LZSS reference exceeds declared size"));
            }
            for i in 0..length {
                let value = ring[(offset + i) & 0x7ff];
                output.push(value);
                ring[write] = value;
                write = (write + 1) & 0x7ff;
            }
        }
    }
    if output.len() != expected {
        return Err(FormatError("LZSS decoded length differs from header"));
    }
    Ok(output)
}

/// Binary search trees over a ring window: `window` bytes, `lookahead` bytes compared.
struct Dictionary {
    window: usize,
    lookahead: usize,
    bytes: Vec<u8>,
    left: Vec<usize>,
    right: Vec<usize>,
    parent: Vec<usize>,
    matched: usize,
    position: usize,
}

impl Dictionary {
    fn new(window: usize, lookahead: usize) -> Self {
        // The value `window` is the empty-link sentinel; right[window + 1 + b] roots byte b.
        Self {
            window,
            lookahead,
            bytes: vec![0; window + lookahead - 1],
            left: vec![window; window + 1],
            right: vec![window; window + 257],
            parent: vec![window; window + 1],
            matched: 0,
            position: 0,
        }
    }
    fn insert(&mut self, node: usize) {
        let nil = self.window;
        self.left[node] = nil;
        self.right[node] = nil;
        self.matched = 0;
        let mut at = nil + 1 + usize::from(self.bytes[node]);
        let mut order = 1i32;
        loop {
            let child = if order >= 0 {
                &mut self.right[at]
            } else {
                &mut self.left[at]
            };
            if *child == nil {
                *child = node;
                self.parent[node] = at;
                return;
            }
            at = *child;
            let mut length = 1;
            while length < self.lookahead {
                order = i32::from(self.bytes[node + length]) - i32::from(self.bytes[at + length]);
                if order != 0 {
                    break;
                }
                length += 1;
            }
            // Only a strictly longer match replaces the result; a full match replaces the node.
            if length > self.matched {
                self.matched = length;
                self.position = at;
                if length == self.lookahead {
                    break;
                }
            }
        }
        self.parent[node] = self.parent[at];
        self.left[node] = self.left[at];
        self.right[node] = self.right[at];
        self.parent[self.left[at]] = node;
        self.parent[self.right[at]] = node;
        let parent = self.parent[at];
        if self.right[parent] == at {
            self.right[parent] = node;
        } else {
            self.left[parent] = node;
        }
        self.parent[at] = nil;
    }
    fn remove(&mut self, node: usize) {
        let nil = self.window;
        if self.parent[node] == nil {
            return;
        }
        let mut replacement;
        if self.right[node] == nil {
            replacement = self.left[node];
        } else if self.left[node] == nil {
            replacement = self.right[node];
        } else {
            replacement = self.left[node];
            if self.right[replacement] != nil {
                while self.right[replacement] != nil {
                    replacement = self.right[replacement];
                }
                self.right[self.parent[replacement]] = self.left[replacement];
                self.parent[self.left[replacement]] = self.parent[replacement];
                self.left[replacement] = self.left[node];
                self.parent[self.left[node]] = replacement;
            }
            self.right[replacement] = self.right[node];
            self.parent[self.right[node]] = replacement;
        }
        self.parent[replacement] = self.parent[node];
        let parent = self.parent[node];
        if self.right[parent] == node {
            self.right[parent] = replacement;
        } else {
            self.left[parent] = replacement;
        }
        self.parent[node] = nil;
    }
}

/// Shared tree encoder: literal flag bit 1, flags low bit first, references as
/// `position low byte, high(position) | (length - 2)`. The window starts zeroed and
/// writing begins at `window - lookahead`, which is where both decoders start.
pub(crate) fn encode_tree(
    input: &[u8],
    window: usize,
    lookahead: usize,
    high: fn(usize) -> u8,
) -> Vec<u8> {
    let mut tree = Dictionary::new(window, lookahead);
    let (mut start, mut read) = (0, window - lookahead);
    let mut remaining = input.len().min(lookahead);
    tree.bytes[read..read + remaining].copy_from_slice(&input[..remaining]);
    if remaining == 0 {
        return Vec::new();
    }
    let mut cursor = remaining;
    for i in 1..=lookahead {
        tree.insert(read - i);
    }
    tree.insert(read);
    let mut output = Vec::new();
    let mut group = vec![0];
    let mut mask = 1u8;
    while remaining > 0 {
        let count = tree.matched.min(remaining).max(1);
        if count == 1 {
            group[0] |= mask;
            group.push(tree.bytes[read]);
        } else {
            group.push(tree.position as u8);
            group.push(high(tree.position) | (count - 2) as u8);
        }
        mask = mask.wrapping_mul(2);
        if mask == 0 {
            output.extend_from_slice(&group);
            group.clear();
            group.push(0);
            mask = 1;
        }
        for _ in 0..count {
            tree.remove(start);
            if let Some(&byte) = input.get(cursor) {
                cursor += 1;
                tree.bytes[start] = byte;
                if start < lookahead - 1 {
                    tree.bytes[start + window] = byte;
                }
            } else {
                remaining -= 1;
            }
            start = (start + 1) & (window - 1);
            read = (read + 1) & (window - 1);
            if remaining > 0 {
                tree.insert(read);
            }
        }
    }
    if group.len() > 1 {
        output.extend(group);
    }
    output
}

/// CSV2 section encoder 0x140068210: 2048-byte window, 33-byte lookahead (start 2015),
/// 11-bit positions and 5-bit lengths, the inverse of `decode`. Empty input yields no bytes
/// (the original returns -1 and its callers never pass empty sections).
pub fn encode(input: &[u8]) -> Vec<u8> {
    encode_tree(input, 2048, 33, |position| ((position >> 3) & 0xe0) as u8)
}
