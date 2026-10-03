//! Executor-owned terminal selection inside the canonical expanded reader.

use super::{
    CommandError, CurrentCommand, DeliveryStatus, ExpandedReaderConsumer, TracedTokenWord,
};

/// Main control executes selected terminal words inside the canonical reader.
/// Selection precedes brace accounting; execution follows it. A declined word
/// remains available to the caller's actual semantic ownership boundary.
pub trait MainControlConsumer<Processor, G> {
    fn accepts_native_terminal(
        &mut self,
        processor: &mut Processor,
        meaning: &tex_state::meaning::MeaningWord<G>,
    ) -> bool;

    fn consume_native_terminal(
        &mut self,
        processor: &mut Processor,
        spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        control_sequence: Option<tex_state::interner::Symbol>,
    ) -> std::ops::ControlFlow<()>;
}

// Lexical inspection asks for the actual terminal record, without executing it.
impl<Processor, G> MainControlConsumer<Processor, G> for () {
    fn accepts_native_terminal(
        &mut self,
        _: &mut Processor,
        _: &tex_state::meaning::MeaningWord<G>,
    ) -> bool {
        false
    }

    fn consume_native_terminal(
        &mut self,
        _: &mut Processor,
        _: TracedTokenWord,
        _: &tex_state::meaning::MeaningWord<G>,
        _: Option<tex_state::interner::Symbol>,
    ) -> std::ops::ControlFlow<()> {
        unreachable!("lexical inspection requests the terminal record")
    }
}

pub(super) struct MainControlWords<'a, Consumer>(pub(super) &'a mut Consumer);

impl<Processor, G, Consumer: MainControlConsumer<Processor, G>> ExpandedReaderConsumer<Processor, G>
    for MainControlWords<'_, Consumer>
{
    const NATIVE_TERMINAL: bool = true;

    fn accepts_native_terminal(
        &mut self,
        processor: &mut Processor,
        meaning: &tex_state::meaning::MeaningWord<G>,
    ) -> bool {
        self.0.accepts_native_terminal(processor, meaning)
    }

    fn native_terminal(
        &mut self,
        processor: &mut Processor,
        spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        control_sequence: Option<tex_state::interner::Symbol>,
    ) -> Result<std::ops::ControlFlow<()>, CommandError> {
        Ok(self
            .0
            .consume_native_terminal(processor, spelling, meaning, control_sequence))
    }

    fn terminal(
        &mut self,
        _: &mut Processor,
        _: &mut Option<CurrentCommand<G>>,
        _: DeliveryStatus,
    ) -> Result<std::ops::ControlFlow<()>, CommandError> {
        Ok(std::ops::ControlFlow::Break(()))
    }
}
