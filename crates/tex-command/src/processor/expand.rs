//! Fused resident-input advancement and raw/expanded command delivery.

use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, ResolvedMeaning};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use crate::command::{CommandClass, DeliveryStamp, HotCommand};
use crate::execution_scratch::ArgumentSetId;
use crate::input::{
    InputLevel, InputLevelId, PackedInputFrame, ResidentBoundary, ResidentSourceAdvance,
    ResidentSourceCharacterRun, ResidentSourceTop, ResidentTokenStorage, SourceLocation,
    SourceNameClass, TokenBehavior,
};
use crate::{CommandError, CommandReplayDelivery, CurrentCommand};

use super::end_input::{RetirementHandoff, SourceExhaustionStatus};
use super::expand_render::format_pdf_date;
use super::{AlignmentLookahead, CommandProcessor, DeliveryStatus};

use crate::observation::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandProvenance,
    InputReason, InputRecord, InputTransition,
};

/// TeX82 §345's invalid source-character report.
const INVALID_SOURCE_CHARACTER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0345;

enum ResidentColdOutcome {
    Retry,
    End,
    ReplayCompleted(crate::CommandReplayEpisode),
    SyntheticCommand { literal_catcode: Option<Catcode> },
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum ResidentStorageKind {
    Stored,
    MacroBody,
    MacroArgument,
}

struct CurrentFrameWord {
    word: TokenWord,
    origin: OriginId,
    position: u32,
}

enum InputFrameTransition<G> {
    Boundary(ResidentBoundary),
    Source {
        resident_index: usize,
    },
    ResidentExhausted {
        resident_index: usize,
        identity: InputLevelId,
    },
    Parameter {
        slot: u8,
        arguments: Option<ArgumentSetId<G>>,
        active_source: Option<tex_state::packed_input::SourceContext>,
    },
}

/// One packed word selected from a resident token row. The selection keeps
/// the row's already-admitted coordinates beside the word so the delivery
/// loops can either resolve it into the hot command or consume an ordinary
/// character directly in the main-control run.
enum ResidentWordRead<G> {
    NoResident,
    Source {
        resident_index: usize,
    },
    Parameter {
        slot: u8,
        arguments: Option<ArgumentSetId<G>>,
        active_source: Option<tex_state::packed_input::SourceContext>,
    },
    Exhausted {
        resident_index: usize,
        identity: InputLevelId,
    },
    Word {
        word: TokenWord,
        origin: OriginId,
        identity: u64,
        position: u64,
        active_source: Option<tex_state::packed_input::SourceContext>,
        suppress_expandable: bool,
        #[cfg(test)]
        storage_kind: ResidentStorageKind,
        #[cfg(feature = "profiling")]
        raw_kind: crate::fuel::RawDeliveryKind,
    },
}

/// Reads one packed word from an already-selected resident storage domain.
///
/// Stack mutation, exhaustion, substitution, diagnostics, and recovery must
/// remain outside this instruction body. The loader is specific to the
/// selected lifetime domain and the packed frame remains the sole logical
/// cursor shared by all of them.
#[inline(always)]
fn next_word_from_current_frame(
    frame: &mut PackedInputFrame,
    load: impl FnOnce(u32) -> Option<(TokenWord, OriginId)>,
) -> Option<CurrentFrameWord> {
    let position = frame.position();
    if position >= frame.limit() {
        return None;
    }
    let (word, origin) = load(position)?;
    debug_assert_eq!(frame.advance_resident(), position);
    Some(CurrentFrameWord {
        word,
        origin,
        position,
    })
}

/// Reads one word from an admitted macro replacement cursor.
///
/// The hot path checks the logical frame bound, indexes the retained
/// immutable chunk, and advances the body/frame scalars. A physical crossing
/// is reported by the body and settled by its cold directory transition only
/// after the final word of the current chunk.
#[inline(always)]
fn next_macro_body_word_from_current_frame<G>(
    frame: &mut PackedInputFrame,
    body: &mut crate::input::MacroBodyCursor<G>,
) -> Option<CurrentFrameWord> {
    let position = frame.position();
    if position >= frame.limit() {
        return None;
    }
    let (word, boundary) = body.body.read_current_word(position)?;
    debug_assert_eq!(frame.advance_resident(), position);
    if boundary {
        body.body.advance_chunk_cold();
    }
    Some(CurrentFrameWord {
        word,
        origin: OriginId::UNKNOWN,
        position,
    })
}

fn static_meaning<G>(meaning: &ResolvedMeaning<G>) -> Option<Meaning> {
    match meaning {
        ResolvedMeaning::Static(meaning) => Some(*meaning),
        ResolvedMeaning::Macro { .. } => None,
    }
}

/// The one decision TeX.web §380 makes after raw delivery has resolved the
/// current meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpandedCommandAction {
    Return,
    EndTemplate,
    Expand(ExpansionDispatch),
}

/// The exact TeX.web §366 branch selected by expanded-command
/// classification. This is call-local control flow, not a retained meaning
/// representation: a resource suspension continues to own only its one
/// `CurrentCommand` and re-borrows that meaning when the operation resumes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExpansionDispatch {
    Macro,
    Primitive(ExpandablePrimitive),
    Undefined,
}

#[cfg(test)]
thread_local! {
    static EXPANDED_CLASSIFICATIONS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

#[cfg(test)]
fn expanded_classifications() -> u64 {
    EXPANDED_CLASSIFICATIONS.with(core::cell::Cell::get)
}

#[inline(always)]
fn classify_expanded_command<G>(command: &CurrentCommand<G>) -> ExpandedCommandAction {
    #[cfg(test)]
    EXPANDED_CLASSIFICATIONS.with(|counter| counter.set(counter.get().saturating_add(1)));

    match command.meaning_ref() {
        ResolvedMeaning::Macro { .. } => ExpandedCommandAction::Expand(ExpansionDispatch::Macro),
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)) => {
            ExpandedCommandAction::EndTemplate
        }
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName)) => {
            ExpandedCommandAction::Return
        }
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive)) => {
            ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(*primitive))
        }
        ResolvedMeaning::Static(Meaning::Undefined)
            if !matches!(command.spelling().semantic_token(), Token::Param(_)) =>
        {
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
        }
        ResolvedMeaning::Static(_) => ExpandedCommandAction::Return,
    }
}

#[inline(always)]
fn classify_hot_command<G>(command: &HotCommand<G>) -> ExpandedCommandAction {
    #[cfg(test)]
    EXPANDED_CLASSIFICATIONS.with(|counter| counter.set(counter.get().saturating_add(1)));

    let word = command.command_word();
    match word.class() {
        CommandClass::Macro => ExpandedCommandAction::Expand(ExpansionDispatch::Macro),
        CommandClass::Expandable => match word.expandable_primitive() {
            Some(ExpandablePrimitive::EndTemplate) => ExpandedCommandAction::EndTemplate,
            Some(ExpandablePrimitive::EndCsName) => ExpandedCommandAction::Return,
            Some(primitive) => {
                ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive))
            }
            None => ExpandedCommandAction::Return,
        },
        CommandClass::Undefined if command.spelling_word().out_parameter_slot().is_none() => {
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
        }
        _ => ExpandedCommandAction::Return,
    }
}

