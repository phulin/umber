//! Separate TeX group-save, checkpoint-delta, and operation-undo journals.
//!
//! The journal stores values, never owners. Generation-scoped coordinates are
//! copied in typed words and remain valid because the enclosing generation is
//! the coarse lifetime owner. Dense state stays directly readable: group exits
//! pop the TeX save stack, named checkpoints retain one reversible alternate
//! value per written cell and interval in a forked chunk lane, and fine-grained
//! rollback reuses an attempt-local lane which is cleared at every commit.

use core::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::env::group::GroupFrame;
use crate::env::{StateCell, StateWord};
use crate::fork_arena::{CheckpointMark, ChunkPool, ForkArena};

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
    checkpoint: CheckpointMark<DenseJournalLane>,
    checkpoint_entries: u32,
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
            && self.checkpoint_entries == other.checkpoint_entries
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
        checkpoint: CheckpointMark<DenseJournalLane>,
        checkpoint_entries: u32,
        group_depth: u32,
        save_stack: SaveStackProjection,
    ) -> Self {
        Self {
            owner,
            group_id,
            group_entry_position,
            checkpoint,
            checkpoint_entries,
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

    const fn checkpoint_mark(self) -> CheckpointMark<DenseJournalLane> {
        self.checkpoint
    }

    pub(super) const fn checkpoint_entries(self) -> u32 {
        self.checkpoint_entries
    }

    pub(super) const fn group_depth(self) -> u32 {
        self.group_depth
    }
}

/// Aggregate operation token for the independently transactional durable-box
/// lane. Ordinary eqtb assignments have TeX's immediate semantics and are not
/// mirrored into an executor-operation undo log.
pub struct StateOperation<G> {
    transaction_position: usize,
    group_depth: usize,
    save_stack: SaveStackProjection,
    durable_box: Option<crate::env::DurableBoxOperation>,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> core::fmt::Debug for StateOperation<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("StateOperation(..)")
    }
}

impl<G> StateOperation<G> {
    pub(crate) const fn transaction(
        position: usize,
        group_depth: usize,
        save_stack: SaveStackProjection,
    ) -> Self {
        Self {
            transaction_position: position,
            group_depth,
            save_stack,
            durable_box: None,
            _brand: PhantomData,
        }
    }

    pub(crate) const fn transaction_position(&self) -> usize {
        self.transaction_position
    }

    pub(crate) const fn group_depth(&self) -> usize {
        self.group_depth
    }

    pub(crate) const fn save_stack(&self) -> SaveStackProjection {
        self.save_stack
    }

    pub(crate) fn attach_durable_box(&mut self, operation: crate::env::DurableBoxOperation) {
        assert!(self.durable_box.replace(operation).is_none());
    }

    pub(crate) fn take_durable_box(&mut self) -> crate::env::DurableBoxOperation {
        self.durable_box
            .take()
            .expect("aggregate operation carries its durable box lane")
    }
}

/// One exact prior cell value for group or operation rollback.
pub(crate) struct Mutation<G> {
    cell: JournalCell,
    pub(crate) before: StateWord<G>,
    pub(crate) before_level: u32,
    /// The cell's direct first-write serial before this mutation.
    pub(crate) before_save_serial: u64,
    /// The TeX group level whose save record this assignment represents.
    /// `None` means TeX would not push a restore record for this write.
    saved_at: u32,
}

