//! Generation-owned authoritative input rows and ordered inverse history.

use core::hash::{Hash, Hasher};
use core::ops::{Deref, Index};
use core::slice::SliceIndex;

use crate::scalar_journal::{PackedJournal, PackedJournalMark};
use crate::timeline::{PayloadHandle, PayloadSlab};

use super::{
    InputLevel, InputLevelInlineState, SourceLevel, SourceLevelExecutionState,
    SourceLexExecutionState, SourceSlot, SourceSlotKey,
};

const INPUT_UNDO_RECORDS_PER_CHUNK: usize = 16;

/// One inverse in the authoritative ordering of all input mutations.
///
/// Source owners and displaced rows live in generation-checked reusable slabs;
/// the journal retains only their compact handles. Token and macro-argument
/// frame state is copy-small and remains inline.
enum InputUndo<G> {
    Inline {
        index: u32,
        state: InputLevelInlineState,
    },
    SourceLex {
        index: u32,
        payload: PayloadHandle,
    },
    SourceOwner {
        index: u32,
        payload: PayloadHandle,
        generation: core::marker::PhantomData<fn() -> G>,
    },
    /// An editor-root backing substitution. Keeping this distinct from a
    /// complete owner capture lets the first later cursor mutation retain
    /// the checkpoint's execution state in the ordinary ordered journal.
    PhysicalBacking {
        index: u32,
        payload: PayloadHandle,
        generation: core::marker::PhantomData<fn() -> G>,
    },
    Replacement {
        index: u32,
        payload: PayloadHandle,
    },
}

/// One rollback coordinate in a generation's dedicated input history.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct InputStackMark {
    pub(crate) top: u32,
    pub(crate) undo: PackedJournalMark,
}

#[derive(Debug)]
struct InputStackFork {
    accepted_top: usize,
}

/// One generation-tied input stack whose live rows are authoritative.
///
/// Stable rows cover sources, stored spans, and direct macro-argument ranges.
/// All mutable execution state, cold source-owner swaps, row replacement, and
/// lifecycle changes share the single ordered [`InputUndo`] journal.
pub(crate) struct InputStack<G> {
    rows: Vec<InputLevel<G>>,
    top: usize,
    undo: PackedJournal<InputUndo<G>, INPUT_UNDO_RECORDS_PER_CHUNK>,
    displaced_rows: PayloadSlab<InputLevel<G>>,
    source_lex_states: PayloadSlab<SourceLexExecutionState>,
    source_owner_states: PayloadSlab<SourceLevelExecutionState<G>>,
    source_slots: PayloadSlab<SourceSlot<G>>,
    fork: Option<InputStackFork>,
    recording: bool,
    interval: u64,
    touched: Vec<u64>,
    partially_captured: Vec<u64>,
    source_owner_captured: Vec<u64>,
    coalesced_mutations: u64,
    row_admissions: u64,
    source_lex_captures: u64,
    source_owner_swaps: u64,
}

impl<G> Default for InputStack<G> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            top: 0,
            undo: PackedJournal::default(),
            displaced_rows: PayloadSlab::default(),
            source_lex_states: PayloadSlab::default(),
            source_owner_states: PayloadSlab::default(),
            source_slots: PayloadSlab::default(),
            fork: None,
            recording: false,
            interval: 1,
            touched: Vec::new(),
            partially_captured: Vec::new(),
            source_owner_captured: Vec::new(),
            coalesced_mutations: 0,
            row_admissions: 0,
            source_lex_captures: 0,
            source_owner_swaps: 0,
        }
    }
}

impl<G: core::fmt::Debug> core::fmt::Debug for InputStack<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InputStack")
            .field("rows", &&self.rows[..self.top])
            .field("top", &self.top)
            .finish_non_exhaustive()
    }
}

impl<G: PartialEq> PartialEq for InputStack<G> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice().len() == other.as_slice().len()
            && self
                .as_slice()
                .iter()
                .zip(other.as_slice())
                .all(|(left, right)| match (left, right) {
                    (InputLevel::Source(left), InputLevel::Source(right)) => {
                        left == right
                            && self.source_level_slot(left) == other.source_level_slot(right)
                    }
                    (InputLevel::Tokens(left), InputLevel::Tokens(right)) => left == right,
                    (InputLevel::MacroArgument(left), InputLevel::MacroArgument(right)) => {
                        left == right
                    }
                    _ => false,
                })
    }
}

impl<G: Eq> Eq for InputStack<G> {}

