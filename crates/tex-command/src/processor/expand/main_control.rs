//! Executor-owned terminal selection inside the canonical expanded reader.

use super::TracedTokenWord;

/// Main control executes selected terminal words inside the canonical reader.
/// Selection precedes brace accounting; execution follows it. A declined word
/// remains available to the caller's actual semantic ownership boundary.
pub trait MainControlConsumer<Processor, G> {
    /// Whether the actual consumer owns expansion at horizontal boundaries.
    const EXPAND_BOUNDARIES: bool;

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
    const EXPAND_BOUNDARIES: bool = false;

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
