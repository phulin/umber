//! Paired semantic and TeX-physical node sequences.

use crate::node::Node;
use crate::node_arena::PageListId;
use ahash::RandomState;
use smallvec::SmallVec;
use std::hash::{BuildHasher, Hash, Hasher};

const NODE_SEMANTIC_IDENTITY_DOMAIN: &[u8] = b"umber-node-semantic-identity-v1";
const SEQUENCE_MULTIPLIER: u64 = 0x9e37_79b1_85eb_ca87;
const SEQUENCE_MULTIPLIER_INVERSE: u64 = 0x0887_4934_32ba_db37;

/// Composable identity of one ordered semantic node lane.
///
/// The polynomial is maintained beside the lane, not recovered from its
/// storage coordinates. Appending, prepending, and consuming either end are
/// constant-time; concatenating the bounded accepted/current regions needs
/// only scalar arithmetic.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SemanticSequenceIdentity {
    hash: u64,
    len: usize,
}

impl SemanticSequenceIdentity {
    #[must_use]
    pub const fn empty() -> Self {
        Self { hash: 0, len: 0 }
    }

    #[must_use]
    pub fn from_nodes<'a>(nodes: impl IntoIterator<Item = &'a Node>) -> Self {
        let mut identity = Self::empty();
        for node in nodes {
            identity.push_back(semantic_node_identity(node));
        }
        identity
    }

    pub fn push_back(&mut self, item: u64) {
        self.hash = self
            .hash
            .wrapping_add(item.wrapping_mul(sequence_power(self.len)));
        self.len += 1;
    }

    pub fn push_front(&mut self, item: u64) {
        self.hash = item.wrapping_add(self.hash.wrapping_mul(SEQUENCE_MULTIPLIER));
        self.len += 1;
    }

    pub fn pop_back(&mut self, item: u64) {
        self.len = self
            .len
            .checked_sub(1)
            .expect("semantic sequence is nonempty");
        self.hash = self
            .hash
            .wrapping_sub(item.wrapping_mul(sequence_power(self.len)));
    }

    pub fn pop_front(&mut self, item: u64) {
        self.len = self
            .len
            .checked_sub(1)
            .expect("semantic sequence is nonempty");
        self.hash = self
            .hash
            .wrapping_sub(item)
            .wrapping_mul(SEQUENCE_MULTIPLIER_INVERSE);
    }

    pub fn replace(&mut self, index: usize, old: u64, new: u64) {
        assert!(index < self.len);
        let power = sequence_power(index);
        self.hash = self
            .hash
            .wrapping_sub(old.wrapping_mul(power))
            .wrapping_add(new.wrapping_mul(power));
    }

    #[must_use]
    pub fn concat(self, suffix: Self) -> Self {
        Self {
            hash: self
                .hash
                .wrapping_add(suffix.hash.wrapping_mul(sequence_power(self.len))),
            len: self.len + suffix.len,
        }
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.hash
    }
}

fn sequence_power(mut exponent: usize) -> u64 {
    let mut base = SEQUENCE_MULTIPLIER;
    let mut power = 1_u64;
    while exponent != 0 {
        if exponent & 1 != 0 {
            power = power.wrapping_mul(base);
        }
        base = base.wrapping_mul(base);
        exponent >>= 1;
    }
    power
}

#[must_use]
pub(crate) fn semantic_node_identity(node: &Node) -> u64 {
    let state = RandomState::with_seeds(
        0x756d_6265_725f_6e6f,
        0x6465_5f73_656d_616e,
        0x7469_635f_7631_5f66,
        0x6978_6564_5f73_6565,
    );
    let mut hasher = state.build_hasher();
    hasher.write(NODE_SEMANTIC_IDENTITY_DOMAIN);
    node.hash(&mut hasher);
    hasher.finish()
}

/// Borrowed semantic view over at most two directly owned regions.
#[derive(Clone, Copy, Debug)]
pub struct NodeSequenceView<'a> {
    prior: &'a [Node],
    current: &'a [Node],
}

impl<'a> NodeSequenceView<'a> {
    #[must_use]
    pub const fn new(prior: &'a [Node], current: &'a [Node]) -> Self {
        Self { prior, current }
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.prior.len() + self.current.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.prior.is_empty() && self.current.is_empty()
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<&'a Node> {
        if index < self.prior.len() {
            self.prior.get(index)
        } else {
            self.current.get(index - self.prior.len())
        }
    }

