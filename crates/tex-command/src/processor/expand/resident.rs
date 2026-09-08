//! Direct borrowing of the semantic input stack's physical readers.

#[cfg(test)]
use super::ResidentStorageKind;
use super::{ReadSite, ResidentWord, ResidentWordRead};
use crate::input::{InputLevel, PackedInputFrame, ResidentTokenStorage};
use crate::{CommandError, CommandProcessor, CommandState};
use std::ops::ControlFlow;
use tex_state::token::{OriginId, TokenWord};

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
) -> Option<LoadedWord> {
    let position = frame.position();
    if position >= frame.limit() {
        return None;
    }
    let (word, origin) = load(position)?;
    let consumed = frame.advance_resident();
    debug_assert_eq!(consumed, position);
    Some((word, origin, position))
}

/// Reads one word from an admitted macro replacement cursor.
///
/// The hot path checks the packed frame bound, loads the retained physical
/// slot, advances the frame's sole logical position, and then advances the
/// body's physical cache. A physical crossing is settled by its cold
/// directory transition only after the frame confirms that replacement words
/// remain.
#[inline(always)]
fn next_macro_body_word_from_current_frame<G>(
    frame: &mut PackedInputFrame,
    body: &mut crate::input::MacroBodyCursor<G>,
) -> Option<LoadedWord> {
    let position = frame.position();
    if position >= frame.limit() {
        return None;
    }
    let word = body.body.load_current_word()?;
    let consumed = frame.advance_resident();
    debug_assert_eq!(consumed, position);
    let boundary = body.body.advance_current_word();
    if boundary && frame.position() < frame.limit() {
        body.body.advance_chunk_cold(frame.position());
    }
    Some((word, OriginId::UNKNOWN, position))
}

pub(super) enum ResidentAdmission {
    Continue,
    Stop,
    Boundary,
}

type LoadedWord = (TokenWord, OriginId, u32);

