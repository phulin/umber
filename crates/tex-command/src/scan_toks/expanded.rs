//! The actual replacement sink's synchronous actions in the expansion reader.

use super::*;
use crate::DeliveryStatus;
use crate::command::CommandClass;
use crate::processor::expand::{ExpandedCommandAction, ExpandedReaderConsumer, ExpansionDispatch};
use std::ops::ControlFlow;

/// Borrows only the actual destination, replacement grammar and active scanner
/// episode. Nothing survives completion or a resource unwind.
pub(super) struct ReplacementConsumer<'a, G> {
    pub macro_parameters: Option<(u8, Option<Symbol>)>,
    pub episode: &'a ScannerEpisode,
    pub collector: &'a mut TokenCollector<G>,
}

impl<G> ExpandedReaderConsumer<CommandProcessor<'_, '_, G>, G> for ReplacementConsumer<'_, G> {
    const COLLECTING: bool = true;

    fn expand(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G>,
        command: &mut CurrentCommand<G>,
        action: ExpandedCommandAction,
    ) -> Result<bool, CommandError> {
        match action {
            ExpandedCommandAction::Return => return Ok(false),
            ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                ExpandablePrimitive::The,
            )) => {
                let mut operand = None;
                if processor.append_direct_the_toks(self.collector, &mut operand)? {
                    return Ok(true);
                }
            }
            ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                ExpandablePrimitive::Unexpanded,
            )) => {
                processor.append_unexpanded(self.collector)?;
                return Ok(true);
            }
            ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                ExpandablePrimitive::Detokenize,
            )) => {
                processor.append_detokenize(self.collector)?;
                return Ok(true);
            }
            _ => {}
        }
        processor.expand_delivered_action(command, action, true)?;
        Ok(true)
    }

    fn recover(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G>,
        failure: CommandError,
    ) -> Result<(), CommandError> {
        match failure {
            CommandError::ParagraphInMacroArgument => Ok(()),
            CommandError::OuterInMacroArgument => {
                processor.resume_scanner_episode_after_recovery(self.episode);
                Ok(())
            }
            failure => Err(failure),
        }
    }

    fn terminal(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G>,
        destination: &mut Option<CurrentCommand<G>>,
        _status: DeliveryStatus,
    ) -> Result<ControlFlow<()>, CommandError> {
        let command = destination.as_mut().expect("replacement token delivery");
        if command.flags().contains(MeaningFlags::PROTECTED)
            && command.class() == CommandClass::Macro
        {
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
        let token = processor.classify_collector_hot_token(command, None);
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
