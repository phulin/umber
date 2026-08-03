//! Retired raw-token delivery bridge.
//!
//! Canonical execution receives commands from `tex-command`. The synchronous
//! compatibility executor still has a few TeX scanners that require one
//! unexpanded semantic token; they cross the lexer boundary only here.

use tex_lex::InputStack;
use tex_state::token::TracedTokenWord;

use crate::ExecError;

pub(crate) fn next_semantic_raw_token(
    input: &mut InputStack,
    universe: &mut tex_state::Universe,
) -> Result<Option<TracedTokenWord>, ExecError> {
    tex_lex::next_semantic_raw_token(input, &mut tex_state::ExpansionContext::new(universe))
        .map_err(Into::into)
}