impl<G> Mutation<G> {
    pub(crate) fn new(
        cell: StateCell,
        before: StateWord<G>,
        before_level: u32,
        before_save_serial: u64,
        saved_at: Option<u32>,
    ) -> Self {
        debug_assert!(saved_at != Some(0));
        Self {
            cell: JournalCell::pack(cell),
            before,
            before_level,
            before_save_serial,
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
            before_save_serial: self.before_save_serial,
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

enum DenseJournalLane {}

/// One compact reversible value retained for a checkpoint interval.
pub(crate) struct CheckpointDelta<G> {
    pub(crate) cell: StateCell,
    pub(crate) alternate: StateWord<G>,
    pub(crate) alternate_level: u32,
    pub(crate) alternate_save_serial: u64,
}

/// Accepted journal material temporarily detached while one rooted candidate
/// owns the live dense state. Entries move into this token; they are neither
/// cloned into a checkpoint nor discarded before accept/reject resolves.
pub(crate) struct AcceptedJournalTail<G> {
    prior_checkpoint_entries: usize,
    groups: AcceptedGroupTail<G>,
}

enum AcceptedGroupTail<G> {
    Root {
        accepted_retained_groups: usize,
        next_group_id: u64,
        save_stack: SaveStackProjection,
    },
    Arbitrary {
        next_group_id: u64,
        accepted_active_ids: Vec<u64>,
        accepted_retained_ids: Vec<u64>,
        other_groups: Vec<GroupSegment<G>>,
        innermost_suffix: Vec<Mutation<G>>,
        save_stack: SaveStackProjection,
    },
}

impl<G> AcceptedJournalTail<G> {
    pub(crate) fn is_root_candidate(&self) -> bool {
        matches!(self.groups, AcceptedGroupTail::Root { .. })
    }
}

/// Allocation-free projection of the state-owned part of TeX's save stack.
///
/// Command-owned `\aftergroup` words and executor-owned box-spec words are
/// merged by main control. `latest_push` orders state pushes against the
/// command owner's journal-relative aftergroup position so §§1334/273 can
/// report the depth immediately before the newest checked push.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SaveStackProjection {
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
    spare_group_entries: Vec<Vec<Mutation<G>>>,
    next_group_id: u64,
    checkpoint_pool: ChunkPool<CheckpointDelta<G>>,
    checkpoint_arena: ForkArena<CheckpointDelta<G>, DenseJournalLane>,
    checkpoint_entries: usize,
    save_serial: u64,
    checkpoint_fork: bool,
    transaction_entries: Vec<Mutation<G>>,
    transaction_depth: usize,
    save_stack: SaveStackProjection,
    group_capacity_bytes: usize,
    checkpoint_capacity_bytes: usize,
    #[cfg(test)]
    first_touch_cell_visits: usize,
    #[cfg(feature = "profiling")]
    profile: SaveJournalProfile,
}

#[cfg(feature = "profiling")]
#[derive(Clone, Default)]
struct SaveJournalProfile {
    mutations: u64,
    mutation_words: [u64; 7],
    group_enters: u64,
    group_exits: u64,
    append_calls: u64,
    growths: u64,
    bytes_moved_by_growth: u64,
    peak_entries: usize,
    group_depth: usize,
    maximum_group_depth: usize,
    entries_at_maximum_group_depth: usize,
}

impl<G> SaveJournal<G> {
    #[must_use]
    pub(crate) fn new() -> Self {
        let owner = NEXT_JOURNAL_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("state journal identity space exhausted");
        let checkpoint_pool = ChunkPool::default();
        let checkpoint_capacity_bytes = checkpoint_pool.allocated_heap_bytes();
        Self {
            owner,
            active_groups: Vec::new(),
            active_group_entries: 0,
            retained_groups: Vec::new(),
            spare_group_entries: Vec::new(),
            next_group_id: 0,
            checkpoint_pool,
            checkpoint_arena: ForkArena::new(),
            checkpoint_entries: 0,
            // Before the first named checkpoint there is no earlier state to
            // retain. Cells and the journal therefore begin in the same
            // interval; sealing the first checkpoint advances the serial and
            // makes its first subsequent write capture one alternate.
            save_serial: 0,
            checkpoint_fork: false,
            transaction_entries: Vec::new(),
            transaction_depth: 0,
            save_stack: SaveStackProjection::default(),
            group_capacity_bytes: 0,
            checkpoint_capacity_bytes,
            #[cfg(test)]
            first_touch_cell_visits: 0,
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
            self.checkpoint_arena
                .seal_boundary(&mut self.checkpoint_pool)
                .and_then(|boundary| self.checkpoint_arena.checkpoint_mark(boundary))
                .expect("dense checkpoint journal seals its sole active tail"),
            u32::try_from(self.checkpoint_entries).expect("checkpoint journal exceeds u32 entries"),
            group_depth,
            self.save_stack,
        );
        self.advance_save_serial();
        cursor
    }

    pub(crate) fn record_mutation(&mut self, mutation: Mutation<G>) {
        #[cfg(feature = "profiling")]
        self.record_profile_mutation(&mutation);
        #[cfg(test)]
        {
            self.first_touch_cell_visits = self.first_touch_cell_visits.saturating_add(1);
        }
        let cell = mutation.cell();
        if mutation.before_save_serial != self.save_serial {
            #[cfg(feature = "profiling")]
            self.record_profile_growth(
                self.checkpoint_entries,
                self.checkpoint_pool.chunk_capacity(),
                core::mem::size_of::<CheckpointDelta<G>>(),
            );
            let mut builder = self
                .checkpoint_arena
                .begin_builder(&mut self.checkpoint_pool)
                .expect("dense journal owns the sole active builder");
            builder
                .push(CheckpointDelta {
                    cell,
                    alternate: mutation.before.clone(),
                    alternate_level: mutation.before_level,
                    alternate_save_serial: mutation.before_save_serial,
                })
                .expect("one dense journal cell fits its coarse chunk");
            let _ = builder.finish();
            self.checkpoint_entries = self
                .checkpoint_entries
                .checked_add(1)
                .expect("checkpoint journal exceeds usize entries");
            self.refresh_checkpoint_capacity_bytes();
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
            self.push_group_mutation(mutation.clone());
            self.active_group_entries = self.active_group_entries.saturating_add(1);
        }
        if self.transaction_depth != 0 {
            #[cfg(feature = "profiling")]
            self.record_profile_growth(
                self.transaction_entries.len(),
                self.transaction_entries.capacity(),
                core::mem::size_of::<Mutation<G>>(),
            );
            self.transaction_entries.push(mutation);
        }
        #[cfg(feature = "profiling")]
        self.record_profile_peak();
    }

    /// Whether this assignment needs any retained inverse at all.
    ///
    /// A checkpoint takes the first prior value in its interval. TeX grouping
    /// independently takes the first value displaced at the current level.
    /// Global/root writes satisfying neither condition replace the dense cell
    /// without constructing a mutation record.
    #[inline(always)]
    pub(crate) const fn needs_mutation(
        &self,
        before_save_serial: u64,
        saved_at: Option<u32>,
    ) -> bool {
        before_save_serial != self.save_serial || saved_at.is_some() || self.transaction_depth != 0
    }

    pub(crate) fn begin_transaction(&mut self) -> usize {
        let position = self.transaction_entries.len();
        self.transaction_depth = self.transaction_depth.saturating_add(1);
        position
    }

    pub(crate) const fn current_save_stack(&self) -> SaveStackProjection {
        self.save_stack
    }

    pub(crate) fn commit_transaction(&mut self, position: usize) {
        assert!(self.transaction_depth != 0, "state transaction is active");
        assert!(position <= self.transaction_entries.len());
        self.transaction_depth -= 1;
        if self.transaction_depth == 0 {
            self.transaction_entries.clear();
        }
    }

    pub(crate) fn transaction_entry(&self, index: usize) -> Option<Mutation<G>> {
        self.transaction_entries.get(index).cloned()
    }

    pub(crate) fn transaction_len(&self) -> usize {
        self.transaction_entries.len()
    }

    pub(crate) fn finish_transaction_rollback(&mut self, position: usize) {
        assert!(self.transaction_depth != 0, "state transaction is active");
        self.transaction_entries.truncate(position);
        self.transaction_depth -= 1;
    }

    pub(crate) fn rollback_group_suffix(
        &mut self,
        group_depth: usize,
        save_stack: SaveStackProjection,
    ) -> bool {
        if self.active_groups.len() < group_depth {
            return false;
        }
        while self.active_groups.len() > group_depth {
            let segment = self.active_groups.pop().expect("group suffix is nonempty");
            self.active_group_entries = self
                .active_group_entries
                .saturating_sub(segment.entries.len().saturating_add(1));
            self.recycle_segment(segment);
        }
        self.save_stack = save_stack;
        true
    }

    /// The monotonic serial written directly into every cell mutated in the
    /// current checkpoint interval.
    #[inline(always)]
    pub(crate) const fn save_serial(&self) -> u64 {
        self.save_serial
    }

    #[cfg(test)]
    pub(crate) const fn checkpoint_entry_count(&self) -> usize {
        self.checkpoint_entries
    }

    #[cfg(test)]
    pub(crate) const fn first_touch_cell_visits(&self) -> usize {
        self.first_touch_cell_visits
    }

    pub(crate) fn record_group_enter(&mut self, frame: GroupFrame) {
        #[cfg(feature = "profiling")]
        self.record_profile_group_enter();
        let position = u32::try_from(self.len().saturating_add(1))
            .expect("group save stack exceeds u32 entries");
        self.save_stack
            .push(&JournalEntry::<G>::GroupEnter(frame), position);
        self.next_group_id = self
            .next_group_id
            .checked_add(1)
            .expect("group segment identity space exhausted");
        let parent = self.active_groups.last().map_or(0, |group| group.id);
        let entries = self.spare_group_entries.pop().unwrap_or_default();
        self.push_active_group(GroupSegment {
            id: self.next_group_id,
            parent,
            frame,
            entries,
            checkpoint_pinned: false,
        });
        self.active_group_entries = self.active_group_entries.saturating_add(1);
        #[cfg(feature = "profiling")]
        self.record_profile_peak();
    }

    pub(crate) fn record_group_exit(&mut self, frame: GroupFrame) {
        #[cfg(feature = "profiling")]
        self.record_profile_group_exit();
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
        if segment.checkpoint_pinned {
            self.push_retained_group(segment);
        } else {
            let mut entries = segment.entries;
            entries.clear();
            self.spare_group_entries.push(entries);
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
            .saturating_add(self.checkpoint_entries)
            .saturating_add(self.transaction_entries.len())
    }

    #[must_use]
    pub(crate) const fn retained_bytes(&self) -> usize {
        self.group_capacity_bytes
            .saturating_add(self.checkpoint_capacity_bytes)
            .saturating_add(
                self.transaction_entries
                    .capacity()
                    .saturating_mul(core::mem::size_of::<Mutation<G>>()),
            )
    }

    #[cfg(test)]
    pub(crate) const fn checkpoint_counters(&self) -> crate::fork_arena::ForkArenaCounters {
        self.checkpoint_arena.counters()
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

    #[cfg(test)]
    pub(crate) fn visit_checkpoint_prefix(
        &self,
        cursor: JournalCursor<G>,
        visit: impl FnMut(&CheckpointDelta<G>),
    ) {
        self.checkpoint_arena
            .visit_checkpoint_values(&self.checkpoint_pool, cursor.checkpoint_mark(), visit)
            .expect("validated dense checkpoint prefix remains live");
    }

    pub(crate) fn visit_checkpoint_suffix(
        &self,
        cursor: JournalCursor<G>,
        visit: impl FnMut(&CheckpointDelta<G>),
    ) {
        if self.checkpoint_fork {
            self.checkpoint_arena
                .visit_current_checkpoint_suffix(
                    &self.checkpoint_pool,
                    cursor.checkpoint_mark(),
                    visit,
                )
                .expect("validated dense candidate suffix remains live");
        } else {
            self.checkpoint_arena
                .visit_accepted_checkpoint_suffix(
                    &self.checkpoint_pool,
                    cursor.checkpoint_mark(),
                    visit,
                )
                .expect("validated dense checkpoint suffix remains live");
        }
    }

    pub(crate) fn visit_checkpoint_suffix_mut_reverse(
        &mut self,
        cursor: JournalCursor<G>,
        visit: impl FnMut(&mut CheckpointDelta<G>),
    ) {
        self.checkpoint_arena
            .visit_accepted_checkpoint_suffix_mut_reverse(
                &mut self.checkpoint_pool,
                cursor.checkpoint_mark(),
                visit,
            )
            .expect("validated dense checkpoint suffix rewinds in place");
    }

    pub(crate) fn visit_current_suffix_mut_reverse(
        &mut self,
        cursor: JournalCursor<G>,
        visit: impl FnMut(&mut CheckpointDelta<G>),
    ) {
        self.checkpoint_arena
            .visit_current_checkpoint_suffix_mut_reverse(
                &mut self.checkpoint_pool,
                cursor.checkpoint_mark(),
                visit,
            )
            .expect("dense candidate suffix undoes in place");
    }

    pub(crate) fn visit_detached_suffix_mut(&mut self, visit: impl FnMut(&mut CheckpointDelta<G>)) {
        self.checkpoint_arena
            .visit_detached_checkpoint_suffix_mut(&mut self.checkpoint_pool, visit)
            .expect("dense accepted suffix redoes in place");
    }

    pub(crate) fn visit_detached_prefix(
        &self,
        cursor: JournalCursor<G>,
        visit: impl FnMut(&CheckpointDelta<G>),
    ) -> bool {
        self.checkpoint_arena
            .visit_detached_checkpoint_prefix(
                &self.checkpoint_pool,
                cursor.checkpoint_mark(),
                visit,
            )
            .is_ok()
    }

    /// Moves the accepted suffix out of the live lane and opens an empty
    /// candidate suffix at `cursor`.
    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        cursor: JournalCursor<G>,
    ) -> AcceptedJournalTail<G> {
        assert!(self.validate_cursor(cursor));
        assert!(!self.checkpoint_fork);
        let prior_checkpoint_entries = self.checkpoint_entries;
        self.checkpoint_arena
            .begin_checkpoint_candidate(&mut self.checkpoint_pool, cursor.checkpoint_mark())
            .expect("validated dense checkpoint begins its sole fork");
        self.checkpoint_entries = cursor.checkpoint_entries() as usize;
        self.checkpoint_fork = true;
        self.advance_save_serial();

        let save_stack = self.save_stack;
        self.save_stack = cursor.save_stack;
        let groups = if cursor.group_depth() == 0 && self.active_groups.is_empty() {
            AcceptedGroupTail::Root {
                accepted_retained_groups: self.retained_groups.len(),
                next_group_id: self.next_group_id,
                save_stack,
            }
        } else {
            let accepted_active_ids = self.active_groups.iter().map(|group| group.id).collect();
            let accepted_retained_ids = self.retained_groups.iter().map(|group| group.id).collect();
            self.remove_group_storage(
                self.active_groups.capacity(),
                Self::group_entry_capacity_bytes(&self.active_groups),
            );
            let mut groups = std::mem::take(&mut self.active_groups);
            self.remove_group_storage(0, Self::group_entry_capacity_bytes(&self.retained_groups));
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
                let segment = groups.swap_remove(index);
                self.admit_active_group(segment);
            }
            let innermost_suffix = self
                .active_groups
                .last_mut()
                .map_or_else(Vec::new, |group| {
                    let before = group.entries.capacity();
                    let suffix = group
                        .entries
                        .split_off(cursor.group_entry_position() as usize);
                    Self::account_capacity_change::<Mutation<G>>(
                        &mut self.group_capacity_bytes,
                        before,
                        group.entries.capacity(),
                    );
                    suffix
                });
            self.active_group_entries = self
                .active_groups
                .iter()
                .map(|group| group.entries.len().saturating_add(1))
                .sum();
            AcceptedGroupTail::Arbitrary {
                next_group_id: self.next_group_id,
                accepted_active_ids,
                accepted_retained_ids,
                other_groups: groups,
                innermost_suffix,
                save_stack,
            }
        };
        AcceptedJournalTail {
            prior_checkpoint_entries,
            groups,
        }
    }

    /// Restores the accepted group/journal topology after the candidate lane
    /// has already been destructively returned to its empty root.
    pub(crate) fn reject_checkpoint_candidate(&mut self, tail: AcceptedJournalTail<G>) {
        match tail.groups {
            AcceptedGroupTail::Root {
                accepted_retained_groups,
                next_group_id,
                save_stack,
            } => {
                while let Some(group) = self.active_groups.pop() {
                    self.recycle_segment(group);
                }
                while self.retained_groups.len() > accepted_retained_groups {
                    let group = self
                        .retained_groups
                        .pop()
                        .expect("candidate retained group");
                    self.recycle_segment(group);
                }
                self.active_group_entries = 0;
                self.next_group_id = next_group_id;
                self.save_stack = save_stack;
            }
            AcceptedGroupTail::Arbitrary {
                next_group_id,
                accepted_active_ids,
                accepted_retained_ids,
                mut other_groups,
                mut innermost_suffix,
                save_stack,
            } => {
                self.remove_group_storage(
                    self.active_groups.capacity(),
                    Self::group_entry_capacity_bytes(&self.active_groups),
                );
                let mut groups = std::mem::take(&mut self.active_groups);
                self.remove_group_storage(
                    0,
                    Self::group_entry_capacity_bytes(&self.retained_groups),
                );
                groups.append(&mut self.retained_groups);
                groups.append(&mut other_groups);
                if let Some(group) = groups
                    .iter_mut()
                    .find(|group| group.id == accepted_active_ids.last().copied().unwrap_or(0))
                {
                    group.entries.append(&mut innermost_suffix);
                }
                let mut take_group = |id| {
                    let index = groups
                        .iter()
                        .position(|group| group.id == id)
                        .expect("accepted group id survives candidate rollback");
                    groups.swap_remove(index)
                };
                let active_before = self.active_groups.capacity();
                let active_groups: Vec<_> = accepted_active_ids
                    .into_iter()
                    .map(&mut take_group)
                    .collect();
                Self::account_capacity_change::<GroupSegment<G>>(
                    &mut self.group_capacity_bytes,
                    active_before,
                    active_groups.capacity(),
                );
                self.group_capacity_bytes = self
                    .group_capacity_bytes
                    .saturating_add(Self::group_entry_capacity_bytes(&active_groups));
                self.active_groups = active_groups;
                let retained_before = self.retained_groups.capacity();
                let retained_groups: Vec<_> = accepted_retained_ids
                    .into_iter()
                    .map(&mut take_group)
                    .collect();
                Self::account_capacity_change::<GroupSegment<G>>(
                    &mut self.group_capacity_bytes,
                    retained_before,
                    retained_groups.capacity(),
                );
                self.group_capacity_bytes = self
                    .group_capacity_bytes
                    .saturating_add(Self::group_entry_capacity_bytes(&retained_groups));
                self.retained_groups = retained_groups;
                self.active_group_entries = self
                    .active_groups
                    .iter()
                    .map(|group| group.entries.len().saturating_add(1))
                    .sum();
                self.next_group_id = next_group_id;
                self.save_stack = save_stack;
            }
        }
        self.checkpoint_entries = tail.prior_checkpoint_entries;
        let boundary = self
            .checkpoint_arena
            .seal_boundary(&mut self.checkpoint_pool)
            .expect("dense rejection seals its current suffix");
        self.checkpoint_arena
            .reject_checkpoint_candidate(&mut self.checkpoint_pool, boundary)
            .expect("dense rejection reattaches its prior suffix");
        self.refresh_checkpoint_capacity_bytes();
        self.checkpoint_fork = false;
        self.advance_save_serial();
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self) {
        assert!(self.checkpoint_fork);
        let boundary = self
            .checkpoint_arena
            .seal_boundary(&mut self.checkpoint_pool)
            .expect("dense acceptance seals its current suffix");
        self.checkpoint_arena
            .accept_checkpoint_candidate(&mut self.checkpoint_pool, boundary)
            .expect("dense acceptance drops its detached prior suffix");
        self.refresh_checkpoint_capacity_bytes();
        self.checkpoint_fork = false;
        self.advance_save_serial();
    }

    pub(crate) fn truncate_checkpoint(&mut self, cursor: JournalCursor<G>) {
        assert_eq!(
            cursor.owner, self.owner,
            "journal cursor belongs to another state"
        );
        self.checkpoint_arena
            .restore_accepted_checkpoint(&mut self.checkpoint_pool, cursor.checkpoint_mark())
            .expect("validated dense checkpoint suffix truncates atomically");
        self.refresh_checkpoint_capacity_bytes();
        self.checkpoint_entries = cursor.checkpoint_entries() as usize;
        self.advance_save_serial();
        self.save_stack = cursor.save_stack;
    }

    pub(crate) fn release_checkpoint_prefix(
        &mut self,
        cursor: JournalCursor<G>,
    ) -> Result<usize, crate::StateError> {
        if !self.validate_cursor(cursor) || self.checkpoint_fork {
            return Err(crate::StateError::InvalidCursor);
        }
        let released = self
            .checkpoint_arena
            .release_accepted_prefix(&mut self.checkpoint_pool, cursor.checkpoint_mark())
            .map_err(|_| crate::StateError::InvalidCursor)?;
        self.refresh_checkpoint_capacity_bytes();
        Ok(released)
    }

    #[must_use]
    pub(crate) fn validate_cursor(&self, cursor: JournalCursor<G>) -> bool {
        if cursor.owner != self.owner
            || cursor.checkpoint_entries() as usize > self.checkpoint_entries
            || !self
                .checkpoint_arena
                .validates_checkpoint(cursor.checkpoint_mark())
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
                    self.push_retained_group(segment);
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
                self.push_retained_group(segment);
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
            self.push_active_group(segment);
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

    pub(crate) fn active_group_frames(&self) -> impl Iterator<Item = GroupFrame> + '_ {
        self.active_groups.iter().map(|group| group.frame)
    }

    fn group_segment(&self, id: u64) -> Option<&GroupSegment<G>> {
        self.active_groups
            .iter()
            .chain(&self.retained_groups)
            .find(|group| group.id == id)
    }

    pub(crate) fn group_save_len(&self) -> usize {
        self.active_groups
            .iter()
            .chain(&self.retained_groups)
            .map(|group| group.entries.len().saturating_add(1))
            .sum()
    }

    #[cfg(feature = "profiling")]
    fn group_mutation_len(&self) -> usize {
        self.active_groups
            .iter()
            .chain(&self.retained_groups)
            .map(|group| group.entries.len())
            .sum()
    }

    #[cfg(feature = "profiling")]
    fn group_mutation_capacity(&self) -> usize {
        self.active_groups
            .iter()
            .chain(&self.retained_groups)
            .map(|group| group.entries.capacity())
            .chain(self.spare_group_entries.iter().map(Vec::capacity))
            .sum()
    }

    #[cfg(test)]
    fn retained_bytes_census(&self) -> usize {
        self.group_capacity_bytes_census()
            .saturating_add(self.checkpoint_pool.allocated_heap_bytes())
            .saturating_add(
                self.transaction_entries
                    .capacity()
                    .saturating_mul(core::mem::size_of::<Mutation<G>>()),
            )
    }

    #[cfg(test)]
    fn group_capacity_bytes_census(&self) -> usize {
        let headers = self
            .active_groups
            .capacity()
            .saturating_add(self.retained_groups.capacity())
            .saturating_mul(core::mem::size_of::<GroupSegment<G>>());
        self.active_groups
            .iter()
            .chain(&self.retained_groups)
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

    fn account_capacity_change<T>(retained_bytes: &mut usize, before: usize, after: usize) {
        let element_bytes = core::mem::size_of::<T>();
        if after >= before {
            *retained_bytes = retained_bytes
                .saturating_add(after.saturating_sub(before).saturating_mul(element_bytes));
        } else {
            *retained_bytes = retained_bytes
                .saturating_sub(before.saturating_sub(after).saturating_mul(element_bytes));
        }
    }

    fn push_group_mutation(&mut self, mutation: Mutation<G>) {
        let group = self
            .active_groups
            .last_mut()
            .expect("a TeX save has an active group");
        let before = group.entries.capacity();
        group.entries.push(mutation);
        Self::account_capacity_change::<Mutation<G>>(
            &mut self.group_capacity_bytes,
            before,
            group.entries.capacity(),
        );
    }

    fn push_active_group(&mut self, segment: GroupSegment<G>) {
        let before = self.active_groups.capacity();
        self.active_groups.push(segment);
        Self::account_capacity_change::<GroupSegment<G>>(
            &mut self.group_capacity_bytes,
            before,
            self.active_groups.capacity(),
        );
    }

    fn admit_active_group(&mut self, segment: GroupSegment<G>) {
        self.add_group_entry_storage(&segment);
        self.push_active_group(segment);
    }

    fn push_retained_group(&mut self, segment: GroupSegment<G>) {
        let before = self.retained_groups.capacity();
        self.retained_groups.push(segment);
        Self::account_capacity_change::<GroupSegment<G>>(
            &mut self.group_capacity_bytes,
            before,
            self.retained_groups.capacity(),
        );
    }

    fn group_entry_capacity_bytes(groups: &[GroupSegment<G>]) -> usize {
        groups
            .iter()
            .map(|group| {
                group
                    .entries
                    .capacity()
                    .saturating_mul(core::mem::size_of::<Mutation<G>>())
            })
            .fold(0, usize::saturating_add)
    }

    fn remove_group_storage(&mut self, header_capacity: usize, entry_bytes: usize) {
        self.group_capacity_bytes = self.group_capacity_bytes.saturating_sub(
            header_capacity.saturating_mul(core::mem::size_of::<GroupSegment<G>>()),
        );
        self.group_capacity_bytes = self.group_capacity_bytes.saturating_sub(entry_bytes);
    }

    fn add_group_entry_storage(&mut self, segment: &GroupSegment<G>) {
        self.group_capacity_bytes = self.group_capacity_bytes.saturating_add(
            segment
                .entries
                .capacity()
                .saturating_mul(core::mem::size_of::<Mutation<G>>()),
        );
    }

    fn refresh_checkpoint_capacity_bytes(&mut self) {
        self.checkpoint_capacity_bytes = self.checkpoint_pool.allocated_heap_bytes();
    }

    fn recycle_segment(&mut self, segment: GroupSegment<G>) {
        let mut entries = segment.entries;
        entries.clear();
        self.spare_group_entries.push(entries);
    }

    fn advance_save_serial(&mut self) {
        self.save_serial = self
            .save_serial
            .checked_add(1)
            .expect("dense save serial space exhausted");
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
            StateWord::Font(_) => 5,
            StateWord::Code(_) => 6,
        };
        self.profile.mutation_words[word] = self.profile.mutation_words[word].saturating_add(1);
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
}

#[cfg(feature = "profiling")]
impl<G> Drop for SaveJournal<G> {
    fn drop(&mut self) {
        crate::measurement::record_save_journal_census(crate::measurement::SaveJournalCensus {
            entries: u64::try_from(self.retained_len()).unwrap_or(u64::MAX),
            capacity: u64::try_from(
                self.group_mutation_capacity()
                    .saturating_add(self.checkpoint_pool.chunk_capacity())
                    .saturating_add(self.transaction_entries.capacity()),
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
            checkpoint_entries: u64::try_from(self.checkpoint_entries).unwrap_or(u64::MAX),
            checkpoint_capacity: u64::try_from(self.checkpoint_pool.chunk_capacity())
                .unwrap_or(u64::MAX),
            checkpoint_entry_size: u64::try_from(core::mem::size_of::<CheckpointDelta<G>>())
                .unwrap_or(u64::MAX),
            operation_entries: u64::try_from(self.transaction_entries.len()).unwrap_or(u64::MAX),
            operation_capacity: u64::try_from(self.transaction_entries.capacity())
                .unwrap_or(u64::MAX),
            operation_entry_size: u64::try_from(core::mem::size_of::<Mutation<G>>())
                .unwrap_or(u64::MAX),
            // Direct stamps live in their authoritative dense cells. The old
            // hash-table occupancy controls remain zero for profile schema
            // compatibility.
            stamp_entries: 0,
            stamp_capacity: 0,
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
