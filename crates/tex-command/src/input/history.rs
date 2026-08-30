//! Generation-owned authoritative input rows and ordered inverse history.

#[cfg(test)]
mod tests;

use core::hash::{Hash, Hasher};
use core::ops::{Deref, Index};
use core::slice::SliceIndex;

use crate::scalar_journal::{PackedJournal, PackedJournalMark};
use crate::timeline::{PayloadHandle, PayloadSlab};

use super::{
    CompactSourceTokenizationStep, InputLevel, InputLevelInlineState, SourceLevel,
    SourceLevelExecutionState, SourceLexExecutionState, SourceSlot, SourceSlotKey,
};

const INPUT_UNDO_RECORDS_PER_CHUNK: usize = 16;

/// Profiling-only proof that active-source delivery is independent of input
/// depth. Shipping builds contain neither the counters nor their updates.
#[cfg(any(test, feature = "profiling"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InputSourceContextCounters {
    pub(crate) top_reads: u64,
    pub(crate) ancestry_rows: u64,
    pub(crate) owner_slot_lookups: u64,
    pub(crate) source_lex_slot_borrows: u64,
}

/// Profiling proof for the unified resident-input cursor mutation boundary.
///
/// One call performs one typed top-row access. The first call in a checkpoint
/// interval appends the row's matching inverse; later calls coalesce against
/// it. The resident transition has no callback dispatch to increment.
#[cfg(any(test, feature = "profiling"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InputCursorMutationCounters {
    pub(crate) typed_top_accesses: u64,
    pub(crate) first_touch_transitions: u64,
    pub(crate) coalesced_transitions: u64,
    pub(crate) closure_dispatches: u64,
}

struct InlineCursorRecorder<'a, G> {
    recording: bool,
    interval: u64,
    index: usize,
    touched: &'a mut [u64],
    partially_captured: &'a mut [u64],
    undo: &'a mut PackedJournal<InputUndo<G>, INPUT_UNDO_RECORDS_PER_CHUNK>,
    coalesced_mutations: &'a mut u64,
    #[cfg(any(test, feature = "profiling"))]
    counters: &'a mut InputCursorMutationCounters,
}

impl<G> InlineCursorRecorder<'_, G> {
    #[inline(always)]
    fn record(&mut self, state: InputLevelInlineState) {
        if !self.recording {
            return;
        }
        if self.touched[self.index] == self.interval {
            *self.coalesced_mutations = self.coalesced_mutations.saturating_add(1);
            #[cfg(any(test, feature = "profiling"))]
            {
                self.counters.coalesced_transitions =
                    self.counters.coalesced_transitions.saturating_add(1);
            }
            return;
        }
        self.touched[self.index] = self.interval;
        self.partially_captured[self.index] = self.interval;
        self.undo.append(InputUndo::Inline {
            index: u32::try_from(self.index).expect("input row index fits u32"),
            state,
        });
        #[cfg(any(test, feature = "profiling"))]
        {
            self.counters.first_touch_transitions =
                self.counters.first_touch_transitions.saturating_add(1);
        }
    }
}

#[cfg(any(test, feature = "profiling"))]
thread_local! {
    static INPUT_SOURCE_CONTEXT_COUNTERS: core::cell::Cell<InputSourceContextCounters> =
        const { core::cell::Cell::new(InputSourceContextCounters {
            top_reads: 0,
            ancestry_rows: 0,
            owner_slot_lookups: 0,
            source_lex_slot_borrows: 0,
        }) };
}

#[cfg(any(test, feature = "profiling"))]
pub(crate) fn input_source_context_counters() -> InputSourceContextCounters {
    INPUT_SOURCE_CONTEXT_COUNTERS.with(core::cell::Cell::get)
}

#[cfg(any(test, feature = "profiling"))]
pub(crate) fn reset_input_source_context_counters() {
    INPUT_SOURCE_CONTEXT_COUNTERS
        .with(|counters| counters.set(InputSourceContextCounters::default()));
}

