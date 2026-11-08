//! Bounded MSB-first reader. Lookahead never makes padding into valid packet bits.
use cmvs_core::FormatError;

pub(super) type Result<T> = std::result::Result<T, FormatError>;

pub(super) struct Bits<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() * 8 - self.position
    }

    #[inline]
    pub fn read(&mut self, n: u32) -> Result<u32> {
        if self.remaining() < n as usize {
            return Err(FormatError("truncated Theora packet"));
        }
        let value = self.peek(n);
        self.position += n as usize;
        Ok(value)
    }

    #[inline]
    pub fn peek(&self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if n == 0 {
            return 0;
        }
        let byte = self.position / 8;
        let shift = self.position % 8;
        let count = (shift + n as usize).div_ceil(8);
        let mut word = 0u64;
        for i in 0..count {
            word = (word << 8) | u64::from(self.data.get(byte + i).copied().unwrap_or(0));
        }
        ((word >> (count * 8 - shift - n as usize)) & ((1u64 << n) - 1)) as u32
    }

    pub fn skip(&mut self, n: u32) -> Result<()> {
        if self.remaining() < n as usize {
            return Err(FormatError("truncated Theora packet"));
        }
        self.position += n as usize;
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct Entry {
    token: u8,
    bits: u8,
    node: u8,
}

pub(super) struct Huffman {
    // A tree has at most 32 leaves and 31 branches. Short codes take one lookup;
    // long codes resume at the node reached by the first eight bits.
    tree: Vec<[i16; 2]>,
    prefix: [Entry; 256],
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
        Ok(Self { tree, prefix })
    }

    #[inline]
    pub fn token(&self, bits: &mut Bits<'_>) -> Result<u8> {
        let entry = self.prefix[bits.peek(8) as usize];
        bits.skip(u32::from(entry.bits))?;
        if entry.node == 0 {
            return Ok(entry.token);
        }
        let mut current = i16::from(entry.node - 1);
        while current >= 0 {
            current = self.tree[current as usize][bits.read(1)? as usize];
        }
        Ok((-1 - current) as u8)
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
    fn degenerate_and_long_huffman_codes_preserve_packet_boundaries() {
        let tree = packed(&[1, 1, 0, 1, 0, 1]); // Root leaf, token 21.
        let h = Huffman::parse(&mut Bits::new(&tree)).unwrap();
        assert_eq!(h.token(&mut Bits::new(&[])).unwrap(), 21);
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
        assert_eq!(h.token(&mut Bits::new(&[0; 4])).unwrap(), 0);
        assert_eq!(h.token(&mut Bits::new(&[0x80])).unwrap(), 31);
        assert!(h.token(&mut Bits::new(&[0; 3])).is_err());
        assert!(Huffman::parse(&mut Bits::new(&[0; 8])).is_err());
    }
}