impl<G: Hash> Hash for InputStack<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().len().hash(state);
        for level in self.as_slice() {
            level.hash(state);
            if let InputLevel::Source(source) = level {
                self.source_level_slot(source).hash(state);
            }
        }
    }
}

impl<G> Deref for InputStack<G> {
    type Target = [InputLevel<G>];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<G, I> Index<I> for InputStack<G>
where
    I: SliceIndex<[InputLevel<G>]>,
{
    type Output = I::Output;

    fn index(&self, index: I) -> &Self::Output {
        &self.as_slice()[index]
    }
}

impl<'a, G> IntoIterator for &'a InputStack<G> {
    type Item = &'a InputLevel<G>;
    type IntoIter = core::slice::Iter<'a, InputLevel<G>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<G> InputStack<G> {
    pub(crate) fn rehome_generated_source(
        &mut self,
        accepted: &[u8],
        bytes: std::sync::Arc<[u8]>,
        old_start: usize,
        old_end: usize,
        new_end: usize,
    ) -> Result<bool, crate::SourceRegistrationError> {
        let offsets = super::source::SourceOffsetMap::new(old_start, old_end, new_end);
        let mut rebound = false;
        let mut failure = None;
        let mut replacement = None;
        let mut root_slot = None;
        self.source_slots.for_each_value_mut(|handle, slot| {
            if failure.is_some() || !slot.cursor.backing.is_editor_backing(accepted) {
                return;
            }
            if replacement.is_none() {
                match slot
                    .cursor
                    .backing
                    .rehome_generated(std::sync::Arc::clone(&bytes))
                {
                    Ok(prepared) => replacement = Some(prepared),
                    Err(error) => {
                        failure = Some(error);
                        return;
                    }
                }
            }
            slot.cursor
                .backing
                .clone_from(replacement.as_ref().expect("replacement was prepared"));
            slot.cursor.rehome_offsets(offsets);
            root_slot = Some(SourceSlotKey(handle));
            rebound = true;
        });
        if rebound {
            let root_slot = root_slot.expect("a rebound source owns its slot");
            self.source_lex_states
                .for_each_value_mut(|_, state| state.rehome_offsets(root_slot, offsets));
            let replacement = replacement
                .as_ref()
                .expect("a rebound source prepared its replacement");
            self.source_owner_states.for_each_value_mut(|_, state| {
                state.rehome_physical_backing(root_slot, accepted, replacement);
                state.rehome_offsets(root_slot, offsets);
            });
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(rebound),
        }
    }

    pub(crate) fn push_source(&mut self, frame: super::PackedInputFrame, slot: SourceSlot<G>) {
        self.source_slots.warm_first_page();
        let slot = SourceSlotKey::new(self.source_slots.insert(slot));
        self.push_row(InputLevel::Source(SourceLevel {
            frame,
            slot,
            generation: core::marker::PhantomData,
        }));
    }

    pub(crate) fn source_slot(&self, key: SourceSlotKey) -> &SourceSlot<G> {
        self.source_slots
            .value(key.0)
            .expect("source row names its live ABA-checked slot")
    }

    pub(crate) fn source_level_slot(&self, source: &SourceLevel<G>) -> &SourceSlot<G> {
        self.source_slot(source.slot)
    }

    pub(crate) fn source_slot_for_level(&self, level: &InputLevel<G>) -> Option<&SourceSlot<G>> {
        match level {
            InputLevel::Source(source) => Some(self.source_level_slot(source)),
            InputLevel::Tokens(_) | InputLevel::MacroArgument(_) => None,
        }
    }

    pub(crate) fn top_source(&self) -> Option<(&SourceLevel<G>, &SourceSlot<G>)> {
        let InputLevel::Source(source) = self.rows.get(self.top.checked_sub(1)?)? else {
            return None;
        };
        Some((source, self.source_slot(source.slot)))
    }

    pub(crate) fn physical_source(
        &self,
        id: tex_state::SourceId,
    ) -> Option<&super::RegisteredSource> {
        self.as_slice().iter().find_map(|level| {
            let InputLevel::Source(source) = level else {
                return None;
            };
            let backing = &self.source_level_slot(source).cursor.backing;
            (backing.id == id).then_some(backing)
        })
    }

