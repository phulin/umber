//! Paired semantic and TeX-physical node sequences.

use crate::node::Node;
use crate::node_view::NodeCursor;
use crate::page_node_arena::PageListId;
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
    pub(crate) const fn from_raw(hash: u64, len: usize) -> Self {
        Self { hash, len }
    }
    #[must_use]
    pub const fn empty() -> Self {
        Self { hash: 0, len: 0 }
    }

    #[must_use]
    pub fn from_nodes<'a, List: Hash + 'a>(
        nodes: impl IntoIterator<Item = &'a Node<List>>,
    ) -> Self {
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

    #[cfg(test)]
    fn without_prefix(self, prefix: Self) -> Self {
        assert!(prefix.len <= self.len);
        Self {
            hash: self
                .hash
                .wrapping_sub(prefix.hash)
                .wrapping_mul(sequence_inverse_power(prefix.len)),
            len: self.len - prefix.len,
        }
    }

    #[cfg(test)]
    fn without_suffix(self, suffix: Self) -> Self {
        assert!(suffix.len <= self.len);
        let retained_len = self.len - suffix.len;
        Self {
            hash: self
                .hash
                .wrapping_sub(suffix.hash.wrapping_mul(sequence_power(retained_len))),
            len: retained_len,
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

#[cfg(test)]
fn sequence_inverse_power(mut exponent: usize) -> u64 {
    let mut base = SEQUENCE_MULTIPLIER_INVERSE;
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
pub(crate) fn semantic_node_identity<List: Hash>(node: &Node<List>) -> u64 {
    semantic_value_identity(node)
}

#[must_use]
pub(crate) fn semantic_value_identity(value: &impl Hash) -> u64 {
    let state = RandomState::with_seeds(
        0x756d_6265_725f_6e6f,
        0x6465_5f73_656d_616e,
        0x7469_635f_7631_5f66,
        0x6978_6564_5f73_6565,
    );
    let mut hasher = state.build_hasher();
    hasher.write(NODE_SEMANTIC_IDENTITY_DOMAIN);
    value.hash(&mut hasher);
    hasher.finish()
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

/// Builds allocator-only lineage rows for an immutable mirrored source.
///
/// The returned values are compact transformation scratch.  They do not own
/// or duplicate any node payload from `nodes`.
#[must_use]
pub fn borrowed_mirrored_high_cell_lineages(nodes: &[Node]) -> Vec<DirectHighCellLineages> {
    borrowed_mirrored_high_cell_lineages_from(nodes.iter().map(Into::into))
}

/// Builds allocator lineage scratch while walking a non-contiguous immutable
/// source. Node payload is borrowed and never copied into the scratch.
#[must_use]
pub fn borrowed_mirrored_high_cell_lineages_from<'a>(
    nodes: impl IntoIterator<Item = crate::node_view::NodeView<'a>>,
) -> Vec<DirectHighCellLineages> {
    nodes
        .into_iter()
        .enumerate()
        .map(|(row, node)| {
            direct_high_cell_lineages(
                node,
                u32::try_from(row).expect("node sequence exceeds u32 rows"),
            )
        })
        .collect()
}

/// Builds allocator lineage scratch through the authoritative sequential
/// cursor traversal. Arena-backed sources therefore admit and visit chunks
/// directly instead of resolving every logical row independently.
#[must_use]
pub fn borrowed_mirrored_high_cell_lineages_cursor(
    nodes: NodeCursor<'_>,
) -> Vec<DirectHighCellLineages> {
    let mut lineages = Vec::with_capacity(nodes.len());
    let mut row = 0_u32;
    nodes.for_each(|node| {
        lineages.push(direct_high_cell_lineages(node, row));
        row = row.checked_add(1).expect("node sequence exceeds u32 rows");
    });
    lineages
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

fn direct_high_cell_lineages(
    node: crate::node_view::NodeView<'_>,
    row: u32,
) -> DirectHighCellLineages {
    let count = match node {
        crate::node_view::NodeView::Char { .. } => 1,
        crate::node_view::NodeView::Lig { orig, .. } => orig.len(),
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
    #[test]
    fn sequence_identity_prefix_and_suffix_subtraction_preserve_middle() {
        let mut whole = SemanticSequenceIdentity::empty();
        let mut prefix = SemanticSequenceIdentity::empty();
        let mut middle = SemanticSequenceIdentity::empty();
        let mut suffix = SemanticSequenceIdentity::empty();
        for item in 10..80_u64 {
            whole.push_back(item);
            match item {
                10..20 => prefix.push_back(item),
                20..70 => middle.push_back(item),
                _ => suffix.push_back(item),
            }
        }

        assert_eq!(whole.without_prefix(prefix).without_suffix(suffix), middle);
        assert_eq!(whole.without_suffix(suffix).without_prefix(prefix), middle);
        assert_eq!(prefix.concat(middle).concat(suffix), whole);
    }
}
