//! Alignment stomach machinery.

mod execution;
pub(crate) use execution::FinishedAlignment;
#[cfg(test)]
pub(crate) use execution::append_finished_alignment;
pub(crate) use execution::{DoEndV, do_endv};

mod noalign;
mod packaging;
mod preamble;
mod support;
mod template;
mod widths;

use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::token::TracedTokenWord;

use crate::{ExecError, ModeNest};

pub(crate) use preamble::scan_preamble;

pub(crate) fn execute_alignment(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    if stores.world().execution_tracing_enabled() {
        stores
            .world_mut()
            .trace_execution("alignment", format!("begin {primitive:?}"));
    }
    let suspended = input.suspend_alignment_cell();
    input.begin_alignment();
    let mut transaction = crate::transaction::ExecutionTransaction::begin(nest, stores);
    let result = (|| {
        let (nest, stores) = transaction.parts();
        let state = scan_preamble(primitive, context, input, stores, execution)?;
        execution::execute_alignment(state, nest, input, stores, execution)
    })();
    match result {
        Ok(()) => {
            input.finish_alignment();
            input.resume_alignment_cell(suspended);
            transaction.commit();
            stores.world_mut().trace_execution("alignment", "commit");
            Ok(())
        }
        Err(error) => {
            input.abort_alignment_and_resume(suspended);
            drop(transaction);
            stores.world_mut().trace_execution("alignment", "rollback");
            Err(error)
        }
    }
}

pub(crate) fn execute_display_halign(
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<FinishedAlignment, ExecError> {
    if stores.world().execution_tracing_enabled() {
        stores
            .world_mut()
            .trace_execution("alignment", "begin display halign");
    }
    let suspended = input.suspend_alignment_cell();
    input.begin_alignment();
    let mut transaction = crate::transaction::ExecutionTransaction::begin(nest, stores);
    let result = (|| {
        let (nest, stores) = transaction.parts();
        let state = scan_preamble(
            UnexpandablePrimitive::HAlign,
            context,
            input,
            stores,
            execution,
        )?;
        execution::execute_alignment_to_nodes(state, nest, input, stores, execution)
    })();
    match result {
        Ok(finished) => {
            input.finish_alignment();
            input.resume_alignment_cell(suspended);
            transaction.commit();
            stores
                .world_mut()
                .trace_execution("alignment", "commit display halign");
            Ok(finished)
        }
        Err(error) => {
            input.abort_alignment_and_resume(suspended);
            drop(transaction);
            stores
                .world_mut()
                .trace_execution("alignment", "rollback display halign");
            Err(error)
        }
    }
}
