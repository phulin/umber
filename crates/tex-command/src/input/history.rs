//! Generation-owned authoritative input rows and ordered inverse history.

#[cfg(test)]
mod tests;

use core::hash::{Hash, Hasher};
use core::ops::{Deref, Index};
use core::slice::SliceIndex;

use crate::observation::{
    AlignmentRecord, CommandObservation, CommandObserver, InputReason, InputRecord, InputTransition,
};
use crate::scalar_journal::{PackedJournal, PackedJournalMark};
use crate::timeline::{PayloadHandle, PayloadSlab};

use super::levels::RowRollbackMarker;
use super::{
    CompactSourceTokenizationStep, InputLevel, InputLevelInlineState, SourceLevel,
    SourceLevelExecutionState, SourceLexExecutionState, SourceSlot, SourceSlotKey,
};

const INPUT_UNDO_RECORDS_PER_CHUNK: usize = 16;

const ROW_ROLLBACK_STATE_BITS: u32 = 2;
const MAX_ROW_ROLLBACK_EPOCH: u64 = u64::MAX >> ROW_ROLLBACK_STATE_BITS;

#[repr(u64)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowRollbackState {
    /// The row was admitted or replaced during this epoch and has no inverse.
    Admitted = 0,
    /// Its ordinary inline cursor or source lexer state has an inverse.
    Inline = 1,
    /// Its complete cold token or source-owner state has an inverse.
    Cold = 2,
}

impl RowRollbackMarker {
    #[inline(always)]
    const fn in_epoch(self, epoch: u64) -> bool {
        self.0 >> ROW_ROLLBACK_STATE_BITS == epoch
    }

    #[inline(always)]
    const fn state(self) -> RowRollbackState {
        match self.0 & ((1 << ROW_ROLLBACK_STATE_BITS) - 1) {
            0 => RowRollbackState::Admitted,
            1 => RowRollbackState::Inline,
            2 => RowRollbackState::Cold,
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn set(&mut self, epoch: u64, state: RowRollbackState) {
        debug_assert!((1..=MAX_ROW_ROLLBACK_EPOCH).contains(&epoch));
        self.0 = (epoch << ROW_ROLLBACK_STATE_BITS) | state as u64;
    }

    #[inline(always)]
    const fn needs_replacement(self, epoch: u64) -> bool {
        !self.in_epoch(epoch) || !matches!(self.state(), RowRollbackState::Admitted)
    }

    #[inline(always)]
    const fn needs_source_owner_inverse(self, epoch: u64) -> bool {
        !self.in_epoch(epoch) || matches!(self.state(), RowRollbackState::Inline)
    }

    #[inline(always)]
    const fn cold_captured(self, epoch: u64) -> bool {
        self.in_epoch(epoch) && matches!(self.state(), RowRollbackState::Cold)
    }
}

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
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InputCursorMutationCounters {
    pub(crate) typed_top_accesses: u64,
    pub(crate) source_branch_entries: u64,
    pub(crate) stored_token_branch_entries: u64,
    pub(crate) macro_argument_branch_entries: u64,
    pub(crate) replay_domain_dispatches: u64,
    pub(crate) durable_domain_dispatches: u64,
    pub(crate) attempt_domain_dispatches: u64,
    pub(crate) macro_body_domain_dispatches: u64,
    pub(crate) first_touch_transitions: u64,
    pub(crate) closure_dispatches: u64,
}

enum SourceOwnerTransition {
    Cursor,
    EveryEof,
    Backing(super::RegisteredSource),
}

impl SourceOwnerTransition {
    fn apply<G>(
        self,
        source: &SourceLevel<G>,
        slot: &mut SourceSlot<G>,
        retain_inverse: bool,
    ) -> Option<SourceLevelExecutionState<G>> {
        match (self, retain_inverse) {
            (Self::Cursor, true) => Some(SourceLevelExecutionState::cursor(source, slot)),
            (Self::EveryEof, true) => Some(SourceLevelExecutionState::every_eof(source, slot)),
            (Self::Backing(replacement), true) => Some(SourceLevelExecutionState::backing(
                source,
                slot,
                replacement,
            )),
            (Self::Cursor, false) => {
                slot.cursor.release_execution_owners();
                None
            }
            (Self::EveryEof, false) => {
                slot.cursor.release_execution_owners();
                slot.every_eof = None;
                None
            }
            (Self::Backing(replacement), false) => {
                slot.cursor.backing = replacement;
                slot.cursor.release_execution_owners();
                None
            }
        }
    }
}

struct ResidentSourceTop<'a, G> {
    index: usize,
    source: &'a mut SourceLevel<G>,
    slot: &'a mut SourceSlot<G>,
    recording: bool,
    interval: u64,
    undo: &'a mut PackedJournal<InputUndo<G>, INPUT_UNDO_RECORDS_PER_CHUNK>,
    source_lex_states: &'a mut PayloadSlab<SourceLexExecutionState>,
    source_lex_captures: &'a mut u64,
    #[cfg(test)]
    counters: &'a mut InputCursorMutationCounters,
}

enum ResidentSourceAdvance {
    Delivered(
        tex_state::token::TokenWord,
        tex_state::token::OriginId,
        super::SourceLocation,
    ),
    InvalidCharacter,
    NeedLine(super::InputLevelId),
    Exhausted(super::InputLevelId),
}

impl<G> ResidentSourceTop<'_, G> {
    #[inline(always)]
    fn force_eof(&self, requested: bool) -> bool {
        requested && self.slot.name_class == super::SourceNameClass::File
    }

    #[inline(always)]
    fn record_first_touch(&mut self) {
        let needs_inverse = self.recording && !self.source.rollback.in_epoch(self.interval);
        if needs_inverse {
            *self.source_lex_captures = self.source_lex_captures.saturating_add(1);
            let payload = self
                .source_lex_states
                .insert(SourceLexExecutionState::capture(self.source, self.slot));
            self.undo.append(InputUndo::SourceLex {
                index: u32::try_from(self.index).expect("input row index fits u32"),
                payload,
            });
            self.source
                .rollback
                .set(self.interval, RowRollbackState::Inline);
            #[cfg(test)]
            {
                self.counters.first_touch_transitions =
                    self.counters.first_touch_transitions.saturating_add(1);
            }
        }
    }

