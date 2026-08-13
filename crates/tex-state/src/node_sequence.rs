//! Paired semantic and TeX-physical node sequences.

use std::sync::Arc;

use crate::node::Node;

/// A node sequence whose semantic channel drives execution and whose physical
/// channel preserves TeX's linked-list topology for diagnostics.
#[derive(Clone, Debug)]
pub struct NodeSequence {
    semantic: Arc<Vec<Node>>,
    physical: Arc<Vec<Node>>,
    physical_boundaries: Arc<Vec<usize>>,
}

impl Default for NodeSequence {
    fn default() -> Self {
        Self::mirrored(Vec::new())
    }
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
            physical_boundaries: Arc::new((0..=nodes.len()).collect()),
            semantic: Arc::new(nodes),
        }
    }

    #[must_use]
    pub fn from_channels(semantic: Vec<Node>, physical: Vec<Node>) -> Self {
        assert_eq!(semantic.len(), physical.len());
        let physical_boundaries = Arc::new((0..=semantic.len()).collect());
        Self {
            semantic: Arc::new(semantic),
            physical: Arc::new(physical),
            physical_boundaries,
        }
    }

    #[must_use]
    pub fn from_projection(
        semantic: Vec<Node>,
        physical: Vec<Node>,
        physical_boundaries: Vec<usize>,
    ) -> Self {
        assert_eq!(physical_boundaries.len(), semantic.len() + 1);
        assert_eq!(physical_boundaries.first(), Some(&0));
        assert_eq!(physical_boundaries.last(), Some(&physical.len()));
        assert!(
            physical_boundaries
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
        );
        Self {
            semantic: Arc::new(semantic),
            physical: Arc::new(physical),
            physical_boundaries: Arc::new(physical_boundaries),
        }
    }

    /// Builds the diagnostic boundary projection after semantic character
    /// runs have been compacted while the physical channel remains in TeX's
    /// one-node-per-character topology.
    #[must_use]
    pub fn from_compacted_semantic(semantic: Vec<Node>, physical: Vec<Node>) -> Self {
        let mut boundary = 0usize;
        let mut physical_boundaries = Vec::with_capacity(semantic.len() + 1);
        physical_boundaries.push(0);
        for node in &semantic {
            boundary = boundary.saturating_add(match node {
                Node::Lig { orig, .. } => orig.len().max(1),
                _ => 1,
            });
            physical_boundaries.push(boundary.min(physical.len()));
        }
        if let Some(last) = physical_boundaries.last_mut() {
            *last = physical.len();
        }
        Self::from_projection(semantic, physical, physical_boundaries)
    }

    #[must_use]
    pub fn semantic(&self) -> &[Node] {
        &self.semantic
    }

    #[must_use]
    pub fn physical(&self) -> &[Node] {
        &self.physical
    }

    #[must_use]
    pub fn physical_boundary(&self, semantic_boundary: usize) -> Option<usize> {
        self.physical_boundaries.get(semantic_boundary).copied()
    }

    pub fn take(self) -> (Vec<Node>, Vec<Node>) {
        (
            Arc::try_unwrap(self.semantic).unwrap_or_else(|nodes| (*nodes).clone()),
            Arc::try_unwrap(self.physical).unwrap_or_else(|nodes| (*nodes).clone()),
        )
    }

    pub fn into_parts(self) -> (Vec<Node>, Vec<Node>, Vec<usize>) {
        let Self {
            semantic,
            physical,
            physical_boundaries,
        } = self;
        let semantic = Arc::try_unwrap(semantic).unwrap_or_else(|nodes| (*nodes).clone());
        let physical = Arc::try_unwrap(physical).unwrap_or_else(|nodes| (*nodes).clone());
        let boundaries =
            Arc::try_unwrap(physical_boundaries).unwrap_or_else(|boundaries| (*boundaries).clone());
        (semantic, physical, boundaries)
    }

    pub fn push_mirrored(&mut self, node: Node) {
        Arc::make_mut(&mut self.semantic).push(node.clone());
        Arc::make_mut(&mut self.physical).push(node);
        Arc::make_mut(&mut self.physical_boundaries).push(self.physical.len());
    }

    pub fn extend_mirrored(&mut self, nodes: impl IntoIterator<Item = Node>) {
        for node in nodes {
            self.push_mirrored(node);
        }
    }

    pub fn replace_channels(&mut self, semantic: Vec<Node>, physical: Vec<Node>) {
        self.semantic = Arc::new(semantic);
        self.physical = Arc::new(physical);
        assert_eq!(self.semantic.len(), self.physical.len());
        self.physical_boundaries = Arc::new((0..=self.semantic.len()).collect());
    }

    /// Mutates semantic nodes and atomically resets the physical channel to
    /// the resulting topology. Callers that perform a topology-aware rewrite
    /// replace both channels explicitly instead.
    pub fn mutate_semantic<R>(&mut self, mutate: impl FnOnce(&mut Vec<Node>) -> R) -> R {
        let result = mutate(Arc::make_mut(&mut self.semantic));
        self.physical = Arc::new((*self.semantic).clone());
        self.physical_boundaries = Arc::new((0..=self.semantic.len()).collect());
        result
    }

    pub fn truncate(&mut self, semantic_len: usize, physical_len: usize) {
        Arc::make_mut(&mut self.semantic).truncate(semantic_len);
        Arc::make_mut(&mut self.physical).truncate(physical_len);
        Arc::make_mut(&mut self.physical_boundaries).truncate(semantic_len + 1);
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

    #[test]
    fn physical_boundaries_map_collapsed_semantic_nodes() {
        let sequence = NodeSequence::from_projection(
            vec![Node::Penalty(1), Node::Penalty(2)],
            vec![Node::Penalty(10), Node::Penalty(11), Node::Penalty(12)],
            vec![0, 2, 3],
        );
        assert_eq!(sequence.physical_boundary(0), Some(0));
        assert_eq!(sequence.physical_boundary(1), Some(2));
        assert_eq!(sequence.physical_boundary(2), Some(3));
    }

    #[test]
    fn compacted_semantic_constructor_counts_ligature_origins() {
        let sequence = NodeSequence::from_compacted_semantic(
            vec![
                Node::Lig {
                    font: crate::font::NULL_FONT,
                    ch: 'x',
                    orig: vec!['a', 'b', 'c'],
                    left_hit: false,
                    right_hit: false,
                    origins: vec![crate::provenance::OriginRef::unknown(); 3],
                },
                Node::Penalty(0),
            ],
            vec![
                Node::Penalty(1),
                Node::Penalty(2),
                Node::Penalty(3),
                Node::Penalty(4),
            ],
        );
        assert_eq!(sequence.physical_boundary(0), Some(0));
        assert_eq!(sequence.physical_boundary(1), Some(3));
        assert_eq!(sequence.physical_boundary(2), Some(4));
    }
}
