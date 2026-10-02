//! Native partoken policy and final incomplete-condition reporting.
use super::super::*;

/// Web2C/pdfTeX `partoken.ch` replaces selected direct `end_graf` calls with
/// an inserted `\par` replay only while unrestricted horizontal mode is
/// current. Context one covers vertical boxes; context two adds insertion,
/// output, alignment-item, and no-align boundaries.
pub(in crate::main_control) fn partoken_context_replays<
    G,
    Facts: tex_command::CommandHostFacts<G>,
>(
    processor: &mut CommandProcessor<'_, '_, G, Facts>,
    mode: Mode,
    threshold: i32,
) -> bool {
    mode == Mode::Horizontal && processor.int_param(IntParam::PAR_TOKEN_CONTEXT) >= threshold
}

/// TeX82 §1335 reports and frees unfinished conditionals innermost-first.
pub(in crate::main_control) fn report_incomplete_conditions<G>(
    stores: &mut Universe<G>,
    incomplete: impl IntoIterator<Item = tex_command::IncompleteCondition>,
) {
    let mut printer = stores.printer();
    for condition in incomplete {
        printer
            .print_nl("(")
            .print_esc("end occurred ")
            .print("when ")
            .print_esc(condition.kind_name());
        if condition.source_line() != 0 {
            printer
                .print(" on line ")
                .print_int(i32::try_from(condition.source_line()).unwrap_or(i32::MAX));
        }
        printer.print(" was incomplete)");
    }
}
