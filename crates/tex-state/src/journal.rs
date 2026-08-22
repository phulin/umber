//! Exact ordered TeX save and operation-undo journal.
//!
//! The journal stores values, never owners. Generation-scoped coordinates are
//! copied in typed words and remain valid because the enclosing generation is
//! the coarse lifetime owner. Group restoration and arbitrary operation
//! rollback share this one ordered history.

use core::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::env::group::GroupFrame;
use crate::env::{StateCell, StateWord};

#[cfg(test)]
#[path = "journal/tests.rs"]
mod tests;

static NEXT_JOURNAL_OWNER: AtomicU64 = AtomicU64::new(1);

/// A stable cursor in one generation's ordered state history.
pub struct JournalCursor<G> {
    owner: u64,
    position: u32,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for JournalCursor<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for JournalCursor<G> {}

impl<G> core::fmt::Debug for JournalCursor<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("JournalCursor(..)")
    }
}

impl<G> PartialEq for JournalCursor<G> {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner && self.position == other.position
    }
}

impl<G> Eq for JournalCursor<G> {}

impl<G> JournalCursor<G> {
    pub(super) const fn new(owner: u64, position: u32) -> Self {
        Self {
            owner,
            position,
            _brand: PhantomData,
        }
    }

    pub(super) const fn position(self) -> u32 {
        self.position
    }
}

/// Why one state mutation appears in the journal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MutationKind {
    Assignment,
    GroupRestore,
}

/// One exact prior/current cell pair.
pub(crate) struct Mutation<G> {
    pub(crate) cell: StateCell,
    pub(crate) before: StateWord<G>,
    pub(crate) before_level: u32,
    pub(crate) after: StateWord<G>,
    pub(crate) after_level: u32,
    /// The TeX group level whose save record this assignment represents.
    /// `None` means TeX would not push a restore record for this write.
    pub(crate) saved_at: Option<u32>,
    pub(crate) kind: MutationKind,
}

impl<G> Clone for Mutation<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for Mutation<G> {}

impl<G> core::fmt::Debug for Mutation<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Mutation")
            .field("cell", &self.cell)
            .field("before_level", &self.before_level)
            .field("after_level", &self.after_level)
            .field("saved_at", &self.saved_at)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// One entry in the exact ordered state timeline.
pub(crate) enum JournalEntry<G> {
    Mutation(Mutation<G>),
    GroupEnter(GroupFrame),
    GroupExit(GroupFrame),
}

impl<G> Clone for JournalEntry<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for JournalEntry<G> {}

impl<G> core::fmt::Debug for JournalEntry<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Mutation(mutation) => mutation.fmt(formatter),
            Self::GroupEnter(frame) => formatter.debug_tuple("GroupEnter").field(frame).finish(),
            Self::GroupExit(frame) => formatter.debug_tuple("GroupExit").field(frame).finish(),
        }
    }
}

/// Append-only history for one live revision generation.
pub(crate) struct SaveJournal<G> {
    owner: u64,
    entries: Vec<JournalEntry<G>>,
}

impl<G> SaveJournal<G> {
    pub(crate) fn dense_copy(
        &self,
        mut map_word: impl FnMut(StateWord<G>) -> Result<StateWord<G>, crate::env::StateError>,
    ) -> Result<Self, crate::env::StateError> {
        let mut destination = Self::new();
        destination
            .entries
            .try_reserve_exact(self.entries.len())
            .map_err(|_| {
                crate::env::StateError::Bank(crate::env::banks::BankError::AllocationFailed)
            })?;
        for entry in &self.entries {
            destination.entries.push(match *entry {
                JournalEntry::Mutation(mutation) => JournalEntry::Mutation(Mutation {
                    cell: mutation.cell,
                    before: map_word(mutation.before)?,
                    before_level: mutation.before_level,
                    after: map_word(mutation.after)?,
                    after_level: mutation.after_level,
                    saved_at: mutation.saved_at,
                    kind: mutation.kind,
                }),
                JournalEntry::GroupEnter(frame) => JournalEntry::GroupEnter(frame),
                JournalEntry::GroupExit(frame) => JournalEntry::GroupExit(frame),
            });
        }
        Ok(destination)
    }

    pub(crate) fn relocate_cursor(
        &self,
        source: &Self,
        cursor: JournalCursor<G>,
    ) -> Result<JournalCursor<G>, crate::env::StateError> {
        if !source.validate_cursor(cursor) || cursor.position() as usize > self.entries.len() {
            return Err(crate::env::StateError::InvalidCursor);
        }
        Ok(JournalCursor::new(self.owner, cursor.position()))
    }

    #[must_use]
    pub(crate) fn new() -> Self {
        let owner = NEXT_JOURNAL_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("state journal identity space exhausted");
        Self {
            owner,
            entries: Vec::new(),
        }
    }

    #[must_use]
    pub(crate) fn cursor(&self) -> JournalCursor<G> {
        JournalCursor::new(
            self.owner,
            u32::try_from(self.entries.len()).expect("state journal exceeds u32 entries"),
        )
    }

    pub(crate) fn push(&mut self, entry: JournalEntry<G>) {
        self.entries.push(entry);
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.entries
            .capacity()
            .saturating_mul(core::mem::size_of::<JournalEntry<G>>())
    }

    #[must_use]
    pub(crate) fn entry(&self, index: usize) -> JournalEntry<G> {
        self.entries[index]
    }

    #[must_use]
    pub(crate) fn suffix(&self, start: usize, end: usize) -> &[JournalEntry<G>] {
        &self.entries[start..end]
    }

    pub(crate) fn truncate(&mut self, cursor: JournalCursor<G>) {
        let position = cursor.position() as usize;
        assert_eq!(
            cursor.owner, self.owner,
            "journal cursor belongs to another state"
        );
        assert!(
            position <= self.entries.len(),
            "journal cursor is past the end"
        );
        self.entries.truncate(position);
    }

    #[must_use]
    pub(crate) fn validate_cursor(&self, cursor: JournalCursor<G>) -> bool {
        cursor.owner == self.owner && cursor.position() as usize <= self.entries.len()
    }
}

impl<G> Default for SaveJournal<G> {
    fn default() -> Self {
        Self::new()
    }
}