    #[inline(never)]
    fn advance(
        &mut self,
        profile: crate::CommandProfile,
        force_eof: bool,
        state: &mut tex_state::CommandContext<'_, G>,
        create_control_sequences: bool,
    ) -> Result<ResidentSourceAdvance, ()> {
        record_source_lex_slot_borrow();
        if state.tracked_region_is_active() {
            super::observe_immutable_source(state, self.source, self.slot);
        }
        self.record_first_touch();
        let identity = self.source.identity();
        let step = {
            let mut queries = super::stack::LiveSourceQueries {
                state,
                create_control_sequences,
            };
            match profile.character_mode() {
                crate::CharacterMode::EightBitExact => self
                    .slot
                    .cursor
                    .next_compact_exact_byte_step(force_eof, &mut queries),
                crate::CharacterMode::UnicodeExtended => self
                    .slot
                    .cursor
                    .next_compact_unicode_step(force_eof, &mut queries),
            }
        };
        match step {
            CompactSourceTokenizationStep::Token(token) => {
                if self.source.frame.identity() != identity.0
                    || self.source.frame.advance().is_none()
                {
                    Err(())
                } else {
                    // The session epoch outlives one materialized JobStart
                    // generation, but its dense meaning bank does not. Admit
                    // an active source spelling before direct row lookup just
                    // as escaped source control sequences are admitted while
                    // their packed token is formed.
                    if let tex_state::token::Token::Char {
                        ch,
                        cat: tex_state::token::Catcode::Active,
                    } = token.word.semantic_token()
                    {
                        state.intern_active_character(ch);
                    }
                    let range = token.provenance.range();
                    let origin = if range.end().saturating_sub(range.start()) == 1 {
                        state.source_token_origin(range.source(), range.start(), range.end())
                    } else {
                        state.source_range_origin(range.source(), range.start(), range.end())
                    };
                    Ok(ResidentSourceAdvance::Delivered(
                        token.word,
                        origin,
                        token.provenance.location(),
                    ))
                }
            }
            CompactSourceTokenizationStep::InvalidCharacter => {
                Ok(ResidentSourceAdvance::InvalidCharacter)
            }
            CompactSourceTokenizationStep::NeedLine => {
                Ok(ResidentSourceAdvance::NeedLine(identity))
            }
            CompactSourceTokenizationStep::End => Ok(ResidentSourceAdvance::Exhausted(identity)),
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
        source: Option<tex_state::packed_input::SourceContext>,
    },
    Replacement {
        index: u32,
        payload: PayloadHandle,
    },
}

#[cold]
#[inline(never)]
fn append_resident_inline_inverse<G>(
    index: usize,
    state: InputLevelInlineState,
    interval: u64,
    rollback: &mut RowRollbackMarker,
    undo: &mut PackedJournal<InputUndo<G>, INPUT_UNDO_RECORDS_PER_CHUNK>,
    #[cfg(test)] first_touch_transitions: &mut u64,
) {
    rollback.set(interval, RowRollbackState::Inline);
    undo.append(InputUndo::Inline {
        index: u32::try_from(index).expect("input row index fits u32"),
        state,
    });
    #[cfg(test)]
    {
        *first_touch_transitions = first_touch_transitions.saturating_add(1);
    }
}

/// One rollback coordinate in a generation's dedicated input history.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct InputStackMark {
    pub(crate) top: u32,
    pub(crate) undo: PackedJournalMark,
    pub(crate) occupied_source_buffer_slots: usize,
    pub(crate) active_macro_bodies: usize,
    pub(crate) active_macro_parameters: usize,
}

