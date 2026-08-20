//! Paired semantic and TeX-physical node sequences.

use crate::node::Node;
use crate::node_arena::{NodeArenaError, PageListId, PageNodeArena};

/// Allocation identity for one direct TeX82 high-memory cell.
///
/// This is transient allocator-projection data. It does not participate in
/// node semantics or any portable format schema.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DirectHighCellLineage {
    /// A direct row in one semantic/physical paragraph projection.
    Sequence { row: u32, unit: u32 },
    /// A direct row copied from one exact frozen discretionary branch.
    Frozen {
        list: PageListId,
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
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let predecessor = predecessor
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    u32::try_from(current.intersection(&predecessor).count())
        .expect("direct high-cell overlap exceeds u32")
}

/// A node sequence whose semantic channel drives execution and whose physical
/// channel preserves TeX's linked-list topology for diagnostics.
#[derive(Clone, Debug)]
pub struct NodeSequence {
    semantic: Vec<Node>,
    physical: Vec<Node>,
    frozen_semantic: Option<PageListId>,
    frozen_physical: Option<PageListId>,
    physical_boundaries: Vec<usize>,
    semantic_high_cell_lineages: Vec<Vec<DirectHighCellLineage>>,
    physical_high_cell_lineages: Vec<Vec<DirectHighCellLineage>>,
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
            semantic,
            physical,
            frozen_semantic: None,
            frozen_physical: None,
            physical_boundaries,
            semantic_high_cell_lineages,
            physical_high_cell_lineages,
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
        (self.semantic, self.physical)
    }

    pub fn into_parts(self) -> (Vec<Node>, Vec<Node>, Vec<usize>) {
        let Self {
            semantic,
            physical,
            physical_boundaries,
            ..
        } = self;
        (semantic, physical, physical_boundaries)
    }

    pub fn push_mirrored(&mut self, node: Node) {
        self.invalidate_frozen_sidecars();
        let row = self.next_sequence_lineage_row;
        self.next_sequence_lineage_row = self
            .next_sequence_lineage_row
            .checked_add(1)
            .expect("node sequence lineage rows exceed u32");
        let lineages = direct_high_cell_lineages(&node, row);
        self.semantic.push(node.clone());
        self.physical.push(node);
        self.physical_boundaries.push(self.physical.len());
        self.semantic_high_cell_lineages.push(lineages.clone());
        self.physical_high_cell_lineages.push(lineages);
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

    /// Mutates semantic nodes and atomically resets the physical channel to
    /// the resulting topology. Callers that perform a topology-aware rewrite
    /// replace both channels explicitly instead.
    pub fn mutate_semantic<R>(&mut self, mutate: impl FnOnce(&mut Vec<Node>) -> R) -> R {
        self.invalidate_frozen_sidecars();
        let result = mutate(&mut self.semantic);
        self.physical = self.semantic.clone();
        self.physical_boundaries = (0..=self.semantic.len()).collect();
        let (semantic, physical, next_sequence_lineage_row) =
            projected_high_cell_lineages(&self.semantic, &self.physical, &self.physical_boundaries);
        self.semantic_high_cell_lineages = semantic;
        self.physical_high_cell_lineages = physical;
        self.next_sequence_lineage_row = next_sequence_lineage_row;
        result
    }

    pub fn truncate(&mut self, semantic_len: usize, physical_len: usize) {
        self.invalidate_frozen_sidecars();
        self.semantic.truncate(semantic_len);
        self.physical.truncate(physical_len);
        self.physical_boundaries.truncate(semantic_len + 1);
        self.semantic_high_cell_lineages.truncate(semantic_len);
        self.physical_high_cell_lineages.truncate(physical_len);
    }

    /// Materializes immutable node/reachability/provenance sidecars at an
    /// externally visible episode boundary while retaining this builder as
    /// the sole mutable continuation.
    pub fn freeze_sidecars(&mut self, arena: &mut PageNodeArena) -> Result<(), NodeArenaError> {
        if self.frozen_semantic.is_none() {
            self.frozen_semantic = Some(arena.publish(self.semantic.clone())?);
        }
        if self.frozen_physical.is_none() {
            self.frozen_physical = Some(arena.publish(self.physical.clone())?);
        }
        Ok(())
    }

    #[must_use]
    pub fn frozen_sidecars(&self) -> Option<(PageListId, PageListId)> {
        Some((self.frozen_semantic?, self.frozen_physical?))
    }

    fn invalidate_frozen_sidecars(&mut self) {
        self.frozen_semantic = None;
        self.frozen_physical = None;
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
    use crate::token::OriginId;

    fn char_node(ch: char) -> Node {
        Node::Char {
            font: NULL_FONT,
            ch,
            origin: OriginId::UNKNOWN,
        }
    }

    fn lig_node(ch: char, orig: &[char]) -> Node {
        Node::Lig {
            font: NULL_FONT,
            ch,
            orig: orig.to_vec(),
            left_hit: false,
            right_hit: false,
            origins: vec![OriginId::UNKNOWN; orig.len()],
        }
    }

    #[test]
    fn equality_and_clone_are_semantic_only() {
        let left = NodeSequence::from_channels(vec![Node::Penalty(1)], vec![Node::Penalty(2)]);
        let right = NodeSequence::from_channels(vec![Node::Penalty(1)], vec![Node::Penalty(3)]);
        assert_eq!(left, right);
        let clone = left.clone();
        assert_eq!(left.semantic(), clone.semantic());
        assert_eq!(left.physical(), clone.physical());
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
                    origins: vec![crate::token::OriginId::UNKNOWN; 3],
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
            .cloned()
            .collect::<Vec<_>>();
        let physical = paired
            .physical_high_cell_lineages()
            .iter()
            .flatten()
            .cloned()
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
            .cloned()
            .collect::<Vec<_>>();
        let physical = unpaired
            .physical_high_cell_lineages()
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(direct_high_cell_overlap(&semantic, &physical), 0);
    }
}
