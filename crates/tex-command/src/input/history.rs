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

use super::{CompactSourceTokenizationStep, InputLevel, InputStack, InputStorage, SourceSlot};

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

impl<G> SourceSlot<G> {
    /// File position remembered by the §342 diagnostic observer. Terminal
    /// and `\read` levels have `name<=17`, so they leave its prior site alone.
    pub(crate) fn diagnostic_location(
        &self,
        location: super::SourceLocation,
    ) -> Option<super::DiagnosticLocation> {
        let line = self.cursor.line.as_ref()?;
        let source = match self.name_class {
            super::SourceNameClass::File | super::SourceNameClass::Scantokens(_) => {
                self.cursor.backing.id
            }
            super::SourceNameClass::Terminal | super::SourceNameClass::ReadStream(_) => {
                return None;
            }
        };
        Some(line.diagnostic_location(source, location))
    }

    pub(crate) fn current_diagnostic_location(&self) -> Option<super::DiagnosticLocation> {
        let line = self.cursor.line.as_ref()?;
        self.diagnostic_location(super::SourceLocation::new(
            line.physical.source,
            line.cursor.byte_cursor.saturating_sub(1),
        ))
    }

    #[inline(always)]
    pub(crate) fn force_eof(&self, requested: bool) -> bool {
        requested && self.name_class == super::SourceNameClass::File
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
        let Some(line) = self.cursor.line.as_ref() else {
            return Ok(None);
        };
        let start = line.cursor.byte_cursor;
        if start >= line.retained_end {
            return Ok(None);
        }
        let backing = self.cursor.current_backing();
        let start_index = usize::try_from(start).map_err(|_| ())?;
        let limit_index = usize::try_from(line.retained_end).map_err(|_| ())?;
        let bytes = backing.bytes.get(start_index..limit_index).ok_or(())?;
        let capability = if self.cursor.line_backing.is_some() {
            self.cursor.line_backing_capability
        } else {
            self.cursor.backing_capability
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
    pub(crate) fn commit_character_run(
        &mut self,
        frame: &mut super::PackedInputFrame,
        count: usize,
    ) -> Result<(), ()> {
        let count = u32::try_from(count).map_err(|_| ())?;
        if count == 0 {
            return Ok(());
        }
        let line = self.cursor.line.as_mut().ok_or(())?;
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
        frame.advance_resident_run(count).map(|_| ()).ok_or(())
    }

    /// Lends the current line's ordinary single-byte prefix without entering
    /// the scalar tokenizer. The source line and packed frame remain the sole
    /// cursors and settle together once for the accepted prefix.
    #[inline(always)]
    pub(crate) fn advance_character_run<E>(
        &mut self,
        frame: &mut super::PackedInputFrame,
        state: &mut tex_state::CommandContext<'_, G>,
        mut consume: impl FnMut(
            &mut tex_state::CommandContext<'_, G>,
            char,
            tex_state::token::OriginId,
        ) -> Result<bool, E>,
    ) -> Result<ResidentSourceCharacterRun<E>, ()> {
        record_source_lex_slot_borrow();
        let Some(line) = self.cursor.line.as_ref() else {
            return Ok(ResidentSourceCharacterRun::Unavailable);
        };
        let start = line.cursor.byte_cursor;
        if start >= line.retained_end {
            return Ok(ResidentSourceCharacterRun::Unavailable);
        }
        let mode = self.cursor.current_backing().mode;
        let bytes = self.cursor.current_backing().bytes.as_ref();
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

        let line = self.cursor.line.as_mut().ok_or(())?;
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
        frame.advance_resident_run(count).ok_or(())?;
        Ok(match failure {
            Some(error) => ResidentSourceCharacterRun::Failed { count, error },
            None => ResidentSourceCharacterRun::Consumed { count },
        })
    }

    #[inline(never)]
    pub(crate) fn advance(
        &mut self,
        frame: &mut super::PackedInputFrame,
        profile: crate::CommandProfile,
        force_eof: bool,
        state: &mut tex_state::CommandContext<'_, G>,
        create_control_sequences: bool,
    ) -> Result<ResidentSourceAdvance, ()> {
        record_source_lex_slot_borrow();
        if state.tracked_region_is_active() {
            super::observe_immutable_source(state, self);
        }
        let identity = super::InputLevelId(frame.identity());
        let step = {
            let mut queries = super::stack::LiveSourceQueries {
                state,
                create_control_sequences,
            };
            match profile.character_mode() {
                crate::CharacterMode::EightBitExact => self
                    .cursor
                    .next_compact_exact_byte_step(force_eof, &mut queries),
                crate::CharacterMode::UnicodeExtended => self
                    .cursor
                    .next_compact_unicode_step(force_eof, &mut queries),
            }
        };
        match step {
            CompactSourceTokenizationStep::Token(token) => {
                if frame.identity() != identity.0 || frame.advance().is_none() {
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
/// rebind is also visible in the top frame. The context epoch rejects owners
/// displaced by restore or rebind. The enclosing scalar coordinate uses the
/// existing input-level identity frontier to reject push/pop ABA.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InputStackContextCoordinate {
    context_epoch: u64,
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

impl<G> InputStack<G> {
    /// Copies a quiescent stack for a checkpoint capture.
    ///
    /// Quiescence excludes macro bodies and arguments, so the live parameter
    /// count is zero and every token row is a plain list.
    pub(crate) fn checkpoint_copy(&self) -> Self {
        debug_assert!(self.arguments.is_empty());
        debug_assert_eq!(self.parameter_count, 0);
        Self {
            current: self.current.as_ref().map(InputLevel::checkpoint_copy),
            suspended: self
                .suspended
                .iter()
                .map(InputLevel::checkpoint_copy)
                .collect(),
            words: self.words.clone(),
            arguments: super::ArgumentArena::default(),
            parameter_count: self.parameter_count,
            occupied_source_buffer_slots: self.occupied_source_buffer_slots,
            context_epoch: 0,
        }
    }

    /// Replaces the live rows with a copy of a captured stack. The cold context
    /// epoch advances so coordinates captured before restore cannot name its rows.
    pub(crate) fn install_checkpoint(&mut self, capture: &Self) {
        self.arguments.clear();
        self.current = None;
        self.suspended.clear();
        self.suspended
            .extend(capture.suspended.iter().map(InputLevel::checkpoint_copy));
        self.current = capture.current.as_ref().map(InputLevel::checkpoint_copy);
        self.words.clone_from(&capture.words);
        self.parameter_count = capture.parameter_count;
        self.occupied_source_buffer_slots = capture.occupied_source_buffer_slots;
        self.context_epoch = self.context_epoch.saturating_add(1);
    }

    /// Whether any macro activation is live.
    pub(crate) fn has_macro_activation(&self) -> bool {
        self.iter().any(|level| {
            matches!(
                level,
                super::InputLevel {
                    storage: super::InputStorage::MacroBody(_),
                    ..
                }
            )
        })
    }

    /// The words a token-list row delivers.
    pub(crate) fn row_words<'a>(
        &'a self,
        row: &'a super::InputLevel<G>,
    ) -> impl DoubleEndedIterator<Item = tex_state::token::TracedTokenWord>
    + ExactSizeIterator
    + Clone
    + 'a {
        let frame = row.frame;
        (0..frame.limit()).map(|index| {
            self.row_word_at(row, index)
                .expect("resident row span is valid")
        })
    }

    /// Reads one word from a resident row, preserving traced words and
    /// synthesizing the same unknown origin used by immutable-list admission.
    #[inline(always)]
    pub(crate) fn row_word_at(
        &self,
        row: &super::InputLevel<G>,
        index: u32,
    ) -> Option<tex_state::token::TracedTokenWord> {
        if index >= row.frame.limit() {
            return None;
        }
        match &row.storage {
            super::InputStorage::Source(_) => None,
            super::InputStorage::StoredList(list) => {
                Some(tex_state::token::TracedTokenWord::from_parts(
                    list.word_at(index as usize)?,
                    tex_state::token::OriginId::UNKNOWN,
                ))
            }
            super::InputStorage::MacroBody(body) => {
                Some(tex_state::token::TracedTokenWord::from_parts(
                    body.body.word(index as usize)?,
                    tex_state::token::OriginId::UNKNOWN,
                ))
            }
            super::InputStorage::MacroArgument { region, slot } => {
                self.arguments.words(*region, *slot - 1).get(index as usize)
            }
            super::InputStorage::List(_) => self
                .words
                .get(row.frame.start() as usize + index as usize)
                .copied(),
        }
    }

    pub(crate) fn link_aftergroup_word(
        &mut self,
        identity: super::InputLevelId,
        word: tex_state::token::TracedTokenWord,
    ) {
        let row @ super::InputLevel {
            storage: super::InputStorage::List(super::PackedTokenOwnership::BackedUp),
            ..
        } = self.current.as_mut().expect("aftergroup backup exists")
        else {
            unreachable!("aftergroup backup is token input")
        };
        assert_eq!(row.identity(), identity);
        assert_eq!(row.frame.absolute_limit(), self.words.len());
        self.words.push(word);
        row.frame.extend_undelivered_tokens(1);
    }

    pub(crate) fn finish_aftergroup_links(&mut self, identity: super::InputLevelId) {
        let row @ super::InputLevel {
            storage: super::InputStorage::List(super::PackedTokenOwnership::BackedUp),
            ..
        } = self.current.as_ref().expect("aftergroup backup exists")
        else {
            unreachable!("aftergroup backup is token input")
        };
        assert_eq!(row.identity(), identity);
        self.words[row.frame.start() as usize..row.frame.absolute_limit()].reverse();
    }

    pub(crate) fn append_macro_argument_word(
        &mut self,
        writer: &mut super::ArgumentCursor,
        word: tex_state::token::TracedTokenWord,
        delimiter: bool,
    ) -> Result<u32, crate::execution_scratch::ScratchError> {
        self.arguments
            .append_from_input(writer, word, self.current.as_ref(), delimiter)
    }

    /// The innermost macro activation's argument words and replacement.
    #[cfg(test)]
    pub(crate) fn innermost_macro_activation(
        &self,
    ) -> Option<(
        Vec<Vec<tex_state::token::TracedTokenWord>>,
        Vec<tex_state::token::TracedTokenWord>,
    )> {
        let (row, body) = self.iter().rev().find_map(|level| match level {
            row @ super::InputLevel {
                storage: super::InputStorage::MacroBody(body),
                ..
            } => Some((row, body)),
            _ => None,
        })?;
        let arguments = body
            .argument_values(&self.arguments)
            .iter()
            .enumerate()
            .map(|(slot, _)| {
                self.arguments
                    .words(body.argument_region.expect("matched region"), slot as u8)
                    .iter()
                    .collect()
            })
            .collect();
        Some((arguments, self.row_words(row).collect()))
    }

    /// The next undelivered word of token row `index`, or `None` when that
    /// row is exhausted or is a source level.
    pub(crate) fn pending_token_word(
        &self,
        index: usize,
    ) -> Option<tex_state::token::TracedTokenWord> {
        let row = self.level(index)?;
        self.row_word_at(row, row.frame.position())
    }

    /// The parameter text stored below a macro activation's replacement.
    pub(crate) fn macro_parameter_text<'a>(
        &'a self,
        row: &'a super::InputLevel<G>,
    ) -> impl DoubleEndedIterator<Item = tex_state::token::TracedTokenWord> + 'a {
        let body = match &row.storage {
            super::InputStorage::MacroBody(body) => Some(&body.body),
            _ => None,
        };
        (0..body.map_or(0, tex_state::ResidentMacroBody::parameter_len)).map(move |index| {
            let word = body
                .expect("macro parameter range has a body")
                .parameter_word(index)
                .expect("macro parameter text span is valid");
            tex_state::token::TracedTokenWord::from_parts(word, tex_state::token::OriginId::UNKNOWN)
        })
    }

    /// Advances the diagnostic context epoch past `past`, so no
    /// coordinate captured from a sibling stack validates against this one.
    pub(crate) fn continue_context_epoch(&mut self, past: &Self) {
        self.context_epoch = self.context_epoch.max(past.context_epoch).saturating_add(1);
    }

    pub(crate) fn diagnostic_context_coordinate(&self) -> InputStackContextCoordinate {
        let top = match self.current.as_ref() {
            None => DiagnosticInputTop::Empty,
            Some(
                source @ InputLevel {
                    storage: InputStorage::Source(_),
                    ..
                },
            ) => DiagnosticInputTop::Source {
                frame: source.frame,
                name_class: source.source_slot().expect("source input").name_class,
                cursor: DiagnosticSourceCursor::capture(
                    &source.source_slot().expect("source input").cursor,
                ),
            },
            Some(
                row @ InputLevel {
                    storage:
                        InputStorage::List(_)
                        | InputStorage::StoredList(_)
                        | InputStorage::MacroBody(_)
                        | InputStorage::MacroArgument { .. },
                    ..
                },
            ) => match &row.storage {
                super::InputStorage::MacroBody(_) => DiagnosticInputTop::MacroBody {
                    identity: row.identity(),
                    position: row.frame.position(),
                },
                super::InputStorage::MacroArgument { .. } => DiagnosticInputTop::MacroArgument {
                    identity: row.identity(),
                    position: row.frame.position(),
                },
                _ => DiagnosticInputTop::Stored {
                    frame: row.frame,
                    retirement: row.retirement(),
                    position: row.frame.position(),
                },
            },
        };
        InputStackContextCoordinate {
            context_epoch: self.context_epoch,
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
        self.iter().find_map(|level| {
            let source @ InputLevel {
                storage: InputStorage::Source(_),
                ..
            } = level
            else {
                return None;
            };
            source
                .source_slot()
                .expect("source input")
                .cursor
                .backing
                .is_editor_backing(accepted)
                .then_some(&source.source_slot().expect("source input").cursor.backing)
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
        for level in self.iter_mut() {
            let source @ InputLevel {
                storage: InputStorage::Source(_),
                ..
            } = level
            else {
                continue;
            };
            let InputStorage::Source(slot) = &mut source.storage else {
                unreachable!("source input")
            };
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
            self.context_epoch = self.context_epoch.saturating_add(1);
            self.refresh_occupied_source_buffer_slots();
        }
        rebound
    }

    pub(crate) fn push_source(&mut self, frame: super::PackedInputFrame, slot: SourceSlot<G>) {
        self.push_row(InputLevel {
            frame,
            storage: super::InputStorage::Source(Box::new(slot)),
        });
    }

    pub(crate) fn top_source(&self) -> Option<&InputLevel<G>> {
        let source @ InputLevel {
            storage: InputStorage::Source(_),
            ..
        } = self.current.as_ref()?
        else {
            return None;
        };
        record_source_owner_slot_lookup();
        Some(source)
    }

    pub(crate) fn physical_source(
        &self,
        id: tex_state::SourceId,
    ) -> Option<&super::RegisteredSource> {
        self.iter().find_map(|level| {
            let source @ InputLevel {
                storage: InputStorage::Source(_),
                ..
            } = level
            else {
                return None;
            };
            let backing = &source.source_slot().expect("source input").cursor.backing;
            (backing.id == id).then_some(backing)
        })
    }

    pub(crate) fn rebind_physical_source(
        &mut self,
        id: tex_state::SourceId,
        replacement: super::RegisteredSource,
    ) -> bool {
        let Some(index) = self.iter().position(|level| {
            let source @ InputLevel {
                storage: InputStorage::Source(_),
                ..
            } = level
            else {
                return false;
            };
            source
                .source_slot()
                .expect("source input")
                .cursor
                .backing
                .id
                == id
        }) else {
            return false;
        };
        let source @ InputLevel {
            storage: InputStorage::Source(_),
            ..
        } = self.level_mut(index).expect("admitted source coordinate")
        else {
            unreachable!("physical source lookup returned a token row")
        };
        let InputStorage::Source(slot) = &mut source.storage else {
            unreachable!("source input")
        };
        let prior_buffer_slots = slot.occupied_buffer_slots;
        let replacement_id = replacement.id;
        slot.cursor.backing = replacement;
        slot.cursor.backing_registered = false;
        slot.cursor.backing_capability = None;
        let current_buffer_slots = super::source::occupied_source_buffer_slots(&slot.cursor);
        slot.occupied_buffer_slots = current_buffer_slots;
        for level in self.iter_mut() {
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
        self.context_epoch = self.context_epoch.saturating_add(1);
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
        self.current.as_ref().and_then(InputLevel::source_context)
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
        let InputLevel {
            frame,
            storage: InputStorage::Source(slot),
        } = self.current.as_mut()?
        else {
            return None;
        };
        record_source_lex_slot_borrow();
        Some(mutate(frame, slot))
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
    /// scanner therefore appends into its final destination once for the
    /// whole admitted physical span rather than once per ordinary token.
    pub(crate) fn consume_plain_resident_run(
        &mut self,
        writer: &mut impl crate::execution_scratch::PlainRunSink<G>,
        fuel: &mut crate::fuel::CommandFuel,
        mut admission: crate::execution_scratch::ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<ResidentRun, crate::CommandError> {
        let InputStack {
            current, arguments, ..
        } = &mut self.roots.input.levels;
        let Some(row) = current.as_mut() else {
            return Ok(ResidentRun::default());
        };
        let frame = row.frame;
        if frame.position() >= frame.limit() {
            return Ok(ResidentRun {
                consumed: 0,
                at_transition: matches!(
                    row.storage,
                    super::InputStorage::MacroBody(_) | super::InputStorage::MacroArgument { .. }
                ),
            });
        }
        // Suppression restricts the same admission cursor in place.
        if frame.flags().contains(
            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
        ) {
            admission.restrict_to_characters();
        }
        let available = fuel.remaining().min(u64::from(u32::MAX)) as usize;
        let (count, parameter_next) = match &row.storage {
            InputStorage::Source(_) => return Ok(ResidentRun::default()),
            super::InputStorage::MacroBody(body) => {
                if available == 0 {
                    fuel.charge()?;
                    unreachable!("zero remaining fuel must fail");
                }
                let span = body.body.replacement_from(frame.position());
                let bound = available.min(span.len());
                let count = writer
                    .consume_token_span(
                        arguments,
                        &body.body,
                        frame.position() as usize..frame.position() as usize + bound,
                        &mut admission,
                    )
                    .map_err(|_| crate::CommandError::input_invariant())?;
                let parameter_next = count < bound && span[count].out_parameter_slot().is_some();
                (count, parameter_next)
            }
            super::InputStorage::MacroArgument { region, slot } => {
                if available == 0 {
                    fuel.charge()?;
                    unreachable!("zero remaining fuel must fail");
                }
                let start = frame.position() as usize;
                let bound = available.min((frame.limit() - frame.position()) as usize);
                let count = writer
                    .consume_region_span(
                        arguments,
                        *region,
                        *slot - 1,
                        start..start + bound,
                        &mut admission,
                    )
                    .map_err(|_| crate::CommandError::input_invariant())?;
                (count, false)
            }
            super::InputStorage::List(_) | super::InputStorage::StoredList(_) => {
                return Ok(ResidentRun::default());
            }
        };
        if count == 0 {
            return Ok(ResidentRun {
                consumed: 0,
                at_transition: parameter_next,
            });
        }
        let consumed = count as u32;
        // The span append above succeeded in full, so the authoritative cursor
        // and fuel settle as one atomic scalar run.
        row.frame
            .advance_resident_run(consumed)
            .ok_or_else(crate::CommandError::input_invariant)?;
        let exhausted = row.frame.position() >= row.frame.limit();
        // TeX82 §347 counts the run's interior braces in `align_state`; no
        // alignment is active when the admission accepts braces.
        if admission.brace_delta() != 0 {
            self.roots.alignment.align_state += admission.brace_delta();
        }
        fuel.charge_run(consumed)?;
        #[cfg(feature = "profiling")]
        fuel.record_raw_run(
            true,
            if matches!(row.storage, super::InputStorage::MacroBody(_)) {
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
        region: Option<super::ArgumentRegionId>,
        span: Option<u32>,
        active_source: Option<tex_state::packed_input::SourceContext>,
        observer: &mut Option<&mut dyn CommandObserver>,
    ) -> Result<(), ()> {
        let span = span.ok_or(())?;
        let region = region.ok_or(())?;
        let identity = super::InputLevelId(self.roots.input.next_level_identity);
        self.roots.input.next_level_identity = self.roots.input.next_level_identity.wrapping_add(1);
        self.stack_usage.input_stack = self
            .stack_usage
            .input_stack
            .max(self.roots.input.levels.len());
        let mut frame = super::packed_token_frame(
            identity,
            0,
            span as usize,
            &super::TokenBehavior::Parameter,
            super::RetirementBehavior::Pop,
            &super::ReplayTrace::MacroParameter { slot },
        );
        frame.set_source_context(active_source);
        self.roots.input.levels.push(super::InputLevel {
            frame,
            storage: super::InputStorage::MacroArgument { slot, region },
        });
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
        let Some(cursor) = self
            .current
            .as_mut()
            .filter(|row| !matches!(row.storage, InputStorage::Source(_)))
        else {
            return false;
        };
        cursor.set_retirement(super::RetirementBehavior::AwaitingVTemplateRetirement);
        cursor
            .frame
            .add_flags(tex_state::packed_input::InputFrameFlags::RETAIN_AT_END);
        true
    }

    #[inline(always)]
    pub(crate) fn push(&mut self, value: InputLevel<G>) {
        assert!(
            !matches!(
                value,
                InputLevel {
                    storage: InputStorage::Source(_),
                    ..
                }
            ),
            "source rows are admitted with their sole owner slot"
        );
        self.push_row(value);
    }

    #[inline(always)]
    fn push_row(&mut self, value: InputLevel<G>) {
        if let source @ InputLevel {
            storage: InputStorage::Source(_),
            ..
        } = &value
        {
            self.occupied_source_buffer_slots = self.occupied_source_buffer_slots.saturating_add(
                source
                    .source_slot()
                    .expect("source input")
                    .occupied_buffer_slots,
            );
        }
        if let Some(previous) = self.current.replace(value) {
            self.suspended.push(previous);
        }
    }

    pub(crate) fn pop_project<R>(
        &mut self,
        project: impl FnOnce(&InputLevel<G>, Option<&SourceSlot<G>>) -> R,
    ) -> Option<R> {
        let retired = self.current.take()?;
        self.current = self.suspended.pop();
        let source = match &retired {
            source @ InputLevel {
                storage: InputStorage::Source(_),
                ..
            } => Some(source.source_slot().expect("source input")),
            row => {
                Self::release_row_words(
                    row,
                    &mut self.words,
                    &mut self.arguments,
                    &mut self.parameter_count,
                );
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

    /// Releases the actual current resident owner, then restores its parent.
    /// Detached source coordinates do not participate in resident retirement.
    #[inline(never)]
    pub(super) fn pop_resident(&mut self) {
        if let Some(
            row @ InputLevel {
                storage:
                    InputStorage::List(_)
                    | InputStorage::StoredList(_)
                    | InputStorage::MacroBody(_)
                    | InputStorage::MacroArgument { .. },
                ..
            },
        ) = self.current.as_ref()
        {
            Self::release_row_words(
                row,
                &mut self.words,
                &mut self.arguments,
                &mut self.parameter_count,
            );
            // Keep the retiring payload alive through suffix release, then
            // drop it in place while restoring the suspended parent.
            self.current = self.suspended.pop();
        } else {
            unreachable!("current input is resident");
        }
    }

    /// Releases the ordinary suffix or final argument region a popped row owned.
    /// Parameter borrowers retire above their owning body. Capture is independent.
    #[inline(always)]
    fn release_row_words(
        row: &super::InputLevel<G>,
        words: &mut Vec<tex_state::token::TracedTokenWord>,
        arguments: &mut super::ArgumentArena<G>,
        parameter_count: &mut usize,
    ) {
        match &row.storage {
            InputStorage::Source(_) => {}
            super::InputStorage::List(_) => {
                words.truncate(row.frame.start() as usize);
            }
            super::InputStorage::StoredList(_) => {}
            super::InputStorage::MacroBody(body) => {
                if let Some(region) = body.argument_region {
                    *parameter_count -= arguments.argument_count(region);
                    arguments.release(region);
                }
            }
            super::InputStorage::MacroArgument { .. } => {}
        }
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
        let source @ InputLevel {
            storage: InputStorage::Source(_),
            ..
        } = self.current.as_mut()?
        else {
            return None;
        };
        let InputStorage::Source(slot) = &mut source.storage else {
            unreachable!("source input")
        };
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
        let top = self.top_source().map_or(0, |source| {
            source
                .source_slot()
                .expect("source input")
                .occupied_buffer_slots
        });
        self.occupied_source_buffer_slots.saturating_sub(top)
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.suspended
                    .capacity()
                    .saturating_mul(std::mem::size_of::<InputLevel<G>>()),
            )
            .saturating_add(
                self.words
                    .capacity()
                    .saturating_mul(std::mem::size_of::<tex_state::token::TracedTokenWord>()),
            )
            .saturating_add(self.arguments.retained_bytes())
    }

    fn replace_source_buffer_slots(&mut self, prior: usize, current: usize) {
        self.occupied_source_buffer_slots = self
            .occupied_source_buffer_slots
            .saturating_sub(prior)
            .saturating_add(current);
    }

    fn refresh_occupied_source_buffer_slots(&mut self) {
        self.occupied_source_buffer_slots = self.iter().fold(0, |total, level| {
            let source @ InputLevel {
                storage: InputStorage::Source(_),
                ..
            } = level
            else {
                return total;
            };
            total.saturating_add(
                source
                    .source_slot()
                    .expect("source input")
                    .occupied_buffer_slots,
            )
        });
    }
}
