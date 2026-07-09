//! Alignment stomach machinery.

mod preamble;

use tex_expand::ExpansionHooks;
use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::meaning::UnexpandablePrimitive;

use crate::{ExecError, ModeNest};

pub(crate) use preamble::scan_preamble;

pub(crate) fn execute_alignment<S, H>(
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let state = scan_preamble(primitive, input, stores, hooks)?;
    nest.current_list_mut().set_align_state(state);
    Ok(())
}