/// Allocation-free identity of the one mutable input row that can affect a
/// newly captured TeX82 error context.
///
/// Rows below the semantic top are immutable while buried. Their source
/// context is copied into every descendant frame, so a generated-source
/// rebind is also visible in the top frame. `row_admissions` prevents a
/// push/pop ABA from making an older coordinate name the exposed row again.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InputStackContextCoordinate {
    history_interval: u64,
    row_admissions: u64,
    source_owner_swaps: u64,
    top: DiagnosticInputTop,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum DiagnosticInputTop {
    Empty,
    Source {
        frame: super::PackedInputFrame,
        slot: SourceSlotKey,
        name_class: super::SourceNameClass,
        cursor: DiagnosticSourceCursor,
    },
    Stored {
        frame: super::PackedInputFrame,
        retirement: super::RetirementBehavior,
        position: u32,
    },
    MacroBody {
        identity: super::InputLevelId,
        position: u32,
    },
    MacroArgument {
        identity: super::InputLevelId,
        position: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DiagnosticSourceCursor {
    physical_backing: DiagnosticBacking,
    current_backing: DiagnosticBacking,
    pending_acquired_line: bool,
    next_physical_offset: u64,
    next_line_number: u64,
    line: Option<DiagnosticSourceLine>,
    end_after_line: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DiagnosticBacking {
    source: tex_state::SourceId,
    bytes: usize,
    len: usize,
}

impl DiagnosticBacking {
    fn capture(source: &super::RegisteredSource) -> Self {
        Self {
            source: source.id,
            bytes: source.bytes.as_ptr() as usize,
            len: source.bytes.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DiagnosticSourceLine {
    physical: super::PhysicalLine,
    retained_end: u64,
    endline: Option<crate::CharacterCode>,
    cursor: super::lines::SourceLexCursor,
}

impl DiagnosticSourceCursor {
    fn capture(cursor: &super::SourceCursor) -> Self {
        Self {
            physical_backing: DiagnosticBacking::capture(&cursor.backing),
            current_backing: DiagnosticBacking::capture(cursor.current_backing()),
            pending_acquired_line: cursor.pending_acquired_line,
            next_physical_offset: cursor.next_physical_offset,
            next_line_number: cursor.next_line_number,
            line: cursor.line.as_ref().map(|line| DiagnosticSourceLine {
                physical: line.physical,
                retained_end: line.retained_end,
                endline: line.endline,
                cursor: line.cursor,
            }),
            end_after_line: cursor.end_after_line,
        }
    }
}

#[derive(Debug)]
struct InputStackFork {
    accepted_top: usize,
    accepted_occupied_source_buffer_slots: usize,
    accepted_active_macro_bodies: usize,
    accepted_active_macro_parameters: usize,
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
    active_macro_bodies: usize,
    active_macro_parameters: usize,
    fork: Option<InputStackFork>,
    recording: bool,
    interval: u64,
    row_admissions: u64,
    source_lex_captures: u64,
    source_owner_swaps: u64,
    #[cfg(test)]
    cursor_mutations: InputCursorMutationCounters,
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
            active_macro_bodies: 0,
            active_macro_parameters: 0,
            fork: None,
            recording: false,
            interval: 1,
            row_admissions: 0,
            source_lex_captures: 0,
            source_owner_swaps: 0,
            #[cfg(test)]
            cursor_mutations: InputCursorMutationCounters::default(),
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
                    (InputLevel::ReplayTokens(left), InputLevel::ReplayTokens(right)) => {
                        left == right
                    }
                    (InputLevel::DurableTokens(left), InputLevel::DurableTokens(right)) => {
                        left == right
                    }
                    (InputLevel::AttemptTokens(left), InputLevel::AttemptTokens(right)) => {
                        left == right
                    }
                    (InputLevel::MacroBody(left), InputLevel::MacroBody(right)) => left == right,
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
    pub(crate) const fn active_macro_parameters(&self) -> usize {
        self.active_macro_parameters
    }

    pub(crate) const fn active_macro_bodies(&self) -> usize {
        self.active_macro_bodies
    }

    pub(crate) fn push_macro_body(&mut self, value: InputLevel<G>, parameter_count: usize) {
        assert!(matches!(value, InputLevel::MacroBody(_)));
        self.active_macro_bodies = self
            .active_macro_bodies
            .checked_add(1)
            .expect("macro body stack fits usize");
        self.active_macro_parameters = self
            .active_macro_parameters
            .checked_add(parameter_count)
            .expect("macro parameter stack fits usize");
        self.push_row(value);
    }

    pub(crate) fn retire_macro_body(&mut self, parameter_count: usize) {
        self.active_macro_bodies = self
            .active_macro_bodies
            .checked_sub(1)
            .expect("input macro body count matches live macro rows");
        self.active_macro_parameters = self
            .active_macro_parameters
            .checked_sub(parameter_count)
            .expect("input parameter count matches live macro rows");
    }

    pub(crate) fn diagnostic_context_coordinate(&self) -> InputStackContextCoordinate {
        let top = match self.rows.get(self.top.saturating_sub(1)) {
            None => DiagnosticInputTop::Empty,
            Some(InputLevel::Source(source)) => {
                let slot = self.source_slot(source.slot);
                DiagnosticInputTop::Source {
                    frame: source.frame,
                    slot: source.slot,
                    name_class: slot.name_class,
                    cursor: DiagnosticSourceCursor::capture(&slot.cursor),
                }
            }
            Some(
                level @ (InputLevel::ReplayTokens(_)
                | InputLevel::DurableTokens(_)
                | InputLevel::AttemptTokens(_)),
            ) => DiagnosticInputTop::Stored {
                frame: level.stored_common().expect("stored row").frame,
                retirement: level.stored_common().expect("stored row").retirement,
                position: u32::try_from(level.stored_position().expect("stored row"))
                    .expect("stored token position fits u32"),
            },
            Some(InputLevel::MacroBody(cursor)) => DiagnosticInputTop::MacroBody {
                identity: cursor.identity(),
                position: cursor.position() as u32,
            },
            Some(InputLevel::MacroArgument(cursor)) => DiagnosticInputTop::MacroArgument {
                identity: cursor.identity(),
                position: cursor.position() as u32,
            },
        };
        InputStackContextCoordinate {
            history_interval: self.interval,
            row_admissions: self.row_admissions,
            source_owner_swaps: self.source_owner_swaps,
            top,
        }
    }

    pub(crate) fn validates_diagnostic_context(
        &self,
        coordinate: InputStackContextCoordinate,
    ) -> bool {
        self.diagnostic_context_coordinate() == coordinate
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
                    .rehome_generated(std::sync::Arc::clone(&bytes).into())
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
            self.source_owner_swaps = self.source_owner_swaps.saturating_add(1);
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
            rollback: RowRollbackMarker::default(),
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
        }
        for index in 0..self.top {
            let Some(source_context) = self.rows[index].source_context() else {
                continue;
            };
            if source_context.source() != id {
                continue;
            }
            if self.recording {
                self.undo.append(InputUndo::SourceContext {
                    index: u32::try_from(index).expect("input row index fits u32"),
                    source: Some(source_context),
                });
            }
            self.rows[index].set_source_context(Some(tex_state::packed_input::SourceContext::new(
                replacement_id,
                source_context.role(),
            )));
        }
        self.source_owner_swaps = self.source_owner_swaps.saturating_add(1);
        self.replace_source_buffer_slots(prior_buffer_slots, current_buffer_slots);
        true
    }

    /// External file or `\scantokens` context of the semantic top.
    ///
    /// Every admitted row carries this compact immutable execution fact in
    /// its common frame, so the query reads one row and never walks input
    /// ancestry or validates a source-owner slot.
    #[inline(always)]
    pub(crate) fn current_source_context(&self) -> Option<tex_state::packed_input::SourceContext> {
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
        let recording = self.recording;
        let interval = self.interval;
        let (rows, slots, states, undo) = (
            &mut self.rows,
            &mut self.source_slots,
            &mut self.source_lex_states,
            &mut self.undo,
        );
        let InputLevel::Source(source) = &mut rows[index] else {
            return None;
        };
        let key = source.slot;
        let needs_inverse = recording && !source.rollback.in_epoch(interval);
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
            source.rollback.set(interval, RowRollbackState::Inline);
        }
        let result = mutate(source, slot);
        Some(result)
    }
}

impl<G> crate::CommandState<G> {
    /// Delivers and advances the exact resident input cursor at the semantic top.
    ///
    /// Source, token-list, and direct macro-argument rows enter this one typed
    /// path. It indexes and discriminates the top once, performs the matching
    /// ordered first-touch transition directly, and ends each resident borrow
    /// with only the word, origin, position, and source scalars needed by the
    /// one final command-admission and settlement tail. Cold source,
    /// exhaustion, and parameter statuses carry no command borrow.
    pub(crate) fn advance_resident_command_into(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        fuel: &mut crate::fuel::CommandFuel,
        create_control_sequences: bool,
        mut destination: crate::command::EmptyCommand<'_, G>,
        retirement_publication: (
            &mut Option<&mut dyn CommandObserver>,
            &mut Option<super::InputLevelId>,
        ),
    ) -> Result<super::ResidentCommandInterception, super::ResidentCommandColdTransition> {
        let (observer, immediate_write_retirement) = retirement_publication;
        macro_rules! record_resident_first_touch {
            ($resident_index:expr, $rollback:expr, $state:expr) => {{
                if self.roots.input.levels.recording {
                    let interval = self.roots.input.levels.interval;
                    if !$rollback.in_epoch(interval) {
                        append_resident_inline_inverse(
                            $resident_index,
                            $state,
                            interval,
                            $rollback,
                            &mut self.roots.input.levels.undo,
                            #[cfg(test)]
                            &mut self
                                .roots
                                .input
                                .levels
                                .cursor_mutations
                                .first_touch_transitions,
                        );
                    }
                }
            }};
        }
        'resident: loop {
            let Some(resident_index) = self.roots.input.levels.top.checked_sub(1) else {
                return Err(super::ResidentCommandColdTransition::Empty);
            };
            #[cfg(test)]
            {
                self.roots.input.levels.cursor_mutations.typed_top_accesses = self
                    .roots
                    .input
                    .levels
                    .cursor_mutations
                    .typed_top_accesses
                    .saturating_add(1);
                self.raw_delivery_path_counters.resident_transitions = self
                    .raw_delivery_path_counters
                    .resident_transitions
                    .saturating_add(1);
            }
            let mut direct_source = false;
            let mut direct_source_line = None;
            let mut suppress_expandable = false;
            #[cfg(feature = "profiling")]
            let mut raw_delivery_kind = crate::fuel::RawDeliveryKind::StoredToken;
            #[cfg(test)]
            let mut generic_stored_delivery = false;
            #[cfg(test)]
            let mut macro_body_delivery = false;
            #[cfg(test)]
            let mut macro_argument_delivery = false;
            #[cfg(test)]
            let mut source_delivery = false;
            let (word, origin, identity, position, active_source) = 'delivery: {
                let (identity, resident_index) = match &mut self.roots.input.levels.rows
                    [resident_index]
                {
                    InputLevel::Source(source) => {
                        #[cfg(test)]
                        {
                            self.roots
                                .input
                                .levels
                                .cursor_mutations
                                .source_branch_entries = self
                                .roots
                                .input
                                .levels
                                .cursor_mutations
                                .source_branch_entries
                                .saturating_add(1);
                        }
                        let slot = self
                            .roots
                            .input
                            .levels
                            .source_slots
                            .resident_value_mut(source.slot.0.slot);
                        let mut top = ResidentSourceTop {
                            index: resident_index,
                            source,
                            slot,
                            recording: self.roots.input.levels.recording,
                            interval: self.roots.input.levels.interval,
                            undo: &mut self.roots.input.levels.undo,
                            source_lex_states: &mut self.roots.input.levels.source_lex_states,
                            source_lex_captures: &mut self.roots.input.levels.source_lex_captures,
                            #[cfg(test)]
                            counters: &mut self.roots.input.levels.cursor_mutations,
                        };
                        let force_eof = top.force_eof(self.roots.input.force_eof);
                        let identity = top.source.identity();
                        let position = top.slot.cursor.next_physical_offset;
                        let active_source = top.source.frame.source_context();
                        match top
                            .advance(
                                self.roots.expansion.profile,
                                force_eof,
                                state,
                                create_control_sequences,
                            )
                            .map_err(|()| super::ResidentCommandColdTransition::Failure)?
                        {
                            ResidentSourceAdvance::Delivered(word, origin, location) => {
                                direct_source = true;
                                direct_source_line = top.slot.cursor.line.as_ref().map(|line| {
                                    u32::try_from(line.physical.number()).unwrap_or(u32::MAX)
                                });
                                #[cfg(feature = "profiling")]
                                {
                                    raw_delivery_kind = crate::fuel::RawDeliveryKind::Source;
                                }
                                #[cfg(test)]
                                {
                                    source_delivery = true;
                                }
                                self.last_diagnostic_location = Some(location);
                                break 'delivery (
                                    word,
                                    origin,
                                    identity.0,
                                    position,
                                    active_source,
                                );
                            }
                            ResidentSourceAdvance::InvalidCharacter => {
                                return Err(super::ResidentCommandColdTransition::InvalidCharacter);
                            }
                            ResidentSourceAdvance::NeedLine(identity) => {
                                return Err(super::ResidentCommandColdTransition::NeedLine(
                                    identity,
                                ));
                            }
                            ResidentSourceAdvance::Exhausted(identity) => {
                                return Err(super::ResidentCommandColdTransition::SourceExhausted(
                                    identity,
                                ));
                            }
                        }
                    }
                    InputLevel::ReplayTokens(cursor) => {
                        record_resident_first_touch!(
                            resident_index,
                            &mut cursor.rollback,
                            InputLevelInlineState::replay_cursor(cursor.resident)
                        );
                        #[cfg(test)]
                        {
                            self.roots
                                .input
                                .levels
                                .cursor_mutations
                                .stored_token_branch_entries = self
                                .roots
                                .input
                                .levels
                                .cursor_mutations
                                .stored_token_branch_entries
                                .saturating_add(1);
                            self.roots
                                .input
                                .levels
                                .cursor_mutations
                                .replay_domain_dispatches = self
                                .roots
                                .input
                                .levels
                                .cursor_mutations
                                .replay_domain_dispatches
                                .saturating_add(1);
                            self.stored_token_advance_counters.span_selections = self
                                .stored_token_advance_counters
                                .span_selections
                                .saturating_add(1);
                        }
                        let position = cursor.resident.position();
                        let identity = super::InputLevelId(cursor.frame.identity());
                        let active_source = cursor.frame.source_context();
                        suppress_expandable = cursor.frame.flags().contains(
                            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
                        );
                        let spelling = self.roots.input.replay.advance_sequential(
                            cursor.replay,
                            &mut cursor.resident,
                            #[cfg(test)]
                            &mut self
                                .stored_token_advance_counters
                                .replay_segment_inspections,
                            #[cfg(test)]
                            &mut self.stored_token_advance_counters.replay_run_transitions,
                        );
                        match spelling {
                            Some(spelling) => {
                                #[cfg(test)]
                                {
                                    self.stored_token_advance_counters.packed_loads = self
                                        .stored_token_advance_counters
                                        .packed_loads
                                        .saturating_add(1);
                                    self.stored_token_advance_counters.cursor_advances = self
                                        .stored_token_advance_counters
                                        .cursor_advances
                                        .saturating_add(1);
                                    generic_stored_delivery = true;
                                }
                                if !matches!(cursor.behavior, super::TokenBehavior::Parameter)
                                    && let Some(slot) = spelling.token_word().out_parameter_slot()
                                {
                                    #[cfg(test)]
                                    {
                                        self.stored_token_advance_counters
                                            .parameter_interceptions = self
                                            .stored_token_advance_counters
                                            .parameter_interceptions
                                            .saturating_add(1);
                                        self.raw_delivery_path_counters
                                            .out_parameter_interceptions = self
                                            .raw_delivery_path_counters
                                            .out_parameter_interceptions
                                            .saturating_add(1);
                                    }
                                    self.push_resident_parameter_cursor(
                                        slot,
                                        None,
                                        active_source,
                                        observer,
                                    )
                                    .map_err(|()| super::ResidentCommandColdTransition::Failure)?;
                                    continue 'resident;
                                }
                                break 'delivery (
                                    spelling.token_word(),
                                    spelling.origin(),
                                    identity.0,
                                    u64::from(position),
                                    active_source,
                                );
                            }
                            None => (identity, resident_index),
                        }
                    }
                    InputLevel::AttemptTokens(cursor) => {
                        record_resident_first_touch!(
                            resident_index,
                            &mut cursor.rollback,
                            InputLevelInlineState::token_position(cursor.frame.position())
                        );
                        #[cfg(test)]
                        {
                            self.roots
                                .input
                                .levels
                                .cursor_mutations
                                .stored_token_branch_entries = self
                                .roots
                                .input
                                .levels
                                .cursor_mutations
                                .stored_token_branch_entries
                                .saturating_add(1);
                            self.roots
                                .input
                                .levels
                                .cursor_mutations
                                .attempt_domain_dispatches = self
                                .roots
                                .input
                                .levels
                                .cursor_mutations
                                .attempt_domain_dispatches
                                .saturating_add(1);
                        }
                        let position = cursor.frame.position();
                        let identity = super::InputLevelId(cursor.frame.identity());
                        let active_source = cursor.frame.source_context();
                        suppress_expandable = cursor.frame.flags().contains(
                            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
                        );
                        let spelling = self
                            .attempt
                            .arena()
                            .token_word(cursor.list, position as usize)
                            .ok();
                        match spelling {
                            Some(spelling) => {
                                #[cfg(test)]
                                {
                                    self.stored_token_advance_counters.packed_loads = self
                                        .stored_token_advance_counters
                                        .packed_loads
                                        .saturating_add(1);
                                }
                                if cursor.frame.advance() != Some(position) {
                                    return Err(super::ResidentCommandColdTransition::Failure);
                                }
                                #[cfg(test)]
                                {
                                    self.stored_token_advance_counters.cursor_advances = self
                                        .stored_token_advance_counters
                                        .cursor_advances
                                        .saturating_add(1);
                                    generic_stored_delivery = true;
                                }
                                if !matches!(cursor.behavior, super::TokenBehavior::Parameter)
                                    && let Some(slot) = spelling.token_word().out_parameter_slot()
                                {
                                    #[cfg(test)]
                                    {
                                        self.stored_token_advance_counters
                                            .parameter_interceptions = self
                                            .stored_token_advance_counters
                                            .parameter_interceptions
                                            .saturating_add(1);
                                        self.raw_delivery_path_counters
                                            .out_parameter_interceptions = self
                                            .raw_delivery_path_counters
                                            .out_parameter_interceptions
                                            .saturating_add(1);
                                    }
                                    self.push_resident_parameter_cursor(
                                        slot,
                                        None,
                                        active_source,
                                        observer,
                                    )
                                    .map_err(|()| super::ResidentCommandColdTransition::Failure)?;
                                    continue 'resident;
                                }
                                break 'delivery (
                                    spelling.token_word(),
                                    spelling.origin(),
                                    identity.0,
                                    u64::from(position),
                                    active_source,
                                );
                            }
                            None => (identity, resident_index),
                        }
                    }
                    InputLevel::DurableTokens(cursor) => {
                        record_resident_first_touch!(
                            resident_index,
                            &mut cursor.rollback,
                            InputLevelInlineState::token_position(cursor.frame.position())
                        );
                        #[cfg(test)]
                        {
                            self.roots
                                .input
                                .levels
                                .cursor_mutations
                                .stored_token_branch_entries = self
                                .roots
                                .input
                                .levels
                                .cursor_mutations
                                .stored_token_branch_entries
                                .saturating_add(1);
                            self.roots
                                .input
                                .levels
                                .cursor_mutations
                                .durable_domain_dispatches = self
                                .roots
                                .input
                                .levels
                                .cursor_mutations
                                .durable_domain_dispatches
                                .saturating_add(1);
                        }
                        let position = cursor.frame.position();
                        let identity = super::InputLevelId(cursor.frame.identity());
                        let active_source = cursor.frame.source_context();
                        suppress_expandable = cursor.frame.flags().contains(
                            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
                        );
                        let spelling = cursor
                            .list
                            .word_at(position as usize)
                            .map(|word| (word, tex_state::token::OriginId::UNKNOWN));
                        match spelling {
                            Some((word, origin)) => {
                                #[cfg(test)]
                                {
                                    self.stored_token_advance_counters.packed_loads = self
                                        .stored_token_advance_counters
                                        .packed_loads
                                        .saturating_add(1);
                                }
                                if cursor.frame.advance() != Some(position) {
                                    return Err(super::ResidentCommandColdTransition::Failure);
                                }
                                #[cfg(test)]
                                {
                                    self.stored_token_advance_counters.cursor_advances = self
                                        .stored_token_advance_counters
                                        .cursor_advances
                                        .saturating_add(1);
                                    generic_stored_delivery = true;
                                }
                                if !matches!(cursor.behavior, super::TokenBehavior::Parameter)
                                    && let Some(slot) = word.out_parameter_slot()
                                {
                                    #[cfg(test)]
                                    {
                                        self.stored_token_advance_counters
                                            .parameter_interceptions = self
                                            .stored_token_advance_counters
                                            .parameter_interceptions
                                            .saturating_add(1);
                                        self.raw_delivery_path_counters
                                            .out_parameter_interceptions = self
                                            .raw_delivery_path_counters
                                            .out_parameter_interceptions
                                            .saturating_add(1);
                                    }
                                    self.push_resident_parameter_cursor(
                                        slot,
                                        None,
                                        active_source,
                                        observer,
                                    )
                                    .map_err(|()| super::ResidentCommandColdTransition::Failure)?;
                                    continue 'resident;
                                }
                                break 'delivery (
                                    word,
                                    origin,
                                    identity.0,
                                    u64::from(position),
                                    active_source,
                                );
                            }
                            None => (identity, resident_index),
                        }
                    }
                    InputLevel::MacroBody(top) => {
                        record_resident_first_touch!(
                            resident_index,
                            &mut top.rollback,
                            InputLevelInlineState::macro_span(top.body.cursor())
                        );
                        #[cfg(test)]
                        {
                            self.roots
                                .input
                                .levels
                                .cursor_mutations
                                .macro_body_domain_dispatches = self
                                .roots
                                .input
                                .levels
                                .cursor_mutations
                                .macro_body_domain_dispatches
                                .saturating_add(1);
                        }
                        let exhausted_identity = top.identity();
                        let identity = exhausted_identity.0;
                        let active_source = top.active_source();
                        let arguments = top.arguments;
                        let Some((position, word)) = top.advance_word(state) else {
                            if let Some(transition) = self
                                .finish_resident_exhaustion(
                                    resident_index,
                                    exhausted_identity,
                                    observer,
                                    immediate_write_retirement,
                                )
                                .map_err(|()| super::ResidentCommandColdTransition::Failure)?
                            {
                                return Err(transition);
                            }
                            continue 'resident;
                        };
                        #[cfg(test)]
                        {
                            self.macro_kernel_counters.body_words =
                                self.macro_kernel_counters.body_words.saturating_add(1);
                            self.macro_kernel_counters.body_cursor_advances = self
                                .macro_kernel_counters
                                .body_cursor_advances
                                .saturating_add(1);
                            macro_body_delivery = true;
                        }
                        if let Some(slot) = word.out_parameter_slot() {
                            #[cfg(test)]
                            {
                                self.macro_kernel_counters.body_parameter_pushes = self
                                    .macro_kernel_counters
                                    .body_parameter_pushes
                                    .saturating_add(1);
                                self.raw_delivery_path_counters.out_parameter_interceptions = self
                                    .raw_delivery_path_counters
                                    .out_parameter_interceptions
                                    .saturating_add(1);
                            }
                            self.push_resident_parameter_cursor(
                                slot,
                                arguments,
                                active_source,
                                observer,
                            )
                            .map_err(|()| super::ResidentCommandColdTransition::Failure)?;
                            continue 'resident;
                        }
                        break 'delivery (
                            word,
                            tex_state::token::OriginId::UNKNOWN,
                            identity,
                            u64::from(position),
                            active_source,
                        );
                    }
                    InputLevel::MacroArgument(top) => {
                        record_resident_first_touch!(
                            resident_index,
                            &mut top.rollback,
                            InputLevelInlineState::macro_argument(
                                top.absolute_position(),
                                top.origin_run,
                            )
                        );
                        #[cfg(test)]
                        {
                            self.roots
                                .input
                                .levels
                                .cursor_mutations
                                .macro_argument_branch_entries = self
                                .roots
                                .input
                                .levels
                                .cursor_mutations
                                .macro_argument_branch_entries
                                .saturating_add(1);
                        }
                        let exhausted_identity = top.identity();
                        let identity = exhausted_identity.0;
                        let active_source = top.active_source();
                        let Some((position, word, origin)) = top
                            .advance_delivery(&self.scratch)
                            .map_err(|()| super::ResidentCommandColdTransition::Failure)?
                        else {
                            if let Some(transition) = self
                                .finish_resident_exhaustion(
                                    resident_index,
                                    exhausted_identity,
                                    observer,
                                    immediate_write_retirement,
                                )
                                .map_err(|()| super::ResidentCommandColdTransition::Failure)?
                            {
                                return Err(transition);
                            }
                            continue 'resident;
                        };
                        #[cfg(test)]
                        {
                            self.macro_kernel_counters.argument_words =
                                self.macro_kernel_counters.argument_words.saturating_add(1);
                            self.macro_kernel_counters.argument_cursor_advances = self
                                .macro_kernel_counters
                                .argument_cursor_advances
                                .saturating_add(1);
                            macro_argument_delivery = true;
                        }
                        #[cfg(feature = "profiling")]
                        {
                            raw_delivery_kind = crate::fuel::RawDeliveryKind::MacroArgument;
                        }
                        break 'delivery (
                            word,
                            origin,
                            identity,
                            u64::from(position),
                            active_source,
                        );
                    }
                };
                let Some(retirement) = self
                    .retire_resident_ordinary_input(resident_index)
                    .map_err(|_| super::ResidentCommandColdTransition::Failure)?
                else {
                    // Terminal token input and v-templates still waiting for
                    // `do_endv` are explicit cold boundaries and still carry
                    // their identity outward. An awaiting post-`do_endv`
                    // v-template is popped above as a resident §357 restart.
                    return Err(super::ResidentCommandColdTransition::TokenExhausted {
                        identity,
                        resident_index,
                    });
                };
                if let Some(episode) = self.settle_resident_ordinary_retirement(
                    retirement,
                    observer,
                    immediate_write_retirement,
                ) {
                    return Err(super::ResidentCommandColdTransition::ReplayCompleted(
                        episode,
                    ));
                }
                continue 'resident;
            };
            #[cfg(test)]
            {
                if generic_stored_delivery {
                    self.stored_token_advance_counters.command_writes = self
                        .stored_token_advance_counters
                        .command_writes
                        .saturating_add(1);
                    self.raw_delivery_path_counters.stored_direct = self
                        .raw_delivery_path_counters
                        .stored_direct
                        .saturating_add(1);
                }
                if macro_body_delivery {
                    self.macro_kernel_counters.body_command_writes = self
                        .macro_kernel_counters
                        .body_command_writes
                        .saturating_add(1);
                }
                if macro_argument_delivery {
                    self.macro_kernel_counters.argument_command_writes = self
                        .macro_kernel_counters
                        .argument_command_writes
                        .saturating_add(1);
                    self.raw_delivery_path_counters.macro_argument_direct = self
                        .raw_delivery_path_counters
                        .macro_argument_direct
                        .saturating_add(1);
                }
                if source_delivery {
                    self.raw_delivery_path_counters.source_direct = self
                        .raw_delivery_path_counters
                        .source_direct
                        .saturating_add(1);
                }
            }
            let resolution = destination.reborrow().write_resolved_delivery(
                word,
                origin,
                identity,
                position,
                active_source,
                direct_source,
                direct_source_line,
                suppress_expandable,
                state,
            );
            #[cfg(test)]
            if generic_stored_delivery {
                self.stored_token_advance_counters.meaning_lookups = self
                    .stored_token_advance_counters
                    .meaning_lookups
                    .saturating_add(u64::from(resolution.meaning_lookup()));
            }
            return self.settle_resident_delivery(
                fuel,
                destination.reborrow(),
                resolution,
                #[cfg(feature = "profiling")]
                raw_delivery_kind,
            );
        }
    }

    /// Completes the one destination-owned resident transition in the caller's
    /// hot frame. Keeping this tail fused is load-bearing: an out-of-line
    /// boundary repeats the per-token result handoff after the typed branch
    /// has already written the final command.
    #[inline(always)]
    fn settle_resident_delivery(
        &mut self,
        _fuel: &mut crate::fuel::CommandFuel,
        destination: crate::command::EmptyCommand<'_, G>,
        resolution: tex_state::token::PackedMeaningResolution,
        #[cfg(feature = "profiling")] kind: crate::fuel::RawDeliveryKind,
    ) -> Result<super::ResidentCommandInterception, super::ResidentCommandColdTransition> {
        let scanner_active = !matches!(
            self.roots.scanner.status(),
            crate::processor::ScannerStatus::Normal
        );
        let command = destination.into_resident();
        if command.suppresses_expandable_control_sequence() {
            command.suppress_expandable();
        }
        #[cfg(feature = "profiling")]
        _fuel.record_raw_delivery(scanner_active, resolution.meaning_lookup(), kind);
        let interception = if command.is_outer() && scanner_active {
            super::ResidentCommandInterception::Outer
        } else {
            self.roots.alignment.classify_delivery(
                &mut self.timeline,
                command,
                resolution.literal_catcode(),
            );
            super::ResidentCommandInterception::Ready
        };
        Ok(interception)
    }

    #[cold]
    fn push_resident_parameter_cursor(
        &mut self,
        slot: u8,
        arguments: Option<crate::execution_scratch::ArgumentSetId<G>>,
        active_source: Option<tex_state::packed_input::SourceContext>,
        observer: &mut Option<&mut dyn CommandObserver>,
    ) -> Result<(), ()> {
        if !(1..=9).contains(&slot) {
            return Err(());
        }
        let owner = arguments.ok_or(())?;
        let range = self
            .scratch
            .argument_range(owner, slot)
            .map_err(|_| ())?
            .ok_or(())?;
        let identity = super::InputLevelId(self.roots.input.next_level_identity);
        self.timeline
            .record_next_input_level_identity(self.roots.input.next_level_identity);
        self.roots.input.next_level_identity = self.roots.input.next_level_identity.wrapping_add(1);
        let origin_run = self
            .scratch
            .admitted_argument_origin_run(range)
            .map_err(|_| ())?;
        self.stack_usage.input_stack = self
            .stack_usage
            .input_stack
            .max(self.roots.input.levels.len());
        self.roots
            .input
            .levels
            .push(InputLevel::MacroArgument(super::MacroArgumentCursor::new(
                identity,
                range,
                slot,
                origin_run,
                active_source,
            )));
        if let Some(sink) = observer.as_deref_mut() {
            sink.committed(CommandObservation::Input(InputRecord {
                transition: InputTransition::Push,
                reason: InputReason::Parameter,
                source_name: None,
                source: None,
                level: identity.0,
                position: 0,
            }));
        }
        Ok(())
    }

    #[cold]
    fn finish_resident_exhaustion(
        &mut self,
        resident_index: usize,
        identity: super::InputLevelId,
        observer: &mut Option<&mut dyn CommandObserver>,
        immediate_write_retirement: &mut Option<super::InputLevelId>,
    ) -> Result<Option<super::ResidentCommandColdTransition>, ()> {
        let Some(retirement) = self
            .retire_resident_ordinary_input(resident_index)
            .map_err(|_| ())?
        else {
            return Ok(Some(super::ResidentCommandColdTransition::TokenExhausted {
                identity,
                resident_index,
            }));
        };
        Ok(self
            .settle_resident_ordinary_retirement(retirement, observer, immediate_write_retirement)
            .map(super::ResidentCommandColdTransition::ReplayCompleted))
    }

    #[inline(always)]
    fn settle_resident_ordinary_retirement(
        &mut self,
        retirement: super::InputRetirement,
        observer: &mut Option<&mut dyn CommandObserver>,
        immediate_write_retirement: &mut Option<super::InputLevelId>,
    ) -> Option<crate::CommandReplayEpisode> {
        debug_assert!(retirement.is_resident_restart());
        self.settle_input_retirement(retirement, observer, immediate_write_retirement)
    }

    pub(crate) fn settle_input_retirement(
        &mut self,
        retirement: super::InputRetirement,
        observer: &mut Option<&mut dyn CommandObserver>,
        immediate_write_retirement: &mut Option<super::InputLevelId>,
    ) -> Option<crate::CommandReplayEpisode> {
        let identity = retirement.identity;
        let reason = if *immediate_write_retirement == Some(identity) {
            *immediate_write_retirement = None;
            InputReason::Write
        } else {
            observed_retirement_reason(retirement.action, retirement.reason)
        };
        if !matches!(
            retirement.action,
            super::InputRetirementAction::VTemplateRetained
        ) && let Some(sink) = observer.as_deref_mut()
        {
            sink.committed(CommandObservation::Input(InputRecord {
                transition: if matches!(
                    retirement.action,
                    super::InputRetirementAction::TerminalStop
                ) {
                    InputTransition::Stop
                } else {
                    InputTransition::Retire
                },
                reason,
                source_name: retirement.name_class,
                source: retirement.source,
                level: identity.0,
                position: 0,
            }));
        }
        if let Some(transition) = match retirement.reason {
            _ if matches!(
                retirement.action,
                super::InputRetirementAction::VTemplateRetained
            ) =>
            {
                None
            }
            super::InputRetirementReason::AlignmentUTemplate => Some("u_template_retire"),
            super::InputRetirementReason::AlignmentVTemplate => Some("v_template_retire"),
            super::InputRetirementReason::AlignmentOmitTemplate => Some("omit_template_retire"),
            _ => None,
        } && let Some(sink) = observer.as_deref_mut()
        {
            sink.committed(CommandObservation::Alignment(AlignmentRecord {
                transition,
                alignment: self
                    .roots
                    .alignment
                    .active_alignment
                    .map(|alignment| alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                align_state: self.roots.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            }));
        }
        let popped = matches!(
            retirement.action,
            super::InputRetirementAction::SourcePopped
                | super::InputRetirementAction::TokenListPopped
                | super::InputRetirementAction::VTemplatePopped
        );
        if popped {
            let previous_align_state = self.roots.alignment.align_state;
            self.record_alignment_phase();
            if self.roots.alignment.finish_u_template(identity)
                && let Some(sink) = observer.as_deref_mut()
            {
                sink.committed(CommandObservation::Alignment(AlignmentRecord {
                    transition: "state_change",
                    alignment: self
                        .roots
                        .alignment
                        .active_alignment
                        .map(|alignment| alignment.raw()),
                    nesting: self.alignment_observation_nesting(),
                    align_state: self.roots.alignment.align_state,
                    delimiter: None,
                    previous_align_state: Some(previous_align_state),
                }));
            }
        }
        popped.then(|| self.complete_replay(identity)).flatten()
    }
}

