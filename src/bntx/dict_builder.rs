//! Patricia-trie builder for the BNTX `_DIC` section.
//!
//! BNTX (and BFRES) use a binary radix trie keyed by name strings. Each
//! node carries:
//!
//! - `ref_bit`: a bit index into the search key. Compared against the
//!   search-key's bit at that position to choose `left` or `right`.
//! - `left`, `right`: indices into the dict's entry array.
//! - `name_ptr`: a file-absolute pointer to the entry's name string.
//!
//! Traversal: start at entry[0] (root, sentinel `ref_bit = 0xFFFFFFFF`),
//! follow `entry[0].left`. At each subsequent entry, look at the search
//! key's bit at position `ref_bit` and follow the corresponding child.
//! Terminate when the next entry's index is <= the current's (a
//! back-edge — the candidate leaf).
//!
//! Bit indexing convention (Nintendo-specific): the key bytes are
//! treated as a big-endian arbitrary-precision integer, then bit 0 of
//! the integer is the LSB of the LAST key byte. So `bit_at(b"abc", 0)`
//! returns the LSB of `c`.
//!
//! Insertion follows the standard Patricia split: find the first
//! differing bit between the new key and the closest existing key,
//! then splice in a new node at the right position in the trie.

use super::DictEntry;

#[derive(Debug, Clone)]
struct TrieNode {
    /// Bit index this node branches on. -1 for the root sentinel.
    bit_inx: i32,
    /// The full byte content of the key associated with this node. (For
    /// the root sentinel, this is an empty Vec.)
    key: Vec<u8>,
    /// `string_index` recorded for use when the trie is materialized into
    /// `DictEntry`s.
    string_index: u32,
    /// Indices into `Trie.nodes`. Children are `[left, right]`.
    children: [usize; 2],
    parent: usize,
}

#[derive(Debug)]
pub struct Trie {
    nodes: Vec<TrieNode>,
}

impl Trie {
    pub fn new() -> Self {
        // Root: bit_inx = -1, both children point to itself.
        Self {
            nodes: vec![TrieNode {
                bit_inx: -1,
                key: Vec::new(),
                string_index: 0,
                children: [0, 0],
                parent: 0,
            }],
        }
    }

    /// Insert a key. `string_index` identifies which entry in
    /// `BntxFile.strings` this name corresponds to.
    ///
    /// Algorithm follows Switch-Toolbox's `ResDict.Tree.Insert` (which
    /// itself implements the standard Patricia-trie split). On the first
    /// insertion the trie's root has both children pointing at itself;
    /// the general logic still routes correctly so we don't special-case.
    pub fn insert(&mut self, key: &[u8], string_index: u32) {
        let current = self.search_prev(key);
        let bit_idx = bit_mismatch(&self.nodes[current].key, key) as i32;

        // Walk up the tree until we find an ancestor whose bit_inx is
        // less than the new node's bit_idx — that's the splice point.
        let mut splice_target = current;
        while bit_idx < self.nodes[self.nodes[splice_target].parent].bit_inx {
            splice_target = self.nodes[splice_target].parent;
        }

        let target_bit = self.nodes[splice_target].bit_inx;
        if bit_idx < target_bit {
            // New node sits ABOVE splice_target.
            let parent_of_target = self.nodes[splice_target].parent;
            let new_idx = self.nodes.len();
            let bit_for_target = bit_at(key, bit_idx as u32);
            let other = bit_for_target ^ 1;
            let mut new_children = [new_idx, new_idx]; // self-loops by default
            new_children[other as usize] = splice_target;

            self.nodes.push(TrieNode {
                bit_inx: bit_idx,
                key: key.to_vec(),
                string_index,
                children: new_children,
                parent: parent_of_target,
            });
            // Update parent_of_target's child slot. Use the bit index of
            // parent_of_target, which may be -1 (root); in that case the
            // branch is always 0 to mirror C#'s `_bit(data, -1) == 0`.
            let parent_branch = self.bit_for_branch(parent_of_target, key) as usize;
            self.nodes[parent_of_target].children[parent_branch] = new_idx;
            self.nodes[splice_target].parent = new_idx;
        } else if bit_idx > target_bit {
            // New node sits BELOW splice_target as one of its children.
            let new_idx = self.nodes.len();
            let key_bit = bit_at(key, bit_idx as u32);
            let other = key_bit ^ 1;
            let mut new_children = [new_idx, new_idx];

            let branch_into_target = self.bit_for_branch(splice_target, key) as usize;
            let displaced = self.nodes[splice_target].children[branch_into_target];

            // The displaced child becomes the OTHER child of the new
            // node, but only if it actually differs from `key` at
            // `bit_idx`; otherwise we point at the root sentinel (per the
            // C# reference).
            let displaced_bit = bit_at(&self.nodes[displaced].key, bit_idx as u32);
            new_children[other as usize] = if displaced_bit == (key_bit ^ 1) {
                displaced
            } else {
                0
            };

            self.nodes.push(TrieNode {
                bit_inx: bit_idx,
                key: key.to_vec(),
                string_index,
                children: new_children,
                parent: splice_target,
            });
            self.nodes[splice_target].children[branch_into_target] = new_idx;
        } else {
            // bit_idx == target_bit — duplicate splitting point. Use the
            // first 1-bit of the new key as the new bit_idx, OR the first
            // mismatch with the existing child.
            let new_idx = self.nodes.len();
            let branch = self.bit_for_branch(splice_target, key) as usize;
            let displaced = self.nodes[splice_target].children[branch];
            let new_bit_idx = if displaced == 0 {
                first_one_bit(key) as i32
            } else {
                bit_mismatch(&self.nodes[displaced].key, key) as i32
            };
            let other = bit_at(key, new_bit_idx as u32) ^ 1;
            let mut new_children = [new_idx, new_idx];
            new_children[other as usize] = displaced;

            self.nodes.push(TrieNode {
                bit_inx: new_bit_idx,
                key: key.to_vec(),
                string_index,
                children: new_children,
                parent: splice_target,
            });
            self.nodes[splice_target].children[branch] = new_idx;
        }
    }

