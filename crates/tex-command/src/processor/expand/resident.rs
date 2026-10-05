//! Direct, consumer-selected reads of the authoritative input frame.

use super::ReadSite;
use crate::input::{InputStorage, InputWords};
use crate::{CommandError, CommandProcessor};
use std::ops::ControlFlow;
use tex_state::token::{OriginId, TokenWord};

/// The borrowed suffix has no full processor callback. Its iterator and
/// admission temporaries leave the native stack before recursive expansion.
#[inline(always)]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn consume_suffix<Word: Copy, Consumer, R, Admission, const PARAMETERS: bool>(
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

#[inline(never)]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn consume_suffix_outlined<Word: Copy, Consumer, R, Admission, const PARAMETERS: bool>(
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
    consume_suffix::<_, _, _, _, PARAMETERS>(words, frame, consumer, fuel, project, admit)
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
        match self.advance_input_boundary(
            self.create_source_control_sequences,
            &mut |_, _, _, _, _, _| Err::<(), _>(CommandError::input_invariant()),
        )? {
            None => Ok(true),
            Some(_) => Err(CommandError::input_invariant()),
        }
    }

    /// Selects the physical span once, admits its ordinary prefix in place,
    /// and hands boundary scalars straight to the caller's actual consumer.
    /// Only a cold input transition leaves without consuming a word.
    #[inline(always)]
    pub(super) fn consume_resident_input<R, Consumer, Admission: Default>(
        &mut self,
        consumer: &mut Consumer,
        outline_suffix: bool,
        glyph_origins: bool,
        retain: &impl Fn(&Consumer, bool, crate::processor::delivery_mode::DeliveryMode) -> bool,
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
    ) -> Result<Option<R>, CommandError> {
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
            return Ok(None);
        }
        let suppress = frame.flags().contains(
            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
        );
        let (words, kind) = match &row.storage {
            InputStorage::Source(_) => return Ok(None),
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
        let (word, origin, admission) = 'span: {
            let start = frame.start() as usize;
            let position = start
                .checked_add(frame.position() as usize)
                .ok_or_else(CommandError::input_invariant)?;
            let spelling = words
                .get(position)
                .ok_or_else(CommandError::input_invariant)?;
            let word = spelling.token_word();
            let origin = spelling.origin();
            if word.out_parameter_slot().is_some()
                && !matches!(kind, crate::fuel::RawDeliveryKind::MacroArgument)
            {
                return Ok(None);
            }
            frame.advance_resident();
            if !retain(consumer, suppress, self.command.delivery_mode) {
                break 'span (word, origin, Admission::default());
            }
            let entered_line =
                u32::try_from(self.command.roots.input.retained_file_line_number).unwrap_or(0);
            match admit(
                consumer,
                self.state,
                self.fuel,
                self.diagnostic_effects,
                &mut self.command.roots.alignment,
                &mut self.host.facts,
                entered_line,
                macro_origin,
                word,
                origin,
                suppress,
                kind,
                self.command.delivery_mode,
            ) {
                ControlFlow::Continue(Ok(())) => self.fuel.charge()?,
                ControlFlow::Continue(Err(admission)) => break 'span (word, origin, admission),
                ControlFlow::Break(result) => return Ok(Some(result)),
            }
            let limit = start
                .checked_add(frame.limit() as usize)
                .ok_or_else(CommandError::input_invariant)?;
            let words = words
                .slice(position + 1..limit)
                .ok_or_else(CommandError::input_invariant)?;
            let admit = &mut {
                #[inline(always)]
                |consumer: &mut Consumer, fuel: &mut crate::fuel::CommandFuel, word, origin| {
                    admit(
                        consumer,
                        self.state,
                        fuel,
                        self.diagnostic_effects,
                        &mut self.command.roots.alignment,
                        &mut self.host.facts,
                        entered_line,
                        macro_origin,
                        word,
                        origin,
                        suppress,
                        kind,
                        self.command.delivery_mode,
                    )
                }
            };
            // Representation and parameter semantics are selected once for
            // the suffix. Both traverse the actual frame-owned cursor.
            macro_rules! consume_words {
                ($words:expr, $project:expr, $parameters:expr) => {
                    if outline_suffix {
                        consume_suffix_outlined::<_, _, _, _, $parameters>(
                            $words, frame, consumer, self.fuel, $project, admit,
                        )
                    } else {
                        consume_suffix::<_, _, _, _, $parameters>(
                            $words, frame, consumer, self.fuel, $project, admit,
                        )
                    }
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
                Some(ControlFlow::Continue(boundary)) => break 'span boundary,
                Some(ControlFlow::Break(result)) => return Ok(Some(result)),
                None => return Ok(None),
            }
        };
        finish(
            consumer,
            self,
            word,
            origin,
            suppress,
            ReadSite::Resident,
            kind,
            admission,
        )
        .map(Some)
    }
}