#[inline(always)]
fn record_source_context_read() {
    #[cfg(any(test, feature = "profiling"))]
    INPUT_SOURCE_CONTEXT_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        counters.top_reads = counters.top_reads.saturating_add(1);
        slot.set(counters);
    });
}

#[inline(always)]
fn record_source_lex_slot_borrow() {
    #[cfg(any(test, feature = "profiling"))]
    INPUT_SOURCE_CONTEXT_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        counters.source_lex_slot_borrows = counters.source_lex_slot_borrows.saturating_add(1);
        slot.set(counters);
    });
}

#[inline(always)]
fn record_source_owner_slot_lookup() {
    #[cfg(any(test, feature = "profiling"))]
    INPUT_SOURCE_CONTEXT_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        counters.owner_slot_lookups = counters.owner_slot_lookups.saturating_add(1);
        slot.set(counters);
    });
}

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
    /// One active external-source scalar changed by editor-root rebinding.
    /// This remains separate from ordinary cursor first-touch capture so a
    /// later delivery still records its initial execution position.
    SourceContext {
        index: u32,
        source: Option<tex_state::SourceId>,
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
    pub(crate) occupied_source_buffer_slots: usize,
}

#[derive(Debug)]
struct InputStackFork {
    accepted_top: usize,
    accepted_occupied_source_buffer_slots: usize,
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
    occupied_source_buffer_slots: usize,
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
    #[cfg(any(test, feature = "profiling"))]
    cursor_mutations: InputCursorMutationCounters,
    /// Monotonic runtime incarnation of the live diagnostic projection.
    /// Compact publication coordinates validate this before reading rows.
    context_revision: u64,
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
            occupied_source_buffer_slots: 0,
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
            #[cfg(any(test, feature = "profiling"))]
            cursor_mutations: InputCursorMutationCounters::default(),
            context_revision: 1,
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
    fn note_context_mutation(&mut self) {
        self.context_revision = self.context_revision.wrapping_add(1).max(1);
    }

    pub(crate) const fn context_revision(&self) -> u64 {
        self.context_revision
    }

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
            slot.occupied_buffer_slots = super::source::occupied_source_buffer_slots(&slot.cursor);
            root_slot = Some(SourceSlotKey(handle));
            rebound = true;
        });
        if rebound {
            self.note_context_mutation();
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
            self.refresh_occupied_source_buffer_slots();
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
        record_source_owner_slot_lookup();
        self.source_slots
            .value(key.0)
            .expect("source row names its live ABA-checked slot")
    }

    pub(crate) fn source_level_slot(&self, source: &SourceLevel<G>) -> &SourceSlot<G> {
        self.source_slot(source.slot)
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
        let prior_buffer_slots = slot.occupied_buffer_slots;
        let replacement_id = replacement.id;
        let state = SourceLevelExecutionState::physical_backing(source, slot, replacement);
        let current_buffer_slots = super::source::occupied_source_buffer_slots(&slot.cursor);
        slot.occupied_buffer_slots = current_buffer_slots;
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
        for index in 0..self.top {
            if self.rows[index].source_context() != Some(id) {
                continue;
            }
            if self.recording {
                self.undo.append(InputUndo::SourceContext {
                    index: u32::try_from(index).expect("input row index fits u32"),
                    source: Some(id),
                });
            }
            self.rows[index].set_source_context(Some(replacement_id));
        }
        self.note_context_mutation();
        self.replace_source_buffer_slots(prior_buffer_slots, current_buffer_slots);
        true
    }

    /// External file or `\scantokens` context of the semantic top.
    ///
    /// Every admitted row carries this compact immutable execution fact in
    /// its common frame, so the query reads one row and never walks input
    /// ancestry or validates a source-owner slot.
    #[inline(always)]
    pub(crate) fn current_source_context(&self) -> Option<tex_state::SourceId> {
        record_source_context_read();
        self.rows
            .get(self.top.checked_sub(1)?)
            .and_then(InputLevel::source_context)
    }

    /// Mutates only copy-small lexer execution state.
    ///
    /// The closure must not replace the loaded line or its backing, retained
    /// end, or normalized endline. Those cold owner changes must use
    /// [`Self::mutate_top_source`] so buffer occupancy is updated once.
    pub(crate) fn mutate_top_source_lex<R>(
        &mut self,
        mutate: impl FnOnce(&mut SourceLevel<G>, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        let index = self.top.checked_sub(1)?;
        let key = match &self.rows[index] {
            InputLevel::Source(source) => source.slot,
            _ => return None,
        };
        let needs_inverse = self.recording && self.touched[index] != self.interval;
        if self.recording && !needs_inverse {
            self.coalesced_mutations = self.coalesced_mutations.saturating_add(1);
        }
        let (rows, slots, states, undo) = (
            &mut self.rows,
            &mut self.source_slots,
            &mut self.source_lex_states,
            &mut self.undo,
        );
        let InputLevel::Source(source) = &mut rows[index] else {
            unreachable!()
        };
        let slot = slots
            .value_mut(key.0)
            .expect("source row names its live ABA-checked slot");
        record_source_lex_slot_borrow();
        if needs_inverse {
            self.source_lex_captures = self.source_lex_captures.saturating_add(1);
            let payload = states.insert(SourceLexExecutionState::capture(source, slot));
            undo.append(InputUndo::SourceLex {
                index: u32::try_from(index).expect("input row index fits u32"),
                payload,
            });
            self.touched[index] = self.interval;
            self.partially_captured[index] = self.interval;
        }
        let result = mutate(source, slot);
        self.note_context_mutation();
        Some(result)
    }
}

