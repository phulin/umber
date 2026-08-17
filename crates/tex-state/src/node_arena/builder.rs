use super::NodeListRef;
use crate::node::Node;

/// The sole mutable builder for native node material.
///
/// Child ownership, semantic identity, and compact storage sidecars are
/// deliberately absent while a semantic episode is live. The aggregate
/// freeze boundary derives them once from `buf` before publishing an
/// immutable [`NodeListRef`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeListBuilder {
    buf: Vec<Node>,
}

impl NodeListBuilder {
    pub(crate) fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn push(&mut self, node: Node) {
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
    pub fn as_slice(&self) -> &[Node] {
        &self.buf
    }

    pub(crate) fn as_mut_vec(&mut self) -> &mut Vec<Node> {
        &mut self.buf
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        self.buf.truncate(len)
    }

    pub(crate) fn into_nodes(self) -> Vec<Node> {
        self.buf
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }

    pub(crate) fn direct_children(&self) -> Vec<NodeListRef> {
        let mut direct_children = Vec::new();
        for node in &self.buf {
            node.visit_node_lists(|child| {
                if !direct_children.iter().any(|existing: &NodeListRef| {
                    existing.id() == child.id() && existing.shares_payload(child)
                }) {
                    direct_children.push(child.clone());
                }
            });
        }
        direct_children
    }
}