/// The finite expansion set selected by the pinned structural census.
///
/// These families execute against the borrowed live command in the one
/// processor episode. Everything else remains a cold arm in this same
/// interpreter; the profiling materialization counter records only that
/// explicit fallback boundary.
#[inline(always)]
#[cfg(feature = "profiling")]
fn is_ranked_fused_expansion(dispatch: ExpansionDispatch) -> bool {
    matches!(
        dispatch,
        ExpansionDispatch::Macro
            | ExpansionDispatch::Primitive(
                ExpandablePrimitive::ExpandAfter
                    | ExpandablePrimitive::Fi
                    | ExpandablePrimitive::IfX
                    | ExpandablePrimitive::IfNum
                    | ExpandablePrimitive::If
                    | ExpandablePrimitive::CsName
                    | ExpandablePrimitive::NoExpand
                    | ExpandablePrimitive::Detokenize
                    | ExpandablePrimitive::String
                    | ExpandablePrimitive::IfFalse
                    | ExpandablePrimitive::RomanNumeral
                    | ExpandablePrimitive::Else
                    | ExpandablePrimitive::Expanded
                    | ExpandablePrimitive::IfCsName
                    | ExpandablePrimitive::Number
                    | ExpandablePrimitive::The
                    | ExpandablePrimitive::PdfUniformDeviate
                    | ExpandablePrimitive::PdfXImageBBox
            )
    )
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Selects one word from the already-admitted resident row. This is the
    /// small cursor instruction shared by scalar delivery and the main-loop
    /// character path; source rows and all exhaustion transitions stay in the
    /// cold reader below.
    #[inline(always)]
    fn next_resident_word(&mut self) -> Result<ResidentWordRead<G>, CommandError> {
        let command_state = &mut *self.command;
        let Some(resident_index) = command_state.roots.input.levels.top.checked_sub(1) else {
            return Ok(ResidentWordRead::NoResident);
        };
        #[cfg(test)]
        {
            command_state
                .roots
                .input
                .levels
                .cursor_mutations
                .typed_top_accesses = command_state
                .roots
                .input
                .levels
                .cursor_mutations
                .typed_top_accesses
                .saturating_add(1);
            command_state
                .raw_delivery_path_counters
                .resident_transitions = command_state
                .raw_delivery_path_counters
                .resident_transitions
                .saturating_add(1);
        }

        let InputLevel::Resident(row) = &mut command_state.roots.input.levels.rows[resident_index]
        else {
            return Ok(ResidentWordRead::Source { resident_index });
        };
        let exhausted_identity = row.header.identity();
        let identity = exhausted_identity.0;
        let active_source = row.header.frame.source_context();
        let suppress_expandable = row.header.frame.flags().contains(
            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
        );
        #[cfg(test)]
        let storage_kind = match &row.storage {
            ResidentTokenStorage::MacroBody(_) => ResidentStorageKind::MacroBody,
            ResidentTokenStorage::MacroArgument(_) => ResidentStorageKind::MacroArgument,
            ResidentTokenStorage::Replay { .. }
            | ResidentTokenStorage::Attempt(_)
            | ResidentTokenStorage::Durable(_) => ResidentStorageKind::Stored,
        };
        #[cfg(feature = "profiling")]
        let raw_kind = match &row.storage {
            ResidentTokenStorage::MacroArgument(_) => crate::fuel::RawDeliveryKind::MacroArgument,
            ResidentTokenStorage::Replay { .. }
            | ResidentTokenStorage::Attempt(_)
            | ResidentTokenStorage::Durable(_)
            | ResidentTokenStorage::MacroBody(_) => crate::fuel::RawDeliveryKind::StoredToken,
        };

        let current = match &mut row.storage {
            ResidentTokenStorage::Replay { replay, cursor } => {
                #[cfg(test)]
                {
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .replay_domain_dispatches = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .replay_domain_dispatches
                        .saturating_add(1);
                    command_state.stored_token_advance_counters.span_selections = command_state
                        .stored_token_advance_counters
                        .span_selections
                        .saturating_add(1);
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries
                        .saturating_add(1);
                }
                next_word_from_current_frame(&mut row.header.frame, |_position| {
                    command_state
                        .roots
                        .input
                        .replay
                        .advance_sequential(
                            *replay,
                            cursor,
                            #[cfg(test)]
                            &mut command_state
                                .stored_token_advance_counters
                                .replay_segment_inspections,
                            #[cfg(test)]
                            &mut command_state
                                .stored_token_advance_counters
                                .replay_run_transitions,
                        )
                        .map(|word| (word.token_word(), word.origin()))
                })
            }
            ResidentTokenStorage::Attempt(list) => {
                #[cfg(test)]
                {
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .attempt_domain_dispatches = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .attempt_domain_dispatches
                        .saturating_add(1);
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries
                        .saturating_add(1);
                }
                next_word_from_current_frame(&mut row.header.frame, |position| {
                    command_state
                        .attempt
                        .arena()
                        .resident_token_word(list, position as usize)
                        .map(|word| (word.token_word(), word.origin()))
                })
            }
            ResidentTokenStorage::Durable(list) => {
                #[cfg(test)]
                {
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .durable_domain_dispatches = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .durable_domain_dispatches
                        .saturating_add(1);
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .stored_token_branch_entries
                        .saturating_add(1);
                }
                next_word_from_current_frame(&mut row.header.frame, |position| {
                    list.word_at(position as usize)
                        .map(|word| (word, tex_state::token::OriginId::UNKNOWN))
                })
            }
            ResidentTokenStorage::MacroBody(body) => {
                #[cfg(test)]
                {
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .macro_body_domain_dispatches = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .macro_body_domain_dispatches
                        .saturating_add(1);
                }
                next_macro_body_word_from_current_frame(&mut row.header.frame, body)
            }
            ResidentTokenStorage::MacroArgument(argument) => {
                #[cfg(test)]
                {
                    command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .macro_argument_branch_entries = command_state
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .macro_argument_branch_entries
                        .saturating_add(1);
                }
                next_word_from_current_frame(&mut row.header.frame, |position| {
                    argument.advance_delivery(position, &command_state.scratch)
                })
            }
        };

        let Some(CurrentFrameWord {
            word,
            origin,
            position,
        }) = current
        else {
            return Ok(ResidentWordRead::Exhausted {
                resident_index,
                identity: exhausted_identity,
            });
        };

        #[cfg(test)]
        match storage_kind {
            ResidentStorageKind::Stored => {
                command_state.stored_token_advance_counters.packed_loads = command_state
                    .stored_token_advance_counters
                    .packed_loads
                    .saturating_add(1);
                command_state.stored_token_advance_counters.cursor_advances = command_state
                    .stored_token_advance_counters
                    .cursor_advances
                    .saturating_add(1);
            }
            ResidentStorageKind::MacroBody => {
                command_state.macro_kernel_counters.body_words = command_state
                    .macro_kernel_counters
                    .body_words
                    .saturating_add(1);
                command_state.macro_kernel_counters.body_cursor_advances = command_state
                    .macro_kernel_counters
                    .body_cursor_advances
                    .saturating_add(1);
            }
            ResidentStorageKind::MacroArgument => {
                command_state.macro_kernel_counters.argument_words = command_state
                    .macro_kernel_counters
                    .argument_words
                    .saturating_add(1);
                command_state.macro_kernel_counters.argument_cursor_advances = command_state
                    .macro_kernel_counters
                    .argument_cursor_advances
                    .saturating_add(1);
            }
        }

        let arguments = match &row.storage {
            ResidentTokenStorage::MacroBody(body) => Some(body.arguments),
            _ if !matches!(row.header.behavior(), TokenBehavior::Parameter) => Some(None),
            _ => None,
        };
        if let Some(arguments) = arguments
            && let Some(slot) = word.out_parameter_slot()
        {
            #[cfg(test)]
            match storage_kind {
                ResidentStorageKind::Stored => {
                    command_state
                        .stored_token_advance_counters
                        .parameter_interceptions = command_state
                        .stored_token_advance_counters
                        .parameter_interceptions
                        .saturating_add(1);
                }
                ResidentStorageKind::MacroBody => {
                    command_state.macro_kernel_counters.body_parameter_pushes = command_state
                        .macro_kernel_counters
                        .body_parameter_pushes
                        .saturating_add(1);
                }
                ResidentStorageKind::MacroArgument => {}
            }
            return Ok(ResidentWordRead::Parameter {
                slot,
                arguments,
                active_source,
            });
        }

        Ok(ResidentWordRead::Word {
            word,
            origin,
            identity,
            position: u64::from(position),
            active_source,
            suppress_expandable,
            #[cfg(test)]
            storage_kind,
            #[cfg(feature = "profiling")]
            raw_kind,
        })
    }

    /// Substitution changes the input stack and therefore stays on the cold
    /// transition side of the delivery boundary. The resident reader only
    /// reports the parameter coordinate; each concrete loop retries its own
    /// fetch after this one transition.
    #[cold]
    #[inline(never)]
    fn retry_parameter_delivery(
        &mut self,
        slot: u8,
        arguments: Option<ArgumentSetId<G>>,
        active_source: Option<tex_state::packed_input::SourceContext>,
        command: &mut HotCommand<G>,
    ) -> Result<(), CommandError> {
        match self.transition_input_frame(
            InputFrameTransition::Parameter {
                slot,
                arguments,
                active_source,
            },
            command,
        )? {
            ResidentColdOutcome::Retry => Ok(()),
            ResidentColdOutcome::End
            | ResidentColdOutcome::ReplayCompleted(_)
            | ResidentColdOutcome::SyntheticCommand { .. } => Err(CommandError::input_invariant()),
        }
    }

    /// The concrete TeX82 §341 raw-token loop. It owns one fuel charge for
    /// each semantic raw token and retries only through cold input transitions.
    #[inline(always)]
    pub(super) fn raw_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.invalidate_delivery_freshness();
        let depth = self.command.transient.active_expansion_depth;
        let mut command = HotCommand::empty();
        if let Err(failure) = self.charge_command_action() {
            return self.fail_expanded_delivery(destination, depth, failure);
        }
        let literal_catcode = 'fetch: loop {
            let selected = match self.next_resident_word() {
                Ok(selected) => selected,
                Err(failure) => {
                    return self.fail_expanded_delivery(destination, depth, failure);
                }
            };
            match selected {
                ResidentWordRead::NoResident => {
                    let cold = match self.transition_input_frame(
                        InputFrameTransition::Boundary(ResidentBoundary::Empty),
                        &mut command,
                    ) {
                        Ok(cold) => cold,
                        Err(failure) => {
                            return self.fail_expanded_delivery(destination, depth, failure);
                        }
                    };
                    match cold {
                        ResidentColdOutcome::Retry => continue 'fetch,
                        ResidentColdOutcome::End => {
                            destination.take();
                            return Ok(DeliveryStatus::End);
                        }
                        ResidentColdOutcome::ReplayCompleted(episode) => {
                            destination.take();
                            return Ok(DeliveryStatus::ReplayCompleted(episode));
                        }
                        ResidentColdOutcome::SyntheticCommand { literal_catcode } => {
                            break 'fetch literal_catcode;
                        }
                    }
                }
                ResidentWordRead::Source { resident_index } => {
                    let cold = match self.transition_input_frame(
                        InputFrameTransition::Source { resident_index },
                        &mut command,
                    ) {
                        Ok(cold) => cold,
                        Err(failure) => {
                            return self.fail_expanded_delivery(destination, depth, failure);
                        }
                    };
                    match cold {
                        ResidentColdOutcome::Retry => continue 'fetch,
                        ResidentColdOutcome::End => {
                            destination.take();
                            return Ok(DeliveryStatus::End);
                        }
                        ResidentColdOutcome::ReplayCompleted(episode) => {
                            destination.take();
                            return Ok(DeliveryStatus::ReplayCompleted(episode));
                        }
                        ResidentColdOutcome::SyntheticCommand { literal_catcode } => {
                            break 'fetch literal_catcode;
                        }
                    }
                }
                ResidentWordRead::Parameter {
                    slot,
                    arguments,
                    active_source,
                } => {
                    self.retry_parameter_delivery(slot, arguments, active_source, &mut command)?;
                    continue 'fetch;
                }
                ResidentWordRead::Exhausted {
                    resident_index,
                    identity,
                } => {
                    let cold = match self.transition_input_frame(
                        InputFrameTransition::ResidentExhausted {
                            resident_index,
                            identity,
                        },
                        &mut command,
                    ) {
                        Ok(cold) => cold,
                        Err(failure) => {
                            return self.fail_expanded_delivery(destination, depth, failure);
                        }
                    };
                    match cold {
                        ResidentColdOutcome::Retry => continue 'fetch,
                        ResidentColdOutcome::End => {
                            destination.take();
                            return Ok(DeliveryStatus::End);
                        }
                        ResidentColdOutcome::ReplayCompleted(episode) => {
                            destination.take();
                            return Ok(DeliveryStatus::ReplayCompleted(episode));
                        }
                        ResidentColdOutcome::SyntheticCommand { literal_catcode } => {
                            break 'fetch literal_catcode;
                        }
                    }
                }
                ResidentWordRead::Word {
                    word,
                    origin,
                    identity,
                    position,
                    active_source,
                    suppress_expandable,
                    #[cfg(test)]
                    storage_kind,
                    #[cfg(feature = "profiling")]
                    raw_kind,
                } => {
                    #[cfg(test)]
                    match storage_kind {
                        ResidentStorageKind::Stored => {
                            self.command.stored_token_advance_counters.command_writes = self
                                .command
                                .stored_token_advance_counters
                                .command_writes
                                .saturating_add(1);
                            self.command.raw_delivery_path_counters.stored_direct = self
                                .command
                                .raw_delivery_path_counters
                                .stored_direct
                                .saturating_add(1);
                        }
                        ResidentStorageKind::MacroBody => {
                            self.command.macro_kernel_counters.body_command_writes = self
                                .command
                                .macro_kernel_counters
                                .body_command_writes
                                .saturating_add(1);
                        }
                        ResidentStorageKind::MacroArgument => {
                            self.command.macro_kernel_counters.argument_command_writes = self
                                .command
                                .macro_kernel_counters
                                .argument_command_writes
                                .saturating_add(1);
                            self.command
                                .raw_delivery_path_counters
                                .macro_argument_direct = self
                                .command
                                .raw_delivery_path_counters
                                .macro_argument_direct
                                .saturating_add(1);
                        }
                    }
                    let resolution = command.write_resolved_delivery(
                        word,
                        origin,
                        identity,
                        position,
                        active_source,
                        false,
                        None,
                        suppress_expandable,
                        self.state,
                    );
                    #[cfg(test)]
                    if matches!(storage_kind, ResidentStorageKind::Stored) {
                        self.command.stored_token_advance_counters.meaning_lookups = self
                            .command
                            .stored_token_advance_counters
                            .meaning_lookups
                            .saturating_add(u64::from(resolution.meaning_lookup()));
                    }
                    #[cfg(feature = "profiling")]
                    self.fuel.record_raw_delivery(
                        self.command.delivery_mode.scanner_active(),
                        resolution.meaning_lookup(),
                        raw_kind,
                    );
                    break 'fetch resolution.literal_catcode();
                }
            }
        };
        self.command.delivery_mode.begin_token(
            command.suppresses_expandable_control_sequence(),
            command.is_outer(),
        );
        self.command.roots.alignment.account_literal_brace(
            &mut self.command.timeline,
            &mut command,
            literal_catcode,
        );
        self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
        if command.is_direct_source_delivery() {
            self.readmit_delivery_stamp(command.delivery_stamp());
        } else {
            self.publish_resident_delivery();
        }
        if self.command.delivery_mode.requires_slow_settlement()
            && let Err(failure) = self.settle_exceptional_delivery(&mut command)
        {
            return self.fail_expanded_delivery(destination, depth, failure);
        }
        *destination = Some(command.materialize());
        Ok(DeliveryStatus::Command)
    }

    /// Delivers one expanded command through the compact loop and materializes
    /// only at the caller's rich-command boundary. The scanner-owned callers
    /// use the hot entry directly, so a terminal delimiter operand never
    /// crosses this boundary merely to be classified.
    pub(super) fn expanded_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut hot_destination = destination.take().map(HotCommand::from_current);
        let result = self.expanded_next_hot(&mut hot_destination);
        match result {
            Ok(status) => {
                if matches!(
                    status,
                    DeliveryStatus::End
                        | DeliveryStatus::ReplayCompleted(_)
                        | DeliveryStatus::CharacterRun
                ) {
                    hot_destination.take();
                } else {
                    *destination = hot_destination.take().map(|command| command.materialize());
                }
                Ok(status)
            }
            Err(error) => {
                hot_destination.take();
                destination.take();
                Err(error)
            }
        }
    }

    /// The concrete TeX82 §380 `get_x_token` loop. Expansion remains in the
    /// continuously occupied hot command; only scanner/diagnostic/resource
    /// boundaries materialize or park it.
    #[inline(always)]
    fn expanded_next_hot(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.invalidate_delivery_freshness();
        let depth = self.command.transient.active_expansion_depth;
        self.command.scratch.note_delivery_entry(depth);
        let Some(active_depth) = depth.checked_add(1) else {
            return self.fail_hot_expanded_delivery(
                destination,
                depth,
                CommandError::input_invariant(),
            );
        };
        self.command.transient.active_expansion_depth = active_depth;
        let resuming = self.expansion_resume.is_some()
            || self
                .scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion);
        let (mut command, mut fetch, mut delivery_expanded) = if resuming {
            match self.resume_expanded_delivery(destination.take()) {
                Ok((command, resumed_expanded)) => (command, false, resumed_expanded),
                Err(failure) => {
                    return self.fail_hot_expanded_delivery(destination, depth, failure);
                }
            }
        } else if let Some(command) = destination.take() {
            (command, false, false)
        } else {
            (HotCommand::empty(), true, false)
        };
        let mut suppress_first_expansion_trace = delivery_expanded;
        let status = 'delivery: loop {
            if fetch {
                self.invalidate_delivery_freshness();
                if let Err(failure) = self.charge_command_action() {
                    return self.fail_hot_expanded_delivery(destination, depth, failure);
                }
                let literal_catcode = 'fetch: loop {
                    let selected = match self.next_resident_word() {
                        Ok(selected) => selected,
                        Err(failure) => {
                            return self.fail_hot_expanded_delivery(destination, depth, failure);
                        }
                    };
                    match selected {
                        ResidentWordRead::NoResident => {
                            let cold = match self.transition_input_frame(
                                InputFrameTransition::Boundary(ResidentBoundary::Empty),
                                &mut command,
                            ) {
                                Ok(cold) => cold,
                                Err(failure) => {
                                    return self.fail_hot_expanded_delivery(
                                        destination,
                                        depth,
                                        failure,
                                    );
                                }
                            };
                            match cold {
                                ResidentColdOutcome::Retry => continue 'fetch,
                                ResidentColdOutcome::End => {
                                    if self.finish_number_continuation_at_end()? {
                                        continue 'fetch;
                                    }
                                    if self.finish_pdf_ximage_bbox_continuation_at_end()? {
                                        continue 'fetch;
                                    }
                                    if self.command.scratch.driver_continuation_depth() != 0 {
                                        self.command
                                            .scratch
                                            .abort_synchronous_controls()
                                            .map_err(crate::scan_toks::scratch_command_error)?;
                                    }
                                    break 'delivery DeliveryStatus::End;
                                }
                                ResidentColdOutcome::ReplayCompleted(episode) => {
                                    break 'delivery DeliveryStatus::ReplayCompleted(episode);
                                }
                                ResidentColdOutcome::SyntheticCommand { literal_catcode } => {
                                    break 'fetch literal_catcode;
                                }
                            }
                        }
                        ResidentWordRead::Source { resident_index } => {
                            let cold = match self.transition_input_frame(
                                InputFrameTransition::Source { resident_index },
                                &mut command,
                            ) {
                                Ok(cold) => cold,
                                Err(failure) => {
                                    return self.fail_hot_expanded_delivery(
                                        destination,
                                        depth,
                                        failure,
                                    );
                                }
                            };
                            match cold {
                                ResidentColdOutcome::Retry => continue 'fetch,
                                ResidentColdOutcome::End => {
                                    if self.finish_number_continuation_at_end()? {
                                        continue 'fetch;
                                    }
                                    if self.finish_pdf_ximage_bbox_continuation_at_end()? {
                                        continue 'fetch;
                                    }
                                    if self.command.scratch.driver_continuation_depth() != 0 {
                                        self.command
                                            .scratch
                                            .abort_synchronous_controls()
                                            .map_err(crate::scan_toks::scratch_command_error)?;
                                    }
                                    break 'delivery DeliveryStatus::End;
                                }
                                ResidentColdOutcome::ReplayCompleted(episode) => {
                                    break 'delivery DeliveryStatus::ReplayCompleted(episode);
                                }
                                ResidentColdOutcome::SyntheticCommand { literal_catcode } => {
                                    break 'fetch literal_catcode;
                                }
                            }
                        }
                        ResidentWordRead::Parameter {
                            slot,
                            arguments,
                            active_source,
                        } => {
                            self.retry_parameter_delivery(
                                slot,
                                arguments,
                                active_source,
                                &mut command,
                            )?;
                            continue 'fetch;
                        }
                        ResidentWordRead::Exhausted {
                            resident_index,
                            identity,
                        } => {
                            let cold = match self.transition_input_frame(
                                InputFrameTransition::ResidentExhausted {
                                    resident_index,
                                    identity,
                                },
                                &mut command,
                            ) {
                                Ok(cold) => cold,
                                Err(failure) => {
                                    return self.fail_hot_expanded_delivery(
                                        destination,
                                        depth,
                                        failure,
                                    );
                                }
                            };
                            match cold {
                                ResidentColdOutcome::Retry => continue 'fetch,
                                ResidentColdOutcome::End => {
                                    if self.finish_number_continuation_at_end()? {
                                        continue 'fetch;
                                    }
                                    if self.finish_pdf_ximage_bbox_continuation_at_end()? {
                                        continue 'fetch;
                                    }
                                    if self.command.scratch.driver_continuation_depth() != 0 {
                                        self.command
                                            .scratch
                                            .abort_synchronous_controls()
                                            .map_err(crate::scan_toks::scratch_command_error)?;
                                    }
                                    break 'delivery DeliveryStatus::End;
                                }
                                ResidentColdOutcome::ReplayCompleted(episode) => {
                                    break 'delivery DeliveryStatus::ReplayCompleted(episode);
                                }
                                ResidentColdOutcome::SyntheticCommand { literal_catcode } => {
                                    break 'fetch literal_catcode;
                                }
                            }
                        }
                        ResidentWordRead::Word {
                            word,
                            origin,
                            identity,
                            position,
                            active_source,
                            suppress_expandable,
                            #[cfg(test)]
                            storage_kind,
                            #[cfg(feature = "profiling")]
                            raw_kind,
                        } => {
                            #[cfg(test)]
                            match storage_kind {
                                ResidentStorageKind::Stored => {
                                    self.command.stored_token_advance_counters.command_writes =
                                        self.command
                                            .stored_token_advance_counters
                                            .command_writes
                                            .saturating_add(1);
                                    self.command.raw_delivery_path_counters.stored_direct = self
                                        .command
                                        .raw_delivery_path_counters
                                        .stored_direct
                                        .saturating_add(1);
                                }
                                ResidentStorageKind::MacroBody => {
                                    self.command.macro_kernel_counters.body_command_writes = self
                                        .command
                                        .macro_kernel_counters
                                        .body_command_writes
                                        .saturating_add(1);
                                }
                                ResidentStorageKind::MacroArgument => {
                                    self.command.macro_kernel_counters.argument_command_writes =
                                        self.command
                                            .macro_kernel_counters
                                            .argument_command_writes
                                            .saturating_add(1);
                                    self.command
                                        .raw_delivery_path_counters
                                        .macro_argument_direct = self
                                        .command
                                        .raw_delivery_path_counters
                                        .macro_argument_direct
                                        .saturating_add(1);
                                }
                            }
                            let resolution = command.write_resolved_delivery(
                                word,
                                origin,
                                identity,
                                position,
                                active_source,
                                false,
                                None,
                                suppress_expandable,
                                self.state,
                            );
                            #[cfg(test)]
                            if matches!(storage_kind, ResidentStorageKind::Stored) {
                                self.command.stored_token_advance_counters.meaning_lookups = self
                                    .command
                                    .stored_token_advance_counters
                                    .meaning_lookups
                                    .saturating_add(u64::from(resolution.meaning_lookup()));
                            }
                            #[cfg(feature = "profiling")]
                            self.fuel.record_raw_delivery(
                                self.command.delivery_mode.scanner_active(),
                                resolution.meaning_lookup(),
                                raw_kind,
                            );
                            break 'fetch resolution.literal_catcode();
                        }
                    }
                };
                self.command.delivery_mode.begin_token(
                    command.suppresses_expandable_control_sequence(),
                    command.is_outer(),
                );
                self.command.roots.alignment.account_literal_brace(
                    &mut self.command.timeline,
                    &mut command,
                    literal_catcode,
                );
                self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
                if command.is_direct_source_delivery() {
                    self.readmit_delivery_stamp(command.delivery_stamp());
                } else {
                    self.publish_resident_delivery();
                }
                if self.command.delivery_mode.requires_slow_settlement()
                    && let Err(failure) = self.settle_exceptional_delivery(&mut command)
                {
                    return self.fail_hot_expanded_delivery(destination, depth, failure);
                }
            }

            let action = classify_hot_command(&command);

            // e-TeX `\expanded` is a balanced expanded-token collector.  Its
            // body stays in the same hot delivery loop: expandable commands
            // fall through to the ordinary dispatch below, while settled
            // words are appended to the attempt-owned buffer here.  This is
            // deliberately before the other operand controls so a nested
            // `\the`/conditional can use the same LIFO lane.
            let expanded_control = self
                .command
                .scratch
                .top_expanded_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if let Some(control) = expanded_control {
                match control.phase {
                    crate::expansion_work::control::SynchronousExpandedPhase::NeedOpening => {
                        let is_space =
                            command.character_catcode() == Some(tex_state::token::Catcode::Space);
                        let is_relax = matches!(
                            command.resolved_meaning(),
                            ResolvedMeaning::Static(Meaning::Relax)
                        );
                        if is_space || is_relax {
                            fetch = true;
                            continue;
                        }
                        if command.character_catcode()
                            == Some(tex_state::token::Catcode::BeginGroup)
                        {
                            self.command
                                .scratch
                                .begin_expanded_body()
                                .map_err(crate::scan_toks::scratch_command_error)?;
                            fetch = true;
                            continue;
                        }
                        // §403's recovery backs the rejected command up,
                        // installs the synthetic opening brace in alignment
                        // state, and then continues this same collector.
                        self.recover_expanded_opening(command)?;
                        fetch = true;
                        continue;
                    }
                    crate::expansion_work::control::SynchronousExpandedPhase::Collecting => {
                        if matches!(
                            control.kind,
                            crate::expansion_work::control::SynchronousExpandedKind::Unexpanded
                                | crate::expansion_work::control::SynchronousExpandedKind::Detokenize
                        ) {
                            let _ = self.append_expanded_word(&command)?;
                            fetch = true;
                            continue;
                        }
                        if matches!(
                            action,
                            ExpandedCommandAction::Expand(ExpansionDispatch::Macro)
                        ) && command
                            .command_word()
                            .flags()
                            .contains(tex_state::meaning::MeaningFlags::PROTECTED)
                        {
                            // e-TeX's expanded collector suppresses protected
                            // macros for this delivery while retaining their
                            // original spelling in the resulting token list.
                            command.suppress_expandable();
                            let _ = self.append_expanded_word(&command)?;
                            fetch = true;
                            continue;
                        }
                        if matches!(
                            action,
                            ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate
                        ) {
                            let _ = self.append_expanded_word(&command)?;
                            fetch = true;
                            continue;
                        }
                    }
                }
            }

            // Starting `\expanded` itself is a control-lane transition.  A
            // nested occurrence follows the same path and is therefore
            // reduced iteratively rather than invoking `scan_toks` from the
            // live delivery frame.
            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                ExpandablePrimitive::Expanded,
            )) = action
            {
                self.begin_expanded_continuation(command.origin())?;
                fetch = true;
                continue;
            }

            // Within an expanded collector, `\unexpanded` consumes a raw
            // balanced child and splices its words into the parent's writer.
            // Keeping that child in the same control lane avoids the legacy
            // collector's recursive scan and preserves expandable spellings.
            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                ExpandablePrimitive::Unexpanded,
            )) = action
                && let Some(control) = expanded_control
                && control.kind == crate::expansion_work::control::SynchronousExpandedKind::Expanded
            {
                self.begin_unexpanded_continuation(command.origin(), control.writer)?;
                fetch = true;
                continue;
            }

            // `\detokenize` consumes its balanced child without expansion,
            // but writes the canonical token spelling as character tokens
            // directly into the enclosing expanded collector.
            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                ExpandablePrimitive::Detokenize,
            )) = action
                && let Some(control) = expanded_control
                && control.kind == crate::expansion_work::control::SynchronousExpandedKind::Expanded
            {
                self.begin_detokenize_continuation(command.origin(), control.writer)?;
                fetch = true;
                continue;
            }

            // `\expandafter` owns two raw operands but only the second one is
            // expanded. Its compact control intercepts the first command and
            // then lets every nested expansion continue through this same
            // delivery loop. Once that second stream settles on a returned
            // command, backup/replay is performed at the semantic boundary.
            let expandafter_control = self
                .command
                .scratch
                .top_expandafter_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if let Some(control) = expandafter_control {
                match control.phase {
                    crate::expansion_work::control::SynchronousExpandAfterPhase::NeedFirst => {
                        self.command
                            .scratch
                            .save_expandafter_first(command)
                            .map_err(crate::scan_toks::scratch_command_error)?;
                        fetch = true;
                        continue;
                    }
                    crate::expansion_work::control::SynchronousExpandAfterPhase::NeedSecond => {
                        if matches!(action, ExpandedCommandAction::Return) {
                            self.complete_expandafter_continuation(command)?;
                            fetch = true;
                            continue;
                        }
                    }
                    crate::expansion_work::control::SynchronousExpandAfterPhase::AwaitNested => {}
                }
            }

            // `\if` and `\ifcat` each request two expanded operands. Keep
            // only their compact scalar projection in the control lane; an
            // operand that is itself expandable is allowed to run normally
            // and returns here when its result settles.
            let if_compare_control = self
                .command
                .scratch
                .top_if_compare_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if let Some(control) = if_compare_control {
                match control.phase {
                    crate::expansion_work::control::SynchronousIfComparePhase::NeedFirst => {
                        if matches!(action, ExpandedCommandAction::Return) {
                            self.command
                                .scratch
                                .save_if_compare_first(
                                    command.conditional_character_code(),
                                    (control.kind == crate::conditionals::ConditionalKind::IfCat)
                                        .then(|| command.conditional_category_code())
                                        .flatten(),
                                )
                                .map_err(crate::scan_toks::scratch_command_error)?;
                            fetch = true;
                            continue;
                        }
                    }
                    crate::expansion_work::control::SynchronousIfComparePhase::NeedSecond {
                        ..
                    } => {
                        if matches!(action, ExpandedCommandAction::Return) {
                            self.complete_if_compare_continuation(command)?;
                            fetch = true;
                            continue;
                        }
                    }
                    crate::expansion_work::control::SynchronousIfComparePhase::AwaitFirst
                    | crate::expansion_work::control::SynchronousIfComparePhase::AwaitSecond {
                        ..
                    } => {}
                }
            }

            // Numeric and dimension conditionals consume their common
            // literal form directly from the hot command.  Expandable
            // operands remain ordinary delivery actions and return to this
            // compact phase instead of retaining a scalar scanner frame on
            // the Rust stack.
            let if_number_control = self
                .command
                .scratch
                .top_if_number_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if let Some(control) = if_number_control {
                let nested_delimiter = matches!(
                    action,
                    ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                        ExpandablePrimitive::Else
                            | ExpandablePrimitive::Or
                            | ExpandablePrimitive::Fi,
                    ))
                ) && self
                    .command
                    .conditions
                    .current()
                    .is_some_and(|frame| frame.identity != control.condition);
                if matches!(
                    action,
                    ExpandedCommandAction::Return
                        | ExpandedCommandAction::EndTemplate
                        | ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                            ExpandablePrimitive::Else
                                | ExpandablePrimitive::Or
                                | ExpandablePrimitive::Fi,
                        ))
                ) && !nested_delimiter && !matches!(
                    control.phase,
                    crate::expansion_work::control::SynchronousIfNumberPhase::AwaitLeft { .. }
                        | crate::expansion_work::control::SynchronousIfNumberPhase::AwaitRelation { .. }
                        | crate::expansion_work::control::SynchronousIfNumberPhase::AwaitRight { .. }
                ) {
                    match self.advance_if_number_continuation(command)? {
                        crate::conditionals::IfNumberAdvance::Continue
                        | crate::conditionals::IfNumberAdvance::Complete => {
                            fetch = true;
                            continue;
                        }
                    }
                }
            }

            let if_dimension_control = self
                .command
                .scratch
                .top_if_dimension_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if let Some(control) = if_dimension_control {
                let nested_delimiter = matches!(
                    action,
                    ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                        ExpandablePrimitive::Else
                            | ExpandablePrimitive::Or
                            | ExpandablePrimitive::Fi,
                    ))
                ) && self
                    .command
                    .conditions
                    .current()
                    .is_some_and(|frame| frame.identity != control.condition);
                if matches!(
                    action,
                    ExpandedCommandAction::Return
                        | ExpandedCommandAction::EndTemplate
                        | ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                            ExpandablePrimitive::Else
                                | ExpandablePrimitive::Or
                                | ExpandablePrimitive::Fi,
                        ))
                ) && !nested_delimiter && !matches!(
                    control.phase,
                    crate::expansion_work::control::SynchronousIfDimensionPhase::AwaitLeft {
                        ..
                    }
                        | crate::expansion_work::control::SynchronousIfDimensionPhase::AwaitRelation {
                            ..
                        }
                        | crate::expansion_work::control::SynchronousIfDimensionPhase::AwaitRight {
                            ..
                        }
                ) {
                    match self.advance_if_dimension_continuation(command)? {
                        crate::conditionals::IfDimensionAdvance::Continue
                        | crate::conditionals::IfDimensionAdvance::Complete => {
                            fetch = true;
                            continue;
                        }
                    }
                }
            }

            let number_control = self
                .command
                .scratch
                .top_number_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if let Some(control) = number_control
                && matches!(
                    action,
                    ExpandedCommandAction::Return
                        | ExpandedCommandAction::EndTemplate
                        | ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                            ExpandablePrimitive::Else
                                | ExpandablePrimitive::Or
                                | ExpandablePrimitive::Fi,
                        ))
                ) && !matches!(
                    control.phase,
                    crate::expansion_work::control::SynchronousNumberPhase::Await { .. }
                        | crate::expansion_work::control::SynchronousNumberPhase::RegisterIndexAwait {
                            ..
                        }
                )
            {
                let _complete = self.advance_number_continuation(command)?;
                fetch = true;
                continue;
            }

            let ximage_bbox_control = self
                .command
                .scratch
                .top_pdf_ximage_bbox_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if ximage_bbox_control.is_some()
                && matches!(
                    action,
                    ExpandedCommandAction::Return
                        | ExpandedCommandAction::EndTemplate
                        | ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                            ExpandablePrimitive::Else
                                | ExpandablePrimitive::Or
                                | ExpandablePrimitive::Fi,
                        ))
                )
            {
                let _ = self.advance_pdf_ximage_bbox_continuation(command, false)?;
                fetch = true;
                continue;
            }

            // `\fontname` consumes one expanded font identifier.  Keep its
            // opener in the compact control lane so nested conversions are
            // reduced by this loop rather than by recursively re-entering a
            // font scanner.
            let fontname_control = self
                .command
                .scratch
                .top_fontname_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if fontname_control.is_some() {
                match action {
                    ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                        primitive @ (ExpandablePrimitive::FontName
                        | ExpandablePrimitive::PdfFontSize
                        | ExpandablePrimitive::PdfFontName
                        | ExpandablePrimitive::PdfFontObjectNumber),
                    )) => {
                        match primitive {
                            ExpandablePrimitive::FontName => {
                                self.begin_fontname_continuation(command.origin())?;
                            }
                            ExpandablePrimitive::PdfFontSize => {
                                self.begin_pdf_font_size_continuation(command.origin())?;
                            }
                            ExpandablePrimitive::PdfFontName => {
                                self.begin_pdf_font_name_continuation(command.origin())?;
                            }
                            ExpandablePrimitive::PdfFontObjectNumber => {
                                self.begin_pdf_font_object_number_continuation(command.origin())?;
                            }
                            _ => unreachable!("font control branch validates its primitive"),
                        }
                        fetch = true;
                        continue;
                    }
                    ExpandedCommandAction::Expand(_) => {}
                    ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate => {
                        self.complete_fontname_continuation(command)?;
                        fetch = true;
                        continue;
                    }
                }
            }

            // The comparison controls are entered from the hot loop rather
            // than through the legacy scalar conditional evaluator. Keeping
            // this cutover here leaves that evaluator available to cold
            // callers while every ordinary delivery stays on one loop.
            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                primitive @ (ExpandablePrimitive::If
                | ExpandablePrimitive::IfCat
                | ExpandablePrimitive::IfNum
                | ExpandablePrimitive::IfPdfAbsNum
                | ExpandablePrimitive::IfDim
                | ExpandablePrimitive::IfPdfAbsDim
                | ExpandablePrimitive::IfOdd
                | ExpandablePrimitive::IfCase
                | ExpandablePrimitive::IfVoid
                | ExpandablePrimitive::IfHBox
                | ExpandablePrimitive::IfVBox
                | ExpandablePrimitive::IfEof
                | ExpandablePrimitive::IfFontChar),
            )) = action
            {
                let kind = crate::conditionals::ConditionalKind::from_primitive(primitive)
                    .ok_or_else(CommandError::input_invariant)?;
                if matches!(
                    kind,
                    crate::conditionals::ConditionalKind::If
                        | crate::conditionals::ConditionalKind::IfCat
                ) {
                    self.begin_if_compare_continuation(kind, false)?;
                } else if matches!(
                    kind,
                    crate::conditionals::ConditionalKind::IfDim
                        | crate::conditionals::ConditionalKind::IfPdfAbsDim
                ) {
                    self.begin_if_dimension_continuation(kind, false)?;
                } else {
                    self.begin_if_number_continuation(kind, false)?;
                }
                fetch = true;
                continue;
            }

            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                primitive @ (ExpandablePrimitive::FontName
                | ExpandablePrimitive::PdfFontSize
                | ExpandablePrimitive::PdfFontName
                | ExpandablePrimitive::PdfFontObjectNumber),
            )) = action
            {
                match primitive {
                    ExpandablePrimitive::FontName => {
                        self.begin_fontname_continuation(command.origin())?;
                    }
                    ExpandablePrimitive::PdfFontSize => {
                        self.begin_pdf_font_size_continuation(command.origin())?;
                    }
                    ExpandablePrimitive::PdfFontName => {
                        self.begin_pdf_font_name_continuation(command.origin())?;
                    }
                    ExpandablePrimitive::PdfFontObjectNumber => {
                        self.begin_pdf_font_object_number_continuation(command.origin())?;
                    }
                    _ => unreachable!("font primitive branch validates its primitive"),
                }
                fetch = true;
                continue;
            }

            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                primitive @ (ExpandablePrimitive::PdfInsertHeight
                | ExpandablePrimitive::PdfXFormName
                | ExpandablePrimitive::PdfPageRef
                | ExpandablePrimitive::PdfLastMatch),
            )) = action
            {
                match primitive {
                    ExpandablePrimitive::PdfInsertHeight => {
                        self.begin_pdf_insert_height_continuation(command.origin())?;
                    }
                    ExpandablePrimitive::PdfXFormName => {
                        self.begin_pdf_xform_name_continuation(command.origin())?;
                    }
                    ExpandablePrimitive::PdfPageRef => {
                        self.begin_pdf_page_ref_continuation(command.origin())?;
                    }
                    ExpandablePrimitive::PdfLastMatch => {
                        self.begin_pdf_last_match_continuation(command.origin())?;
                    }
                    _ => unreachable!("PDF integer branch validates its primitive"),
                }
                fetch = true;
                continue;
            }

            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                ExpandablePrimitive::PdfXImageBBox,
            )) = action
            {
                self.begin_pdf_ximage_bbox_continuation(command.origin())?;
                fetch = true;
                continue;
            }

            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                primitive @ (ExpandablePrimitive::PdfEscapeString
                | ExpandablePrimitive::PdfEscapeHex
                | ExpandablePrimitive::PdfUnescapeHex
                | ExpandablePrimitive::StringCompare),
            )) = action
            {
                let kind = match primitive {
                    ExpandablePrimitive::PdfEscapeString => crate::expansion_work::control::SynchronousExpandedKind::PdfEscapeString,
                    ExpandablePrimitive::PdfEscapeHex => crate::expansion_work::control::SynchronousExpandedKind::PdfEscapeHex,
                    ExpandablePrimitive::PdfUnescapeHex => crate::expansion_work::control::SynchronousExpandedKind::PdfUnescapeHex,
                    ExpandablePrimitive::StringCompare => crate::expansion_work::control::SynchronousExpandedKind::PdfStringCompareLeft,
                    _ => unreachable!("PDF string branch validates its primitive"),
                };
                self.begin_pdf_string_continuation(command.origin(), kind)?;
                fetch = true;
                continue;
            }

            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                primitive @ (ExpandablePrimitive::TopMark
                | ExpandablePrimitive::FirstMark
                | ExpandablePrimitive::BotMark
                | ExpandablePrimitive::SplitFirstMark
                | ExpandablePrimitive::SplitBotMark),
            )) = action
            {
                self.expand_mark(primitive)?;
                fetch = true;
                continue;
            }

            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                primitive @ (ExpandablePrimitive::TopMarks
                | ExpandablePrimitive::FirstMarks
                | ExpandablePrimitive::BotMarks
                | ExpandablePrimitive::SplitFirstMarks
                | ExpandablePrimitive::SplitBotMarks),
            )) = action
            {
                self.begin_mark_class_continuation(command.origin(), primitive)?;
                fetch = true;
                continue;
            }

            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                primitive @ (ExpandablePrimitive::Number | ExpandablePrimitive::RomanNumeral),
            )) = action
            {
                self.begin_number_continuation(
                    command.origin(),
                    primitive == ExpandablePrimitive::RomanNumeral,
                )?;
                fetch = true;
                continue;
            }

            // pdfTeX's uniform-deviate conversion shares TeX's integer
            // operand grammar. Give it the same compact accumulator so a
            // nested enquiry returns through this driver instead of
            // re-entering the retained scalar scanner.
            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                ExpandablePrimitive::PdfUniformDeviate,
            )) = action
            {
                self.begin_pdf_uniform_deviate_continuation(command.origin())?;
                fetch = true;
                continue;
            }

            if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                primitive @ (ExpandablePrimitive::LeftMarginKern
                | ExpandablePrimitive::RightMarginKern),
            )) = action
            {
                let side = if primitive == ExpandablePrimitive::LeftMarginKern {
                    tex_state::node::MarginKernSide::Left
                } else {
                    tex_state::node::MarginKernSide::Right
                };
                self.begin_pdf_margin_kern_continuation(command.origin(), side)?;
                fetch = true;
                continue;
            }

            // A `\the` scalar child may cross an immutable resource barrier
            // (for example while resolving a font/register operand).  Its
            // control has already been removed before entering the scalar
            // scanner, so the resumed phase carries only the opener origin
            // and re-enters this same loop with the original target command.
            // This branch must run before ordinary classification: the
            // restored command is the target, not a new top-level expansion.
            let resumed_the = match self.resumed_expansion.take() {
                Some(crate::state::PendingExpansionResume::The { opener }) => Some(opener),
                Some(other) => {
                    self.resumed_expansion = Some(other);
                    None
                }
                None => None,
            };
            if let Some(opener) = resumed_the {
                let target = command.materialize();
                match self.complete_the_continuation(&target, opener) {
                    Ok(()) => {
                        fetch = true;
                        continue;
                    }
                    Err(error) if error.is_resource_suspension() => {
                        return self.park_the_continuation(
                            target,
                            opener,
                            delivery_expanded,
                            error,
                            destination,
                            depth,
                        );
                    }
                    Err(error) => return self.fail_expanded_delivery(destination, depth, error),
                }
            }

            // `\csname` is another expanded-token consumer. Its spelling is
            // kept in the generation-owned name lane while this compact
            // control remains at the top of the same delivery stack. Nested
            // character-producing expansions therefore return here instead
            // of entering `scan_csname_characters` recursively.
            let csname_control = self
                .command
                .scratch
                .top_csname_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if csname_control.is_some() {
                match action {
                    ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                        ExpandablePrimitive::CsName,
                    )) => {
                        self.begin_csname_continuation(command.origin())?;
                        fetch = true;
                        continue;
                    }
                    ExpandedCommandAction::Expand(_) => {}
                    ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate => {
                        if command.command_word().expandable_primitive()
                            == Some(ExpandablePrimitive::EndCsName)
                        {
                            self.complete_csname_continuation(None)?;
                        } else if let Some(character) = command.character_token() {
                            self.append_csname_character(character)?;
                            fetch = true;
                            continue;
                        } else {
                            self.complete_csname_continuation(Some(command.materialize()))?;
                        }
                        fetch = true;
                        continue;
                    }
                }
            }

            // `\ifcsname` shares the expanded character stream with
            // `\csname`, but its terminator completes a conditional frame
            // instead of backing a control-sequence token. Keeping this
            // predicate in the same control lane removes the recursive
            // scanner edge while preserving the evaluating condition limit.
            let ifcsname_control = self
                .command
                .scratch
                .top_ifcsname_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if ifcsname_control.is_some() {
                match action {
                    ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                        ExpandablePrimitive::IfCsName,
                    )) => {
                        self.begin_ifcsname_continuation(false)?;
                        fetch = true;
                        continue;
                    }
                    ExpandedCommandAction::Expand(_) => {}
                    ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate => {
                        if command.command_word().expandable_primitive()
                            == Some(ExpandablePrimitive::EndCsName)
                        {
                            self.complete_ifcsname_continuation(None)?;
                        } else if let Some(character) = command.character_token() {
                            self.append_csname_character(character)?;
                            fetch = true;
                            continue;
                        } else {
                            self.complete_ifcsname_continuation(Some(command.materialize()))?;
                        }
                        fetch = true;
                        continue;
                    }
                }
            }

            // A `\the` operand is itself an expanded-token request.  Keep
            // that request in the generation-owned control lane and consume
            // targets from this same hot loop.  In particular, a nested
            // `\the` pushes another copy-small control and never invokes a
            // second `expanded_next`/`get_x_token` call.  We remove the
            // completed control before entering a scalar scanner because a
            // register's own index probe is an independent scalar child.
            let the_control = self
                .command
                .scratch
                .top_the_control()
                .map_err(crate::scan_toks::scratch_command_error)?;
            if let Some(the_control) = the_control {
                match (the_control.phase, action) {
                    (
                        crate::expansion_work::control::ThePhase::NeedTarget,
                        ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                            ExpandablePrimitive::The,
                        )),
                    ) => {
                        self.begin_the_continuation(command.origin())?;
                        fetch = true;
                        continue;
                    }
                    (
                        crate::expansion_work::control::ThePhase::NeedTarget,
                        ExpandedCommandAction::Expand(_),
                    ) => {}
                    (
                        crate::expansion_work::control::ThePhase::Index { .. },
                        ExpandedCommandAction::Expand(_),
                    ) => {}
                    (
                        crate::expansion_work::control::ThePhase::Expression { .. },
                        ExpandedCommandAction::Expand(_),
                    ) => {}
                    (
                        crate::expansion_work::control::ThePhase::DimensionExpression { .. },
                        ExpandedCommandAction::Expand(_),
                    ) => {}
                    (
                        crate::expansion_work::control::ThePhase::Index { .. },
                        ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate,
                    ) => {
                        if self.advance_the_index_continuation(command)? {
                            fetch = true;
                            continue;
                        }
                        fetch = true;
                        continue;
                    }
                    (
                        crate::expansion_work::control::ThePhase::Expression { .. },
                        ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate,
                    ) => {
                        if self.advance_the_expression_continuation(command)? {
                            fetch = true;
                            continue;
                        }
                        fetch = true;
                        continue;
                    }
                    (
                        crate::expansion_work::control::ThePhase::DimensionExpression { .. },
                        ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate,
                    ) => {
                        if self.advance_the_dimension_expression_continuation(command)? {
                            fetch = true;
                            continue;
                        }
                        fetch = true;
                        continue;
                    }
                    (
                        crate::expansion_work::control::ThePhase::NeedTarget,
                        ExpandedCommandAction::Return | ExpandedCommandAction::EndTemplate,
                    ) => {
                        let meaning = match command.resolved_meaning() {
                            ResolvedMeaning::Static(meaning) => meaning,
                            ResolvedMeaning::Macro { .. } => Meaning::Undefined,
                        };
                        if Self::compact_the_expression_target(meaning) {
                            self.command.scratch.set_the_phase(
                                crate::expansion_work::control::ThePhase::Expression {
                                    target: meaning,
                                    expression: 0,
                                    expression_sign: 1,
                                    term: 0,
                                    term_operator: 0,
                                    term_active: false,
                                    negative: false,
                                    value: 0,
                                    seen_digit: false,
                                },
                            )?;
                            fetch = true;
                            continue;
                        }
                        if Self::compact_the_dimension_expression_target(meaning) {
                            self.command.scratch.set_the_phase(
                                crate::expansion_work::control::ThePhase::DimensionExpression {
                                    target: meaning,
                                    as_number: false,
                                    expression: 0,
                                    expression_sign: 1,
                                    term: 0,
                                    term_operator: 0,
                                    term_active: false,
                                    negative: false,
                                    value: 0,
                                    fraction: 0,
                                    fraction_digits: 0,
                                    decimal: false,
                                    unit: 0,
                                    seen_digit: false,
                                },
                            )?;
                            fetch = true;
                            continue;
                        }
                        if Self::compact_the_register_target(meaning) {
                            self.command.scratch.set_the_phase(
                                crate::expansion_work::control::ThePhase::Index {
                                    target: meaning,
                                    negative: false,
                                    value: 0,
                                    seen_digit: false,
                                },
                            )?;
                            fetch = true;
                            continue;
                        }
                        if let Some(value) = self.scan_the_direct_value(meaning)? {
                            let opener = self
                                .command
                                .scratch
                                .pop_the_control()
                                .map_err(crate::scan_toks::scratch_command_error)?;
                            self.expand_the_value(opener, value)?;
                            fetch = true;
                            continue;
                        }
                        let _ = self
                            .command
                            .scratch
                            .pop_the_control()
                            .map_err(crate::scan_toks::scratch_command_error)?;
                        let target = command.materialize();
                        match self.complete_the_continuation(&target, the_control.opener) {
                            Ok(()) => {
                                fetch = true;
                                continue;
                            }
                            Err(error) if error.is_resource_suspension() => {
                                return self.park_the_continuation(
                                    target,
                                    the_control.opener,
                                    delivery_expanded,
                                    error,
                                    destination,
                                    depth,
                                );
                            }
                            Err(error) => {
                                return self.fail_expanded_delivery(destination, depth, error);
                            }
                        }
                    }
                }
            }
            match action {
                ExpandedCommandAction::Return => {
                    break 'delivery self.finish_expanded_command(&command, delivery_expanded);
                }
                ExpandedCommandAction::EndTemplate => {
                    if matches!(
                        command.alignment_adjustment(),
                        crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                    ) {
                        break 'delivery DeliveryStatus::AlignmentEndTemplate;
                    }
                    command.convert_end_template_to_endv(self.state.frozen_endv_token());
                    break 'delivery self.finish_expanded_command(&command, delivery_expanded);
                }
                ExpandedCommandAction::Expand(dispatch) => {
                    delivery_expanded = true;
                    let report_trace = !std::mem::take(&mut suppress_first_expansion_trace);
                    let macro_input_before = (dispatch == ExpansionDispatch::Macro)
                        .then(|| self.command.top_input_level_identity());
                    let expandafter_was_awaiting = self.expandafter_awaiting_nested()?;
                    let expandafter_should_await = self.expandafter_second_pending()?
                        && !matches!(
                            dispatch,
                            ExpansionDispatch::Macro
                                | ExpansionDispatch::Undefined
                                | ExpansionDispatch::Primitive(
                                    ExpandablePrimitive::EndTemplate
                                        | ExpandablePrimitive::ExpandAfter
                                        | ExpandablePrimitive::CsName
                                        | ExpandablePrimitive::IfCsName
                                        | ExpandablePrimitive::The
                                )
                        );
                    if expandafter_should_await {
                        self.await_expandafter_nested()?;
                    }
                    let if_compare_was_awaiting = self
                        .command
                        .scratch
                        .top_if_compare_control()
                        .map_err(crate::scan_toks::scratch_command_error)?
                        .is_some_and(|control| {
                            matches!(
                                control.phase,
                                crate::expansion_work::control::SynchronousIfComparePhase::
                                    AwaitFirst
                                    | crate::expansion_work::control::SynchronousIfComparePhase::
                                        AwaitSecond { .. }
                            )
                        });
                    let if_compare_should_await = self
                        .command
                        .scratch
                        .top_if_compare_control()
                        .map_err(crate::scan_toks::scratch_command_error)?
                        .is_some_and(|control| {
                            matches!(
                                control.phase,
                                crate::expansion_work::control::SynchronousIfComparePhase::
                                    NeedFirst
                                    | crate::expansion_work::control::SynchronousIfComparePhase::
                                        NeedSecond { .. }
                            )
                        });
                    let if_number_was_awaiting = self
                        .command
                        .scratch
                        .top_if_number_control()
                        .map_err(crate::scan_toks::scratch_command_error)?
                        .is_some_and(|control| {
                            matches!(
                                control.phase,
                                crate::expansion_work::control::SynchronousIfNumberPhase::AwaitLeft {
                                    ..
                                }
                                    | crate::expansion_work::control::SynchronousIfNumberPhase::AwaitRelation {
                                        ..
                                    }
                                    | crate::expansion_work::control::SynchronousIfNumberPhase::AwaitRight {
                                        ..
                                    }
                                    | crate::expansion_work::control::SynchronousIfNumberPhase::RegisterIndexAwait {
                                        ..
                                    }
                            )
                        });
                    let if_number_should_await = self
                        .command
                        .scratch
                        .top_if_number_control()
                        .map_err(crate::scan_toks::scratch_command_error)?
                        .is_some_and(|control| {
                            matches!(
                                control.phase,
                                crate::expansion_work::control::SynchronousIfNumberPhase::NeedLeft
                                    | crate::expansion_work::control::SynchronousIfNumberPhase::Left {
                                        ..
                                    }
                                    | crate::expansion_work::control::SynchronousIfNumberPhase::NeedRelation {
                                        ..
                                    }
                                    | crate::expansion_work::control::SynchronousIfNumberPhase::Right {
                                        ..
                                    }
                                    | crate::expansion_work::control::SynchronousIfNumberPhase::RegisterIndex {
                                        ..
                                    }
                            )
                        });
                    let if_dimension_was_awaiting = self
                        .command
                        .scratch
                        .top_if_dimension_control()
                        .map_err(crate::scan_toks::scratch_command_error)?
                        .is_some_and(|control| {
                            matches!(
                                control.phase,
                                crate::expansion_work::control::SynchronousIfDimensionPhase::AwaitLeft {
                                    ..
                                }
                                    | crate::expansion_work::control::SynchronousIfDimensionPhase::AwaitRelation {
                                        ..
                                    }
                                    | crate::expansion_work::control::SynchronousIfDimensionPhase::AwaitRight {
                                        ..
                                    }
                                    | crate::expansion_work::control::SynchronousIfDimensionPhase::RegisterIndexAwait {
                                        ..
                                    }
                            )
                        });
                    let if_dimension_should_await = self
                        .command
                        .scratch
                        .top_if_dimension_control()
                        .map_err(crate::scan_toks::scratch_command_error)?
                        .is_some_and(|control| {
                            matches!(
                                control.phase,
                                crate::expansion_work::control::SynchronousIfDimensionPhase::NeedLeft
                                    | crate::expansion_work::control::SynchronousIfDimensionPhase::Left {
                                        ..
                                    }
                                    | crate::expansion_work::control::SynchronousIfDimensionPhase::NeedRelation {
                                        ..
                                    }
                                    | crate::expansion_work::control::SynchronousIfDimensionPhase::Right {
                                        ..
                                    }
                                    | crate::expansion_work::control::SynchronousIfDimensionPhase::RegisterIndex {
                                        ..
                                    }
                            )
                        });
                    let number_was_awaiting = self
                        .command
                        .scratch
                        .top_number_control()
                        .map_err(crate::scan_toks::scratch_command_error)?
                        .is_some_and(|control| {
                            matches!(
                                control.phase,
                                crate::expansion_work::control::SynchronousNumberPhase::Await { .. }
                            )
                        });
                    let number_should_await = self
                        .command
                        .scratch
                        .top_number_control()
                        .map_err(crate::scan_toks::scratch_command_error)?
                        .is_some_and(|control| {
                            matches!(
                                control.phase,
                            crate::expansion_work::control::SynchronousNumberPhase::Need
                                | crate::expansion_work::control::SynchronousNumberPhase::Accumulating {
                                        ..
                                    }
                                | crate::expansion_work::control::SynchronousNumberPhase::RegisterIndex {
                                    ..
                                }
                        )
                        });
                    if if_compare_should_await {
                        self.command
                            .scratch
                            .await_if_compare_operand()
                            .map_err(crate::scan_toks::scratch_command_error)?;
                    }
                    if if_number_should_await {
                        self.command
                            .scratch
                            .await_if_number_operand()
                            .map_err(crate::scan_toks::scratch_command_error)?;
                    }
                    if if_dimension_should_await {
                        self.command
                            .scratch
                            .await_if_dimension_operand()
                            .map_err(crate::scan_toks::scratch_command_error)?;
                    }
                    if number_should_await {
                        self.command
                            .scratch
                            .await_number_operand()
                            .map_err(crate::scan_toks::scratch_command_error)?;
                    }
                    let mut command_parked = false;
                    let failure = match self.expand_classified_occupied(
                        &mut command,
                        dispatch,
                        report_trace,
                        delivery_expanded,
                        &mut command_parked,
                    ) {
                        Ok(()) => {
                            if expandafter_should_await || expandafter_was_awaiting {
                                self.resume_expandafter_second()?;
                            }
                            if if_compare_should_await || if_compare_was_awaiting {
                                self.command
                                    .scratch
                                    .resume_if_compare_operand()
                                    .map_err(crate::scan_toks::scratch_command_error)?;
                            }
                            if if_number_should_await || if_number_was_awaiting {
                                self.command
                                    .scratch
                                    .resume_if_number_operand()
                                    .map_err(crate::scan_toks::scratch_command_error)?;
                            }
                            if if_dimension_should_await || if_dimension_was_awaiting {
                                self.command
                                    .scratch
                                    .resume_if_dimension_operand()
                                    .map_err(crate::scan_toks::scratch_command_error)?;
                            }
                            if number_should_await || number_was_awaiting {
                                self.command
                                    .scratch
                                    .resume_number_operand()
                                    .map_err(crate::scan_toks::scratch_command_error)?;
                            }
                            // Some expandable commands consume themselves
                            // without putting a command back on input. In an
                            // `\expandafter` second-operand phase, replay the
                            // saved first token now instead of consuming an
                            // unrelated third token as the second result.
                            let no_output = match dispatch {
                                ExpansionDispatch::Undefined => true,
                                ExpansionDispatch::Primitive(primitive)
                                    if crate::conditionals::ConditionalKind::from_primitive(
                                        primitive,
                                    )
                                    .is_some_and(|kind| {
                                        kind != crate::conditionals::ConditionalKind::IfCsName
                                    }) =>
                                {
                                    true
                                }
                                ExpansionDispatch::Primitive(
                                    ExpandablePrimitive::Else
                                    | ExpandablePrimitive::Or
                                    | ExpandablePrimitive::Fi,
                                )
                                | ExpansionDispatch::Primitive(ExpandablePrimitive::Unless) => true,
                                ExpansionDispatch::Macro => {
                                    let input_changed = macro_input_before.flatten()
                                        != self.command.top_input_level_identity();
                                    !(input_changed
                                        && self.command.input.levels.last().is_some_and(|level| {
                                            level
                                                .macro_body()
                                                .is_some_and(|body| !body.body.is_empty())
                                        }))
                                }
                                _ => false,
                            };
                            if no_output && self.expandafter_second_pending()? {
                                self.complete_expandafter_without_second()?;
                            }
                            fetch = true;
                            continue;
                        }
                        Err(failure) => failure,
                    };
                    match failure {
                        CommandError::ParagraphInMacroArgument
                        | CommandError::OuterInMacroArgument => {
                            fetch = true;
                        }
                        failure => {
                            return self.fail_hot_expanded_delivery(destination, depth, failure);
                        }
                    }
                }
            }
        };
        debug_assert_eq!(
            self.command.transient.active_expansion_depth, active_depth,
            "expanded delivery balances its depth"
        );
        self.command.transient.active_expansion_depth = depth;
        if matches!(
            status,
            DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_) | DeliveryStatus::CharacterRun
        ) {
            destination.take();
        } else {
            *destination = Some(command);
        }
        Ok(status)
    }

    /// Completes a source or synthetic `endv` command after the main-loop
    /// reader has crossed a cold input boundary. Such a command still needs
    /// the ordinary delivery settlement, but it never belongs to the warm
    /// character-run body.
    #[cold]
    #[inline(never)]
    fn finish_main_loop_synthetic(
        &mut self,
        command: &mut HotCommand<G>,
        literal_catcode: Option<Catcode>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.fuel.charge()?;
        self.command.delivery_mode.begin_token(
            command.suppresses_expandable_control_sequence(),
            command.is_outer(),
        );
        self.command.roots.alignment.account_literal_brace(
            &mut self.command.timeline,
            command,
            literal_catcode,
        );
        self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
        if command.is_direct_source_delivery() {
            self.readmit_delivery_stamp(command.delivery_stamp());
        } else {
            self.publish_resident_delivery();
        }
        if self.command.delivery_mode.requires_slow_settlement() {
            self.settle_exceptional_delivery(command)?;
        }
        *destination = Some(command.materialize());
        Ok(DeliveryStatus::CharacterRunBoundary)
    }

    /// Consumes the direct ordinary-character prefix owned by main control.
    /// The consumer is mandatory here; no other delivery loop carries it.
    #[inline(always)]
    pub(super) fn main_character_run(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        consume: &mut super::MainLoopCharacterConsumer<'_, G>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        self.invalidate_delivery_freshness();
        let mut command = HotCommand::empty();
        let mut consumed_characters = false;
        #[cfg(feature = "profiling")]
        let mut character_run_count = 0_u32;
        #[cfg(feature = "profiling")]
        let mut character_run_kind = None;

        loop {
            let Some(resident_index) = self.command.roots.input.levels.top.checked_sub(1) else {
                if consumed_characters {
                    #[cfg(feature = "profiling")]
                    if let Some(kind) = character_run_kind.take() {
                        self.fuel.record_raw_run(false, kind, character_run_count);
                    }
                    return Ok(DeliveryStatus::CharacterRun);
                }
                let cold = self.transition_input_frame(
                    InputFrameTransition::Boundary(ResidentBoundary::Empty),
                    &mut command,
                )?;
                match cold {
                    ResidentColdOutcome::Retry => continue,
                    ResidentColdOutcome::End => return Ok(DeliveryStatus::End),
                    ResidentColdOutcome::ReplayCompleted(episode) => {
                        return Ok(DeliveryStatus::ReplayCompleted(episode));
                    }
                    ResidentColdOutcome::SyntheticCommand { literal_catcode } => {
                        return self.finish_main_loop_synthetic(
                            &mut command,
                            literal_catcode,
                            destination,
                        );
                    }
                }
            };

            let is_source = matches!(
                self.command.roots.input.levels.rows[resident_index],
                InputLevel::Source(_)
            );
            if is_source {
                #[cfg(test)]
                {
                    self.command
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .typed_top_accesses = self
                        .command
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .typed_top_accesses
                        .saturating_add(1);
                    self.command
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .source_branch_entries = self
                        .command
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .source_branch_entries
                        .saturating_add(1);
                    self.command.raw_delivery_path_counters.resident_transitions = self
                        .command
                        .raw_delivery_path_counters
                        .resident_transitions
                        .saturating_add(1);
                }
                if self.command.delivery_mode.allows_character_run()
                    && self
                        .advance_source_character_run(resident_index, consume)?
                        .is_some()
                {
                    return Ok(DeliveryStatus::CharacterRun);
                }
                let cold = self.transition_input_frame(
                    InputFrameTransition::Source { resident_index },
                    &mut command,
                )?;
                match cold {
                    ResidentColdOutcome::Retry => continue,
                    ResidentColdOutcome::End => return Ok(DeliveryStatus::End),
                    ResidentColdOutcome::ReplayCompleted(episode) => {
                        return Ok(DeliveryStatus::ReplayCompleted(episode));
                    }
                    ResidentColdOutcome::SyntheticCommand { literal_catcode } => {
                        return self.finish_main_loop_synthetic(
                            &mut command,
                            literal_catcode,
                            destination,
                        );
                    }
                }
            }

            let selected = self.next_resident_word()?;
            let ResidentWordRead::Word {
                word,
                origin,
                identity,
                position,
                active_source,
                suppress_expandable,
                #[cfg(test)]
                storage_kind,
                #[cfg(feature = "profiling")]
                raw_kind,
            } = selected
            else {
                match selected {
                    ResidentWordRead::NoResident => unreachable!("top row was selected"),
                    ResidentWordRead::Source { .. } => unreachable!("source handled above"),
                    ResidentWordRead::Parameter {
                        slot,
                        arguments,
                        active_source,
                    } => {
                        self.retry_parameter_delivery(
                            slot,
                            arguments,
                            active_source,
                            &mut command,
                        )?;
                        continue;
                    }
                    ResidentWordRead::Exhausted { .. } if consumed_characters => {
                        #[cfg(feature = "profiling")]
                        if let Some(kind) = character_run_kind.take() {
                            self.fuel.record_raw_run(false, kind, character_run_count);
                        }
                        return Ok(DeliveryStatus::CharacterRun);
                    }
                    ResidentWordRead::Exhausted {
                        resident_index,
                        identity,
                    } => {
                        match self.transition_input_frame(
                            InputFrameTransition::ResidentExhausted {
                                resident_index,
                                identity,
                            },
                            &mut command,
                        )? {
                            ResidentColdOutcome::Retry => continue,
                            ResidentColdOutcome::End => return Ok(DeliveryStatus::End),
                            ResidentColdOutcome::ReplayCompleted(episode) => {
                                return Ok(DeliveryStatus::ReplayCompleted(episode));
                            }
                            ResidentColdOutcome::SyntheticCommand { literal_catcode } => {
                                return self.finish_main_loop_synthetic(
                                    &mut command,
                                    literal_catcode,
                                    destination,
                                );
                            }
                        }
                    }
                    ResidentWordRead::Word { .. } => unreachable!("word was matched above"),
                }
            };

            let is_character = matches!(
                word.semantic_token(),
                Token::Char {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                }
            );
            if is_character && self.command.delivery_mode.allows_character_run() {
                self.fuel.charge()?;
                consumed_characters = true;
                #[cfg(feature = "profiling")]
                {
                    character_run_kind = Some(raw_kind);
                    character_run_count = character_run_count.saturating_add(1);
                }
                let Token::Char { ch, .. } = word.semantic_token() else {
                    unreachable!("main-loop character predicate accepts only characters")
                };
                if consume(self.state, self.fuel, self.diagnostic_effects, ch, origin) {
                    continue;
                }
                #[cfg(feature = "profiling")]
                if let Some(kind) = character_run_kind.take() {
                    self.fuel.record_raw_run(false, kind, character_run_count);
                }
                return Ok(DeliveryStatus::CharacterRun);
            }

            self.fuel.charge()?;
            #[cfg(feature = "profiling")]
            if let Some(kind) = character_run_kind.take() {
                self.fuel.record_raw_run(false, kind, character_run_count);
            }
            #[cfg(test)]
            match storage_kind {
                ResidentStorageKind::Stored => {
                    self.command.stored_token_advance_counters.command_writes = self
                        .command
                        .stored_token_advance_counters
                        .command_writes
                        .saturating_add(1);
                    self.command.raw_delivery_path_counters.stored_direct = self
                        .command
                        .raw_delivery_path_counters
                        .stored_direct
                        .saturating_add(1);
                }
                ResidentStorageKind::MacroBody => {
                    self.command.macro_kernel_counters.body_command_writes = self
                        .command
                        .macro_kernel_counters
                        .body_command_writes
                        .saturating_add(1);
                }
                ResidentStorageKind::MacroArgument => {
                    self.command.macro_kernel_counters.argument_command_writes = self
                        .command
                        .macro_kernel_counters
                        .argument_command_writes
                        .saturating_add(1);
                    self.command
                        .raw_delivery_path_counters
                        .macro_argument_direct = self
                        .command
                        .raw_delivery_path_counters
                        .macro_argument_direct
                        .saturating_add(1);
                }
            }
            let resolution = command.write_resolved_delivery(
                word,
                origin,
                identity,
                position,
                active_source,
                false,
                None,
                suppress_expandable,
                self.state,
            );
            #[cfg(feature = "profiling")]
            self.fuel.record_raw_delivery(
                self.command.delivery_mode.scanner_active(),
                resolution.meaning_lookup(),
                raw_kind,
            );
            self.command.delivery_mode.begin_token(
                command.suppresses_expandable_control_sequence(),
                command.is_outer(),
            );
            self.command.roots.alignment.account_literal_brace(
                &mut self.command.timeline,
                &mut command,
                resolution.literal_catcode(),
            );
            self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
            if command.is_direct_source_delivery() {
                self.readmit_delivery_stamp(command.delivery_stamp());
            } else {
                self.publish_resident_delivery();
            }
            if self.command.delivery_mode.requires_slow_settlement()
                && let Err(failure) = self.settle_exceptional_delivery(&mut command)
            {
                return Err(failure);
            }
            *destination = Some(command.materialize());
            return Ok(DeliveryStatus::CharacterRunBoundary);
        }
    }

    /// Replay-aware raw delivery is a cold entry for the same raw owner. The
    /// public ordinary wrapper consumes completion statuses; replay callers
    /// keep them visible.
    #[cold]
    #[inline(never)]
    pub(super) fn raw_next_with_replay_completion(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.raw_next(destination)
    }

    /// Replay-aware ordinary expansion enters the canonical expanded loop and
    /// leaves its completion status visible to the caller.
    #[cold]
    #[inline(never)]
    pub(super) fn expanded_next_with_replay_completion(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next(destination)
    }

    /// The protected entry is intentionally out of line. Its full protected
    /// classifier is installed below once a raw command has been settled.
    #[cold]
    #[inline(never)]
    pub(super) fn protected_expanded_next_with_replay_completion(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let status = if destination.is_some() {
                DeliveryStatus::Command
            } else if self.expansion_resume.is_some()
                || self
                    .scanner_resume
                    .as_ref()
                    .is_some_and(crate::ScannerFrameKey::is_expansion)
            {
                self.expanded_next(destination)?
            } else {
                self.raw_next(destination)?
            };
            match status {
                DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_) => return Ok(status),
                DeliveryStatus::Command => {}
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                    continue;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("protected delivery has no character consumer")
                }
            }

            let command = destination
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            if matches!(
                command.meaning_ref(),
                ResolvedMeaning::Macro { flags, .. } if flags.contains(MeaningFlags::PROTECTED)
            ) || !is_expandable_command(command)
            {
                return Ok(DeliveryStatus::Command);
            }
            match self.expanded_next(destination)? {
                status @ (DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_)) => {
                    return Ok(status);
                }
                DeliveryStatus::Command => return Ok(DeliveryStatus::Command),
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("protected delivery has no character consumer")
                }
            }
        }
    }

    /// Diagnostic callers keep the undefined command instead of entering its
    /// recovery branch. The exceptional wrapper is cold and owns that one
    /// classifier choice.
    #[cold]
    #[inline(never)]
    pub(super) fn expanded_next_preserving_undefined(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let status = if destination.is_some() {
                DeliveryStatus::Command
            } else if self.expansion_resume.is_some()
                || self
                    .scanner_resume
                    .as_ref()
                    .is_some_and(crate::ScannerFrameKey::is_expansion)
            {
                self.expanded_next(destination)?
            } else {
                self.raw_next(destination)?
            };
            match status {
                DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_) => return Ok(status),
                DeliveryStatus::Command => {}
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                    continue;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("undefined-preserving delivery has no character consumer")
                }
            }
            if destination.as_ref().is_some_and(|command| {
                matches!(
                    command.meaning_ref(),
                    ResolvedMeaning::Static(Meaning::Undefined)
                )
            }) {
                return Ok(DeliveryStatus::Command);
            }
            match self.expanded_next(destination)? {
                status @ (DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_)) => {
                    return Ok(status);
                }
                DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentClosingBrace => return Ok(DeliveryStatus::Command),
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("undefined-preserving delivery has no character consumer")
                }
            }
        }
    }

    /// `x_token` starts with a command already in hand. Ordinary uses have no
    /// pending command and therefore enter the expanded loop directly.
    #[cold]
    #[inline(never)]
    pub(super) fn x_token_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        if destination.as_ref().is_some_and(|command| {
            matches!(
                command.meaning_ref(),
                ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                    ExpandablePrimitive::EndTemplate
                ))
            )
        }) {
            let alignment_delimiter = destination.as_ref().is_some_and(|command| {
                matches!(
                    command.alignment_adjustment(),
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                )
            });
            if alignment_delimiter {
                return Ok(DeliveryStatus::AlignmentEndTemplate);
            }
            destination.take();
            self.insert_frozen_endv()?;
        }
        self.expanded_next(destination)
    }

    /// Main-control lookahead first returns a raw character without expansion;
    /// non-character commands continue through the x-token entry.
    #[cold]
    #[inline(never)]
    pub(super) fn main_loop_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let status = if destination.is_some() {
            DeliveryStatus::Command
        } else {
            self.raw_next(destination)?
        };
        if status != DeliveryStatus::Command {
            return Ok(status);
        }
        let command = destination
            .as_ref()
            .ok_or_else(CommandError::input_invariant)?;
        let hot = HotCommand::from_current_ref(command);
        if hot.command_word().is_main_loop_character() {
            return Ok(DeliveryStatus::Command);
        }
        if !matches!(classify_hot_command(&hot), ExpandedCommandAction::Return) {
            let pending = destination.take();
            return self.x_token_from_into(pending, destination);
        }
        self.observe_expanded_delivery(command);
        Ok(DeliveryStatus::Command)
    }

    /// Main-control preflight owns its first raw fetch and then continues from
    /// that resident command through ordinary expansion.
    #[cold]
    #[inline(never)]
    pub(super) fn preflight_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            if destination.is_none() {
                match self.raw_next(destination)? {
                    DeliveryStatus::Command => {}
                    status => return Ok(status),
                }
            }
            let command = destination
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            if !is_expandable_command(command) {
                let _ = classify_expanded_command(command);
                self.observe_expanded_delivery(command);
                return Ok(DeliveryStatus::Command);
            }
            let pending = destination.take();
            match self.expanded_next_from_pending(pending, destination)? {
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                result => return Ok(result),
            }
        }
    }

    /// Resumed expansion restores its command once, then uses the x-token
    /// semantics owned by the cold continuation wrapper.
    #[cold]
    #[inline(never)]
    pub(super) fn resumed_expanded_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.x_token_next(destination)? {
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                result => return Ok(result),
            }
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn resumed_main_loop_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.main_loop_next(destination)? {
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                result => return Ok(result),
            }
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn alignment_expanded_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        match self.expanded_next(destination)? {
            DeliveryStatus::PendingExpanded => Ok(DeliveryStatus::Command),
            result => Ok(result),
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn alignment_main_loop_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        match self.main_loop_next(destination)? {
            DeliveryStatus::PendingExpanded => Ok(DeliveryStatus::Command),
            result => Ok(result),
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn tex_alignment_lookahead_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.expanded_next(destination)? {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::AlignmentClosingBrace => return Ok(DeliveryStatus::Command),
                result => return Ok(result),
            }
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn etex_alignment_lookahead_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.protected_expanded_next_with_replay_completion(destination)? {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::AlignmentClosingBrace => return Ok(DeliveryStatus::Command),
                result => return Ok(result),
            }
        }
    }

    fn expanded_next_from_pending(
        &mut self,
        pending: Option<CurrentCommand<G>>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        *destination = pending;
        self.expanded_next(destination)
    }

    /// Delivers one ordinary expanded command through TeX.web's `get_x_token`.
    ///
    /// This thin canonical entry point enters the ordinary expanded loop.
    /// Expansion mutates canonical command state and restarts in that loop;
    /// it never returns a push-bearing dispatch result or enters a second
    /// interpreter.
    pub fn get_x_token(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.get_x_token_into(&mut destination)? {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary expanded delivery returns only commands"),
        }
    }

    /// Delivers one expanded command directly into caller-provided storage.
    pub fn get_x_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        loop {
            let result = self.expanded_next(destination)?;
            match result {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::End | DeliveryStatus::Command => return Ok(result),
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("expanded delivery does not own a character consumer")
                }
            }
        }
    }

    /// Requests one expanded token from the generation-scoped delivery
    /// driver.  Scanner and primitive code uses this typed status boundary;
    /// it never reaches into the driver's loop or recursively calls a
    /// delivery implementation by name.
    pub(crate) fn request_expanded_token(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.get_x_token_into(destination)
    }

    /// Requests one already-delivered command's expansion from the same
    /// driver.  This is the only nested expansion request used by structural
    /// scanners; suspension and completion remain represented by the typed
    /// `Result` status returned here.
    pub(crate) fn request_expansion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        self.expand_into(destination, report_trace)
    }

    /// Delivers protected replay-aware expansion into caller-provided storage.
    pub(crate) fn get_x_or_protected_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let preserve = self.command.profile().capabilities().supports_etex();
        let result = if preserve {
            self.protected_expanded_next_with_replay_completion(destination)?
        } else {
            self.expanded_next_with_replay_completion(destination)?
        };
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Delivers one expanded command to a diagnostic host while preserving
    /// TeX82 §370's undefined command instead of consuming it after recovery.
    pub fn get_x_token_preserving_undefined(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        let result = self.expanded_next_preserving_undefined(&mut destination)?;
        match result {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary expanded delivery returns only commands"),
        }
    }

    /// TeX.web §381's `x_token` entered with `cur_cmd`/`cur_chr` already set.
    ///
    /// §381 does not begin with `get_next`: it expands whatever the caller
    /// left in the current command and only then reads on. Ordinary delivery
    /// leaves nothing, which is [`Self::get_x_token`]; §1152 loads an active
    /// character's meaning directly and passes it here, so that meaning is
    /// expanded without ever having been delivered raw.
    fn x_token_from_into(
        &mut self,
        pending: Option<CurrentCommand<G>>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        *destination = pending;
        let result = self.x_token_next(destination)?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End
                | DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentEndTemplate
                | DeliveryStatus::AlignmentClosingBrace
                | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Completes TeX82 §1152's active-character `x_token` handoff.
    ///
    /// The ordinary destination-directed expanded entry exposes
    /// `PendingExpanded` and `AlignmentClosingBrace` only as internal
    /// observer transport markers; both already leave the settled command in
    /// `destination`. Active-character treatment has the same settled-command
    /// ownership, so it must normalize those statuses without constructing or
    /// redelivering another command. An intercepted alignment end-template is
    /// the one exceptional boundary: its command is consumed to begin the
    /// scalar v-template, after which `x_token` retries with no pending
    /// command above the newly installed input frame.
    #[cold]
    #[inline(never)]
    fn active_x_token_into(
        &mut self,
        pending: CurrentCommand<G>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut pending = Some(pending);
        loop {
            match self.x_token_from_into(pending.take(), destination)? {
                DeliveryStatus::End => return Ok(DeliveryStatus::End),
                DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentClosingBrace => return Ok(DeliveryStatus::Command),
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                    // The intercepted delimiter has been consumed by the
                    // alignment transition. The next x-token starts with a
                    // fresh input fetch, rather than redelivering it.
                    pending = None;
                }
                DeliveryStatus::ReplayCompleted(_) => {
                    // Stored replay retirement is an input-boundary event,
                    // not a settled active-character command. Continue the
                    // same x-token operation after the continuation retires.
                    pending = None;
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("active-character delivery has no character consumer")
                }
            }
        }
    }

    /// TeX82 §1152's `@<Treat |cur_chr| as an active character@>`:
    ///
    /// ```text
    /// begin cur_cs:=cur_chr+active_base;
    /// cur_cmd:=eq_type(cur_cs); cur_chr:=equiv(cur_cs);
    /// x_token; back_input;
    /// end
    /// ```
    ///
    /// This is the whole of TeX's `\mathcode` escape hatch. §1155's
    /// `set_math_char` and §1151's `scan_math` both branch here when a
    /// character's `math_code` is `@'100000`, which is what makes plain
    /// TeX's ``\mathcode`\'="8000`` route `'` through the active `'` macro
    /// that builds `\prime` lists.
    ///
    /// The character is not backed up and reread. §1152 loads the
    /// `active_base + c` cell's meaning straight into `cur_cmd`/`cur_chr`,
    /// so there is no raw delivery for it at all: `x_token` expands that
    /// meaning in place -- observing a macro push, not a backup -- and only
    /// the unexpandable token expansion settles on is backed up, from where
    /// the caller rereads it. An active character bound to an unexpandable
    /// meaning still reaches §381's tail, so it is still observed as one
    /// expanded delivery and backed up unchanged.
    pub fn treat_as_active_character(
        &mut self,
        ch: char,
        origin: OriginId,
    ) -> Result<(), CommandError> {
        let spelling = TracedTokenWord::pack(
            Token::Char {
                ch,
                cat: Catcode::Active,
            },
            origin,
        );
        let stamp = DeliveryStamp::new(0, 0);
        self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
        let command = CurrentCommand::<G>::resolve(spelling, stamp, None, false, None, self.state);
        let mut destination = None;
        let status = self.active_x_token_into(command, &mut destination)?;
        let settled = match status {
            DeliveryStatus::End => return Ok(()),
            DeliveryStatus::Command => destination
                .take()
                .expect("command status initializes destination"),
            _ => unreachable!("active-character delivery normalizes to commands"),
        };
        // §325 needs only `cur_tok`; the settled token is `x_token`'s result
        // rather than a delivery this call is undoing, exactly as in §326.
        self.back_input_saved(settled)
    }

    /// TeX82 §404's `<Get the next non-blank non-relax non-call token>`:
    /// `repeat get_x_token until (cur_cmd<>spacer)and(cur_cmd<>relax)`.
    ///
    /// This is the shared spelling of that module, used by §403's
    /// `scan_left_brace`, §1078, §1084, §1151's `scan_math`, §1160's
    /// non-radical `scan_delimiter`, §1211's `prefixed_command`, §1226 and
    /// §1270's `scan_optional_equals`. It differs from §406's
    /// `<Get the next non-blank non-call token>` only by also skipping
    /// `\relax`, and the two are not interchangeable: §1160 classifies the
    /// token it stops on, so a `\relax` that reached it as a command rather
    /// than as a skipped filler would scan as an invalid delimiter.
    pub fn next_non_blank_non_relax_x_token(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        loop {
            match self.get_x_token_into(&mut destination)? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary expanded delivery returns only commands"),
            }
            let command = destination
                .as_ref()
                .expect("command status initializes destination");
            if !matches!(
                static_meaning(command.meaning_ref()),
                Some(
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } | Meaning::Relax
                )
            ) {
                return Ok(destination);
            }
            destination = None;
        }
    }

    /// TeX82 §404's expanded nonblank/non-relax fetch for scanners that can
    /// classify the terminal command directly from the compact delivery.
    ///
    /// The hot command is the sole result owner: it is overwritten on each
    /// fetch, and exactly the command that stops the loop remains in the
    /// caller's slot. In particular, no rich command is made merely to hand
    /// one delimiter operand from expansion to `scan_delimiter`.
    pub(crate) fn next_non_blank_non_relax_x_token_hot(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        loop {
            match self.expanded_next_hot(destination)? {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::End => return Ok(DeliveryStatus::End),
                DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentClosingBrace => {}
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?
                        .materialize();
                    self.begin_scalar_alignment_v_template(&command)?;
                    continue;
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    return Err(CommandError::input_invariant());
                }
            }
            let command = destination
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            if !matches!(
                command.command_word().static_meaning(),
                Some(
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } | Meaning::Relax
                )
            ) {
                return Ok(DeliveryStatus::Command);
            }
            destination.take();
        }
    }

    /// TeX82 §406's `<Get the next non-blank non-call token>`:
    /// `repeat get_x_token until cur_cmd<>spacer`.
    ///
    /// Unlike §404's similarly named helper, this preserves `\relax`. The
    /// returned command is the exact expanded delivery that stopped the
    /// loop: callers such as §1045's `\ignorespaces` dispatch it in place
    /// without backing it up or rebuilding its provenance.
    pub fn next_non_blank_x_token(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        loop {
            match self.get_x_token_into(&mut destination)? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary expanded delivery returns only commands"),
            }
            let command = destination
                .as_ref()
                .expect("command status initializes destination");
            if !matches!(
                static_meaning(command.meaning_ref()),
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                return Ok(destination);
            }
            destination = None;
        }
    }

    /// TeX82 §§785/791's shared alignment lookahead fetch.
    ///
    /// TeX82's `get_x_token` commits the terminal expanded command before
    /// `init_col` backs an ordinary command up. The backup is later read
    /// again above its u-template, producing a second raw/expanded delivery.
    /// Spacers skipped by §406 are complete deliveries and are committed here
    /// normally.
    ///
    /// e-TeX 2.6 change sections [37.785] and [37.791] replace that helper
    /// with `get_x_or_protected`. Its terminal unexpandable command comes
    /// straight from `get_token`, so neither skipped spacers nor a consumed
    /// `\noalign`, `\crcr`, `\omit`, or closing brace has an expanded
    /// delivery. A protected macro is likewise terminal and is backed up as
    /// the first command of the next cell.
    pub fn next_alignment_lookahead(
        &mut self,
    ) -> Result<Option<AlignmentLookahead<G>>, CommandError> {
        loop {
            let etex_protected_fetch = self.command.profile().capabilities().supports_etex();
            let mut destination = None;
            let result = if etex_protected_fetch {
                self.etex_alignment_lookahead_next(&mut destination)
            } else {
                self.tex_alignment_lookahead_next(&mut destination)
            };
            let lookahead = match result? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => AlignmentLookahead::Committed(
                    destination.expect("command status initializes destination"),
                ),
                DeliveryStatus::PendingExpanded => AlignmentLookahead::PendingExpanded(
                    destination.expect("pending status initializes destination"),
                ),
                _ => unreachable!("alignment lookahead consumes replay completions"),
            };
            if matches!(
                lookahead.command().meaning(),
                ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                let _ = self.commit_alignment_lookahead_delivery(lookahead);
                continue;
            }
            return Ok(Some(lookahead));
        }
    }

    /// Commits a terminal TeX82 lookahead delivery that alignment control
    /// consumes instead of passing to an ordinary `back_input` branch.
    pub fn commit_alignment_lookahead_delivery(
        &mut self,
        lookahead: AlignmentLookahead<G>,
    ) -> CurrentCommand<G> {
        match lookahead {
            AlignmentLookahead::Committed(command) => command,
            AlignmentLookahead::PendingExpanded(command) => {
                self.observe_expanded_delivery(&command);
                command
            }
        }
    }

    /// Completes TeX82 §§785/791's ordinary `align_peek`/`init_col` branch.
    ///
    /// A command reached through §380's expansion loop is still pending only
    /// in Umber's observer transport. TeX has already completed
    /// `get_x_token`, so its expanded delivery precedes §789's `back_input`;
    /// the later replay above the u-template is a distinct delivery.
    pub fn back_alignment_lookahead(
        &mut self,
        lookahead: AlignmentLookahead<G>,
    ) -> Result<(), CommandError> {
        let command = self.commit_alignment_lookahead_delivery(lookahead);
        self.back_input(command)
    }

    /// Delivers one expanded command or the completion of an executor-owned
    /// stored replay episode.
    ///
    /// Completion is published after the command machine has retired and
    /// observed the exact stored level, but before it resumes the enclosing
    /// source.  Callers must finish the corresponding isolated execution
    /// lifecycle before requesting another delivery.
    pub fn get_x_token_with_replay_completion(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let result = self.get_x_token_with_replay_completion_into(&mut destination)?;
        Ok(match result {
            DeliveryStatus::End => None,
            DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("ordinary replay-aware delivery has no alignment event"),
        })
    }

    /// Delivers replay-aware expanded input into caller-provided storage.
    pub fn get_x_token_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let result = self.expanded_next_with_replay_completion(destination)?;
            match result {
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::End
                | DeliveryStatus::Command
                | DeliveryStatus::ReplayCompleted(_) => return Ok(result),
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("expanded delivery does not own a character consumer")
                }
            }
        }
    }

    /// Delivers main-control preflight through one raw-fetch/classification
    /// loop. An ordinary unexpandable command publishes its canonical expanded
    /// observation directly, without completing a second expanded-driver
    /// episode; a macro, expandable primitive, or undefined command continues
    /// in place through the canonical expanded loop.
    pub fn preflight_command_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let result = self.preflight_next(destination)?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Resumes one genuinely suspended expansion from its stable parked root.
    /// The key is the executor's only command-related retry owner; consuming
    /// it moves the command once into `destination` before scalar expansion
    /// continues at the retained typed phase.
    pub fn resume_expansion_into(
        &mut self,
        key: crate::ExpansionWorkKey<G>,
        main_loop: bool,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        self.install_expansion_resume(key);
        if main_loop {
            self.resumed_main_loop_next(destination)
        } else {
            self.resumed_expanded_next(destination)
        }
    }

    /// Delivers one command through TeX82 §1038's `main_loop_lookahead`.
    ///
    /// `main_control`'s inner character loop (§1034) never returns to
    /// `big_switch`'s `get_x_token` between adjacent characters. §1038 fetches
    /// the next command with a bare `get_next` -- "set only `cur_cmd` and
    /// `cur_chr`, for speed" -- and jumps straight back into the loop when
    /// that raw command is `letter`, `other_char`, or `char_given`. Only a
    /// raw command outside that set reaches `x_token`, which is the sole
    /// reason a run of ordinary characters produces one raw delivery each and
    /// no expanded delivery at all.
    ///
    /// `char_num` is deliberately *not* in the raw set: §1038 accepts it only
    /// after `x_token`, because `\char` can be reached by expansion.
    pub fn main_loop_lookahead(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let result = self.main_loop_lookahead_into(&mut destination)?;
        Ok(match result {
            DeliveryStatus::End => None,
            DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("main-loop lookahead has no alignment event"),
        })
    }

    /// Delivers main-loop lookahead into caller-provided command storage.
    pub fn main_loop_lookahead_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let result = self.main_loop_next(destination)?;
            match result {
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::End
                | DeliveryStatus::Command
                | DeliveryStatus::ReplayCompleted(_) => {
                    return Ok(result);
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("main-loop lookahead has no character consumer")
                }
            }
        }
    }

    /// Lends the consecutive ordinary-character prefix of §1038's raw
    /// lookahead directly to the admitted list builder.
    ///
    /// The resident input row stays authoritative. Letter/other tokens charge
    /// fuel and advance provenance in place without constructing a
    /// [`CurrentCommand`]; the first non-character is resolved into
    /// `destination` and continues through the canonical expansion tail.
    /// Observation keeps scalar delivery so its one-record-per-command
    /// contract remains exact.
    pub fn main_loop_character_run_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        consume: &mut super::MainLoopCharacterConsumer<'_, G>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        debug_assert!(!self.is_observed());
        self.main_character_run(destination, consume)
    }

    #[cold]
    #[inline(never)]
    fn resume_expanded_delivery(
        &mut self,
        destination: Option<HotCommand<G>>,
    ) -> Result<(HotCommand<G>, bool), CommandError> {
        let key = self.expansion_resume.take().or_else(|| {
            self.scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion)
                .then(|| {
                    let wrapper = self
                        .scanner_resume
                        .take()
                        .expect("matched expansion wrapper");
                    self.command
                        .scratch
                        .take_expansion_key(wrapper)
                        .expect("live wrapper owns expansion work")
                })
        });
        let mut retained = self
            .command
            .scratch
            .resume_expansion(key.expect("genuine suspension owns expansion work"))
            .map_err(crate::scan_toks::scratch_command_error)?;
        if destination
            .is_some_and(|command| command != HotCommand::from_current_ref(&retained.command))
        {
            if let Some(child) = retained.take_child() {
                self.abort_continuation(child)?;
            }
            return Err(CommandError::input_invariant());
        }
        if let Some(child) = retained.child.take() {
            let (key, child_destination) = child.restore();
            if child_destination != crate::state::PendingExpansionChildDestination::Dispatch {
                return Err(CommandError::input_invariant());
            }
            self.scanner_resume = Some(key);
        }
        self.resumed_expansion = Some(retained.resume);
        let delivery_expanded = retained.delivery_expanded;
        self.resume_current_command(&retained.command);
        Ok((
            HotCommand::from_current(retained.command),
            delivery_expanded,
        ))
    }

    #[cold]
    #[inline(never)]
    fn fail_hot_expanded_delivery(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
        depth: u32,
        failure: CommandError,
    ) -> Result<DeliveryStatus, CommandError> {
        destination.take();
        self.command.transient.active_expansion_depth = depth;
        self.invalidate_delivery_freshness();
        Err(failure)
    }

    #[cold]
    #[inline(never)]
    fn fail_expanded_delivery(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        depth: u32,
        failure: CommandError,
    ) -> Result<DeliveryStatus, CommandError> {
        destination.take();
        self.command.transient.active_expansion_depth = depth;
        self.invalidate_delivery_freshness();
        Err(failure)
    }

    /// Parks a completed `\the` target and its scalar child at the one cold
    /// resource boundary.  The synchronous control itself was popped before
    /// scanning the target (register indexes and font selectors have their
    /// own expanded lookahead), so the compact resume payload carries only
    /// the opener provenance needed to finish rendering after retry.
    #[cold]
    #[inline(never)]
    fn park_the_continuation(
        &mut self,
        command: CurrentCommand<G>,
        opener: OriginId,
        delivery_expanded: bool,
        error: CommandError,
        destination: &mut Option<CurrentCommand<G>>,
        depth: u32,
    ) -> Result<DeliveryStatus, CommandError> {
        let child = crate::execution_scratch::ChildContinuation::capture(
            &mut self.scanner_resume,
            crate::state::PendingExpansionChildDestination::Dispatch,
        );
        let pending = crate::state::PendingExpansion {
            command,
            resume: crate::state::PendingExpansionResume::The { opener },
            delivery_expanded,
            child,
        };
        match self.command.scratch.store_expansion_frame(pending) {
            Ok(key) => {
                self.scanner_resume = Some(key);
                self.fail_expanded_delivery(destination, depth, error)
            }
            Err((store_error, mut pending)) => {
                if let Some(child) = pending.take_child()
                    && let Err(failure) = self.abort_continuation(child)
                {
                    return self.fail_expanded_delivery(destination, depth, failure);
                }
                self.fail_expanded_delivery(
                    destination,
                    depth,
                    crate::scan_toks::scratch_command_error(store_error),
                )
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn transition_source_input_frame(
        &mut self,
        resident_index: usize,
        command: &mut HotCommand<G>,
    ) -> Result<ResidentColdOutcome, CommandError> {
        let command_state = &mut *self.command;
        let state = &mut *self.state;
        let create_control_sequences = self.create_source_control_sequences;
        let profile = command_state.roots.profile;
        let force_eof_requested = command_state.roots.input.force_eof;
        #[cfg(test)]
        {
            command_state
                .roots
                .input
                .levels
                .cursor_mutations
                .source_branch_entries = command_state
                .roots
                .input
                .levels
                .cursor_mutations
                .source_branch_entries
                .saturating_add(1);
        }
        let InputLevel::Source(source) = &mut command_state.roots.input.levels.rows[resident_index]
        else {
            return Err(CommandError::input_invariant());
        };
        let slot = command_state
            .roots
            .input
            .levels
            .source_slots
            .resident_value_mut(source.slot.0.slot);
        let mut top = ResidentSourceTop { source, slot };
        let force_eof = top.force_eof(force_eof_requested);
        let identity = top.source.identity();
        let position = top.slot.cursor.next_physical_offset;
        let active_source = top.source.frame.source_context();

        match top
            .advance(profile, force_eof, state, create_control_sequences)
            .map_err(|()| CommandError::input_invariant())?
        {
            ResidentSourceAdvance::Delivered(word, origin, location) => {
                let direct_source_line = top
                    .slot
                    .cursor
                    .line
                    .as_ref()
                    .map(|line| u32::try_from(line.physical.number()).unwrap_or(u32::MAX));
                command_state.last_diagnostic_location = Some(location);
                #[cfg(test)]
                {
                    command_state.raw_delivery_path_counters.source_direct = command_state
                        .raw_delivery_path_counters
                        .source_direct
                        .saturating_add(1);
                }
                let resolution = command.write_resolved_delivery(
                    word,
                    origin,
                    identity.0,
                    position,
                    active_source,
                    true,
                    direct_source_line,
                    false,
                    state,
                );
                #[cfg(feature = "profiling")]
                self.fuel.record_raw_delivery(
                    command_state.delivery_mode.scanner_active(),
                    resolution.meaning_lookup(),
                    crate::fuel::RawDeliveryKind::Source,
                );
                Ok(ResidentColdOutcome::SyntheticCommand {
                    literal_catcode: resolution.literal_catcode(),
                })
            }
            ResidentSourceAdvance::InvalidCharacter => self.transition_input_frame(
                InputFrameTransition::Boundary(ResidentBoundary::InvalidCharacter),
                command,
            ),
            ResidentSourceAdvance::NeedLine(identity) => self.transition_input_frame(
                InputFrameTransition::Boundary(ResidentBoundary::NeedLine(identity)),
                command,
            ),
            ResidentSourceAdvance::Exhausted(identity) => self.transition_input_frame(
                InputFrameTransition::Boundary(ResidentBoundary::SourceExhausted(identity)),
                command,
            ),
        }
    }

    /// Consumes the ordinary-character prefix of a source line for
    /// `main_character_run`. This is the only source transition that borrows
    /// the character consumer; ordinary raw and expanded delivery never carry
    /// that capability.
    #[cold]
    #[inline(never)]
    fn advance_source_character_run(
        &mut self,
        resident_index: usize,
        consume: &mut super::MainLoopCharacterConsumer<'_, G>,
    ) -> Result<Option<u32>, CommandError> {
        let command_state = &mut *self.command;
        let state = &mut *self.state;
        let fuel = &mut *self.fuel;
        let diagnostic_effects = &mut *self.diagnostic_effects;
        let InputLevel::Source(source) = &mut command_state.roots.input.levels.rows[resident_index]
        else {
            return Err(CommandError::input_invariant());
        };
        let slot = command_state
            .roots
            .input
            .levels
            .source_slots
            .resident_value_mut(source.slot.0.slot);
        let mut top = ResidentSourceTop { source, slot };
        if !command_state.delivery_mode.allows_character_run() {
            return Ok(None);
        }
        let run = top
            .advance_character_run(state, |state, ch, origin| {
                fuel.charge()?;
                Ok(consume(state, fuel, diagnostic_effects, ch, origin))
            })
            .map_err(|()| CommandError::input_invariant())?;
        match run {
            ResidentSourceCharacterRun::Unavailable => Ok(None),
            ResidentSourceCharacterRun::Consumed { count } => {
                let line = top
                    .slot
                    .cursor
                    .line
                    .as_ref()
                    .expect("a consumed source run retains its line");
                command_state.last_diagnostic_location = Some(SourceLocation::new(
                    line.physical.source,
                    line.cursor.byte_cursor.saturating_sub(1),
                ));
                #[cfg(feature = "profiling")]
                fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, count);
                Ok(Some(count))
            }
            ResidentSourceCharacterRun::Failed { count, error } => {
                if count != 0 {
                    let line = top
                        .slot
                        .cursor
                        .line
                        .as_ref()
                        .expect("a consumed source prefix retains its line");
                    command_state.last_diagnostic_location = Some(SourceLocation::new(
                        line.physical.source,
                        line.cursor.byte_cursor.saturating_sub(1),
                    ));
                    #[cfg(feature = "profiling")]
                    fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, count);
                }
                Err(error)
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn transition_input_frame(
        &mut self,
        transition: InputFrameTransition<G>,
        command: &mut HotCommand<G>,
    ) -> Result<ResidentColdOutcome, CommandError> {
        let cold = match transition {
            InputFrameTransition::Boundary(boundary) => boundary,
            InputFrameTransition::Source { resident_index } => {
                return self.transition_source_input_frame(resident_index, command);
            }
            InputFrameTransition::ResidentExhausted {
                resident_index,
                identity,
            } => {
                let retirement = self
                    .command
                    .finish_resident_exhaustion(
                        resident_index,
                        identity,
                        &mut self.observer,
                        &mut self.immediate_write_retirement,
                    )
                    .map_err(|()| CommandError::input_invariant())?;
                let Some(retirement) = retirement else {
                    return Ok(ResidentColdOutcome::Retry);
                };
                retirement
            }
            InputFrameTransition::Parameter {
                slot,
                arguments,
                active_source,
            } => {
                #[cfg(test)]
                {
                    self.command
                        .raw_delivery_path_counters
                        .out_parameter_interceptions = self
                        .command
                        .raw_delivery_path_counters
                        .out_parameter_interceptions
                        .saturating_add(1);
                }
                self.command
                    .push_resident_parameter_cursor(
                        slot,
                        arguments,
                        active_source,
                        &mut self.observer,
                    )
                    .map_err(|()| CommandError::input_invariant())?;
                return Ok(ResidentColdOutcome::Retry);
            }
        };
        match cold {
            ResidentBoundary::Empty => {
                observe!(
                    self,
                    CommandObservation::Input(InputRecord {
                        transition: InputTransition::Stop,
                        reason: InputReason::Source,
                        source_name: Some(SourceNameClass::Terminal),
                        source: None,
                        level: 0,
                        position: 0,
                    }),
                );
                match self.raw_end_restarts() {
                    Ok(true) => Ok(ResidentColdOutcome::Retry),
                    Ok(false) => Ok(ResidentColdOutcome::End),
                    Err(failure) => Err(failure),
                }
            }
            ResidentBoundary::InvalidCharacter => {
                self.report_recoverable(
                    INVALID_SOURCE_CHARACTER_DIAGNOSTIC,
                    "Text line contains an invalid character".into(),
                    &[
                        "A funny symbol that I can't read has just been input.",
                        "Continue, and I'll forget that it ever happened.",
                    ],
                );
                Ok(ResidentColdOutcome::Retry)
            }
            ResidentBoundary::NeedLine(identity) => {
                let line = self.acquire_source_line(true)?;
                let exhausted = if line.is_none() {
                    match self.finish_exhausted_source(identity) {
                        Ok(status) => matches!(status, SourceExhaustionStatus::End),
                        Err(failure) => return Err(failure),
                    }
                } else {
                    false
                };
                if exhausted {
                    match self.raw_end_restarts() {
                        Ok(true) => Ok(ResidentColdOutcome::Retry),
                        Ok(false) => Ok(ResidentColdOutcome::End),
                        Err(failure) => Err(failure),
                    }
                } else {
                    Ok(ResidentColdOutcome::Retry)
                }
            }
            ResidentBoundary::SourceExhausted(identity) => {
                #[cfg(test)]
                {
                    self.command
                        .raw_delivery_path_counters
                        .cold_source_retirements = self
                        .command
                        .raw_delivery_path_counters
                        .cold_source_retirements
                        .saturating_add(1);
                }
                let exhausted = match self.finish_exhausted_source(identity) {
                    Ok(status) => matches!(status, SourceExhaustionStatus::End),
                    Err(failure) => return Err(failure),
                };
                if exhausted {
                    match self.raw_end_restarts() {
                        Ok(true) => Ok(ResidentColdOutcome::Retry),
                        Ok(false) => Ok(ResidentColdOutcome::End),
                        Err(failure) => Err(failure),
                    }
                } else {
                    Ok(ResidentColdOutcome::Retry)
                }
            }
            ResidentBoundary::TokenExhausted { identity, .. } => {
                #[cfg(test)]
                {
                    self.command
                        .raw_delivery_path_counters
                        .exhaustion_status_relays = self
                        .command
                        .raw_delivery_path_counters
                        .exhaustion_status_relays
                        .saturating_add(1);
                }
                let Some((index, active_source)) =
                    self.command
                        .input
                        .levels
                        .last()
                        .and_then(|level| match level {
                            level
                                if level
                                    .stored_common()
                                    .is_some_and(|cursor| cursor.identity() == identity) =>
                            {
                                level.stored_common().map(|cursor| {
                                    (
                                        u32::try_from(
                                            level.stored_position().expect("stored row position"),
                                        )
                                        .expect("stored row position fits u32"),
                                        cursor.frame.source_context(),
                                    )
                                })
                            }
                            _ => None,
                        })
                else {
                    return Err(CommandError::input_invariant());
                };
                let handoff = self.retire_input_top(identity)?;
                match handoff {
                    RetirementHandoff::Stop => match self.raw_end_restarts() {
                        Ok(true) => Ok(ResidentColdOutcome::Retry),
                        Ok(false) => Ok(ResidentColdOutcome::End),
                        Err(failure) => Err(failure),
                    },
                    RetirementHandoff::Continue => Ok(ResidentColdOutcome::Retry),
                    RetirementHandoff::Completed(episode) => {
                        Ok(ResidentColdOutcome::ReplayCompleted(episode))
                    }
                    RetirementHandoff::EndV(level) => {
                        let _resolution = command.write_resolved_delivery(
                            TokenWord::pack(self.state.frozen_end_template_token()),
                            OriginId::UNKNOWN,
                            level.0,
                            u64::from(index),
                            active_source,
                            false,
                            None,
                            false,
                            self.state,
                        );
                        #[cfg(feature = "profiling")]
                        self.fuel.record_raw_delivery(
                            self.command.delivery_mode.scanner_active(),
                            _resolution.meaning_lookup(),
                            crate::fuel::RawDeliveryKind::SyntheticEndV,
                        );
                        self.readmit_delivery_stamp(command.delivery_stamp());
                        Ok(ResidentColdOutcome::SyntheticCommand {
                            literal_catcode: _resolution.literal_catcode(),
                        })
                    }
                }
            }
            ResidentBoundary::ReplayCompleted(episode) => {
                Ok(ResidentColdOutcome::ReplayCompleted(episode))
            }
        }
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Settles the semantic conditions represented by the authoritative
    /// delivery-mode word without widening the ordinary hot loops.
    #[cold]
    #[inline(never)]
    fn settle_exceptional_delivery(
        &mut self,
        command: &mut HotCommand<G>,
    ) -> Result<(), CommandError> {
        let mode = self.command.delivery_mode;
        if mode.suppresses_next() {
            command.suppress_expandable();
        }
        if mode.scanner_active() && mode.outer() {
            let mut rich = command.materialize();
            self.check_outer_validity_entry(&mut rich)?;
            *command = HotCommand::from_current(rich);
        } else if mode.alignment_active()
            && matches!(
                command.alignment_adjustment(),
                crate::processor::AlignmentDeliveryAdjustment::None
            )
        {
            self.command.roots.alignment.classify_delimiter(command);
        }
        if mode.observing() {
            self.observe_resident_hot_command(command);
        }
        Ok(())
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    #[inline(always)]
    fn finish_expanded_command(
        &mut self,
        command: &HotCommand<G>,
        delivery_expanded: bool,
    ) -> DeliveryStatus {
        #[cfg(feature = "profiling")]
        self.record_expanded_delivery();
        if self.is_observed() {
            self.observe_expanded_hot_delivery(command);
        }
        if self
            .command
            .alignment
            .needs_hot_closing_brace_recovery(command)
        {
            DeliveryStatus::AlignmentClosingBrace
        } else if delivery_expanded {
            DeliveryStatus::PendingExpanded
        } else {
            DeliveryStatus::Command
        }
    }

    #[doc(hidden)]
    pub fn observe_expanded_delivery(&mut self, command: &CurrentCommand<G>) {
        observe!(self, {
            #[cfg(test)]
            {}
            let (command_name, command_operand) =
                crate::observation::canonical_current_command_identity_for_profile(
                    self.command.profile(),
                    command,
                );
            let spelling = self.observed_command_spelling(command);
            let semantic_operand = crate::observation::canonical_sparse_register_operand(
                self.command.profile(),
                command.meaning(),
            );
            CommandObservation::Command(CommandDeliveryRecord {
                boundary: CommandDeliveryBoundary::Expanded,
                spelling,
                command: command_name,
                command_operand,
                semantic_operand,
                provenance: CommandProvenance::from_stamp(
                    command.delivery_stamp(),
                    self.current_delivery_sequence(),
                    command.origin(),
                    self.direct_source_provenance(command),
                ),
            })
        });
    }

    /// Compact observation counterpart for the scanner-owned expanded
    /// delivery.  The terminal command remains in the hot slot while its
    /// canonical identity, spelling, and provenance are projected into the
    /// observer record.
    fn observe_expanded_hot_delivery(&mut self, command: &HotCommand<G>) {
        observe!(self, {
            #[cfg(test)]
            {}
            let meaning = command.resolved_meaning();
            let (command_name, command_operand) =
                crate::observation::canonical_delivery_identity_for_profile(
                    self.command.profile(),
                    command.identity(),
                    meaning,
                );
            let spelling = self.observed_hot_command_spelling(command);
            let semantic_operand = crate::observation::canonical_sparse_register_operand(
                self.command.profile(),
                meaning,
            );
            CommandObservation::Command(CommandDeliveryRecord {
                boundary: CommandDeliveryBoundary::Expanded,
                spelling,
                command: command_name,
                command_operand,
                semantic_operand,
                provenance: CommandProvenance::from_stamp(
                    command.delivery_stamp(),
                    self.current_delivery_sequence(),
                    command.origin(),
                    self.direct_source_provenance_hot(command),
                ),
            })
        });
    }

    /// TeX82 §375's ``@<Insert a token containing |frozen_endv|@>``:
    ///
    /// ```text
    /// begin cur_tok:=cs_token_flag+frozen_endv; back_input;
    /// end
    /// ```
    ///
    /// This is §366 `expand`'s entire `end_template` case, and the reason
    /// §780 installs *two* frozen `\endtemplate` control sequences: the one
    /// stored in a template (`frozen_end_template`, command code
    /// `end_template`) is `>outer_call`, so §336's `check_outer_validity`
    /// still catches a template that ends inside an unfinished scan, and only
    /// once it has been delivered is it replaced by `frozen_endv`, whose
    /// command code is the ordinary unexpandable `endv`.
    ///
    /// §325's stack-conservation loop stops at a `v_template` level, so the
    /// exhausted template stays on the stack underneath this backup and
    /// retires only after `endv` has been acted on.
    pub(crate) fn insert_frozen_endv(&mut self) -> Result<(), CommandError> {
        let frozen_endv = self.state.frozen_endv_token();
        self.back_input_token(TracedTokenWord::pack(frozen_endv, OriginId::UNKNOWN))
    }

    /// Expands the command in one caller-owned destination, moving that sole
    /// owner into parked work only across an immutable-resource suspension.
    /// Resumption restores the same value into the destination before
    /// continuing, while preserving §367's already emitted trace.
    pub(crate) fn expand_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        mut report_trace: bool,
    ) -> Result<(), CommandError> {
        if self.resumed_expansion.is_none()
            && self.scanner_resume.is_some()
            && !self
                .scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion)
        {
            return Err(CommandError::input_invariant());
        }
        if self.resumed_expansion.is_none()
            && self
                .scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion)
        {
            let wrapper = self
                .scanner_resume
                .take()
                .expect("matched expansion wrapper");
            let key = self
                .command
                .scratch
                .take_expansion_key(wrapper)
                .map_err(crate::scan_toks::scratch_command_error)?;
            let mut retained = self
                .command
                .scratch
                .resume_expansion(key)
                .map_err(crate::scan_toks::scratch_command_error)?;
            if destination.is_some() {
                if let Some(child) = retained.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            if let Some(child) = retained.child.take() {
                let (key, destination) = child.restore();
                if destination != crate::state::PendingExpansionChildDestination::Dispatch {
                    return Err(CommandError::input_invariant());
                }
                self.scanner_resume = Some(key);
            }
            *destination = Some(retained.command);
            self.resumed_expansion = Some(retained.resume);
            self.resume_current_command(
                destination
                    .as_ref()
                    .expect("resumed expansion restores its command destination"),
            );
            report_trace = false;
        }
        let dispatch = match classify_expanded_command(
            destination
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?,
        ) {
            ExpandedCommandAction::Expand(dispatch) => dispatch,
            // Direct callers implement TeX82 §366 `expand`, where the
            // `end_template` branch inserts frozen `endv`; only §380's
            // expanded-delivery classifier handles it inline.
            ExpandedCommandAction::EndTemplate => {
                ExpansionDispatch::Primitive(ExpandablePrimitive::EndTemplate)
            }
            ExpandedCommandAction::Return => return Err(CommandError::input_invariant()),
        };
        self.expand_classified_into(destination, dispatch, report_trace, false)
    }

    /// Executes the dispatch selected by the expanded-delivery classifier
    /// without wrapping and rediscriminating it at the expansion boundary.
    fn expand_classified_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        dispatch: ExpansionDispatch,
        report_trace: bool,
        delivery_expanded: bool,
    ) -> Result<(), CommandError> {
        let mut command = destination
            .take()
            .ok_or_else(CommandError::input_invariant)?;
        let mut command_parked = false;
        let result = self.expand_classified_rich_occupied(
            &mut command,
            dispatch,
            report_trace,
            delivery_expanded,
            &mut command_parked,
        );
        if !command_parked {
            *destination = Some(command);
        }
        result
    }

    /// Expands directly from the delivery loop's continuously occupied
    /// command destination. Only a real resource suspension moves that value
    /// into parked work; every synchronous arm retains the same owner.
    fn expand_classified_occupied(
        &mut self,
        command: &mut HotCommand<G>,
        dispatch: ExpansionDispatch,
        report_trace: bool,
        delivery_expanded: bool,
        command_parked: &mut bool,
    ) -> Result<(), CommandError> {
        if dispatch == ExpansionDispatch::Macro {
            if self.resumed_expansion.is_some() || self.scanner_resume.is_some() {
                return Err(CommandError::input_invariant());
            }
            #[cfg(feature = "profiling")]
            {
                tex_state::measurement::record_hot_core_macro_expansion();
                if self.write_expansion_depth != 0 {
                    self.record_write_expansion();
                }
            }
            let _activated = self.macro_call_hot(command)?;
            return Ok(());
        }

        // Primitive scanners, diagnostics, and genuine suspension are rich
        // semantic boundaries. Macro-to-macro chains never enter this arm.
        let mut rich = command.materialize();
        let result = self.expand_classified_rich_occupied(
            &mut rich,
            dispatch,
            report_trace,
            delivery_expanded,
            command_parked,
        );
        if !*command_parked {
            *command = HotCommand::from_current(rich);
        }
        result
    }

    fn expand_classified_rich_occupied(
        &mut self,
        command: &mut CurrentCommand<G>,
        dispatch: ExpansionDispatch,
        report_trace: bool,
        delivery_expanded: bool,
        command_parked: &mut bool,
    ) -> Result<(), CommandError> {
        let resumed_here = self.resumed_expansion.is_some();
        let mut expansion_resume = self
            .resumed_expansion
            .take()
            .unwrap_or(crate::state::PendingExpansionResume::Dispatch);
        if !resumed_here && self.scanner_resume.is_some() {
            return Err(CommandError::input_invariant());
        }
        #[cfg(feature = "profiling")]
        {
            if !is_ranked_fused_expansion(dispatch) {
                tex_state::measurement::record_hot_core_materialization(
                    tex_state::measurement::HotCoreMaterialization::ExpansionCommand,
                );
            }
            match dispatch {
                ExpansionDispatch::Primitive(primitive) => {
                    tex_state::measurement::record_hot_core_expandable_opcode(
                        usize::try_from(primitive.operand())
                            .expect("expandable primitive operand fits usize"),
                    );
                }
                ExpansionDispatch::Macro => {
                    tex_state::measurement::record_hot_core_macro_expansion();
                }
                ExpansionDispatch::Undefined => {}
            }
        }
        #[cfg(feature = "profiling")]
        if self.write_expansion_depth != 0 {
            self.record_write_expansion();
        }
        // TeX82 §367 traces non-macro expandable commands inside `expand`,
        // before the primitive consumes operands or changes the input stack.
        // Undefined control sequences reach the same branch through §370.
        // Macros and `end_template` take §366's other two branches and do not
        // cross this diagnostic boundary.
        let traceable = matches!(
            dispatch,
            ExpansionDispatch::Primitive(primitive)
                if primitive != ExpandablePrimitive::EndTemplate
        ) || dispatch == ExpansionDispatch::Undefined;
        if report_trace && traceable && self.command.delivery_mode.tracing() {
            self.print_command_trace(crate::PrintCommand::from_current(command));
        }
        let mut suspended_resume = None;
        let result = (|| {
            match dispatch {
                ExpansionDispatch::Macro => {
                    let _activated = self.macro_call(command)?;
                    Ok(())
                }
                ExpansionDispatch::Undefined => {
                    #[cfg(feature = "profiling")]
                    tex_state::measurement::record_hot_core_undefined_expansion();
                    let context = self.command.output_open_context(self.state);
                    self.command.semantic_diagnostics.push(
                        crate::CommandSemanticDiagnostic::UndefinedControlSequence { context },
                    );
                    if !self.command.profile().capabilities().supports_etex() {
                        // TeX82 §370 still owns the recoverable user-visible
                        // error above. The pinned e-TeX 2.6 observer has no
                        // diagnostic seam at that error site, so its detached
                        // event stream advances directly to the next input
                        // transition.
                        self.observe_command_diagnostic("undefined_control_sequence", command);
                    }
                    Ok(())
                }
                ExpansionDispatch::Primitive(primitive)
                    if crate::conditionals::ConditionalKind::from_primitive(primitive)
                        .is_some_and(|kind| {
                            kind != crate::conditionals::ConditionalKind::IfCsName
                        }) =>
                {
                    self.expand_conditional(
                        command,
                        false,
                        &mut expansion_resume,
                        &mut suspended_resume,
                    )
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Unless) => {
                    self.expand_unless(command, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::IfCsName) => {
                    self.begin_ifcsname_continuation(false)
                }
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::Else
                    | ExpandablePrimitive::Or
                    | ExpandablePrimitive::Fi),
                ) => self.expand_conditional_delimiter(command, primitive),
                // TeX82 §375's `end_template` case replaces the inaccessible
                // sentinel that ended a v-template with the distinct frozen
                // `endv` token. Neither sentinel is a user-installable primitive;
                // §780 gives them only frozen control-sequence slots.
                ExpansionDispatch::Primitive(ExpandablePrimitive::EndTemplate) => {
                    self.insert_frozen_endv()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::NoExpand) => {
                    self.expand_noexpand()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::ExpandAfter) => self
                    .command
                    .scratch
                    .push_expandafter_control(command.origin())
                    .map_err(crate::scan_toks::scratch_command_error),
                ExpansionDispatch::Primitive(ExpandablePrimitive::CsName) => {
                    self.begin_csname_continuation(command.origin())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::String) => {
                    self.expand_string(command)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Meaning) => {
                    self.expand_meaning(command)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Number) => {
                    self.expand_number(command, false, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::RomanNumeral) => {
                    self.expand_number(command, true, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::The) => {
                    self.begin_the_continuation(command.origin())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Unexpanded) => {
                    self.expand_unexpanded()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Expanded) => {
                    self.expand_expanded()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Detokenize) => {
                    self.expand_detokenize(command)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Scantokens) => {
                    self.expand_scantokens()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::FontName) => self
                    .expand_fontname(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470's `pdf_font_size_code` conversion prints the
                // selected font size as an ordinary scaled dimension.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfFontSize) => self
                    .expand_pdf_font_size(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470 scans e-TeX's extended box-register domain,
                // then queries typed hlist state for the first non-skipable node
                // at the requested edge.
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::LeftMarginKern
                    | ExpandablePrimitive::RightMarginKern),
                ) => self.expand_margin_kern(
                    command.copy_for_backup(),
                    primitive,
                    &mut expansion_resume,
                    &mut suspended_resume,
                ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::Input) => {
                    self.expand_input(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::EndInput) => {
                    self.expand_endinput()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::JobName) => {
                    self.state.unsupported_host_capability();
                    let job_name = self.host.job_name().to_owned();
                    self.push_rendered_text(&job_name, command.origin());
                    Ok(())
                }
                // e-TeX 2.6 etex.ch §3211 installs `\eTeXrevision` as a
                // `convert` command; §1387 prints the immutable revision string
                // through TeX82 §470's ordinary conversion-token path.
                ExpansionDispatch::Primitive(ExpandablePrimitive::ETeXRevision) => {
                    self.push_rendered_text(".6", command.origin());
                    Ok(())
                }
                // pdfTeX §57.4 exposes the revision suffix independently of the
                // integer `\pdftexversion` parameter.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfTeXRevision) => {
                    self.push_rendered_text("27", command.origin());
                    Ok(())
                }
                // pdftex.web §§494 and 496--498 install `\pdftexbanner` as an
                // operand-free `convert`: `conv_toks` prints the process banner,
                // then returns it through the ordinary `str_toks`/`ins_list`
                // conversion path. `utils.c::makepdftexbanner` appends the pinned
                // TeX Live and kpathsea identities to pdftex.web §2's banner.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfTeXBanner) => {
                    self.push_rendered_text(
                    "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) kpathsea version 6.4.2",
                    command.origin(),
                );
                    Ok(())
                }
                // pdftex.web §§1587--1588 use the ordinary integer scanner for
                // the signed uniform bound, then advance the single checkpointed
                // MetaPost-derived stream shared with the operand-free normal
                // deviate conversion.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfUniformDeviate) => self
                    .expand_pdf_uniform_deviate(
                        command,
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfNormalDeviate) => {
                    let value = self.state.pdf_normal_deviate();
                    self.push_rendered_text(&value.to_string(), command.origin());
                    Ok(())
                }
                // pdftex.web §1590's `pdf_creation_date_code` conversion calls
                // `getcreationdate`, then returns the fixed job-start timestamp
                // through the ordinary `str_toks`/`ins_list` conversion path.
                // Both the LaTeX-compatible `\creationdate` spelling and
                // pdfTeX's `\pdfcreationdate` spelling share this meaning.
                ExpansionDispatch::Primitive(ExpandablePrimitive::CreationDate) => {
                    let clock = self.state.job_clock();
                    self.push_rendered_text(&format_pdf_date(clock, 0), command.origin());
                    Ok(())
                }
                // pdfTeX and XeTeX change section [53a] report shell escape as
                // 0 (disabled), 1 (unrestricted), or 2 (restricted). Umber's
                // LaTeX compatibility spelling is an expandable alias over the
                // same tracked World policy used by `\pdfshellescape`.
                ExpansionDispatch::Primitive(ExpandablePrimitive::ShellEscape) => {
                    let status = self
                        .state
                        .internal_integer(tex_state::meaning::InternalInteger::PdfShellEscape)
                        .expect("the shell-escape status is an integer enquiry");
                    self.push_rendered_text(&status.to_string(), command.origin());
                    Ok(())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::StringCompare) => {
                    self.expand_string_compare(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfEscapeString) => {
                    self.expand_pdf_escape_string(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfEscapeHex) => {
                    self.expand_pdf_escape_hex(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfUnescapeHex) => {
                    self.expand_pdf_unescape_hex(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfColorStackInit) => self
                    .expand_pdf_color_stack_init(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfMatch) => self
                    .expand_pdf_match(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfLastMatch) => self
                    .expand_pdf_last_match(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfFileDump) => self
                    .expand_pdf_file_dump(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::FileSize) => self
                    .expand_pdf_file_size(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfFileModificationDate) => self
                    .expand_pdf_file_modification_date(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfMdFiveSum) => self
                    .expand_pdf_md_five_sum(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfInsertHeight) => self
                    .expand_pdf_insert_height(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470's `pdf_ximage_bbox_code` conversion scans an
                // existing image object before its one-based page-box coordinate.
                // The enquiry reads detached metadata only; it never reserves an
                // image or writer object while expanding.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfXImageBBox) => self
                    .expand_pdf_ximage_bbox(command, &mut expansion_resume, &mut suspended_resume),
                // pdftex.web §1549's `pdf_xform_name_code` conversion scans a
                // form object number and prints its independent resource identity.
                // Unknown object numbers produce zero, matching the other PDF
                // object enquiries rather than manufacturing ledger state.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfXFormName) => self
                    .expand_pdf_xform_name(command, &mut expansion_resume, &mut suspended_resume),
                // pdftex.web §470's `pdf_page_ref_code` conversion scans a one-based
                // shipped-page number and prints its page-object identity. Pages
                // that do not exist yet expand to zero without reserving
                // speculative writer state; nonpositive operands are rejected by
                // the conversion's `pdf_error` guard.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfPageRef) => {
                    self.expand_pdf_page_ref(command, &mut expansion_resume, &mut suspended_resume)
                }
                // pdfTeX §57.1 consumes one raw token and, only for a registered
                // primitive spelling, replays the immutable frozen primitive.
                // The ordinary expanded loop then dispatches that original
                // meaning without consulting the shadowable live cell.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfPrimitive) => {
                    let mut destination = None;
                    match self.get_next_into(&mut destination)? {
                        DeliveryStatus::End => return Err(CommandError::input_invariant()),
                        DeliveryStatus::Command => {}
                        _ => unreachable!("ordinary raw delivery returns only commands"),
                    }
                    let target = destination
                        .take()
                        .expect("command status initializes destination");
                    let Some(symbol) = target.control_sequence() else {
                        return Ok(());
                    };
                    let name = self.state.resolve(symbol);
                    let Some(frozen) = self.state.primitive_token(name) else {
                        return Ok(());
                    };
                    self.back_input_token(TracedTokenWord::pack(frozen, target.origin()))
                }
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::TopMark
                    | ExpandablePrimitive::FirstMark
                    | ExpandablePrimitive::BotMark
                    | ExpandablePrimitive::SplitFirstMark
                    | ExpandablePrimitive::SplitBotMark),
                ) => self.expand_mark(primitive),
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::TopMarks
                    | ExpandablePrimitive::FirstMarks
                    | ExpandablePrimitive::BotMarks
                    | ExpandablePrimitive::SplitFirstMarks
                    | ExpandablePrimitive::SplitBotMarks),
                ) => {
                    self.expand_mark_class(primitive, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(primitive) => {
                    Err(CommandError::UnsupportedExpandablePrimitive(primitive))
                }
            }
        })();
        if result
            .as_ref()
            .is_err_and(CommandError::is_resource_suspension)
        {
            let child = crate::execution_scratch::ChildContinuation::capture(
                &mut self.scanner_resume,
                crate::state::PendingExpansionChildDestination::Dispatch,
            );
            let error = result.expect_err("matched resource suspension");
            let suspended_command = std::mem::replace(command, CurrentCommand::empty());
            *command_parked = true;
            let pending = crate::state::PendingExpansion {
                command: suspended_command,
                resume: suspended_resume
                    .take()
                    .unwrap_or(crate::state::PendingExpansionResume::Dispatch),
                delivery_expanded,
                child,
            };
            return match self.command.scratch.store_expansion_frame(pending) {
                Ok(key) => {
                    self.scanner_resume = Some(key);
                    Err(error)
                }
                Err((store_error, mut pending)) => {
                    if let Some(child) = pending.take_child()
                        && let Err(failure) = self.abort_continuation(child)
                    {
                        return Err(failure);
                    }
                    Err(crate::scan_toks::scratch_command_error(store_error))
                }
            };
        } else if let Some(child) = self.scanner_resume.take() {
            self.abort_continuation(child)?;
            if result.is_ok() {
                return Err(CommandError::input_invariant());
            }
        }
        result
    }

    pub(super) fn retain_expansion_scalar<T>(
        &mut self,
        scan: crate::RetainedScalarScan<G, T>,
        phase: crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<T, CommandError> {
        match scan {
            crate::RetainedScalarScan::Complete(value) => Ok(value),
            crate::RetainedScalarScan::Suspended { error, child } => {
                self.install_scanner_resume(Some(child));
                *suspended = Some(phase);
                Err(error)
            }
            crate::RetainedScalarScan::Failed(error) => Err(error),
        }
    }

    /// Creates one invocation provenance node and atomically exposes its
    /// activation/body ownership pair to the input stack.
    ///
    /// The scalar macro matcher owns argument matching and calls this only
    /// after it has completed every range. Nested invocations use the live
    /// activation chain, not a replay trace, as their provenance parent.
    #[allow(dead_code)] // consumed by the ordered scalar macro matcher issue
    pub(crate) fn push_macro_activation(
        &mut self,
        name: tex_state::interner::Symbol,
        body: tex_state::ResidentMacroBody<G>,
        call_site: OriginId,
        arguments: Option<ArgumentSetId<G>>,
    ) -> InputLevelId {
        let invocation = call_site;
        self.command
            .push_macro_activation(name, body, arguments, invocation)
    }
}

/// TeX82 §1038's raw-accepted set: `letter`, `other_char`, and `char_given`.
///
/// These are exactly the three commands §1034's inner loop can continue on
/// without expanding, so they are the only ones the lookahead delivers
/// straight out of `get_next`.
/// TeX82 §366's `cur_cmd>max_command` test for Umber's resolved command.
///
/// `Meaning::Undefined` normally represents §207's `undefined_cs` command,
/// which is expanded solely to perform §370's diagnostic recovery. A compact
/// out-parameter token also carries that meaning as its invalid-slot recovery,
/// but its command remains `out_param<max_command`; its token spelling keeps
/// the two command identities distinct here.
pub(crate) fn is_expandable_command<G>(command: &CurrentCommand<G>) -> bool {
    let meaning = command.meaning_ref();
    matches!(meaning, ResolvedMeaning::Macro { .. })
        || matches!(meaning, ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive)) if *primitive != ExpandablePrimitive::EndCsName)
        || (matches!(meaning, ResolvedMeaning::Static(Meaning::Undefined))
            && !matches!(command.spelling().semantic_token(), Token::Param(_)))
}

#[cfg(test)]
mod tests;
