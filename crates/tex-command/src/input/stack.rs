//! Canonical input-stack replay and retirement mechanics.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

use crate::CommandState;
use tex_state::DefinitionId;
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use crate::macro_call::{MacroActivationId, MacroArguments};

use super::{
    CompactSourceStepQueries, CompactSourceTokenizationStep, InputLevel, InputLevelId,
    PackedTokenSources, PackedTokenSpanHandle, ReplayTrace, RetirementBehavior,
    SourceControlSequenceKind, SourceNameClass, SourceToken, StoredReplayReason, TokenBehavior,
    TokenCursor,
};

/// Result of one admission of the current input top.
pub(crate) enum InputTopTransition {
    Delivered,
    ParameterPushed(InputLevelId),
    InvalidCharacter,
    NeedLine(InputLevelId),
    SourceExhausted(InputLevelId),
    TokenExhausted(InputLevelId),
    Empty,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceRegistrationCounters {
    pub(crate) checks: u64,
    pub(crate) calls: u64,
}

#[cfg(test)]
thread_local! {
    static SOURCE_REGISTRATION_COUNTERS: std::cell::Cell<SourceRegistrationCounters> =
        const { std::cell::Cell::new(SourceRegistrationCounters { checks: 0, calls: 0 }) };
}

#[cfg(test)]
pub(crate) fn source_registration_counters() -> SourceRegistrationCounters {
    SOURCE_REGISTRATION_COUNTERS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn record_source_registration_check() {
    SOURCE_REGISTRATION_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        counters.checks += 1;
        slot.set(counters);
    });
}

#[cfg(test)]
fn record_source_registration_call() {
    SOURCE_REGISTRATION_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        counters.calls += 1;
        slot.set(counters);
    });
}

/// One committed input-lifecycle transition.
///
/// `trace` explains the replay that ended but does not select `action`.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct InputRetirement {
    pub(crate) identity: InputLevelId,
    pub(crate) action: InputRetirementAction,
    pub(crate) reason: InputRetirementReason,
    /// tex.web §303's `name` classification of the level that ended, present
    /// exactly when a source level ended. §329's `end_file_reading` is the
    /// only retirement that consults it (`if name>17 then a_close`), and
    /// §307's token-list levels have no `name` classification at all.
    pub(crate) name_class: Option<SourceNameClass>,
    pub(crate) source: Option<tex_state::SourceId>,
    pub(crate) trace: Option<ReplayTrace>,
    /// Copy-only result of comparing the still-borrowed source ancestry with
    /// current group and conditional stacks before the row is popped.
    pub(crate) file_warning_boundary: Option<FileWarningBoundary>,
    /// Whether §362 must print this source retirement's bare `)` now.
    pub(crate) closes_file_frame: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FileWarningBoundary {
    pub(crate) group_start: u32,
    pub(crate) condition_start: u32,
}

enum RetiredInputLevel<G> {
    Source {
        identity: InputLevelId,
        name_class: SourceNameClass,
        source: tex_state::SourceId,
        retirement: super::SourceRetirement,
        framed: bool,
    },
    Tokens {
        identity: InputLevelId,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
        replay: Option<super::ReplayPayloadId<G>>,
    },
}

impl<G> RetiredInputLevel<G> {
    fn borrowed(level: &InputLevel<G>) -> Self {
        match level {
            InputLevel::Source(source) => Self::Source {
                identity: source.identity(),
                name_class: source.name_class,
                source: source.slot.cursor.current_backing().id,
                retirement: source.retirement,
                framed: source_level_is_framed(source),
            },
            InputLevel::Tokens(cursor) => Self::Tokens {
                identity: cursor.identity(),
                behavior: cursor.behavior,
                retirement: cursor.retirement,
                trace: cursor.trace.clone(),
                replay: match &cursor.span {
                    PackedTokenSpanHandle::Replay { replay, .. } => Some(*replay),
                    _ => None,
                },
            },
            InputLevel::MacroArgument(cursor) => Self::Tokens {
                identity: cursor.identity(),
                behavior: TokenBehavior::Parameter,
                retirement: RetirementBehavior::Pop,
                trace: ReplayTrace::MacroParameter { slot: cursor.slot },
                replay: None,
            },
        }
    }

    fn owned(level: InputLevel<G>) -> Self {
        match level {
            InputLevel::Source(source) => {
                let framed = source_level_is_framed(&source);
                Self::Source {
                    identity: source.identity(),
                    name_class: source.name_class,
                    source: source.slot.cursor.current_backing().id,
                    retirement: source.retirement,
                    framed,
                }
            }
            InputLevel::Tokens(cursor) => Self::Tokens {
                identity: cursor.identity(),
                behavior: cursor.behavior,
                retirement: cursor.retirement,
                trace: cursor.trace,
                replay: match cursor.span {
                    PackedTokenSpanHandle::Replay { replay, .. } => Some(replay),
                    _ => None,
                },
            },
            InputLevel::MacroArgument(cursor) => Self::Tokens {
                identity: cursor.identity(),
                behavior: TokenBehavior::Parameter,
                retirement: RetirementBehavior::Pop,
                trace: ReplayTrace::MacroParameter { slot: cursor.slot },
                replay: None,
            },
        }
    }
}

