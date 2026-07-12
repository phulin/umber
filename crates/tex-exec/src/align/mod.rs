//! Alignment stomach machinery.

mod execution;
#[cfg(test)]
pub(crate) use execution::{FinishedAlignment, append_finished_alignment};

mod noalign;
mod packaging;
mod preamble;
mod support;
mod template;
mod widths;

use tex_expand::{ExpansionHooks, ReadRecorder};
use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::token::TracedTokenWord;

use crate::{ExecError, ModeNest};

pub(crate) use preamble::scan_preamble;

pub(crate) fn execute_alignment<S, R, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
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
        let state = scan_preamble(primitive, context, input, stores, hooks)?;
        execution::execute_alignment(state, nest, input, stores, recorder, hooks)
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

pub(crate) fn execute_display_halign<S, R, H>(
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Vec<tex_state::node::Node>, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
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
        let state = scan_preamble(UnexpandablePrimitive::HAlign, context, input, stores, hooks)?;
        execution::execute_alignment_to_nodes(state, nest, input, stores, recorder, hooks)
    })();
    match result {
        Ok(nodes) => {
            input.finish_alignment();
            input.resume_alignment_cell(suspended);
            transaction.commit();
            stores
                .world_mut()
                .trace_execution("alignment", "commit display halign");
            Ok(nodes)
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
