//! TeX82 §368's actual second operand: expand once, then return to its caller.

use super::{ExpandedCommandAction, ExpandedReaderConsumer};
use crate::{CommandError, CommandProcessor, CurrentCommand, DeliveryStatus};
use std::ops::ControlFlow;

struct OneExpansion;

impl<G, Facts: crate::CommandHostFacts<G>>
    ExpandedReaderConsumer<CommandProcessor<'_, '_, G, Facts>, G> for OneExpansion
{
    const RAW_TOKEN: bool = true;
    const NATIVE_TERMINAL: bool = true;

    fn native_terminal(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G, Facts>,
        spelling: tex_state::token::TracedTokenWord,
        _meaning: &tex_state::meaning::MeaningWord<G>,
        _code: u64,
        _control_sequence: Option<tex_state::interner::Symbol>,
    ) -> Result<ControlFlow<()>, CommandError> {
        processor.back_input_token(spelling)?;
        Ok(ControlFlow::Break(()))
    }

    fn before_action(&mut self, processor: &mut CommandProcessor<'_, '_, G, Facts>) {
        // get_token permits source-name creation while reading, then restores
        // normal delivery before its operand invokes any nested scanner.
        processor.create_source_control_sequences = false;
    }

    fn expanded(&mut self) -> ControlFlow<()> {
        ControlFlow::Break(())
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
        processor.expand_delivered_action(command, action, true)?;
        Ok(true)
    }

    fn recover(
        &mut self,
        _: &mut CommandProcessor<'_, '_, G, Facts>,
        failure: CommandError,
    ) -> Result<(), CommandError> {
        // A failed second operand returns before the first token is replayed.
        Err(failure)
    }

    fn terminal(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G, Facts>,
        destination: &mut Option<CurrentCommand<G>>,
        _: DeliveryStatus,
    ) -> Result<ControlFlow<()>, CommandError> {
        processor.back_input(destination.take().expect("unexpandable operand"))?;
        Ok(ControlFlow::Break(()))
    }
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    pub(in crate::processor) fn expand_next_token(&mut self) -> Result<(), CommandError> {
        debug_assert!(!self.create_source_control_sequences);
        self.create_source_control_sequences = true;
        let result = self.consume_raw_expansion::<false>(&mut OneExpansion);
        self.create_source_control_sequences = false;
        result
    }
}
