//! Bounded MSB-first reader. Lookahead never makes padding into valid packet bits.
use cmvs_core::FormatError;

pub(super) type Result<T> = std::result::Result<T, FormatError>;

pub(super) struct Bits<'a> {
    data: &'a [u8],
    offset: usize,
    cache: u64,
    available: u32,
    remaining: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            cache: 0,
            available: 0,
            remaining: data.len() * 8,
        }
    }

    pub fn remaining(&self) -> usize {
        self.remaining
    }

    #[inline]
    pub fn read(&mut self, n: u32) -> Result<u32> {
        if self.remaining() < n as usize {
            return Err(FormatError("truncated Theora packet"));
        }
        let value = self.peek(n);
        self.consume(n);
        Ok(value)
    }

    #[inline]
    pub fn peek(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if n == 0 {
            return 0;
        }
        if self.available < n {
            self.refill();
        }
        (self.cache >> (64 - n)) as u32
    }

    #[inline]
    pub fn skip(&mut self, n: u32) -> Result<()> {
        if self.remaining() < n as usize {
            return Err(FormatError("truncated Theora packet"));
        }
        if self.available < n {
            self.refill();
        }
        self.consume(n);
        Ok(())
    }

    #[inline]
    fn consume(&mut self, n: u32) {
        self.cache <<= n;
        self.available -= n;
        self.remaining -= n as usize;
    }

    #[inline]
    fn refill(&mut self) {
        // Refill is requested only with fewer than 32 cached bits. Appending a
        // complete word keeps the register below 64 bits and avoids byte walks.
        debug_assert!(self.available < 32);
        if let Some(bytes) = self.data.get(self.offset..self.offset + 4) {
            self.cache |=
                u64::from(u32::from_be_bytes(bytes.try_into().unwrap())) << (32 - self.available);
            self.offset += 4;
            self.available += 32;
        } else {
            while let Some(&byte) = self.data.get(self.offset) {
                self.cache |= u64::from(byte) << (56 - self.available);
                self.available += 8;
                self.offset += 1;
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Entry {
    token: u8,
    bits: u8,
    node: u8,
}

#[derive(Clone, Copy)]
struct FastEntry {
    bits: u8,
    token: u8,
    extra: u16,
}

const FAST_BITS: u8 = 10;
const FAST_SIZE: usize = 1 << FAST_BITS;

pub(super) struct Huffman {
    // A tree has at most 32 leaves and 31 branches. Short codes take one lookup;
    // long codes resume at the node reached by the first eight bits.
    tree: Vec<[i16; 2]>,
    prefix: [Entry; 256],
    fast: Option<Box<[FastEntry; FAST_SIZE]>>,
}

impl Huffman {
    pub fn parse(bits: &mut Bits<'_>) -> Result<Self> {
        fn node(
            bits: &mut Bits<'_>,
            tree: &mut Vec<[i16; 2]>,
            depth: u8,
            leaves: &mut u8,
        ) -> Result<i16> {
            if depth > 32 {
                return Err(FormatError("Theora Huffman code exceeds 32 bits"));
            }
            if bits.read(1)? != 0 {
                *leaves += 1;
                if *leaves > 32 {
                    return Err(FormatError("too many Theora Huffman leaves"));
                }
                return Ok(-1 - bits.read(5)? as i16);
            }
            if tree.len() >= 31 {
                return Err(FormatError("too many Theora Huffman branches"));
            }
            let index = tree.len();
            tree.push([0; 2]);
            let a = node(bits, tree, depth + 1, leaves)?;
            let b = node(bits, tree, depth + 1, leaves)?;
            tree[index] = [a, b];
            Ok(index as i16)
        }
        let mut tree = Vec::with_capacity(31);
        let root = node(bits, &mut tree, 0, &mut 0)?;
        let prefix = std::array::from_fn(|code| {
            let mut current = root;
            let mut n = 0;
            while current >= 0 && n < 8 {
                current = tree[current as usize][(code >> (7 - n)) & 1];
                n += 1;
            }
            if current < 0 {
                Entry {
                    token: (-1 - current) as u8,
                    bits: n,
                    node: 0,
                }
            } else {
                Entry {
                    token: 0,
                    bits: 8,
                    node: current as u8 + 1,
                }
            }
        });
        Ok(Self {
            tree,
            prefix,
            fast: None,
        })
    }

    /// Expand short codes including their payload at setup, instead of extracting
    /// the Huffman code and its extra bits separately for every coefficient.
    pub fn prepare(&mut self, extra_bits: &[u8; 32]) {
        self.fast = Some(Box::new(std::array::from_fn(|code| {
            let entry = self.prefix[code >> (FAST_BITS - 8)];
            let mut n = entry.bits;
            let mut current = if entry.node == 0 {
                -1 - i16::from(entry.token)
            } else {
                i16::from(entry.node - 1)
            };
            while current >= 0 && n < FAST_BITS {
                current = self.tree[current as usize][(code >> (FAST_BITS - 1 - n)) & 1];
                n += 1;
            }
            if current < 0 {
                let token = (-1 - current) as u8;
                let extra = extra_bits[token as usize];
                if n + extra <= FAST_BITS {
                    return FastEntry {
                        bits: n + extra,
                        token,
                        extra: ((code >> (FAST_BITS - n - extra)) & ((1 << extra) - 1)) as u16,
                    };
                }
            }
            FastEntry {
                bits: 255,
                token: 0,
                extra: 0,
            }
        })));
    }

    #[inline]
    pub fn token(&self, bits: &mut Bits<'_>, extra_bits: &[u8; 32]) -> Result<(u8, u32)> {
        if let Some(fast) = &self.fast {
            let entry = fast[bits.peek(u32::from(FAST_BITS)) as usize];
            if entry.bits != 255 {
                bits.skip(u32::from(entry.bits))?;
                return Ok((entry.token, u32::from(entry.extra)));
            }
        }
        let entry = self.prefix[bits.peek(8) as usize];
        if entry.node == 0 {
            // Consume a short Huffman code and its payload with one bounds check
            // and cache shift. Even at the packet tail, padding is never consumed.
            let extra = extra_bits[entry.token as usize];
            let value = bits.read(u32::from(entry.bits + extra))?;
            return Ok((entry.token, value & ((1 << extra) - 1)));
        }
        bits.skip(u32::from(entry.bits))?;
        let mut current = i16::from(entry.node - 1);
        while current >= 0 {
            current = self.tree[current as usize][bits.read(1)? as usize];
        }
        let token = (-1 - current) as u8;
        Ok((token, bits.read(u32::from(extra_bits[token as usize]))?))
    }
}

#[cfg(test)]
pub(super) fn packed(bits: &[u8]) -> Vec<u8> {
    let mut data = vec![0; bits.len().div_ceil(8)];
    for (i, bit) in bits.iter().enumerate() {
        data[i / 8] |= bit << (7 - i % 8);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_reads_match_individual_bits_at_every_alignment_and_width() {
        let data: Vec<u8> = (0..100).map(|v| (v * 71) as u8).collect();
        for offset in 0..8 {
            for n in 0..=32 {
                let mut b = Bits::new(&data);
                b.skip(offset).unwrap();
                let mut expected = 0u32;
                for i in offset..offset + n {
                    expected =
                        (expected << 1) | u32::from((data[i as usize / 8] >> (7 - i % 8)) & 1);
                }
                assert_eq!(b.read(n).unwrap(), expected);
                assert_eq!(b.remaining(), data.len() * 8 - (offset + n) as usize);
            }
        }
        let mut b = Bits::new(&[0xff]);
        b.skip(7).unwrap();
        assert_eq!(b.peek(8), 128);
        assert!(b.read(2).is_err());
        assert_eq!(b.read(1).unwrap(), 1);
        assert!(b.read(1).is_err());
    }

    #[test]
    fn cached_reads_cross_words_and_preserve_short_packet_tails() {
        let data: Vec<u8> = (0..257).map(|i| (i * 73 + 19) as u8).collect();
        for length in 0..=data.len() {
            let data = &data[..length];
            let mut bits = Bits::new(data);
            let mut position = 0usize;
            let mut random = 0x1451_abcdu32;
            while position < data.len() * 8 {
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                let n = (random % 32 + 1).min(bits.remaining() as u32);
                let mut expected = 0;
                for bit in position..position + n as usize {
                    expected = (expected << 1) | u32::from((data[bit / 8] >> (7 - bit % 8)) & 1);
                }
                assert_eq!(bits.read(n).unwrap(), expected);
                position += n as usize;
                assert_eq!(bits.remaining(), data.len() * 8 - position);
            }
            assert_eq!(bits.peek(32), 0);
            assert!(bits.read(1).is_err());
            assert_eq!(bits.read(0).unwrap(), 0);
        }
    }

    #[test]
    fn expanded_tokens_match_tree_decoding_at_packet_tails() {
        fn balanced(out: &mut Vec<u8>, depth: u8, token: u8) {
            if depth == 0 {
                out.push(1);
                out.extend((0..5).rev().map(|bit| (token >> bit) & 1));
            } else {
                out.push(0);
                balanced(out, depth - 1, token);
                balanced(out, depth - 1, token + (1 << (depth - 1)));
            }
        }
        let mut tree = Vec::new();
        balanced(&mut tree, 5, 0);
        let mut long = vec![0; 31];
        for token in 0..32 {
            long.push(1);
            long.extend((0..5).rev().map(|bit| (token >> bit) & 1));
        }
        let extras = super::super::packet::TOKEN_EXTRA_BITS;
        let data: Vec<u8> = (0..32).map(|i| (i * 73 + 19) as u8).collect();
        for description in [vec![1, 1, 0, 1, 0, 1], tree, long] {
            let description = packed(&description);
            let reference = Huffman::parse(&mut Bits::new(&description)).unwrap();
            let mut fast = Huffman::parse(&mut Bits::new(&description)).unwrap();
            fast.prepare(&extras);
            for length in 0..=data.len() {
                for offset in 0..8 {
                    let mut a = Bits::new(&data[..length]);
                    let mut b = Bits::new(&data[..length]);
                    if a.skip(offset).is_err() {
                        continue;
                    }
                    b.skip(offset).unwrap();
                    for _ in 0..32 {
                        let expected = reference.token(&mut a, &extras);
                        let actual = fast.token(&mut b, &extras);
                        assert_eq!(actual, expected);
                        // Errors may consume a Huffman prefix before detecting a
                        // truncated payload, but must never consume padding.
                        if actual.is_err() {
                            break;
                        }
                        assert_eq!(a.remaining(), b.remaining());
                    }
                }
            }
        }
    }

    #[test]
    fn degenerate_and_long_huffman_codes_preserve_packet_boundaries() {
        let tree = packed(&[1, 1, 0, 1, 0, 1]); // Root leaf, token 21.
        let h = Huffman::parse(&mut Bits::new(&tree)).unwrap();
        assert_eq!(h.token(&mut Bits::new(&[]), &[0; 32]).unwrap(), (21, 0));
        let mut extra = [0; 32];
        extra[21] = 9;
        assert_eq!(
            h.token(&mut Bits::new(&[0xaa, 0x80]), &extra).unwrap(),
            (21, 0x155)
        );
        assert!(h.token(&mut Bits::new(&[0xaa]), &extra).is_err());
        // A maximally unbalanced tree: 31 branches, 32 leaves.
        let mut description = vec![0; 31];
        for token in 0..32 {
            description.push(1);
            for bit in (0..5).rev() {
                description.push((token >> bit) & 1);
            }
        }
        let tree = packed(&description);
        let h = Huffman::parse(&mut Bits::new(&tree)).unwrap();
        assert_eq!(h.token(&mut Bits::new(&[0; 4]), &[0; 32]).unwrap(), (0, 0));
        assert_eq!(h.token(&mut Bits::new(&[0x80]), &[0; 32]).unwrap(), (31, 0));
        assert!(h.token(&mut Bits::new(&[0; 3]), &[0; 32]).is_err());
        extra[31] = 3;
        assert_eq!(h.token(&mut Bits::new(&[0xb0]), &extra).unwrap(), (31, 3));
        extra[0] = 2;
        assert!(h.token(&mut Bits::new(&[0; 4]), &extra).is_err());
        assert!(Huffman::parse(&mut Bits::new(&[0; 8])).is_err());
    }
}