/// Observer-visible class of an exhausted input level.
///
/// It is derived only after retirement has selected its canonical action, so
/// replay explanation cannot influence input semantics.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum InputRetirementReason {
    Source,
    Backup,
    Macro,
    Parameter,
    AlignmentUTemplate,
    AlignmentVTemplate,
    /// tex.web §789's constant `omit_template`, installed in place of a
    /// column's ⟨v_j⟩ part. It is still `token_type=v_template` (§307); only
    /// the list differs, which is exactly how `end_token_list` tells the two
    /// apart when it names the level.
    AlignmentOmitTemplate,
    Recovery,
    /// A stored token list, carrying which one it is: tex.web names an input
    /// level by its §307 `token_type`, so a retirement that reported only
    /// "some token list" could not be named at the observation boundary.
    TokenList(StoredReplayReason),
}

/// Canonical effect of exhausting or explicitly retiring one input level.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum InputRetirementAction {
    SourcePopped,
    TokenListPopped,
    TerminalStop,
    /// tex.web §360's `\read` pseudo-file: the level's one line has ended,
    /// which `get_next` reports as `cur_cmd:=cur_chr:=0` rather than by
    /// resuming the enclosing level.
    ReadLineEnded,
    VTemplateRetained,
    VTemplatePopped,
}

/// Why an exact input level could not be retired.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum InputRetirementError {
    NoInput,
    LevelChanged {
        expected: InputLevelId,
        actual: InputLevelId,
    },
    MacroActivationOrder {
        expected: MacroActivationId,
        actual: Option<MacroActivationId>,
    },
    AttemptRootInvariant,
    NotRetainedVTemplate,
}

/// Result of applying canonical `OutParameter` handling to one delivery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum OutParameterReplay {
    /// Parameter input is literal and therefore cannot substitute itself.
    Literal,
    /// A range owned by the matching macro activation was pushed.
    Pushed(InputLevelId),
}

/// Why an `OutParameter` could not resolve through the live `param_start`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ParameterReplayError {
    NoInput,
    LevelChanged {
        expected: InputLevelId,
        actual: InputLevelId,
    },
    InvalidSlot(u8),
    NoMacroOwner,
    MissingArgument {
        slot: u8,
    },
    ArgumentRangeOutsideBuffer {
        slot: u8,
    },
}

impl<G> CommandState<G> {
    /// Advances the exact top input row once and writes the caller's final
    /// command value in place. Physical-line acquisition is deliberately a
    /// separate transition: a cursor without a loaded line returns
    /// [`InputTopTransition::NeedLine`] without loading or journaling one.
    pub(crate) fn transition_input_top_into(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        create_control_sequences: bool,
        destination: &mut crate::CurrentCommand<G>,
    ) -> Result<InputTopTransition, ()> {
        let profile = self.profile();
        let force_eof = self.source_force_eof();
        let (delivered, delivered_by_parameter, admitted_identity) = {
            let attempt = self.attempt.arena();
            let scratch = &self.scratch;
            let roots = &mut self.roots;
            let replay_lane = &roots.input.replay;
            let Some(level) = roots.input.levels.last_mut() else {
                return Ok(InputTopTransition::Empty);
            };
            match level {
                InputLevel::Source(source) => {
                    let identity = source.identity();
                    let position = source.slot.cursor.next_physical_offset;
                    if state.tracked_region_is_active() {
                        super::observe_immutable_source(state, source);
                    }
                    let mut queries = LiveSourceQueries {
                        state,
                        create_control_sequences,
                    };
                    let step = match profile.character_mode() {
                        crate::CharacterMode::EightBitExact => source
                            .slot
                            .cursor
                            .next_compact_exact_byte_step(force_eof, &mut queries),
                        crate::CharacterMode::UnicodeExtended => source
                            .slot
                            .cursor
                            .next_compact_unicode_step(force_eof, &mut queries),
                    };
                    match step {
                        CompactSourceTokenizationStep::Token(token) => {
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
                            let direct_source_line = source.slot.cursor.line.as_ref().map(|line| {
                                u32::try_from(line.physical.number()).unwrap_or(u32::MAX)
                            });
                            if source.frame.identity() != identity.0
                                || source.frame.advance().is_none()
                            {
                                return Err(());
                            }
                            destination.write_raw_delivery(
                                TracedTokenWord::from_parts(token.word, origin),
                                identity.0,
                                position,
                                Some(token.provenance),
                                true,
                                direct_source_line,
                                false,
                            );
                            (InputTopTransition::Delivered, false, Some(identity))
                        }
                        CompactSourceTokenizationStep::InvalidCharacter => {
                            (InputTopTransition::InvalidCharacter, false, Some(identity))
                        }
                        CompactSourceTokenizationStep::NeedLine => (
                            InputTopTransition::NeedLine(identity),
                            false,
                            Some(identity),
                        ),
                        CompactSourceTokenizationStep::End => (
                            InputTopTransition::SourceExhausted(identity),
                            false,
                            Some(identity),
                        ),
                    }
                }
                InputLevel::Tokens(cursor) => {
                    let identity = cursor.identity();
                    let delivered = cursor
                        .deliver_into(PackedTokenSources::new(replay_lane, attempt), destination)?;
                    if delivered {
                        (
                            InputTopTransition::Delivered,
                            matches!(cursor.behavior, TokenBehavior::Parameter),
                            Some(identity),
                        )
                    } else {
                        (
                            InputTopTransition::TokenExhausted(identity),
                            false,
                            Some(identity),
                        )
                    }
                }
                InputLevel::MacroArgument(cursor) => {
                    let identity = cursor.identity();
                    let delivered = cursor.deliver_into(scratch, destination)?;
                    if delivered {
                        (InputTopTransition::Delivered, true, Some(identity))
                    } else {
                        (
                            InputTopTransition::TokenExhausted(identity),
                            false,
                            Some(identity),
                        )
                    }
                }
            }
        };

        if !matches!(delivered, InputTopTransition::Delivered) {
            return Ok(delivered);
        }
        let Token::Param(slot) = destination.spelling().semantic_token() else {
            return Ok(InputTopTransition::Delivered);
        };
        let delivering_level = InputLevelId(destination.delivery_stamp().input_level());
        if admitted_identity != Some(delivering_level) {
            return Err(());
        }
        match self
            .replay_out_parameter_after_admission(delivering_level, delivered_by_parameter, slot)
            .map_err(|_| ())?
        {
            OutParameterReplay::Literal => Ok(InputTopTransition::Delivered),
            OutParameterReplay::Pushed(level) => Ok(InputTopTransition::ParameterPushed(level)),
        }
    }