impl<G> crate::CommandState<G> {
    /// Delivers and advances the exact resident input cursor at the semantic top.
    ///
    /// Source, token-list, and direct macro-argument rows enter this one typed
    /// path. It indexes and discriminates the top once, performs the matching
    /// ordered first-touch transition directly, and writes an ordinary word
    /// into the caller-owned command before the resident borrow ends. Cold
    /// source, exhaustion, and parameter statuses carry no command borrow.
    pub(crate) fn advance_resident_command_into(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        fuel: &mut crate::fuel::CommandFuel,
        create_control_sequences: bool,
        destination: crate::command::EmptyCommand<'_, G>,
        sequence: u64,
    ) -> Result<super::ResidentCommandTransition, ()> {
        let profile = self.profile();
        let force_eof = self.source_force_eof();
        let scanner_active = !matches!(
            self.roots.scanner.status(),
            crate::processor::ScannerStatus::Normal
        );
        let attempt = self.attempt.arena();
        let scratch = &self.scratch;
        let roots = &mut self.roots;
        let timeline = &mut self.timeline;
        let stack_usage = &mut self.stack_usage;
        #[cfg(test)]
        let path_counters = &mut self.raw_delivery_path_counters;
        let input = &mut roots.input;
        let sources = super::PackedTokenSources::new(&input.replay, attempt);
        let levels = &mut input.levels;

        let Some(index) = levels.top.checked_sub(1) else {
            return Ok(super::ResidentCommandTransition::Empty);
        };
        #[cfg(test)]
        {
            path_counters.resident_transitions =
                path_counters.resident_transitions.saturating_add(1);
        }
        #[cfg(any(test, feature = "profiling"))]
        {
            levels.cursor_mutations.typed_top_accesses =
                levels.cursor_mutations.typed_top_accesses.saturating_add(1);
        }
        let delivery = match &mut levels.rows[index] {
            InputLevel::Source(source) => {
                // This authoritative resident row prevents its slot from
                // being released or reused until the row itself retires. ABA
                // validation belongs to cold history coordinates, not every
                // ordinary token delivered through the live row.
                let slot = levels.source_slots.resident_value_mut(source.slot.0.slot);
                record_source_lex_slot_borrow();
                if state.tracked_region_is_active() {
                    super::observe_immutable_source(state, source, slot);
                }
                let needs_inverse = levels.recording && levels.touched[index] != levels.interval;
                if levels.recording && !needs_inverse {
                    levels.coalesced_mutations = levels.coalesced_mutations.saturating_add(1);
                    #[cfg(any(test, feature = "profiling"))]
                    {
                        levels.cursor_mutations.coalesced_transitions = levels
                            .cursor_mutations
                            .coalesced_transitions
                            .saturating_add(1);
                    }
                }
                if needs_inverse {
                    levels.source_lex_captures = levels.source_lex_captures.saturating_add(1);
                    let payload = levels
                        .source_lex_states
                        .insert(SourceLexExecutionState::capture(source, slot));
                    levels.undo.append(InputUndo::SourceLex {
                        index: u32::try_from(index).expect("input row index fits u32"),
                        payload,
                    });
                    levels.touched[index] = levels.interval;
                    levels.partially_captured[index] = levels.interval;
                    #[cfg(any(test, feature = "profiling"))]
                    {
                        levels.cursor_mutations.first_touch_transitions = levels
                            .cursor_mutations
                            .first_touch_transitions
                            .saturating_add(1);
                    }
                }
                let identity = source.identity();
                let position = slot.cursor.next_physical_offset;
                let active_source = source.frame.source_id();
                let step = {
                    let mut queries = super::stack::LiveSourceQueries {
                        state,
                        create_control_sequences,
                    };
                    match profile.character_mode() {
                        crate::CharacterMode::EightBitExact => slot
                            .cursor
                            .next_compact_exact_byte_step(force_eof, &mut queries),
                        crate::CharacterMode::UnicodeExtended => slot
                            .cursor
                            .next_compact_unicode_step(force_eof, &mut queries),
                    }
                };
                let direct_source_line = slot
                    .cursor
                    .line
                    .as_ref()
                    .map(|line| u32::try_from(line.physical.number()).unwrap_or(u32::MAX));
                match step {
                    CompactSourceTokenizationStep::Token(token) => {
                        if source.frame.identity() != identity.0 || source.frame.advance().is_none()
                        {
                            Err(())
                        } else {
                            let range = token.provenance.range();
                            let origin = if range.end().saturating_sub(range.start()) == 1 {
                                state.source_token_origin(
                                    range.source(),
                                    range.start(),
                                    range.end(),
                                )
                            } else {
                                state.source_range_origin(
                                    range.source(),
                                    range.start(),
                                    range.end(),
                                )
                            };
                            let (resolved, resolution) = destination.write_resolved_delivery(
                                token.word,
                                origin,
                                identity.0,
                                position,
                                sequence,
                                Some(token.provenance),
                                active_source,
                                true,
                                direct_source_line,
                                false,
                                state,
                            );
                            #[cfg(test)]
                            {
                                path_counters.source_direct =
                                    path_counters.source_direct.saturating_add(1);
                            }
                            Ok(super::InputTopTransition::Delivered {
                                resolved,
                                resolution,
                            })
                        }
                    }
                    CompactSourceTokenizationStep::InvalidCharacter => {
                        Ok(super::InputTopTransition::InvalidCharacter)
                    }
                    CompactSourceTokenizationStep::NeedLine => {
                        Ok(super::InputTopTransition::NeedLine(identity))
                    }
                    CompactSourceTokenizationStep::End => {
                        Ok(super::InputTopTransition::SourceExhausted(identity))
                    }
                }
            }
            InputLevel::Tokens(cursor) => {
                let mut recorder = InlineCursorRecorder {
                    recording: levels.recording,
                    interval: levels.interval,
                    index,
                    touched: &mut levels.touched,
                    partially_captured: &mut levels.partially_captured,
                    undo: &mut levels.undo,
                    coalesced_mutations: &mut levels.coalesced_mutations,
                    #[cfg(any(test, feature = "profiling"))]
                    counters: &mut levels.cursor_mutations,
                };
                recorder.record(InputLevelInlineState::new(cursor.frame, cursor.retirement));
                let delivery = cursor.deliver_into(sources, destination, sequence, state);
                #[cfg(test)]
                match &delivery {
                    Ok(super::InputTopTransition::Delivered { .. }) => {
                        path_counters.stored_direct = path_counters.stored_direct.saturating_add(1);
                    }
                    Ok(super::InputTopTransition::OutParameter { .. }) => {
                        path_counters.out_parameter_interceptions =
                            path_counters.out_parameter_interceptions.saturating_add(1);
                    }
                    Ok(_) | Err(()) => {}
                }
                delivery
            }
            InputLevel::MacroArgument(cursor) => {
                let mut recorder = InlineCursorRecorder {
                    recording: levels.recording,
                    interval: levels.interval,
                    index,
                    touched: &mut levels.touched,
                    partially_captured: &mut levels.partially_captured,
                    undo: &mut levels.undo,
                    coalesced_mutations: &mut levels.coalesced_mutations,
                    #[cfg(any(test, feature = "profiling"))]
                    counters: &mut levels.cursor_mutations,
                };
                recorder.record(InputLevelInlineState::new(
                    cursor.frame,
                    super::RetirementBehavior::Pop,
                ));
                let delivery = cursor.deliver_into(scratch, destination, sequence, state);
                #[cfg(test)]
                if matches!(&delivery, Ok(super::InputTopTransition::Delivered { .. })) {
                    path_counters.macro_argument_direct =
                        path_counters.macro_argument_direct.saturating_add(1);
                }
                delivery
            }
        };
        levels.note_context_mutation();
        match delivery? {
            super::InputTopTransition::Delivered {
                mut resolved,
                resolution,
            } => {
                if resolved.as_ref().suppresses_expandable_control_sequence() {
                    resolved.as_mut().suppress_expandable();
                }
                fuel.record_raw_delivery(scanner_active, resolution.meaning_lookup());
                let interception = if resolved.as_ref().is_outer() && scanner_active {
                    super::ResidentCommandInterception::Outer
                } else {
                    roots.alignment.classify_delivery(
                        timeline,
                        resolved.as_mut(),
                        resolution.literal_catcode(),
                    );
                    super::ResidentCommandInterception::Ready
                };
                Ok(super::ResidentCommandTransition::Delivered { interception })
            }
            super::InputTopTransition::OutParameter {
                slot,
                has_macro_lineage,
                active_source,
            } => {
                if !(1..=9).contains(&slot) || !has_macro_lineage {
                    return Err(());
                }
                let owner = scratch.active_macro_frame().ok_or(())?;
                let range = scratch
                    .argument_range(owner, slot)
                    .map_err(|_| ())?
                    .ok_or(())?;
                let identity = super::InputLevelId(input.next_level_identity);
                timeline.record_next_input_level_identity(input.next_level_identity);
                input.next_level_identity = input.next_level_identity.wrapping_add(1);
                let trace = super::ReplayTrace::MacroParameter { slot };
                let mut frame = super::packed_token_frame(
                    identity,
                    range.len() as usize,
                    &super::TokenBehavior::Parameter,
                    super::RetirementBehavior::Pop,
                    &trace,
                );
                frame.set_source_context(active_source);
                stack_usage.input_stack = stack_usage.input_stack.max(levels.len());
                levels.push(InputLevel::MacroArgument(super::MacroArgumentCursor {
                    range,
                    slot,
                    frame,
                }));
                Ok(super::ResidentCommandTransition::ParameterPushed(identity))
            }
            super::InputTopTransition::InvalidCharacter => {
                Ok(super::ResidentCommandTransition::InvalidCharacter)
            }
            super::InputTopTransition::NeedLine(identity) => {
                Ok(super::ResidentCommandTransition::NeedLine(identity))
            }
            super::InputTopTransition::SourceExhausted(identity) => {
                Ok(super::ResidentCommandTransition::SourceExhausted(identity))
            }
            super::InputTopTransition::TokenExhausted(identity) => {
                Ok(super::ResidentCommandTransition::TokenExhausted(identity))
            }
            super::InputTopTransition::Empty => Ok(super::ResidentCommandTransition::Empty),
        }
    }
}

