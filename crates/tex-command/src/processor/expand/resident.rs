//! Direct borrowing of the semantic input stack's physical readers.

use super::{ReadSite, ResidentWord, ResidentWordRead};
use crate::input::{InputLevel, InputLevelId, ResidentTokenStorage};
use crate::{CommandError, CommandProcessor, CommandState};
use std::ops::ControlFlow;
use tex_state::token::{OriginId, TokenWord};

/// One admission decision. It is deliberately infallible: a consumer whose
/// admission can fail records the failure itself and answers `Stop`, so the
/// per-word decision stays a register-sized value on the run's hot loop.
pub(super) enum ResidentAdmission {
    Continue,
    Stop,
    Boundary,
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
        if !matches!(row.storage, ResidentTokenStorage::MacroBody(_)) {
            return Ok(false);
        }
        let frame = row.header.frame;
        if frame.position() >= frame.limit()
            || levels.words[frame.absolute_position()]
                .token_word()
                .out_parameter_slot()
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
    pub(super) fn read_resident_word(&mut self) -> ResidentWordRead {
        match Self::read_resident_run(self.command, |_, _| ResidentAdmission::Boundary) {
            ControlFlow::Continue(read) => read,
            ControlFlow::Break(()) => unreachable!("single-word admission cannot stop"),
        }
    }

    /// Reads the exposed token-list frame: `words[start + position]`, one
    /// bounds check and one frame advance per word whatever owns the list.
    #[inline(always)]
    pub(super) fn read_resident_run(
        command_state: &mut CommandState<G>,
        mut admit: impl FnMut(TokenWord, OriginId) -> ResidentAdmission,
    ) -> ControlFlow<(), ResidentWordRead> {
        let levels = &mut command_state.roots.input.levels;
        let Some(resident_index) = levels.rows.len().checked_sub(1) else {
            return ControlFlow::Continue(ResidentWordRead::NoResident);
        };
        let row = match &mut levels.rows[resident_index] {
            InputLevel::Source(_) => {
                return ControlFlow::Continue(ResidentWordRead::Source { resident_index });
            }
            InputLevel::Resident(row) => row,
        };
        let words = &levels.words;
        let frame = &mut row.header.frame;
        let start = frame.start() as usize;
        let word = loop {
            let position = frame.position();
            if position >= frame.limit() {
                return ControlFlow::Continue(ResidentWordRead::Exhausted {
                    resident_index,
                    identity: InputLevelId(frame.identity()),
                });
            }
            let word = words[start + position as usize];
            frame.advance_resident();
            match admit(word.token_word(), word.origin()) {
                ResidentAdmission::Continue => {}
                ResidentAdmission::Stop => return ControlFlow::Break(()),
                ResidentAdmission::Boundary => break word,
            }
        };
        let (word, origin) = (word.token_word(), word.origin());
        if let Some(slot) = word.out_parameter_slot() {
            let span = match &row.storage {
                ResidentTokenStorage::MacroBody(body) => {
                    Some(body.argument_span(&levels.parameters, slot))
                }
                ResidentTokenStorage::MacroArgument { .. } => None,
                ResidentTokenStorage::List(_) => Some(None),
            };
            if let Some(span) = span {
                return ControlFlow::Continue(ResidentWordRead::Parameter {
                    slot,
                    span,
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
            raw_kind: match row.storage {
                ResidentTokenStorage::MacroArgument { .. } => {
                    crate::fuel::RawDeliveryKind::MacroArgument
                }
                _ => crate::fuel::RawDeliveryKind::StoredToken,
            },
        }))
    }
}
