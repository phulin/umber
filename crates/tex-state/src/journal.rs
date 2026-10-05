//! TeX group saves and first-touch checkpoint undo, with independent preimages.

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

/// A stable logical cursor in one generation's checkpoint history.
pub struct JournalCursor<G> {
    owner: u64,
    checkpoint: CheckpointCut,
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
        // Equality projects the ordered state-history position. Admission
        // identity is checked separately by restore/release validation: two
        // captures at the same position can have different retained lifetimes.
        self.owner == other.owner
            && self.checkpoint_entries == other.checkpoint_entries
            && self.group_depth == other.group_depth
            && self.save_stack == other.save_stack
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
            _brand: PhantomData,
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
    sparse_link: Option<SaveIndex>,
}

impl<G> Mutation<G> {
    pub(crate) fn new(cell: StateCell, before: StateWord<G>, before_level: u32) -> Self {
        Self {
            cell: JournalCell::pack(cell),
            before,
            before_level,
            sparse_link: None,
        }
    }

    pub(crate) fn cell(&self) -> StateCell {
        self.cell.unpack()
    }
}

impl<G> core::fmt::Debug for Mutation<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Mutation")
            .field("cell", &self.cell())
            .field("before_level", &self.before_level)
            .finish_non_exhaustive()
    }
}

mod groups;
pub(crate) use groups::JournalEntry;
use groups::{GroupBoundary, GroupSaveStack, SaveIndex};
pub use groups::{GroupFrames, GroupFramesIter};

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
    _brand: PhantomData<fn() -> G>,
}

/// Allocation-free projection of the state-owned part of TeX's save stack.
///
/// The native stack includes `\aftergroup` words. The last checked push adds
/// one or two words; zero means no push. Box-spec words remain executor-owned.
/// §§1334/273 report the depth immediately before that checked push.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SaveStackProjection {
    words: usize,
    last_push_words: u8,
}

impl SaveStackProjection {
    fn push<G>(&mut self, entry: &JournalEntry<G>) {
        match entry {
            JournalEntry::Mutation(mutation) => {
                let words = canonical_restore_words(mutation);
                self.words = self.words.saturating_add(usize::from(words));
                self.last_push_words = words;
            }
            JournalEntry::Box { .. } => {
                self.words += 2;
                self.last_push_words = 2;
            }
            JournalEntry::AfterGroup(_) | JournalEntry::SparseMarker { .. } => {
                self.words += 1;
                self.last_push_words = 1;
            }
            JournalEntry::Vacant => {}
        }
    }
}

/// Split rollback storage for one live revision generation.
pub(crate) struct SaveJournal<G> {
    owner: u64,
    /// Current append/truncate sequence.  Its slots are the only ordinary
    /// save-record address space.
    active_sequence: GroupSaveStack<G>,
    checkpoint_undo: CheckpointUndoLog<G>,
    checkpoint_entries: usize,
    host_epoch: u64,
    save_stack: SaveStackProjection,
    box_spec_words: usize,
    max_save_words: [usize; 2],
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
    #[cfg(test)]
    pub(crate) fn attach_definition_roots(
        &mut self,
        roots: crate::definition_arena::DefinitionRoots,
    ) {
        self.active_sequence.roots = roots.clone();
        self.checkpoint_undo.roots = roots;
    }

