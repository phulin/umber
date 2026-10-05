//! Demanded command tracing reads native meanings and the actual mode owner.
use super::*;

#[inline(always)]
/// Returns the live tracing demand for this terminal entry.
pub(super) fn command<G>(
    processor: &mut Processor<'_, '_, G>,
    code: u64,
    meaning: &tex_state::meaning::MeaningWord<G>,
    symbol: Option<Symbol>,
) -> bool {
    if processor.int_param(IntParam::TRACING_COMMANDS) <= 0 {
        return false;
    }
    print_command(processor, code, meaning, symbol);
    true
}

#[cold]
fn print_command<G>(
    processor: &mut Processor<'_, '_, G>,
    code: u64,
    meaning: &tex_state::meaning::MeaningWord<G>,
    symbol: Option<Symbol>,
) {
    let suppress = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        let raw_character = execution.main_loop_active
            && matches!(
                code,
                codes::CHAR_LETTER | codes::CHAR_OTHER | codes::CHAR_GIVEN | codes::PRIMITIVE_CHAR
            );
        raw_character
            || execution.boxes.pending_leader.is_some()
            || (execution.boxes.output_routine_opening_pending && code == codes::CHAR_BEGIN_GROUP)
    });
    if !suppress {
        processor.print_command_trace(tex_command::PrintCommand::from_native(meaning, symbol));
    }
}