    /// Acquires, firms, registers, and accounts for one physical line on the
    /// current source. This is the only production transition that computes
    /// lower-buffer occupancy or changes TeX's retained `line` scalar.
    pub(crate) fn acquire_input_top_line(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        create_control_sequences: bool,
        endlinechar: i32,
        firm: bool,
        pending_acquired_line: bool,
    ) -> Result<Option<super::PhysicalLine>, ()> {
        if let Some(InputLevel::Source(source)) = self.input.levels.last_mut() {
            register_pending_source_backings(state, &mut source.slot.cursor);
            if state.tracked_region_is_active() {
                super::observe_immutable_source(state, source);
            }
        }
        let physical = {
            let mut queries = LiveSourceQueries {
                state,
                create_control_sequences,
            };
            self.acquire_input_top_line_with_queries(
                endlinechar,
                firm,
                pending_acquired_line,
                &mut queries,
            )?
        };
        if let Some(InputLevel::Source(source)) = self.input.levels.last_mut() {
            register_pending_source_backings(state, &mut source.slot.cursor);
            if state.tracked_region_is_active() {
                super::observe_immutable_source(state, source);
            }
        }
        Ok(physical)
    }

    pub(crate) fn acquire_input_top_line_with_queries(
        &mut self,
        endlinechar: i32,
        firm: bool,
        pending_acquired_line: bool,
        queries: &mut dyn crate::SourceStepQueries,
    ) -> Result<Option<super::PhysicalLine>, ()> {
        let occupied_below_active =
            self.input
                .levels
                .iter()
                .rev()
                .skip(1)
                .fold(0_usize, |total, level| {
                    let Some((len, endline)) = crate::state::source_buffer_line(level) else {
                        return total;
                    };
                    total
                        .saturating_add(len)
                        .saturating_add(usize::from(endline))
                        .saturating_add(1)
                });
        let buffer_start = 1_usize
            .saturating_add(self.terminal_buffer_slots)
            .saturating_add(occupied_below_active);
        let profile = self.profile();
        let old_next_source_identity = self.input.next_source_identity;
        let (physical, retained_line) = {
            let usage = &mut self.stack_usage;
            let input = &mut self.roots.input;
            let Some(result) = input.levels.mutate_top_source(|level| {
                let InputLevel::Source(source) = level else {
                    unreachable!("physical acquisition requires a source top");
                };
                let identity = source.identity();
                let name_class = source.name_class;
                let stored = super::SourceLevelExecutionState::cursor(source);
                if pending_acquired_line {
                    source.slot.cursor.pending_acquired_line = true;
                }
                let mut lines = super::LineBackingRegistry {
                    profile,
                    next_identity: &mut input.next_source_identity,
                    usage,
                    buffer_start,
                    name_class: Some(name_class),
                };
                let result = source
                    .slot
                    .cursor
                    .load_next_line(endlinechar)
                    .map(|line| line.physical)
                    .map(|physical| {
                        lines.record_line_usage(&source.slot.cursor);
                        if firm {
                            source
                                .slot
                                .cursor
                                .firm_up_the_line(endlinechar, queries, &mut lines);
                            lines.record_line_usage(&source.slot.cursor);
                        }
                        let retained_line = match name_class {
                            SourceNameClass::File | SourceNameClass::Scantokens(_) => source
                                .slot
                                .cursor
                                .line
                                .as_ref()
                                .map(|line| line.physical.number().min(i32::MAX as u64) as i32)
                                .unwrap_or(0),
                            SourceNameClass::Terminal | SourceNameClass::ReadStream(_) => 0,
                        };
                        debug_assert_eq!(source.identity(), identity);
                        (physical, retained_line)
                    });
                (stored, result)
            }) else {
                return Err(());
            };
            let Some(result) = result else {
                return Ok(None);
            };
            result
        };
        if self.input.next_source_identity != old_next_source_identity {
            self.timeline
                .record_next_source_identity(old_next_source_identity);
        }
        self.set_retained_file_line_number(retained_line);
        Ok(Some(physical))
    }

