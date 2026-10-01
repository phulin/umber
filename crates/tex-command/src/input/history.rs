//! Generation-owned authoritative input rows.
//!
//! The live stack is the only copy a running job mutates. Checkpoints copy
//! the whole stack by value at quiescence, so no row carries rollback state
//! and no transition records an inverse.

#[cfg(test)]
mod tests;

use crate::observation::{
    AlignmentRecord, CommandObservation, CommandObserver, InputReason, InputRecord, InputTransition,
};

use core::hash::{Hash, Hasher};
use core::ops::{Deref, Index};
use core::slice::SliceIndex;

use super::{CompactSourceTokenizationStep, InputLevel, SourceLevel, SourceSlot};

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

enum SourceOwnerTransition {
    Cursor,
    EveryEof,
    Backing(super::RegisteredSource),
}

impl SourceOwnerTransition {
    /// Releases the owners a cold source transition displaces. The scalar
    /// cursor state stays resident for the transition that follows.
    fn apply<G>(self, slot: &mut SourceSlot<G>) {
        match self {
            Self::Cursor => slot.cursor.release_execution_owners(),
            Self::EveryEof => {
                slot.cursor.release_execution_owners();
                slot.every_eof = None;
            }
            Self::Backing(replacement) => {
                slot.cursor.backing = replacement;
                slot.cursor.backing_registered = false;
                slot.cursor.backing_capability = None;
                slot.cursor.release_execution_owners();
            }
        }
    }
}

pub(crate) struct ResidentSourceTop<'a, G> {
    pub(crate) source: &'a mut SourceLevel<G>,
}

pub(crate) enum ResidentSourceAdvance {
    Delivered(
        tex_state::token::TokenWord,
        tex_state::token::OriginId,
        super::SourceLocation,
    ),
    InvalidCharacter,
    NeedLine(super::InputLevelId),
    Exhausted(super::InputLevelId),
}

pub(crate) enum ResidentSourceCharacterRun<E> {
    Unavailable,
    Consumed { count: u32 },
    Failed { count: u32, error: E },
}

/// A borrowed retained suffix of the currently loaded source line.
///
/// The line backing and source-map capability remain owned by the resident
/// source slot.  This value is only a short-lived admission contract: the
/// executor may inspect the bytes and origins, then the processor commits the
/// source cursor once for the accepted prefix.
#[derive(Clone, Copy, Debug)]
pub struct BorrowedSourceCharacterRun<'a> {
    bytes: &'a [u8],
    source: tex_state::SourceId,
    byte_start: u64,
    capability: Option<tex_state::source_map::RegisteredSource>,
    fuel_limited: bool,
}

impl BorrowedSourceCharacterRun<'_> {
    /// The borrowed physical bytes presented to ordinary character admission.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }

    /// The source identity of the current line.
    #[must_use]
    pub const fn source(&self) -> tex_state::SourceId {
        self.source
    }

    /// The source-relative byte offset of the first borrowed byte.
    #[must_use]
    pub const fn byte_start(&self) -> u64 {
        self.byte_start
    }

    /// Whether the prefix was capped by the command-fuel budget.
    #[must_use]
    pub const fn fuel_limited(&self) -> bool {
        self.fuel_limited
    }

    pub(crate) fn limit_to(self, count: usize) -> Self {
        Self {
            bytes: &self.bytes[..count],
            fuel_limited: true,
            ..self
        }
    }

    /// Returns the direct origin for one borrowed byte without a source-map
    /// lookup.  Registration failure intentionally degrades to unknown, as it
    /// does for the scalar source tokenizer.
    #[must_use]
    pub fn origin(&self, index: usize) -> tex_state::token::OriginId {
        let Some(offset) = self.byte_start.checked_add(index as u64) else {
            return tex_state::token::OriginId::UNKNOWN;
        };
        self.capability
            .and_then(|capability| capability.direct_origin(offset, offset.saturating_add(1)))
            .unwrap_or(tex_state::token::OriginId::UNKNOWN)
    }
}