    #[must_use]
    pub fn first(self) -> Option<&'a Node> {
        self.prior.first().or_else(|| self.current.first())
    }

    #[must_use]
    pub fn last(self) -> Option<&'a Node> {
        self.current.last().or_else(|| self.prior.last())
    }

    pub fn iter(self) -> NodeSequenceIter<'a> {
        NodeSequenceIter {
            view: self,
            front: 0,
            back: self.len(),
        }
    }

    #[must_use]
    pub fn to_vec(self) -> Vec<Node> {
        self.iter().cloned().collect()
    }

    #[must_use]
    pub const fn regions(self) -> (&'a [Node], &'a [Node]) {
        (self.prior, self.current)
    }

    /// Returns the backing slice only when this view occupies one region.
    #[must_use]
    pub fn contiguous(self) -> Option<&'a [Node]> {
        if self.current.is_empty() {
            Some(self.prior)
        } else if self.prior.is_empty() {
            Some(self.current)
        } else {
            None
        }
    }

    #[must_use]
    pub fn expect_contiguous(self) -> &'a [Node] {
        self.contiguous()
            .expect("node sequence occupies more than one lineage region")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NodeSequenceIter<'a> {
    view: NodeSequenceView<'a>,
    front: usize,
    back: usize,
}

impl<'a> Iterator for NodeSequenceIter<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let index = self.front;
        self.front += 1;
        self.view.get(index)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.back - self.front;
        (len, Some(len))
    }
}

impl DoubleEndedIterator for NodeSequenceIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.view.get(self.back)
    }
}

impl ExactSizeIterator for NodeSequenceIter<'_> {}

impl<'a> IntoIterator for NodeSequenceView<'a> {
    type Item = &'a Node;
    type IntoIter = NodeSequenceIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl std::ops::Index<usize> for NodeSequenceView<'_> {
    type Output = Node;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("node sequence index in bounds")
    }
}

impl PartialEq<&[Node]> for NodeSequenceView<'_> {
    fn eq(&self, other: &&[Node]) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<const N: usize> PartialEq<&[Node; N]> for NodeSequenceView<'_> {
    fn eq(&self, other: &&[Node; N]) -> bool {
        self.len() == N && self.iter().eq(other.iter())
    }
}

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
    semantic_high_cell_lineages: Vec<DirectHighCellLineages>,
    next_sequence_lineage_row: u32,
    page_node_root_count: usize,
    semantic_identity_enabled: bool,
    semantic_identity: SemanticSequenceIdentity,
}

/// The detached accepted suffix of one directly mutable node sequence.
///
/// A rooted edit keeps this value only until the aggregate transaction is
/// settled. Rejection appends it back to the selected prefix; acceptance
/// drops it as one obsolete chunk.
#[derive(Debug)]
pub struct NodeSequenceAcceptedTail {
    semantic: Vec<Node>,
    semantic_high_cell_lineages: Vec<DirectHighCellLineages>,
    physical: Option<DistinctAcceptedTail>,
    next_sequence_lineage_row: u32,
    page_node_root_count: usize,
    semantic_identity: SemanticSequenceIdentity,
}

#[derive(Debug)]
struct DistinctAcceptedTail {
    nodes: Vec<Node>,
    boundaries: Vec<usize>,
    high_cell_lineages: Vec<DirectHighCellLineages>,
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
    /// Enables the inline semantic lane before a convergence session mutates it.
    pub fn enable_semantic_identity(&mut self) {
        if !self.semantic_identity_enabled {
            self.semantic_identity = SemanticSequenceIdentity::from_nodes(&self.semantic);
            self.semantic_identity_enabled = true;
        }
    }