    /// Commits provenance for an exhausted source before any observer can see
    /// its retirement and before the owning input row is popped.
    ///
    /// This cold seam covers sources which never reached production physical
    /// acquisition, including forced EOF. Registration failures remain
    /// diagnostic-only and retry on a later exhaustion transition while the
    /// source is still live.
    pub(crate) fn register_exhausted_source_backings(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        identity: InputLevelId,
    ) -> Result<(), ()> {
        let Some(InputLevel::Source(source)) = self.input.levels.last_mut() else {
            return Err(());
        };
        if source.identity() != identity {
            return Err(());
        }
        register_pending_source_backings(state, &mut source.slot.cursor);
        Ok(())
    }

    pub(crate) fn set_retained_file_line_number(&mut self, line: i32) {
        if self.input.retained_file_line_number == line {
            return;
        }
        self.timeline
            .record_retained_file_line_number(self.input.retained_file_line_number);
        self.input.retained_file_line_number = line;
    }

    fn pop_retired_input_level(&mut self) -> Option<RetiredInputLevel<G>> {
        if !self.input.levels.records_history() {
            return self.input.levels.pop_owned().map(RetiredInputLevel::owned);
        }
        self.input.levels.pop_project(RetiredInputLevel::borrowed)
    }

    /// TeX82 one-word nodes owned by live command input and argument buffers.
    ///
    /// An execution operation that allocates recursively must compose its
    /// peak with these owners before the command stack can retire them.
    #[must_use]
    pub fn transient_dynamic_words(&self) -> usize {
        let arguments = self.scratch.argument_word_len();
        self.input.levels.iter().fold(arguments, |words, level| {
            let InputLevel::Tokens(cursor) = level else {
                return words;
            };
            let owned = match cursor.span {
                PackedTokenSpanHandle::Replay { replay, len }
                    if matches!(
                        self.input.replay.ownership(replay),
                        Some(
                            super::PackedTokenOwnership::Transient
                                | super::PackedTokenOwnership::BackedUp
                        )
                    ) =>
                {
                    len as usize
                }
                _ => 0,
            };
            words.saturating_add(owned)
        })
    }

    pub(crate) fn top_input_level_identity(&self) -> Option<InputLevelId> {
        self.input.levels.last().map(input_level_identity)
    }

    /// Installs one complete macro activation and its replacement-body level.
    ///
    /// The canonical replacement list remains in immutable storage. Only the
    /// already matched arguments are transient, and they have one activation
    /// owner before this level can become visible to delivery.
    pub(crate) fn push_macro_activation(
        &mut self,
        name: tex_state::interner::Symbol,
        definition: DefinitionId<G>,
        arguments: MacroArguments<G>,
        invocation: OriginId,
        replacement_len: usize,
    ) -> InputLevelId {
        let parameter_count = self
            .scratch
            .argument_count(arguments.frame())
            .expect("live macro argument record");
        let parameter_ptr = self
            .parameters
            .activations
            .iter()
            .map(|activation| {
                self.scratch
                    .argument_count(activation.arguments.frame())
                    .expect("live macro argument record")
            })
            .sum::<usize>()
            .saturating_add(parameter_count);
        self.stack_usage.record_parameter_push(parameter_ptr);
        let activation =
            self.parameters
                .push_activation(name, definition.clone(), arguments, invocation);
        self.push_token_level_with_macro_lineage(
            PackedTokenSpanHandle::MacroReplacement {
                definition,
                len: u32::try_from(replacement_len).expect("macro replacement exceeds u32"),
            },
            TokenBehavior::MacroBody(activation),
            RetirementBehavior::Pop,
            ReplayTrace::MacroReplacement,
            true,
        )
    }

    pub(crate) fn push_token_level<P: super::PackedTokenSpanSource<G>>(
        &mut self,
        source: P,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
    ) -> InputLevelId {
        let has_macro_lineage = match self.input.levels.last() {
            Some(InputLevel::Tokens(cursor)) => cursor
                .frame
                .flags()
                .contains(tex_state::packed_input::InputFrameFlags::HAS_MACRO_LINEAGE),
            Some(InputLevel::MacroArgument(_)) => true,
            Some(InputLevel::Source(_)) | None => false,
        };
        self.push_token_level_with_macro_lineage(
            source,
            behavior,
            retirement,
            trace,
            has_macro_lineage,
        )
    }

    fn push_token_level_with_macro_lineage<P: super::PackedTokenSpanSource<G>>(
        &mut self,
        source: P,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
        has_macro_lineage: bool,
    ) -> InputLevelId {
        let span = source
            .admit(&mut self.input.replay)
            .expect("generation replay lane admission");
        let identity = self.allocate_input_level_identity();
        let mut frame =
            super::packed_token_frame(identity, span.frame_len(), &behavior, retirement, &trace);
        if has_macro_lineage {
            frame.add_flags(tex_state::packed_input::InputFrameFlags::HAS_MACRO_LINEAGE);
        }
        self.push_input_level(InputLevel::Tokens(TokenCursor {
            span,
            behavior,
            retirement,
            trace,
            frame,
        }));
        identity
    }

    /// Pushes one attempt-local list owned by its enclosing attempt scope.
    ///
    /// The arena's scope stack, rather than this input level, owns storage.
    /// Exact-LIFO macro/scanner retirement therefore keeps this coordinate
    /// live without a copied high-water mark or an input-stack census.
    pub(crate) fn push_attempt_list_level(
        &mut self,
        list: crate::attempt::AttemptTokenListId,
        len: u32,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
    ) -> Result<InputLevelId, crate::AttemptError> {
        self.attempt.arena().token_words(list)?;
        Ok(self.push_token_level(
            PackedTokenSpanHandle::AttemptList { list, len },
            behavior,
            retirement,
            trace,
        ))
    }

