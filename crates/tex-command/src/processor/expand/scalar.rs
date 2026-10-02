//! Synchronous native-word consumption by the actual continuing scalar scanner.

use super::{ExpandedReaderConsumer, ExpandedReaderRole};
use crate::{CommandError, CommandProcessor, CurrentCommand, DeliveryStatus};
use std::ops::ControlFlow;
use tex_state::interner::Symbol;
use tex_state::meaning::MeaningWord;
use tex_state::token::TracedTokenWord;

type ScalarConsumer<'a, Processor, G> = dyn FnMut(
        &mut Processor,
        TracedTokenWord,
        &MeaningWord<G>,
        Option<Symbol>,
        Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError>
    + 'a;

struct ScalarWords<'a, 'callback, Processor, G> {
    consume: &'a mut ScalarConsumer<'callback, Processor, G>,
}

impl<'state, 'host, G> ExpandedReaderConsumer<CommandProcessor<'state, 'host, G>, G>
    for ScalarWords<'_, '_, CommandProcessor<'state, 'host, G>, G>
{
    const NATIVE_TERMINAL: bool = true;

    fn native_terminal(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G>,
        spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
        control_sequence: Option<Symbol>,
    ) -> Result<ControlFlow<()>, CommandError> {
        (self.consume)(processor, spelling, meaning, control_sequence, None)
    }

    fn terminal(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G>,
        destination: &mut Option<CurrentCommand<G>>,
        status: DeliveryStatus,
    ) -> Result<ControlFlow<()>, CommandError> {
        let command = destination
            .as_ref()
            .ok_or_else(CommandError::input_invariant)?;
        if status == DeliveryStatus::AlignmentEndTemplate {
            processor.begin_scalar_alignment_v_template(command)?;
            return Ok(ControlFlow::Continue(()));
        }
        (self.consume)(
            processor,
            command.spelling(),
            command.native_meaning(),
            command.control_sequence(),
            Some(command),
        )
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    pub(crate) fn consume_expanded_words(
        &mut self,
        consume: &mut ScalarConsumer<'_, Self, G>,
    ) -> Result<(), CommandError> {
        let mut destination = None;
        loop {
            let result = self.consume_expanded_with_mode::<false, false>(
                &mut destination,
                None,
                ExpandedReaderRole::Scanner,
                &mut ScalarWords { consume },
            )?;
            match result {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::End
                | DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentClosingBrace => return Ok(()),
                _ => unreachable!("expanded scanner has no character consumer"),
            }
        }
    }

    /// Ordinary synchronous consumers back up spelling; exceptional deliveries
    /// preserve their actual alignment correction and observed command identity.
    pub(crate) fn back_consumed_word(
        &mut self,
        spelling: TracedTokenWord,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> Result<(), CommandError> {
        if let Some(command) = exceptional {
            self.back_input(*command)
        } else {
            self.back_input_token(spelling)
        }
    }

    pub(crate) fn capture_consumed_site(
        &self,
        spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> tex_state::diagnostic::DiagnosticSite {
        if let Some(command) = exceptional {
            self.capture_diagnostic_site(Some(command))
        } else {
            self.capture_spelled_diagnostic_site(
                crate::command::CommandIdentity::from_meaning(meaning),
                spelling,
                control_sequence,
                meaning.resolve(),
            )
        }
    }
}
