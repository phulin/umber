//! Paired semantic and TeX-physical node sequences.

use std::sync::Arc;

use crate::node::Node;

/// A node sequence whose semantic channel drives execution and whose physical
/// channel preserves TeX's linked-list topology for diagnostics.
#[derive(Clone, Debug, Default)]
pub struct NodeSequence {
    semantic: Arc<Vec<Node>>,
    physical: Arc<Vec<Node>>,
}

impl PartialEq for NodeSequence {
    fn eq(&self, other: &Self) -> bool {
        self.semantic == other.semantic
    }
}

impl NodeSequence {
    #[must_use]
    pub fn mirrored(nodes: Vec<Node>) -> Self {
        Self {
            physical: Arc::new(nodes.clone()),
            semantic: Arc::new(nodes),
        }
    }

    #[must_use]
    pub fn from_channels(semantic: Vec<Node>, physical: Vec<Node>) -> Self {
        Self {
            semantic: Arc::new(semantic),
            physical: Arc::new(physical),
        }
    }

    #[must_use]
    pub fn semantic(&self) -> &[Node] {
        &self.semantic
    }

    #[must_use]
    pub fn physical(&self) -> &[Node] {
        &self.physical
    }

    pub fn take(self) -> (Vec<Node>, Vec<Node>) {
        (
            Arc::try_unwrap(self.semantic).unwrap_or_else(|nodes| (*nodes).clone()),
            Arc::try_unwrap(self.physical).unwrap_or_else(|nodes| (*nodes).clone()),
        )
    }

    pub fn push_mirrored(&mut self, node: Node) {
        Arc::make_mut(&mut self.semantic).push(node.clone());
        Arc::make_mut(&mut self.physical).push(node);
    }

    pub fn extend_mirrored(&mut self, nodes: impl IntoIterator<Item = Node>) {
        for node in nodes {
            self.push_mirrored(node);
        }
    }

    pub fn replace_channels(&mut self, semantic: Vec<Node>, physical: Vec<Node>) {
        self.semantic = Arc::new(semantic);
        self.physical = Arc::new(physical);
    }

    pub fn semantic_arc(&self) -> Arc<Vec<Node>> {
        self.semantic.clone()
    }

    pub fn physical_arc(&self) -> Arc<Vec<Node>> {
        self.physical.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equality_and_clone_identity_are_semantic_only() {
        let left = NodeSequence::from_channels(vec![Node::Penalty(1)], vec![Node::Penalty(2)]);
        let right = NodeSequence::from_channels(vec![Node::Penalty(1)], vec![Node::Penalty(3)]);
        assert_eq!(left, right);
        let clone = left.clone();
        assert!(Arc::ptr_eq(&left.semantic_arc(), &clone.semantic_arc()));
        assert!(Arc::ptr_eq(&left.physical_arc(), &clone.physical_arc()));
    }
}