    #[must_use]
    pub(crate) fn new(roots: crate::definition_arena::DefinitionRoots) -> Self {
        let owner = NEXT_JOURNAL_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("state journal identity space exhausted");

        Self {
            owner,
            active_sequence: GroupSaveStack::new(roots.clone()),
            checkpoint_undo: CheckpointUndoLog::new(roots),
            checkpoint_entries: 0,
            // Before the first named checkpoint there is no earlier state to
            // retain. Cells and the journal therefore begin in the same
            // interval; sealing the first checkpoint advances the serial and
            // makes its first subsequent write capture one alternate.
            host_epoch: 0,
            save_stack: SaveStackProjection::default(),
            box_spec_words: 0,
            max_save_words: [0; 2],
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
        if group_depth != 0
            || !self.active_sequence.is_empty()
            || !self.active_sequence.entries.is_empty()
        {
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
        );
        self.host_epoch = next_serial;

        Ok(cursor)
    }

    #[must_use]
    pub(crate) fn checkpoint_eligible(&self) -> bool {
        self.active_sequence.is_empty() && self.active_sequence.entries.is_empty()
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
        }

        if saved_at.is_some() {
            let mutation = Mutation::new(cell, before, before_level);
            self.append_group_entry(
                JournalEntry::Mutation(mutation),
                is_extended_register_cell(cell),
            );
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
    pub(crate) fn needs_mutation(&self, before_host_epoch: u64, saved_at: Option<u32>) -> bool {
        (self.checkpoint_undo.retained() && before_host_epoch != self.host_epoch)
            || saved_at.is_some()
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

    #[inline]
    pub(crate) fn record_group_enter(&mut self, frame: GroupFrame) {
        #[cfg(feature = "profiling")]
        {
            self.record_profile_group_enter();
            self.record_profile_group_append();
            self.record_profile_growth(
                self.active_sequence.boundaries.len(),
                self.active_sequence.boundaries.capacity(),
                core::mem::size_of::<GroupBoundary>(),
            );
        }
        debug_assert_eq!(self.save_stack.words, frame.save_stack_words_before);
        debug_assert_eq!(
            self.save_stack.last_push_words,
            frame.last_save_push_words_before
        );
        self.save_stack.words = self.save_stack.words.saturating_add(1);
        self.save_stack.last_push_words = 1;
        self.active_sequence.boundaries.push(GroupBoundary {
            frame,
            sparse_marker: None,
        });
        self.record_save_words_peak(self.active_sequence.boundaries.len());
    }

    fn append_group_entry(&mut self, mut entry: JournalEntry<G>, sparse: bool) {
        if sparse {
            let marker = self
                .active_sequence
                .boundaries
                .last()
                .expect("save has an active group")
                .sparse_marker
                .map(SaveIndex::get);
            let marker = marker.unwrap_or_else(|| {
                let marker = self.active_sequence.entries.len();
                self.append_group_entry(JournalEntry::SparseMarker { head: None }, false);
                self.active_sequence
                    .boundaries
                    .last_mut()
                    .expect("active group")
                    .sparse_marker = Some(SaveIndex::new(marker));
                marker
            });
            let previous = self.active_sequence.sparse_head(marker);
            entry.set_sparse_link(previous);
            self.active_sequence
                .set_sparse_head(marker, Some(self.active_sequence.entries.len()));
        }
        assert!(
            self.active_sequence.entries.len() < u32::MAX as usize,
            "group save stack fits u32"
        );
        if !sparse {
            self.save_stack.push(&entry);
            self.record_save_words_peak(self.active_sequence.boundaries.len());
        }
        #[cfg(feature = "profiling")]
        {
            self.record_profile_group_append();
            self.record_profile_growth(
                self.active_sequence.entries.len(),
                self.active_sequence.entries.capacity(),
                core::mem::size_of::<JournalEntry<G>>(),
            );
        }
        self.active_sequence.append(entry);
    }

    #[cfg(test)]
    pub(crate) fn record_group_exit(&mut self, frame: GroupFrame) {
        self.record_group_exit_with_records(frame);
    }

    pub(crate) fn record_group_exit_with_records(&mut self, frame: GroupFrame) {
        #[cfg(feature = "profiling")]
        self.record_profile_group_exit();
        let boundary = self
            .active_sequence
            .boundaries
            .pop()
            .expect("group exit has a boundary");
        debug_assert_eq!(boundary.frame.lineage(), frame.lineage());
        if self.active_sequence.entries.len() != frame.journal_start as usize {
            self.active_sequence.truncate(frame.journal_start as usize);
        }
        self.active_sequence.unsaving = false;
        self.save_stack = SaveStackProjection {
            words: frame.save_stack_words_before,
            last_push_words: frame.last_save_push_words_before,
        };
    }

    pub(crate) fn record_box_save(&mut self, saved: crate::env::durable_boxes::DurableMutation) {
        let sparse = saved.index > 255;
        self.append_group_entry(
            JournalEntry::Box {
                saved,
                sparse_link: None,
            },
            sparse,
        );
    }

    pub(crate) fn retire_group_boxes(
        &mut self,
        boxes: &mut crate::env::durable_boxes::DurableBoxState,
        arena: &mut crate::page_node_arena::PageMaterialArena,
    ) {
        for index in 0..self.active_sequence.entries.len() {
            if matches!(
                self.active_sequence.entries[index],
                JournalEntry::Box { .. }
            ) && let JournalEntry::Box { saved, .. } = self.take_group_entry(index)
            {
                boxes.retire_group_record(arena, saved);
            }
        }
    }

    pub(crate) fn save_aftergroup(&mut self, token: crate::token::TracedTokenWord) {
        self.append_group_entry(JournalEntry::AfterGroup(token), false);
    }

    pub(crate) fn begin_unsave(&mut self) {
        assert!(!self.active_sequence.unsaving, "unsave is synchronous");
        self.active_sequence.unsaving = true;
    }

    pub(crate) fn current_level(&self) -> u32 {
        // Group admission checks that depth plus the root level fits u32.
        self.active_sequence.boundaries.len() as u32 + crate::env::banks::LEVEL_ONE
            - u32::from(self.active_sequence.unsaving)
    }

    pub(crate) fn group_frames(&self) -> GroupFrames<'_, G> {
        self.active_sequence.frames()
    }

    pub(crate) fn sparse_head(&self, marker: usize) -> Option<usize> {
        self.active_sequence.sparse_head(marker)
    }

    pub(crate) fn take_group_entry(&mut self, index: usize) -> JournalEntry<G> {
        std::mem::replace(
            &mut self.active_sequence.entries[index],
            JournalEntry::Vacant,
        )
    }

    /// Native live words and the word count of the last checked push.
    #[must_use]
    pub(crate) const fn save_stack_projection(&self) -> (usize, u8) {
        (self.save_stack.words, self.save_stack.last_push_words)
    }

    fn record_save_words_peak(&mut self, depth: usize) {
        let words = self
            .save_stack
            .words
            .saturating_add(self.box_spec_words)
            .saturating_sub(usize::from(self.save_stack.last_push_words));
        self.max_save_words[0] = self.max_save_words[0].max(words);
        self.max_save_words[1] = self.max_save_words[1].max(words.saturating_add(depth));
    }

    pub(crate) const fn max_save_stack_words(&self, source_lines: bool) -> usize {
        self.max_save_words[source_lines as usize]
    }

    pub(crate) fn add_box_spec_words(&mut self, words: usize) {
        self.box_spec_words += words;
        self.record_save_words_peak(self.active_sequence.boundaries.len());
    }

    pub(crate) fn remove_box_spec_words(&mut self, words: usize) {
        self.box_spec_words -= words;
    }

    /// Current active sequence length, including fixed group-enter markers.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.active_sequence.entries.len()
    }

    #[must_use]
    pub(crate) fn retained_len(&self) -> usize {
        self.group_save_len()
            .saturating_add(self.checkpoint_undo.len())
    }

    #[must_use]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.group_capacity_bytes()
            .saturating_add(self.checkpoint_undo.retained_bytes())
    }

