//! Source reading and cold transitions of the shared semantic input stack.

use super::ReadSite;
use crate::input::{
    InputLevel, InputLevelId, ResidentBoundary, ResidentSourceAdvance, ResidentSourceCharacterRun,
    ResidentSourceTop, SourceNameClass,
};
use crate::observation::{CommandObservation, InputReason, InputRecord, InputTransition};
use crate::processor::end_input::{RetirementHandoff, SourceExhaustionStatus};
use crate::processor::{MainCharacterConsumer, MainCharacterInput};
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use std::ops::ControlFlow;
use tex_state::token::{OriginId, TokenWord};

/// TeX82 §345's invalid source-character report.
const INVALID_SOURCE_CHARACTER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0345;

impl<G> CommandProcessor<'_, '_, G> {
    /// Reads a token from the live source frame; only refill, EOF and recovery
    /// leave through the cold input-transition handler.
    #[inline(always)]
    pub(super) fn advance_source_token<R>(
        &mut self,
        resident_index: usize,
        create_control_sequences: bool,
        finish: &mut impl FnMut(
            &mut Self,
            TokenWord,
            OriginId,
            bool,
            ReadSite,
            crate::fuel::RawDeliveryKind,
        ) -> Result<R, CommandError>,
    ) -> Result<Option<ControlFlow<DeliveryStatus, R>>, CommandError> {
        let command_state = &mut *self.command;
        let state = &mut *self.state;
        let profile = command_state.roots.profile;
        let force_eof_requested = command_state.roots.input.force_eof;
        let InputLevel::Source(source) = &mut command_state.roots.input.levels.rows[resident_index]
        else {
            return Err(CommandError::input_invariant());
        };
        let mut top = ResidentSourceTop { source };
        let force_eof = top.force_eof(force_eof_requested);

        match top
            .advance(profile, force_eof, state, create_control_sequences)
            .map_err(|()| CommandError::input_invariant())?
        {
            ResidentSourceAdvance::Delivered(word, origin, location) => {
                let direct_source_line = top
                    .source
                    .slot
                    .cursor
                    .line
                    .as_ref()
                    .map(|line| u32::try_from(line.physical.number()).unwrap_or(u32::MAX));
                self.pending_diagnostic_location = top.diagnostic_location(location);
                finish(
                    self,
                    word,
                    origin,
                    false,
                    ReadSite::Source(direct_source_line),
                    crate::fuel::RawDeliveryKind::Source,
                )
                .map(|result| Some(ControlFlow::Continue(result)))
            }
            ResidentSourceAdvance::InvalidCharacter => {
                self.finish_input_boundary(ResidentBoundary::InvalidCharacter, finish)
            }
            ResidentSourceAdvance::NeedLine(identity) => {
                self.finish_input_boundary(ResidentBoundary::NeedLine(identity), finish)
            }
            ResidentSourceAdvance::Exhausted(identity) => {
                self.finish_input_boundary(ResidentBoundary::SourceExhausted(identity), finish)
            }
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
        let mut top = ResidentSourceTop { source };
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
                if let Some(location) = top.current_diagnostic_location() {
                    command_state.set_last_diagnostic_location(Some(location));
                }
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
                if let Some(location) = top.current_diagnostic_location() {
                    command_state.set_last_diagnostic_location(Some(location));
                }
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
                if let Some(location) = top.current_diagnostic_location() {
                    command_state.set_last_diagnostic_location(Some(location));
                }
                #[cfg(feature = "profiling")]
                fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, count);
                Ok(Some(count))
            }
            ResidentSourceCharacterRun::Failed { count, error } => {
                if count != 0 {
                    if let Some(location) = top.current_diagnostic_location() {
                        command_state.set_last_diagnostic_location(Some(location));
                    }
                    #[cfg(feature = "profiling")]
                    fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, count);
                }
                Err(error)
            }
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn finish_resident_eof<R>(
        &mut self,
    ) -> Result<Option<ControlFlow<DeliveryStatus, R>>, CommandError> {
        match self.raw_end_restarts() {
            Ok(true) => Ok(None),
            Ok(false) => Ok(Some(ControlFlow::Break(DeliveryStatus::End))),
            Err(failure) => Err(failure),
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn advance_input_boundary<R>(
        &mut self,
        create_control_sequences: bool,
        finish: &mut impl FnMut(
            &mut Self,
            TokenWord,
            OriginId,
            bool,
            ReadSite,
            crate::fuel::RawDeliveryKind,
        ) -> Result<R, CommandError>,
    ) -> Result<Option<ControlFlow<DeliveryStatus, R>>, CommandError> {
        let Some(index) = self.command.roots.input.levels.rows.len().checked_sub(1) else {
            return self.finish_input_boundary(ResidentBoundary::Empty, finish);
        };
        if matches!(
            self.command.roots.input.levels.rows[index],
            InputLevel::Source(_)
        ) {
            return self.advance_source_token(index, create_control_sequences, finish);
        }
        let levels = &mut self.command.roots.input.levels;
        let InputLevel::Resident(row) = &mut levels.rows[index] else {
            return Err(CommandError::input_invariant());
        };
        let frame = &mut row.header.frame;
        if frame.position() < frame.limit() {
            let position = frame.start() as usize + frame.position() as usize;
            let word = match &row.storage {
                crate::input::ResidentTokenStorage::MacroBody(body) => {
                    body.body.definition_words().get(position).copied()
                }
                crate::input::ResidentTokenStorage::StoredList(list) => {
                    list.words().get(position).copied()
                }
                crate::input::ResidentTokenStorage::List(_) => {
                    levels.words.get(position).map(|word| word.token_word())
                }
                crate::input::ResidentTokenStorage::MacroArgument { .. } => None,
            }
            .ok_or_else(CommandError::input_invariant)?;
            let slot = word
                .out_parameter_slot()
                .ok_or_else(CommandError::input_invariant)?;
            let (region, span) = match &row.storage {
                crate::input::ResidentTokenStorage::MacroBody(body) => (
                    body.argument_region,
                    body.argument_span(&levels.parameters, slot),
                ),
                _ => (None, None),
            };
            let source = frame.source_context();
            frame.advance_resident();
            self.command
                .push_resident_parameter_cursor(slot, region, span, source, &mut self.observer)
                .map_err(|()| CommandError::input_invariant())?;
            return Ok(None);
        }
        let identity = InputLevelId(frame.identity());
        let Some(boundary) = self.command.finish_resident_exhaustion(
            index,
            identity,
            &mut self.observer,
            &mut self.immediate_write_retirement,
        ) else {
            return Ok(None);
        };
        self.finish_input_boundary(boundary, finish)
    }

    #[cold]
    #[inline(never)]
    fn finish_input_boundary<R>(
        &mut self,
        cold: ResidentBoundary,
        finish: &mut impl FnMut(
            &mut Self,
            TokenWord,
            OriginId,
            bool,
            ReadSite,
            crate::fuel::RawDeliveryKind,
        ) -> Result<R, CommandError>,
    ) -> Result<Option<ControlFlow<DeliveryStatus, R>>, CommandError> {
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
                Ok(None)
            }
            ResidentBoundary::NeedLine(identity) => {
                let line = self.acquire_source_line(true)?;
                if line.is_some() {
                    Ok(None)
                } else if matches!(
                    self.finish_exhausted_source(identity)?,
                    SourceExhaustionStatus::End
                ) {
                    self.finish_resident_eof()
                } else {
                    Ok(None)
                }
            }
            ResidentBoundary::SourceExhausted(identity) => {
                if matches!(
                    self.finish_exhausted_source(identity)?,
                    SourceExhaustionStatus::End
                ) {
                    self.finish_resident_eof()
                } else {
                    Ok(None)
                }
            }
            ResidentBoundary::TokenExhausted { identity, .. } => {
                if !self
                    .command
                    .input
                    .levels
                    .last()
                    .and_then(|level| level.stored_common())
                    .is_some_and(|cursor| cursor.identity() == identity)
                {
                    return Err(CommandError::input_invariant());
                }
                let handoff = self.retire_input_top(identity)?;
                match handoff {
                    RetirementHandoff::Stop => match self.raw_end_restarts() {
                        Ok(true) => Ok(None),
                        Ok(false) => Ok(Some(ControlFlow::Break(DeliveryStatus::End))),
                        Err(failure) => Err(failure),
                    },
                    RetirementHandoff::Continue => Ok(None),
                    RetirementHandoff::Completed(episode) => Ok(Some(ControlFlow::Break(
                        DeliveryStatus::ReplayCompleted(episode),
                    ))),
                    RetirementHandoff::EndV => finish(
                        self,
                        TokenWord::pack(self.state.frozen_end_template_token()),
                        OriginId::UNKNOWN,
                        false,
                        ReadSite::Synthetic,
                        crate::fuel::RawDeliveryKind::SyntheticEndV,
                    )
                    .map(|result| Some(ControlFlow::Continue(result))),
                }
            }
            ResidentBoundary::ReplayCompleted(episode) => Ok(Some(ControlFlow::Break(
                DeliveryStatus::ReplayCompleted(episode),
            ))),
        }
    }
}
