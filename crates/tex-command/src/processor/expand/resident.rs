//! Direct borrowing of the semantic input stack's physical readers.

#[cfg(test)]
use super::ResidentStorageKind;
use super::{ResidentWord, ResidentWordRead};
use crate::CommandProcessor;
use crate::input::{InputLevel, PackedInputFrame, ResidentTokenStorage};
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
) -> Option<(TokenWord, OriginId, u32)> {
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
) -> Option<(TokenWord, OriginId, u32)> {
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

impl<G> CommandProcessor<'_, '_, G> {
    /// Borrows the exposed semantic frame's reader. The frame owns both its
    /// logical position and physical cursor across nested input and rollback;
    /// no parallel storage selector needs refresh or invalidation.
    #[inline(always)]
    pub(super) fn read_resident_word(&mut self) -> ResidentWordRead<G> {
        let command_state = &mut *self.command;
        let Some(resident_index) = command_state.roots.input.levels.top.checked_sub(1) else {
            return ResidentWordRead::NoResident;
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
            InputLevel::Source(_) => return ResidentWordRead::Source { resident_index },
            InputLevel::Resident(row) => row,
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
            _ => ResidentStorageKind::Stored,
        };
        #[cfg(feature = "profiling")]
        let raw_kind = match &row.storage {
            ResidentTokenStorage::MacroArgument(_) => crate::fuel::RawDeliveryKind::MacroArgument,
            _ => crate::fuel::RawDeliveryKind::StoredToken,
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

        let Some((word, origin, position)) = current else {
            return ResidentWordRead::Exhausted {
                resident_index,
                identity: exhausted_identity,
            };
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
                command_state.macro_kernel_counters.body_frame_advances = command_state
                    .macro_kernel_counters
                    .body_frame_advances
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
                    ResidentStorageKind::MacroArgument => {}
                }
                return ResidentWordRead::Parameter {
                    slot,
                    arguments,
                    active_source,
                };
            }
        }
        ResidentWordRead::Word(ResidentWord {
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
}