impl<G> InputStack<G> {
    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn set_top_token_retirement(
        &mut self,
        retirement: super::RetirementBehavior,
    ) -> bool {
        let index = match self.top.checked_sub(1) {
            Some(index) => index,
            None => return false,
        };
        let InputLevel::Tokens(cursor) = &self.rows[index] else {
            return false;
        };
        let state = InputLevelInlineState::new(cursor.frame, cursor.retirement);
        self.record_inline(index, state);
        let InputLevel::Tokens(cursor) = &mut self.rows[index] else {
            unreachable!()
        };
        cursor.retirement = retirement;
        self.note_context_mutation();
        true
    }

    pub(crate) fn retain_top_v_template(&mut self) -> bool {
        let index = match self.top.checked_sub(1) {
            Some(index) => index,
            None => return false,
        };
        let InputLevel::Tokens(cursor) = &self.rows[index] else {
            return false;
        };
        let state = InputLevelInlineState::new(cursor.frame, cursor.retirement);
        self.record_inline(index, state);
        let InputLevel::Tokens(cursor) = &mut self.rows[index] else {
            unreachable!()
        };
        cursor.retirement = super::RetirementBehavior::AwaitingVTemplateRetirement;
        cursor
            .frame
            .add_flags(tex_state::packed_input::InputFrameFlags::RETAIN_AT_END);
        self.note_context_mutation();
        true
    }