    /// Applies TeX's `param_start` ownership rule to one `OutParameter`.
    ///
    /// The owner is the nearest macro-body level at or below the delivering
    /// level, not merely the newest activation record. Parameter replay is
    /// literal, so an `OutParameter` delivered by a parameter level is not
    /// recursively substituted. This is the typed ownership counterpart of
    /// TeX.web §§357 and 359 (pdfTeX.web §§379 and 381).
    pub(crate) fn replay_out_parameter(
        &mut self,
        delivering_level: InputLevelId,
        slot: u8,
    ) -> Result<OutParameterReplay, ParameterReplayError> {
        let (actual, delivered_by_parameter) = match self.input.levels.last() {
            Some(level) => (
                input_level_identity(level),
                matches!(
                    level,
                    InputLevel::Tokens(TokenCursor {
                        behavior: TokenBehavior::Parameter,
                        ..
                    })
                ),
            ),
            None => return Err(ParameterReplayError::NoInput),
        };
        if actual != delivering_level {
            return Err(ParameterReplayError::LevelChanged {
                expected: delivering_level,
                actual,
            });
        }
        self.replay_out_parameter_after_admission(delivering_level, delivered_by_parameter, slot)
    }

    fn replay_out_parameter_after_admission(
        &mut self,
        _delivering_level: InputLevelId,
        delivered_by_parameter: bool,
        slot: u8,
    ) -> Result<OutParameterReplay, ParameterReplayError> {
        if delivered_by_parameter {
            return Ok(OutParameterReplay::Literal);
        }
        if !(1..=9).contains(&slot) {
            return Err(ParameterReplayError::InvalidSlot(slot));
        }

        let has_macro_lineage = match self.input.levels.last() {
            Some(InputLevel::Tokens(cursor)) => cursor
                .frame
                .flags()
                .contains(tex_state::packed_input::InputFrameFlags::HAS_MACRO_LINEAGE),
            Some(InputLevel::Source(_)) | Some(InputLevel::MacroArgument(_)) | None => false,
        };
        if !has_macro_lineage {
            return Err(ParameterReplayError::NoMacroOwner);
        }
        let owner = self
            .scratch
            .active_macro_frame()
            .ok_or(ParameterReplayError::NoMacroOwner)?;
        let range = self
            .scratch
            .argument_range(owner, slot)
            .map_err(|_| ParameterReplayError::ArgumentRangeOutsideBuffer { slot })?
            .ok_or(ParameterReplayError::MissingArgument { slot })?;
        let identity = self.allocate_input_level_identity();
        let trace = ReplayTrace::MacroParameter { slot };
        let frame = super::packed_token_frame(
            identity,
            range.len() as usize,
            &TokenBehavior::Parameter,
            RetirementBehavior::Pop,
            &trace,
        );
        self.push_input_level(InputLevel::MacroArgument(super::MacroArgumentCursor {
            range,
            slot,
            frame,
        }));
        Ok(OutParameterReplay::Pushed(identity))
    }

    /// Commits the end-of-level action selected by the exhausted top level.
    ///
    /// Exact identity prevents a stale delivery from retiring a replacement
    /// level. All popped payloads drop here, so transient allocations live
    /// exactly as long as their last cursor/snapshot owner; stored payloads
    /// drop only their immutable store handles. Macro-body retirement removes
    /// precisely its `param_start` activation.
    ///
    /// The v-template split mirrors tex.web §§325/§390 and §1131. An exhausted
    /// v-part first reports its frozen `end_template` boundary and stays
    /// reachable, because §325's `back_input` and §390's `macro_call` write
    /// `while (state=token_list)and(loc=null)and(token_type<>v_template) do
    /// end_token_list` -- v-template is their sole exception -- and §1131's
    /// `do_endv` walks the stack expecting to find that frame still live.
    /// After that boundary the frame is an ordinary depleted token list:
    /// §1131 only inspects it, and §357's `else begin end_token_list; goto
    /// restart; end` pops it the next time `get_next` reaches it. That is
    /// why the retained frame pops here rather than at a `do_endv` call site:
    /// with a non-empty `\everycr`, §799's `begin_token_list(every_cr,
    /// every_cr_text)` buries it and it survives the whole `\noalign` body.
    pub(crate) fn retire_exhausted_input(
        &mut self,
        expected: InputLevelId,
    ) -> Result<InputRetirement, InputRetirementError> {
        self.retire_exhausted_input_with_file_warning(expected, None)
    }

