//! Direct borrowing of the semantic input stack's physical readers.

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

/// Reads one word from an admitted macro replacement body: one frame bound
/// check, one slice load, and one frame advance.
#[inline(always)]
fn next_macro_body_word_from_current_frame<G>(
    frame: &mut PackedInputFrame,
    body: &crate::input::MacroBodyCursor<G>,
) -> Option<LoadedWord> {
    let position = frame.position();
    if position >= frame.limit() {
        return None;
    }
    let word = body.body.word_at(position)?;
    let consumed = frame.advance_resident();
    debug_assert_eq!(consumed, position);
    Some((word, OriginId::UNKNOWN, position))
}

/// One admission decision. It is deliberately infallible: a consumer whose
/// admission can fail records the failure itself and answers `Stop`, so the
/// per-word decision stays a register-sized value on the run's hot loop.
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
    admit: &mut impl FnMut(TokenWord, OriginId) -> ResidentAdmission,
    loaded: &mut u64,
) -> ControlFlow<(), Option<LoadedWord>> {
    loop {
        let Some((word, origin, position)) = load() else {
            return ControlFlow::Continue(None);
        };
        *loaded += 1;
        match admit(word, origin) {
            ResidentAdmission::Stop => return ControlFlow::Break(()),
            ResidentAdmission::Continue => {}
            ResidentAdmission::Boundary => {
                return ControlFlow::Continue(Some((word, origin, position)));
            }
        }
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Settles the input transition that ended a resident run, so the
    /// caller's next run resumes on the newly exposed frame: TeX82 §324's
    /// end of a drained token list, or §357's out-parameter reference in a
    /// macro body. Returns whether one was settled; every other boundary is
    /// left to scalar delivery.
    pub(crate) fn settle_resident_run_boundary(&mut self) -> Result<bool, CommandError> {
        if self.depleted_input_top().is_some() {
            self.conserve_input_stack()?;
            return Ok(true);
        }
        let levels = &self.command.roots.input.levels;
        let Some(InputLevel::Resident(row)) = levels
            .rows
            .len()
            .checked_sub(1)
            .map(|top| &levels.rows[top])
        else {
            return Ok(false);
        };
        let ResidentTokenStorage::MacroBody(body) = &row.storage else {
            return Ok(false);
        };
        let position = row.header.frame.position();
        if position >= row.header.frame.limit()
            || body
                .body
                .word_at(position)
                .and_then(|word| word.out_parameter_slot())
                .is_none()
        {
            return Ok(false);
        }
        let selected = self.read_resident_word();
        match self.transition_resident_word(selected, self.create_source_control_sequences)? {
            super::ResidentColdOutcome::Retry => Ok(true),
            _ => Err(CommandError::input_invariant()),
        }
    }

    #[inline(always)]
    pub(super) fn read_resident_word(&mut self) -> ResidentWordRead<G> {
        match Self::read_resident_run(self.command, |_, _| ResidentAdmission::Boundary) {
            ControlFlow::Continue(read) => read,
            ControlFlow::Break(()) => unreachable!("single-word admission cannot stop"),
        }
    }

    /// Borrows the exposed semantic frame's reader. The frame owns both its
    /// logical position and physical cursor across nested input and rollback;
    /// no parallel storage selector needs refresh or invalidation.
    #[inline(always)]
    pub(super) fn read_resident_run(
        command_state: &mut CommandState<G>,
        mut admit: impl FnMut(TokenWord, OriginId) -> ResidentAdmission,
    ) -> ControlFlow<(), ResidentWordRead<G>> {
        let Some(resident_index) = command_state.roots.input.levels.rows.len().checked_sub(1)
        else {
            return ControlFlow::Continue(ResidentWordRead::NoResident);
        };
        let row = match &mut command_state.roots.input.levels.rows[resident_index] {
            InputLevel::Source(_) => {
                return ControlFlow::Continue(ResidentWordRead::Source { resident_index });
            }
            InputLevel::Resident(row) => row,
        };
        #[cfg(feature = "profiling")]
        let raw_kind = match &row.storage {
            ResidentTokenStorage::MacroArgument(_) => crate::fuel::RawDeliveryKind::MacroArgument,
            _ => crate::fuel::RawDeliveryKind::StoredToken,
        };

        let mut loaded = 0;
        let current = match &mut row.storage {
            ResidentTokenStorage::Replay { replay, cursor } => read_selected_run(
                || {
                    next_word_from_current_frame(&mut row.header.frame, |_position| {
                        command_state
                            .roots
                            .input
                            .replay
                            .advance_sequential(*replay, cursor)
                            .map(|word| (word.token_word(), word.origin()))
                    })
                },
                &mut admit,
                &mut loaded,
            ),
            ResidentTokenStorage::Attempt(list) => read_selected_run(
                || {
                    next_word_from_current_frame(&mut row.header.frame, |position| {
                        command_state
                            .attempt
                            .resident_token_word(list, position as usize)
                            .map(|word| (word.token_word(), word.origin()))
                    })
                },
                &mut admit,
                &mut loaded,
            ),
            ResidentTokenStorage::BackedUpWord(word) => {
                let word = *word;
                read_selected_run(
                    || {
                        next_word_from_current_frame(&mut row.header.frame, |position| {
                            (position == 0).then(|| (word.token_word(), word.origin()))
                        })
                    },
                    &mut admit,
                    &mut loaded,
                )
            }
            ResidentTokenStorage::Durable(list) => read_selected_run(
                || {
                    next_word_from_current_frame(&mut row.header.frame, |position| {
                        list.word_at(position as usize)
                            .map(|word| (word, tex_state::token::OriginId::UNKNOWN))
                    })
                },
                &mut admit,
                &mut loaded,
            ),
            ResidentTokenStorage::MacroBody(body) => read_selected_run(
                || next_macro_body_word_from_current_frame(&mut row.header.frame, body),
                &mut admit,
                &mut loaded,
            ),
            ResidentTokenStorage::MacroArgument(argument) => read_selected_run(
                || {
                    next_word_from_current_frame(&mut row.header.frame, |position| {
                        argument.advance_delivery(position, &command_state.scratch)
                    })
                },
                &mut admit,
                &mut loaded,
            ),
        };

        let current = match current {
            ControlFlow::Break(()) => return ControlFlow::Break(()),
            ControlFlow::Continue(current) => current,
        };
        let Some((word, origin, _position)) = current else {
            return ControlFlow::Continue(ResidentWordRead::Exhausted {
                resident_index,
                identity: row.header.identity(),
            });
        };

        if let Some(slot) = word.out_parameter_slot() {
            let arguments = match &row.storage {
                ResidentTokenStorage::MacroBody(body) => Some(body.arguments),
                ResidentTokenStorage::MacroArgument(_) => None,
                ResidentTokenStorage::Replay { .. }
                | ResidentTokenStorage::BackedUpWord(_)
                | ResidentTokenStorage::Durable(_)
                | ResidentTokenStorage::Attempt(_) => Some(None),
            };
            if let Some(arguments) = arguments {
                return ControlFlow::Continue(ResidentWordRead::Parameter {
                    slot,
                    arguments,
                    active_source: row.header.frame.source_context(),
                });
            }
        }
        let suppress_expandable = row.header.frame.flags().contains(
            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
        );
        ControlFlow::Continue(ResidentWordRead::Word(ResidentWord {
            word,
            origin,
            suppress_expandable,
            site: ReadSite::Resident,
            #[cfg(feature = "profiling")]
            raw_kind,
        }))
    }
}
