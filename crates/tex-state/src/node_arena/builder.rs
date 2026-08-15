use super::{NodeListRef, NodeSemanticId};
use crate::ids::NodeListId;
use crate::node::Node;

/// One operation-local builder for an immutable structurally owned node list.
pub struct NodeListBuilder {
    buf: Vec<Node>,
    direct_children: Vec<NodeListRef>,
}

impl NodeListBuilder {
    pub(crate) fn new() -> Self {
        Self {
            buf: Vec::new(),
            direct_children: Vec::new(),
        }
    }

    pub fn push(&mut self, node: Node) {
        node.visit_node_lists(|child| {
            if !self
                .direct_children
                .iter()
                .any(|existing| existing.id() == child.id() && existing.shares_payload(child))
            {
                self.direct_children.push(child.clone());
            }
        });
        self.buf.push(node)
    }

    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    #[must_use]
    pub(crate) fn as_slice(&self) -> &[Node] {
        &self.buf
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.direct_children.clear();
    }

    pub(crate) fn owns_direct_child(&self, id: NodeListId) -> bool {
        id.len() == 0
            || self
                .direct_children
                .iter()
                .any(|owner| owner.resolve(id).is_some())
    }

    pub(crate) fn direct_child_semantic_id(&self, id: NodeListId) -> Option<NodeSemanticId> {
        if id.is_empty() {
            return Some(NodeSemanticId::empty());
        }
        self.direct_children
            .iter()
            .find_map(|owner| owner.resolve(id).map(|child| child.semantic_id()))
    }

    pub(crate) fn into_direct_parts(self) -> (Vec<Node>, Vec<NodeListRef>) {
        (self.buf, self.direct_children)
    }
}