    pub(crate) fn retire_exhausted_input_with_file_warning(
        &mut self,
        expected: InputLevelId,
        file_warning_boundary: Option<FileWarningBoundary>,
    ) -> Result<InputRetirement, InputRetirementError> {
        let level = self
            .input
            .levels
            .last()
            .ok_or(InputRetirementError::NoInput)?;
        let actual = input_level_identity(level);
        if actual != expected {
            return Err(InputRetirementError::LevelChanged { expected, actual });
        }

        if matches!(level, InputLevel::MacroArgument(_)) {
            let RetiredInputLevel::Tokens {
                behavior,
                trace,
                replay,
                ..
            } = self
                .pop_retired_input_level()
                .expect("the inspected macro-argument level remains live")
            else {
                unreachable!("macro-argument retirement is token retirement");
            };
            debug_assert!(replay.is_none());
            return Ok(InputRetirement {
                identity: expected,
                action: InputRetirementAction::TokenListPopped,
                reason: input_retirement_reason(&behavior, &trace),
                name_class: None,
                source: None,
                trace: Some(trace),
                file_warning_boundary: None,
                closes_file_frame: false,
            });
        }

        let InputLevel::Tokens(cursor) = level else {
            let RetiredInputLevel::Source {
                name_class,
                source: source_id,
                retirement,
                framed,
                ..
            } = self
                .pop_retired_input_level()
                .expect("the inspected top source remains live")
            else {
                unreachable!("the inspected top level was not a source cursor");
            };
            self.restore_retained_line_after_source_pop();
            let action = source_retirement_action(retirement);
            // TeX82 §362 tests and clears the process-global `force_eof`
            // only in §360's `name>17` (real-file) refill branch. §483's
            // `name<=17` read/terminal pseudo-sources retire without
            // consuming a pending forced EOF meant for their parent file.
            //
            // §362 also prints `)` for exactly that same `name>17` case.
            // `pop_input_level_at_end_of_job` deliberately does not mirror
            // it: its unconditional §1335 unwinding is the *other* closing
            // mechanism, `final_cleanup`'s `␣)` per still-open file, which
            // the engine renders from its own `open_parens` count rather than
            // from this queue.
            //
            // This exactly mirrors `push_source_level`'s open gate. Named
            // files and traced scantokens (numeric name 19) receive a close;
            // an unnamed File registration cannot manufacture an orphan.
            //
            // §362 clears the process-global `force_eof` for the same
            // `name>17` case, which is why both live under this gate.
            if name_class == SourceNameClass::File {
                self.timeline.record_force_eof(self.input.force_eof);
                self.input.force_eof = false;
            }
            return Ok(InputRetirement {
                identity: expected,
                action,
                reason: InputRetirementReason::Source,
                name_class: Some(name_class),
                source: Some(source_id),
                trace: None,
                file_warning_boundary,
                closes_file_frame: framed,
            });
        };
        if cursor.retirement == RetirementBehavior::RetainExhaustedVTemplate {
            if !matches!(cursor.behavior, TokenBehavior::VTemplate) {
                return Err(InputRetirementError::NotRetainedVTemplate);
            }
            let trace = cursor.trace.clone();
            let InputLevel::Tokens(cursor) = self
                .input
                .levels
                .last_mut()
                .expect("the inspected top level remains live")
            else {
                unreachable!("the inspected top level was a token cursor");
            };
            cursor.retirement = RetirementBehavior::AwaitingVTemplateRetirement;
            cursor
                .frame
                .add_flags(tex_state::packed_input::InputFrameFlags::RETAIN_AT_END);
            return Ok(InputRetirement {
                identity: expected,
                action: InputRetirementAction::VTemplateRetained,
                reason: input_retirement_reason(&cursor.behavior, &trace),
                name_class: None,
                source: None,
                trace: Some(trace),
                file_warning_boundary: None,
                closes_file_frame: false,
            });
        }

        self.validate_macro_body_retirement(&cursor.behavior)?;
        let RetiredInputLevel::Tokens {
            behavior,
            retirement,
            trace,
            replay,
            ..
        } = self
            .pop_retired_input_level()
            .expect("the inspected top level remains live")
        else {
            unreachable!("the inspected top level was a token cursor");
        };
        self.finish_macro_body_retirement(&behavior)
            .map_err(|_| InputRetirementError::AttemptRootInvariant)?;
        if let Some(replay) = replay {
            self.input
                .replay
                .release(replay)
                .map_err(|_| InputRetirementError::AttemptRootInvariant)?;
        }
        let action = match retirement {
            RetirementBehavior::Pop => InputRetirementAction::TokenListPopped,
            RetirementBehavior::StopAtEnd => InputRetirementAction::TerminalStop,
            // tex.web §357: a v-template that has already reported its frozen
            // `end_template` boundary is an ordinary depleted token list, so
            // the next `get_next` that reaches it runs `end_token_list`.
            RetirementBehavior::AwaitingVTemplateRetirement => {
                InputRetirementAction::VTemplatePopped
            }
            RetirementBehavior::RetainExhaustedVTemplate => {
                unreachable!("retained templates returned before popping")
            }
        };
        Ok(InputRetirement {
            identity: expected,
            action,
            reason: input_retirement_reason(&behavior, &trace),
            name_class: None,
            source: None,
            trace: Some(trace),
            file_warning_boundary: None,
            closes_file_frame: false,
        })
    }

