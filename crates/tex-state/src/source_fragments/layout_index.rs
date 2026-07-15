use std::mem;

use super::{EditorLayoutError, FragmentId};

const NO_INDEX: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
struct RangeMinNode {
    left: u32,
    right: u32,
    piece_index: usize,
}

impl Default for RangeMinNode {
    fn default() -> Self {
        Self {
            left: NO_INDEX,
            right: NO_INDEX,
            piece_index: usize::MAX,
        }
    }
}

/// Static two-dimensional lookup for the views of one fragment.
///
/// `roots[n]` contains the minimum document-order piece index by end offset
/// after inserting the first `n` pieces ordered by start offset. A query first
/// selects the prefix whose starts are no greater than its low offset, then
/// range-mins the suffix whose ends cover its high offset.
#[derive(Debug)]
pub(super) struct FragmentPieceIndex {
    pub(super) fragment: FragmentId,
    starts: Box<[u32]>,
    ends: Box<[u32]>,
    roots: Box<[u32]>,
    nodes: Box<[RangeMinNode]>,
}

impl FragmentPieceIndex {
    pub(super) fn build(
        fragment: FragmentId,
        mut pieces: Vec<(u32, u32, usize)>,
    ) -> Result<Self, EditorLayoutError> {
        pieces.sort_unstable_by_key(|&(start, _, piece_index)| (start, piece_index));
        let mut ends: Vec<u32> = pieces.iter().map(|&(_, end, _)| end).collect();
        ends.sort_unstable();
        ends.dedup();

        let mut starts = Vec::with_capacity(pieces.len());
        let mut roots = Vec::with_capacity(pieces.len().saturating_add(1));
        let mut nodes = Vec::new();
        roots.push(NO_INDEX);
        for (start, end, piece_index) in pieces {
            starts.push(start);
            let end_index = ends.binary_search(&end).expect("piece end was indexed");
            let root = Self::insert(
                &mut nodes,
                *roots.last().expect("empty root is present"),
                0,
                ends.len(),
                end_index,
                piece_index,
            )?;
            roots.push(root);
        }
        Ok(Self {
            fragment,
            starts: starts.into_boxed_slice(),
            ends: ends.into_boxed_slice(),
            roots: roots.into_boxed_slice(),
            nodes: nodes.into_boxed_slice(),
        })
    }

    fn insert(
        nodes: &mut Vec<RangeMinNode>,
        root: u32,
        lower: usize,
        upper: usize,
        end_index: usize,
        piece_index: usize,
    ) -> Result<u32, EditorLayoutError> {
        let mut node = if root == NO_INDEX {
            RangeMinNode::default()
        } else {
            nodes[root as usize]
        };
        node.piece_index = node.piece_index.min(piece_index);
        if upper - lower > 1 {
            let middle = lower + (upper - lower) / 2;
            if end_index < middle {
                node.left = Self::insert(nodes, node.left, lower, middle, end_index, piece_index)?;
            } else {
                node.right =
                    Self::insert(nodes, node.right, middle, upper, end_index, piece_index)?;
            }
        }
        let index = u32::try_from(nodes.len()).map_err(|_| EditorLayoutError::DocumentTooLarge)?;
        if index == NO_INDEX {
            return Err(EditorLayoutError::DocumentTooLarge);
        }
        nodes.push(node);
        Ok(index)
    }

    pub(super) fn covering_piece(&self, lo: u64, hi: u64) -> Option<usize> {
        let required_end = if lo == hi { lo } else { hi };
        let start_count = self.starts.partition_point(|start| u64::from(*start) <= lo);
        let end_index = self
            .ends
            .partition_point(|end| u64::from(*end) < required_end);
        if start_count == 0 || end_index == self.ends.len() {
            return None;
        }
        let piece_index = Self::range_min(
            &self.nodes,
            self.roots[start_count],
            0,
            self.ends.len(),
            end_index,
        );
        (piece_index != usize::MAX).then_some(piece_index)
    }

    fn range_min(
        nodes: &[RangeMinNode],
        root: u32,
        lower: usize,
        upper: usize,
        query_lower: usize,
    ) -> usize {
        if root == NO_INDEX || upper <= query_lower {
            return usize::MAX;
        }
        let node = nodes[root as usize];
        if query_lower <= lower {
            return node.piece_index;
        }
        let middle = lower + (upper - lower) / 2;
        Self::range_min(nodes, node.left, lower, middle, query_lower).min(Self::range_min(
            nodes,
            node.right,
            middle,
            upper,
            query_lower,
        ))
    }

    pub(super) fn retained_bytes(&self) -> usize {
        mem::size_of::<Self>()
            .saturating_add(self.starts.len().saturating_mul(mem::size_of::<u32>()))
            .saturating_add(self.ends.len().saturating_mul(mem::size_of::<u32>()))
            .saturating_add(self.roots.len().saturating_mul(mem::size_of::<u32>()))
            .saturating_add(
                self.nodes
                    .len()
                    .saturating_mul(mem::size_of::<RangeMinNode>()),
            )
    }
}