pub(crate) fn observed_retirement_reason(
    action: super::InputRetirementAction,
    reason: super::InputRetirementReason,
) -> InputReason {
    match (action, reason) {
        (
            super::InputRetirementAction::SourcePopped
            | super::InputRetirementAction::TerminalStop
            | super::InputRetirementAction::ReadLineEnded,
            _,
        ) => InputReason::Source,
        (_, super::InputRetirementReason::Backup) => InputReason::Backup,
        (_, super::InputRetirementReason::Macro) => InputReason::Macro,
        (_, super::InputRetirementReason::Parameter) => InputReason::Parameter,
        (_, super::InputRetirementReason::AlignmentUTemplate) => InputReason::AlignmentUTemplate,
        (
            _,
            super::InputRetirementReason::AlignmentVTemplate
            | super::InputRetirementReason::AlignmentOmitTemplate,
        ) => InputReason::AlignmentVTemplate,
        (_, super::InputRetirementReason::Recovery) => InputReason::Recovery,
        (_, super::InputRetirementReason::TokenList(stored)) => {
            crate::processor::stored_input_reason(stored)
        }
        (_, super::InputRetirementReason::Source) => InputReason::Source,
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
        if self.rows[index].stored_common().is_none() {
            return false;
        }
        self.record_token_state(index);
        let cursor = self.rows[index].stored_common_mut().expect("stored row");
        cursor.retirement = retirement;
        true
    }

    pub(crate) fn retain_top_v_template(&mut self) -> bool {
        let index = match self.top.checked_sub(1) {
            Some(index) => index,
            None => return false,
        };
        if self.rows[index].stored_common().is_none() {
            return false;
        }
        self.record_token_state(index);
        let cursor = self.rows[index].stored_common_mut().expect("stored row");
        cursor.retirement = super::RetirementBehavior::AwaitingVTemplateRetirement;
        cursor
            .frame
            .add_flags(tex_state::packed_input::InputFrameFlags::RETAIN_AT_END);
        true
    }

    pub(crate) fn extend_top_token_limit(
        &mut self,
        additional: u32,
        replay_cursor: super::ResidentReplayCursor,
    ) -> Option<bool> {
        let index = self.top.checked_sub(1)?;
        let InputLevel::ReplayTokens(_) = &self.rows[index] else {
            return None;
        };
        if self.recording && !self.rows[index].rollback_marker().in_epoch(self.interval) {
            let InputLevel::ReplayTokens(cursor) = &self.rows[index] else {
                unreachable!()
            };
            self.record_inline(index, InputLevelInlineState::replay_cursor(cursor.resident));
        }
        self.record_token_state(index);
        let InputLevel::ReplayTokens(cursor) = &mut self.rows[index] else {
            unreachable!()
        };
        cursor.resident = replay_cursor;
        cursor.common.len = cursor.common.len.checked_add(additional)?;
        let extended = cursor.frame.extend_limit(additional).is_some();
        Some(extended)
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn toggle_top_token_retirement(&mut self) -> bool {
        let Some(current) = self
            .rows
            .get(self.top.saturating_sub(1))
            .and_then(|row| row.stored_common().map(|cursor| cursor.retirement))
        else {
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

    fn push_row(&mut self, mut value: InputLevel<G>) {
        self.row_admissions = self.row_admissions.saturating_add(1);
        value
            .rollback_marker_mut()
            .set(self.interval, RowRollbackState::Admitted);
        if self.top == self.rows.len() {
            self.rows.push(value);
        } else if self.recording
            && self.rows[self.top]
                .rollback_marker()
                .needs_replacement(self.interval)
        {
            self.displaced_rows.warm_first_page();
            let old = std::mem::replace(&mut self.rows[self.top], value);
            let payload = self.displaced_rows.insert(old);
            self.undo.append(InputUndo::Replacement {
                index: u32::try_from(self.top).expect("input row index fits u32"),
                payload,
            });
        } else {
            let old = std::mem::replace(&mut self.rows[self.top], value);
            if let InputLevel::Source(source) = old {
                self.source_slots.release(source.slot.0);
            }
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
        self.top = index;
        if !self.recording {
            let retired = self.rows.pop().expect("input top exists");
            if let InputLevel::Source(source) = retired {
                self.source_slots.release(source.slot.0);
            }
        }
        Some(result)
    }

    /// Pops a non-source row already admitted as the resident semantic top.
    ///
    /// The caller carries `index` from the one top selection which found
    /// exhaustion, so this performs neither another top lookup nor the
    /// identity validation required by cold detached coordinates.
    pub(super) fn pop_resident_project<R>(
        &mut self,
        index: usize,
        project: impl FnOnce(&InputLevel<G>) -> R,
    ) -> R {
        debug_assert_eq!(index.checked_add(1), Some(self.top));
        debug_assert!(!matches!(self.rows[index], InputLevel::Source(_)));
        let result = project(&self.rows[index]);
        self.top = index;
        if !self.recording {
            self.rows.pop();
        }
        result
    }

    pub(super) fn resident_at(&self, index: usize) -> &InputLevel<G> {
        debug_assert_eq!(index.checked_add(1), Some(self.top));
        &self.rows[index]
    }

    pub(crate) fn mutate_top_source_cursor<R>(
        &mut self,
        mutate: impl FnOnce(&mut SourceLevel<G>, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        self.mutate_top_source(SourceOwnerTransition::Cursor, mutate)
    }

    pub(crate) fn mutate_top_source_every_eof<R>(
        &mut self,
        mutate: impl FnOnce(&mut SourceLevel<G>, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        self.mutate_top_source(SourceOwnerTransition::EveryEof, mutate)
    }

    pub(crate) fn mutate_top_source_backing<R>(
        &mut self,
        replacement: super::RegisteredSource,
        mutate: impl FnOnce(&mut SourceLevel<G>, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        self.mutate_top_source(SourceOwnerTransition::Backing(replacement), mutate)
    }

    /// Moves the first source-owner inverse in an interval into the same
    /// ordered history as row advances and replacements.
    ///
    /// Later owner transitions of that row release their displaced owner in
    /// place: rollback needs only the interval's initial owner, and candidate
    /// redo obtains the final owner by swapping that one inverse a second
    /// time. A row admitted during the interval likewise releases directly.
    fn mutate_top_source<R>(
        &mut self,
        transition: SourceOwnerTransition,
        mutate: impl FnOnce(&mut SourceLevel<G>, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        let index = self.top.checked_sub(1)?;
        if !matches!(self.rows[index], InputLevel::Source(_)) {
            return None;
        }
        let key = match &self.rows[index] {
            InputLevel::Source(source) => source.slot,
            _ => unreachable!("source mutation names a source row"),
        };
        let prior_buffer_slots = self
            .source_slots
            .value(key.0)
            .expect("source slot remains live")
            .occupied_buffer_slots;
        let retain_inverse = self.recording
            && self.rows[index]
                .rollback_marker()
                .needs_source_owner_inverse(self.interval);
        if retain_inverse {
            self.source_owner_states.warm_first_page();
        }
        let (payload, result, current_buffer_slots) = {
            let (rows, slots, owner_states) = (
                &mut self.rows,
                &mut self.source_slots,
                &mut self.source_owner_states,
            );
            let InputLevel::Source(source) = &mut rows[index] else {
                unreachable!()
            };
            let slot = slots.value_mut(key.0).expect("source slot remains live");
            let payload = transition
                .apply(source, slot, retain_inverse)
                .map(|state| owner_states.insert(state));
            let result = mutate(source, slot);
            let current_buffer_slots = super::source::occupied_source_buffer_slots(&slot.cursor);
            slot.occupied_buffer_slots = current_buffer_slots;
            (payload, result, current_buffer_slots)
        };
        if let Some(payload) = payload {
            self.undo.append(InputUndo::SourceOwner {
                index: u32::try_from(index).expect("input row index fits u32"),
                payload,
                generation: core::marker::PhantomData,
            });
            self.rows[index]
                .rollback_marker_mut()
                .set(self.interval, RowRollbackState::Cold);
            self.source_owner_swaps = self.source_owner_swaps.saturating_add(1);
        }
        self.replace_source_buffer_slots(prior_buffer_slots, current_buffer_slots);
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
            occupied_source_buffer_slots: self.occupied_source_buffer_slots,
            active_macro_bodies: self.active_macro_bodies,
            active_macro_parameters: self.active_macro_parameters,
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
            self.occupied_source_buffer_slots = mark.occupied_source_buffer_slots;
            self.active_macro_bodies = mark.active_macro_bodies;
            self.active_macro_parameters = mark.active_macro_parameters;
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
        let accepted_active_macro_bodies = self.active_macro_bodies;
        let accepted_active_macro_parameters = self.active_macro_parameters;
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
        self.active_macro_bodies = mark.active_macro_bodies;
        self.active_macro_parameters = mark.active_macro_parameters;
        self.fork = Some(InputStackFork {
            accepted_top,
            accepted_occupied_source_buffer_slots,
            accepted_active_macro_bodies,
            accepted_active_macro_parameters,
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
        self.active_macro_bodies = fork.accepted_active_macro_bodies;
        self.active_macro_parameters = fork.accepted_active_macro_parameters;
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

    #[cfg(test)]
    pub(crate) const fn cursor_mutation_counters(&self) -> InputCursorMutationCounters {
        self.cursor_mutations
    }

    #[cfg(test)]
    pub(crate) fn reset_cursor_mutation_counters(&mut self) {
        self.cursor_mutations = InputCursorMutationCounters::default();
    }

    fn record_inline(&mut self, index: usize, state: InputLevelInlineState) {
        if !self.recording || self.rows[index].rollback_marker().in_epoch(self.interval) {
            return;
        }
        self.rows[index]
            .rollback_marker_mut()
            .set(self.interval, RowRollbackState::Inline);
        self.undo.append(InputUndo::Inline {
            index: u32::try_from(index).expect("input row index fits u32"),
            state,
        });
    }

    /// Captures the complete cold token execution state once per interval.
    /// A preceding resident advance may already have retained only the scalar
    /// position, so cold retirement/limit/flag mutation has its own capture
    /// bit and ordered inverse.
    fn record_token_state(&mut self, index: usize) {
        if !self.recording
            || self.rows[index]
                .rollback_marker()
                .cold_captured(self.interval)
        {
            return;
        }
        let cursor = self.rows[index]
            .stored_common()
            .expect("cold token capture names a token row");
        let state = InputLevelInlineState::new(cursor.frame, cursor.retirement);
        self.rows[index]
            .rollback_marker_mut()
            .set(self.interval, RowRollbackState::Cold);
        self.undo.append(InputUndo::Inline {
            index: u32::try_from(index).expect("input row index fits u32"),
            state,
        });
    }

    fn next_interval(&mut self) {
        self.interval = if self.interval == MAX_ROW_ROLLBACK_EPOCH {
            1
        } else {
            self.interval + 1
        };
        if self.interval == 1 {
            for row in &mut self.rows {
                *row.rollback_marker_mut() = RowRollbackMarker::default();
            }
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
                let displaced_row = displaced
                    .value_mut(*payload)
                    .expect("input replacement inverse remains live");
                rows[*index as usize].swap_payload_preserving_rollback(displaced_row);
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
