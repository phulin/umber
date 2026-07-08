//! TeX hyphenation patterns and exceptions.
//!
//! The trie is stored as immutable nodes with sorted outgoing edges. This is
//! not Knuth's packed `trie_link`/`trie_char` array layout, but it preserves the
//! same edge labels and hyphen-value semantics used by Liang's algorithm.

use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HyphenationTable {
    nodes: Vec<TrieNode>,
    exceptions: BTreeMap<String, Vec<usize>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TrieNode {
    edges: Vec<(char, usize)>,
    values: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatternSpec {
    pub letters: Vec<char>,
    pub values: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExceptionSpec {
    pub word: String,
    pub positions: Vec<usize>,
}

impl HyphenationTable {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: vec![TrieNode::default()],
            exceptions: BTreeMap::new(),
        }
    }

    pub fn add_pattern(&mut self, pattern: PatternSpec) {
        if pattern.letters.is_empty() {
            return;
        }
        let mut node = 0usize;
        for ch in pattern.letters {
            node = self.edge_or_insert(node, ch);
        }
        self.nodes[node].values = pattern.values;
    }

    pub fn add_exception(&mut self, exception: ExceptionSpec) {
        if exception.word.is_empty() {
            return;
        }
        self.exceptions.insert(exception.word, exception.positions);
    }

    #[must_use]
    pub fn hyphen_positions(&self, word: &str, left_min: usize, right_min: usize) -> Vec<usize> {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() < left_min.saturating_add(right_min) {
            return Vec::new();
        }
        if let Some(positions) = self.exceptions.get(word) {
            return filter_bounds(positions.iter().copied(), chars.len(), left_min, right_min);
        }

        let mut decorated = Vec::with_capacity(chars.len() + 2);
        decorated.push('.');
        decorated.extend(chars.iter().copied());
        decorated.push('.');
        let mut values = vec![0u8; decorated.len() + 1];
        for start in 0..decorated.len() {
            let mut node = 0usize;
            for ch in decorated[start..].iter().copied() {
                let Some(next) = self.edge(node, ch) else {
                    break;
                };
                node = next;
                for (i, value) in self.nodes[node].values.iter().copied().enumerate() {
                    let pos = start + i;
                    if pos < values.len() && value > values[pos] {
                        values[pos] = value;
                    }
                }
            }
        }
        filter_bounds(
            values.iter().enumerate().filter_map(|(i, value)| {
                if value % 2 == 1 && i > 0 {
                    Some(i - 1)
                } else {
                    None
                }
            }),
            chars.len(),
            left_min,
            right_min,
        )
    }

    #[must_use]
    pub fn exception(&self, word: &str) -> Option<&[usize]> {
        self.exceptions.get(word).map(Vec::as_slice)
    }

    fn edge(&self, node: usize, ch: char) -> Option<usize> {
        self.nodes[node]
            .edges
            .binary_search_by_key(&ch, |(edge_ch, _)| *edge_ch)
            .ok()
            .map(|index| self.nodes[node].edges[index].1)
    }

    fn edge_or_insert(&mut self, node: usize, ch: char) -> usize {
        match self.nodes[node]
            .edges
            .binary_search_by_key(&ch, |(edge_ch, _)| *edge_ch)
        {
            Ok(index) => self.nodes[node].edges[index].1,
            Err(index) => {
                let next = self.nodes.len();
                self.nodes.push(TrieNode::default());
                self.nodes[node].edges.insert(index, (ch, next));
                next
            }
        }
    }

    pub(crate) fn hash_semantic(&self, hasher: &mut crate::state_hash::StateHasher) {
        hasher.tag(0x70);
        hasher.usize(self.nodes.len());
        for node in &self.nodes {
            hasher.usize(node.edges.len());
            for (ch, target) in &node.edges {
                hasher.u32(*ch as u32);
                hasher.usize(*target);
            }
            hasher.usize(node.values.len());
            for value in &node.values {
                hasher.u8(*value);
            }
        }
        hasher.usize(self.exceptions.len());
        for (word, positions) in &self.exceptions {
            hasher.str(word);
            hasher.usize(positions.len());
            for position in positions {
                hasher.usize(*position);
            }
        }
    }
}

fn filter_bounds(
    positions: impl Iterator<Item = usize>,
    len: usize,
    left_min: usize,
    right_min: usize,
) -> Vec<usize> {
    positions
        .filter(|&pos| pos >= left_min && len.saturating_sub(pos) >= right_min)
        .collect()
}

#[cfg(test)]
mod tests;
