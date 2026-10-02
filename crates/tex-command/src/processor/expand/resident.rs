//! Direct, consumer-selected reads of the authoritative input frame.

use super::{InputFrameTransition, ReadSite};
use crate::input::{InputLevel, InputLevelId, ResidentTokenStorage};
use crate::{CommandError, CommandProcessor};
use std::ops::ControlFlow;
use tex_state::token::{OriginId, TokenWord};

impl<G> CommandProcessor<'_, '_, G> {
    /// Settles a drained frame or a replacement parameter after a bulk scan.
    pub(crate) fn settle_resident_run_boundary(&mut self) -> Result<bool, CommandError> {
        if self.depleted_input_top().is_some() {
            self.conserve_input_stack()?;
            return Ok(true);
        }
        let levels = &self.command.roots.input.levels;
        let Some(InputLevel::Resident(row)) = levels.rows.last() else {
            return Ok(false);
        };
        let ResidentTokenStorage::MacroBody(body) = &row.storage else {
            return Ok(false);
        };
        if row.header.frame.position() >= row.header.frame.limit()
            || body
                .body
                .word(row.header.frame.position() as usize)
                .and_then(TokenWord::out_parameter_slot)
                .is_none()
        {
            return Ok(false);
        }
        let selected = self.consume_resident_input(
            &mut |_, _, _, _, _, _| ControlFlow::<(), bool>::Continue(false),
            &mut |_, _, _, _, _, _| Err(CommandError::input_invariant()),
        )?;
        let ControlFlow::Continue(transition) = selected else {
            return Err(CommandError::input_invariant());
        };
        match self.transition_input_frame(
            transition,
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
    pub(super) fn consume_resident_input<R>(
        &mut self,
        admit: &mut impl FnMut(
            &mut tex_state::CommandContext<'_, G>,
            &mut crate::fuel::CommandFuel,
            &mut tex_state::diagnostic::DiagnosticEffects,
            &mut crate::processor::alignment::AlignmentDeliveryState<G>,
            TokenWord,
            OriginId,
        ) -> ControlFlow<R, bool>,
        finish: &mut impl FnMut(
            &mut Self,
            TokenWord,
            OriginId,
            bool,
            ReadSite,
            crate::fuel::RawDeliveryKind,
        ) -> Result<R, CommandError>,
    ) -> Result<ControlFlow<R, InputFrameTransition>, CommandError> {
        let levels = &mut self.command.roots.input.levels;
        let Some(resident_index) = levels.rows.len().checked_sub(1) else {
            return Ok(ControlFlow::Continue(InputFrameTransition::Boundary(
                crate::input::ResidentBoundary::Empty,
            )));
        };
        let row = match &mut levels.rows[resident_index] {
            InputLevel::Source(_) => {
                return Ok(ControlFlow::Continue(InputFrameTransition::Source {
                    resident_index,
                }));
            }
            InputLevel::Resident(row) => row,
        };
        let frame = &mut row.header.frame;
        if frame.position() >= frame.limit() {
            return Ok(ControlFlow::Continue(
                InputFrameTransition::ResidentExhausted {
                    resident_index,
                    identity: InputLevelId(frame.identity()),
                },
            ));
        }
        macro_rules! consume_run {
            ($words:expr, $project:expr) => {{
                let words = $words;
                let start = frame.start() as usize;
                loop {
                    let position = frame.position();
                    if position >= frame.limit() {
                        return Ok(ControlFlow::Continue(
                            InputFrameTransition::ResidentExhausted {
                                resident_index,
                                identity: InputLevelId(frame.identity()),
                            },
                        ));
                    }
                    let (word, origin) = $project(words[start + position as usize]);
                    frame.advance_resident();
                    match admit(
                        self.state,
                        self.fuel,
                        self.diagnostic_effects,
                        &mut self.command.roots.alignment,
                        word,
                        origin,
                    ) {
                        ControlFlow::Continue(true) => {}
                        ControlFlow::Continue(false) => break (word, origin),
                        ControlFlow::Break(result) => return Ok(ControlFlow::Break(result)),
                    }
                }
            }};
        }
        let (word, origin) = match &row.storage {
            ResidentTokenStorage::MacroBody(body) => consume_run!(
                body.body.definition_words(),
                |word| (word, OriginId::UNKNOWN)
            ),
            ResidentTokenStorage::StoredList(list) => {
                consume_run!(list.words(), |word| (word, OriginId::UNKNOWN))
            }
            ResidentTokenStorage::MacroArgument { region, .. } => consume_run!(
                levels.arguments.region(*region),
                |word: tex_state::token::TracedTokenWord| (word.token_word(), word.origin())
            ),
            _ => consume_run!(&levels.words, |word: tex_state::token::TracedTokenWord| (
                word.token_word(),
                word.origin()
            )),
        };
        if let Some(slot) = word.out_parameter_slot() {
            let span = match &row.storage {
                ResidentTokenStorage::MacroBody(body) => {
                    Some(body.argument_span(&levels.parameters, slot))
                }
                ResidentTokenStorage::MacroArgument { .. } => None,
                ResidentTokenStorage::List(_) | ResidentTokenStorage::StoredList(_) => Some(None),
            };
            if let Some(span) = span {
                return Ok(ControlFlow::Continue(InputFrameTransition::Parameter {
                    slot,
                    region: match &row.storage {
                        ResidentTokenStorage::MacroBody(body) => body.argument_region,
                        _ => None,
                    },
                    span,
                    active_source: frame.source_context(),
                }));
            }
        }
        let suppress = frame.flags().contains(
            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
        );
        let kind = match row.storage {
            ResidentTokenStorage::MacroArgument { .. } => {
                crate::fuel::RawDeliveryKind::MacroArgument
            }
            _ => crate::fuel::RawDeliveryKind::StoredToken,
        };
        finish(self, word, origin, suppress, ReadSite::Resident, kind).map(ControlFlow::Break)
    }
}
