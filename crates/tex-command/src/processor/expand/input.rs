//! Source reading and cold transitions of the shared semantic input stack.

#[cfg(test)]
use super::ResidentStorageKind;
use super::{InputFrameTransition, ReadSite, ResidentColdOutcome, ResidentWord};
use crate::input::{
    InputLevel, ResidentBoundary, ResidentSourceAdvance, ResidentSourceCharacterRun,
    ResidentSourceTop, SourceLocation, SourceNameClass,
};
use crate::observation::{CommandObservation, InputReason, InputRecord, InputTransition};
use crate::processor::end_input::{RetirementHandoff, SourceExhaustionStatus};
use crate::processor::{MainCharacterConsumer, MainCharacterInput};
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use tex_state::token::{OriginId, TokenWord};

/// TeX82 §345's invalid source-character report.
const INVALID_SOURCE_CHARACTER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0345;

impl<G> CommandProcessor<'_, '_, G> {
    /// Reads a token from the live source frame; only refill, EOF and recovery
    /// leave through the cold input-transition handler.
    #[inline(always)]
    pub(super) fn advance_source_token(
        &mut self,
        resident_index: usize,
        create_control_sequences: bool,
    ) -> Result<ResidentColdOutcome, CommandError> {
        let command_state = &mut *self.command;
        let state = &mut *self.state;
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
        // The source cursor advances the physical-line pointer when it loads
        // a line, so `next_physical_offset` names the following line rather
        // than this token.  Stamp the token's actual pre-advance byte cursor
        // instead; otherwise every token on one source line would share a
        // coordinate and a backed-up stale copy could pass after a later
        // direct delivery.
        let position = top
            .slot
            .cursor
            .line
            .as_ref()
            .map_or(top.slot.cursor.next_physical_offset, |line| {
                line.cursor.byte_cursor
            });
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
                Ok(ResidentColdOutcome::Word(ResidentWord {
                    word,
                    origin,
                    identity: identity.0,
                    position,
                    active_source,
                    suppress_expandable: false,
                    site: ReadSite::Source(direct_source_line),
                    #[cfg(test)]
                    storage_kind: ResidentStorageKind::Source,
                    #[cfg(feature = "profiling")]
                    raw_kind: crate::fuel::RawDeliveryKind::Source,
                }))
            }
            ResidentSourceAdvance::InvalidCharacter => self.transition_input_frame(
                InputFrameTransition::Boundary(ResidentBoundary::InvalidCharacter),
            ),
            ResidentSourceAdvance::NeedLine(identity) => self.transition_input_frame(
                InputFrameTransition::Boundary(ResidentBoundary::NeedLine(identity)),
            ),
            ResidentSourceAdvance::Exhausted(identity) => self.transition_input_frame(
                InputFrameTransition::Boundary(ResidentBoundary::SourceExhausted(identity)),
            ),
        }
    }

    /// Performs one source-character step for `main_character_run`.
    ///
    /// An eligible physical line lends its retained suffix to `admit` first;
    /// the executor stops at the first lexical or metric boundary. A lexical
    /// first byte falls through to the scalar tokenizer without reopening a
    /// processor or selecting a second top row. Stored source rows and every
    /// cold/source boundary remain on the scalar transition below.
    #[cold]
    #[inline(never)]
    pub(super) fn advance_source_character_step<C: MainCharacterConsumer<G>>(
        &mut self,
        resident_index: usize,
        consume: &mut C,
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

        if let Some(mut run) = top
            .borrow_character_run()
            .map_err(|()| CommandError::input_invariant())?
        {
            let available = usize::try_from(fuel.remaining()).unwrap_or(usize::MAX);
            if available == 0 {
                return Err(fuel.charge().expect_err("zero remaining fuel is exhausted"));
            }
            if run.bytes().len() > available {
                run = run.limit_to(available);
            }
            let run_len = run.bytes().len();
            let admission = consume.admit(
                state,
                fuel,
                diagnostic_effects,
                MainCharacterInput::Borrowed(run),
            );
            let count =
                usize::try_from(admission.count()).map_err(|_| CommandError::input_invariant())?;
            if count > run_len {
                return Err(CommandError::input_invariant());
            }
            if count == 0 && admission.needs_scalar_fallback() {
                // A lexically accepted byte with missing metrics retains the
                // existing direct scalar fallback. The borrowed path never
                // reaches this branch for a non-ASCII or non-ordinary byte.
                let byte = *run
                    .bytes()
                    .first()
                    .ok_or_else(CommandError::input_invariant)?;
                let ch = char::from(byte);
                let origin = run.origin(0);
                fuel.charge()?;
                let scalar = consume.admit(
                    state,
                    fuel,
                    diagnostic_effects,
                    MainCharacterInput::Scalar { ch, origin },
                );
                if scalar.count() != 1 {
                    // A scalar admission error is retained by the caller's
                    // side-channel outcome. Do not move either source cursor
                    // until the consumer has accepted this byte.
                    return Ok(Some(0));
                }
                top.commit_character_run(1)
                    .map_err(|()| CommandError::input_invariant())?;
                let line = top
                    .slot
                    .cursor
                    .line
                    .as_ref()
                    .expect("a scalar fallback retains its line");
                command_state.last_diagnostic_location = Some(SourceLocation::new(
                    line.physical.source,
                    line.cursor.byte_cursor.saturating_sub(1),
                ));
                #[cfg(feature = "profiling")]
                fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, 1);
                return Ok(Some(1));
            }
            if count == 0 && !admission.needs_tokenizer_fallback() {
                // A consumer failure is represented by its surrounding
                // operation error slot. Do not run scalar admission after it:
                // the source cursor and hmode state must remain owned by this
                // same source step until that failure settles.
                return Ok(Some(0));
            }
            if count != 0 {
                let count = u32::try_from(count).map_err(|_| CommandError::input_invariant())?;
                fuel.charge_run(count)?;
                top.commit_character_run(usize::try_from(count).expect("u32 fits usize"))
                    .map_err(|()| CommandError::input_invariant())?;
                let line = top
                    .slot
                    .cursor
                    .line
                    .as_ref()
                    .expect("a committed source run retains its line");
                command_state.last_diagnostic_location = Some(SourceLocation::new(
                    line.physical.source,
                    line.cursor.byte_cursor.saturating_sub(1),
                ));
                #[cfg(feature = "profiling")]
                fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, count);
                return Ok(Some(count));
            }
        }

        let run = top
            .advance_character_run(state, |state, ch, origin| {
                fuel.charge()?;
                Ok(consume
                    .admit(
                        state,
                        fuel,
                        diagnostic_effects,
                        MainCharacterInput::Scalar { ch, origin },
                    )
                    .continue_run())
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
    pub(super) fn finish_resident_eof(&mut self) -> Result<ResidentColdOutcome, CommandError> {
        match self.raw_end_restarts() {
            Ok(true) => Ok(ResidentColdOutcome::Retry),
            Ok(false) => Ok(ResidentColdOutcome::Finished(DeliveryStatus::End)),
            Err(failure) => Err(failure),
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn transition_input_frame(
        &mut self,
        transition: InputFrameTransition<G>,
    ) -> Result<ResidentColdOutcome, CommandError> {
        self.invalidate_delivery_freshness();
        let cold = match transition {
            InputFrameTransition::Boundary(boundary) => boundary,
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
                self.finish_resident_eof()
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
                if line.is_some() {
                    Ok(ResidentColdOutcome::Retry)
                } else if matches!(
                    self.finish_exhausted_source(identity)?,
                    SourceExhaustionStatus::End
                ) {
                    self.finish_resident_eof()
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
                if matches!(
                    self.finish_exhausted_source(identity)?,
                    SourceExhaustionStatus::End
                ) {
                    self.finish_resident_eof()
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
                        Ok(false) => Ok(ResidentColdOutcome::Finished(DeliveryStatus::End)),
                        Err(failure) => Err(failure),
                    },
                    RetirementHandoff::Continue => Ok(ResidentColdOutcome::Retry),
                    RetirementHandoff::Completed(episode) => Ok(ResidentColdOutcome::Finished(
                        DeliveryStatus::ReplayCompleted(episode),
                    )),
                    RetirementHandoff::EndV(level) => Ok(ResidentColdOutcome::Word(ResidentWord {
                        word: TokenWord::pack(self.state.frozen_end_template_token()),
                        origin: OriginId::UNKNOWN,
                        identity: level.0,
                        position: u64::from(index),
                        active_source,
                        suppress_expandable: false,
                        site: ReadSite::Synthetic,
                        #[cfg(test)]
                        storage_kind: ResidentStorageKind::Synthetic,
                        #[cfg(feature = "profiling")]
                        raw_kind: crate::fuel::RawDeliveryKind::SyntheticEndV,
                    })),
                }
            }
            ResidentBoundary::ReplayCompleted(episode) => Ok(ResidentColdOutcome::Finished(
                DeliveryStatus::ReplayCompleted(episode),
            )),
        }
    }
}
