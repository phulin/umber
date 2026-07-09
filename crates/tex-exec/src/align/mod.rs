//! Alignment stomach machinery.

mod execution;
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
    let state = scan_preamble(primitive, context, input, stores, hooks)?;
    execution::execute_alignment(state, nest, input, stores, recorder, hooks)
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
    let state = scan_preamble(UnexpandablePrimitive::HAlign, context, input, stores, hooks)?;
    execution::execute_alignment_to_nodes(state, nest, input, stores, recorder, hooks)
}