impl<G> ResidentSourceTop<'_, G> {
    /// File position remembered by the §342 diagnostic observer. Terminal
    /// and `\read` levels have `name<=17`, so they leave its prior site alone.
    pub(crate) fn diagnostic_location(
        &self,
        location: super::SourceLocation,
    ) -> Option<super::DiagnosticLocation> {
        let line = self.source.slot.cursor.line.as_ref()?;
        let source = match self.source.slot.name_class {
            super::SourceNameClass::File | super::SourceNameClass::Scantokens(_) => {
                self.source.slot.cursor.backing.id
            }
            super::SourceNameClass::Terminal | super::SourceNameClass::ReadStream(_) => {
                return None;
            }
        };
        Some(line.diagnostic_location(source, location))
    }

    pub(crate) fn current_diagnostic_location(&self) -> Option<super::DiagnosticLocation> {
        let line = self.source.slot.cursor.line.as_ref()?;
        self.diagnostic_location(super::SourceLocation::new(
            line.physical.source,
            line.cursor.byte_cursor.saturating_sub(1),
        ))
    }

    #[inline(always)]
    pub(crate) fn force_eof(&self, requested: bool) -> bool {
        requested && self.source.slot.name_class == super::SourceNameClass::File
    }

    /// Borrows the retained suffix of the currently loaded source line.
    ///
    /// Character admission owns the live catcode and font checks. Keeping the
    /// source operation to one backing slice leaves non-ASCII bytes and every
    /// non-ordinary catcode at the exact first scalar boundary, where the
    /// canonical tokenizer can handle them.
    #[inline(always)]
    pub(crate) fn borrow_character_run<'a>(
        &'a mut self,
    ) -> Result<Option<BorrowedSourceCharacterRun<'a>>, ()> {
        let Some(line) = self.source.slot.cursor.line.as_ref() else {
            return Ok(None);
        };
        let start = line.cursor.byte_cursor;
        if start >= line.retained_end {
            return Ok(None);
        }
        let backing = self.source.slot.cursor.current_backing();
        let start_index = usize::try_from(start).map_err(|_| ())?;
        let limit_index = usize::try_from(line.retained_end).map_err(|_| ())?;
        let bytes = backing.bytes.get(start_index..limit_index).ok_or(())?;
        let capability = if self.source.slot.cursor.line_backing.is_some() {
            self.source.slot.cursor.line_backing_capability
        } else {
            self.source.slot.cursor.backing_capability
        };
        if capability.is_none() {
            return Ok(None);
        }
        record_source_lex_slot_borrow();
        Ok(Some(BorrowedSourceCharacterRun {
            bytes,
            source: line.physical.source,
            byte_start: start,
            capability,
            fuel_limited: false,
        }))
    }

    /// Marks exactly the admitted prefix in both source cursors.
    #[inline(always)]
    pub(crate) fn commit_character_run(&mut self, count: usize) -> Result<(), ()> {
        let count = u32::try_from(count).map_err(|_| ())?;
        if count == 0 {
            return Ok(());
        }
        let line = self.source.slot.cursor.line.as_mut().ok_or(())?;
        line.cursor.byte_cursor = line
            .cursor
            .byte_cursor
            .checked_add(u64::from(count))
            .ok_or(())?;
        line.cursor.scalar_cursor = line
            .cursor
            .scalar_cursor
            .checked_add(u64::from(count))
            .ok_or(())?;
        line.cursor.lexer_state = super::LexerState::MidLine;
        self.source
            .frame
            .advance_resident_run(count)
            .map(|_| ())
            .ok_or(())
    }

    /// Lends the current line's ordinary single-byte prefix without entering
    /// the scalar tokenizer. The source line and packed frame remain the sole
    /// cursors and settle together once for the accepted prefix.
    #[inline(always)]
    pub(crate) fn advance_character_run<E>(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        mut consume: impl FnMut(
            &mut tex_state::CommandContext<'_, G>,
            char,
            tex_state::token::OriginId,
        ) -> Result<bool, E>,
    ) -> Result<ResidentSourceCharacterRun<E>, ()> {
        record_source_lex_slot_borrow();
        let Some(line) = self.source.slot.cursor.line.as_ref() else {
            return Ok(ResidentSourceCharacterRun::Unavailable);
        };
        let start = line.cursor.byte_cursor;
        if start >= line.retained_end {
            return Ok(ResidentSourceCharacterRun::Unavailable);
        }
        let mode = self.source.slot.cursor.current_backing().mode;
        let bytes = self.source.slot.cursor.current_backing().bytes.as_ref();
        let source = line.physical.source;
        let limit = line.retained_end;
        let start_index = usize::try_from(start).map_err(|_| ())?;
        let limit_index = usize::try_from(limit).map_err(|_| ())?;
        let run_bytes = bytes.get(start_index..limit_index).ok_or(())?;
        let mut count = 0_u32;
        let mut failure = None;
        for &byte in run_bytes {
            let offset = start.checked_add(u64::from(count)).ok_or(())?;
            let code = match mode {
                crate::CharacterMode::EightBitExact => crate::CharacterCode::from_byte(byte),
                crate::CharacterMode::UnicodeExtended if byte.is_ascii() => {
                    crate::CharacterCode::from(char::from(byte))
                }
                crate::CharacterMode::UnicodeExtended => break,
            };
            let ch = crate::profile::token_character(code);
            if !matches!(
                state.catcode(ch),
                tex_state::token::Catcode::Letter | tex_state::token::Catcode::Other
            ) {
                break;
            }
            let origin = state.source_token_origin(source, offset, offset.saturating_add(1));
            match consume(state, ch, origin) {
                Ok(keep_going) => {
                    count = count.checked_add(1).ok_or(())?;
                    if !keep_going {
                        break;
                    }
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        if count == 0 {
            return Ok(match failure {
                Some(error) => ResidentSourceCharacterRun::Failed { count, error },
                None => ResidentSourceCharacterRun::Unavailable,
            });
        }

        let line = self.source.slot.cursor.line.as_mut().ok_or(())?;
        line.cursor.byte_cursor = line
            .cursor
            .byte_cursor
            .checked_add(u64::from(count))
            .ok_or(())?;
        line.cursor.scalar_cursor = line
            .cursor
            .scalar_cursor
            .checked_add(u64::from(count))
            .ok_or(())?;
        line.cursor.lexer_state = super::LexerState::MidLine;
        self.source.frame.advance_resident_run(count).ok_or(())?;
        Ok(match failure {
            Some(error) => ResidentSourceCharacterRun::Failed { count, error },
            None => ResidentSourceCharacterRun::Consumed { count },
        })
    }

    #[inline(never)]
    pub(crate) fn advance(
        &mut self,
        profile: crate::CommandProfile,
        force_eof: bool,
        state: &mut tex_state::CommandContext<'_, G>,
        create_control_sequences: bool,
    ) -> Result<ResidentSourceAdvance, ()> {
        record_source_lex_slot_borrow();
        if state.tracked_region_is_active() {
            super::observe_immutable_source(state, self.source);
        }
        let identity = self.source.identity();
        let step = {
            let mut queries = super::stack::LiveSourceQueries {
                state,
                create_control_sequences,
            };
            match profile.character_mode() {
                crate::CharacterMode::EightBitExact => self
                    .source
                    .slot
                    .cursor
                    .next_compact_exact_byte_step(force_eof, &mut queries),
                crate::CharacterMode::UnicodeExtended => self
                    .source
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

/// Allocation-free identity of the one mutable input row that can affect a
/// newly captured TeX82 error context.
///
/// Rows below the semantic top are immutable while buried. Their source
/// context is copied into every descendant frame, so a generated-source
/// rebind is also visible in the top frame. `row_admissions` prevents a
/// push/pop ABA from making an older coordinate name the exposed row again;
/// it is monotonic across checkpoint installs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InputStackContextCoordinate {
    row_admissions: u64,
    source_owner_swaps: u64,
    top: DiagnosticInputTop,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum DiagnosticInputTop {
    Empty,
    Source {
        frame: super::PackedInputFrame,
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

/// One generation-tied input stack whose live rows are authoritative.
///
/// Rows cover sources and token lists. Each source row owns its variable
/// source state directly; every token-list row reads a span of `words`, and
/// a macro activation's argument spans live in `parameters` (TeX82's
/// `param_stack`). Pushing appends and popping truncates all three; a
/// checkpoint copies the stack instead of journaling it.
pub(crate) struct InputStack<G> {
    pub(crate) rows: Vec<InputLevel<G>>,
    /// Token words of every live token-list row, in push order.
    pub(crate) words: Vec<tex_state::token::TracedTokenWord>,
    /// Argument spans of every live macro activation, in push order.
    pub(crate) parameters: Vec<super::ArgumentSpan>,
    occupied_source_buffer_slots: usize,
    row_admissions: u64,
    source_owner_swaps: u64,
}

impl<G> Default for InputStack<G> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            words: Vec::new(),
            parameters: Vec::new(),
            occupied_source_buffer_slots: 0,
            row_admissions: 0,
            source_owner_swaps: 0,
        }
    }
}

impl<G: core::fmt::Debug> core::fmt::Debug for InputStack<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InputStack")
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

impl<G: PartialEq> PartialEq for InputStack<G> {
    fn eq(&self, other: &Self) -> bool {
        self.rows == other.rows && self.words == other.words && self.parameters == other.parameters
    }
}

impl<G: Eq> Eq for InputStack<G> {}

impl<G: Hash> Hash for InputStack<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.rows.hash(state);
        self.words.hash(state);
        self.parameters.hash(state);
    }
}

impl<G> Deref for InputStack<G> {
    type Target = [InputLevel<G>];

    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}

impl<G, I> Index<I> for InputStack<G>
where
    I: SliceIndex<[InputLevel<G>]>,
{
    type Output = I::Output;

    fn index(&self, index: I) -> &Self::Output {
        &self.rows[index]
    }
}

impl<'a, G> IntoIterator for &'a InputStack<G> {
    type Item = &'a InputLevel<G>;
    type IntoIter = core::slice::Iter<'a, InputLevel<G>>;

    fn into_iter(self) -> Self::IntoIter {
        self.rows.iter()
    }
}

impl<G> InputStack<G> {
    /// Copies a quiescent stack for a checkpoint capture.
    ///
    /// Quiescence excludes macro bodies and arguments, so the argument-span
    /// stack is empty and every token row is a plain list.
    pub(crate) fn checkpoint_copy(&self) -> Self {
        Self {
            rows: self.rows.iter().map(InputLevel::checkpoint_copy).collect(),
            words: self.words.clone(),
            parameters: self.parameters.clone(),
            occupied_source_buffer_slots: self.occupied_source_buffer_slots,
            row_admissions: 0,
            source_owner_swaps: 0,
        }
    }

    /// Replaces the live rows with a copy of a captured stack. Admission and
    /// owner-swap counters stay monotonic so no diagnostic coordinate taken
    /// before the install can name a row after it.
    pub(crate) fn install_checkpoint(&mut self, capture: &Self) {
        self.rows.clear();
        self.rows
            .extend(capture.rows.iter().map(InputLevel::checkpoint_copy));
        self.words.clone_from(&capture.words);
        self.parameters.clone_from(&capture.parameters);
        self.occupied_source_buffer_slots = capture.occupied_source_buffer_slots;
        self.row_admissions = self.row_admissions.saturating_add(1);
        self.source_owner_swaps = self.source_owner_swaps.saturating_add(1);
    }

    /// Whether any macro activation is live.
    pub(crate) fn has_macro_activation(&self) -> bool {
        self.rows.iter().any(|level| {
            matches!(
                level,
                super::InputLevel::Resident(super::ResidentTokenRow {
                    storage: super::ResidentTokenStorage::MacroBody(_),
                    ..
                })
            )
        })
    }

    /// The words a token-list row delivers.
    pub(crate) fn row_words(
        &self,
        row: &super::ResidentTokenRow,
    ) -> &[tex_state::token::TracedTokenWord] {
        row.words(&self.words)
    }

    /// The innermost macro activation's argument words and replacement.
    #[cfg(test)]
    pub(crate) fn innermost_macro_activation(
        &self,
    ) -> Option<(
        Vec<&[tex_state::token::TracedTokenWord]>,
        &[tex_state::token::TracedTokenWord],
    )> {
        let (row, body) = self.rows.iter().rev().find_map(|level| match level {
            super::InputLevel::Resident(
                row @ super::ResidentTokenRow {
                    storage: super::ResidentTokenStorage::MacroBody(body),
                    ..
                },
            ) => Some((row, body)),
            _ => None,
        })?;
        let arguments = self.parameters[body.parameter_base as usize..]
            .iter()
            .map(|span| &self.words[span.start as usize..(span.start + span.len) as usize])
            .collect();
        Some((arguments, self.row_words(row)))
    }

    /// The next undelivered word of token row `index`, or `None` when that
    /// row is exhausted or is a source level.
    pub(crate) fn pending_token_word(
        &self,
        index: usize,
    ) -> Option<tex_state::token::TracedTokenWord> {
        let super::InputLevel::Resident(row) = self.rows.get(index)? else {
            return None;
        };
        self.row_words(row).get(row.header.position()).copied()
    }

    /// The parameter text stored below a macro activation's replacement.
    pub(crate) fn macro_parameter_text(
        &self,
        row: &super::ResidentTokenRow,
    ) -> &[tex_state::token::TracedTokenWord] {
        let super::ResidentTokenStorage::MacroBody(body) = row.storage else {
            return &[];
        };
        let start = row.header.frame.start() as usize;
        &self.words[start - body.parameter_text_len as usize..start]
    }

    /// Advances the diagnostic-coordinate counters past `past`, so no
    /// coordinate captured from a sibling stack validates against this one.
    pub(crate) fn continue_context_epoch(&mut self, past: &Self) {
        self.row_admissions = self
            .row_admissions
            .max(past.row_admissions)
            .saturating_add(1);
        self.source_owner_swaps = self
            .source_owner_swaps
            .max(past.source_owner_swaps)
            .saturating_add(1);
    }

    pub(crate) fn diagnostic_context_coordinate(&self) -> InputStackContextCoordinate {
        let top = match self.rows.last() {
            None => DiagnosticInputTop::Empty,
            Some(InputLevel::Source(source)) => DiagnosticInputTop::Source {
                frame: source.frame,
                name_class: source.slot.name_class,
                cursor: DiagnosticSourceCursor::capture(&source.slot.cursor),
            },
            Some(InputLevel::Resident(row)) => match &row.storage {
                super::ResidentTokenStorage::MacroBody(_) => DiagnosticInputTop::MacroBody {
                    identity: row.header.identity(),
                    position: row.header.frame.position(),
                },
                super::ResidentTokenStorage::MacroArgument { .. } => {
                    DiagnosticInputTop::MacroArgument {
                        identity: row.header.identity(),
                        position: row.header.frame.position(),
                    }
                }
                _ => DiagnosticInputTop::Stored {
                    frame: row.header.frame,
                    retirement: row.header.retirement(),
                    position: row.header.frame.position(),
                },
            },
        };
        InputStackContextCoordinate {
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

    /// Finds the first editor-generated physical backing among the live
    /// source rows.
    pub(crate) fn editor_backing(&self, accepted: &[u8]) -> Option<&super::RegisteredSource> {
        self.rows.iter().find_map(|level| {
            let InputLevel::Source(source) = level else {
                return None;
            };
            source
                .slot
                .cursor
                .backing
                .is_editor_backing(accepted)
                .then_some(&source.slot.cursor.backing)
        })
    }

    /// Moves every source row reading the accepted editor revision onto
    /// `replacement`, remapping its offsets around the edited span.
    pub(crate) fn rehome_generated_source(
        &mut self,
        accepted: &[u8],
        replacement: &super::RegisteredSource,
        offsets: super::source::SourceOffsetMap,
    ) -> bool {
        let mut rebound = false;
        for level in &mut self.rows {
            let InputLevel::Source(source) = level else {
                continue;
            };
            let slot = &mut *source.slot;
            if !slot.cursor.backing.is_editor_backing(accepted) {
                continue;
            }
            slot.cursor.backing.clone_from(replacement);
            slot.cursor.backing_registered = false;
            slot.cursor.backing_capability = None;
            slot.cursor.rehome_offsets(offsets);
            slot.occupied_buffer_slots = super::source::occupied_source_buffer_slots(&slot.cursor);
            rebound = true;
        }
        if rebound {
            self.source_owner_swaps = self.source_owner_swaps.saturating_add(1);
            self.refresh_occupied_source_buffer_slots();
        }
        rebound
    }

    pub(crate) fn push_source(&mut self, frame: super::PackedInputFrame, slot: SourceSlot<G>) {
        self.push_row(InputLevel::Source(SourceLevel {
            frame,
            slot: Box::new(slot),
        }));
    }

    pub(crate) fn top_source(&self) -> Option<&SourceLevel<G>> {
        let InputLevel::Source(source) = self.rows.last()? else {
            return None;
        };
        record_source_owner_slot_lookup();
        Some(source)
    }

    pub(crate) fn physical_source(
        &self,
        id: tex_state::SourceId,
    ) -> Option<&super::RegisteredSource> {
        self.rows.iter().find_map(|level| {
            let InputLevel::Source(source) = level else {
                return None;
            };
            let backing = &source.slot.cursor.backing;
            (backing.id == id).then_some(backing)
        })
    }

    pub(crate) fn rebind_physical_source(
        &mut self,
        id: tex_state::SourceId,
        replacement: super::RegisteredSource,
    ) -> bool {
        let Some(index) = self.rows.iter().position(|level| {
            let InputLevel::Source(source) = level else {
                return false;
            };
            source.slot.cursor.backing.id == id
        }) else {
            return false;
        };
        let InputLevel::Source(source) = &mut self.rows[index] else {
            unreachable!("physical source lookup returned a token row")
        };
        let slot = &mut *source.slot;
        let prior_buffer_slots = slot.occupied_buffer_slots;
        let replacement_id = replacement.id;
        slot.cursor.backing = replacement;
        slot.cursor.backing_registered = false;
        slot.cursor.backing_capability = None;
        let current_buffer_slots = super::source::occupied_source_buffer_slots(&slot.cursor);
        slot.occupied_buffer_slots = current_buffer_slots;
        for level in &mut self.rows {
            let Some(source_context) = level.source_context() else {
                continue;
            };
            if source_context.source() != id {
                continue;
            }
            level.set_source_context(Some(tex_state::packed_input::SourceContext::new(
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
        self.rows.last().and_then(InputLevel::source_context)
    }

    /// Mutates only copy-small lexer execution state.
    ///
    /// The closure must not replace the loaded line or its backing, retained
    /// end, or normalized endline. Those cold owner changes must use
    /// [`Self::mutate_top_source`] so buffer occupancy is updated once.
    pub(crate) fn mutate_top_source_lex<R>(
        &mut self,
        mutate: impl FnOnce(&mut super::PackedInputFrame, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        let InputLevel::Source(source) = self.rows.last_mut()? else {
            return None;
        };
        record_source_lex_slot_borrow();
        Some(mutate(&mut source.frame, &mut source.slot))
    }
}

/// The result of one plain resident run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResidentRun {
    /// Words admitted and appended.
    pub(crate) consumed: u32,
    /// The run ended at an input transition rather than a refused word: its
    /// frame is exhausted, or the next macro-body word is an out-parameter.
    /// The caller may settle it in place and run again.
    pub(crate) at_transition: bool,
}

impl<G> crate::CommandState<G> {
    /// Consumes the ordinary prefix of the active macro-body or argument span
    /// directly into a macro-argument writer or an open definition.
    ///
    /// The prefix stops before every token that needs the canonical delivery
    /// machine, as decided by the caller's admission: outer or alignment-bound
    /// commands, parameter substitution, closing braces, or input exhaustion. The
    /// scanner therefore crosses the input/scratch boundary once for the
    /// whole admitted physical span rather than once per ordinary token.
    pub(crate) fn consume_plain_resident_run(
        &mut self,
        writer: &mut impl crate::execution_scratch::PlainRunSink<G>,
        fuel: &mut crate::fuel::CommandFuel,
        mut admission: crate::execution_scratch::ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<ResidentRun, crate::CommandError> {
        let InputStack { rows, words, .. } = &mut self.roots.input.levels;
        let Some(InputLevel::Resident(row)) = rows.last_mut() else {
            return Ok(ResidentRun::default());
        };
        let body = match row.storage {
            super::ResidentTokenStorage::MacroBody(_) => true,
            super::ResidentTokenStorage::MacroArgument { .. } => false,
            super::ResidentTokenStorage::List(_) => return Ok(ResidentRun::default()),
        };
        let frame = row.header.frame;
        if frame.position() >= frame.limit() {
            return Ok(ResidentRun {
                consumed: 0,
                at_transition: true,
            });
        }
        // A `\noexpand`-marked frame settles its control sequences through
        // ordinary delivery.
        admission = if frame.flags().contains(
            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
        ) {
            admission.characters_only()
        } else {
            admission
        };
        let available = fuel.remaining().min(u64::from(u32::MAX)) as usize;
        if available == 0 {
            fuel.charge()?;
            unreachable!("zero remaining fuel must fail")
        }
        let span = &words[frame.absolute_position()..frame.absolute_limit()];
        let bound = available.min(span.len());
        let count = span[..bound]
            .iter()
            .take_while(|word| admission.admits(word.token_word()))
            .count();
        // An out-parameter can only stand in a macro replacement.
        let parameter_next =
            body && count < bound && span[count].token_word().out_parameter_slot().is_some();
        if count == 0 {
            return Ok(ResidentRun {
                consumed: 0,
                at_transition: parameter_next,
            });
        }
        writer
            .append_span(&mut self.scratch, &span[..count])
            .map_err(|_| crate::CommandError::input_invariant())?;
        let consumed = count as u32;
        // The span append above succeeded in full, so the authoritative cursor
        // and fuel settle as one atomic scalar run.
        row.header
            .frame
            .advance_resident_run(consumed)
            .ok_or_else(crate::CommandError::input_invariant)?;
        let exhausted = row.header.frame.position() >= row.header.frame.limit();
        // TeX82 §347 counts the run's interior braces in `align_state`; no
        // alignment is active when the admission accepts braces.
        if admission.brace_delta() != 0 {
            self.roots.alignment.align_state += admission.brace_delta();
        }
        fuel.charge_run(consumed)?;
        #[cfg(feature = "profiling")]
        fuel.record_raw_run(
            true,
            if body {
                crate::fuel::RawDeliveryKind::StoredToken
            } else {
                crate::fuel::RawDeliveryKind::MacroArgument
            },
            consumed,
        );
        Ok(ResidentRun {
            consumed,
            at_transition: parameter_next || exhausted,
        })
    }

    /// TeX82 §359's `out_param` case: `begin_token_list(param_stack[
    /// param_start+cur_chr-1], parameter)`.
    #[inline(always)]
    pub(crate) fn push_resident_parameter_cursor(
        &mut self,
        slot: u8,
        span: Option<super::ArgumentSpan>,
        active_source: Option<tex_state::packed_input::SourceContext>,
        observer: &mut Option<&mut dyn CommandObserver>,
    ) -> Result<(), ()> {
        let span = span.ok_or(())?;
        let identity = super::InputLevelId(self.roots.input.next_level_identity);
        self.roots.input.next_level_identity = self.roots.input.next_level_identity.wrapping_add(1);
        self.stack_usage.input_stack = self
            .stack_usage
            .input_stack
            .max(self.roots.input.levels.len());
        let mut frame = super::packed_token_frame(
            identity,
            span.start,
            span.len as usize,
            &super::TokenBehavior::Parameter,
            super::RetirementBehavior::Pop,
            &super::ReplayTrace::MacroParameter { slot },
        );
        frame.set_source_context(active_source);
        self.roots
            .input
            .levels
            .push(InputLevel::Resident(super::ResidentTokenRow {
                header: super::TokenRowHeader::new(frame),
                storage: super::ResidentTokenStorage::MacroArgument { slot },
            }));
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

    #[inline(always)]
    pub(crate) fn finish_resident_exhaustion(
        &mut self,
        resident_index: usize,
        identity: super::InputLevelId,
        observer: &mut Option<&mut dyn CommandObserver>,
        immediate_write_retirement: &mut Option<super::InputLevelId>,
    ) -> Option<super::ResidentBoundary> {
        let retirement = self.retire_resident_ordinary_input(
            resident_index,
            observer,
            immediate_write_retirement,
        );
        if self.input.levels.len() == resident_index + 1 {
            return Some(super::ResidentBoundary::TokenExhausted { identity });
        }
        retirement.map(super::ResidentBoundary::ReplayCompleted)
    }

    pub(crate) fn settle_input_retirement(
        &mut self,
        retirement: super::InputRetirement,
        observer: &mut Option<&mut dyn CommandObserver>,
        immediate_write_retirement: &mut Option<super::InputLevelId>,
    ) -> Option<crate::CommandReplayEpisode> {
        self.settle_retirement(
            retirement.identity,
            retirement.action,
            Some(retirement.reason),
            retirement.name_class.zip(retirement.source),
            observer,
            immediate_write_retirement,
        )
    }

    #[inline(always)]
    pub(super) fn settle_resident_retirement(
        &mut self,
        identity: super::InputLevelId,
        action: super::InputRetirementAction,
        reason: Option<super::InputRetirementReason>,
        observer: &mut Option<&mut dyn CommandObserver>,
        immediate_write_retirement: &mut Option<super::InputLevelId>,
    ) -> Option<crate::CommandReplayEpisode> {
        debug_assert!(matches!(
            action,
            super::InputRetirementAction::TokenListPopped
                | super::InputRetirementAction::VTemplatePopped
        ));
        self.settle_retirement(
            identity,
            action,
            reason,
            None,
            observer,
            immediate_write_retirement,
        )
    }

    #[inline(always)]
    fn settle_retirement(
        &mut self,
        identity: super::InputLevelId,
        action: super::InputRetirementAction,
        retirement_reason: Option<super::InputRetirementReason>,
        source_context: Option<(super::SourceNameClass, tex_state::SourceId)>,
        observer: &mut Option<&mut dyn CommandObserver>,
        immediate_write_retirement: &mut Option<super::InputLevelId>,
    ) -> Option<crate::CommandReplayEpisode> {
        // Unobserved retirements may omit the reason, which only names the
        // observer's records; the write-retirement slot still clears.
        debug_assert!(observer.is_none() || retirement_reason.is_some());
        let write = *immediate_write_retirement == Some(identity);
        if write {
            *immediate_write_retirement = None;
        }
        if !matches!(action, super::InputRetirementAction::VTemplateRetained)
            && let Some(sink) = observer.as_deref_mut()
            && let Some(retirement_reason) = retirement_reason
        {
            let reason = if write {
                InputReason::Write
            } else {
                observed_retirement_reason(action, retirement_reason)
            };
            sink.committed(CommandObservation::Input(InputRecord {
                transition: if matches!(action, super::InputRetirementAction::TerminalStop) {
                    InputTransition::Stop
                } else {
                    InputTransition::Retire
                },
                reason,
                source_name: source_context.map(|(name, _)| name),
                source: source_context.map(|(_, source)| source),
                level: identity.0,
                position: 0,
            }));
        }
        if let Some(transition) = match retirement_reason {
            _ if matches!(action, super::InputRetirementAction::VTemplateRetained) => None,
            Some(super::InputRetirementReason::AlignmentUTemplate) => Some("u_template_retire"),
            Some(super::InputRetirementReason::AlignmentVTemplate) => Some("v_template_retire"),
            Some(super::InputRetirementReason::AlignmentOmitTemplate) => {
                Some("omit_template_retire")
            }
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
            action,
            super::InputRetirementAction::SourcePopped
                | super::InputRetirementAction::TokenListPopped
                | super::InputRetirementAction::VTemplatePopped
        );
        if popped {
            let previous_align_state = self.roots.alignment.align_state;
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
    pub(crate) fn retain_top_v_template(&mut self) -> bool {
        let Some(cursor) = self.rows.last_mut().and_then(InputLevel::stored_common_mut) else {
            return false;
        };
        cursor.set_retirement(super::RetirementBehavior::AwaitingVTemplateRetirement);
        cursor
            .frame
            .add_flags(tex_state::packed_input::InputFrameFlags::RETAIN_AT_END);
        true
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
        if let InputLevel::Source(source) = &value {
            self.occupied_source_buffer_slots = self
                .occupied_source_buffer_slots
                .saturating_add(source.slot.occupied_buffer_slots);
        }
        self.rows.push(value);
    }

    pub(crate) fn pop_project<R>(
        &mut self,
        project: impl FnOnce(&InputLevel<G>, Option<&SourceSlot<G>>) -> R,
    ) -> Option<R> {
        let retired = self.rows.pop()?;
        let source = match &retired {
            InputLevel::Source(source) => Some(&*source.slot),
            InputLevel::Resident(row) => {
                self.release_row_words(row);
                None
            }
        };
        let result = project(&retired, source);
        if let Some(source) = source {
            self.occupied_source_buffer_slots = self
                .occupied_source_buffer_slots
                .saturating_sub(source.occupied_buffer_slots);
        }
        Some(result)
    }

    /// Pops a non-source row already admitted as the resident semantic top.
    ///
    /// The caller carries `index` from the one top selection which found
    /// exhaustion, so this performs neither another top lookup nor the
    /// identity validation required by cold detached coordinates.
    #[inline(always)]
    pub(super) fn pop_resident(&mut self, index: usize) {
        debug_assert_eq!(index.checked_add(1), Some(self.rows.len()));
        if let Some(InputLevel::Resident(row)) = self.rows.pop() {
            self.release_row_words(&row);
        } else {
            unreachable!("resident coordinate selects a resident row");
        }
    }

    /// Releases the stack words and argument spans a popped row owned.
    /// Rows are LIFO, so everything above the row's own words is gone.
    #[inline(always)]
    fn release_row_words(&mut self, row: &super::ResidentTokenRow) {
        match row.storage {
            super::ResidentTokenStorage::List(_) => {
                self.words.truncate(row.header.frame.start() as usize);
            }
            super::ResidentTokenStorage::MacroBody(body) => {
                self.words.truncate(body.region_start as usize);
                self.parameters.truncate(body.parameter_base as usize);
            }
            super::ResidentTokenStorage::MacroArgument { .. } => {}
        }
    }

    pub(super) fn resident_at(&self, index: usize) -> &InputLevel<G> {
        debug_assert_eq!(index.checked_add(1), Some(self.rows.len()));
        &self.rows[index]
    }

    pub(crate) fn mutate_top_source_cursor<R>(
        &mut self,
        mutate: impl FnOnce(&mut super::PackedInputFrame, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        self.mutate_top_source(SourceOwnerTransition::Cursor, mutate)
    }

    pub(crate) fn mutate_top_source_every_eof<R>(
        &mut self,
        mutate: impl FnOnce(&mut super::PackedInputFrame, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        self.mutate_top_source(SourceOwnerTransition::EveryEof, mutate)
    }

    pub(crate) fn mutate_top_source_backing<R>(
        &mut self,
        replacement: super::RegisteredSource,
        mutate: impl FnOnce(&mut super::PackedInputFrame, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        self.mutate_top_source(SourceOwnerTransition::Backing(replacement), mutate)
    }

    /// Applies one cold source-owner transition to the top row and refreshes
    /// its contribution to TeX's shared buffer.
    fn mutate_top_source<R>(
        &mut self,
        transition: SourceOwnerTransition,
        mutate: impl FnOnce(&mut super::PackedInputFrame, &mut SourceSlot<G>) -> R,
    ) -> Option<R> {
        let InputLevel::Source(source) = self.rows.last_mut()? else {
            return None;
        };
        let slot = &mut *source.slot;
        let prior_buffer_slots = slot.occupied_buffer_slots;
        transition.apply(slot);
        let result = mutate(&mut source.frame, slot);
        let current_buffer_slots = super::source::occupied_source_buffer_slots(&slot.cursor);
        slot.occupied_buffer_slots = current_buffer_slots;
        self.replace_source_buffer_slots(prior_buffer_slots, current_buffer_slots);
        Some(result)
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
            .map_or(0, |source| source.slot.occupied_buffer_slots);
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
                self.words
                    .capacity()
                    .saturating_mul(std::mem::size_of::<tex_state::token::TracedTokenWord>()),
            )
            .saturating_add(
                self.parameters
                    .capacity()
                    .saturating_mul(std::mem::size_of::<super::ArgumentSpan>()),
            )
    }

    fn replace_source_buffer_slots(&mut self, prior: usize, current: usize) {
        self.occupied_source_buffer_slots = self
            .occupied_source_buffer_slots
            .saturating_sub(prior)
            .saturating_add(current);
    }

    fn refresh_occupied_source_buffer_slots(&mut self) {
        self.occupied_source_buffer_slots = self.rows.iter().fold(0, |total, level| {
            let InputLevel::Source(source) = level else {
                return total;
            };
            total.saturating_add(source.slot.occupied_buffer_slots)
        });
    }
}
