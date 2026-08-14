//! Paired semantic and TeX-physical node sequences.

use std::sync::Arc;

use crate::node::Node;

/// Allocation identity for one direct TeX82 high-memory cell.
///
/// This is transient allocator-projection data. It does not participate in
/// node semantics or any portable format schema.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DirectHighCellLineage {
    /// A direct row in one semantic/physical paragraph projection.
    Sequence { row: u32, unit: u32 },
    /// A direct row copied from one exact frozen discretionary branch.
    Frozen {
        list: crate::ids::NodeListId,
        row: u32,
        unit: u32,
        role: FrozenListRole,
    },
}

/// The discretionary branch that owns a frozen direct high-memory cell.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FrozenListRole {
    Pre,
    Post,
    Replace,
}

/// Counts exact direct-cell allocation identities shared by two projections.
#[must_use]
pub fn direct_high_cell_overlap(
    current: &[DirectHighCellLineage],
    predecessor: &[DirectHighCellLineage],
) -> u32 {
    let current = current
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let predecessor = predecessor
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    u32::try_from(current.intersection(&predecessor).count())
        .expect("direct high-cell overlap exceeds u32")
}

/// A node sequence whose semantic channel drives execution and whose physical
/// channel preserves TeX's linked-list topology for diagnostics.
#[derive(Clone, Debug)]
pub struct NodeSequence {
    semantic: Arc<Vec<Node>>,
    physical: Arc<Vec<Node>>,
    physical_boundaries: Arc<Vec<usize>>,
    semantic_high_cell_lineages: Arc<Vec<Vec<DirectHighCellLineage>>>,
    physical_high_cell_lineages: Arc<Vec<Vec<DirectHighCellLineage>>>,
    next_sequence_lineage_row: u32,
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
        let len = nodes.len();
        Self::from_projection(nodes.clone(), nodes, (0..=len).collect())
    }

    #[must_use]
    pub fn from_channels(semantic: Vec<Node>, physical: Vec<Node>) -> Self {
        assert_eq!(semantic.len(), physical.len());
        let len = semantic.len();
        Self::from_projection(semantic, physical, (0..=len).collect())
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
        let (semantic_high_cell_lineages, physical_high_cell_lineages, next_sequence_lineage_row) =
            projected_high_cell_lineages(&semantic, &physical, &physical_boundaries);
        Self {
            semantic: Arc::new(semantic),
            physical: Arc::new(physical),
            physical_boundaries: Arc::new(physical_boundaries),
            semantic_high_cell_lineages: Arc::new(semantic_high_cell_lineages),
            physical_high_cell_lineages: Arc::new(physical_high_cell_lineages),
            next_sequence_lineage_row,
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

    #[must_use]
    pub fn semantic_high_cell_lineages(&self) -> &[Vec<DirectHighCellLineage>] {
        &self.semantic_high_cell_lineages
    }

    #[must_use]
    pub fn physical_high_cell_lineages(&self) -> &[Vec<DirectHighCellLineage>] {
        &self.physical_high_cell_lineages
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
            ..
        } = self;
        let semantic = Arc::try_unwrap(semantic).unwrap_or_else(|nodes| (*nodes).clone());
        let physical = Arc::try_unwrap(physical).unwrap_or_else(|nodes| (*nodes).clone());
        let boundaries =
            Arc::try_unwrap(physical_boundaries).unwrap_or_else(|boundaries| (*boundaries).clone());
        (semantic, physical, boundaries)
    }

    pub fn push_mirrored(&mut self, node: Node) {
        let row = self.next_sequence_lineage_row;
        self.next_sequence_lineage_row = self
            .next_sequence_lineage_row
            .checked_add(1)
            .expect("node sequence lineage rows exceed u32");
        let lineages = direct_high_cell_lineages(&node, row);
        Arc::make_mut(&mut self.semantic).push(node.clone());
        Arc::make_mut(&mut self.physical).push(node);
        Arc::make_mut(&mut self.physical_boundaries).push(self.physical.len());
        Arc::make_mut(&mut self.semantic_high_cell_lineages).push(lineages.clone());
        Arc::make_mut(&mut self.physical_high_cell_lineages).push(lineages);
    }

    pub fn extend_mirrored(&mut self, nodes: impl IntoIterator<Item = Node>) {
        for node in nodes {
            self.push_mirrored(node);
        }
    }

    pub fn replace_channels(&mut self, semantic: Vec<Node>, physical: Vec<Node>) {
        assert_eq!(semantic.len(), physical.len());
        *self = Self::from_channels(semantic, physical);
    }

    /// Rewrites direct child-list handles while preserving the diagnostic
    /// physical channel and its semantic-boundary projection.
    pub fn visit_node_lists_mut(&mut self, mut visit: impl FnMut(&mut crate::ids::NodeListId)) {
        for node in Arc::make_mut(&mut self.semantic) {
            node.visit_node_lists_mut(&mut visit);
        }
        for node in Arc::make_mut(&mut self.physical) {
            node.visit_node_lists_mut(&mut visit);
        }
    }

    /// Mutates semantic nodes and atomically resets the physical channel to
    /// the resulting topology. Callers that perform a topology-aware rewrite
    /// replace both channels explicitly instead.
    pub fn mutate_semantic<R>(&mut self, mutate: impl FnOnce(&mut Vec<Node>) -> R) -> R {
        let result = mutate(Arc::make_mut(&mut self.semantic));
        self.physical = Arc::new((*self.semantic).clone());
        self.physical_boundaries = Arc::new((0..=self.semantic.len()).collect());
        let (semantic, physical, next_sequence_lineage_row) =
            projected_high_cell_lineages(&self.semantic, &self.physical, &self.physical_boundaries);
        self.semantic_high_cell_lineages = Arc::new(semantic);
        self.physical_high_cell_lineages = Arc::new(physical);
        self.next_sequence_lineage_row = next_sequence_lineage_row;
        result
    }

    pub fn truncate(&mut self, semantic_len: usize, physical_len: usize) {
        Arc::make_mut(&mut self.semantic).truncate(semantic_len);
        Arc::make_mut(&mut self.physical).truncate(physical_len);
        Arc::make_mut(&mut self.physical_boundaries).truncate(semantic_len + 1);
        Arc::make_mut(&mut self.semantic_high_cell_lineages).truncate(semantic_len);
        Arc::make_mut(&mut self.physical_high_cell_lineages).truncate(physical_len);
    }

    pub fn semantic_arc(&self) -> Arc<Vec<Node>> {
        self.semantic.clone()
    }

    pub fn physical_arc(&self) -> Arc<Vec<Node>> {
        self.physical.clone()
    }
}

