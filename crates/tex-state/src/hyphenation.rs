//! TeX hyphenation patterns and exceptions.
//!
//! The trie is stored as immutable nodes with sorted outgoing edges. This is
//! not Knuth's packed `trie_link`/`trie_char` array layout, but it preserves the
//! same edge labels and hyphen-value semantics used by Liang's algorithm.

use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// pdfTeX's default maximum number of nodes in the hyphenation pattern trie.
pub const PDFTEX_TRIE_SIZE: usize = 1_100_000;
/// TeX82's compiled maximum number of pattern-trie nodes.
pub const TEX82_TRIE_SIZE: usize = 8_000;

const fn default_trie_capacity() -> usize {
    TEX82_TRIE_SIZE
}

const fn default_exception_capacity() -> usize {
    307
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HyphenationExceptionUsage {
    pub occupied: usize,
    pub capacity: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExceptionInsertion {
    Ignored,
    Replaced,
    Allocated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HyphenationCapacityError {
    pub capacity: usize,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct HyphenationTable {
    languages: BTreeMap<u8, LanguageHyphenation>,
    hyphen_codes: BTreeMap<u8, BTreeMap<char, char>>,
    /// TeX82's `trie_not_ready`: pattern insertion remains legal until the
    /// first hyphenation pass initializes the runtime trie.
    ///
    /// A dumped format has already run §1335's `init_trie`, so this live
    /// build-state flag is deliberately absent from the format section and
    /// deserializes as `false`.
    #[serde(skip)]
    patterns_open: bool,
    #[serde(skip)]
    dependency_fingerprints: OnceLock<BTreeMap<(u8, u8), u64>>,
    /// Runtime `trie_size`. This is configuration, not format-image state.
    #[serde(skip, default = "default_trie_capacity")]
    trie_capacity: usize,
    /// TeX82 §1334's `hyph_count` and §934's configured `hyph_size`.
    exception_occupied: usize,
    #[serde(default = "default_exception_capacity")]
    exception_capacity: usize,
}

impl PartialEq for HyphenationTable {
    fn eq(&self, other: &Self) -> bool {
        self.languages == other.languages
            && self.hyphen_codes == other.hyphen_codes
            && self.patterns_open == other.patterns_open
            && self.trie_capacity == other.trie_capacity
            && self.exception_occupied == other.exception_occupied
            && self.exception_capacity == other.exception_capacity
    }
}

impl Eq for HyphenationTable {}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
struct LanguageHyphenation {
    nodes: Vec<TrieNode>,
    exceptions: BTreeMap<String, Vec<usize>>,
}

impl Default for LanguageHyphenation {
    fn default() -> Self {
        Self {
            nodes: vec![TrieNode::default()],
            exceptions: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
struct TrieNode {
    edges: Vec<(char, usize)>,
    values: Vec<u8>,
}

impl LanguageHyphenation {
    fn missing_nodes(&self, letters: &[char]) -> usize {
        let mut node = 0;
        for (index, &ch) in letters.iter().enumerate() {
            let Some(next) = self.nodes[node]
                .edges
                .iter()
                .find_map(|&(edge, next)| (edge == ch).then_some(next))
            else {
                return letters.len() - index;
            };
            node = next;
        }
        0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatternSpec {
    pub letters: Vec<char>,
    pub values: Vec<u8>,
}

impl PatternSpec {
    /// Whether TeX82 §963's `v` is a real trie op rather than
    /// `min_quarterword`.
    ///
    /// Section 964 clears the values outside an explicit `.` boundary before
    /// it computes `v`; a pattern with no remaining nonzero value creates its
    /// letter path but does not occupy that path for duplicate detection.
    #[must_use]
    pub fn has_trie_operation(&self) -> bool {
        self.values.iter().enumerate().any(|(index, &value)| {
            value != 0
                && !(index == 0 && self.letters.first() == Some(&'.'))
                && !(index == self.letters.len() && self.letters.last() == Some(&'.'))
        })
    }

    fn canonicalize_trie_operation(&mut self) {
        if self.letters.first() == Some(&'.')
            && let Some(value) = self.values.first_mut()
        {
            *value = 0;
        }
        if self.letters.last() == Some(&'.')
            && let Some(value) = self.values.get_mut(self.letters.len())
        {
            *value = 0;
        }
        if !self.has_trie_operation() {
            self.values.clear();
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExceptionSpec {
    pub word: String,
    pub positions: Vec<usize>,
}

impl HyphenationTable {
    /// Number of live language-qualified hyphenation exceptions.
    #[must_use]
    pub(crate) const fn exception_usage(&self) -> HyphenationExceptionUsage {
        HyphenationExceptionUsage {
            occupied: self.exception_occupied,
            capacity: self.exception_capacity,
        }
    }

    #[must_use]
    pub fn new() -> Self {
        Self {
            languages: BTreeMap::new(),
            hyphen_codes: BTreeMap::new(),
            patterns_open: true,
            dependency_fingerprints: OnceLock::new(),
            trie_capacity: default_trie_capacity(),
            exception_occupied: 0,
            exception_capacity: default_exception_capacity(),
        }
    }

    /// Overrides pdfTeX's runtime `trie_size` configuration.
    ///
    /// Keeping this explicit permits deterministic capacity-boundary tests and
    /// avoids making exhaustion depend on the host allocator.
    pub fn set_trie_capacity(&mut self, capacity: usize) {
        self.trie_capacity = capacity;
    }

    pub fn set_exception_capacity(&mut self, capacity: usize) {
        self.exception_capacity = capacity;
    }

    #[must_use]
    pub(crate) const fn patterns_open(&self) -> bool {
        self.patterns_open
    }

    pub(crate) fn close_patterns(&mut self) {
        self.patterns_open = false;
    }

    pub(crate) fn validate_frozen(&self) -> Result<(), String> {
        let occupied = self
            .languages
            .values()
            .map(|table| table.exceptions.len())
            .sum::<usize>();
        if occupied != self.exception_occupied || occupied > self.exception_capacity {
            return Err(format!(
                "invalid frozen hyphenation exception occupancy: recorded={}, actual={}, capacity={}",
                self.exception_occupied, occupied, self.exception_capacity
            ));
        }
        let trie_nodes = self
            .languages
            .values()
            .try_fold(0_usize, |total, table| total.checked_add(table.nodes.len()));
        if trie_nodes.is_none_or(|nodes| nodes > self.trie_capacity) {
            return Err(format!(
                "invalid frozen hyphenation trie occupancy: nodes={}, capacity={}",
                trie_nodes.map_or("overflow".to_owned(), |nodes| nodes.to_string()),
                self.trie_capacity
            ));
        }
        for table in self.languages.values() {
            if table.nodes.is_empty() {
                return Err("frozen hyphenation language has no root".to_owned());
            }
            let mut incoming = vec![0_u32; table.nodes.len()];
            for node in &table.nodes {
                let mut previous = None;
                for &(ch, target) in &node.edges {
                    if previous.is_some_and(|prior| prior >= ch) {
                        return Err("non-canonical frozen hyphenation edges".to_owned());
                    }
                    previous = Some(ch);
                    let count = incoming
                        .get_mut(target)
                        .ok_or_else(|| "frozen hyphenation edge target is not live".to_owned())?;
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| "frozen hyphenation edge count overflow".to_owned())?;
                }
            }
            if incoming[0] != 0 || incoming[1..].iter().any(|count| *count != 1) {
                return Err("frozen hyphenation trie is not a rooted tree".to_owned());
            }
            for (word, positions) in &table.exceptions {
                let len = word.chars().count();
                if word.is_empty() || positions.iter().any(|position| *position > len) {
                    return Err("invalid frozen hyphenation exception".to_owned());
                }
            }
        }
        Ok(())
    }

    pub fn add_pattern(&mut self, pattern: PatternSpec) -> Result<(), HyphenationCapacityError> {
        self.add_pattern_for_language(0, pattern).map(|_| ())
    }

    /// Inserts or replaces a pattern and reports whether the same letter path
    /// already carried pattern values (TeX82 §963's duplicate test).
    pub fn add_pattern_for_language(
        &mut self,
        language: u8,
        mut pattern: PatternSpec,
    ) -> Result<bool, HyphenationCapacityError> {
        if pattern.letters.is_empty() {
            return Ok(false);
        }
        let existing_nodes = self
            .languages
            .values()
            .map(|table| table.nodes.len())
            .sum::<usize>();
        let missing_nodes = match self.languages.get(&language) {
            Some(table) => table.missing_nodes(&pattern.letters),
            None => pattern.letters.len() + 1,
        };
        if existing_nodes.saturating_add(missing_nodes) > self.trie_capacity {
            return Err(HyphenationCapacityError {
                capacity: self.trie_capacity,
            });
        }
        self.dependency_fingerprints = OnceLock::new();
        let table = self.languages.entry(language).or_default();
        let mut node = 0usize;
        pattern.canonicalize_trie_operation();
        for ch in pattern.letters {
            node = table.edge_or_insert(node, ch);
        }
        let duplicate = !table.nodes[node].values.is_empty();
        table.nodes[node].values = pattern.values;
        Ok(duplicate)
    }

    /// Reports whether a language already has values on this pattern path.
    ///
    /// TeX82 §963 makes this test while the separator after a pattern is still
    /// current. Canonical scanning uses the read-only query so it can preserve
    /// that error timing while the executor retains ownership of insertion.
    #[must_use]
    pub(crate) fn contains_pattern_for_language(&self, language: u8, letters: &[char]) -> bool {
        let Some(table) = self.languages.get(&language) else {
            return false;
        };
        let mut node = 0usize;
        for &ch in letters {
            let Some(next) = table.nodes[node]
                .edges
                .iter()
                .find_map(|&(edge, next)| (edge == ch).then_some(next))
            else {
                return false;
            };
            node = next;
        }
        !letters.is_empty() && !table.nodes[node].values.is_empty()
    }

    pub fn add_exception(&mut self, exception: ExceptionSpec) -> ExceptionInsertion {
        self.add_exception_for_language(0, exception)
    }

    pub fn add_exception_for_language(
        &mut self,
        language: u8,
        exception: ExceptionSpec,
    ) -> ExceptionInsertion {
        // TeX82 §934 enters a completed exception only when `n>1`.
        if exception.word.chars().nth(1).is_none() {
            return ExceptionInsertion::Ignored;
        }
        self.dependency_fingerprints = OnceLock::new();
        let replaced = self
            .languages
            .entry(language)
            .or_default()
            .exceptions
            .insert(exception.word, exception.positions)
            .is_some();
        if replaced {
            ExceptionInsertion::Replaced
        } else {
            self.exception_occupied = self.exception_occupied.saturating_add(1);
            ExceptionInsertion::Allocated
        }
    }

    pub fn save_hyphen_codes(
        &mut self,
        language: u8,
        codes: impl IntoIterator<Item = (char, char)>,
    ) {
        self.dependency_fingerprints = OnceLock::new();
        self.hyphen_codes
            .insert(language, codes.into_iter().collect());
    }

    #[must_use]
    pub fn saved_hyphen_code(&self, language: u8, ch: char) -> Option<Option<char>> {
        self.hyphen_codes
            .get(&language)
            .map(|codes| codes.get(&ch).copied())
    }

    #[must_use]
    pub fn hyphen_positions(&self, word: &str, left_min: usize, right_min: usize) -> Vec<usize> {
        self.hyphen_positions_for_language(0, word, left_min, right_min)
    }

    #[must_use]
    pub fn hyphen_positions_for_language(
        &self,
        language: u8,
        word: &str,
        left_min: usize,
        right_min: usize,
    ) -> Vec<usize> {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() < left_min.saturating_add(right_min) {
            return Vec::new();
        }
        let Some(table) = self.languages.get(&language) else {
            return Vec::new();
        };
        if let Some(positions) = table.exceptions.get(word) {
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
                let Some(next) = table.edge(node, ch) else {
                    break;
                };
                node = next;
                for (i, value) in table.nodes[node].values.iter().copied().enumerate() {
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
        self.exception_for_language(0, word)
    }

    #[must_use]
    pub fn exception_for_language(&self, language: u8, word: &str) -> Option<&[usize]> {
        self.languages
            .get(&language)?
            .exceptions
            .get(word)
            .map(Vec::as_slice)
    }

    #[cfg(test)]
    pub(crate) fn dependency_fingerprint(&self, language: u8, kind: u8) -> u64 {
        assert!(kind < 3, "hyphenation dependency kind is fixed");
        self.dependency_fingerprints
            .get_or_init(|| {
                self.languages
                    .keys()
                    .chain(self.hyphen_codes.keys())
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .flat_map(|language| {
                        (0..3).map(move |kind| {
                            (
                                (language, kind),
                                self.compute_dependency_fingerprint(language, kind),
                            )
                        })
                    })
                    .collect()
            })
            .get(&(language, kind))
            .copied()
            .unwrap_or_else(|| self.compute_dependency_fingerprint(language, kind))
    }

    #[cfg(test)]
    fn compute_dependency_fingerprint(&self, language: u8, kind: u8) -> u64 {
        let mut hasher =
            crate::state_hash::StateHasher::new(0x6879_7068_6465_7000_u64 | u64::from(kind));
        hasher.u8(language);
        match kind {
            0 => {
                let nodes = self.languages.get(&language).map(|table| &table.nodes);
                hasher.usize(nodes.map_or(0, Vec::len));
                if let Some(nodes) = nodes {
                    for node in nodes {
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
                }
            }
            1 => {
                let exceptions = self.languages.get(&language).map(|table| &table.exceptions);
                hasher.usize(exceptions.map_or(0, BTreeMap::len));
                if let Some(exceptions) = exceptions {
                    for (word, positions) in exceptions {
                        hasher.str(word);
                        hasher.usize(positions.len());
                        for position in positions {
                            hasher.usize(*position);
                        }
                    }
                }
            }
            2 => {
                let codes = self.hyphen_codes.get(&language);
                hasher.usize(codes.map_or(0, BTreeMap::len));
                if let Some(codes) = codes {
                    for (from, to) in codes {
                        hasher.u32(*from as u32);
                        hasher.u32(*to as u32);
                    }
                }
            }
            _ => unreachable!("validated hyphenation dependency kind"),
        }
        hasher.finish()
    }
}

impl LanguageHyphenation {
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