/// Consume words under one storage selection. A rejected word has already
/// advanced the reader and is returned exactly once for semantic settlement.
#[inline(always)]
fn read_selected_run(
    mut load: impl FnMut() -> Option<LoadedWord>,
    admit: &mut impl FnMut(TokenWord, OriginId) -> Result<ResidentAdmission, CommandError>,
    loaded: &mut u64,
) -> Result<ControlFlow<(), Option<LoadedWord>>, CommandError> {
    loop {
        let Some((word, origin, position)) = load() else {
            return Ok(ControlFlow::Continue(None));
        };
        *loaded += 1;
        match admit(word, origin)? {
            ResidentAdmission::Stop => return Ok(ControlFlow::Break(())),
            ResidentAdmission::Continue => {}
            ResidentAdmission::Boundary => {
                return Ok(ControlFlow::Continue(Some((word, origin, position))));
            }
        }
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    #[inline(always)]
    pub(super) fn read_resident_word(&mut self) -> ResidentWordRead<G> {
        match Self::read_resident_run(self.command, |_, _| Ok(ResidentAdmission::Boundary)) {
            Ok(ControlFlow::Continue(read)) => read,
            _ => unreachable!("single-word admission cannot stop or fail"),
        }
    }

    /// Borrows the exposed semantic frame's reader. The frame owns both its
    /// logical position and physical cursor across nested input and rollback;
    /// no parallel storage selector needs refresh or invalidation.
    #[inline(always)]
    pub(super) fn read_resident_run(
        command_state: &mut CommandState<G>,
        mut admit: impl FnMut(TokenWord, OriginId) -> Result<ResidentAdmission, CommandError>,
    ) -> Result<ControlFlow<(), ResidentWordRead<G>>, CommandError> {
        let Some(resident_index) = command_state.roots.input.levels.top.checked_sub(1) else {
            return Ok(ControlFlow::Continue(ResidentWordRead::NoResident));
        };
        #[cfg(test)]
        {
            command_state
                .roots
                .input
                .levels
                .cursor_mutations
                .typed_top_accesses += 1;
            command_state
                .raw_delivery_path_counters
                .resident_transitions += 1;
        }
        let row = match &mut command_state.roots.input.levels.rows[resident_index] {
            InputLevel::Source(_) => {
                return Ok(ControlFlow::Continue(ResidentWordRead::Source {
                    resident_index,
                }));
            }
            InputLevel::Resident(row) => row,
        };
        #[cfg(test)]
        let storage_kind = match &row.storage {
            ResidentTokenStorage::MacroBody(_) => ResidentStorageKind::MacroBody,
            ResidentTokenStorage::MacroArgument(_) => ResidentStorageKind::MacroArgument,
            _ => ResidentStorageKind::Stored,
        };
        #[cfg(feature = "profiling")]
        let raw_kind = match &row.storage {
            ResidentTokenStorage::MacroArgument(_) => crate::fuel::RawDeliveryKind::MacroArgument,
            _ => crate::fuel::RawDeliveryKind::StoredToken,
        };

        let mut loaded = 0;
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
                read_selected_run(
                    || {
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
                    },
                    &mut admit,
                    &mut loaded,
                )
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
                read_selected_run(
                    || {
                        next_word_from_current_frame(&mut row.header.frame, |position| {
                            command_state
                                .attempt
                                .arena()
                                .resident_token_word(list, position as usize)
                                .map(|word| (word.token_word(), word.origin()))
                        })
                    },
                    &mut admit,
                    &mut loaded,
                )
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
                read_selected_run(
                    || {
                        next_word_from_current_frame(&mut row.header.frame, |position| {
                            list.word_at(position as usize)
                                .map(|word| (word, tex_state::token::OriginId::UNKNOWN))
                        })
                    },
                    &mut admit,
                    &mut loaded,
                )
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
                read_selected_run(
                    || next_macro_body_word_from_current_frame(&mut row.header.frame, body),
                    &mut admit,
                    &mut loaded,
                )
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
                read_selected_run(
                    || {
                        next_word_from_current_frame(&mut row.header.frame, |position| {
                            argument.advance_delivery(position, &command_state.scratch)
                        })
                    },
                    &mut admit,
                    &mut loaded,
                )
            }
        };

        #[cfg(test)]
        match storage_kind {
            ResidentStorageKind::Stored => {
                command_state.stored_token_advance_counters.packed_loads = command_state
                    .stored_token_advance_counters
                    .packed_loads
                    .saturating_add(loaded);
                command_state.stored_token_advance_counters.cursor_advances = command_state
                    .stored_token_advance_counters
                    .cursor_advances
                    .saturating_add(loaded);
            }
            ResidentStorageKind::MacroBody => {
                command_state.macro_kernel_counters.body_words = command_state
                    .macro_kernel_counters
                    .body_words
                    .saturating_add(loaded);
                command_state.macro_kernel_counters.body_frame_advances = command_state
                    .macro_kernel_counters
                    .body_frame_advances
                    .saturating_add(loaded);
            }
            ResidentStorageKind::Source | ResidentStorageKind::Synthetic => unreachable!(),
            ResidentStorageKind::MacroArgument => {
                command_state.macro_kernel_counters.argument_words = command_state
                    .macro_kernel_counters
                    .argument_words
                    .saturating_add(loaded);
                command_state.macro_kernel_counters.argument_cursor_advances = command_state
                    .macro_kernel_counters
                    .argument_cursor_advances
                    .saturating_add(loaded);
            }
        }

        let current = match current? {
            ControlFlow::Break(()) => return Ok(ControlFlow::Break(())),
            ControlFlow::Continue(current) => current,
        };
        let exhausted_identity = row.header.identity();
        let identity = exhausted_identity.0;
        let active_source = row.header.frame.source_context();
        let suppress_expandable = row.header.frame.flags().contains(
            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
        );
        let Some((word, origin, position)) = current else {
            return Ok(ControlFlow::Continue(ResidentWordRead::Exhausted {
                resident_index,
                identity: exhausted_identity,
            }));
        };

        if let Some(slot) = word.out_parameter_slot() {
            let arguments = match &row.storage {
                ResidentTokenStorage::MacroBody(body) => Some(body.arguments),
                ResidentTokenStorage::MacroArgument(_) => None,
                ResidentTokenStorage::Replay { .. }
                | ResidentTokenStorage::Durable(_)
                | ResidentTokenStorage::Attempt(_) => Some(None),
            };
            if let Some(arguments) = arguments {
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
                    ResidentStorageKind::MacroArgument
                    | ResidentStorageKind::Source
                    | ResidentStorageKind::Synthetic => {}
                }
                return Ok(ControlFlow::Continue(ResidentWordRead::Parameter {
                    slot,
                    arguments,
                    active_source,
                }));
            }
        }
        Ok(ControlFlow::Continue(ResidentWordRead::Word(
            ResidentWord {
                word,
                origin,
                identity,
                position: u64::from(position),
                active_source,
                suppress_expandable,
                site: ReadSite::Resident,
                #[cfg(test)]
                storage_kind,
                #[cfg(feature = "profiling")]
                raw_kind,
            },
        )))
    }
}
