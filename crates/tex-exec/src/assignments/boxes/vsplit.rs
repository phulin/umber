use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::node::Node;
use tex_state::token::TracedTokenWord;

use crate::ExecError;

use super::super::{scan_optional_keyword_x, scan_register_index, scan_scaled};

pub(super) fn scan_vsplit_node(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<Option<Node>, ExecError> {
    let index = scan_register_index(input, stores, execution, context)?;
    if !scan_optional_keyword_x(input, stores, execution, "to")? {
        // TeX.web §1082 inserts the keyword conceptually; keyword scanning
        // has already backed up the first nonmatching token, which is the
        // dimension's first token.
        crate::error_report::report_input_error(
            input,
            stores,
            "Missing `to' inserted",
            &[
                "I'm working on `\\vsplit<box number> to <dimen>';",
                "will look for the <dimen> next.",
            ],
        )?;
    }
    let height = scan_scaled(input, stores, execution, context)?;
    crate::canonical_box_runtime::split_vbox_register(stores, index, height)
}

#[cfg(test)]
mod tests;
