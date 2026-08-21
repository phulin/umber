//! Shared vertical-list splitting helpers for insertions and `\vsplit`.

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::glue::GlueSpec;
use tex_state::node::{BoxNode, GlueKind, Node, Whatsit};
use tex_state::node_arena::{NodeRef, PageListId};
use tex_state::scaled::Scaled;
use tex_typeset::{INF_BAD, PackSpec, VpackParams};

use crate::ExecError;

pub(crate) fn prune_page_top<G>(
    stores: &mut CommandContext<'_, G>,
    nodes: Vec<Node>,
    split_top_skip: GlueSpec,
) -> Vec<Node> {
    prune_page_top_with_discards(stores, nodes, split_top_skip).0
}

pub(crate) fn prune_page_top_with_discards<G>(
    stores: &mut CommandContext<'_, G>,
    nodes: Vec<Node>,
    split_top_skip: GlueSpec,
) -> (Vec<Node>, Vec<Node>) {
    let mut out = Vec::new();
    let mut discarded = Vec::new();
    let mut inserted_top_skip = false;
    for node in nodes {
        match &node {
            Node::HList(_) | Node::VList(_) | Node::Rule { .. } if !inserted_top_skip => {
                let top_skip = split_top_skip;
                let adjusted = GlueSpec {
                    width: top_skip
                        .width
                        .checked_sub(vertical_height(&node))
                        .filter(|width| width.raw() > 0)
                        .unwrap_or_else(|| Scaled::from_raw(0)),
                    stretch: top_skip.stretch,
                    stretch_order: top_skip.stretch_order,
                    shrink: top_skip.shrink,
                    shrink_order: top_skip.shrink_order,
                };
                let spec = adjusted;
                out.push(Node::Glue {
                    spec,
                    kind: GlueKind::SplitTopSkip,
                    leader: None,
                });
                out.push(node);
                inserted_top_skip = true;
            }
            _ if !inserted_top_skip && is_page_top_discardable(&node) => {
                discarded.push(node);
            }
            _ => out.push(node),
        }
    }
    (out, discarded)
}

/// TeX82 §969's discardable page-top material plus pdfTeX §1378's snap node.
pub(crate) fn is_page_top_discardable(node: &Node) -> bool {
    matches!(
        node,
        Node::Glue { .. }
            | Node::Kern { .. }
            | Node::Penalty(_)
            | Node::Whatsit(Whatsit::PdfSnapY { .. })
    )
}

pub(crate) fn natural_vlist_size<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    content: PageListId,
) -> Result<Scaled, ExecError> {
    let packed = vpack_natural(stores, diagnostic_effects, diagnostic_context, content);
    packed
        .height
        .checked_add(packed.depth)
        .ok_or(ExecError::ArithmeticOverflow)
}

pub(crate) fn vpack_natural<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    content: PageListId,
) -> BoxNode {
    crate::packing_params::vpack(
        stores,
        diagnostic_effects,
        diagnostic_context,
        content,
        PackSpec::Natural,
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: Scaled::MAX_DIMEN,
            box_max_depth: Scaled::MAX_DIMEN,
        },
    )
    .node
}

fn vertical_height(node: &Node) -> Scaled {
    NodeRef::from(node)
        .vertical_dimensions()
        .map_or(Scaled::from_raw(0), |(height, _)| height)
}

#[cfg(test)]
mod tests;