    /// Direct absolute save-index lookup.  This function deliberately has no
    /// topology fallback: group marks are consulted only by group-specific
    /// checkpoint operations, never by ordinary journal access.
    pub(crate) fn entry(&self, index: usize) -> &JournalEntry<G> {
        #[cfg(test)]
        self.group_entry_visits
            .set(self.group_entry_visits.get().saturating_add(1));
        self.active_sequence
            .entry(index)
            .unwrap_or_else(|| panic!("group save entry index {index} out of bounds"))
    }

    #[cfg(test)]
    pub(crate) const fn group_entry_visits(&self) -> usize {
        self.group_entry_visits.get()
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
            || !self.active_sequence.is_empty()
            || !self.active_sequence.entries.is_empty()
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
            _brand: PhantomData,
        };
        self.save_stack = SaveStackProjection::default();
        self.host_epoch = candidate_serial;

        Ok(tail)
    }

    /// Restores the accepted group/journal topology after the candidate lane
    /// has already been destructively returned to its empty root.
    pub(crate) fn reject_checkpoint_candidate(&mut self, tail: AcceptedJournalTail<G>) {
        self.active_sequence.truncate(0);
        self.active_sequence.boundaries.clear();
        self.box_spec_words = 0;
        self.active_sequence.unsaving = false;
        self.save_stack = tail.save_stack;
        self.checkpoint_entries = tail.prior_checkpoint_entries;
        self.checkpoint_undo.reject();

        self.advance_host_epoch();
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self) -> Result<(), crate::StateError> {
        if !self.checkpoint_undo.is_candidate()
            || !self.active_sequence.is_empty()
            || !self.active_sequence.entries.is_empty()
        {
            return Err(crate::StateError::CheckpointIneligible);
        }
        self.preflight_epoch_advance(1)?;
        self.checkpoint_undo.accept()?;

        self.advance_host_epoch();
        Ok(())
    }

    pub(crate) fn truncate_checkpoint(&mut self, cursor: JournalCursor<G>) {
        assert_eq!(
            cursor.owner, self.owner,
            "journal cursor belongs to another state"
        );
        self.checkpoint_undo.restore(cursor.checkpoint);

        self.checkpoint_entries = cursor.checkpoint_entries() as usize;
        self.advance_host_epoch();
        self.save_stack = cursor.save_stack;
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

        Ok(())
    }

    pub(crate) fn release_unretained_capture(&mut self, cursor: JournalCursor<G>) {
        self.checkpoint_undo.release_capture(cursor.checkpoint);
    }

    /// Prefix reclamation does not restore state or consume ordinary group
    /// saves. It remains valid during TeX's final cleanup with open groups.
    pub(crate) fn validate_release_cursor(&self, cursor: JournalCursor<G>) -> bool {
        self.validate_cursor_shape(cursor) && !self.checkpoint_undo.is_candidate()
    }

    #[must_use]
    pub(crate) fn validate_cursor(&self, cursor: JournalCursor<G>) -> bool {
        self.validate_cursor_shape(cursor)
            && self.active_sequence.is_empty()
            && self.active_sequence.entries.is_empty()
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
            && cursor.save_stack == SaveStackProjection::default()
    }

    pub(crate) fn restore_group_cursor(&mut self, cursor: JournalCursor<G>) {
        debug_assert!(self.validate_cursor(cursor));
        self.restore_group_cursor_after_replay(cursor)
    }

    pub(crate) fn restore_group_cursor_after_replay(&mut self, cursor: JournalCursor<G>) {
        self.active_sequence.truncate(0);
        self.active_sequence.boundaries.clear();
        self.box_spec_words = 0;
        self.active_sequence.unsaving = false;
        self.save_stack = cursor.save_stack;
    }

    pub(crate) fn group_save_len(&self) -> usize {
        self.active_sequence.entries.len() + self.active_sequence.boundaries.len()
    }

    #[cfg(test)]
    fn retained_bytes_census(&self) -> usize {
        self.group_capacity_bytes_census() + self.checkpoint_undo.retained_bytes()
    }

    fn group_capacity_bytes(&self) -> usize {
        self.active_sequence.entries.capacity() * core::mem::size_of::<JournalEntry<G>>()
            + self.active_sequence.boundaries.capacity() * core::mem::size_of::<GroupBoundary>()
    }

    #[cfg(test)]
    fn group_capacity_bytes_census(&self) -> usize {
        self.active_sequence.entries.capacity() * core::mem::size_of::<JournalEntry<G>>()
            + self.active_sequence.boundaries.capacity() * core::mem::size_of::<GroupBoundary>()
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
                self.active_sequence
                    .entries
                    .capacity()
                    .saturating_add(self.active_sequence.boundaries.capacity())
                    .saturating_add(self.checkpoint_undo.capacity()),
            )
            .unwrap_or(u64::MAX),
            peak_entries: u64::try_from(self.profile.peak_entries).unwrap_or(u64::MAX),
            entry_size: u64::try_from(core::mem::size_of::<JournalEntry<G>>()).unwrap_or(u64::MAX),
            mutation_size: u64::try_from(core::mem::size_of::<Mutation<G>>()).unwrap_or(u64::MAX),
            group_frame_size: u64::try_from(core::mem::size_of::<GroupFrame>()).unwrap_or(u64::MAX),
            boundary_entries: u64::try_from(self.active_sequence.boundaries.len())
                .unwrap_or(u64::MAX),
            boundary_capacity: u64::try_from(self.active_sequence.boundaries.capacity())
                .unwrap_or(u64::MAX),
            boundary_size: u64::try_from(core::mem::size_of::<GroupBoundary>()).unwrap_or(u64::MAX),
            group_entries: u64::try_from(self.active_sequence.entries.len()).unwrap_or(u64::MAX),
            group_capacity: u64::try_from(self.active_sequence.entries.capacity())
                .unwrap_or(u64::MAX),
            group_entry_size: u64::try_from(core::mem::size_of::<JournalEntry<G>>())
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

fn canonical_restore_words<G>(mutation: &Mutation<G>) -> u8 {
    // TeX82 §§275--276 uses one word for `restore_zero` and two for
    // `restore_old_value`. Undefined meanings are physically level zero.
    // Section 240 gives null token-list parameters the same level-zero
    // representation even though Umber's fixed typed bank stores its virtual
    // default at level one.
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
    }
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
