//! Bounded MSB-first reader. Lookahead never makes padding into valid packet bits.
use cmvs_core::FormatError;

pub(super) type Result<T> = std::result::Result<T, FormatError>;

pub(super) struct Bits<'a> {
    data: &'a [u8],
    offset: usize,
    cache: u64,
    available: u32,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            cache: 0,
            available: 0,
        }
    }

    #[cfg(test)]
    pub fn remaining(&self) -> usize {
        (self.data.len() - self.offset) * 8 + self.available as usize
    }

    #[inline]
    pub fn read(&mut self, n: u32) -> Result<u32> {
        debug_assert!(n <= 32);
        if n == 0 {
            return Ok(0);
        }
        if self.available < n {
            self.refill();
            if self.available < n {
                return Err(FormatError("truncated Theora packet"));
            }
        }
        let value = (self.cache >> (64 - n)) as u32;
        self.cache <<= n;
        self.available -= n;
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

    /// Reads up to `max_ones` unary `1` bits followed by a `0` bit using one
    /// cache inspection when enough bits are buffered.
    #[inline]
    pub fn read_unary(&mut self, max_ones: usize) -> Result<usize> {
        debug_assert!(max_ones <= 8);
        if self.available < max_ones as u32 {
            self.refill();
        }
        if self.available >= max_ones as u32 {
            let ones = (!self.cache).leading_zeros() as usize;
            if ones < max_ones {
                let n = ones as u32 + 1;
                self.cache <<= n;
                self.available -= n;
                return Ok(ones);
            }
            let n = max_ones as u32;
            self.cache <<= n;
            self.available -= n;
            return Ok(max_ones);
        }
        for i in 0..max_ones {
            if self.read(1)? == 0 {
                return Ok(i);
            }
        }
        Ok(max_ones)
    }

    #[inline]
    pub fn skip(&mut self, n: u32) -> Result<()> {
        debug_assert!(n <= 32);
        if n == 0 {
            return Ok(());
        }
        if self.available < n {
            self.refill();
            if self.available < n {
                return Err(FormatError("truncated Theora packet"));
            }
        }
        self.cache <<= n;
        self.available -= n;
        Ok(())
    }

    /// Consumes `n` bits immediately after [`Self::peek`], where `n <= peek_bits`.
    /// Because `peek` already called `refill` if `available < peek_bits`, any
    /// remaining shortfall means the packet ended without enough bits.
    #[inline]
    fn consume_after_peek(&mut self, n: u32) -> Result<()> {
        if self.available < n {
            return Err(FormatError("truncated Theora packet"));
        }
        self.cache <<= n;
        self.available -= n;
        Ok(())
    }

    #[inline]
    fn refill(&mut self) {
        // Refill is requested only with fewer than 32 cached bits. Appending a
        // complete word keeps the register below 64 bits and avoids byte walks.
        debug_assert!(self.available < 32);
        if let Some(chunk) = self.data[self.offset..].first_chunk::<4>() {
            self.cache |= u64::from(u32::from_be_bytes(*chunk)) << (32 - self.available);
            self.offset += 4;
            self.available += 32;
        } else {
            self.refill_tail();
        }
    }

    #[cold]
    fn refill_tail(&mut self) {
        while let Some(&byte) = self.data.get(self.offset) {
            self.cache |= u64::from(byte) << (56 - self.available);
            self.available += 8;
            self.offset += 1;
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct FastEntry {
    /// `<= FAST_BITS`: total bits (code + extra) already unpacked in `val`.
    /// `128..255`: code fits in `FAST_BITS` (`bits - 128` code bits), extra read separately.
    /// `255`: code exceeds `FAST_BITS`; `val` holds the tree node at depth `FAST_BITS`.
    bits: u8,
    token: u8,
    val: u16,
}

const FAST_BITS: u8 = 10;
const FAST_SIZE: usize = 1 << FAST_BITS;

fn parse_tree(bits: &mut Bits<'_>) -> Result<(i16, Vec<[i16; 2]>)> {
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
    Ok((root, tree))
}

fn fill_fast_table<T: Copy>(
    root: i16,
    tree: &[[i16; 2]],
    fast: &mut [T; FAST_SIZE],
    make_leaf: impl Fn(u8, u8, usize) -> T,
    make_node: impl Fn(i16) -> T,
) {
    for (code, slot) in fast.iter_mut().enumerate() {
        let mut current = root;
        let mut n = 0u8;
        while current >= 0 && n < FAST_BITS {
            current = tree[current as usize][(code >> (FAST_BITS - 1 - n)) & 1];
            n += 1;
        }
        *slot = if current < 0 {
            make_leaf((-1 - current) as u8, n, code)
        } else {
            make_node(current)
        };
    }
}

#[cfg(test)]
pub(super) struct Huffman {
    // A tree has at most 32 leaves and 31 branches. Short codes take one lookup;
    // longer codes resume from the node reached after FAST_BITS bits.
    tree: Vec<[i16; 2]>,
    fast: Box<[FastEntry; FAST_SIZE]>,
}

#[cfg(test)]
impl Huffman {
    pub fn parse(bits: &mut Bits<'_>) -> Result<Self> {
        Self::parse_inner(bits, None)
    }

    fn parse_inner(bits: &mut Bits<'_>, extra_bits: Option<&[u8; 32]>) -> Result<Self> {
        let (root, tree) = parse_tree(bits)?;
        let mut out = Self {
            tree,
            fast: Box::new(
                [FastEntry {
                    bits: 255,
                    token: 0,
                    val: 0,
                }; FAST_SIZE],
            ),
        };
        out.prepare_from_root(root, extra_bits);
        Ok(out)
    }

    /// Expand short codes (and inline payloads when code + extra fits within `FAST_BITS`)
    /// at setup, instead of traversing the tree for every coefficient.
    pub fn prepare(&mut self, extra_bits: &[u8; 32]) {
        let root = if self.tree.is_empty() {
            // Degenerate single-leaf tree: preserved in entry 0 by initial parse.
            let entry = self.fast[0];
            -1 - i16::from(entry.token)
        } else {
            0
        };
        self.prepare_from_root(root, Some(extra_bits));
    }

    fn prepare_from_root(&mut self, root: i16, extra_bits: Option<&[u8; 32]>) {
        fill_fast_table(
            root,
            &self.tree,
            &mut self.fast,
            |token, n, code| {
                let fast_extra = extra_bits.and_then(|extras| {
                    let extra = extras[token as usize];
                    (n + extra <= FAST_BITS).then_some(extra)
                });
                if let Some(extra) = fast_extra {
                    FastEntry {
                        bits: n + extra,
                        token,
                        val: ((code >> (FAST_BITS - n - extra)) & ((1 << extra) - 1)) as u16,
                    }
                } else {
                    FastEntry {
                        bits: 128 + n,
                        token,
                        val: 0,
                    }
                }
            },
            |current| FastEntry {
                bits: 255,
                token: 0,
                val: current as u16,
            },
        );
    }

    #[inline]
    pub fn token(&self, bits: &mut Bits<'_>, extra_bits: &[u8; 32]) -> Result<(u8, u32)> {
        let entry = self.fast[bits.peek(u32::from(FAST_BITS)) as usize];
        if entry.bits <= FAST_BITS {
            bits.consume_after_peek(u32::from(entry.bits))?;
            return Ok((entry.token, u32::from(entry.val)));
        }
        if entry.bits < 255 {
            bits.consume_after_peek(u32::from(entry.bits - 128))?;
            return Ok((
                entry.token,
                bits.read(u32::from(extra_bits[entry.token as usize]))?,
            ));
        }
        bits.consume_after_peek(u32::from(FAST_BITS))?;
        let mut current = entry.val as i16;
        while current >= 0 {
            current = self.tree[current as usize][bits.read(1)? as usize];
        }
        let token = (-1 - current) as u8;
        Ok((token, bits.read(u32::from(extra_bits[token as usize]))?))
    }
}

#[derive(Clone, Copy)]
struct CoeffEntry {
    /// `0..=FAST_BITS`: resolved coefficient (`bits = tag`, `aux = run`, `val = coeff`).
    /// `16..=26`: resolved EOB (`bits = tag - 16`, `aux = remaining eobs` in `0..=30`).
    /// `128..255`: code fits in `FAST_BITS` (`code_bits = tag - 128`, `aux = token`), read extra.
    /// `255`: code exceeds `FAST_BITS` (`aux = tree_node_idx`).
    tag: u8,
    aux: u8,
    val: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CoeffToken {
    /// EOB followed by `remaining` queued EOBs, or `None` for EOBRUN 0 (clear all remaining).
    Eob(Option<usize>),
    /// Zero run plus signed quantized coefficient (or `(run, 0)` for a pure zero run).
    Coeff(usize, i16),
}

pub(super) struct CoeffHuffman {
    tree: Vec<[i16; 2]>,
    fast: Box<[CoeffEntry; FAST_SIZE]>,
}

impl CoeffHuffman {
    pub fn parse(
        bits: &mut Bits<'_>,
        extra_bits: &[u8; 32],
        resolve: fn(u8, u32) -> (u8, i16),
    ) -> Result<Self> {
        let (root, tree) = parse_tree(bits)?;
        let mut fast = Box::new(
            [CoeffEntry {
                tag: 255,
                aux: 0,
                val: 0,
            }; FAST_SIZE],
        );
        fill_fast_table(
            root,
            &tree,
            &mut fast,
            |token, n, code| {
                let extra_len = extra_bits[token as usize];
                if n + extra_len <= FAST_BITS {
                    let extra =
                        ((code >> (FAST_BITS - n - extra_len)) & ((1 << extra_len) - 1)) as u32;
                    if token < 7 {
                        // Tokens 0..=5 fit in <= 10 bits and always produce EOB run 1..=31.
                        let eob_run = match token {
                            0..=2 => token as u32 + 1,
                            3..=5 => (1 << (token - 1)) + extra,
                            _ => unreachable!("token 6 requires 12 extra bits > FAST_BITS"),
                        };
                        CoeffEntry {
                            tag: 16 + n + extra_len,
                            aux: (eob_run - 1) as u8,
                            val: 0,
                        }
                    } else {
                        let (run, val) = resolve(token, extra);
                        CoeffEntry {
                            tag: n + extra_len,
                            aux: run,
                            val,
                        }
                    }
                } else {
                    CoeffEntry {
                        tag: 128 + n,
                        aux: token,
                        val: 0,
                    }
                }
            },
            |current| CoeffEntry {
                tag: 255,
                aux: current as u8,
                val: 0,
            },
        );
        Ok(Self { tree, fast })
    }

    #[inline(always)]
    pub fn next(
        &self,
        bits: &mut Bits<'_>,
        extra_bits: &[u8; 32],
        resolve: fn(u8, u32) -> (u8, i16),
    ) -> Result<CoeffToken> {
        let entry = self.fast[bits.peek(u32::from(FAST_BITS)) as usize];
        if entry.tag <= FAST_BITS {
            bits.consume_after_peek(u32::from(entry.tag))?;
            return Ok(CoeffToken::Coeff(usize::from(entry.aux), entry.val));
        }
        if entry.tag <= 16 + FAST_BITS {
            bits.consume_after_peek(u32::from(entry.tag - 16))?;
            return Ok(CoeffToken::Eob(Some(usize::from(entry.aux))));
        }
        self.decode_slow(bits, entry, extra_bits, resolve)
    }

    #[cold]
    #[inline(never)]
    fn decode_slow(
        &self,
        bits: &mut Bits<'_>,
        entry: CoeffEntry,
        extra_bits: &[u8; 32],
        resolve: fn(u8, u32) -> (u8, i16),
    ) -> Result<CoeffToken> {
        let (token, extra) = if entry.tag < 255 {
            bits.consume_after_peek(u32::from(entry.tag - 128))?;
            (
                entry.aux,
                bits.read(u32::from(extra_bits[entry.aux as usize]))?,
            )
        } else {
            bits.consume_after_peek(u32::from(FAST_BITS))?;
            let mut current = i16::from(entry.aux);
            while current >= 0 {
                current = self.tree[current as usize][bits.read(1)? as usize];
            }
            let token = (-1 - current) as u8;
            (token, bits.read(u32::from(extra_bits[token as usize]))?)
        };
        if token < 7 {
            let eob = match token {
                0..=2 => Some(token as usize),
                3..=5 => Some((1 << (token - 1)) + extra as usize - 1),
                _ => {
                    let n = extra as usize;
                    if n == 0 { None } else { Some(n - 1) }
                }
            };
            Ok(CoeffToken::Eob(eob))
        } else {
            let (run, val) = resolve(token, extra);
            Ok(CoeffToken::Coeff(usize::from(run), val))
        }
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

    #[test]
    fn coeff_huffman_matches_reference_across_trees_and_packet_tails() {
        use super::super::packet::{TOKEN_EXTRA_BITS, resolve_coeff};

        fn reference_next(
            h: &Huffman,
            bits: &mut Bits<'_>,
            extras: &[u8; 32],
        ) -> Result<CoeffToken> {
            let (token, extra) = h.token(bits, extras)?;
            if token < 7 {
                let eob = match token {
                    0..=2 => Some(token as usize),
                    3..=5 => Some((1 << (token - 1)) + extra as usize - 1),
                    _ => {
                        if extra == 0 {
                            None
                        } else {
                            Some(extra as usize - 1)
                        }
                    }
                };
                Ok(CoeffToken::Eob(eob))
            } else {
                let (run, val) = resolve_coeff(token, extra);
                Ok(CoeffToken::Coeff(usize::from(run), val))
            }
        }

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

        let mut balanced_tree = Vec::new();
        balanced(&mut balanced_tree, 5, 0);
        let mut long_tree = vec![0; 31];
        for token in 0..32 {
            long_tree.push(1);
            long_tree.extend((0..5).rev().map(|bit| (token >> bit) & 1));
        }

        let extras = TOKEN_EXTRA_BITS;
        let single_eob0 = vec![1, 0, 0, 0, 0, 0];
        let single_eob_long = vec![1, 0, 0, 1, 1, 0]; // token 6 (12 extra bits)
        let single_eob5 = vec![1, 0, 0, 1, 0, 1]; // token 5 (4 extra bits, fast EOB path)
        let single_coeff21 = vec![1, 1, 0, 1, 0, 1]; // token 21 (5 extra bits, fast Coeff path)

        let data: Vec<u8> = (0..32).map(|i| (i * 73 + 19) as u8).collect();
        for description in [
            single_eob0,
            single_eob_long,
            single_eob5,
            single_coeff21,
            balanced_tree,
            long_tree,
        ] {
            let description = packed(&description);
            let reference = Huffman::parse(&mut Bits::new(&description)).unwrap();
            let coeff =
                CoeffHuffman::parse(&mut Bits::new(&description), &extras, resolve_coeff).unwrap();
            for length in 0..=data.len() {
                for offset in 0..8 {
                    let mut a = Bits::new(&data[..length]);
                    let mut b = Bits::new(&data[..length]);
                    if a.skip(offset).is_err() {
                        continue;
                    }
                    b.skip(offset).unwrap();
                    for _ in 0..32 {
                        let expected = reference_next(&reference, &mut a, &extras);
                        let actual = coeff.next(&mut b, &extras, resolve_coeff);
                        assert_eq!(actual, expected);
                        if actual.is_err() {
                            break;
                        }
                        assert_eq!(a.remaining(), b.remaining());
                    }
                }
            }
        }

        // Explicitly exercise EOBRUN 0 (12 zero extra bits -> None) and non-zero EOBRUN 6.
        let tree_eob_long = packed(&[1, 0, 0, 1, 1, 0]);
        let coeff_eob_long =
            CoeffHuffman::parse(&mut Bits::new(&tree_eob_long), &extras, resolve_coeff).unwrap();
        assert_eq!(
            coeff_eob_long
                .next(&mut Bits::new(&[0x00, 0x00]), &extras, resolve_coeff)
                .unwrap(),
            CoeffToken::Eob(None)
        );
        assert_eq!(
            coeff_eob_long
                .next(&mut Bits::new(&[0xff, 0xf0]), &extras, resolve_coeff)
                .unwrap(),
            CoeffToken::Eob(Some(4094))
        );
        assert!(
            coeff_eob_long
                .next(&mut Bits::new(&[0x00]), &extras, resolve_coeff)
                .is_err()
        );
    }
}
