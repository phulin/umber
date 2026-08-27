//! Separate TeX group-save, checkpoint-delta, and operation-undo journals.
//!
//! The journal stores values, never owners. Generation-scoped coordinates are
//! copied in typed words and remain valid because the enclosing generation is
//! the coarse lifetime owner. Dense state stays directly readable: group exits
//! pop the TeX save stack, named checkpoints retain one first-before value per
//! written cell and interval, and fine-grained rollback reuses an attempt-local
//! lane which is cleared at every commit.

use core::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::env::group::GroupFrame;
use crate::env::{StateCell, StateWord};

#[path = "journal/cell.rs"]
mod cell;
use cell::JournalCell;

#[cfg(test)]
#[path = "journal/tests.rs"]
mod tests;

static NEXT_JOURNAL_OWNER: AtomicU64 = AtomicU64::new(1);

/// A stable logical cursor in one generation's checkpoint history.
pub struct JournalCursor<G> {
    owner: u64,
    group_id: u64,
    group_entry_position: u32,
    checkpoint_position: u32,
    group_depth: u32,
    save_stack: SaveStackProjection,
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
        self.owner == other.owner
            && self.group_id == other.group_id
            && self.group_entry_position == other.group_entry_position
            && self.checkpoint_position == other.checkpoint_position
            && self.group_depth == other.group_depth
            && self.save_stack == other.save_stack
    }
}

impl<G> Eq for JournalCursor<G> {}

impl<G> JournalCursor<G> {
    const fn new(
        owner: u64,
        group_id: u64,
        group_entry_position: u32,
        checkpoint_position: u32,
        group_depth: u32,
        save_stack: SaveStackProjection,
    ) -> Self {
        Self {
            owner,
            group_id,
            group_entry_position,
            checkpoint_position,
            group_depth,
            save_stack,
            _brand: PhantomData,
        }
    }

    const fn group_id(self) -> u64 {
        self.group_id
    }

    const fn group_entry_position(self) -> u32 {
        self.group_entry_position
    }

    pub(super) const fn checkpoint_position(self) -> u32 {
        self.checkpoint_position
    }

    pub(super) const fn group_depth(self) -> u32 {
        self.group_depth
    }
}

/// Suffix cursor for one possibly nested fine-grained state operation.
pub struct StateOperation<G> {
    owner: u64,
    serial: u64,
    group_id: u64,
    group_entry_position: u32,
    group_depth: u32,
    checkpoint_position: u32,
    operation_position: u32,
    pending_group_position: u32,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> core::fmt::Debug for StateOperation<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("StateOperation(..)")
    }
}

/// One exact prior cell value for group or operation rollback.
pub(crate) struct Mutation<G> {
    cell: JournalCell,
    pub(crate) before: StateWord<G>,
    pub(crate) before_level: u32,
    /// The TeX group level whose save record this assignment represents.
    /// `None` means TeX would not push a restore record for this write.
    saved_at: u32,
}

impl<G> Mutation<G> {
    pub(crate) fn new(
        cell: StateCell,
        before: StateWord<G>,
        before_level: u32,
        saved_at: Option<u32>,
    ) -> Self {
        debug_assert!(saved_at != Some(0));
        Self {
            cell: JournalCell::pack(cell),
            before,
            before_level,
            saved_at: saved_at.unwrap_or(0),
        }
    }

    pub(crate) fn cell(&self) -> StateCell {
        self.cell.unpack()
    }

    pub(crate) fn saved_at(&self) -> Option<u32> {
        (self.saved_at != 0).then_some(self.saved_at)
    }
}

impl<G> Clone for Mutation<G> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell,
            before: self.before.clone(),
            before_level: self.before_level,
            saved_at: self.saved_at,
        }
    }
}

impl<G> core::fmt::Debug for Mutation<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Mutation")
            .field("cell", &self.cell())
            .field("before_level", &self.before_level)
            .field("saved_at", &self.saved_at())
            .finish_non_exhaustive()
    }
}

/// One entry in the exact ordered state timeline.
pub(crate) enum JournalEntry<G> {
    Mutation(Mutation<G>),
    GroupEnter(GroupFrame),
    GroupExit(GroupFrame),
}

struct GroupSegment<G> {
    id: u64,
    parent: u64,
    frame: GroupFrame,
    entries: Vec<Mutation<G>>,
    checkpoint_pinned: bool,
}

impl<G> Clone for GroupSegment<G> {
    fn clone(&self) -> Self {
        let mut entries = Vec::with_capacity(self.entries.len().saturating_add(1));
        entries.extend(self.entries.iter().cloned());
        Self {
            id: self.id,
            parent: self.parent,
            frame: self.frame,
            entries,
            checkpoint_pinned: self.checkpoint_pinned,
        }
    }
}

/// One compact bidirectional value retained for a checkpoint interval.
pub(crate) struct CheckpointDelta<G> {
    pub(crate) cell: StateCell,
    pub(crate) before: StateWord<G>,
    pub(crate) before_level: u32,
    pub(crate) after: StateWord<G>,
    pub(crate) after_level: u32,
}