    pub(crate) fn extend_top_token_limit(&mut self, additional: u32) -> Option<bool> {
        let index = self.top.checked_sub(1)?;
        let InputLevel::Tokens(cursor) = &self.rows[index] else {
            return None;
        };
        let state = InputLevelInlineState::new(cursor.frame, cursor.retirement);
        self.record_inline(index, state);
        let InputLevel::Tokens(cursor) = &mut self.rows[index] else {
            unreachable!()
        };
        let extended = cursor.frame.extend_limit(additional).is_some();
        self.note_context_mutation();
        Some(extended)
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn toggle_top_token_retirement(&mut self) -> bool {
        let Some(current) = self.rows.get(self.top.saturating_sub(1)).and_then(|row| {
            let InputLevel::Tokens(cursor) = row else {
                return None;
            };
            Some(cursor.retirement)
        }) else {
            return false;
        };
        let retirement = match current {
            super::RetirementBehavior::Pop => super::RetirementBehavior::StopAtEnd,
            _ => super::RetirementBehavior::Pop,
        };
        self.set_top_token_retirement(retirement)
    }

    pub(crate) fn push(&mut self, value: InputLevel<G>) {
        assert!(
            !matches!(value, InputLevel::Source(_)),
            "source rows are admitted with their sole owner slot"
        );
        self.push_row(value);
    }

    fn push_row(&mut self, value: InputLevel<G>) {
        self.note_context_mutation();
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
        if let InputLevel::Source(source) = &self.rows[self.top] {
            self.occupied_source_buffer_slots = self
                .occupied_source_buffer_slots
                .saturating_add(self.source_slot(source.slot).occupied_buffer_slots);
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
        if let Some(source) = source {
            self.occupied_source_buffer_slots = self
                .occupied_source_buffer_slots
                .saturating_sub(source.occupied_buffer_slots);
        }
        self.note_context_mutation();
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
        let prior_buffer_slots = self
            .source_slots
            .value(key.0)
            .expect("source slot remains live")
            .occupied_buffer_slots;
        if !self.recording {
            let (rows, slots) = (&mut self.rows, &mut self.source_slots);
            let InputLevel::Source(source) = &mut rows[index] else {
                unreachable!()
            };
            let slot = slots.value_mut(key.0).expect("source slot remains live");
            let (state, result) = mutate(source, slot);
            drop(state);
            let current_buffer_slots = super::source::occupied_source_buffer_slots(&slot.cursor);
            slot.occupied_buffer_slots = current_buffer_slots;
            self.replace_source_buffer_slots(prior_buffer_slots, current_buffer_slots);
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
            let current_buffer_slots = super::source::occupied_source_buffer_slots(&slot.cursor);
            slot.occupied_buffer_slots = current_buffer_slots;
            self.replace_source_buffer_slots(prior_buffer_slots, current_buffer_slots);
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
        let current_buffer_slots = super::source::occupied_source_buffer_slots(&slot.cursor);
        slot.occupied_buffer_slots = current_buffer_slots;
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
        self.note_context_mutation();
        self.replace_source_buffer_slots(prior_buffer_slots, current_buffer_slots);
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
            occupied_source_buffer_slots: self.occupied_source_buffer_slots,
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
            self.note_context_mutation();
            self.top = mark.top as usize;
            self.occupied_source_buffer_slots = mark.occupied_source_buffer_slots;
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
        let accepted_occupied_source_buffer_slots = self.occupied_source_buffer_slots;
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
        self.occupied_source_buffer_slots = mark.occupied_source_buffer_slots;
        self.note_context_mutation();
        self.fork = Some(InputStackFork {
            accepted_top,
            accepted_occupied_source_buffer_slots,
        });
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
        self.occupied_source_buffer_slots = fork.accepted_occupied_source_buffer_slots;
        self.note_context_mutation();
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
        self.note_context_mutation();
    }

    pub(crate) fn as_slice(&self) -> &[InputLevel<G>] {
        &self.rows[..self.top]
    }

    /// Exact occupied TeX source-buffer slots across all live source levels.
    #[inline(always)]
    pub(crate) const fn occupied_source_buffer_slots(&self) -> usize {
        self.occupied_source_buffer_slots
    }

    /// Exact occupied source-buffer slots below the current semantic top.
    #[inline(always)]
    pub(crate) fn occupied_source_buffer_slots_below_top(&self) -> usize {
        let top = self
            .top_source()
            .map_or(0, |(_, slot)| slot.occupied_buffer_slots);
        self.occupied_source_buffer_slots.saturating_sub(top)
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

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) const fn cursor_mutation_counters(&self) -> InputCursorMutationCounters {
        self.cursor_mutations
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn reset_cursor_mutation_counters(&mut self) {
        self.cursor_mutations = InputCursorMutationCounters::default();
    }

    fn record_inline(&mut self, index: usize, state: InputLevelInlineState) {
        if !self.recording || self.touched[index] == self.interval {
            if self.recording {
                self.coalesced_mutations = self.coalesced_mutations.saturating_add(1);
            }
            return;
        }
        self.touched[index] = self.interval;
        self.partially_captured[index] = self.interval;
        self.undo.append(InputUndo::Inline {
            index: u32::try_from(index).expect("input row index fits u32"),
            state,
        });
    }

    fn next_interval(&mut self) {
        self.interval = self.interval.wrapping_add(1).max(1);
        if self.interval == 1 {
            self.touched.fill(0);
            self.partially_captured.fill(0);
            self.source_owner_captured.fill(0);
        }
    }

    fn replace_source_buffer_slots(&mut self, prior: usize, current: usize) {
        self.occupied_source_buffer_slots = self
            .occupied_source_buffer_slots
            .saturating_sub(prior)
            .saturating_add(current);
    }

    fn refresh_occupied_source_buffer_slots(&mut self) {
        self.occupied_source_buffer_slots = self.as_slice().iter().fold(0, |total, level| {
            let InputLevel::Source(source) = level else {
                return total;
            };
            total.saturating_add(self.source_level_slot(source).occupied_buffer_slots)
        });
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
                slot.occupied_buffer_slots =
                    super::source::occupied_source_buffer_slots(&slot.cursor);
            }
            Self::SourceContext { index, source } => {
                let level = &mut rows[*index as usize];
                let mut current = level.source_context();
                level.set_source_context(*source);
                core::mem::swap(source, &mut current);
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
            Self::Inline { .. } | Self::SourceContext { .. } => {}
        }
    }
}