    #[must_use]
    pub fn mirrored(nodes: Vec<Node>) -> Self {
        let (semantic_high_cell_lineages, next_sequence_lineage_row) =
            mirrored_high_cell_lineages(&nodes);
        let page_node_root_count = nodes
            .iter()
            .filter(|node| node_retains_page_handle(node))
            .count();
        Self {
            semantic_identity_enabled: false,
            semantic_identity: SemanticSequenceIdentity::empty(),
            semantic: nodes,
            projection: PhysicalProjection::Mirrored,
            semantic_high_cell_lineages,
            next_sequence_lineage_row,
            page_node_root_count,
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
        let page_node_root_count = semantic
            .iter()
            .chain(&physical)
            .filter(|node| node_retains_page_handle(node))
            .count();
        Self {
            semantic_identity_enabled: false,
            semantic_identity: SemanticSequenceIdentity::empty(),
            semantic,
            projection: PhysicalProjection::Distinct {
                nodes: physical,
                boundaries: physical_boundaries,
                high_cell_lineages: physical_high_cell_lineages,
            },
            semantic_high_cell_lineages,
            next_sequence_lineage_row,
            page_node_root_count,
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
    pub fn semantic_view(&self) -> NodeSequenceView<'_> {
        NodeSequenceView::new(&self.semantic, &[])
    }

    #[must_use]
    pub fn physical(&self) -> &[Node] {
        match &self.projection {
            PhysicalProjection::Mirrored => &self.semantic,
            PhysicalProjection::Distinct { nodes, .. } => nodes,
        }
    }

    #[must_use]
    pub fn physical_view(&self) -> NodeSequenceView<'_> {
        let physical = match &self.projection {
            PhysicalProjection::Mirrored => &self.semantic,
            PhysicalProjection::Distinct { nodes, .. } => nodes,
        };
        NodeSequenceView::new(physical, &[])
    }

    /// Rewinds this direct owner to a rooted prefix and returns the accepted
    /// suffix without copying its node payload.
    pub fn split_accepted_tail(
        &mut self,
        semantic_root: usize,
        physical_root: usize,
        page_node_root_count: usize,
        semantic_identity: SemanticSequenceIdentity,
    ) -> NodeSequenceAcceptedTail {
        assert!(semantic_root <= self.semantic.len());
        assert!(physical_root <= self.physical().len());
        let next_sequence_lineage_row = self.next_sequence_lineage_row;
        let accepted_page_node_root_count = self.page_node_root_count;
        let accepted_semantic_identity = self.semantic_identity;
        let semantic = self.semantic.split_off(semantic_root);
        let semantic_high_cell_lineages = self.semantic_high_cell_lineages.split_off(semantic_root);
        let physical = match &mut self.projection {
            PhysicalProjection::Mirrored => {
                assert_eq!(semantic_root, physical_root);
                None
            }
            PhysicalProjection::Distinct {
                nodes,
                boundaries,
                high_cell_lineages,
            } => Some(DistinctAcceptedTail {
                nodes: nodes.split_off(physical_root),
                boundaries: boundaries.split_off(semantic_root + 1),
                high_cell_lineages: high_cell_lineages.split_off(physical_root),
            }),
        };
        self.page_node_root_count = page_node_root_count;
        self.semantic_identity = semantic_identity;
        NodeSequenceAcceptedTail {
            semantic,
            semantic_high_cell_lineages,
            physical,
            next_sequence_lineage_row,
            page_node_root_count: accepted_page_node_root_count,
            semantic_identity: accepted_semantic_identity,
        }
    }

    /// Reattaches a previously detached accepted suffix after candidate undo.
    pub fn restore_accepted_tail(&mut self, mut tail: NodeSequenceAcceptedTail) {
        self.semantic.append(&mut tail.semantic);
        self.semantic_high_cell_lineages
            .append(&mut tail.semantic_high_cell_lineages);
        match (&mut self.projection, tail.physical) {
            (PhysicalProjection::Mirrored, None) => {}
            (
                PhysicalProjection::Distinct {
                    nodes,
                    boundaries,
                    high_cell_lineages,
                },
                Some(mut physical),
            ) => {
                nodes.append(&mut physical.nodes);
                boundaries.append(&mut physical.boundaries);
                high_cell_lineages.append(&mut physical.high_cell_lineages);
            }
            _ => panic!("node projection changed while accepted tail was detached"),
        }
        self.next_sequence_lineage_row = tail.next_sequence_lineage_row;
        self.page_node_root_count = tail.page_node_root_count;
        self.semantic_identity = tail.semantic_identity;
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
                #[cfg(feature = "profiling")]
                crate::measurement::record_node_diagnostic_projection(self.semantic.len());
                let physical = self.semantic.clone();
                (self.semantic, physical)
            }
            PhysicalProjection::Distinct { nodes, .. } => (self.semantic, nodes),
        }
    }

    /// Consumes the builder's semantic channel without materializing a
    /// mirrored diagnostic channel that the caller will immediately discard.
    #[must_use]
    pub fn into_semantic(self) -> Vec<Node> {
        self.semantic
    }

    pub fn into_parts(self) -> (Vec<Node>, Vec<Node>, Vec<usize>) {
        match self.projection {
            PhysicalProjection::Mirrored => {
                #[cfg(feature = "profiling")]
                crate::measurement::record_node_diagnostic_projection(self.semantic.len());
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
        let retains_page_root = node_retains_page_handle(&node);
        let root_multiplier = if matches!(self.projection, PhysicalProjection::Mirrored) {
            1
        } else {
            2
        };
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
        self.page_node_root_count += usize::from(retains_page_root) * root_multiplier;
        if self.semantic_identity_enabled {
            self.semantic_identity.push_back(semantic_node_identity(
                self.semantic.last().expect("pushed semantic node"),
            ));
        }
    }

    pub fn extend_mirrored(&mut self, nodes: impl IntoIterator<Item = Node>) {
        for node in nodes {
            self.push_mirrored(node);
        }
    }

    pub fn replace_channels(&mut self, semantic: Vec<Node>, physical: Vec<Node>) {
        assert_eq!(semantic.len(), physical.len());
        let enabled = self.semantic_identity_enabled;
        *self = Self::from_channels(semantic, physical);
        if enabled {
            self.enable_semantic_identity();
        }
    }

    /// Mutates semantic nodes and atomically resets the physical channel to
    /// the resulting topology. Callers that perform a topology-aware rewrite
    /// replace both channels explicitly instead.
    pub fn mutate_semantic<R>(&mut self, mutate: impl FnOnce(&mut Vec<Node>) -> R) -> R {
        let result = mutate(&mut self.semantic);
        self.projection = PhysicalProjection::Mirrored;
        let (semantic, next_sequence_lineage_row) = mirrored_high_cell_lineages(&self.semantic);
        self.semantic_high_cell_lineages = semantic;
        self.next_sequence_lineage_row = next_sequence_lineage_row;
        self.page_node_root_count = self
            .semantic
            .iter()
            .filter(|node| node_retains_page_handle(node))
            .count();
        if self.semantic_identity_enabled {
            self.semantic_identity = SemanticSequenceIdentity::from_nodes(&self.semantic);
        }
        result
    }

    pub fn truncate(&mut self, semantic_len: usize, physical_len: usize) {
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
        self.page_node_root_count = self
            .semantic
            .iter()
            .filter(|node| node_retains_page_handle(node))
            .count()
            + match &self.projection {
                PhysicalProjection::Mirrored => 0,
                PhysicalProjection::Distinct { nodes, .. } => nodes
                    .iter()
                    .filter(|node| node_retains_page_handle(node))
                    .count(),
            };
        if self.semantic_identity_enabled {
            self.semantic_identity = SemanticSequenceIdentity::from_nodes(&self.semantic);
        }
    }

    /// Restores a checkpointed suffix cursor and its maintained root count.
    /// The count is captured beside the cursor, so rollback never rescans the
    /// retained prefix merely to rediscover page-arena reachability.
    pub fn restore_checkpoint_lengths(
        &mut self,
        semantic_len: usize,
        physical_len: usize,
        page_node_root_count: usize,
        semantic_identity: SemanticSequenceIdentity,
    ) {
        if matches!(self.projection, PhysicalProjection::Mirrored) {
            assert_eq!(semantic_len, physical_len);
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
        self.page_node_root_count = page_node_root_count;
        self.semantic_identity = semantic_identity;
    }

    #[must_use]
    pub const fn semantic_identity(&self) -> SemanticSequenceIdentity {
        self.semantic_identity
    }

    #[must_use]
    pub const fn page_node_root_count(&self) -> usize {
        self.page_node_root_count
    }

    /// Whether this checkpointable sequence explicitly carries a page-arena
    /// coordinate in either its mutable channels or frozen sidecars.
    #[must_use]
    pub fn retains_page_node_handles(&self) -> bool {
        self.page_node_root_count != 0
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
    fn semantic_equality_ignores_explicit_projection_storage() {
        let mirrored = NodeSequence::mirrored(vec![Node::Penalty(1)]);
        let distinct = NodeSequence::from_channels(vec![Node::Penalty(1)], vec![Node::Penalty(99)]);
        assert_eq!(mirrored, distinct);
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn consuming_only_the_semantic_channel_copies_no_node_payload() {
        let sequence = NodeSequence::mirrored(vec![Node::Penalty(1), Node::Penalty(2)]);
        let before = crate::measurement::node_graph_census();

        assert_eq!(
            sequence.into_semantic(),
            [Node::Penalty(1), Node::Penalty(2)]
        );
        let delta = crate::measurement::node_graph_census().saturating_sub(before);
        assert_eq!(delta.physical_copy_rows, 0);
        assert_eq!(delta.physical_copy_nodes, 0);
    }
}
