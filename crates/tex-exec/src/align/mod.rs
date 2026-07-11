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
    let suspended = input.suspend_alignment_cell();
    let result = (|| {
        let state = scan_preamble(primitive, context, input, stores, hooks)?;
        execution::execute_alignment(state, nest, input, stores, recorder, hooks)
    })();
    match result {
        Ok(()) => {
            input.resume_alignment_cell(suspended);
            Ok(())
        }
        Err(error) => Err(error),
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
    let suspended = input.suspend_alignment_cell();
    let result = (|| {
        let state = scan_preamble(UnexpandablePrimitive::HAlign, context, input, stores, hooks)?;
        execution::execute_alignment_to_nodes(state, nest, input, stores, recorder, hooks)
    })();
    match result {
        Ok(nodes) => {
            input.resume_alignment_cell(suspended);
            Ok(nodes)
        }
        Err(error) => Err(error),
    }
}
