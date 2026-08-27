use crate::node::Node;
use crate::node_sequence::{SemanticSequenceIdentity, semantic_node_identity};

/// Current-page suffix owned directly by the page lifetime.
///
/// The page builder restores its canonical length before truncating rejected
/// page-arena rows. It does not retain persistent COW roots per operation.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PageNodeSequence {
    nodes: Vec<Node>,
    identity_enabled: bool,
    identity: SemanticSequenceIdentity,
}

impl PageNodeSequence {
    pub(super) fn from_nodes(nodes: Vec<Node>) -> Self {
        Self {
            nodes,
            identity_enabled: false,
            identity: SemanticSequenceIdentity::empty(),
        }
    }

    pub(super) fn enable_semantic_identity(&mut self) {
        if !self.identity_enabled {
            self.identity = SemanticSequenceIdentity::from_nodes(&self.nodes);
            self.identity_enabled = true;
        }
    }

    pub(super) fn into_nodes(self) -> Vec<Node> {
        self.nodes
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.nodes
            .capacity()
            .saturating_mul(std::mem::size_of::<Node>())
    }

    pub(super) fn iter(&self) -> impl DoubleEndedIterator<Item = &Node> {
        self.nodes.iter()
    }

    #[cfg(test)]
    pub(super) fn as_slice(&self) -> &[Node] {
        &self.nodes
    }

    pub(super) fn get(&self, index: usize) -> Option<&Node> {
        self.nodes.get(index)
    }

    pub(super) const fn len(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn push(&mut self, node: Node) {
        if self.identity_enabled {
            self.identity.push_back(semantic_node_identity(&node));
        }
        self.nodes.push(node);
    }

    pub(super) fn pop(&mut self) -> Option<Node> {
        let node = self.nodes.pop()?;
        if self.identity_enabled {
            self.identity.pop_back(semantic_node_identity(&node));
        }
        Some(node)
    }

    pub(super) fn clear(&mut self) {
        self.nodes.clear();
        self.identity = SemanticSequenceIdentity::empty();
    }

    pub(super) fn truncate(&mut self, len: usize) {
        self.nodes.truncate(len);
        if self.identity_enabled {
            self.identity = SemanticSequenceIdentity::from_nodes(&self.nodes);
        }
    }

    pub(super) fn take_prefix(&mut self, split_index: usize) -> (Vec<Node>, Vec<Node>) {
        let split_index = split_index.min(self.nodes.len());
        let after = self.nodes.split_off(split_index);
        let before = std::mem::take(&mut self.nodes);
        self.identity = SemanticSequenceIdentity::empty();
        (before, after)
    }

    pub(super) const fn semantic_identity(&self) -> SemanticSequenceIdentity {
        self.identity
    }
}
