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
        Self {
            cell: self.cell,
            before: self.before.clone(),
            before_level: self.before_level,
            after: self.after.clone(),
            after_level: self.after_level,
            saved_at: self.saved_at,
            kind: self.kind,
        }
    }
}

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

/// Allocation-free projection of the state-owned part of TeX's save stack.
///
/// Command-owned `\aftergroup` words and executor-owned box-spec words are
/// merged by main control. `latest_push` orders state pushes against the
/// command owner's journal-relative aftergroup position so §§1334/273 can
/// report the depth immediately before the newest checked push.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SaveStackProjection {
    words: usize,
    latest_push: Option<(u32, usize)>,
}

impl SaveStackProjection {
    fn push<G>(&mut self, entry: &JournalEntry<G>, position: u32) {
        match entry {
            JournalEntry::Mutation(mutation) => {
                let Some(words) = canonical_restore_words(mutation) else {
                    return;
                };
                self.words = self.words.saturating_add(words);
                self.latest_push = Some((position, words));
            }
            JournalEntry::GroupEnter(frame) => {
                debug_assert_eq!(self.words, frame.save_stack_words_before);
                debug_assert_eq!(self.latest_push, frame.latest_save_push_before);
                self.words = self.words.saturating_add(1);
                self.latest_push = Some((position, 1));
            }
            JournalEntry::GroupExit(frame) => {
                self.words = frame.save_stack_words_before;
                self.latest_push = frame.latest_save_push_before;
            }
        }
    }
}

impl<G> Clone for JournalEntry<G> {
    fn clone(&self) -> Self {
        match self {
            Self::Mutation(mutation) => Self::Mutation(mutation.clone()),
            Self::GroupEnter(frame) => Self::GroupEnter(*frame),
            Self::GroupExit(frame) => Self::GroupExit(*frame),
        }
    }
}

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
    save_stack: SaveStackProjection,
}

impl<G> SaveJournal<G> {
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
            save_stack: SaveStackProjection::default(),
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
        let position = u32::try_from(self.entries.len().saturating_add(1))
            .expect("state journal exceeds u32 entries");
        self.save_stack.push(&entry, position);
        self.entries.push(entry);
    }

    /// State-owned live words and the journal-relative newest checked push.
    #[must_use]
    pub(crate) const fn save_stack_projection(&self) -> (usize, Option<(u32, usize)>) {
        (self.save_stack.words, self.save_stack.latest_push)
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
        self.entries[index].clone()
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
        self.rebuild_save_stack_projection();
    }

    #[must_use]
    pub(crate) fn validate_cursor(&self, cursor: JournalCursor<G>) -> bool {
        cursor.owner == self.owner && cursor.position() as usize <= self.entries.len()
    }

    fn rebuild_save_stack_projection(&mut self) {
        let mut projection = SaveStackProjection::default();
        for (index, entry) in self.entries.iter().enumerate() {
            let position =
                u32::try_from(index.saturating_add(1)).expect("state journal exceeds u32 entries");
            projection.push(entry, position);
        }
        self.save_stack = projection;
    }
}

fn canonical_restore_words<G>(mutation: &Mutation<G>) -> Option<usize> {
    mutation.saved_at?;
    // TeX82 §§275--276 uses one word for `restore_zero` and two for
    // `restore_old_value`. Undefined meanings are physically level zero.
    // Section 240 gives null token-list parameters the same level-zero
    // representation even though Umber's fixed typed bank stores its
    // virtual default at level one.
    Some(
        if mutation.before_level == crate::env::banks::LEVEL_ZERO
            || matches!(
                (mutation.cell, &mutation.before),
                (
                    crate::env::StateCell::TokenParameter(_),
                    crate::env::StateWord::TokenList(None)
                )
            )
        {
            1
        } else {
            2
        },
    )
}

impl<G> Default for SaveJournal<G> {
    fn default() -> Self {
        Self::new()
    }
}
