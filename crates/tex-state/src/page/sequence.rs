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
    #[cfg(test)]
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

    pub(super) fn last(&self) -> Option<&Node> {
        self.nodes.last()
    }

    pub(super) const fn len(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn push(&mut self, node: Node) {
        self.nodes.push(node);
    }

    pub(super) fn clear(&mut self) {
        self.nodes.clear();
    }

    pub(super) fn take_prefix(&mut self, split_index: usize) -> (Vec<Node>, Vec<Node>) {
        let split_index = split_index.min(self.nodes.len());
        let after = self.nodes.split_off(split_index);
        let before = std::mem::take(&mut self.nodes);
        (before, after)
    }
}
