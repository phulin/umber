use tex_lex::{InputSource, InputStack};
use tex_state::env::banks::DimenParam;
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

pub(super) fn execute_shipout<S>(
    context: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_, S>,
) -> Result<Option<PreparedDviPage>, ExecError>
where
    S: InputSource,
{
    let node = scan_required_box_node(input, stores, execution, context)?;
    shipout_node(node, input, stores, execution)
}

pub(crate) fn shipout_node<S>(
    node: Node,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_, S>,
) -> Result<Option<PreparedDviPage>, ExecError>
where
    S: InputSource,
{
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
