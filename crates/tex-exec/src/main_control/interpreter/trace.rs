//! Demanded command tracing reads native meanings and the actual mode owner.
use super::*;

pub(super) fn prepare<G>(processor: &mut Processor<'_, '_, G>) {
    if processor.int_param(IntParam::TRACING_COMMANDS) > 0
        || processor.int_param(IntParam::TRACING_IFS) > 0
    {
        let (mode, shown) = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
            (execution.modes.current_mode(), execution.shown_mode)
        });
        prepare_command_trace(processor, mode, shown);
    }
}

pub(super) fn command<G>(
    processor: &mut Processor<'_, '_, G>,
    code: u64,
    meaning: &tex_state::meaning::MeaningWord<G>,
    symbol: Option<Symbol>,
) {
    if processor.int_param(IntParam::TRACING_COMMANDS) <= 0 {
        if processor.command_trace_printed() {
            processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.shown_mode = Some(execution.modes.current_mode())
            });
        }
        return;
    }
    let (mode, suppress) = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        let mode = execution.modes.current_mode();
        let raw_character = execution.main_loop_active
            && matches!(
                code,
                codes::CHAR_LETTER | codes::CHAR_OTHER | codes::CHAR_GIVEN | codes::PRIMITIVE_CHAR
            );
        let suppress = raw_character
            || execution.boxes.pending_leader.is_some()
            || (execution.boxes.output_routine_opening_pending && code == codes::CHAR_BEGIN_GROUP)
            || (execution.boxes.pending_shipout.is_some()
                && matches!(
                    code,
                    codes::PRIMITIVE_H_BOX | codes::PRIMITIVE_V_BOX | codes::PRIMITIVE_V_TOP
                ));
        (mode, suppress)
    });
    if !suppress {
        processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
            execution.shown_mode = Some(mode)
        });
        processor.print_command_trace(tex_command::PrintCommand::from_native(meaning, symbol));
    }
}