/// Accepted journal material temporarily detached while one rooted candidate
/// owns the live dense state. Entries move into this token; they are neither
/// cloned into a checkpoint nor discarded before accept/reject resolves.
pub(crate) struct AcceptedJournalTail<G> {
    checkpoint_entries: Vec<CheckpointDelta<G>>,
    checkpoint_stamps: std::collections::HashMap<StateCell, (u64, usize)>,
    checkpoint_epoch: u64,
    next_group_id: u64,
    accepted_active_ids: Vec<u64>,
    accepted_retained_ids: Vec<u64>,
    other_groups: Vec<GroupSegment<G>>,
    innermost_suffix: Vec<Mutation<G>>,
    save_stack: SaveStackProjection,
}

impl<G> AcceptedJournalTail<G> {
    pub(crate) fn deltas(&self) -> &[CheckpointDelta<G>] {
        &self.checkpoint_entries
    }
}

impl<G> Clone for CheckpointDelta<G> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell,
            before: self.before.clone(),
            before_level: self.before_level,
            after: self.after.clone(),
            after_level: self.after_level,
        }
    }
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

pub(crate) enum RestoredGroups {
    Truncate(usize),
    Replace(Vec<GroupFrame>),
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

/// Split rollback storage for one live revision generation.
pub(crate) struct SaveJournal<G> {
    owner: u64,
    active_groups: Vec<GroupSegment<G>>,
    active_group_entries: usize,
    retained_groups: Vec<GroupSegment<G>>,
    pending_operation_groups: Vec<GroupSegment<G>>,
    spare_group_entries: Vec<Vec<Mutation<G>>>,
    next_group_id: u64,
    checkpoint_entries: Vec<CheckpointDelta<G>>,
    checkpoint_stamps: std::collections::HashMap<StateCell, (u64, usize)>,
    checkpoint_epoch: u64,
    operation_entries: Vec<JournalEntry<G>>,
    active_operations: Vec<u64>,
    next_operation: u64,
    save_stack: SaveStackProjection,
    #[cfg(feature = "profiling")]
    profile: SaveJournalProfile,
}

#[cfg(feature = "profiling")]
#[derive(Clone, Default)]
struct SaveJournalProfile {
    mutations: u64,
    mutation_words: [u64; 8],
    group_enters: u64,
    group_exits: u64,
    append_calls: u64,
    growths: u64,
    bytes_moved_by_growth: u64,
    peak_entries: usize,
    group_depth: usize,
    maximum_group_depth: usize,
    entries_at_maximum_group_depth: usize,
    interval_cells: std::collections::HashSet<StateCell>,
    interval_mutations: u64,
    checkpoints: u64,
    checkpoint_mutations: u64,
    checkpoint_unique_cells: u64,
    maximum_checkpoint_mutations: u64,
    maximum_checkpoint_unique_cells: u64,
    checkpoints_with_open_groups: u64,
    maximum_checkpoint_group_depth: usize,
}

impl<G> Clone for SaveJournal<G> {
    fn clone(&self) -> Self {
        let mut checkpoint_entries =
            Vec::with_capacity(self.checkpoint_entries.len().saturating_add(64));
        checkpoint_entries.extend(self.checkpoint_entries.iter().cloned());
        let mut checkpoint_stamps = self.checkpoint_stamps.clone();
        checkpoint_stamps.reserve(64);
        Self {
            owner: self.owner,
            active_groups: self.active_groups.clone(),
            active_group_entries: self.active_group_entries,
            retained_groups: self.retained_groups.clone(),
            pending_operation_groups: Vec::new(),
            spare_group_entries: Vec::new(),
            next_group_id: self.next_group_id,
            checkpoint_entries,
            checkpoint_stamps,
            checkpoint_epoch: self.checkpoint_epoch,
            operation_entries: Vec::new(),
            active_operations: Vec::new(),
            next_operation: self.next_operation,
            save_stack: self.save_stack,
            #[cfg(feature = "profiling")]
            profile: self.profile.clone(),
        }
    }
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
            active_groups: Vec::new(),
            active_group_entries: 0,
            retained_groups: Vec::new(),
            pending_operation_groups: Vec::new(),
            spare_group_entries: Vec::new(),
            next_group_id: 0,
            checkpoint_entries: Vec::new(),
            checkpoint_stamps: std::collections::HashMap::new(),
            checkpoint_epoch: 1,
            operation_entries: Vec::new(),
            active_operations: Vec::new(),
            next_operation: 0,
            save_stack: SaveStackProjection::default(),
            #[cfg(feature = "profiling")]
            profile: SaveJournalProfile::default(),
        }
    }

    #[must_use]
    pub(crate) fn checkpoint_cursor(&mut self, group_depth: usize) -> JournalCursor<G> {
        let group_depth = u32::try_from(group_depth).expect("group depth fits u32");
        for group in &mut self.active_groups {
            group.checkpoint_pinned = true;
        }
        let (group_id, group_entry_position) = self.active_groups.last().map_or((0, 0), |group| {
            (
                group.id,
                u32::try_from(group.entries.len()).expect("group save segment exceeds u32 entries"),
            )
        });
        let cursor = JournalCursor::new(
            self.owner,
            group_id,
            group_entry_position,
            u32::try_from(self.checkpoint_entries.len())
                .expect("checkpoint journal exceeds u32 entries"),
            group_depth,
            self.save_stack,
        );
        self.advance_checkpoint_epoch();
        cursor
    }

    pub(crate) fn begin_operation(&mut self) -> StateOperation<G> {
        self.next_operation = self
            .next_operation
            .checked_add(1)
            .expect("state operation identity space exhausted");
        let (group_id, group_entry_position) = self.active_groups.last().map_or((0, 0), |group| {
            (
                group.id,
                u32::try_from(group.entries.len()).expect("group save segment exceeds u32 entries"),
            )
        });
        self.active_operations.push(self.next_operation);
        StateOperation {
            owner: self.owner,
            serial: self.next_operation,
            group_id,
            group_entry_position,
            group_depth: u32::try_from(self.active_groups.len()).expect("group depth fits u32"),
            checkpoint_position: u32::try_from(self.checkpoint_entries.len())
                .expect("checkpoint journal exceeds u32 entries"),
            operation_position: u32::try_from(self.operation_entries.len())
                .expect("operation journal exceeds u32 entries"),
            pending_group_position: u32::try_from(self.pending_operation_groups.len())
                .expect("pending group journal exceeds u32 entries"),
            _brand: PhantomData,
        }
    }

    pub(crate) fn commit_operation(&mut self, operation: StateOperation<G>) {
        self.validate_operation(&operation);
        self.active_operations.pop();
        if self.active_operations.is_empty() {
            self.operation_entries.clear();
            for segment in self.pending_operation_groups.drain(..) {
                if segment.checkpoint_pinned {
                    self.retained_groups.push(segment);
                } else {
                    let mut entries = segment.entries;
                    entries.clear();
                    self.spare_group_entries.push(entries);
                }
            }
        }
    }

    pub(crate) fn record_mutation(
        &mut self,
        mutation: Mutation<G>,
        after: StateWord<G>,
        after_level: u32,
    ) {
        #[cfg(feature = "profiling")]
        self.record_profile_mutation(&mutation);
        let cell = mutation.cell();
        let stamped = self.checkpoint_stamps.get(&cell).copied();
        if stamped.map(|(epoch, _)| epoch) != Some(self.checkpoint_epoch) {
            let index = self.checkpoint_entries.len();
            self.checkpoint_stamps
                .insert(cell, (self.checkpoint_epoch, index));
            #[cfg(feature = "profiling")]
            self.record_profile_growth(
                self.checkpoint_entries.len(),
                self.checkpoint_entries.capacity(),
                core::mem::size_of::<CheckpointDelta<G>>(),
            );
            self.checkpoint_entries.push(CheckpointDelta {
                cell,
                before: mutation.before.clone(),
                before_level: mutation.before_level,
                after,
                after_level,
            });
        } else {
            let index = stamped
                .expect("the current checkpoint epoch has an index")
                .1;
            let delta = self
                .checkpoint_entries
                .get_mut(index)
                .expect("checkpoint stamp names its live journal entry");
            delta.after = after;
            delta.after_level = after_level;
        }
        if mutation.saved_at().is_some() {
            let position = u32::try_from(self.len().saturating_add(1))
                .expect("group save stack exceeds u32 entries");
            self.save_stack
                .push(&JournalEntry::Mutation(mutation.clone()), position);
            #[cfg(feature = "profiling")]
            {
                let (group_len, group_capacity) = self
                    .active_groups
                    .last()
                    .map(|group| (group.entries.len(), group.entries.capacity()))
                    .expect("a TeX save has an active group");
                self.record_profile_growth(
                    group_len,
                    group_capacity,
                    core::mem::size_of::<Mutation<G>>(),
                );
            }
            self.active_groups
                .last_mut()
                .expect("a TeX save has an active group")
                .entries
                .push(mutation.clone());
            self.active_group_entries = self.active_group_entries.saturating_add(1);
        }
        if !self.active_operations.is_empty() {
            #[cfg(feature = "profiling")]
            self.record_profile_growth(
                self.operation_entries.len(),
                self.operation_entries.capacity(),
                core::mem::size_of::<JournalEntry<G>>(),
            );
            self.operation_entries
                .push(JournalEntry::Mutation(mutation));
        }
        #[cfg(feature = "profiling")]
        self.record_profile_peak();
    }

    pub(crate) fn record_group_enter(&mut self, frame: GroupFrame) {
        #[cfg(feature = "profiling")]
        self.record_profile_group_enter();
        let entry = JournalEntry::GroupEnter(frame);
        let position = u32::try_from(self.len().saturating_add(1))
            .expect("group save stack exceeds u32 entries");
        self.save_stack.push(&entry, position);
        self.next_group_id = self
            .next_group_id
            .checked_add(1)
            .expect("group segment identity space exhausted");
        let parent = self.active_groups.last().map_or(0, |group| group.id);
        self.active_groups.push(GroupSegment {
            id: self.next_group_id,
            parent,
            frame,
            entries: self.spare_group_entries.pop().unwrap_or_default(),
            checkpoint_pinned: false,
        });
        self.active_group_entries = self.active_group_entries.saturating_add(1);
        if !self.active_operations.is_empty() {
            #[cfg(feature = "profiling")]
            self.record_profile_growth(
                self.operation_entries.len(),
                self.operation_entries.capacity(),
                core::mem::size_of::<JournalEntry<G>>(),
            );
            self.operation_entries.push(entry);
        }
        #[cfg(feature = "profiling")]
        self.record_profile_peak();
    }

    pub(crate) fn record_group_exit(&mut self, frame: GroupFrame) {
        #[cfg(feature = "profiling")]
        self.record_profile_group_exit();
        let entry = JournalEntry::GroupExit(frame);
        let segment = self
            .active_groups
            .pop()
            .expect("group exit has a save segment");
        self.active_group_entries = self
            .active_group_entries
            .saturating_sub(segment.entries.len().saturating_add(1));
        debug_assert_eq!(segment.frame, frame);
        self.save_stack = SaveStackProjection {
            words: frame.save_stack_words_before,
            latest_push: frame.latest_save_push_before,
        };
        if !self.active_operations.is_empty() {
            self.pending_operation_groups.push(segment);
        } else if segment.checkpoint_pinned {
            self.retained_groups.push(segment);
        } else {
            let mut entries = segment.entries;
            entries.clear();
            self.spare_group_entries.push(entries);
        }
        if !self.active_operations.is_empty() {
            #[cfg(feature = "profiling")]
            self.record_profile_growth(
                self.operation_entries.len(),
                self.operation_entries.capacity(),
                core::mem::size_of::<JournalEntry<G>>(),
            );
            self.operation_entries.push(entry);
        }
        #[cfg(feature = "profiling")]
        self.record_profile_peak();
    }

    /// State-owned live words and the journal-relative newest checked push.
    #[must_use]
    pub(crate) const fn save_stack_projection(&self) -> (usize, Option<(u32, usize)>) {
        (self.save_stack.words, self.save_stack.latest_push)
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.active_group_entries
    }

    #[must_use]
    pub(crate) fn retained_len(&self) -> usize {
        self.group_save_len()
            .saturating_add(self.checkpoint_entries.len())
            .saturating_add(self.operation_entries.len())
    }

    #[must_use]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.group_capacity_bytes()
            .saturating_add(
                self.checkpoint_entries
                    .capacity()
                    .saturating_mul(core::mem::size_of::<CheckpointDelta<G>>()),
            )
            .saturating_add(
                self.operation_entries
                    .capacity()
                    .saturating_mul(core::mem::size_of::<JournalEntry<G>>()),
            )
    }

    #[must_use]
    pub(crate) fn entry(&self, index: usize) -> JournalEntry<G> {
        let mut remaining = index;
        for group in &self.active_groups {
            if remaining == 0 {
                return JournalEntry::GroupEnter(group.frame);
            }
            remaining -= 1;
            if remaining < group.entries.len() {
                return JournalEntry::Mutation(group.entries[remaining].clone());
            }
            remaining -= group.entries.len();
        }
        panic!("group save entry index out of bounds");
    }

    pub(crate) fn checkpoint_suffix(&self, cursor: JournalCursor<G>) -> &[CheckpointDelta<G>] {
        &self.checkpoint_entries[cursor.checkpoint_position() as usize..]
    }

    /// Moves the accepted suffix out of the live lane and opens an empty
    /// candidate suffix at `cursor`.
    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        cursor: JournalCursor<G>,
    ) -> AcceptedJournalTail<G> {
        assert!(self.validate_cursor(cursor));
        assert!(self.active_operations.is_empty());
        assert!(self.pending_operation_groups.is_empty());

        let checkpoint_entries = self
            .checkpoint_entries
            .split_off(cursor.checkpoint_position() as usize);
        let checkpoint_stamps = std::mem::take(&mut self.checkpoint_stamps);
        let checkpoint_epoch = self.checkpoint_epoch;
        self.advance_checkpoint_epoch();

        let accepted_active_ids = self.active_groups.iter().map(|group| group.id).collect();
        let accepted_retained_ids = self.retained_groups.iter().map(|group| group.id).collect();
        let mut groups = std::mem::take(&mut self.active_groups);
        groups.append(&mut self.retained_groups);
        let mut target_ids = Vec::with_capacity(cursor.group_depth() as usize);
        let mut id = cursor.group_id();
        while id != 0 {
            target_ids.push(id);
            id = groups
                .iter()
                .find(|group| group.id == id)
                .expect("validated group cursor retains its ancestry")
                .parent;
        }
        target_ids.reverse();
        for id in target_ids {
            let index = groups
                .iter()
                .position(|group| group.id == id)
                .expect("validated target group remains in the accepted pool");
            self.active_groups.push(groups.swap_remove(index));
        }
        let innermost_suffix = self
            .active_groups
            .last_mut()
            .map_or_else(Vec::new, |group| {
                group
                    .entries
                    .split_off(cursor.group_entry_position() as usize)
            });
        self.active_group_entries = self
            .active_groups
            .iter()
            .map(|group| group.entries.len().saturating_add(1))
            .sum();
        let save_stack = self.save_stack;
        self.save_stack = cursor.save_stack;

        AcceptedJournalTail {
            checkpoint_entries,
            checkpoint_stamps,
            checkpoint_epoch,
            next_group_id: self.next_group_id,
            accepted_active_ids,
            accepted_retained_ids,
            other_groups: groups,
            innermost_suffix,
            save_stack,
        }
    }

    /// Restores the accepted group/journal topology after the candidate lane
    /// has already been destructively returned to its empty root.
    pub(crate) fn reject_checkpoint_candidate(&mut self, mut tail: AcceptedJournalTail<G>) {
        debug_assert!(self.active_operations.is_empty());
        let mut groups = std::mem::take(&mut self.active_groups);
        groups.append(&mut self.retained_groups);
        groups.append(&mut tail.other_groups);
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.id == tail.accepted_active_ids.last().copied().unwrap_or(0))
        {
            group.entries.append(&mut tail.innermost_suffix);
        }

        let mut take_group = |id| {
            let index = groups
                .iter()
                .position(|group| group.id == id)
                .expect("accepted group id survives candidate rollback");
            groups.swap_remove(index)
        };
        self.active_groups = tail
            .accepted_active_ids
            .into_iter()
            .map(&mut take_group)
            .collect();
        self.retained_groups = tail
            .accepted_retained_ids
            .into_iter()
            .map(&mut take_group)
            .collect();
        self.active_group_entries = self
            .active_groups
            .iter()
            .map(|group| group.entries.len().saturating_add(1))
            .sum();
        self.next_group_id = tail.next_group_id;
        self.save_stack = tail.save_stack;
        self.checkpoint_entries.append(&mut tail.checkpoint_entries);
        self.checkpoint_stamps = tail.checkpoint_stamps;
        self.checkpoint_epoch = tail.checkpoint_epoch;
    }

    pub(crate) fn checkpoint_prefix(&self, cursor: JournalCursor<G>) -> &[CheckpointDelta<G>] {
        &self.checkpoint_entries[..cursor.checkpoint_position() as usize]
    }

    pub(crate) fn operation_suffix(&self, operation: &StateOperation<G>) -> &[JournalEntry<G>] {
        self.validate_operation(operation);
        &self.operation_entries[operation.operation_position as usize..]
    }

    /// Returns one detached entry from an active operation's suffix.
    ///
    /// Rollback uses this narrow accessor so it can release the journal borrow
    /// before mutating the corresponding state cell, without copying the whole
    /// suffix into a temporary allocation.
    pub(crate) fn operation_entry(
        &self,
        operation: &StateOperation<G>,
        index: usize,
    ) -> Option<JournalEntry<G>> {
        self.operation_suffix(operation).get(index).cloned()
    }

    pub(crate) fn truncate_checkpoint(&mut self, cursor: JournalCursor<G>) {
        assert_eq!(
            cursor.owner, self.owner,
            "journal cursor belongs to another state"
        );
        self.checkpoint_entries
            .truncate(cursor.checkpoint_position() as usize);
        self.advance_checkpoint_epoch();
        self.save_stack = cursor.save_stack;
    }

    pub(crate) fn finish_operation_rollback(&mut self, operation: StateOperation<G>) {
        self.validate_operation(&operation);
        while self.active_groups.len() > operation.group_depth as usize {
            self.recycle_active_group();
        }
        while self.pending_operation_groups.len() > operation.pending_group_position as usize {
            let segment = self
                .pending_operation_groups
                .pop()
                .expect("pending operation group suffix");
            self.active_groups.push(segment);
        }
        for entry in self.operation_entries[operation.operation_position as usize..]
            .iter()
            .rev()
        {
            let JournalEntry::Mutation(mutation) = entry else {
                continue;
            };
            let Some(level) = mutation.saved_at() else {
                continue;
            };
            if let Some(group) = self
                .active_groups
                .iter_mut()
                .rev()
                .find(|group| group.frame.level() == level)
            {
                let saved = group
                    .entries
                    .pop()
                    .expect("operation save remains in its group");
                debug_assert_eq!(saved.cell(), mutation.cell());
            }
        }
        if operation.group_id != 0 {
            let group = self
                .active_groups
                .last_mut()
                .expect("operation started in a live group");
            debug_assert_eq!(group.id, operation.group_id);
            group
                .entries
                .truncate(operation.group_entry_position as usize);
        }
        self.operation_entries
            .truncate(operation.operation_position as usize);
        self.active_operations.pop();
        for delta in &self.checkpoint_entries[operation.checkpoint_position as usize..] {
            if self
                .checkpoint_stamps
                .get(&delta.cell)
                .is_some_and(|(epoch, _)| *epoch == self.checkpoint_epoch)
            {
                self.checkpoint_stamps.remove(&delta.cell);
            }
        }
        self.checkpoint_entries
            .truncate(operation.checkpoint_position as usize);
        self.rebuild_save_stack_projection();
    }

    #[must_use]
    pub(crate) fn validate_cursor(&self, cursor: JournalCursor<G>) -> bool {
        if cursor.owner != self.owner
            || cursor.checkpoint_position() as usize > self.checkpoint_entries.len()
        {
            return false;
        }
        if cursor.group_id() == 0 {
            return cursor.group_depth() == 0 && cursor.group_entry_position() == 0;
        }
        if cursor.group_depth() as usize <= self.active_groups.len() {
            let group = &self.active_groups[cursor.group_depth() as usize - 1];
            if group.id == cursor.group_id()
                && cursor.group_entry_position() as usize <= group.entries.len()
            {
                return true;
            }
        }
        let Some(group) = self.group_segment(cursor.group_id()) else {
            return false;
        };
        if cursor.group_entry_position() as usize > group.entries.len() {
            return false;
        }
        let mut depth = 0_u32;
        let mut id = cursor.group_id();
        while id != 0 {
            let Some(group) = self.group_segment(id) else {
                return false;
            };
            depth = depth.saturating_add(1);
            id = group.parent;
        }
        depth == cursor.group_depth()
    }

    pub(crate) fn restore_group_cursor(&mut self, cursor: JournalCursor<G>) -> RestoredGroups {
        if cursor.group_depth() as usize <= self.active_groups.len()
            && (cursor.group_depth() == 0
                || self.active_groups[cursor.group_depth() as usize - 1].id == cursor.group_id())
        {
            while self.active_groups.len() > cursor.group_depth() as usize {
                let segment = self.active_groups.pop().expect("active group suffix");
                self.active_group_entries = self
                    .active_group_entries
                    .saturating_sub(segment.entries.len().saturating_add(1));
                if segment.checkpoint_pinned {
                    self.retained_groups.push(segment);
                } else {
                    self.recycle_segment(segment);
                }
            }
            if let Some(group) = self.active_groups.last_mut() {
                self.active_group_entries = self.active_group_entries.saturating_sub(
                    group
                        .entries
                        .len()
                        .saturating_sub(cursor.group_entry_position() as usize),
                );
                group
                    .entries
                    .truncate(cursor.group_entry_position() as usize);
            }
            self.save_stack = cursor.save_stack;
            return RestoredGroups::Truncate(cursor.group_depth() as usize);
        }
        let mut target = Vec::with_capacity(cursor.group_depth() as usize);
        let mut id = cursor.group_id();
        while id != 0 {
            target.push(id);
            id = self
                .group_segment(id)
                .expect("validated group cursor")
                .parent;
        }
        target.reverse();

        let common = self
            .active_groups
            .iter()
            .map(|group| group.id)
            .zip(target.iter().copied())
            .take_while(|(active, target)| active == target)
            .count();
        while self.active_groups.len() > common {
            let segment = self.active_groups.pop().expect("active group suffix");
            if segment.checkpoint_pinned {
                self.retained_groups.push(segment);
            } else {
                self.recycle_segment(segment);
            }
        }
        for id in target.into_iter().skip(common) {
            let index = self
                .retained_groups
                .iter()
                .position(|group| group.id == id)
                .expect("validated retained group cursor");
            let segment = self.retained_groups.swap_remove(index);
            self.active_groups.push(segment);
        }
        if let Some(group) = self.active_groups.last_mut() {
            group
                .entries
                .truncate(cursor.group_entry_position() as usize);
        }
        self.active_group_entries = self
            .active_groups
            .iter()
            .map(|group| group.entries.len().saturating_add(1))
            .sum();
        self.save_stack = cursor.save_stack;
        RestoredGroups::Replace(self.active_groups.iter().map(|group| group.frame).collect())
    }

    pub(crate) fn visit_group_mutations(&self, mut visit: impl FnMut(&Mutation<G>)) {
        for group in self
            .active_groups
            .iter()
            .chain(&self.retained_groups)
            .chain(&self.pending_operation_groups)
        {
            for mutation in &group.entries {
                visit(mutation);
            }
        }
    }

    pub(crate) fn active_group_frames(&self) -> impl Iterator<Item = GroupFrame> + '_ {
        self.active_groups.iter().map(|group| group.frame)
    }

    pub(crate) fn validate_operation(&self, operation: &StateOperation<G>) {
        assert_eq!(
            operation.owner, self.owner,
            "operation belongs to another state"
        );
        assert_eq!(
            self.active_operations.last().copied(),
            Some(operation.serial),
            "operation is not active"
        );
    }

    fn rebuild_save_stack_projection(&mut self) {
        let mut projection = SaveStackProjection::default();
        let mut position = 0_u32;
        for group in &self.active_groups {
            position = position.checked_add(1).expect("save position fits u32");
            projection.push(&JournalEntry::<G>::GroupEnter(group.frame), position);
            for mutation in &group.entries {
                position = position.checked_add(1).expect("save position fits u32");
                projection.push(&JournalEntry::Mutation(mutation.clone()), position);
            }
        }
        self.save_stack = projection;
        self.active_group_entries = position as usize;
    }

    fn group_segment(&self, id: u64) -> Option<&GroupSegment<G>> {
        self.active_groups
            .iter()
            .chain(&self.retained_groups)
            .chain(&self.pending_operation_groups)
            .find(|group| group.id == id)
    }

    pub(crate) fn group_save_len(&self) -> usize {
        self.active_groups
            .iter()
            .chain(&self.retained_groups)
            .chain(&self.pending_operation_groups)
            .map(|group| group.entries.len().saturating_add(1))
            .sum()
    }

    #[cfg(feature = "profiling")]
    fn group_mutation_len(&self) -> usize {
        self.active_groups
            .iter()
            .chain(&self.retained_groups)
            .chain(&self.pending_operation_groups)
            .map(|group| group.entries.len())
            .sum()
    }

    #[cfg(feature = "profiling")]
    fn group_mutation_capacity(&self) -> usize {
        self.active_groups
            .iter()
            .chain(&self.retained_groups)
            .chain(&self.pending_operation_groups)
            .map(|group| group.entries.capacity())
            .chain(self.spare_group_entries.iter().map(Vec::capacity))
            .sum()
    }

    fn group_capacity_bytes(&self) -> usize {
        let headers = self
            .active_groups
            .capacity()
            .saturating_add(self.retained_groups.capacity())
            .saturating_add(self.pending_operation_groups.capacity())
            .saturating_mul(core::mem::size_of::<GroupSegment<G>>());
        self.active_groups
            .iter()
            .chain(&self.retained_groups)
            .chain(&self.pending_operation_groups)
            .map(|group| {
                group
                    .entries
                    .capacity()
                    .saturating_mul(core::mem::size_of::<Mutation<G>>())
            })
            .chain(self.spare_group_entries.iter().map(|entries| {
                entries
                    .capacity()
                    .saturating_mul(core::mem::size_of::<Mutation<G>>())
            }))
            .fold(headers, usize::saturating_add)
    }

    fn recycle_active_group(&mut self) {
        let segment = self.active_groups.pop().expect("active group segment");
        self.active_group_entries = self
            .active_group_entries
            .saturating_sub(segment.entries.len().saturating_add(1));
        self.recycle_segment(segment);
    }

    fn recycle_segment(&mut self, segment: GroupSegment<G>) {
        let mut entries = segment.entries;
        entries.clear();
        self.spare_group_entries.push(entries);
    }

    fn advance_checkpoint_epoch(&mut self) {
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).unwrap_or_else(|| {
            self.checkpoint_stamps.clear();
            1
        });
    }

    #[cfg(feature = "profiling")]
    fn record_profile_mutation(&mut self, mutation: &Mutation<G>) {
        self.profile.append_calls = self.profile.append_calls.saturating_add(1);
        self.profile.mutations = self.profile.mutations.saturating_add(1);
        let word = match &mutation.before {
            StateWord::Meaning(_) => 0,
            StateWord::Integer(_) => 1,
            StateWord::Dimension(_) => 2,
            StateWord::TokenList(_) => 3,
            StateWord::Glue(_) => 4,
            StateWord::NodeList(_) => 5,
            StateWord::Font(_) => 6,
            StateWord::Code(_) => 7,
        };
        self.profile.mutation_words[word] = self.profile.mutation_words[word].saturating_add(1);
        self.profile.interval_mutations = self.profile.interval_mutations.saturating_add(1);
        self.profile.interval_cells.insert(mutation.cell());
    }

    #[cfg(feature = "profiling")]
    fn record_profile_group_enter(&mut self) {
        self.profile.append_calls = self.profile.append_calls.saturating_add(1);
        self.profile.group_enters = self.profile.group_enters.saturating_add(1);
        self.profile.group_depth = self.profile.group_depth.saturating_add(1);
        if self.profile.group_depth > self.profile.maximum_group_depth {
            self.profile.maximum_group_depth = self.profile.group_depth;
            self.profile.entries_at_maximum_group_depth = self.retained_len() + 1;
        }
    }

    #[cfg(feature = "profiling")]
    fn record_profile_group_exit(&mut self) {
        self.profile.append_calls = self.profile.append_calls.saturating_add(1);
        self.profile.group_exits = self.profile.group_exits.saturating_add(1);
        self.profile.group_depth = self.profile.group_depth.saturating_sub(1);
    }

    #[cfg(feature = "profiling")]
    fn record_profile_growth(&mut self, len: usize, capacity: usize, entry_size: usize) {
        if len == capacity {
            self.profile.growths = self.profile.growths.saturating_add(1);
            self.profile.bytes_moved_by_growth = self.profile.bytes_moved_by_growth.saturating_add(
                u64::try_from(len).unwrap_or(u64::MAX)
                    * u64::try_from(entry_size).unwrap_or(u64::MAX),
            );
        }
    }

    #[cfg(feature = "profiling")]
    fn record_profile_peak(&mut self) {
        self.profile.peak_entries = self.profile.peak_entries.max(self.retained_len());
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn record_checkpoint(&mut self, group_depth: usize) {
        self.profile.checkpoints = self.profile.checkpoints.saturating_add(1);
        self.profile.checkpoint_mutations = self
            .profile
            .checkpoint_mutations
            .saturating_add(self.profile.interval_mutations);
        self.profile.checkpoint_unique_cells = self
            .profile
            .checkpoint_unique_cells
            .saturating_add(u64::try_from(self.profile.interval_cells.len()).unwrap_or(u64::MAX));
        self.profile.maximum_checkpoint_mutations = self
            .profile
            .maximum_checkpoint_mutations
            .max(self.profile.interval_mutations);
        self.profile.maximum_checkpoint_unique_cells = self
            .profile
            .maximum_checkpoint_unique_cells
            .max(u64::try_from(self.profile.interval_cells.len()).unwrap_or(u64::MAX));
        if group_depth != 0 {
            self.profile.checkpoints_with_open_groups =
                self.profile.checkpoints_with_open_groups.saturating_add(1);
            self.profile.maximum_checkpoint_group_depth =
                self.profile.maximum_checkpoint_group_depth.max(group_depth);
        }
        self.profile.interval_cells.clear();
        self.profile.interval_mutations = 0;
    }
}

