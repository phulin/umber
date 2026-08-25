//! Paired semantic and TeX-physical node sequences.

use crate::node::Node;
use crate::node_arena::PageListId;
use smallvec::SmallVec;

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

/// Inline storage for the direct cells contributed by one node.
///
/// Ordinary character nodes contribute exactly one cell. Keeping that value
/// inline avoids a heap allocation per character while ligatures can still
/// spill for their uncommon multi-cell source spelling.
pub type DirectHighCellLineages = SmallVec<[DirectHighCellLineage; 1]>;

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
    projection: PhysicalProjection,
    frozen_semantic: Option<PageListId>,
    frozen_physical: Option<PageListId>,
    semantic_high_cell_lineages: Vec<DirectHighCellLineages>,
    next_sequence_lineage_row: u32,
}

/// Explicit physical-channel state. Mirrored sequences store no duplicate
/// nodes or lineage rows; a distinct projection is created only by a caller
/// that supplies one, never by comparing channel content.
#[derive(Clone, Debug)]
enum PhysicalProjection {
    Mirrored,
    Distinct {
        nodes: Vec<Node>,
        boundaries: Vec<usize>,
        high_cell_lineages: Vec<DirectHighCellLineages>,
    },
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
        let (semantic_high_cell_lineages, next_sequence_lineage_row) =
            mirrored_high_cell_lineages(&nodes);
        Self {
            semantic: nodes,
            projection: PhysicalProjection::Mirrored,
            frozen_semantic: None,
            frozen_physical: None,
            semantic_high_cell_lineages,
            next_sequence_lineage_row,
        }
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
            projection: PhysicalProjection::Distinct {
                nodes: physical,
                boundaries: physical_boundaries,
                high_cell_lineages: physical_high_cell_lineages,
            },
            frozen_semantic: None,
            frozen_physical: None,
            semantic_high_cell_lineages,
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
        match &self.projection {
            PhysicalProjection::Mirrored => &self.semantic,
            PhysicalProjection::Distinct { nodes, .. } => nodes,
        }
    }

    #[must_use]
    pub fn physical_boundary(&self, semantic_boundary: usize) -> Option<usize> {
        match &self.projection {
            PhysicalProjection::Mirrored => {
                (semantic_boundary <= self.semantic.len()).then_some(semantic_boundary)
            }
            PhysicalProjection::Distinct { boundaries, .. } => {
                boundaries.get(semantic_boundary).copied()
            }
        }
    }

    #[must_use]
    pub fn semantic_high_cell_lineages(&self) -> &[DirectHighCellLineages] {
        &self.semantic_high_cell_lineages
    }

    #[must_use]
    pub fn physical_high_cell_lineages(&self) -> &[DirectHighCellLineages] {
        match &self.projection {
            PhysicalProjection::Mirrored => &self.semantic_high_cell_lineages,
            PhysicalProjection::Distinct {
                high_cell_lineages, ..
            } => high_cell_lineages,
        }
    }

    pub fn take(self) -> (Vec<Node>, Vec<Node>) {
        match self.projection {
            PhysicalProjection::Mirrored => {
                let physical = self.semantic.clone();
                (self.semantic, physical)
            }
            PhysicalProjection::Distinct { nodes, .. } => (self.semantic, nodes),
        }
    }

    pub fn into_parts(self) -> (Vec<Node>, Vec<Node>, Vec<usize>) {
        match self.projection {
            PhysicalProjection::Mirrored => {
                let physical = self.semantic.clone();
                let boundaries = (0..=self.semantic.len()).collect();
                (self.semantic, physical, boundaries)
            }
            PhysicalProjection::Distinct {
                nodes, boundaries, ..
            } => (self.semantic, nodes, boundaries),
        }
    }

    pub fn push_mirrored(&mut self, node: Node) {
        self.invalidate_frozen_sidecars();
        let row = self.next_sequence_lineage_row;
        self.next_sequence_lineage_row = self
            .next_sequence_lineage_row
            .checked_add(1)
            .expect("node sequence lineage rows exceed u32");
        let lineages = direct_high_cell_lineages(&node, row);
        match &mut self.projection {
            PhysicalProjection::Mirrored => {
                self.semantic.push(node);
                self.semantic_high_cell_lineages.push(lineages);
            }
            PhysicalProjection::Distinct {
                nodes,
                boundaries,
                high_cell_lineages,
            } => {
                self.semantic.push(node.clone());
                nodes.push(node);
                boundaries.push(nodes.len());
                self.semantic_high_cell_lineages.push(lineages.clone());
                high_cell_lineages.push(lineages);
            }
        }
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
        self.projection = PhysicalProjection::Mirrored;
        let (semantic, next_sequence_lineage_row) = mirrored_high_cell_lineages(&self.semantic);
        self.semantic_high_cell_lineages = semantic;
        self.next_sequence_lineage_row = next_sequence_lineage_row;
        result
    }

    pub fn truncate(&mut self, semantic_len: usize, physical_len: usize) {
        self.invalidate_frozen_sidecars();
        if matches!(self.projection, PhysicalProjection::Mirrored) {
            assert_eq!(
                semantic_len, physical_len,
                "mirrored channels must roll back to one common cursor"
            );
        }
        self.semantic.truncate(semantic_len);
        self.semantic_high_cell_lineages.truncate(semantic_len);
        if let PhysicalProjection::Distinct {
            nodes,
            boundaries,
            high_cell_lineages,
        } = &mut self.projection
        {
            nodes.truncate(physical_len);
            boundaries.truncate(semantic_len + 1);
            high_cell_lineages.truncate(physical_len);
        }
    }

    /// Materializes immutable node/reachability/provenance sidecars at an
    /// externally visible episode boundary while retaining this builder as
    /// the sole mutable continuation.
    pub fn publish_sidecars<G>(&mut self, universe: &mut crate::Universe<G>) {
        if self.frozen_semantic.is_none() {
            self.frozen_semantic = Some(universe.publish_page_nodes(&self.semantic));
        }
        if self.frozen_physical.is_none() {
            self.frozen_physical = match &self.projection {
                PhysicalProjection::Mirrored => self.frozen_semantic,
                PhysicalProjection::Distinct { nodes, .. } => {
                    Some(universe.publish_page_nodes(nodes))
                }
            };
        }
    }

    #[must_use]
    pub fn frozen_sidecars(&self) -> Option<(PageListId, PageListId)> {
        Some((self.frozen_semantic?, self.frozen_physical?))
    }

    /// Whether this checkpointable sequence explicitly carries a page-arena
    /// coordinate in either its mutable channels or frozen sidecars.
    #[must_use]
    pub fn retains_page_node_handles(&self) -> bool {
        self.frozen_semantic.is_some_and(|list| !list.is_empty())
            || self.frozen_physical.is_some_and(|list| !list.is_empty())
            || self.semantic.iter().any(node_retains_page_handle)
            || match &self.projection {
                PhysicalProjection::Mirrored => false,
                PhysicalProjection::Distinct { nodes, .. } => {
                    nodes.iter().any(node_retains_page_handle)
                }
            }
    }

    fn invalidate_frozen_sidecars(&mut self) {
        self.frozen_semantic = None;
        self.frozen_physical = None;
    }
}