    /// Walk the trie following `key`'s bits and return the index of the
    /// node we'd land on JUST BEFORE the back-edge (i.e., the node whose
    /// child completes the search). This matches `Search(data, prev=true)`
    /// in the reference C# implementation.
    ///
    /// For an empty trie (root.children[0] == root), returns 0 (root).
    fn search_prev(&self, key: &[u8]) -> usize {
        if self.nodes[0].children[0] == 0 {
            return 0;
        }
        let mut node = self.nodes[0].children[0];
        let mut prev = node;
        loop {
            prev = node;
            let next_bit = self.bit_for_branch(node, key);
            node = self.nodes[node].children[next_bit as usize];
            if self.nodes[node].bit_inx <= self.nodes[prev].bit_inx {
                break;
            }
        }
        prev
    }

    /// Determine which child of `node` to follow when searching for `key`.
    /// For a node with `bit_inx == -1` (the root), C#'s `_bit(data, -1)`
    /// returns 0 (because shifting a non-negative BigInteger by a huge
    /// positive number yields 0). We replicate that.
    fn bit_for_branch(&self, node: usize, key: &[u8]) -> u8 {
        let b = self.nodes[node].bit_inx;
        if b < 0 {
            0
        } else {
            bit_at(key, b as u32)
        }
    }

    /// Materialize the trie into a flat `DictEntry` list ordered the same
    /// way Switch-Toolbox / Nintendo emit them: entry[0] is the root
    /// sentinel, then entries are appended in insertion order.
    pub fn to_entries(&self) -> Vec<DictEntry> {
        self.nodes
            .iter()
            .map(|n| DictEntry {
                ref_bit: n.bit_inx as u32, // -1 wraps to 0xFFFFFFFF
                left: n.children[0] as u16,
                right: n.children[1] as u16,
                string_index: n.string_index,
            })
            .collect()
    }
}

/// Extract bit `idx` of `bytes` treating `bytes` as a big-endian
/// arbitrary-precision integer (bit 0 = LSB of last byte).
fn bit_at(bytes: &[u8], idx: u32) -> u8 {
    let total_bits = (bytes.len() * 8) as u32;
    if idx >= total_bits {
        return 0;
    }
    let byte_from_end = (idx / 8) as usize;
    let bit_in_byte = (idx % 8) as u8;
    let byte_idx = bytes.len() - 1 - byte_from_end;
    (bytes[byte_idx] >> bit_in_byte) & 1
}

/// Find the first bit index where `a` and `b` differ. If they're equal
/// (including length), returns `(a.len() * 8).max(b.len() * 8)`.
fn bit_mismatch(a: &[u8], b: &[u8]) -> u32 {
    let max_bits = (a.len().max(b.len()) * 8) as u32;
    for i in 0..max_bits {
        if bit_at(a, i) != bit_at(b, i) {
            return i;
        }
    }
    max_bits
}

/// First bit position (LSB-first) where `bytes` has a 1. Returns 0 if
/// `bytes` is all zero.
fn first_one_bit(bytes: &[u8]) -> u32 {
    let total_bits = (bytes.len() * 8) as u32;
    for i in 0..total_bits {
        if bit_at(bytes, i) == 1 {
            return i;
        }
    }
    0
}

/// Convenience: build a trie from a slice of (key_bytes, string_index)
/// pairs in insertion order. The empty sentinel (string_index 0) is the
/// root and isn't passed in.
pub fn build_trie(entries: &[(&[u8], u32)]) -> Vec<DictEntry> {
    let mut trie = Trie::new();
    for (key, idx) in entries {
        trie.insert(key, *idx);
    }
    trie.to_entries()
}
