use crate::node::Node;

/// Current-page suffix owned directly by the page lifetime.
///
/// The page builder restores its canonical length before truncating rejected
/// page-arena rows. It does not retain persistent COW roots per operation.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PageNodeSequence {
    nodes: Vec<Node>,
}

impl PageNodeSequence {
    pub(super) fn from_nodes(nodes: Vec<Node>) -> Self {
        Self { nodes }
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
        self.nodes.push(node);
    }

    pub(super) fn pop(&mut self) -> Option<Node> {
        self.nodes.pop()
    }

    pub(super) fn clear(&mut self) {
        self.nodes.clear();
    }

    pub(super) fn truncate(&mut self, len: usize) {
        self.nodes.truncate(len);
    }

    pub(super) fn take_prefix(&mut self, split_index: usize) -> (Vec<Node>, Vec<Node>) {
        let split_index = split_index.min(self.nodes.len());
        let after = self.nodes.split_off(split_index);
        let before = std::mem::take(&mut self.nodes);
        (before, after)
    }
}
