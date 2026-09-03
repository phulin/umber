//! Separate TeX group-save, checkpoint-delta, and operation-undo journals.
//!
//! The group-save lane is one fixed-chunk sequence of ordered records.  A
//! group keeps only a scalar mark into that sequence; entering a group does
//! not allocate an entry-sized owner and an ordinary lookup maps an absolute
//! save index directly to a chunk and slot.  Checkpoint deltas and operation
//! inverses retain their existing independent representations.

use core::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use std::cell::Cell;

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

/// Number of slots in one reusable group-save chunk.
///
/// The slot size is deliberately fixed.  A chunk is allocated only when an
/// append crosses this boundary, and truncation leaves the chunk available for
/// the next append in the same generation.
const SAVE_CHUNK_SLOTS: usize = 64;

/// A stable logical cursor in one generation's checkpoint history.
pub struct JournalCursor<G> {
    owner: u64,
    group_id: u64,
    group_entry_position: u32,
    checkpoint: CheckpointMark<DenseJournalLane>,
    checkpoint_entries: u32,
    group_depth: u32,
    save_stack: SaveStackProjection,
    /// The active save-sequence cursor at capture.  This is kept separately
    /// from the checkpoint-lane mark because the two lanes have independent
    /// storage and rollback lifetimes.
    save_position: u32,
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
            && self.save_position == other.save_position
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
            save_position: 0,
            _brand: PhantomData,
        }
    }

    const fn with_save_position(self, save_position: u32) -> Self {
        Self {
            save_position,
            ..self
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
///
/// Group-enter markers occupy fixed journal slots so the long-standing
/// journal-relative save-stack projection and test-facing `entry` coordinate
/// remain exact.  The marker is not an owner or a per-group allocation; the
/// active group itself is the scalar [`GroupMark`] below.
pub(crate) enum JournalEntry<G> {
    Mutation(Mutation<G>),
    GroupEnter(GroupFrame),
    GroupExit(GroupFrame),
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

/// Scalar active-group metadata.  All saved values live in the shared chunk
/// sequence, never in a group-owned `Vec`.
#[derive(Clone, Copy)]
struct GroupMark {
    id: u64,
    parent: u64,
    frame: GroupFrame,
    /// Absolute slot of the fixed group-enter marker.
    start: usize,
    /// Number of mutation records belonging to this group.
    entries: usize,
    /// First sparse-array mutation in this segment.  Keeping this boundary
    /// lets one reverse walk produce TeX's dense-then-sparse restoration order.
    sparse_start: Option<usize>,
    checkpoint_pinned: bool,
}

/// Metadata for one closed group whose inverse records remain reachable from
/// a checkpoint lineage.  Records are stored in `retained_sequence` at the
/// direct range `[start, start + entries)`.
#[derive(Clone, Copy)]
struct RetainedGroup {
    id: u64,
    parent: u64,
    frame: GroupFrame,
    start: usize,
    entries: usize,
    sparse_start: Option<usize>,
}

struct SaveChunk<G> {
    entries: Vec<JournalEntry<G>>,
}

/// Reusable fixed-chunk storage for one append/truncate sequence.
///
/// `entry` is intentionally the only ordinary lookup path.  It performs two
/// integer operations and two direct indexing operations; it never consults
/// group topology.
struct SaveSequence<G> {
    chunks: Vec<SaveChunk<G>>,
    len: usize,
    capacity_bytes: usize,
}

impl<G> Default for SaveSequence<G> {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            len: 0,
            capacity_bytes: 0,
        }
    }
}

impl<G> SaveSequence<G> {
    #[inline]
    fn entry(&self, index: usize) -> Option<&JournalEntry<G>> {
        let chunk = index / SAVE_CHUNK_SLOTS;
        let slot = index % SAVE_CHUNK_SLOTS;
        self.chunks
            .get(chunk)
            .and_then(|chunk| chunk.entries.get(slot))
    }

    #[cfg(feature = "profiling")]
    #[inline]
    fn capacity_slots(&self) -> usize {
        self.chunks.capacity().saturating_mul(SAVE_CHUNK_SLOTS)
    }

    fn append(&mut self, entry: JournalEntry<G>) -> bool {
        let mut grew = false;
        let chunk_index = self.len / SAVE_CHUNK_SLOTS;
        if chunk_index == self.chunks.len() {
            let before = self.chunks.capacity();
            let chunk = SaveChunk {
                entries: Vec::with_capacity(SAVE_CHUNK_SLOTS),
            };
            self.chunks.push(chunk);
            let after = self.chunks.capacity();
            self.capacity_bytes = self
                .capacity_bytes
                .saturating_add(
                    after
                        .saturating_sub(before)
                        .saturating_mul(core::mem::size_of::<SaveChunk<G>>()),
                )
                .saturating_add(
                    SAVE_CHUNK_SLOTS.saturating_mul(core::mem::size_of::<JournalEntry<G>>()),
                );
            grew = true;
        }
        debug_assert_eq!(
            self.chunks[chunk_index].entries.len(),
            self.len % SAVE_CHUNK_SLOTS,
            "truncated save chunks are reused at their exact slot"
        );
        self.chunks
            .get_mut(chunk_index)
            .expect("a group-save chunk was just admitted or reused")
            .entries
            .push(entry);
        self.len = self.len.saturating_add(1);
        #[cfg(not(feature = "profiling"))]
        let _ = grew;
        grew
    }

    fn truncate(&mut self, target: usize) {
        assert!(
            target <= self.len,
            "save sequence truncates only its suffix"
        );
        if target == self.len {
            return;
        }
        let first = target / SAVE_CHUNK_SLOTS;
        let slot = target % SAVE_CHUNK_SLOTS;
        if let Some(chunk) = self.chunks.get_mut(first) {
            chunk.entries.truncate(slot);
        }
        for chunk in self.chunks.iter_mut().skip(first.saturating_add(1)) {
            chunk.entries.clear();
        }
        self.len = target;
    }

    fn swap_entries(&mut self, left: usize, right: usize) {
        if left == right {
            return;
        }
        let left_chunk = left / SAVE_CHUNK_SLOTS;
        let left_slot = left % SAVE_CHUNK_SLOTS;
        let right_chunk = right / SAVE_CHUNK_SLOTS;
        let right_slot = right % SAVE_CHUNK_SLOTS;
        if left_chunk == right_chunk {
            self.chunks[left_chunk].entries.swap(left_slot, right_slot);
            return;
        }
        if left_chunk < right_chunk {
            let (before, after) = self.chunks.split_at_mut(right_chunk);
            core::mem::swap(
                &mut before[left_chunk].entries[left_slot],
                &mut after[0].entries[right_slot],
            );
        } else {
            let (before, after) = self.chunks.split_at_mut(left_chunk);
            core::mem::swap(
                &mut after[0].entries[left_slot],
                &mut before[right_chunk].entries[right_slot],
            );
        }
    }
}

enum DenseJournalLane {}

/// One compact reversible value retained for a checkpoint interval.
pub(crate) struct CheckpointDelta<G> {
    pub(crate) cell: StateCell,
    pub(crate) alternate: StateWord<G>,
    pub(crate) alternate_level: u32,
    pub(crate) alternate_save_serial: u64,
}

/// Accepted journal material temporarily detached while the current
/// candidate owns the live dense state.  The detached group sequences move as
/// whole chunk owners; no per-record owner is manufactured.
pub(crate) struct AcceptedJournalTail<G> {
    prior_checkpoint_entries: usize,
    groups: AcceptedGroupTail<G>,
}

enum AcceptedGroupTail<G> {
    Root {
        accepted_retained_groups: usize,
        accepted_retained_entries: usize,
        next_group_id: u64,
        save_stack: SaveStackProjection,
        save_position: u32,
    },
    Arbitrary {
        next_group_id: u64,
        active_groups: Vec<GroupMark>,
        retained_groups: Vec<RetainedGroup>,
        active_sequence: SaveSequence<G>,
        retained_sequence: SaveSequence<G>,
        save_stack: SaveStackProjection,
        save_position: u32,
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

/// Split rollback storage for one live revision generation.
pub(crate) struct SaveJournal<G> {
    owner: u64,
    /// Current append/truncate sequence.  Its slots are the only ordinary
    /// save-record address space.
    active_sequence: SaveSequence<G>,
    active_groups: Vec<GroupMark>,
    /// Closed checkpoint-pinned groups retain their records in fixed chunks so
    /// an in-group cursor can be restored after the live branch exits.
    retained_sequence: SaveSequence<G>,
    retained_groups: Vec<RetainedGroup>,
    next_group_id: u64,
    checkpoint_pool: ChunkPool<CheckpointDelta<G>>,
    checkpoint_arena: ForkArena<CheckpointDelta<G>, DenseJournalLane>,
    checkpoint_entries: usize,
    save_serial: u64,
    checkpoint_fork: bool,
    transaction_entries: Vec<Mutation<G>>,
    transaction_depth: usize,
    save_stack: SaveStackProjection,
    save_position: u32,
    /// Reusable scratch for sparse-array records deferred to the restore
    /// marker during one group unwind.  It is journal-owned so repeated
    /// sparse groups reuse its capacity instead of allocating on every exit.
    sparse_scratch: Vec<Mutation<G>>,
    group_capacity_bytes: usize,
    checkpoint_capacity_bytes: usize,
    #[cfg(test)]
    first_touch_cell_visits: usize,
    #[cfg(test)]
    group_entry_visits: Cell<usize>,
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
            active_sequence: SaveSequence::default(),
            active_groups: Vec::new(),
            retained_sequence: SaveSequence::default(),
            retained_groups: Vec::new(),
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
            save_position: 0,
            sparse_scratch: Vec::new(),
            group_capacity_bytes: 0,
            checkpoint_capacity_bytes,
            #[cfg(test)]
            first_touch_cell_visits: 0,
            #[cfg(test)]
            group_entry_visits: Cell::new(0),
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
                u32::try_from(group.entries).expect("group save segment exceeds u32 entries"),
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
        )
        .with_save_position(self.save_position);
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

        let saved_at = mutation.saved_at();
        if saved_at.is_some() {
            let position = self
                .active_sequence
                .len
                .checked_add(1)
                .and_then(|position| u32::try_from(position).ok())
                .expect("group save stack exceeds u32 entries");
            self.save_stack
                .push(&JournalEntry::Mutation(mutation.clone()), position);
            if self.transaction_depth != 0 {
                self.transaction_entries.push(mutation.clone());
            }
            #[cfg(feature = "profiling")]
            self.record_profile_group_append();
            let grew = self
                .active_sequence
                .append(JournalEntry::Mutation(mutation));
            #[cfg(feature = "profiling")]
            if grew {
                self.record_profile_growth(
                    self.active_sequence.len.saturating_sub(1),
                    self.active_sequence
                        .capacity_slots()
                        .saturating_sub(SAVE_CHUNK_SLOTS),
                    core::mem::size_of::<JournalEntry<G>>(),
                );
            }
            #[cfg(not(feature = "profiling"))]
            let _ = grew;
            let group = self
                .active_groups
                .last_mut()
                .expect("a TeX save has an active group");
            group.entries = group.entries.saturating_add(1);
            let index = self.active_sequence.len.saturating_sub(1);
            if group.sparse_start.is_none() && is_extended_register_cell(cell) {
                group.sparse_start = Some(index);
            }
            self.save_position = position;
        } else if self.transaction_depth != 0 {
            self.transaction_entries.push(mutation);
        }
        self.refresh_group_capacity_bytes();
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

    pub(crate) fn take_sparse_scratch(&mut self) -> Vec<Mutation<G>> {
        std::mem::take(&mut self.sparse_scratch)
    }

    pub(crate) fn return_sparse_scratch(&mut self, mut scratch: Vec<Mutation<G>>) {
        scratch.clear();
        self.sparse_scratch = scratch;
        self.refresh_group_capacity_bytes();
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
            let group = self.active_groups.pop().expect("group suffix is nonempty");
            self.active_sequence.truncate(group.start);
        }
        self.save_stack = save_stack;
        self.save_position =
            u32::try_from(self.active_sequence.len).expect("group save stack exceeds u32 entries");
        self.refresh_group_capacity_bytes();
        true
    }

    /// The monotonic serial written directly into every cell mutated in the
    /// current checkpoint interval.
    #[inline(always)]
    pub(crate) const fn save_serial(&self) -> u64 {
        self.save_serial
    }

    #[cfg(all(test, not(feature = "profiling")))]
    pub(crate) const fn checkpoint_entry_count(&self) -> usize {
        self.checkpoint_entries
    }

    #[cfg(all(test, not(feature = "profiling")))]
    pub(crate) const fn first_touch_cell_visits(&self) -> usize {
        self.first_touch_cell_visits
    }

    pub(crate) fn record_group_enter(&mut self, frame: GroupFrame) {
        #[cfg(feature = "profiling")]
        self.record_profile_group_enter();
        let marker = self.active_sequence.len;
        #[cfg(feature = "profiling")]
        if self.active_sequence.len % SAVE_CHUNK_SLOTS == 0 {
            self.record_profile_growth(
                self.active_sequence.len,
                self.active_sequence.capacity_slots(),
                core::mem::size_of::<JournalEntry<G>>(),
            );
        }
        self.active_sequence.append(JournalEntry::GroupEnter(frame));
        self.next_group_id = self
            .next_group_id
            .checked_add(1)
            .expect("group segment identity space exhausted");
        let parent = self.active_groups.last().map_or(0, |group| group.id);
        self.active_groups.push(GroupMark {
            id: self.next_group_id,
            parent,
            frame,
            start: marker,
            entries: 0,
            sparse_start: None,
            checkpoint_pinned: false,
        });
        self.save_stack.push(
            &JournalEntry::<G>::GroupEnter(frame),
            u32::try_from(marker.saturating_add(1)).expect("group save stack exceeds u32 entries"),
        );
        self.save_position =
            u32::try_from(marker.saturating_add(1)).expect("group save stack exceeds u32 entries");
        self.refresh_group_capacity_bytes();
        #[cfg(feature = "profiling")]
        self.record_profile_peak();
    }

    /// Closes a group from a direct caller that did not already walk its
    /// records.  Production `Env` uses `record_group_exit_with_records` so a
    /// pinned segment is captured during its one reverse restoration walk.
    #[cfg(test)]
    pub(crate) fn record_group_exit(&mut self, frame: GroupFrame) {
        let mark = *self
            .active_groups
            .last()
            .expect("group exit has a save segment");
        let retained = if mark.checkpoint_pinned {
            self.collect_active_group_records(mark)
        } else {
            Vec::new()
        };
        self.record_group_exit_with_records(frame, retained);
    }

    /// Finishes one group after the environment has already consumed its
    /// records in reverse. `retained_records` is in original journal order;
    /// moving that temporary vector into fixed chunks avoids a second journal
    /// walk while retaining a checkpoint-visible closed group.
    pub(crate) fn record_group_exit_with_records(
        &mut self,
        frame: GroupFrame,
        retained_records: Vec<Mutation<G>>,
    ) {
        #[cfg(feature = "profiling")]
        self.record_profile_group_exit();
        let mark = self
            .active_groups
            .pop()
            .expect("group exit has a save segment");
        // A checkpoint candidate may rehome the physical journal start while
        // the direct journal test (and a caller retaining the original
        // semantic frame) still supplies the pre-rehome value.  Group kind,
        // level, lineage, and diagnostic scalars remain identical.
        let pinned = mark.checkpoint_pinned;
        if pinned {
            let retained_start = self.retained_sequence.len;
            let mut sparse_start = None;
            for mutation in retained_records {
                let index = self.retained_sequence.len;
                let cell = mutation.cell();
                self.retained_sequence
                    .append(JournalEntry::Mutation(mutation));
                if sparse_start.is_none() && is_extended_register_cell(cell) {
                    sparse_start = Some(index);
                }
            }
            self.retained_groups.push(RetainedGroup {
                id: mark.id,
                parent: mark.parent,
                frame: mark.frame,
                start: retained_start,
                entries: self.retained_sequence.len.saturating_sub(retained_start),
                sparse_start,
            });
        }
        // The marker and all records belonging to the ending group are a
        // suffix of the active sequence.  Child records have already been
        // truncated at their own exits.
        self.active_sequence.truncate(mark.start);
        self.save_stack = SaveStackProjection {
            words: frame.save_stack_words_before,
            latest_push: frame.latest_save_push_before,
        };
        self.save_position =
            u32::try_from(self.active_sequence.len).expect("group save stack exceeds u32 entries");
        self.refresh_group_capacity_bytes();
        #[cfg(feature = "profiling")]
        self.record_profile_peak();
    }

    /// State-owned live words and the journal-relative newest checked push.
    #[must_use]
    pub(crate) const fn save_stack_projection(&self) -> (usize, Option<(u32, usize)>) {
        (self.save_stack.words, self.save_stack.latest_push)
    }

    /// Current active sequence length, including fixed group-enter markers.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.active_sequence.len
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

    /// Direct absolute save-index lookup.  This function deliberately has no
    /// topology fallback: group marks are consulted only by group-specific
    /// checkpoint operations, never by ordinary journal access.
    pub(crate) fn entry(&self, index: usize) -> JournalEntry<G> {
        #[cfg(test)]
        self.group_entry_visits
            .set(self.group_entry_visits.get().saturating_add(1));
        self.active_sequence
            .entry(index)
            .cloned()
            .unwrap_or_else(|| panic!("group save entry index {index} out of bounds"))
    }

    #[cfg(test)]
    pub(crate) const fn group_entry_visits(&self) -> usize {
        self.group_entry_visits.get()
    }

    pub(crate) fn group_entry_count(&self, frame: GroupFrame) -> usize {
        self.active_groups
            .last()
            .filter(|group| group.frame == frame)
            .map_or(0, |group| group.entries)
    }

    pub(crate) fn group_sparse_start(&self, frame: GroupFrame) -> Option<usize> {
        self.active_groups
            .last()
            .filter(|group| group.frame == frame)
            .and_then(|group| group.sparse_start)
    }

    pub(crate) fn group_checkpoint_pinned(&self, frame: GroupFrame) -> bool {
        self.active_groups
            .last()
            .filter(|group| group.frame == frame)
            .is_some_and(|group| group.checkpoint_pinned)
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

    /// Moves the accepted checkpoint suffix out of the live checkpoint lane
    /// and switches dense/group state to `cursor` without copying the accepted
    /// group sequence.
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
        let save_position = self.save_position;
        self.save_stack = cursor.save_stack;
        self.save_position = cursor.save_position;
        let root = cursor.group_depth() == 0 && self.active_groups.is_empty();
        let groups = if root {
            AcceptedGroupTail::Root {
                accepted_retained_groups: self.retained_groups.len(),
                accepted_retained_entries: self.retained_sequence.len,
                next_group_id: self.next_group_id,
                save_stack,
                save_position,
            }
        } else {
            let (target_sequence, target_groups) = if cursor.group_depth() == 0 {
                (SaveSequence::default(), Vec::new())
            } else {
                self.capture_target_path(cursor)
            };
            let accepted_active_groups = std::mem::take(&mut self.active_groups);
            let accepted_retained_groups = std::mem::take(&mut self.retained_groups);
            let active_sequence = std::mem::take(&mut self.active_sequence);
            let retained_sequence = std::mem::take(&mut self.retained_sequence);
            self.active_groups = target_groups;
            self.active_sequence = target_sequence;
            self.retained_groups = Vec::new();
            self.retained_sequence = SaveSequence::default();
            AcceptedGroupTail::Arbitrary {
                next_group_id: self.next_group_id,
                active_groups: accepted_active_groups,
                retained_groups: accepted_retained_groups,
                active_sequence,
                retained_sequence,
                save_stack,
                save_position,
            }
        };
        self.refresh_group_capacity_bytes();
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
                accepted_retained_entries,
                next_group_id,
                save_stack,
                save_position,
            } => {
                self.active_sequence.truncate(0);
                self.active_groups.clear();
                self.retained_groups.truncate(accepted_retained_groups);
                self.retained_sequence.truncate(accepted_retained_entries);
                self.next_group_id = next_group_id;
                self.save_stack = save_stack;
                self.save_position = save_position;
            }
            AcceptedGroupTail::Arbitrary {
                next_group_id,
                active_groups,
                retained_groups,
                active_sequence,
                retained_sequence,
                save_stack,
                save_position,
            } => {
                self.active_groups = active_groups;
                self.retained_groups = retained_groups;
                self.active_sequence = active_sequence;
                self.retained_sequence = retained_sequence;
                self.next_group_id = next_group_id;
                self.save_stack = save_stack;
                self.save_position = save_position;
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
        self.refresh_group_capacity_bytes();
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
        self.save_position = cursor.save_position;
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
                && cursor.group_entry_position() as usize <= group.entries
            {
                return true;
            }
        }
        let Some(group) = self.group_metadata(cursor.group_id()) else {
            return false;
        };
        if cursor.group_entry_position() as usize > group.entries() {
            return false;
        }
        let mut depth = 0_u32;
        let mut id = cursor.group_id();
        while id != 0 {
            let Some(group) = self.group_metadata(id) else {
                return false;
            };
            depth = depth.saturating_add(1);
            id = group.parent();
        }
        depth == cursor.group_depth()
    }

    pub(crate) fn restore_group_cursor(&mut self, cursor: JournalCursor<G>) -> RestoredGroups {
        let target_ids = self.target_group_ids(cursor);
        let current_ids: Vec<u64> = self.active_groups.iter().map(|group| group.id).collect();
        let common = current_ids
            .iter()
            .copied()
            .zip(target_ids.iter().copied())
            .take_while(|(active, target)| active == target)
            .count();
        if common == target_ids.len() {
            while self.active_groups.len() > target_ids.len() {
                let group = self.active_groups.pop().expect("active group suffix");
                if group.checkpoint_pinned {
                    self.retain_active_group(group);
                }
                self.active_sequence.truncate(group.start);
            }
            if let Some(group) = self.active_groups.last_mut() {
                let target_entries = cursor.group_entry_position() as usize;
                self.active_sequence
                    .truncate(group.start.saturating_add(1).saturating_add(target_entries));
                group.entries = target_entries;
                group.sparse_start = group
                    .sparse_start
                    .filter(|start| *start < self.active_sequence.len);
            } else {
                self.active_sequence.truncate(0);
            }
            self.save_stack = cursor.save_stack;
            self.save_position = cursor.save_position;
            self.refresh_group_capacity_bytes();
            return RestoredGroups::Truncate(target_ids.len());
        }

        let (target_sequence, target_groups) = if target_ids.is_empty() {
            (SaveSequence::default(), Vec::new())
        } else {
            self.capture_target_path(cursor)
        };
        while let Some(group) = self.active_groups.pop() {
            if group.checkpoint_pinned {
                self.retain_active_group(group);
            }
        }
        self.active_sequence.truncate(0);
        self.remove_retained_groups(&target_ids);
        self.retained_groups
            .retain(|group| !target_ids.contains(&group.id));
        self.active_sequence = target_sequence;
        self.active_groups = target_groups;
        self.save_stack = cursor.save_stack;
        self.save_position = cursor.save_position;
        self.refresh_group_capacity_bytes();
        RestoredGroups::Replace(self.active_groups.iter().map(|group| group.frame).collect())
    }

    pub(crate) fn active_group_frames(&self) -> impl Iterator<Item = GroupFrame> + '_ {
        self.active_groups.iter().map(|group| group.frame)
    }

    pub(crate) fn group_save_len(&self) -> usize {
        self.active_sequence
            .len
            .saturating_add(self.retained_sequence.len)
            .saturating_add(self.retained_groups.len())
    }

    #[cfg(feature = "profiling")]
    fn group_mutation_len(&self) -> usize {
        self.active_groups
            .iter()
            .map(|group| group.entries)
            .chain(self.retained_groups.iter().map(|group| group.entries))
            .sum()
    }

    #[cfg(feature = "profiling")]
    fn group_mutation_capacity(&self) -> usize {
        self.active_sequence
            .capacity_slots()
            .saturating_add(self.retained_sequence.capacity_slots())
            .saturating_add(self.sparse_scratch.capacity())
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
        self.active_sequence
            .capacity_bytes
            .saturating_add(self.retained_sequence.capacity_bytes)
            .saturating_add(
                self.active_groups
                    .capacity()
                    .saturating_mul(core::mem::size_of::<GroupMark>()),
            )
            .saturating_add(
                self.retained_groups
                    .capacity()
                    .saturating_mul(core::mem::size_of::<RetainedGroup>()),
            )
            .saturating_add(
                self.sparse_scratch
                    .capacity()
                    .saturating_mul(core::mem::size_of::<Mutation<G>>()),
            )
    }

    fn refresh_group_capacity_bytes(&mut self) {
        self.group_capacity_bytes = self
            .active_sequence
            .capacity_bytes
            .saturating_add(self.retained_sequence.capacity_bytes)
            .saturating_add(
                self.active_groups
                    .capacity()
                    .saturating_mul(core::mem::size_of::<GroupMark>()),
            )
            .saturating_add(
                self.retained_groups
                    .capacity()
                    .saturating_mul(core::mem::size_of::<RetainedGroup>()),
            )
            .saturating_add(
                self.sparse_scratch
                    .capacity()
                    .saturating_mul(core::mem::size_of::<Mutation<G>>()),
            );
    }

    fn refresh_checkpoint_capacity_bytes(&mut self) {
        self.checkpoint_capacity_bytes = self.checkpoint_pool.allocated_heap_bytes();
    }

    #[cfg(test)]
    fn collect_active_group_records(&self, group: GroupMark) -> Vec<Mutation<G>> {
        let mut records = Vec::with_capacity(group.entries);
        for index in group.start.saturating_add(1)..self.active_sequence.len {
            let Some(JournalEntry::Mutation(mutation)) = self.active_sequence.entry(index) else {
                continue;
            };
            if mutation.saved_at() == Some(group.frame.level()) {
                records.push(mutation.clone());
            }
        }
        records
    }

    fn retain_active_group(&mut self, group: GroupMark) {
        let retained_start = self.retained_sequence.len;
        let mut sparse_start = None;
        for index in group.start.saturating_add(1)..self.active_sequence.len {
            let Some(JournalEntry::Mutation(mutation)) = self.active_sequence.entry(index) else {
                continue;
            };
            if mutation.saved_at() != Some(group.frame.level()) {
                continue;
            }
            let retained_index = self.retained_sequence.len;
            self.retained_sequence
                .append(JournalEntry::Mutation(mutation.clone()));
            if sparse_start.is_none() && is_extended_register_cell(mutation.cell()) {
                sparse_start = Some(retained_index);
            }
        }
        self.retained_groups.push(RetainedGroup {
            id: group.id,
            parent: group.parent,
            frame: group.frame,
            start: retained_start,
            entries: self.retained_sequence.len.saturating_sub(retained_start),
            sparse_start,
        });
    }

    fn remove_retained_groups(&mut self, target_ids: &[u64]) {
        let mut destination = 0;
        for index in 0..self.retained_groups.len() {
            let group = self.retained_groups[index];
            if target_ids.contains(&group.id) {
                continue;
            }
            for offset in 0..group.entries {
                self.retained_sequence
                    .swap_entries(group.start.saturating_add(offset), destination);
                destination = destination.saturating_add(1);
            }
            self.retained_groups[index] = RetainedGroup {
                start: destination.saturating_sub(group.entries),
                sparse_start: group.sparse_start.map(|sparse| {
                    destination
                        .saturating_sub(group.entries)
                        .saturating_add(sparse.saturating_sub(group.start))
                }),
                ..group
            };
        }
        self.retained_sequence.truncate(destination);
    }

    fn active_group(&self, id: u64) -> Option<GroupMark> {
        self.active_groups
            .iter()
            .find(|group| group.id == id)
            .copied()
    }

    fn retained_group(&self, id: u64) -> Option<RetainedGroup> {
        self.retained_groups
            .iter()
            .find(|group| group.id == id)
            .copied()
    }

    fn group_metadata(&self, id: u64) -> Option<GroupMetadata> {
        self.active_group(id)
            .map(GroupMetadata::Active)
            .or_else(|| self.retained_group(id).map(GroupMetadata::Retained))
    }

    fn target_group_ids(&self, cursor: JournalCursor<G>) -> Vec<u64> {
        let mut ids = Vec::with_capacity(cursor.group_depth() as usize);
        let mut id = cursor.group_id();
        while id != 0 {
            ids.push(id);
            id = self
                .group_metadata(id)
                .expect("validated group cursor")
                .parent();
        }
        ids.reverse();
        ids
    }

    fn capture_target_path(&self, cursor: JournalCursor<G>) -> (SaveSequence<G>, Vec<GroupMark>) {
        let ids = self.target_group_ids(cursor);
        let mut sequence = SaveSequence::default();
        let mut groups = Vec::with_capacity(ids.len());
        for id in ids {
            let source = self
                .group_metadata(id)
                .expect("validated target group remains in journal history");
            let old_frame = source.frame();
            let marker = sequence.len;
            let frame = old_frame.with_journal_start(
                u32::try_from(marker.saturating_add(1)).expect("group save stack exceeds u32"),
            );
            sequence.append(JournalEntry::GroupEnter(frame));
            let limit = if id == cursor.group_id() {
                cursor.group_entry_position() as usize
            } else {
                source.entries()
            };
            let mut entries = 0;
            let mut sparse_start = None;
            match source {
                GroupMetadata::Active(group) => {
                    for index in group.start.saturating_add(1)..self.active_sequence.len {
                        let Some(JournalEntry::Mutation(mutation)) =
                            self.active_sequence.entry(index)
                        else {
                            continue;
                        };
                        if mutation.saved_at() != Some(old_frame.level()) || entries >= limit {
                            continue;
                        }
                        let destination = sequence.len;
                        sequence.append(JournalEntry::Mutation(mutation.clone()));
                        if sparse_start.is_none() && is_extended_register_cell(mutation.cell()) {
                            sparse_start = Some(destination);
                        }
                        entries += 1;
                    }
                }
                GroupMetadata::Retained(group) => {
                    let source_sparse_start = group.sparse_start;
                    for index in group.start..group.start.saturating_add(group.entries) {
                        let Some(JournalEntry::Mutation(mutation)) =
                            self.retained_sequence.entry(index)
                        else {
                            continue;
                        };
                        if entries >= limit {
                            break;
                        }
                        let destination = sequence.len;
                        sequence.append(JournalEntry::Mutation(mutation.clone()));
                        if sparse_start.is_none()
                            && (source_sparse_start == Some(index)
                                || is_extended_register_cell(mutation.cell()))
                        {
                            sparse_start = Some(destination);
                        }
                        entries += 1;
                    }
                }
            }
            groups.push(GroupMark {
                id,
                parent: source.parent(),
                frame,
                start: marker,
                entries,
                sparse_start,
                checkpoint_pinned: true,
            });
        }
        (sequence, groups)
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
    fn record_profile_group_append(&mut self) {
        self.profile.append_calls = self.profile.append_calls.saturating_add(1);
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

#[derive(Clone, Copy)]
enum GroupMetadata {
    Active(GroupMark),
    Retained(RetainedGroup),
}

impl GroupMetadata {
    const fn parent(self) -> u64 {
        match self {
            Self::Active(group) => group.parent,
            Self::Retained(group) => group.parent,
        }
    }

    const fn frame(self) -> GroupFrame {
        match self {
            Self::Active(group) => group.frame,
            Self::Retained(group) => group.frame,
        }
    }

    const fn entries(self) -> usize {
        match self {
            Self::Active(group) => group.entries,
            Self::Retained(group) => group.entries,
        }
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
    // representation even though Umber's fixed typed bank stores its virtual
    // default at level one.
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

fn is_extended_register_cell(cell: StateCell) -> bool {
    match cell {
        StateCell::Count(index)
        | StateCell::Dimension(index)
        | StateCell::TokenRegister(index)
        | StateCell::GlueRegister(index)
        | StateCell::BoxRegister(index)
        | StateCell::MuGlueRegister(index) => index > u8::MAX.into(),
        _ => false,
    }
}

impl<G> Default for SaveJournal<G> {
    fn default() -> Self {
        Self::new()
    }
}
