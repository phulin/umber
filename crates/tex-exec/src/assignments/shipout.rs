use tex_lex::InputStack;
use tex_state::env::banks::DimenParam;
use tex_state::env::banks::IntParam;
use tex_state::node::Node;
use tex_state::token::TracedTokenWord;
use tex_state::{PrintSink, Universe};

use super::scan_required_box_node;
use crate::ExecError;
use crate::dispatch::PreparedDviPage;

mod direct;

// TeX82 map: `ship_out` consumes a box whose child list is visited by
// `hlist_out`/`vlist_out`. Fresh pages use the direct two-phase emitter in
// `direct`: mutation and rare-node normalization finish first, then one live
// compact-list traversal writes canonical artifact bytes and DVI plan bytes.
// No detached node tree or per-list snapshot crosses that traversal.

pub(super) fn execute_shipout(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<PreparedDviPage>, ExecError> {
    let node = scan_required_box_node(input, stores, execution, context)?;
    shipout_node(node, input, stores, execution)
}

pub(crate) fn shipout_node(
    node: Node,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<PreparedDviPage>, ExecError> {
    report_pdf_output_policy_diagnostics(stores);
    if huge_shipout_box(&node, stores) {
        stores.world_mut().write_text(
            PrintSink::TerminalAndLog,
            "\n! Huge page cannot be shipped out.\nThe page just created is more than 18 feet tall or\nmore than 18 feet wide, so I suspect something went wrong.\n",
        );
        return Ok(None);
    }
    let mut transaction = stores.begin_shipout();
    let staged = direct::stage_shipout(node, input, &mut transaction, execution)?;
    let hash = transaction.commit(staged.artifact, staged.effect_pos)?;
    Ok(Some(PreparedDviPage {
        hash,
        plan: staged.dvi_plan,
    }))
}

fn report_pdf_output_policy_diagnostics(stores: &mut Universe) {
    let current_output = stores.int_param(IntParam::PDF_OUTPUT);
    if let Some(fixed) = stores.fixed_pdf_output_parameters() {
        if current_output != fixed.output {
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! pdfTeX error (setup): \\pdfoutput can only be changed before anything is written to the output.\n",
            );
        }
        let current_major = stores.int_param(IntParam::PDF_MAJOR_VERSION);
        let current_minor = stores.int_param(IntParam::PDF_MINOR_VERSION);
        if fixed.output > 0
            && (current_major != fixed.major_version || current_minor != fixed.minor_version)
        {
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! pdfTeX error (setup): PDF version cannot be changed after data is written to the PDF file.\n",
            );
        }
        return;
    }
    if current_output <= 0 {
        return;
    }

    let major = stores.int_param(IntParam::PDF_MAJOR_VERSION);
    if major < 1 {
        stores.world_mut().write_text(
            PrintSink::TerminalAndLog,
            "\n! pdfTeX error (invalid pdfmajorversion).\nThe pdfmajorversion must be 1 or greater.\nI changed this to 1.\n",
        );
        stores.set_int_param(IntParam::PDF_MAJOR_VERSION, 1);
    }
    let minor = stores.int_param(IntParam::PDF_MINOR_VERSION);
    if !(0..=9).contains(&minor) {
        stores.world_mut().write_text(
            PrintSink::TerminalAndLog,
            "\n! pdfTeX error (invalid pdfminorversion).\nThe pdfminorversion must be between 0 and 9.\nI changed this to 4.\n",
        );
        stores.set_int_param(IntParam::PDF_MINOR_VERSION, 4);
    }

    let major = stores.int_param(IntParam::PDF_MAJOR_VERSION);
    let minor = stores.int_param(IntParam::PDF_MINOR_VERSION);
    if stores
        .int_param(IntParam::PDF_OBJ_COMPRESS_LEVEL)
        .clamp(0, 3)
        > 0
        && major == 1
        && minor < 5
    {
        stores.world_mut().write_text(
            PrintSink::TerminalAndLog,
            "\npdfTeX warning (Object streams): \\pdfobjcompresslevel > 0 requires PDF-1.5 or greater. Object streams disabled now.\n",
        );
    }
}

fn huge_shipout_box(node: &Node, stores: &Universe) -> bool {
    let (width, height, depth) = match node {
        Node::HList(box_node) | Node::VList(box_node) => {
            (box_node.width, box_node.height, box_node.depth)
        }
        _ => return false,
    };
    height > tex_state::scaled::Scaled::MAX_DIMEN
        || depth > tex_state::scaled::Scaled::MAX_DIMEN
        || height
            .checked_add(depth)
            .and_then(|value| value.checked_add(stores.dimen_param(DimenParam::V_OFFSET)))
            .is_none_or(|value| value > tex_state::scaled::Scaled::MAX_DIMEN)
        || width
            .checked_add(stores.dimen_param(DimenParam::H_OFFSET))
            .is_none_or(|value| value > tex_state::scaled::Scaled::MAX_DIMEN)
}
