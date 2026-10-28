//! CSS1 encoder: 1024-byte dictionary, 48-byte lookahead, binary search trees.
//! Reimplemented from 0x14008db90 / 0x14008d870 / 0x14008da30 (hash in research.md).
const N: usize = 1024;
const F: usize = 48;

struct Dictionary {
    bytes: [u8; N + F - 1],
    left: [usize; N + 1],
    right: [usize; N + 257],
    parent: [usize; N + 1],
    matched: usize,
    position: usize,
}
impl Dictionary {
    fn new() -> Self {
        Self {
            bytes: [0; N + F - 1],
            left: [N; N + 1],
            right: [N; N + 257],
            parent: [N; N + 1],
            matched: 0,
            position: 0,
        }
    }
    fn insert(&mut self, node: usize) {
        self.left[node] = N;
        self.right[node] = N;
        self.matched = 0;
        let mut at = N + 1 + usize::from(self.bytes[node]);
        let mut order = 1i32;
        loop {
            let child = if order >= 0 {
                &mut self.right[at]
            } else {
                &mut self.left[at]
            };
            if *child == N {
                *child = node;
                self.parent[node] = at;
                return;
            }
            at = *child;
            let mut length = 1;
            while length < F {
                order = i32::from(self.bytes[node + length]) - i32::from(self.bytes[at + length]);
                if order != 0 {
                    break;
                }
                length += 1;
            }
            if length > self.matched {
                self.matched = length;
                self.position = at;
                if length == F {
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
        self.parent[at] = N;
    }
    fn remove(&mut self, node: usize) {
        if self.parent[node] == N {
            return;
        }
        let mut replacement;
        if self.right[node] == N {
            replacement = self.left[node];
        } else if self.left[node] == N {
            replacement = self.right[node];
        } else {
            replacement = self.left[node];
            if self.right[replacement] != N {
                while self.right[replacement] != N {
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
        self.parent[node] = N;
    }
}

pub(super) fn compress(input: &[u8]) -> Vec<u8> {
    let mut tree = Dictionary::new();
    let (mut start, mut read) = (0, N - F);
    let mut remaining = input.len().min(F);
    tree.bytes[read..read + remaining].copy_from_slice(&input[..remaining]);
    if remaining == 0 {
        return Vec::new();
    }
    let mut cursor = remaining;
    for i in 1..=F {
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
            group.push(((tree.position >> 2) & 0xc0) as u8 | (count - 2) as u8);
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
                if start < F - 1 {
                    tree.bytes[start + N] = byte;
                }
            } else {
                remaining -= 1;
            }
            start = (start + 1) & (N - 1);
            read = (read + 1) & (N - 1);
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
