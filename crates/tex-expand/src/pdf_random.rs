use tex_lex::InputStack;
use tex_state::ExpansionState;
use tex_state::token::TracedTokenWord;

use crate::{Dispatch, ExpandError, ExpansionContext, ExpansionMode};

pub(crate) fn execute_uniform(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let scanned =
        crate::scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?;
    if let Some(diagnostic) = scanned.diagnostic() {
        stores.report_expansion_diagnostic(&format!("\n! {diagnostic}.\n"));
    }
    let value = stores.pdf_uniform_deviate(scanned.value());
    Ok(crate::pdf_strings::render_bytes(
        stores,
        value.to_string().as_bytes(),
        context,
    ))
}

pub(crate) fn execute_normal(
    stores: &mut tex_state::ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Dispatch {
    let value = stores.pdf_normal_deviate();
    crate::pdf_strings::render_bytes(stores, value.to_string().as_bytes(), context)
}
