use crate::{LeaderPayload, PageNode};

/// One event in the canonical, pre-order artifact node grammar.
///
/// Lists are explicit events because their lengths occur on the wire before
/// their children.  Keeping them separate from nodes lets both the binary
/// emitter and semantic consumers use one nonrecursive traversal authority.
#[derive(Clone, Copy)]
pub(crate) enum ArtifactNodeEvent<'a> {
    Node { node: &'a PageNode, depth: usize },
    List { nodes: &'a [PageNode] },
}

/// Explicit-stack cursor over every node and nested list in artifact order.
pub(crate) struct ArtifactNodeCursor<'a> {
    pending: Vec<ArtifactNodeEvent<'a>>,
    validation_order: bool,
}

impl<'a> ArtifactNodeCursor<'a> {
    pub(crate) fn new(root: &'a PageNode) -> Self {
        Self {
            pending: vec![ArtifactNodeEvent::Node {
                node: root,
                depth: 1,
            }],
            validation_order: false,
        }
    }

    /// Uses the historical owned-validation sibling order. This preserves
    /// semantic error precedence while sharing the node/list grammar.
    pub(crate) fn for_validation(root: &'a PageNode) -> Self {
        Self {
            validation_order: true,
            ..Self::new(root)
        }
    }

    fn schedule_list(&mut self, nodes: &'a [PageNode], depth: usize) {
        self.pending.extend(
            nodes
                .iter()
                .rev()
                .map(|node| ArtifactNodeEvent::Node { node, depth }),
        );
        self.pending.push(ArtifactNodeEvent::List { nodes });
    }

    fn schedule_children(&mut self, node: &'a PageNode, depth: usize) {
        let child_depth = depth + 1;
        match node {
            PageNode::HList(box_node) | PageNode::VList(box_node) => {
                self.schedule_list(&box_node.children, child_depth);
            }
            PageNode::Glue {
                leader: Some(LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node)),
                ..
            } => self.schedule_list(&box_node.children, child_depth),
            PageNode::Disc {
                pre, post, replace, ..
            } => {
                if self.validation_order {
                    // Preserve the established semantic-error precedence:
                    // replace, post, then pre.
                    self.schedule_list(pre, child_depth);
                    self.schedule_list(post, child_depth);
                    self.schedule_list(replace, child_depth);
                } else {
                    // Stack order is reversed so the cursor exposes pre,
                    // post, replace exactly as the versioned byte grammar.
                    self.schedule_list(replace, child_depth);
                    self.schedule_list(post, child_depth);
                    self.schedule_list(pre, child_depth);
                }
            }
            PageNode::Insert { content, .. } | PageNode::Adjust(content) => {
                self.schedule_list(content, child_depth);
            }
            PageNode::Char { .. }
            | PageNode::Lig { .. }
            | PageNode::Kern { .. }
            | PageNode::MarginKern { .. }
            | PageNode::Glue { .. }
            | PageNode::Penalty(_)
            | PageNode::Rule { .. }
            | PageNode::Mark { .. }
            | PageNode::WhatsitAnchor { .. }
            | PageNode::MathOn(_)
            | PageNode::MathOff(_) => {}
        }
    }
}

impl<'a> Iterator for ArtifactNodeCursor<'a> {
    type Item = ArtifactNodeEvent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let event = self.pending.pop()?;
        if let ArtifactNodeEvent::Node { node, depth } = event {
            self.schedule_children(node, depth);
        }
        Some(event)
    }
}
