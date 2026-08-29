//! Generation-owned authoritative input rows and ordered inverse history.

use core::hash::{Hash, Hasher};
use core::ops::{Deref, Index, IndexMut};
use core::slice::SliceIndex;

use crate::scalar_journal::{PackedJournal, PackedJournalMark};
use crate::timeline::{PayloadHandle, PayloadSlab};

use super::{
    InputCapturedState, InputLevel, InputLevelInlineState, SourceLevelExecutionState,
    SourceLexExecutionState,
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
    fork: Option<InputStackFork>,
    recording: bool,
    interval: u64,
    touched: Vec<u64>,
    partially_captured: Vec<u64>,
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
            fork: None,
            recording: false,
            interval: 1,
            touched: Vec::new(),
            partially_captured: Vec::new(),
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
        self.as_slice() == other.as_slice()
    }
}

impl<G: Eq> Eq for InputStack<G> {}

impl<G: Hash> Hash for InputStack<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
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

impl<G> IndexMut<usize> for InputStack<G> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.record(index);
        &mut self.rows[index]
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
    pub(crate) fn push(&mut self, value: InputLevel<G>) {
        self.row_admissions = self.row_admissions.saturating_add(1);
        if self.top == self.rows.len() {
            self.rows.push(value);
            self.touched.push(self.interval);
            self.partially_captured.push(0);
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
        } else {
            self.rows[self.top] = value;
            self.touched[self.top] = self.interval;
            self.partially_captured[self.top] = 0;
        }
        self.top += 1;
    }

    pub(crate) fn pop_project<R>(
        &mut self,
        project: impl FnOnce(&InputLevel<G>) -> R,
    ) -> Option<R> {
        let index = self.top.checked_sub(1)?;
        let result = project(&self.rows[index]);
        self.top = index;
        if !self.recording {
            drop(self.rows.pop());
            self.touched.pop();
            self.partially_captured.pop();
        }
        Some(result)
    }

    pub(crate) fn pop_owned(&mut self) -> Option<InputLevel<G>> {
        if self.recording || self.top == 0 || self.top != self.rows.len() {
            return None;
        }
        self.top -= 1;
        self.touched.pop();
        self.partially_captured.pop();
        self.rows.pop()
    }

    pub(crate) const fn records_history(&self) -> bool {
        self.recording
    }

    pub(crate) fn last_mut(&mut self) -> Option<&mut InputLevel<G>> {
        let index = self.top.checked_sub(1)?;
        self.record(index);
        self.rows.get_mut(index)
    }

    /// Moves one source-owner inverse into the same ordered history as row
    /// advances and replacements.
    pub(crate) fn mutate_top_source<R>(
        &mut self,
        mutate: impl FnOnce(&mut InputLevel<G>) -> (SourceLevelExecutionState<G>, R),
    ) -> Option<R> {
        let index = self.top.checked_sub(1)?;
        if !self.recording {
            let (state, result) = mutate(&mut self.rows[index]);
            drop(state);
            return Some(result);
        }
        self.source_owner_states.warm_first_page();
        let (state, result) = mutate(&mut self.rows[index]);
        let payload = self.source_owner_states.insert(state);
        self.undo.append(InputUndo::SourceOwner {
            index: u32::try_from(index).expect("input row index fits u32"),
            payload,
            generation: core::marker::PhantomData,
        });
        self.touched[index] = self.interval;
        self.partially_captured[index] = self.interval;
        self.source_owner_swaps = self.source_owner_swaps.saturating_add(1);
        Some(result)
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
        let (rows, displaced, source_lex, source_owners) = (
            &mut self.rows,
            &mut self.displaced_rows,
            &mut self.source_lex_states,
            &mut self.source_owner_states,
        );
        let restored = self.undo.restore_with(
            mark.undo,
            &mut (rows, displaced, source_lex, source_owners),
            |inverse, state| inverse.swap(state),
            |inverse, (_, displaced, source_lex, source_owners)| {
                inverse.release(displaced, source_lex, source_owners);
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
        let (displaced, source_lex, source_owners) = (
            &mut self.displaced_rows,
            &mut self.source_lex_states,
            &mut self.source_owner_states,
        );
        self.undo.release_prefix(mark.undo, |inverse| {
            inverse.release(displaced, source_lex, source_owners);
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
        let (rows, displaced, source_lex, source_owners) = (
            &mut self.rows,
            &mut self.displaced_rows,
            &mut self.source_lex_states,
            &mut self.source_owner_states,
        );
        self.undo.begin_checkpoint_candidate(mark.undo, |inverse| {
            inverse.swap(&mut (rows, displaced, source_lex, source_owners));
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
        let (rows, displaced, source_lex, source_owners) = (
            &mut self.rows,
            &mut self.displaced_rows,
            &mut self.source_lex_states,
            &mut self.source_owner_states,
        );
        self.undo.reject_checkpoint_candidate_with(
            &mut (rows, displaced, source_lex, source_owners),
            |inverse, state| inverse.swap(state),
            |inverse, (_, displaced, source_lex, source_owners)| {
                inverse.release(displaced, source_lex, source_owners);
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
            ),
            |inverse, (displaced, source_lex, source_owners)| {
                inverse.release(displaced, source_lex, source_owners);
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
            .saturating_add(self.undo.retained_bytes())
            .saturating_add(self.displaced_rows.retained_bytes())
            .saturating_add(self.source_lex_states.retained_bytes())
            .saturating_add(self.source_owner_states.retained_bytes())
    }

    #[cfg(any(test, feature = "profiling"))]
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
        match self.rows[index as usize].capture_input_state() {
            InputCapturedState::Inline(state) => {
                self.undo.append(InputUndo::Inline { index, state });
            }
            InputCapturedState::SourceLex(state) => {
                self.source_lex_captures = self.source_lex_captures.saturating_add(1);
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
        }
    }
}

type InputUndoState<'a, G> = (
    &'a mut Vec<InputLevel<G>>,
    &'a mut PayloadSlab<InputLevel<G>>,
    &'a mut PayloadSlab<SourceLexExecutionState>,
    &'a mut PayloadSlab<SourceLevelExecutionState<G>>,
);

impl<G> InputUndo<G> {
    fn swap(&mut self, state: &mut InputUndoState<'_, G>) {
        let (rows, displaced, source_lex, source_owners) = state;
        match self {
            Self::Inline { index, state } => {
                rows[*index as usize].swap_input_inline_state(state);
            }
            Self::SourceLex { index, payload } => {
                let state = source_lex
                    .value_mut(*payload)
                    .expect("input source-lexer inverse remains live");
                rows[*index as usize].swap_source_lex_state(state);
            }
            Self::SourceOwner { index, payload, .. } => {
                let state = source_owners
                    .value_mut(*payload)
                    .expect("input source-owner inverse remains live");
                rows[*index as usize].swap_source_execution_state(state);
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
    ) {
        match self {
            Self::Replacement { payload, .. } => displaced.release(payload),
            Self::SourceLex { payload, .. } => source_lex.release(payload),
            Self::SourceOwner { payload, .. } => source_owners.release(payload),
            Self::Inline { .. } => {}
        }
    }
}
