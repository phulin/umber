//! Compact ownership for TeX string-pool spellings eligible for recycling.
//!
//! TeX's `slow_make_string` needs exact content membership, but it does not
//! need ordered string owners. Bytes are therefore appended once to one dense
//! arena. Compact end offsets delimit entries, and an open-addressed table is
//! the sole lookup index into that owner.

use ahash::AHasher;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

const EMPTY_BUCKET: u32 = u32::MAX;
const MIN_BUCKETS: usize = 8;

/// Opaque suffix coordinate for one speculative string-pool operation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RecycledStringPoolMark {
    bytes: usize,
    entries: usize,
}

/// The post-checkpoint owner suffix detached while a candidate owns the
/// retained prefix. The stored end offsets remain absolute, so rejection can
/// append both lanes directly before rebuilding the sole lookup index.
#[derive(Debug, Default)]
pub(crate) struct RecycledStringPoolSuffix {
    bytes: Vec<u8>,
    ends: Vec<u32>,
}

#[derive(Debug, Default)]
pub(crate) struct RecycledStringPool {
    /// One retained byte per byte of each distinct UTF-8 spelling.
    bytes: Vec<u8>,
    /// One four-byte exclusive end offset per distinct spelling.
    ends: Vec<u32>,
    /// One four-byte entry index per power-of-two lookup bucket. The table is
    /// rebuilt before occupancy exceeds 75%; it owns no spelling bytes.
    buckets: Vec<u32>,
}

impl RecycledStringPool {
    #[cfg(test)]
    fn len(&self) -> usize {
        self.ends.len()
    }

    #[cfg(test)]
    fn character_len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn insert(&mut self, value: &str) -> bool {
        let hash = string_hash(value.as_bytes());
        if self.lookup(value.as_bytes(), hash).is_some() {
            return false;
        }

        self.reserve(1, value.len());
        let index = u32::try_from(self.ends.len())
            .expect("TeX string-pool entry count fits its executable capacity");
        self.bytes.extend_from_slice(value.as_bytes());
        self.ends.push(
            u32::try_from(self.bytes.len())
                .expect("TeX string-pool bytes fit their executable capacity"),
        );
        self.insert_index(index, hash);
        true
    }

    /// Captures the append-only owner frontier without copying membership.
    pub(crate) fn mark(&self) -> RecycledStringPoolMark {
        RecycledStringPoolMark {
            bytes: self.bytes.len(),
            entries: self.ends.len(),
        }
    }

    /// Validates that `mark` still names an exact prefix of this pool.
    pub(crate) fn assert_contains_mark(&self, mark: RecycledStringPoolMark) {
        let byte_boundary = mark.entries.checked_sub(1).map_or(0, |index| {
            self.ends.get(index).copied().unwrap_or(u32::MAX) as usize
        });
        assert!(
            mark.entries <= self.ends.len()
                && mark.bytes <= self.bytes.len()
                && byte_boundary == mark.bytes,
            "string-pool rollback mark belongs to the active suffix"
        );
    }

    /// Removes a speculative suffix while retaining every warmed allocation.
    ///
    /// Most rejected operations append no new spelling and return in constant
    /// time. A rejection that did append rebuilds the sole membership index in
    /// place; speculative rollback is the cold boundary that pays this scan.
    pub(crate) fn rollback_to(&mut self, mark: RecycledStringPoolMark) {
        self.assert_contains_mark(mark);
        if mark.entries == self.ends.len() {
            return;
        }
        self.bytes.truncate(mark.bytes);
        self.ends.truncate(mark.entries);
        self.rebuild_index_in_place();
    }

    /// Gives a checkpoint candidate the existing prefix and retains only the
    /// later owner suffix needed to reject it. The retained prefix never moves.
    pub(crate) fn detach_suffix(
        &mut self,
        mark: RecycledStringPoolMark,
    ) -> RecycledStringPoolSuffix {
        self.assert_contains_mark(mark);
        let suffix = RecycledStringPoolSuffix {
            bytes: self.bytes.split_off(mark.bytes),
            ends: self.ends.split_off(mark.entries),
        };
        self.rebuild_index_in_place();
        suffix
    }

