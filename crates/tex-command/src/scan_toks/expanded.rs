//! The actual replacement sink's synchronous actions in the expansion reader.

use super::*;
use crate::DeliveryStatus;
use crate::processor::expand::{ExpandedCommandAction, ExpandedReaderConsumer, ExpansionDispatch};
use std::ops::ControlFlow;

/// Borrows only the actual destination, replacement grammar and active scanner
/// episode. Nothing survives completion or a resource unwind.
pub(super) struct ReplacementConsumer<'a, G> {
    pub macro_parameters: Option<(u8, Option<Symbol>)>,
    pub episode: &'a ScannerEpisode,
    pub collector: &'a mut TokenCollector<G>,
}

impl<G, Facts: crate::CommandHostFacts<G>>
    ExpandedReaderConsumer<CommandProcessor<'_, '_, G, Facts>, G, Facts>
    for ReplacementConsumer<'_, G>
{
    const RAW_TOKEN: bool = true;
    const NATIVE_TERMINAL: bool = true;

    fn native_terminal(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G, Facts>,
        spelling: TracedTokenWord,
        _meaning: &tex_state::meaning::MeaningWord<G>,
        _code: u64,
        _control_sequence: Option<Symbol>,
    ) -> Result<ControlFlow<()>, CommandError> {
        // Protected macros are retained as spelling; suppression affects only
        // the demanded observation, which is handled by the cold terminal.
        #[cfg(test)]
        {
            processor
                .command
                .token_collector_path_counters
                .raw_classifications += 1;
        }
        let token = spelling;
        if processor.accept_replacement_word(
            self.macro_parameters,
            self.collector,
            token,
            |processor| processor.back_input_token(spelling),
        )? {
            Ok(ControlFlow::Break(()))
        } else {
            Ok(ControlFlow::Continue(()))
        }
    }

    fn expand(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G, Facts>,
        command: &mut CurrentCommand<G>,
        action: ExpandedCommandAction,
    ) -> Result<bool, CommandError> {
        if matches!(action, ExpandedCommandAction::Return) {
            return Ok(false);
        }
        if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive)) = action
            && self.splice(processor, primitive)?
        {
            return Ok(true);
        }
        processor.expand_delivered_action(command, action, true)?;
        Ok(true)
    }

    fn splice(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G, Facts>,
        primitive: ExpandablePrimitive,
    ) -> Result<bool, CommandError> {
        match primitive {
            ExpandablePrimitive::The => processor.append_direct_the_toks(self.collector),
            ExpandablePrimitive::Unexpanded => {
                processor.append_unexpanded(self.collector)?;
                Ok(true)
            }
            ExpandablePrimitive::Detokenize => {
                processor.append_detokenize(self.collector)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn recover(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G, Facts>,
        failure: CommandError,
    ) -> Result<(), CommandError> {
        match failure.kind {
            crate::CommandErrorKind::ParagraphInMacroArgument => Ok(()),
            crate::CommandErrorKind::OuterInMacroArgument => {
                processor.resume_scanner_episode_after_recovery(self.episode);
                Ok(())
            }
            _ => Err(failure),
        }
    }

    fn terminal(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G, Facts>,
        destination: &mut Option<CurrentCommand<G>>,
        _status: DeliveryStatus,
    ) -> Result<ControlFlow<()>, CommandError> {
        let command = destination.as_mut().expect("replacement token delivery");
        if command.flags().contains(MeaningFlags::PROTECTED) {
            // e-TeX [27.465] retains the spelling as relax/no_expand_flag.
            observe!(
                processor,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "protected_expansion_suppression",
                    tokens: vec![processor.observed_token(command.spelling())],
                }),
            );
            command.suppress_expandable();
        }
        processor.observe_expanded_delivery(command);
        // §23's transient recovery space is not replacement input. The
        // inserted right brace, still owned by the input stack, closes it.
        if command.is_outer_recovery_space() {
            return Ok(ControlFlow::Continue(()));
        }
        #[cfg(test)]
        {
            processor
                .command
                .token_collector_path_counters
                .raw_classifications += 1;
        }
        let token = command.spelling();
        if processor.accept_replacement_word(
            self.macro_parameters,
            self.collector,
            token,
            |processor| {
                processor.back_input(destination.take().expect("rejected replacement token"))
            },
        )? {
            Ok(ControlFlow::Break(()))
        } else {
            Ok(ControlFlow::Continue(()))
        }
    }
}
