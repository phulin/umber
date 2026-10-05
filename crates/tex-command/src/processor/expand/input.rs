//! Source reading and cold transitions of the shared semantic input stack.

use super::ReadSite;
use crate::input::{
    InputLevel, InputStorage, ResidentBoundary, ResidentSourceAdvance, ResidentSourceCharacterRun,
    SourceNameClass,
};
use crate::observation::{CommandObservation, InputReason, InputRecord, InputTransition};
use crate::processor::end_input::{RetirementHandoff, SourceExhaustionStatus};
use crate::processor::{MainCharacterConsumer, MainCharacterInput};
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use std::ops::ControlFlow;
use tex_state::token::{OriginId, TokenWord};

/// TeX82 §345's invalid source-character report.
const INVALID_SOURCE_CHARACTER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0345;

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// Reads a token from the live source frame; only refill, EOF and recovery
    /// leave through the cold input-transition handler.
    #[inline(always)]
    pub(super) fn advance_source_token<R>(
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
        let command_state = &mut *self.command;
        let state = &mut *self.state;
        let profile = command_state.roots.profile;
        let force_eof_requested = command_state.roots.input.force_eof;
        let InputLevel {
            frame,
            storage: InputStorage::Source(slot),
        } = command_state
            .roots
            .input
            .levels
            .current
            .as_mut()
            .expect("current source input")
        else {
            return Err(CommandError::input_invariant());
        };
        let force_eof = slot.force_eof(force_eof_requested);

        match slot
            .advance(frame, profile, force_eof, state, create_control_sequences)
            .map_err(|()| CommandError::input_invariant())?
        {
            ResidentSourceAdvance::Delivered(word, origin, location) => {
                let direct_source_line = slot
                    .cursor
                    .line
                    .as_ref()
                    .map(|line| u32::try_from(line.physical.number()).unwrap_or(u32::MAX));
                self.pending_diagnostic_location = slot.diagnostic_location(location);
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

    /// Appends a borrowed source prefix through the live character consumer.
    ///
    /// An eligible physical line lends its retained suffix to `admit` first;
    /// the executor stops at the first lexical or metric boundary. A lexical
    /// first byte falls through to the scalar tokenizer without reopening a
    /// processor or selecting a second top row. Stored source rows and every
    /// cold/source boundary remain on the scalar transition below.
    pub(super) fn advance_source_character_step<C: MainCharacterConsumer<G, Facts>>(
        &mut self,
        maximum: usize,
        consume: &mut C,
    ) -> Result<Option<u32>, CommandError> {
        let command_state = &mut *self.command;
        let state = &mut *self.state;
        let fuel = &mut *self.fuel;
        let diagnostic_effects = &mut *self.diagnostic_effects;
        let execution = &mut self.host.facts;
        let InputLevel {
            frame,
            storage: InputStorage::Source(slot),
        } = command_state
            .roots
            .input
            .levels
            .current
            .as_mut()
            .expect("current source input")
        else {
            return Err(CommandError::input_invariant());
        };
        if !command_state.delivery_mode.allows_character_run()
            || state.int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS) > 1
        {
            return Ok(None);
        }

        if let Some(mut run) = slot
            .borrow_character_run()
            .map_err(|()| CommandError::input_invariant())?
        {
            let available = usize::try_from(fuel.remaining())
                .unwrap_or(usize::MAX)
                .min(maximum);
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
                execution,
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
                    execution,
                    MainCharacterInput::Scalar { ch, origin },
                );
                if scalar.count() != 1 {
                    // A scalar admission error is retained by the caller's
                    // side-channel outcome. Do not move either source cursor
                    // until the consumer has accepted this byte.
                    return Ok(Some(0));
                }
                slot.commit_character_run(frame, 1)
                    .map_err(|()| CommandError::input_invariant())?;
                if let Some(location) = slot.current_diagnostic_location() {
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
                slot.commit_character_run(frame, usize::try_from(count).expect("u32 fits usize"))
                    .map_err(|()| CommandError::input_invariant())?;
                if let Some(location) = slot.current_diagnostic_location() {
                    command_state.set_last_diagnostic_location(Some(location));
                }
                #[cfg(feature = "profiling")]
                fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, count);
                return Ok(Some(count));
            }
        }

        let mut remaining = maximum;
        let run = slot
            .advance_character_run(frame, state, |state, ch, origin| {
                fuel.charge()?;
                remaining -= 1;
                Ok(consume
                    .admit(
                        state,
                        fuel,
                        diagnostic_effects,
                        execution,
                        MainCharacterInput::Scalar { ch, origin },
                    )
                    .continue_run()
                    && remaining != 0)
            })
            .map_err(|()| CommandError::input_invariant())?;
        match run {
            ResidentSourceCharacterRun::Unavailable => Ok(None),
            ResidentSourceCharacterRun::Consumed { count } => {
                if let Some(location) = slot.current_diagnostic_location() {
                    command_state.set_last_diagnostic_location(Some(location));
                }
                #[cfg(feature = "profiling")]
                fuel.record_raw_run(false, crate::fuel::RawDeliveryKind::Source, count);
                Ok(Some(count))
            }
            ResidentSourceCharacterRun::Failed { count, error } => {
                if count != 0 {
                    if let Some(location) = slot.current_diagnostic_location() {
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

    #[inline(always)]
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
        if self
            .command
            .roots
            .input
            .levels
            .current
            .as_ref()
            .is_some_and(|row| {
                !matches!(row.storage, InputStorage::Source(_))
                    && row.frame.position() >= row.frame.limit()
            })
        {
            return match self.command.finish_resident_exhaustion(
                &mut self.observer,
                &mut self.immediate_write_retirement,
            ) {
                None => Ok(None),
                Some(boundary) => self.finish_input_boundary(boundary, finish),
            };
        }
        self.advance_source_or_parameter(create_control_sequences, finish)
    }

    #[cold]
    #[inline(never)]
    fn advance_source_or_parameter<R>(
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
        let Some(current) = self.command.roots.input.levels.current.as_ref() else {
            return self.finish_input_boundary(ResidentBoundary::Empty, finish);
        };
        if matches!(
            current,
            InputLevel {
                storage: InputStorage::Source(_),
                ..
            }
        ) {
            return self.advance_source_token(create_control_sequences, finish);
        }
        let levels = &mut self.command.roots.input.levels;
        let row = levels.current.as_mut().expect("current input");
        let frame = &mut row.frame;
        let position = frame.start() as usize + frame.position() as usize;
        let word = match &row.storage {
            InputStorage::Source(_) => None,
            crate::input::InputStorage::MacroBody(body) => {
                body.body.definition_words().get(position).copied()
            }
            crate::input::InputStorage::StoredList(list) => list.words().get(position).copied(),
            crate::input::InputStorage::List(_) => {
                levels.words.get(position).map(|word| word.token_word())
            }
            crate::input::InputStorage::MacroArgument { .. } => None,
        }
        .ok_or_else(CommandError::input_invariant)?;
        let slot = word
            .out_parameter_slot()
            .ok_or_else(CommandError::input_invariant)?;
        let (region, span) = match &row.storage {
            crate::input::InputStorage::MacroBody(body) => (
                body.argument_region,
                body.argument_len(&levels.arguments, slot),
            ),
            _ => (None, None),
        };
        let source = frame.source_context();
        frame.advance_resident();
        self.command
            .push_resident_parameter_cursor(slot, region, span, source, &mut self.observer)
            .map_err(|()| CommandError::input_invariant())?;
        Ok(None)
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
                    .current
                    .as_ref()
                    .filter(|level| !matches!(level.storage, InputStorage::Source(_)))
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