    /// Pops one input level for TeX82 §1335's `final_cleanup` unwinding.
    ///
    /// §1335 runs `while input_ptr>0 do if state=token_list then
    /// end_token_list else end_file_reading` once §1054's `its_all_over` has
    /// returned true.  Both arms are unconditional pops: the job is over, no
    /// level will ever be read again, and none of the exhaustion-time rules
    /// [`Self::retire_exhausted_input`] enforces (exact expected identity,
    /// macro-activation order, v-template retention) applies.  Levels
    /// therefore unwind top-down here even when a macro body, an alignment
    /// template, or a partially consumed source is still live.
    pub(crate) fn pop_input_level_at_end_of_job(&mut self) -> Option<InputRetirement> {
        let level = self.pop_retired_input_level()?;
        let RetiredInputLevel::Tokens {
            identity,
            behavior,
            retirement,
            trace,
            replay,
        } = level
        else {
            let RetiredInputLevel::Source {
                identity,
                name_class,
                source,
                retirement,
                ..
            } = level
            else {
                unreachable!("the popped level was not a token cursor");
            };
            self.restore_retained_line_after_source_pop();
            let action = source_retirement_action(retirement);
            return Some(InputRetirement {
                identity,
                action,
                reason: InputRetirementReason::Source,
                name_class: Some(name_class),
                source: Some(source),
                trace: None,
                file_warning_boundary: None,
                closes_file_frame: false,
            });
        };
        self.finish_macro_body_retirement(&behavior)
            .expect("final cleanup runs inside one direct operation");
        if let Some(replay) = replay {
            self.input
                .replay
                .release(replay)
                .expect("final cleanup preserves replay LIFO order");
        }
        let action = match retirement {
            RetirementBehavior::StopAtEnd => InputRetirementAction::TerminalStop,
            RetirementBehavior::Pop
            | RetirementBehavior::RetainExhaustedVTemplate
            | RetirementBehavior::AwaitingVTemplateRetirement => {
                InputRetirementAction::TokenListPopped
            }
        };
        Some(InputRetirement {
            identity,
            action,
            reason: input_retirement_reason(&behavior, &trace),
            name_class: None,
            source: None,
            trace: Some(trace),
            file_warning_boundary: None,
            closes_file_frame: false,
        })
    }

    /// Commits the one canonical input-frame push transition.
    pub(crate) fn push_input_level(&mut self, level: InputLevel<G>) {
        // TeX82 §321 checks `input_ptr` before `push_input` increments it.
        self.stack_usage.input_stack = self.stack_usage.input_stack.max(self.input.levels.len());
        self.input.levels.push(level);
    }

    pub(crate) fn allocate_input_level_identity(&mut self) -> InputLevelId {
        let identity = InputLevelId(self.input.next_level_identity);
        self.timeline
            .record_next_input_level_identity(self.input.next_level_identity);
        self.input.next_level_identity = self.input.next_level_identity.wrapping_add(1);
        identity
    }

    fn validate_macro_body_retirement(
        &self,
        behavior: &TokenBehavior,
    ) -> Result<(), InputRetirementError> {
        let TokenBehavior::MacroBody(expected) = behavior else {
            return Ok(());
        };
        let actual = self.parameters.activations.last();
        if actual.map(|activation| activation.identity) == Some(*expected) {
            Ok(())
        } else {
            Err(InputRetirementError::MacroActivationOrder {
                expected: *expected,
                actual: actual.map(|activation| activation.identity),
            })
        }
    }

    fn finish_macro_body_retirement(
        &mut self,
        behavior: &TokenBehavior,
    ) -> Result<(), crate::execution_scratch::ScratchError> {
        if matches!(behavior, TokenBehavior::MacroBody(_))
            && let Some(arguments) = self.parameters.retire_last_activation()
        {
            self.scratch.pop_macro_frame(arguments.frame())?;
        }
        Ok(())
    }

    fn restore_retained_line_after_source_pop(&mut self) {
        let line = self
            .input
            .levels
            .iter()
            .rev()
            .find_map(|level| match level {
                InputLevel::Tokens(_) | InputLevel::MacroArgument(_) => None,
                InputLevel::Source(source) => Some(match source.name_class {
                    SourceNameClass::File | SourceNameClass::Scantokens(_) => source
                        .slot
                        .cursor
                        .line
                        .as_ref()
                        .map_or_else(
                            || source.slot.cursor.next_line_number.saturating_sub(1),
                            |line| line.physical.number(),
                        )
                        .min(i32::MAX as u64)
                        as i32,
                    SourceNameClass::Terminal | SourceNameClass::ReadStream(_) => 0,
                }),
            })
            .unwrap_or(0);
        self.set_retained_file_line_number(line);
    }
}

fn input_retirement_reason(behavior: &TokenBehavior, trace: &ReplayTrace) -> InputRetirementReason {
    match behavior {
        TokenBehavior::UTemplate => return InputRetirementReason::AlignmentUTemplate,
        TokenBehavior::VTemplate => {
            return if matches!(trace, ReplayTrace::OmitTemplate) {
                InputRetirementReason::AlignmentOmitTemplate
            } else {
                InputRetirementReason::AlignmentVTemplate
            };
        }
        TokenBehavior::Ordinary
        | TokenBehavior::Recovery
        | TokenBehavior::MacroBody(_)
        | TokenBehavior::Parameter
        | TokenBehavior::BackedUp(_) => {}
    }
    match trace {
        ReplayTrace::BackedUp => InputRetirementReason::Backup,
        ReplayTrace::MacroReplacement => InputRetirementReason::Macro,
        ReplayTrace::MacroParameter { .. } => InputRetirementReason::Parameter,
        ReplayTrace::UTemplate | ReplayTrace::VTemplate | ReplayTrace::OmitTemplate => {
            unreachable!("alignment template behavior must accompany its replay trace")
        }
        ReplayTrace::Inserted | ReplayTrace::Transient(_) => InputRetirementReason::Recovery,
        ReplayTrace::Stored(reason) => InputRetirementReason::TokenList(*reason),
    }
}

