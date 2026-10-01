//! Separate TeX group-save and checkpoint-delta journals.
//!
//! The group-save lane is one fixed-chunk sequence of ordered records.  A
//! group keeps only a scalar mark into that sequence; entering a group does
//! not allocate an entry-sized owner and an ordinary lookup maps an absolute
//! save index directly to a chunk and slot.  Checkpoint deltas keep their own
//! fork-arena representation.

use core::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use std::cell::Cell;

use crate::env::group::GroupFrame;
use crate::env::{StateCell, StateWord};
mod checkpoints;
use checkpoints::{CheckpointCut, CheckpointUndoLog};

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
    checkpoint: CheckpointCut,
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
        // Equality projects the ordered state-history position. Admission
        // identity is checked separately by restore/release validation: two
        // captures at the same position can have different retained lifetimes.
        self.owner == other.owner
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
        checkpoint: CheckpointCut,
        checkpoint_entries: u32,
        group_depth: u32,
        save_stack: SaveStackProjection,
    ) -> Self {
        Self {
            owner,
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

    pub(super) const fn checkpoint_entries(self) -> u32 {
        self.checkpoint_entries
    }

    pub(super) const fn group_depth(self) -> u32 {
        self.group_depth
    }
}

/// One exact prior cell value for group or checkpoint rollback.
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

/// Scalar active-group metadata. All saved values live in the shared chunk
/// sequence, never in a group-owned `Vec`.
#[derive(Clone, Copy)]
struct GroupMark {
    frame: GroupFrame,
    /// Absolute slot of the fixed group-enter marker.
    start: usize,
    /// Number of mutation records belonging to this group.
    entries: usize,
    /// First sparse-array mutation in this segment.  Keeping this boundary
    /// lets one reverse walk produce TeX's dense-then-sparse restoration order.
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
}

/// One compact reversible value retained for a checkpoint interval.
pub(crate) struct CheckpointDelta<G> {
    pub(crate) cell: StateCell,
    pub(crate) alternate: StateWord<G>,
    pub(crate) alternate_level: u32,
}

/// Accepted checkpoint material temporarily detached while the current
/// candidate owns the live dense state. The group journal is deliberately not
/// part of this owner: accepted checkpoints are captured at level zero with an
/// empty group journal, and candidate group records are dropped wholesale on
/// rejection.
pub(crate) struct AcceptedJournalTail<G> {
    prior_checkpoint_entries: usize,
    save_stack: SaveStackProjection,
    save_position: u32,
    _brand: PhantomData<fn() -> G>,
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
    checkpoint_undo: CheckpointUndoLog<G>,
    checkpoint_entries: usize,
    host_epoch: u64,
    save_stack: SaveStackProjection,
    save_position: u32,
    /// Reusable scratch for sparse-array records deferred to the restore
    /// marker during one group unwind.  It is journal-owned so repeated
    /// sparse groups reuse its capacity instead of allocating on every exit.
    sparse_scratch: Vec<(usize, Mutation<G>)>,
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

        Self {
            owner,
            active_sequence: SaveSequence::default(),
            active_groups: Vec::new(),
            checkpoint_undo: CheckpointUndoLog::default(),
            checkpoint_entries: 0,
            // Before the first named checkpoint there is no earlier state to
            // retain. Cells and the journal therefore begin in the same
            // interval; sealing the first checkpoint advances the serial and
            // makes its first subsequent write capture one alternate.
            host_epoch: 0,
            save_stack: SaveStackProjection::default(),
            save_position: 0,
            sparse_scratch: Vec::new(),
            group_capacity_bytes: 0,
            checkpoint_capacity_bytes: 0,
            #[cfg(test)]
            first_touch_cell_visits: 0,
            #[cfg(test)]
            group_entry_visits: Cell::new(0),
            #[cfg(feature = "profiling")]
            profile: SaveJournalProfile::default(),
        }
    }

    /// Captures the dense checkpoint lane at a quiescent level-zero boundary.
    ///
    /// The ordinary TeX group journal is candidate-local and must be empty at
    /// this boundary. Keeping this check here (at the lowest state entry)
    /// prevents a caller from accidentally reintroducing an active-group scan
    /// or retaining group records for editor rollback.
    pub(crate) fn checkpoint_cursor(
        &mut self,
        group_depth: usize,
    ) -> Result<JournalCursor<G>, crate::StateError> {
        if group_depth != 0 || !self.active_groups.is_empty() || self.active_sequence.len != 0 {
            return Err(crate::StateError::CheckpointIneligible);
        }
        self.preflight_epoch_advance(if self.checkpoint_undo.is_candidate() {
            2
        } else {
            1
        })?;
        let group_depth =
            u32::try_from(group_depth).map_err(|_| crate::StateError::CheckpointEpochExhausted)?;
        // Check epoch exhaustion before sealing the arena so an error leaves
        // every owner untouched.
        let next_serial = self
            .host_epoch
            .checked_add(1)
            .ok_or(crate::StateError::CheckpointEpochExhausted)?;
        let cursor = JournalCursor::new(
            self.owner,
            self.checkpoint_undo.capture()?,
            u32::try_from(self.checkpoint_entries).expect("checkpoint journal exceeds u32 entries"),
            group_depth,
            self.save_stack,
        )
        .with_save_position(self.save_position);
        self.host_epoch = next_serial;
        self.refresh_checkpoint_capacity_bytes();
        Ok(cursor)
    }

    #[must_use]
    pub(crate) fn checkpoint_eligible(&self) -> bool {
        self.active_groups.is_empty() && self.active_sequence.len == 0
    }

    pub(crate) fn preflight_epoch_advance(&self, advances: u64) -> Result<(), crate::StateError> {
        self.host_epoch
            .checked_add(advances)
            .map(|_| ())
            .ok_or(crate::StateError::CheckpointEpochExhausted)
    }

    pub(crate) fn preflight_restore_epoch(&self) -> Result<(), crate::StateError> {
        // Every cold candidate boundary keeps one epoch available for its
        // infallible acceptance/rejection. Assignments consume no epochs.
        self.preflight_epoch_advance(if self.checkpoint_undo.is_candidate() {
            2
        } else {
            1
        })
    }

    pub(crate) fn record_assignment(
        &mut self,
        cell: StateCell,
        before: StateWord<G>,
        before_level: u32,
        before_host_epoch: u64,
        saved_at: Option<u32>,
    ) {
        #[cfg(feature = "profiling")]
        self.record_profile_word(&before);
        #[cfg(test)]
        {
            self.first_touch_cell_visits = self.first_touch_cell_visits.saturating_add(1);
        }
        if self.checkpoint_undo.retained() && before_host_epoch != self.host_epoch {
            #[cfg(feature = "profiling")]
            self.record_profile_growth(
                self.checkpoint_entries,
                self.checkpoint_undo.capacity(),
                core::mem::size_of::<CheckpointDelta<G>>(),
            );
            self.checkpoint_undo.push(CheckpointDelta {
                cell,
                alternate: before.clone(),
                alternate_level: before_level,
            });
            self.checkpoint_entries = self
                .checkpoint_entries
                .checked_add(1)
                .expect("checkpoint journal exceeds usize entries");
            self.refresh_checkpoint_capacity_bytes();
        }

        if saved_at.is_some() {
            let mutation = Mutation::new(cell, before, before_level, saved_at);
            let position = self
                .active_sequence
                .len
                .checked_add(1)
                .and_then(|position| u32::try_from(position).ok())
                .expect("group save stack exceeds u32 entries");
            let entry = JournalEntry::Mutation(mutation);
            self.save_stack.push(&entry, position);
            #[cfg(feature = "profiling")]
            self.record_profile_group_append();
            let grew = self.active_sequence.append(entry);
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
    pub(crate) fn needs_mutation(&self, before_host_epoch: u64, saved_at: Option<u32>) -> bool {
        (self.checkpoint_undo.retained() && before_host_epoch != self.host_epoch)
            || saved_at.is_some()
    }

    pub(crate) fn take_sparse_scratch(&mut self) -> Vec<(usize, Mutation<G>)> {
        std::mem::take(&mut self.sparse_scratch)
    }

    pub(crate) fn return_sparse_scratch(&mut self, mut scratch: Vec<(usize, Mutation<G>)>) {
        scratch.clear();
        self.sparse_scratch = scratch;
        self.refresh_group_capacity_bytes();
    }

    /// The monotonic serial written directly into every cell mutated in the
    /// current checkpoint interval.
    #[inline(always)]
    pub(crate) const fn host_epoch(&self) -> u64 {
        self.host_epoch
    }

    #[cfg(test)]
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
        self.active_groups.push(GroupMark {
            frame,
            start: marker,
            entries: 0,
            sparse_start: None,
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
    /// records. This helper is test-only; production `Env` restores records
    /// first and then calls the same scalar transition.
    #[cfg(test)]
    pub(crate) fn record_group_exit(&mut self, frame: GroupFrame) {
        self.record_group_exit_with_records(frame);
    }

    /// Finishes one group after the environment has already consumed its
    /// records in reverse. Group history is always candidate-local, so the
    /// active suffix is truncated and no record is copied or retained.
    pub(crate) fn record_group_exit_with_records(&mut self, frame: GroupFrame) {
        #[cfg(feature = "profiling")]
        self.record_profile_group_exit();
        let mark = self
            .active_groups
            .pop()
            .expect("group exit has a save segment");
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
            .saturating_add(self.checkpoint_undo.len())
    }

    #[must_use]
    pub(crate) const fn retained_bytes(&self) -> usize {
        self.group_capacity_bytes
            .saturating_add(self.checkpoint_capacity_bytes)
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

    #[cfg(test)]
    pub(crate) fn visit_checkpoint_prefix(
        &self,
        cursor: JournalCursor<G>,
        visit: impl FnMut(&CheckpointDelta<G>),
    ) {
        self.checkpoint_undo.visit_prefix(cursor.checkpoint, visit);
    }

    pub(crate) fn visit_checkpoint_suffix(
        &self,
        cursor: JournalCursor<G>,
        visit: impl FnMut(&CheckpointDelta<G>),
    ) {
        self.checkpoint_undo.visit_suffix(cursor.checkpoint, visit);
    }

    pub(crate) fn visit_checkpoint_suffix_mut_reverse(
        &mut self,
        cursor: JournalCursor<G>,
        visit: impl FnMut(&mut CheckpointDelta<G>),
    ) {
        self.checkpoint_undo
            .reverse_suffix(cursor.checkpoint, visit);
    }

    pub(crate) fn visit_current_suffix_mut_reverse(
        &mut self,
        cursor: JournalCursor<G>,
        visit: impl FnMut(&mut CheckpointDelta<G>),
    ) {
        self.checkpoint_undo
            .reverse_suffix(cursor.checkpoint, visit);
    }

    pub(crate) fn visit_detached_suffix_mut(&mut self, visit: impl FnMut(&mut CheckpointDelta<G>)) {
        self.checkpoint_undo.forward_accepted(visit);
    }

    pub(crate) fn visit_detached_prefix(
        &self,
        cursor: JournalCursor<G>,
        visit: impl FnMut(&CheckpointDelta<G>),
    ) -> bool {
        self.checkpoint_undo
            .visit_accepted_prefix(cursor.checkpoint, visit)
    }

    /// Detaches the accepted checkpoint-delta suffix and starts a candidate at
    /// a level-zero boundary. The ordinary group journal remains one mutable
    /// candidate-local sequence; it is never forked or retained.
    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        cursor: JournalCursor<G>,
    ) -> Result<AcceptedJournalTail<G>, crate::StateError> {
        if !self.validate_cursor(cursor) || self.checkpoint_undo.is_candidate() {
            return Err(crate::StateError::InvalidCursor);
        }
        if cursor.group_depth() != 0
            || !self.active_groups.is_empty()
            || self.active_sequence.len != 0
        {
            return Err(crate::StateError::CheckpointIneligible);
        }
        // Candidate setup advances once for the new interval and settlement
        // advances once again when that interval is accepted or rejected.
        // Reserve both epochs before detaching the accepted arena so an
        // exhausted serial cannot leave a half-forked journal behind.
        let candidate_serial = self
            .host_epoch
            .checked_add(1)
            .ok_or(crate::StateError::CheckpointEpochExhausted)?;
        candidate_serial
            .checked_add(1)
            .ok_or(crate::StateError::CheckpointEpochExhausted)?;
        let prior_checkpoint_entries = self.checkpoint_entries;
        self.checkpoint_undo.begin_candidate(cursor.checkpoint)?;
        self.checkpoint_entries = cursor.checkpoint_entries() as usize;
        let tail = AcceptedJournalTail {
            prior_checkpoint_entries,
            save_stack: self.save_stack,
            save_position: self.save_position,
            _brand: PhantomData,
        };
        self.save_stack = SaveStackProjection::default();
        self.save_position = 0;
        self.host_epoch = candidate_serial;
        self.refresh_group_capacity_bytes();
        Ok(tail)
    }

    /// Restores the accepted group/journal topology after the candidate lane
    /// has already been destructively returned to its empty root.
    pub(crate) fn reject_checkpoint_candidate(&mut self, tail: AcceptedJournalTail<G>) {
        self.active_sequence.truncate(0);
        self.active_groups.clear();
        self.save_stack = tail.save_stack;
        self.save_position = tail.save_position;
        self.checkpoint_entries = tail.prior_checkpoint_entries;
        self.checkpoint_undo.reject();
        self.refresh_checkpoint_capacity_bytes();
        self.refresh_group_capacity_bytes();
        self.advance_host_epoch();
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self) -> Result<(), crate::StateError> {
        if !self.checkpoint_undo.is_candidate()
            || !self.active_groups.is_empty()
            || self.active_sequence.len != 0
        {
            return Err(crate::StateError::CheckpointIneligible);
        }
        self.preflight_epoch_advance(1)?;
        self.checkpoint_undo.accept()?;
        self.refresh_checkpoint_capacity_bytes();
        self.advance_host_epoch();
        Ok(())
    }

    pub(crate) fn truncate_checkpoint(&mut self, cursor: JournalCursor<G>) {
        assert_eq!(
            cursor.owner, self.owner,
            "journal cursor belongs to another state"
        );
        self.checkpoint_undo.restore(cursor.checkpoint);
        self.refresh_checkpoint_capacity_bytes();
        self.checkpoint_entries = cursor.checkpoint_entries() as usize;
        self.advance_host_epoch();
        self.save_stack = cursor.save_stack;
        self.save_position = cursor.save_position;
    }

    #[cfg(test)]
    pub(crate) fn release_checkpoint_prefix(
        &mut self,
        cursor: JournalCursor<G>,
    ) -> Result<usize, crate::StateError> {
        if !self.validate_release_cursor(cursor) {
            return Err(crate::StateError::InvalidCursor);
        }
        let released = self.checkpoint_undo.release_prefix(cursor.checkpoint)?;
        self.refresh_checkpoint_capacity_bytes();
        Ok(released)
    }

    pub(crate) fn release_checkpoint_root(
        &mut self,
        released: JournalCursor<G>,
        oldest: Option<JournalCursor<G>>,
    ) -> Result<(), crate::StateError> {
        if !self.validate_release_cursor(released)
            || oldest.is_some_and(|cursor| !self.validate_release_cursor(cursor))
        {
            return Err(crate::StateError::InvalidCursor);
        }
        self.checkpoint_undo
            .release_root(released.checkpoint, oldest.map(|cursor| cursor.checkpoint))?;
        self.refresh_checkpoint_capacity_bytes();
        Ok(())
    }

    pub(crate) fn release_unretained_capture(&mut self, cursor: JournalCursor<G>) {
        self.checkpoint_undo.release_capture(cursor.checkpoint);
        self.refresh_checkpoint_capacity_bytes();
    }

    /// Prefix reclamation does not restore state or consume ordinary group
    /// saves. It remains valid during TeX's final cleanup with open groups.
    pub(crate) fn validate_release_cursor(&self, cursor: JournalCursor<G>) -> bool {
        self.validate_cursor_shape(cursor) && !self.checkpoint_undo.is_candidate()
    }

    #[must_use]
    pub(crate) fn validate_cursor(&self, cursor: JournalCursor<G>) -> bool {
        self.validate_cursor_shape(cursor)
            && self.active_groups.is_empty()
            && self.active_sequence.len == 0
    }

    /// Validates the retained checkpoint coordinate after a halted session.
    /// The target is still an outer level-zero mark, but the interrupted
    /// command may have left ordinary group-save records in the live suffix;
    /// the restore operation owns replacing that suffix.
    pub(crate) fn validate_cursor_after_replay(&self, cursor: JournalCursor<G>) -> bool {
        self.validate_cursor_shape(cursor)
    }

    fn validate_cursor_shape(&self, cursor: JournalCursor<G>) -> bool {
        cursor.owner == self.owner
            && cursor.checkpoint_entries() as usize <= self.checkpoint_entries
            && self.checkpoint_undo.validates(cursor.checkpoint)
            && cursor.group_depth() == 0
            && cursor.save_position == 0
    }

    pub(crate) fn restore_group_cursor(&mut self, cursor: JournalCursor<G>) -> RestoredGroups {
        debug_assert!(self.validate_cursor(cursor));
        self.restore_group_cursor_after_replay(cursor)
    }

    pub(crate) fn restore_group_cursor_after_replay(
        &mut self,
        cursor: JournalCursor<G>,
    ) -> RestoredGroups {
        self.active_sequence.truncate(0);
        self.active_groups.clear();
        self.save_stack = cursor.save_stack;
        self.save_position = cursor.save_position;
        self.refresh_group_capacity_bytes();
        RestoredGroups::Truncate(0)
    }

    pub(crate) fn group_save_len(&self) -> usize {
        self.active_sequence.len
    }

    #[cfg(feature = "profiling")]
    fn group_mutation_len(&self) -> usize {
        self.active_groups.iter().map(|group| group.entries).sum()
    }

    #[cfg(feature = "profiling")]
    fn group_mutation_capacity(&self) -> usize {
        self.active_sequence
            .capacity_slots()
            .saturating_add(self.sparse_scratch.capacity())
    }

    #[cfg(test)]
    fn retained_bytes_census(&self) -> usize {
        self.group_capacity_bytes_census()
            .saturating_add(self.checkpoint_undo.retained_bytes())
    }

    #[cfg(test)]
    fn group_capacity_bytes_census(&self) -> usize {
        self.active_sequence
            .capacity_bytes
            .saturating_add(
                self.active_groups
                    .capacity()
                    .saturating_mul(core::mem::size_of::<GroupMark>()),
            )
            .saturating_add(
                self.sparse_scratch
                    .capacity()
                    .saturating_mul(core::mem::size_of::<(usize, Mutation<G>)>()),
            )
    }

    fn refresh_group_capacity_bytes(&mut self) {
        self.group_capacity_bytes = self
            .active_sequence
            .capacity_bytes
            .saturating_add(
                self.active_groups
                    .capacity()
                    .saturating_mul(core::mem::size_of::<GroupMark>()),
            )
            .saturating_add(
                self.sparse_scratch
                    .capacity()
                    .saturating_mul(core::mem::size_of::<(usize, Mutation<G>)>()),
            );
    }

    fn refresh_checkpoint_capacity_bytes(&mut self) {
        self.checkpoint_capacity_bytes = self.checkpoint_undo.retained_bytes();
    }

    fn advance_host_epoch(&mut self) {
        self.host_epoch = self
            .host_epoch
            .checked_add(1)
            .expect("dense save serial space exhausted");
    }

    #[cfg(feature = "profiling")]
    fn record_profile_word(&mut self, before: &StateWord<G>) {
        self.profile.append_calls = self.profile.append_calls.saturating_add(1);
        self.profile.mutations = self.profile.mutations.saturating_add(1);
        let word = match before {
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

#[cfg(feature = "profiling")]
impl<G> Drop for SaveJournal<G> {
    fn drop(&mut self) {
        crate::measurement::record_save_journal_census(crate::measurement::SaveJournalCensus {
            entries: u64::try_from(self.retained_len()).unwrap_or(u64::MAX),
            capacity: u64::try_from(
                self.group_mutation_capacity()
                    .saturating_add(self.checkpoint_undo.capacity()),
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
            checkpoint_capacity: u64::try_from(self.checkpoint_undo.capacity()).unwrap_or(u64::MAX),
            checkpoint_entry_size: u64::try_from(core::mem::size_of::<CheckpointDelta<G>>())
                .unwrap_or(u64::MAX),
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