fn node_retains_page_handle(node: &Node) -> bool {
    let mut retains = false;
    node.visit_node_lists(|list| retains |= !list.is_empty());
    retains
}

fn projected_high_cell_lineages(
    semantic: &[Node],
    physical: &[Node],
    boundaries: &[usize],
) -> (
    Vec<DirectHighCellLineages>,
    Vec<DirectHighCellLineages>,
    u32,
) {
    let mut semantic_lineages = Vec::with_capacity(semantic.len());
    let mut physical_lineages = vec![DirectHighCellLineages::new(); physical.len()];
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

fn mirrored_high_cell_lineages(nodes: &[Node]) -> (Vec<DirectHighCellLineages>, u32) {
    let lineages = nodes
        .iter()
        .enumerate()
        .map(|(row, node)| {
            direct_high_cell_lineages(
                node,
                u32::try_from(row).expect("node sequence exceeds u32 rows"),
            )
        })
        .collect();
    let next = u32::try_from(nodes.len()).expect("node sequence exceeds u32 rows");
    (lineages, next)
}

fn direct_high_cell_lineages(node: &Node, row: u32) -> DirectHighCellLineages {
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
    use crate::interner::InternerBudget;
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

    #[test]
    fn mirrored_projection_aliases_nodes_lineages_and_identity_boundaries() {
        let sequence = NodeSequence::mirrored(vec![char_node('A'), Node::Penalty(7)]);
        assert!(std::ptr::eq(
            sequence.semantic().as_ptr(),
            sequence.physical().as_ptr()
        ));
        assert!(std::ptr::eq(
            sequence.semantic_high_cell_lineages().as_ptr(),
            sequence.physical_high_cell_lineages().as_ptr()
        ));
        assert_eq!(sequence.physical_boundary(0), Some(0));
        assert_eq!(sequence.physical_boundary(2), Some(2));
        assert_eq!(sequence.physical_boundary(3), None);
    }

    #[test]
    fn explicit_distinct_projection_remains_distinct_after_mirrored_append() {
        let mut sequence = NodeSequence::mirrored(vec![Node::Penalty(0)]);
        sequence.replace_channels(vec![Node::Penalty(1)], vec![Node::Penalty(2)]);
        sequence.push_mirrored(Node::Penalty(3));
        assert_eq!(sequence.semantic(), &[Node::Penalty(1), Node::Penalty(3)]);
        assert_eq!(sequence.physical(), &[Node::Penalty(2), Node::Penalty(3)]);
        assert!(!std::ptr::eq(
            sequence.semantic().as_ptr(),
            sequence.physical().as_ptr()
        ));
    }

    #[test]
    fn mirrored_truncation_preserves_aliasing_and_lineage_rows() {
        let mut sequence = NodeSequence::mirrored(vec![char_node('A')]);
        sequence.push_mirrored(char_node('B'));
        sequence.truncate(1, 1);
        sequence.push_mirrored(char_node('C'));
        assert!(std::ptr::eq(
            sequence.semantic().as_ptr(),
            sequence.physical().as_ptr()
        ));
        assert_eq!(
            sequence.semantic_high_cell_lineages()[1][0],
            DirectHighCellLineage::Sequence { row: 2, unit: 0 }
        );
    }

    #[test]
    fn mirrored_sidecars_share_one_published_page_list() {
        let budget = InternerBudget::new(32, 32, 1024).expect("budget");
        crate::with_universe(budget, |universe| {
            let mut sequence = NodeSequence::mirrored(vec![Node::Penalty(7)]);
            sequence.publish_sidecars(universe);
            let (semantic, physical) = sequence.frozen_sidecars().expect("published sidecars");
            assert_eq!(semantic, physical);
            assert_eq!(
                universe
                    .page_node_list(semantic)
                    .expect("published page list")
                    .nodes(),
                &[Node::Penalty(7)]
            );

            sequence.push_mirrored(Node::Penalty(8));
            assert_eq!(sequence.frozen_sidecars(), None);
        })
        .expect("fresh universe");
    }

    #[test]
    fn semantic_equality_ignores_explicit_projection_storage() {
        let mirrored = NodeSequence::mirrored(vec![Node::Penalty(1)]);
        let distinct = NodeSequence::from_channels(vec![Node::Penalty(1)], vec![Node::Penalty(99)]);
        assert_eq!(mirrored, distinct);
    }
}