    pub(crate) fn rebind_physical_source(
        &mut self,
        id: tex_state::SourceId,
        replacement: super::RegisteredSource,
    ) -> bool {
        let Some(index) = self.as_slice().iter().position(|level| {
            let InputLevel::Source(source) = level else {
                return false;
            };
            self.source_level_slot(source).cursor.backing.id == id
        }) else {
            return false;
        };
        let key = match &self.rows[index] {
            InputLevel::Source(source) => source.slot,
            _ => unreachable!("physical source lookup returned a token row"),
        };
        let (rows, slots) = (&mut self.rows, &mut self.source_slots);
        let InputLevel::Source(source) = &mut rows[index] else {
            unreachable!()
        };
        let slot = slots.value_mut(key.0).expect("source slot remains live");
        let state = SourceLevelExecutionState::physical_backing(source, slot, replacement);
        if self.recording {
            self.source_owner_states.warm_first_page();
            let payload = self.source_owner_states.insert(state);
            self.undo.append(InputUndo::PhysicalBacking {
                index: u32::try_from(index).expect("input row index fits u32"),
                payload,
                generation: core::marker::PhantomData,
            });
            self.source_owner_swaps = self.source_owner_swaps.saturating_add(1);
        }
        true
    }

    pub(crate) fn mutate_top_source_lex<R>(
        &mut self,
        mutate: impl FnOnce(&mut SourceLevel<G>, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        let index = self.top.checked_sub(1)?;
        if !matches!(self.rows[index], InputLevel::Source(_)) {
            return None;
        }
        self.record(index);
        let key = match &self.rows[index] {
            InputLevel::Source(source) => source.slot,
            _ => unreachable!(),
        };
        let (rows, slots) = (&mut self.rows, &mut self.source_slots);
        let InputLevel::Source(source) = &mut rows[index] else {
            unreachable!()
        };
        let slot = slots
            .value_mut(key.0)
            .expect("source row names its live ABA-checked slot");
        Some(mutate(source, slot))
    }

    pub(crate) fn mutate_top_tokens<R>(
        &mut self,
        mutate: impl FnOnce(&mut super::TokenCursor<G>) -> R,
    ) -> Option<R> {
        let index = self.top.checked_sub(1)?;
        if !matches!(self.rows[index], InputLevel::Tokens(_)) {
            return None;
        }
        self.record(index);
        let InputLevel::Tokens(tokens) = &mut self.rows[index] else {
            unreachable!()
        };
        Some(mutate(tokens))
    }

    pub(crate) fn mutate_top_macro_argument<R>(
        &mut self,
        mutate: impl FnOnce(&mut super::MacroArgumentCursor<G>) -> R,
    ) -> Option<R> {
        let index = self.top.checked_sub(1)?;
        if !matches!(self.rows[index], InputLevel::MacroArgument(_)) {
            return None;
        }
        self.record(index);
        let InputLevel::MacroArgument(argument) = &mut self.rows[index] else {
            unreachable!()
        };
        Some(mutate(argument))
    }

    pub(crate) fn push(&mut self, value: InputLevel<G>) {
        assert!(
            !matches!(value, InputLevel::Source(_)),
            "source rows are admitted with their sole owner slot"
        );
        self.push_row(value);
    }

    fn push_row(&mut self, value: InputLevel<G>) {
        self.row_admissions = self.row_admissions.saturating_add(1);
        if self.top == self.rows.len() {
            self.rows.push(value);
            self.touched.push(self.interval);
            self.partially_captured.push(0);
            self.source_owner_captured.push(0);
        } else if self.recording
            && (self.touched[self.top] != self.interval
                || self.partially_captured[self.top] == self.interval)
        {
            self.displaced_rows.warm_first_page();
            let old = std::mem::replace(&mut self.rows[self.top], value);
            let payload = self.displaced_rows.insert(old);
            self.undo.append(InputUndo::Replacement {
                index: u32::try_from(self.top).expect("input row index fits u32"),
                payload,
            });
            self.touched[self.top] = self.interval;
            self.partially_captured[self.top] = 0;
            self.source_owner_captured[self.top] = 0;
        } else {
            let old = std::mem::replace(&mut self.rows[self.top], value);
            if let InputLevel::Source(source) = old {
                self.source_slots.release(source.slot.0);
            }
            self.touched[self.top] = self.interval;
            self.partially_captured[self.top] = 0;
            self.source_owner_captured[self.top] = 0;
        }
        self.top += 1;
    }

    pub(crate) fn pop_project<R>(
        &mut self,
        project: impl FnOnce(&InputLevel<G>, Option<&SourceSlot<G>>) -> R,
    ) -> Option<R> {
        let index = self.top.checked_sub(1)?;
        let source = match &self.rows[index] {
            InputLevel::Source(source) => Some(self.source_slot(source.slot)),
            _ => None,
        };
        let result = project(&self.rows[index], source);
        self.top = index;
        if !self.recording {
            let retired = self.rows.pop().expect("input top exists");
            if let InputLevel::Source(source) = retired {
                self.source_slots.release(source.slot.0);
            }
            self.touched.pop();
            self.partially_captured.pop();
            self.source_owner_captured.pop();
        }
        Some(result)
    }

    /// Moves the first source-owner inverse in an interval into the same
    /// ordered history as row advances and replacements.
    ///
    /// Later owner transitions of that row can drop their displaced owner:
    /// rollback needs the interval's initial owner and candidate redo obtains
    /// the final owner by swapping that one inverse a second time. A row
    /// admitted during the interval needs no inverse until it is displaced.
    pub(crate) fn mutate_top_source<R>(
        &mut self,
        mutate: impl FnOnce(
            &mut SourceLevel<G>,
            &mut SourceSlot<G>,
        ) -> (SourceLevelExecutionState<G>, R),
    ) -> Option<R> {
        let index = self.top.checked_sub(1)?;
        if !matches!(self.rows[index], InputLevel::Source(_)) {
            return None;
        }
        Some(self.mutate_source(index, mutate))
    }

    fn mutate_source<R>(
        &mut self,
        index: usize,
        mutate: impl FnOnce(
            &mut SourceLevel<G>,
            &mut SourceSlot<G>,
        ) -> (SourceLevelExecutionState<G>, R),
    ) -> R {
        let key = match &self.rows[index] {
            InputLevel::Source(source) => source.slot,
            _ => unreachable!("source mutation names a source row"),
        };
        if !self.recording {
            let (rows, slots) = (&mut self.rows, &mut self.source_slots);
            let InputLevel::Source(source) = &mut rows[index] else {
                unreachable!()
            };
            let slot = slots.value_mut(key.0).expect("source slot remains live");
            let (state, result) = mutate(source, slot);
            drop(state);
            return result;
        }
        let row_needs_inverse = self.touched[index] != self.interval
            || (self.partially_captured[index] == self.interval
                && self.source_owner_captured[index] != self.interval);
        if !row_needs_inverse {
            let (rows, slots) = (&mut self.rows, &mut self.source_slots);
            let InputLevel::Source(source) = &mut rows[index] else {
                unreachable!()
            };
            let slot = slots.value_mut(key.0).expect("source slot remains live");
            let (state, result) = mutate(source, slot);
            drop(state);
            self.coalesced_mutations = self.coalesced_mutations.saturating_add(1);
            return result;
        }
        self.source_owner_states.warm_first_page();
        let (rows, slots) = (&mut self.rows, &mut self.source_slots);
        let InputLevel::Source(source) = &mut rows[index] else {
            unreachable!()
        };
        let slot = slots.value_mut(key.0).expect("source slot remains live");
        let (state, result) = mutate(source, slot);
        let payload = self.source_owner_states.insert(state);
        self.undo.append(InputUndo::SourceOwner {
            index: u32::try_from(index).expect("input row index fits u32"),
            payload,
            generation: core::marker::PhantomData,
        });
        self.touched[index] = self.interval;
        self.partially_captured[index] = self.interval;
        self.source_owner_captured[index] = self.interval;
        self.source_owner_swaps = self.source_owner_swaps.saturating_add(1);
        result
    }

    pub(crate) fn mark(&mut self) -> Option<InputStackMark> {
        self.recording = true;
        self.undo.warm_first_page();
        self.displaced_rows.warm_first_page();
        self.source_lex_states.warm_first_page();
        self.source_owner_states.warm_first_page();
        let mark = InputStackMark {
            top: u32::try_from(self.top).ok()?,
            undo: self.undo.mark(),
        };
        self.next_interval();
        Some(mark)
    }

    pub(crate) fn validates(&self, mark: InputStackMark) -> bool {
        mark.top as usize <= self.rows.len() && self.undo.validates(mark.undo)
    }

    pub(crate) fn restore(&mut self, mark: InputStackMark) -> bool {
        if self.fork.is_some() || !self.validates(mark) {
            return false;
        }
        let (rows, displaced, source_lex, source_owners, source_slots) = (
            &mut self.rows,
            &mut self.displaced_rows,
            &mut self.source_lex_states,
            &mut self.source_owner_states,
            &mut self.source_slots,
        );
        let restored = self.undo.restore_with(
            mark.undo,
            &mut (rows, displaced, source_lex, source_owners, source_slots),
            |inverse, state| inverse.swap(state),
            |inverse, (_, displaced, source_lex, source_owners, source_slots)| {
                inverse.release(displaced, source_lex, source_owners, source_slots);
            },
        );
        if restored {
            self.top = mark.top as usize;
            self.next_interval();
        }
        restored
    }

    pub(crate) fn release_prefix(&mut self, mark: InputStackMark) -> Option<usize> {
        if self.fork.is_some() || !self.validates(mark) {
            return None;
        }
        let (displaced, source_lex, source_owners, source_slots) = (
            &mut self.displaced_rows,
            &mut self.source_lex_states,
            &mut self.source_owner_states,
            &mut self.source_slots,
        );
        self.undo.release_prefix(mark.undo, |inverse| {
            inverse.release(displaced, source_lex, source_owners, source_slots);
        })
    }

    pub(crate) fn begin_checkpoint_candidate(&mut self, mark: InputStackMark) {
        assert!(
            self.fork.is_none(),
            "input stack already owns a candidate fork"
        );
        assert!(
            self.validates(mark),
            "input candidate mark was prevalidated"
        );
        let accepted_top = self.top;
        let (rows, displaced, source_lex, source_owners, source_slots) = (
            &mut self.rows,
            &mut self.displaced_rows,
            &mut self.source_lex_states,
            &mut self.source_owner_states,
            &mut self.source_slots,
        );
        self.undo.begin_checkpoint_candidate(mark.undo, |inverse| {
            inverse.swap(&mut (rows, displaced, source_lex, source_owners, source_slots));
        });
        self.top = mark.top as usize;
        self.fork = Some(InputStackFork { accepted_top });
        self.next_interval();
    }

    pub(crate) fn reject_checkpoint_candidate(&mut self) {
        let fork = self
            .fork
            .take()
            .expect("input rejection requires a candidate fork");
        let (rows, displaced, source_lex, source_owners, source_slots) = (
            &mut self.rows,
            &mut self.displaced_rows,
            &mut self.source_lex_states,
            &mut self.source_owner_states,
            &mut self.source_slots,
        );
        self.undo.reject_checkpoint_candidate_with(
            &mut (rows, displaced, source_lex, source_owners, source_slots),
            |inverse, state| inverse.swap(state),
            |inverse, (_, displaced, source_lex, source_owners, source_slots)| {
                inverse.release(displaced, source_lex, source_owners, source_slots);
            },
        );
        self.top = fork.accepted_top;
        self.next_interval();
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self) {
        self.fork
            .take()
            .expect("input acceptance requires a candidate fork");
        self.undo.accept_checkpoint_candidate_with(
            &mut (
                &mut self.displaced_rows,
                &mut self.source_lex_states,
                &mut self.source_owner_states,
                &mut self.source_slots,
            ),
            |inverse, (displaced, source_lex, source_owners, source_slots)| {
                inverse.release(displaced, source_lex, source_owners, source_slots);
            },
        );
        self.next_interval();
    }

    pub(crate) fn as_slice(&self) -> &[InputLevel<G>] {
        &self.rows[..self.top]
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.rows
                    .capacity()
                    .saturating_mul(std::mem::size_of::<InputLevel<G>>()),
            )
            .saturating_add(
                self.touched
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
            .saturating_add(
                self.partially_captured
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
            .saturating_add(
                self.source_owner_captured
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
            .saturating_add(self.undo.retained_bytes())
            .saturating_add(self.displaced_rows.retained_bytes())
            .saturating_add(self.source_lex_states.retained_bytes())
            .saturating_add(self.source_owner_states.retained_bytes())
            .saturating_add(self.source_slots.retained_bytes())
    }

    pub(crate) fn counters(&self) -> crate::timeline::LogicalStackCounters {
        let undo = self.undo.counters();
        crate::timeline::LogicalStackCounters {
            payload_admissions: self.row_admissions,
            full_payload_history_clones: 0,
            undo_records: undo.records,
            undo_record_bytes: undo.record_bytes,
            coalesced_mutations: self.coalesced_mutations,
            displaced_payloads: self.displaced_rows.live,
            displaced_reuses: self.displaced_rows.reuses,
            stored_state_captures: self.source_lex_captures,
            owner_swaps: self.source_owner_swaps,
            selected_rewind_records: undo.selected_rewind_records,
            candidate_reject_records: undo.candidate_reject_records,
            accepted_redo_records: undo.accepted_redo_records,
            candidate_chunks_released: undo.candidate_chunks_released,
            accepted_chunks_released: undo.accepted_chunks_released,
        }
    }

    fn record(&mut self, index: usize) {
        if !self.recording || self.touched[index] == self.interval {
            if self.recording {
                self.coalesced_mutations = self.coalesced_mutations.saturating_add(1);
            }
            return;
        }
        self.touched[index] = self.interval;
        self.partially_captured[index] = self.interval;
        let index = u32::try_from(index).expect("input row index fits u32");
        match &self.rows[index as usize] {
            InputLevel::Tokens(tokens) => {
                let state = InputLevelInlineState::new(tokens.frame, tokens.retirement);
                self.undo.append(InputUndo::Inline { index, state });
            }
            InputLevel::MacroArgument(argument) => {
                let state =
                    InputLevelInlineState::new(argument.frame, super::RetirementBehavior::Pop);
                self.undo.append(InputUndo::Inline { index, state });
            }
            InputLevel::Source(source) => {
                self.source_lex_captures = self.source_lex_captures.saturating_add(1);
                let slot = self.source_slot(source.slot);
                let state = SourceLexExecutionState::capture(source, slot);
                let payload = self.source_lex_states.insert(state);
                self.undo.append(InputUndo::SourceLex { index, payload });
            }
        }
    }

    fn next_interval(&mut self) {
        self.interval = self.interval.wrapping_add(1).max(1);
        if self.interval == 1 {
            self.touched.fill(0);
            self.partially_captured.fill(0);
            self.source_owner_captured.fill(0);
        }
    }
}

type InputUndoState<'a, G> = (
    &'a mut Vec<InputLevel<G>>,
    &'a mut PayloadSlab<InputLevel<G>>,
    &'a mut PayloadSlab<SourceLexExecutionState>,
    &'a mut PayloadSlab<SourceLevelExecutionState<G>>,
    &'a mut PayloadSlab<SourceSlot<G>>,
);

impl<G> InputUndo<G> {
    fn swap(&mut self, state: &mut InputUndoState<'_, G>) {
        let (rows, displaced, source_lex, source_owners, source_slots) = state;
        match self {
            Self::Inline { index, state } => {
                rows[*index as usize].swap_input_inline_state(state);
            }
            Self::SourceLex { index, payload } => {
                let state = source_lex
                    .value_mut(*payload)
                    .expect("input source-lexer inverse remains live");
                let InputLevel::Source(source) = &mut rows[*index as usize] else {
                    unreachable!("source lexer inverse names a source row")
                };
                let slot = source_slots
                    .value_mut(source.slot.0)
                    .expect("source lexer inverse names a live source slot");
                source.swap_lex_state(slot, state);
            }
            Self::SourceOwner { index, payload, .. }
            | Self::PhysicalBacking { index, payload, .. } => {
                let state = source_owners
                    .value_mut(*payload)
                    .expect("input source-owner inverse remains live");
                let InputLevel::Source(source) = &mut rows[*index as usize] else {
                    unreachable!("source owner inverse names a source row")
                };
                let slot = source_slots
                    .value_mut(source.slot.0)
                    .expect("source owner inverse names a live source slot");
                source.swap_execution_state(slot, state);
            }
            Self::Replacement { index, payload } => {
                displaced.swap(*payload, &mut rows[*index as usize]);
            }
        }
    }

    fn release(
        self,
        displaced: &mut PayloadSlab<InputLevel<G>>,
        source_lex: &mut PayloadSlab<SourceLexExecutionState>,
        source_owners: &mut PayloadSlab<SourceLevelExecutionState<G>>,
        source_slots: &mut PayloadSlab<SourceSlot<G>>,
    ) {
        match self {
            Self::Replacement { payload, .. } => {
                if let Some(InputLevel::Source(source)) = displaced.value(payload) {
                    let key = source.slot;
                    displaced.release(payload);
                    source_slots.release(key.0);
                } else {
                    displaced.release(payload);
                }
            }
            Self::SourceLex { payload, .. } => source_lex.release(payload),
            Self::SourceOwner { payload, .. } | Self::PhysicalBacking { payload, .. } => {
                source_owners.release(payload)
            }
            Self::Inline { .. } => {}
        }
    }
}
