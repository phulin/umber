//! Direct, consumer-selected reads of the authoritative input frame.

use super::ReadSite;
use crate::input::{InputStorage, InputWords};
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use std::ops::ControlFlow;
use tex_state::token::{OriginId, TokenWord};

/// The borrowed span has no full processor callback. Its iterator and
/// admission temporaries leave the native stack before recursive expansion.
#[inline(never)]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn consume_span<Word: Copy, Consumer, R, Admission, const PARAMETERS: bool>(
    words: &[Word],
    frame: &mut tex_state::packed_input::InputFrame,
    consumer: &mut Consumer,
    fuel: &mut crate::fuel::CommandFuel,
    project: impl Fn(Word) -> (TokenWord, OriginId),
    admit: &mut impl FnMut(
        &mut Consumer,
        &mut crate::fuel::CommandFuel,
        TokenWord,
        OriginId,
    ) -> ControlFlow<R, Result<(), Admission>>,
) -> Result<Option<ControlFlow<R, (TokenWord, OriginId, Admission)>>, CommandError> {
    for &packed in words {
        let (word, origin) = project(packed);
        if PARAMETERS && word.out_parameter_slot().is_some() {
            return Ok(None);
        }
        frame.advance_resident();
        match admit(consumer, fuel, word, origin) {
            ControlFlow::Continue(Ok(())) => fuel.charge()?,
            ControlFlow::Continue(Err(admission)) => {
                return Ok(Some(ControlFlow::Continue((word, origin, admission))));
            }
            ControlFlow::Break(result) => return Ok(Some(ControlFlow::Break(result))),
        }
    }
    Ok(None)
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// Settles a drained frame or a replacement parameter after a bulk scan.
    pub(crate) fn settle_resident_run_boundary(&mut self) -> Result<bool, CommandError> {
        if self.depleted_input_top().is_some() {
            self.conserve_input_stack()?;
            return Ok(true);
        }
        let levels = &self.command.roots.input.levels;
        let Some(row) = levels.current.as_ref() else {
            return Ok(false);
        };
        let InputStorage::MacroBody(body) = &row.storage else {
            return Ok(false);
        };
        if row.frame.position() >= row.frame.limit()
            || body
                .body
                .word(row.frame.position() as usize)
                .and_then(TokenWord::out_parameter_slot)
                .is_none()
        {
            return Ok(false);
        }
        self.substitute_resident_parameter()?;
        Ok(true)
    }

    /// Selects the physical span once, admits its ordinary prefix in place,
    /// and hands boundary scalars straight to the caller's actual consumer.
    /// Only a cold input transition leaves without consuming a word.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)] // Actual consumer actions and input policies, without an adapter.
    pub(super) fn consume_resident_input<R, Consumer, Admission: Default>(
        &mut self,
        create_control_sequences: bool,
        consumer: &mut Consumer,
        glyph_origins: bool,
        retain: &impl Fn(&Consumer, bool, crate::processor::delivery_mode::DeliveryMode) -> bool,
        interrupt: &mut impl FnMut(&mut Consumer),
        admit: &mut impl FnMut(
            &mut Consumer,
            &mut tex_state::CommandContext<'_, G>,
            &mut crate::fuel::CommandFuel,
            &mut tex_state::diagnostic::DiagnosticEffects,
            &mut crate::processor::alignment::AlignmentDeliveryState<G>,
            &mut Facts,
            u32,
            Option<OriginId>,
            TokenWord,
            OriginId,
            bool,
            crate::fuel::RawDeliveryKind,
            crate::processor::delivery_mode::DeliveryMode,
        ) -> ControlFlow<R, Result<(), Admission>>,
        finish: &mut impl FnMut(
            &mut Consumer,
            &mut Self,
            TokenWord,
            OriginId,
            bool,
            ReadSite,
            crate::fuel::RawDeliveryKind,
            Admission,
        ) -> Result<R, CommandError>,
        finished: &impl Fn(DeliveryStatus) -> Result<R, CommandError>,
    ) -> Result<Option<R>, CommandError> {
        loop {
            if self
                .command
                .roots
                .input
                .levels
                .current
                .as_ref()
                .is_some_and(|row| matches!(row.storage, InputStorage::Source(_)))
            {
                match self.consume_source_input(
                    create_control_sequences,
                    consumer,
                    glyph_origins,
                    retain,
                    interrupt,
                    admit,
                    finish,
                    finished,
                )? {
                    Some(result) => return Ok(Some(result)),
                    None => continue,
                }
            }
            let macro_origin = if glyph_origins {
                self.active_macro_origin()
            } else {
                None
            };
            let levels = &mut self.command.roots.input.levels;
            let Some(row) = levels.current.as_mut() else {
                return Ok(None);
            };
            let frame = &mut row.frame;
            if frame.position() >= frame.limit() {
                let Some(boundary) = self.command.finish_resident_exhaustion(
                    &mut self.observer,
                    &mut self.immediate_write_retirement,
                ) else {
                    continue;
                };
                interrupt(consumer);
                match self.finish_input_boundary(
                    boundary,
                    &mut |processor, word, origin, suppress, site, kind| {
                        finish(
                            consumer,
                            processor,
                            word,
                            origin,
                            suppress,
                            site,
                            kind,
                            Admission::default(),
                        )
                    },
                )? {
                    None => continue,
                    Some(ControlFlow::Continue(result)) => return Ok(Some(result)),
                    Some(ControlFlow::Break(status)) => return finished(status).map(Some),
                }
            }
            let (words, kind) = match &row.storage {
                InputStorage::Source(_) => unreachable!("source loan handled above"),
                InputStorage::MacroBody(body) => (
                    InputWords::Packed(body.body.definition_words()),
                    crate::fuel::RawDeliveryKind::StoredToken,
                ),
                InputStorage::StoredList(list) => (
                    InputWords::Packed(list.words()),
                    crate::fuel::RawDeliveryKind::StoredToken,
                ),
                InputStorage::MacroArgument { region, slot } => (
                    levels.arguments.words(*region, *slot - 1),
                    crate::fuel::RawDeliveryKind::MacroArgument,
                ),
                InputStorage::List(_) => (
                    InputWords::Traced(&levels.words),
                    crate::fuel::RawDeliveryKind::StoredToken,
                ),
            };
            let suppress = frame.flags().contains(
                tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
            );
            let start = frame.start() as usize;
            let position = start
                .checked_add(frame.position() as usize)
                .ok_or_else(CommandError::input_invariant)?;
            let mode = self.command.delivery_mode;
            let (word, origin, admission) = if retain(consumer, suppress, mode) {
                let limit = start
                    .checked_add(frame.limit() as usize)
                    .ok_or_else(CommandError::input_invariant)?;
                let words = words
                    .slice(position..limit)
                    .ok_or_else(CommandError::input_invariant)?;
                let entered_line =
                    u32::try_from(self.command.roots.input.retained_file_line_number).unwrap_or(0);
                let state = &mut *self.state;
                let effects = &mut *self.diagnostic_effects;
                let alignment = &mut self.command.roots.alignment;
                let facts = &mut self.host.facts;
                let admit = &mut {
                    #[inline(always)]
                    |consumer: &mut Consumer, fuel: &mut crate::fuel::CommandFuel, word, origin| {
                        admit(
                            consumer,
                            state,
                            fuel,
                            effects,
                            alignment,
                            facts,
                            entered_line,
                            macro_origin,
                            word,
                            origin,
                            suppress,
                            kind,
                            mode,
                        )
                    }
                };
                // The caller charged the first word. The same traversal admits
                // the whole prefix and charges each next word after acceptance.
                macro_rules! consume_words {
                    ($words:expr, $project:expr, $parameters:expr) => {
                        consume_span::<_, _, _, _, $parameters>(
                            $words, frame, consumer, self.fuel, $project, admit,
                        )
                    };
                }
                let packed = |word| (word, OriginId::UNKNOWN);
                let traced =
                    |word: tex_state::token::TracedTokenWord| (word.token_word(), word.origin());
                let result = match (words, kind) {
                    (InputWords::Packed(words), crate::fuel::RawDeliveryKind::MacroArgument) => {
                        consume_words!(words, packed, false)
                    }
                    (InputWords::Packed(words), _) => consume_words!(words, packed, true),
                    (InputWords::Traced(words), crate::fuel::RawDeliveryKind::MacroArgument) => {
                        consume_words!(words, traced, false)
                    }
                    (InputWords::Traced(words), _) => consume_words!(words, traced, true),
                }?;
                match result {
                    Some(ControlFlow::Continue(boundary)) => boundary,
                    Some(ControlFlow::Break(result)) => return Ok(Some(result)),
                    None => {
                        if frame.position() < frame.limit() {
                            self.substitute_resident_parameter()?;
                        }
                        continue;
                    }
                }
            } else {
                let spelling = words
                    .get(position)
                    .ok_or_else(CommandError::input_invariant)?;
                let word = spelling.token_word();
                if word.out_parameter_slot().is_some()
                    && !matches!(kind, crate::fuel::RawDeliveryKind::MacroArgument)
                {
                    self.substitute_resident_parameter()?;
                    continue;
                }
                frame.advance_resident();
                (word, spelling.origin(), Admission::default())
            };
            return finish(
                consumer,
                self,
                word,
                origin,
                suppress,
                ReadSite::Resident,
                kind,
                admission,
            )
            .map(Some);
        }
    }
}
