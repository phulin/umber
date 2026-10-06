//! Synchronous terminals of the sole shared reader.

use crate::{CommandError, CurrentCommand};
use std::ops::ControlFlow;
use tex_state::interner::Symbol;
use tex_state::meaning::MeaningWord;
use tex_state::token::TracedTokenWord;

/// A terminal consumer may execute input-preserving words while the actual
/// resident span remains borrowed, with the actual command context, fuel, diagnostic effects and host loans.
/// Full processor access ends that borrow.
/// Readiness belongs to this invocation, never to command/checkpoint state.
pub trait NativeWordConsumer<Processor, G, Facts> {
    const RESIDENT_TERMINALS: bool = false;
    const RESIDENT_LITERALS: bool = false;

    fn consume(
        &mut self,
        processor: &mut Processor,
        spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
        code: u64,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError>;

    fn resident_ready(&self) -> bool {
        false
    }

    #[allow(clippy::too_many_arguments)] // Disjoint input-preserving execution owners and spelling.
    fn resident_terminal(
        &mut self,
        _context: &mut tex_state::CommandContext<'_, G>,
        _fuel: &mut crate::CommandFuel,
        _effects: &mut tex_state::diagnostic::DiagnosticEffects,
        _facts: &mut Facts,
        _entered_line: u32,
        _macro_origin: Option<tex_state::token::OriginId>,
        _spelling: TracedTokenWord,
        _code: u64,
        _operand: u64,
    ) -> ControlFlow<(), bool> {
        ControlFlow::Continue(false)
    }

    /// Expansion and input transitions invalidate entry facts.
    fn input_boundary(&mut self) {}
}

impl<Processor, G, Facts, Consume> NativeWordConsumer<Processor, G, Facts> for Consume
where
    Consume: FnMut(
        &mut Processor,
        TracedTokenWord,
        &MeaningWord<G>,
        u64,
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
        code: u64,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError> {
        self(
            processor,
            spelling,
            meaning,
            code,
            control_sequence,
            exceptional,
        )
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
        u64,
        Option<Symbol>,
        Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError>,
{
    consume
}