#[cfg(feature = "profiling")]
impl<G> Drop for SaveJournal<G> {
    fn drop(&mut self) {
        crate::measurement::record_save_journal_census(crate::measurement::SaveJournalCensus {
            entries: u64::try_from(self.retained_len()).unwrap_or(u64::MAX),
            capacity: u64::try_from(
                self.group_mutation_capacity()
                    .saturating_add(self.checkpoint_entries.capacity())
                    .saturating_add(self.operation_entries.capacity()),
            )
            .unwrap_or(u64::MAX),
            peak_entries: u64::try_from(self.profile.peak_entries).unwrap_or(u64::MAX),
            entry_size: u64::try_from(core::mem::size_of::<JournalEntry<G>>()).unwrap_or(u64::MAX),
            mutation_size: u64::try_from(core::mem::size_of::<Mutation<G>>()).unwrap_or(u64::MAX),
            group_frame_size: u64::try_from(core::mem::size_of::<GroupFrame>()).unwrap_or(u64::MAX),
            group_entries: u64::try_from(self.group_mutation_len()).unwrap_or(u64::MAX),
            group_capacity: u64::try_from(self.group_mutation_capacity()).unwrap_or(u64::MAX),
            group_entry_size: u64::try_from(core::mem::size_of::<Mutation<G>>())
                .unwrap_or(u64::MAX),
            checkpoint_entries: u64::try_from(self.checkpoint_entries.len()).unwrap_or(u64::MAX),
            checkpoint_capacity: u64::try_from(self.checkpoint_entries.capacity())
                .unwrap_or(u64::MAX),
            checkpoint_entry_size: u64::try_from(core::mem::size_of::<CheckpointDelta<G>>())
                .unwrap_or(u64::MAX),
            operation_entries: u64::try_from(self.operation_entries.len()).unwrap_or(u64::MAX),
            operation_capacity: u64::try_from(self.operation_entries.capacity())
                .unwrap_or(u64::MAX),
            operation_entry_size: u64::try_from(core::mem::size_of::<JournalEntry<G>>())
                .unwrap_or(u64::MAX),
            stamp_entries: u64::try_from(self.checkpoint_stamps.len()).unwrap_or(u64::MAX),
            stamp_capacity: u64::try_from(self.checkpoint_stamps.capacity()).unwrap_or(u64::MAX),
            mutations: self.profile.mutations,
            mutation_words: self.profile.mutation_words,
            group_enters: self.profile.group_enters,
            group_exits: self.profile.group_exits,
            append_calls: self.profile.append_calls,
            growths: self.profile.growths,
            bytes_moved_by_growth: self.profile.bytes_moved_by_growth,
            maximum_group_depth: u64::try_from(self.profile.maximum_group_depth)
                .unwrap_or(u64::MAX),
            entries_at_maximum_group_depth: u64::try_from(
                self.profile.entries_at_maximum_group_depth,
            )
            .unwrap_or(u64::MAX),
            checkpoints: self.profile.checkpoints,
            checkpoint_mutations: self
                .profile
                .checkpoint_mutations
                .saturating_add(self.profile.interval_mutations),
            checkpoint_unique_cells: self.profile.checkpoint_unique_cells.saturating_add(
                u64::try_from(self.profile.interval_cells.len()).unwrap_or(u64::MAX),
            ),
            maximum_checkpoint_mutations: self
                .profile
                .maximum_checkpoint_mutations
                .max(self.profile.interval_mutations),
            maximum_checkpoint_unique_cells: self
                .profile
                .maximum_checkpoint_unique_cells
                .max(u64::try_from(self.profile.interval_cells.len()).unwrap_or(u64::MAX)),
            checkpoints_with_open_groups: self.profile.checkpoints_with_open_groups,
            maximum_checkpoint_group_depth: u64::try_from(
                self.profile.maximum_checkpoint_group_depth,
            )
            .unwrap_or(u64::MAX),
        });
    }
}

fn canonical_restore_words<G>(mutation: &Mutation<G>) -> Option<usize> {
    mutation.saved_at()?;
    // TeX82 §§275--276 uses one word for `restore_zero` and two for
    // `restore_old_value`. Undefined meanings are physically level zero.
    // Section 240 gives null token-list parameters the same level-zero
    // representation even though Umber's fixed typed bank stores its
    // virtual default at level one.
    Some(
        if mutation.before_level == crate::env::banks::LEVEL_ZERO
            || matches!(
                (mutation.cell(), &mutation.before),
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
