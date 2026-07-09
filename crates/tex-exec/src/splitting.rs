//! Shared vertical-list splitting helpers for insertions and `\vsplit`.

use tex_state::Universe;
use tex_state::glue::GlueSpec;
use tex_state::ids::{GlueId, NodeListId};
use tex_state::node::{BoxNode, GlueKind, Node};
use tex_state::scaled::Scaled;
use tex_typeset::{INF_BAD, PackSpec, VpackParams, vpack};

use crate::ExecError;

pub(crate) fn prune_page_top(
    stores: &mut Universe,
    nodes: Vec<Node>,
    split_top_skip: GlueId,
) -> Vec<Node> {
    let mut out = Vec::new();
    let mut inserted_top_skip = false;
    for node in nodes {
        match &node {
            Node::HList(_) | Node::VList(_) | Node::Rule { .. } if !inserted_top_skip => {
                let top_skip = stores.glue(split_top_skip);
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
                let spec = stores.intern_glue(adjusted);
                out.push(Node::Glue {
                    spec,
                    kind: GlueKind::SplitTopSkip,
                });
                out.push(node);
                inserted_top_skip = true;
            }
            Node::Glue { .. } | Node::Kern { .. } | Node::Penalty(_) if !inserted_top_skip => {}
            _ => out.push(node),
        }
    }
    out
}

pub(crate) fn natural_vlist_size(
    stores: &Universe,
    content: NodeListId,
) -> Result<Scaled, ExecError> {
    let packed = vpack_natural(stores, content);
    packed
        .height
        .checked_add(packed.depth)
        .ok_or(ExecError::ArithmeticOverflow)
}

pub(crate) fn vpack_natural(stores: &Universe, content: NodeListId) -> BoxNode {
    vpack(
        stores,
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
    match node {
        Node::HList(box_node) | Node::VList(box_node) => box_node.height,
        Node::Rule { height, .. } => height.unwrap_or_else(|| Scaled::from_raw(0)),
        _ => Scaled::from_raw(0),
    }
}
