//! Synchronous terminals of the sole shared reader.

use crate::{CommandError, CurrentCommand};
use std::ops::ControlFlow;
use tex_state::interner::Symbol;
use tex_state::meaning::MeaningWord;
use tex_state::token::TracedTokenWord;

/// A terminal consumer may execute input-preserving words while the actual
/// resident span remains borrowed. Full processor access ends that borrow.
/// Readiness belongs to this invocation, never to command/checkpoint state.
pub trait NativeWordConsumer<Processor, G> {
    const RESIDENT_TERMINALS: bool = false;

    fn consume(
        &mut self,
        processor: &mut Processor,
        spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError>;

    fn resident_ready(&self) -> bool {
        false
    }

    fn resident_terminal(&mut self, _meaning: &MeaningWord<G>) -> ControlFlow<(), bool> {
        ControlFlow::Continue(false)
    }

    /// Input transitions, expansion and full terminal delivery end any
    /// input-preserving continuation admitted by the previous terminal.
    fn input_boundary(&mut self) {}
}

impl<Processor, G, Consume> NativeWordConsumer<Processor, G> for Consume
where
    Consume: FnMut(
        &mut Processor,
        TracedTokenWord,
        &MeaningWord<G>,
        Option<Symbol>,
        Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError>,
{
    #[inline(always)]
    fn consume(
        &mut self,
        processor: &mut Processor,
        spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError> {
        self(processor, spelling, meaning, control_sequence, exceptional)
    }
}

/// Gives a full processor callback its actual word signature at construction.
/// The callback itself is the consumer; this creates no wrapper or owner.
#[inline(always)]
pub fn terminal_consumer<Processor, G, Consume>(consume: Consume) -> Consume
where
    Consume: FnMut(
        &mut Processor,
        TracedTokenWord,
        &MeaningWord<G>,
        Option<Symbol>,
        Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError>,
{
    consume
}