fn projected_high_cell_lineages(
    semantic: &[Node],
    physical: &[Node],
    boundaries: &[usize],
) -> (
    Vec<Vec<DirectHighCellLineage>>,
    Vec<Vec<DirectHighCellLineage>>,
    u32,
) {
    let mut semantic_lineages = Vec::with_capacity(semantic.len());
    let mut physical_lineages = vec![Vec::new(); physical.len()];
    let mut next_unpaired_row =
        u32::try_from(semantic.len()).expect("node sequence exceeds u32 rows");
    for (semantic_row, node) in semantic.iter().enumerate() {
        let semantic_row = u32::try_from(semantic_row).expect("node sequence exceeds u32 rows");
        let start = boundaries[semantic_row as usize];
        let end = boundaries[semantic_row as usize + 1];
        if end == start + 1 {
            semantic_lineages.push(direct_high_cell_lineages(node, semantic_row));
            physical_lineages[start] = direct_high_cell_lineages(&physical[start], semantic_row);
        } else {
            semantic_lineages.push(direct_high_cell_lineages(node, semantic_row));
            for physical_row in start..end {
                physical_lineages[physical_row] =
                    direct_high_cell_lineages(&physical[physical_row], next_unpaired_row);
                next_unpaired_row = next_unpaired_row
                    .checked_add(1)
                    .expect("node sequence lineage rows exceed u32");
            }
        }
    }
    (semantic_lineages, physical_lineages, next_unpaired_row)
}

fn direct_high_cell_lineages(node: &Node, row: u32) -> Vec<DirectHighCellLineage> {
    let count = match node {
        Node::Char { .. } => 1,
        Node::Lig { orig, .. } => orig.len(),
        _ => 0,
    };
    (0..count)
        .map(|unit| DirectHighCellLineage::Sequence {
            row,
            unit: u32::try_from(unit).expect("ligature source exceeds u32 cells"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::NULL_FONT;
    use crate::provenance::OriginRef;

    fn char_node(ch: char) -> Node {
        Node::Char {
            font: NULL_FONT,
            ch,
            origin: OriginRef::unknown(),
        }
    }

    fn lig_node(ch: char, orig: &[char]) -> Node {
        Node::Lig {
            font: NULL_FONT,
            ch,
            orig: orig.to_vec(),
            left_hit: false,
            right_hit: false,
            origins: vec![OriginRef::unknown(); orig.len()],
        }
    }

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

    #[test]
    fn direct_cell_lineage_uses_projection_identity_not_content() {
        let paired = NodeSequence::from_channels(
            vec![char_node('A'), lig_node('x', &['B', 'B'])],
            vec![char_node('Z'), lig_node('y', &['C', 'D'])],
        );
        let semantic = paired
            .semantic_high_cell_lineages()
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let physical = paired
            .physical_high_cell_lineages()
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(direct_high_cell_overlap(&semantic, &physical), 3);

        let unequal_units =
            NodeSequence::from_channels(vec![char_node('A')], vec![Node::Penalty(0)]);
        assert_eq!(
            direct_high_cell_overlap(
                &unequal_units.semantic_high_cell_lineages()[0],
                &unequal_units.physical_high_cell_lineages()[0],
            ),
            0
        );

        let unpaired = NodeSequence::from_projection(
            vec![lig_node('x', &['A', 'A', 'A'])],
            vec![char_node('A'), char_node('A'), char_node('A')],
            vec![0, 3],
        );
        let semantic = unpaired
            .semantic_high_cell_lineages()
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let physical = unpaired
            .physical_high_cell_lineages()
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(direct_high_cell_overlap(&semantic, &physical), 0);
    }
}