fn source_level_is_framed<G>(source: &super::SourceLevel<G>) -> bool {
    match source.name_class {
        SourceNameClass::File => {
            source.slot.cursor.backing.name.is_some()
                && source.slot.cursor.backing.framing == crate::SourceFramingPolicy::Canonical
        }
        SourceNameClass::Scantokens(19) => true,
        SourceNameClass::Terminal
        | SourceNameClass::ReadStream(_)
        | SourceNameClass::Scantokens(_) => false,
    }
}

/// Registers every immutable backing currently owned by one source cursor.
///
/// Source-map provenance is diagnostic state, so registration failure does
/// not change TeX delivery. The cursor bit records the aggregate commit rather
/// than the attempt: a failed registration remains retryable, while the warm
/// path neither clones a descriptor nor allocates.
fn register_pending_source_backings<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    cursor: &mut super::SourceCursor,
) {
    #[cfg(test)]
    record_source_registration_check();
    if !cursor.backing_registered {
        #[cfg(test)]
        record_source_registration_call();
        if state
            .register_source(cursor.backing.id, cursor.backing.source_descriptor())
            .is_ok()
        {
            cursor.backing_registered = true;
        }
    }
    #[cfg(test)]
    record_source_registration_check();
    if !cursor.line_backing_registered
        && let Some(backing) = cursor.line_backing.as_ref()
    {
        #[cfg(test)]
        record_source_registration_call();
        if state
            .register_source(backing.id, backing.source_descriptor())
            .is_ok()
        {
            cursor.line_backing_registered = true;
        }
    }
}

/// Live engine queries used while the one admitted source row is borrowed.
struct LiveSourceQueries<'a, 'b, G> {
    state: &'a mut tex_state::CommandContext<'b, G>,
    create_control_sequences: bool,
}

impl<G> crate::SourceStepQueries for LiveSourceQueries<'_, '_, G> {
    fn catcode(&mut self, code: crate::CharacterCode) -> Catcode {
        self.state.catcode(crate::profile::token_character(code))
    }

    fn firm_up_the_line(&mut self, line: &str) -> Option<super::SourceRegistration> {
        use tex_state::env::banks::IntParam;
        if self.state.int_param(IntParam::PAUSING) <= 0
            || !self.state.interaction_permits_terminal_input()
        {
            return None;
        }
        let prompt = format!("\n{line}=>");
        let replacement = self
            .state
            .input_ln(tex_state::CommandLineSource::Terminal { prompt: &prompt })?;
        if replacement.is_empty() {
            return None;
        }
        Some(super::SourceRegistration::new(
            super::RegisteredSourceKind::Generated,
            replacement.into_bytes(),
        ))
    }
}

impl<G> CompactSourceStepQueries for LiveSourceQueries<'_, '_, G> {
    fn compact_control_word(&mut self, name: &str) -> TokenWord {
        let token = if self.create_control_sequences {
            Token::Cs(self.state.intern_hash_control_sequence(name))
        } else {
            self.state
                .known_control_sequence(name)
                .map_or_else(Token::undefined_control_sequence, Token::Cs)
        };
        TokenWord::pack(token)
    }

    fn compact_source_token(&mut self, source_token: &SourceToken) -> TokenWord {
        let token = match source_token {
            SourceToken::Character { code, catcode, .. } => Token::Char {
                ch: crate::profile::token_character(*code),
                cat: *catcode,
            },
            SourceToken::ControlSequence { name, kind, .. } => match kind {
                SourceControlSequenceKind::Active => Token::Char {
                    ch: crate::profile::token_character(name[0]),
                    cat: Catcode::Active,
                },
                SourceControlSequenceKind::Word
                | SourceControlSequenceKind::Symbol
                | SourceControlSequenceKind::Paragraph
                | SourceControlSequenceKind::Null => {
                    if name.len() == 1 {
                        let mut encoded = [0_u8; 4];
                        let spelling =
                            crate::profile::token_character(name[0]).encode_utf8(&mut encoded);
                        Token::Cs(self.state.intern_control_sequence(spelling))
                    } else {
                        let hashed = *kind == SourceControlSequenceKind::Word && name.len() > 1;
                        name.with_text(|name| {
                            if hashed && !self.create_control_sequences {
                                self.state
                                    .known_control_sequence(name)
                                    .map_or_else(Token::undefined_control_sequence, Token::Cs)
                            } else if hashed {
                                Token::Cs(self.state.intern_hash_control_sequence(name))
                            } else {
                                Token::Cs(self.state.intern_control_sequence(name))
                            }
                        })
                    }
                }
            },
        };
        TokenWord::pack(token)
    }
}

pub(crate) fn input_level_identity<G>(level: &InputLevel<G>) -> InputLevelId {
    match level {
        InputLevel::Source(level) => level.identity(),
        InputLevel::Tokens(level) => level.identity(),
        InputLevel::MacroArgument(level) => level.identity(),
    }
}

#[cfg(test)]
mod tests;

/// Names the canonical effect of exhausting one source level (tex.web §360).
const fn source_retirement_action(retirement: super::SourceRetirement) -> InputRetirementAction {
    match retirement {
        super::SourceRetirement::Pop => InputRetirementAction::SourcePopped,
        super::SourceRetirement::EndReadLine => InputRetirementAction::ReadLineEnded,
    }
}