    /// Rejects candidate-local additions and rejoins the detached live suffix.
    pub(crate) fn restore_suffix(
        &mut self,
        mark: RecycledStringPoolMark,
        mut suffix: RecycledStringPoolSuffix,
    ) {
        self.rollback_to(mark);
        self.bytes.append(&mut suffix.bytes);
        self.ends.append(&mut suffix.ends);
        self.rebuild_index_in_place();
    }

    pub(crate) fn to_format_strings(&self) -> BTreeSet<String> {
        self.values()
            .map(|value| {
                std::str::from_utf8(value)
                    .expect("string-pool arena contains appended UTF-8")
                    .to_owned()
            })
            .collect()
    }

    pub(crate) fn from_format_strings(values: &BTreeSet<String>) -> Self {
        let characters = values.iter().map(String::len).sum();
        let mut pool = Self::default();
        pool.reserve(values.len(), characters);
        for value in values {
            assert!(
                pool.insert(value),
                "format string-pool set contains unique values"
            );
        }
        pool
    }

    fn reserve(&mut self, additional_strings: usize, additional_characters: usize) {
        self.bytes.reserve(additional_characters);
        self.ends.reserve(additional_strings);
        let required_entries = self
            .ends
            .len()
            .checked_add(additional_strings)
            .expect("TeX string-pool entry count overflow");
        if required_entries == 0 {
            return;
        }
        let required_buckets = required_entries
            .checked_mul(4)
            .and_then(|scaled| scaled.checked_add(2))
            .map(|scaled| scaled / 3)
            .expect("TeX string-pool lookup capacity overflow")
            .max(MIN_BUCKETS)
            .next_power_of_two();
        if required_buckets > self.buckets.len() {
            self.rebuild_index(required_buckets);
        }
    }

    fn rebuild_index(&mut self, capacity: usize) {
        let mut buckets = vec![EMPTY_BUCKET; capacity];
        for index in 0..self.ends.len() {
            let index = u32::try_from(index)
                .expect("TeX string-pool entry count fits its executable capacity");
            let hash = string_hash(self.value(index));
            insert_bucket(&mut buckets, index, hash);
        }
        self.buckets = buckets;
    }

    fn rebuild_index_in_place(&mut self) {
        self.buckets.fill(EMPTY_BUCKET);
        for index in 0..self.ends.len() {
            let index = u32::try_from(index)
                .expect("TeX string-pool entry count fits its executable capacity");
            let hash = string_hash(self.value(index));
            insert_bucket(&mut self.buckets, index, hash);
        }
    }

    fn insert_index(&mut self, index: u32, hash: u64) {
        debug_assert!(!self.buckets.is_empty());
        insert_bucket(&mut self.buckets, index, hash);
    }

    fn lookup(&self, value: &[u8], hash: u64) -> Option<u32> {
        if self.buckets.is_empty() {
            return None;
        }
        let mask = self.buckets.len() - 1;
        let mut bucket = hash as usize & mask;
        loop {
            let index = self.buckets[bucket];
            if index == EMPTY_BUCKET {
                return None;
            }
            if self.value(index) == value {
                return Some(index);
            }
            bucket = (bucket + 1) & mask;
        }
    }

    fn values(&self) -> impl Iterator<Item = &[u8]> {
        (0..self.ends.len()).map(|index| {
            self.value(
                u32::try_from(index)
                    .expect("TeX string-pool entry count fits its executable capacity"),
            )
        })
    }

    fn value(&self, index: u32) -> &[u8] {
        let index = index as usize;
        let start = index
            .checked_sub(1)
            .map_or(0, |previous| self.ends[previous] as usize);
        &self.bytes[start..self.ends[index] as usize]
    }
}

fn insert_bucket(buckets: &mut [u32], index: u32, hash: u64) {
    let mask = buckets.len() - 1;
    let mut bucket = hash as usize & mask;
    while buckets[bucket] != EMPTY_BUCKET {
        bucket = (bucket + 1) & mask;
    }
    buckets[bucket] = index;
}

fn string_hash(value: &[u8]) -> u64 {
    let mut hasher = AHasher::default();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests;
