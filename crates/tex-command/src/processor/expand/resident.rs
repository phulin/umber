//! Direct, consumer-selected reads of the authoritative input frame.

use super::ReadSite;
use crate::input::{InputLevel, ResidentTokenStorage};
use crate::{CommandError, CommandProcessor};
use std::ops::ControlFlow;
use tex_state::token::{OriginId, TokenWord};

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
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
    pub(super) fn consume_resident_input<R>(
        &mut self,
        reserve: &mut impl FnMut(&mut crate::fuel::CommandFuel) -> Result<(), CommandError>,
        admit: &mut impl FnMut(
            &mut tex_state::CommandContext<'_, G>,
            &mut crate::fuel::CommandFuel,
            &mut tex_state::diagnostic::DiagnosticEffects,
            &mut crate::processor::alignment::AlignmentDeliveryState<G>,
            &mut Facts,
            TokenWord,
            OriginId,
            bool,
            crate::fuel::RawDeliveryKind,
            crate::processor::delivery_mode::DeliveryMode,
        ) -> ControlFlow<R, bool>,
        finish: &mut impl FnMut(
            &mut Self,
            TokenWord,
            OriginId,
            bool,
            ReadSite,
            crate::fuel::RawDeliveryKind,
        ) -> Result<R, CommandError>,
    ) -> Result<Option<R>, CommandError> {
        let levels = &mut self.command.roots.input.levels;
        let Some(row) = levels.rows.last_mut() else {
            return Ok(None);
        };
        let InputLevel::Resident(row) = row else {
            return Ok(None);
        };
        let frame = &mut row.frame;
        if frame.position() >= frame.limit() {
            return Ok(None);
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
        macro_rules! consume_run {
            ($words:expr, $project:expr, $parameters:expr) => {{
                let words = $words;
                let start = frame.start() as usize;
                loop {
                    let position = frame.position();
                    if position >= frame.limit() {
                        return Ok(None);
                    }
                    let (word, origin) = $project(words[start + position as usize]);
                    if $parameters && word.out_parameter_slot().is_some() {
                        // The owning cold parameter call advances this position
                        // exactly once before attempting substitution.
                        return Ok(None);
                    }
                    reserve(self.fuel)?;
                    frame.advance_resident();
                    match admit(
                        self.state,
                        self.fuel,
                        self.diagnostic_effects,
                        &mut self.command.roots.alignment,
                        &mut self.host.facts,
                        word,
                        origin,
                        suppress,
                        kind,
                        self.command.delivery_mode,
                    ) {
                        ControlFlow::Continue(true) => {}
                        ControlFlow::Continue(false) => break (word, origin),
                        ControlFlow::Break(result) => return Ok(Some(result)),
                    }
                }
            }};
        }
        let (word, origin) = match &row.storage {
            ResidentTokenStorage::MacroBody(body) => consume_run!(
                body.body.definition_words(),
                |word| (word, OriginId::UNKNOWN),
                true
            ),
            ResidentTokenStorage::StoredList(list) => {
                consume_run!(list.words(), |word| (word, OriginId::UNKNOWN), true)
            }
            ResidentTokenStorage::MacroArgument { region, slot } => match levels
                .arguments
                .words(*region, *slot - 1)
            {
                crate::input::ArgumentWords::Traced(words) => consume_run!(
                    words,
                    |word: tex_state::token::TracedTokenWord| (word.token_word(), word.origin()),
                    false
                ),
                crate::input::ArgumentWords::Definition(words) => {
                    consume_run!(words, |word| (word, OriginId::UNKNOWN), false)
                }
            },
            _ => consume_run!(
                &levels.words,
                |word: tex_state::token::TracedTokenWord| (word.token_word(), word.origin()),
                true
            ),
        };
        finish(self, word, origin, suppress, ReadSite::Resident, kind).map(Some)
    }
}
