use crate::cell::CellId;
use crate::journal::JournalPos;
use crate::state_hash::exact_identity_bytes;
use ahash::AHashMap;

const ENV_ENTRY_DOMAIN: &[u8] = b"umber-exact-env-entry-v2";
const ENV_IDENTITY_DOMAIN: &[u8] = b"umber-exact-env-identity-v2";

/// Constant-size rollback image of the canonical environment accumulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ExactEnvSnapshot {
    sum: u64,
    xor: u64,
    len: u64,
    journal_pos: JournalPos,
    journal_baseline_serial: u64,
    undo_mark: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    key: u64,
    value: u64,
    atom: u64,
}

#[derive(Clone, Copy, Debug)]
struct Undo {
    cell: CellId,
    previous: Option<Entry>,
}

/// Canonical commutative identity of the current non-default environment cells.
///
/// Each cell contributes one domain-separated atom. Replacing a cell subtracts
/// its former atom and adds its new atom, so mutation and rollback touch fixed
/// state regardless of the number of live cells. `entries` is a current-state
/// lookup only. `undo` retains replacement deltas only while an aggregate
/// snapshot can roll back to them; consuming the last such root clears the
/// suffix directly, without a historical registry or compaction pass.
#[derive(Clone, Debug)]
pub(super) struct ExactEnvIdentity {
    entries: AHashMap<CellId, Entry>,
    undo: Vec<Undo>,
    accumulator: ExactEnvSnapshot,
    #[cfg(test)]
    updates: usize,
}

impl Default for ExactEnvIdentity {
    fn default() -> Self {
        Self {
            entries: AHashMap::new(),
            undo: Vec::new(),
            accumulator: ExactEnvSnapshot {
                sum: 0,
                xor: 0,
                len: 0,
                journal_pos: JournalPos::from_raw(0),
                journal_baseline_serial: 1,
                undo_mark: 0,
            },
            #[cfg(test)]
            updates: 0,
        }
    }
}

impl ExactEnvIdentity {
    pub(super) fn identity(&self) -> u64 {
        identity(self.accumulator)
    }

    pub(super) const fn snapshot(&self) -> ExactEnvSnapshot {
        self.accumulator
    }

    pub(super) fn restore(&mut self, snapshot: ExactEnvSnapshot) {
        assert!(
            snapshot.undo_mark <= self.undo.len(),
            "exact environment snapshot is not an ancestor"
        );
        while self.undo.len() > snapshot.undo_mark {
            let undo = self.undo.pop().expect("checked exact undo length");
            match undo.previous {
                Some(previous) => {
                    self.entries.insert(undo.cell, previous);
                }
                None => {
                    self.entries.remove(&undo.cell);
                }
            }
        }
        self.accumulator = snapshot;
    }

    pub(super) fn discard_undo_history(&mut self) {
        self.undo.clear();
        self.accumulator.undo_mark = 0;
    }

    pub(super) fn reconcile(&mut self, replacement: &Self) {
        let removed = self
            .entries
            .keys()
            .copied()
            .filter(|cell| !replacement.entries.contains_key(cell))
            .collect::<Vec<_>>();
        for cell in removed {
            self.update(cell, 0, None);
        }
        for (&cell, entry) in &replacement.entries {
            self.update(cell, entry.key, Some(entry.value));
        }
    }

    pub(super) const fn journal_cursor(&self) -> (JournalPos, u64) {
        (
            self.accumulator.journal_pos,
            self.accumulator.journal_baseline_serial,
        )
    }

    pub(super) fn mark_journal(&mut self, journal_pos: JournalPos, journal_baseline_serial: u64) {
        self.accumulator.journal_pos = journal_pos;
        self.accumulator.journal_baseline_serial = journal_baseline_serial;
    }

    pub(super) fn update(&mut self, cell: CellId, key: u64, value: Option<u64>) {
        if self
            .entries
            .get(&cell)
            .is_some_and(|entry| Some((entry.key, entry.value)) == value.map(|value| (key, value)))
            || (value.is_none() && !self.entries.contains_key(&cell))
        {
            return;
        }

        #[cfg(test)]
        {
            self.updates += 1;
        }

        self.undo.push(Undo {
            cell,
            previous: self.entries.get(&cell).copied(),
        });
        self.accumulator.undo_mark = self.undo.len();

        if let Some(previous) = self.entries.remove(&cell) {
            self.accumulator.sum = self.accumulator.sum.wrapping_sub(previous.atom);
            self.accumulator.xor ^= previous.atom;
            self.accumulator.len = self
                .accumulator
                .len
                .checked_sub(1)
                .expect("exact environment entry count underflowed");
        }
        if let Some(value) = value {
            let atom = entry_atom(key, value);
            let replaced = self.entries.insert(cell, Entry { key, value, atom });
            debug_assert!(replaced.is_none());
            self.accumulator.sum = self.accumulator.sum.wrapping_add(atom);
            self.accumulator.xor ^= atom;
            self.accumulator.len = self
                .accumulator
                .len
                .checked_add(1)
                .expect("exact environment entry count overflowed");
        }
    }

    pub(super) fn contains(&self, cell: CellId, key: u64, value: Option<u64>) -> bool {
        self.entries
            .get(&cell)
            .map(|entry| (entry.key, entry.value))
            == value.map(|value| (key, value))
    }

    #[cfg(test)]
    pub(super) const fn testing_updates(&self) -> usize {
        self.updates
    }

    #[cfg(test)]
    pub(super) const fn testing_undo_len(&self) -> usize {
        self.undo.len()
    }
}

fn entry_atom(key: u64, value: u64) -> u64 {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&key.to_le_bytes());
    bytes[8..].copy_from_slice(&value.to_le_bytes());
    exact_identity_bytes(ENV_ENTRY_DOMAIN, &bytes)
}

fn identity(accumulator: ExactEnvSnapshot) -> u64 {
    let mut bytes = [0; 24];
    bytes[..8].copy_from_slice(&accumulator.sum.to_le_bytes());
    bytes[8..16].copy_from_slice(&accumulator.xor.to_le_bytes());
    bytes[16..].copy_from_slice(&accumulator.len.to_le_bytes());
    exact_identity_bytes(ENV_IDENTITY_DOMAIN, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::BankTag;

    fn hash(value: u8) -> u64 {
        exact_identity_bytes(b"test", &[value])
    }

    fn cell(index: u32) -> CellId {
        CellId::new(BankTag::Count, index)
    }

    #[test]
    fn insertion_order_does_not_change_identity() {
        let mut forward = ExactEnvIdentity::default();
        let mut reverse = ExactEnvIdentity::default();
        for value in 1..=32 {
            forward.update(cell(u32::from(value)), hash(value), Some(hash(value + 64)));
        }
        for value in (1..=32).rev() {
            reverse.update(cell(u32::from(value)), hash(value), Some(hash(value + 64)));
        }
        assert_eq!(forward.identity(), reverse.identity());
    }

    #[test]
    fn replacement_and_removal_restore_identity() {
        let mut identity = ExactEnvIdentity::default();
        let empty = identity.identity();
        identity.update(cell(1), hash(1), Some(hash(2)));
        let original = identity.identity();
        identity.update(cell(1), hash(1), Some(hash(3)));
        assert_ne!(identity.identity(), original);
        identity.update(cell(1), hash(1), Some(hash(2)));
        assert_eq!(identity.identity(), original);
        identity.update(cell(1), hash(1), None);
        assert_eq!(identity.identity(), empty);
    }

    #[test]
    fn unchanged_and_handle_only_rewrites_do_not_change_identity() {
        let mut env_identity = ExactEnvIdentity::default();
        env_identity.update(cell(1), hash(1), Some(hash(2)));
        let original = env_identity.identity();
        let updates = env_identity.testing_updates();

        // Runtime handles are resolved before this boundary. Replacing one
        // physical representation with the same canonical key/value pair is
        // therefore indistinguishable from an unchanged cell.
        env_identity.update(cell(1), hash(1), Some(hash(2)));
        assert_eq!(env_identity.identity(), original);
        assert_eq!(env_identity.testing_updates(), updates);
    }

    #[test]
    fn snapshots_restore_nested_accumulators() {
        let mut env_identity = ExactEnvIdentity::default();
        env_identity.update(cell(1), hash(1), Some(hash(2)));
        let outer = env_identity.snapshot();
        env_identity.update(cell(2), hash(2), Some(hash(3)));
        let inner = env_identity.snapshot();
        env_identity.update(cell(1), hash(1), Some(hash(4)));
        assert_ne!(env_identity.identity(), identity(inner));
        env_identity.restore(inner);
        assert_eq!(env_identity.identity(), identity(inner));
        env_identity.restore(outer);
        assert_eq!(env_identity.identity(), identity(outer));
    }
}
