//! Arena-owned immutable node lists.
//!
//! Node-list coordinates never own storage. A scratch, page, or revision
//! generation arena owns complete list rows, and callers resolve coordinates
//! only while borrowing that arena. Values cross lifetime boundaries through
//! explicit dense relocation from typed roots.

use core::marker::PhantomData;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::durable_arena::{GlueId, TokenListId};
use crate::glue::GlueSpec;
use crate::node::{Node, NodeTokenList};

#[cfg(test)]
#[path = "node_arena/tests.rs"]
mod tests;

static NEXT_ARENA_OWNER: AtomicU64 = AtomicU64::new(1);

/// Operation-local node-list lifetime.
pub enum ScratchLifetime {}

/// Open-mode and page-builder node-list lifetime.
pub enum PageLifetime {}

/// Copyable coordinate of one immutable list row in a matching arena.
///
/// Row zero is the canonical empty list and resolves without storage. The
/// constructor stays private so a coordinate can be obtained only by arena
/// publication or typed relocation.
pub struct NodeListId<L> {
    row: u32,
    _lifetime: PhantomData<fn(&L) -> &L>,
}

impl<L> NodeListId<L> {
    const EMPTY: Self = Self {
        row: 0,
        _lifetime: PhantomData,
    };

    const fn from_row(row: u32) -> Self {
        Self {
            row,
            _lifetime: PhantomData,
        }
    }

    const fn index(self) -> Option<usize> {
        if self.row == 0 {
            None
        } else {
            Some(self.row as usize - 1)
        }
    }

    /// Returns the canonical empty-list coordinate.
    #[must_use]
    pub const fn empty() -> Self {
        Self::EMPTY
    }

    /// Whether this coordinate names the canonical empty list.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.row == 0
    }
}

impl<L> Clone for NodeListId<L> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<L> Copy for NodeListId<L> {}

impl<L> core::fmt::Debug for NodeListId<L> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NodeListId(..)")
    }
}

impl<L> PartialEq for NodeListId<L> {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row
    }
}

impl<L> Eq for NodeListId<L> {}

impl<L> core::hash::Hash for NodeListId<L> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.row.hash(state);
    }
}

/// List coordinate used by operation scratch.
pub type ScratchListId = NodeListId<ScratchLifetime>;

/// List coordinate used by open modes and the page builder.
pub type PageListId = NodeListId<PageLifetime>;

/// List coordinate retained by one revision generation.
pub type DurableListId<G> = NodeListId<G>;

/// Owner-checked suffix watermark for one node arena.
pub struct NodeArenaCursor<L> {
    owner: u64,
    rows: u32,
    _lifetime: PhantomData<fn(&L) -> &L>,
}

impl<L> Clone for NodeArenaCursor<L> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<L> Copy for NodeArenaCursor<L> {}

impl<L> core::fmt::Debug for NodeArenaCursor<L> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NodeArenaCursor")
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

/// Invalid publication, resolution, relocation, or rollback coordinate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeArenaError {
    CapacityOverflow,
    AllocationFailed,
    ForeignCursor,
    CursorBeyondEnd,
    InvalidList,
    CyclicList,
}

/// One semantic-lifetime owner for immutable node lists.
///
/// Each row owns its node payload directly. Nested lists are copy-only typed
/// coordinates back into this same arena; there is no payload owner, root set,
/// registry lookup, or reference count on a row.
pub struct NodeArena<L, Glue = GlueSpec, Tokens = NodeTokenList> {
    owner: u64,
    rows: Vec<Box<[Node<NodeListId<L>, Glue, Tokens>]>>,
}

impl<L, Glue, Tokens> core::fmt::Debug for NodeArena<L, Glue, Tokens> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NodeArena")
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}

impl<L, Glue, Tokens> Default for NodeArena<L, Glue, Tokens> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L, Glue, Tokens> NodeArena<L, Glue, Tokens> {
    /// Creates an empty lifetime owner.
    #[must_use]
    pub fn new() -> Self {
        let owner = NEXT_ARENA_OWNER.fetch_add(1, Ordering::Relaxed);
        assert_ne!(owner, 0, "node-arena owner identity exhausted");
        Self {
            owner,
            rows: Vec::new(),
        }
    }

    /// Captures the suffix position after canonical roots have been recorded.
    #[must_use]
    pub fn cursor(&self) -> NodeArenaCursor<L> {
        NodeArenaCursor {
            owner: self.owner,
            rows: u32::try_from(self.rows.len()).expect("node arena exceeds u32 rows"),
            _lifetime: PhantomData,
        }
    }

    /// Validates a cursor without mutation.
    pub fn validate_cursor(&self, cursor: NodeArenaCursor<L>) -> Result<(), NodeArenaError> {
        if cursor.owner != self.owner {
            return Err(NodeArenaError::ForeignCursor);
        }
        if cursor.rows as usize > self.rows.len() {
            return Err(NodeArenaError::CursorBeyondEnd);
        }
        Ok(())
    }

    /// Truncates a rejected suffix after callers restore every canonical root.
    pub fn truncate(&mut self, cursor: NodeArenaCursor<L>) -> Result<(), NodeArenaError> {
        self.validate_cursor(cursor)?;
        self.rows.truncate(cursor.rows as usize);
        Ok(())
    }

    /// Publishes one complete list whose children already belong to this arena.
    pub fn publish(
        &mut self,
        nodes: Vec<Node<NodeListId<L>, Glue, Tokens>>,
    ) -> Result<NodeListId<L>, NodeArenaError> {
        if nodes.is_empty() {
            return Ok(NodeListId::empty());
        }
        for node in &nodes {
            let mut valid = true;
            node.visit_node_lists(|child| valid &= self.contains(*child));
            if !valid {
                return Err(NodeArenaError::InvalidList);
            }
        }
        let next = self
            .rows
            .len()
            .checked_add(1)
            .and_then(|row| u32::try_from(row).ok())
            .ok_or(NodeArenaError::CapacityOverflow)?;
        self.rows
            .try_reserve(1)
            .map_err(|_| NodeArenaError::AllocationFailed)?;
        self.rows.push(nodes.into_boxed_slice());
        Ok(NodeListId::from_row(next))
    }

    /// Borrows one complete list.
    pub fn get(&self, id: NodeListId<L>) -> Result<NodeList<'_, L, Glue, Tokens>, NodeArenaError> {
        if !self.contains(id) {
            return Err(NodeArenaError::InvalidList);
        }
        Ok(NodeList { arena: self, id })
    }

    /// Whether a coordinate resolves directly in this owner.
    #[must_use]
    pub fn contains(&self, id: NodeListId<L>) -> bool {
        id.index().is_none_or(|index| index < self.rows.len())
    }

    /// Copies only the closures reachable from explicit roots into another
    /// lifetime, rewriting every child through a dense relocation vector.
    ///
    /// Staging completes before destination publication. Invalid coordinates
    /// or cycles leave the destination unchanged.
    pub fn promote_into<D>(
        &self,
        roots: &[NodeListId<L>],
        destination: &mut NodeArena<D, Glue, Tokens>,
    ) -> Result<Vec<NodeListId<D>>, NodeArenaError>
    where
        Glue: Clone,
        Tokens: Clone,
    {
        self.promote_into_with(
            roots,
            destination,
            core::convert::identity,
            core::convert::identity,
        )
    }

    /// Collects semantic payload roots in the same deterministic order used
    /// by relocation, without scanning unrelated arena rows.
    pub fn escaping_payloads(
        &self,
        roots: &[NodeListId<L>],
    ) -> Result<(Vec<Glue>, Vec<Tokens>), NodeArenaError>
    where
        Glue: Clone,
        Tokens: Clone,
    {
        let mut state = vec![0_u8; self.rows.len()];
        let mut order = Vec::new();
        for root in roots {
            self.postorder(*root, &mut state, &mut order)?;
        }
        let mut glue = Vec::new();
        let mut tokens = Vec::new();
        for list in order {
            let index = list.index().expect("postorder excludes empty lists");
            for node in &self.rows[index] {
                node.visit_payloads(
                    |value| glue.push(value.clone()),
                    |value| tokens.push(value.clone()),
                );
            }
        }
        Ok((glue, tokens))
    }

    /// Relocates an exact closure while changing its semantic payload
    /// coordinates, as page values become generation-durable ids.
    pub fn promote_into_with<D, OtherGlue, OtherTokens>(
        &self,
        roots: &[NodeListId<L>],
        destination: &mut NodeArena<D, OtherGlue, OtherTokens>,
        mut map_glue: impl FnMut(Glue) -> OtherGlue,
        mut map_tokens: impl FnMut(Tokens) -> OtherTokens,
    ) -> Result<Vec<NodeListId<D>>, NodeArenaError>
    where
        Glue: Clone,
        Tokens: Clone,
    {
        let mut state = vec![0_u8; self.rows.len()];
        let mut order = Vec::new();
        for root in roots {
            self.postorder(*root, &mut state, &mut order)?;
        }

        let destination_base = destination.rows.len();
        let final_len = destination_base
            .checked_add(order.len())
            .ok_or(NodeArenaError::CapacityOverflow)?;
        u32::try_from(final_len).map_err(|_| NodeArenaError::CapacityOverflow)?;
        destination
            .rows
            .try_reserve(order.len())
            .map_err(|_| NodeArenaError::AllocationFailed)?;

        let mut relocation = vec![None; self.rows.len()];
        let mut staged = Vec::new();
        staged
            .try_reserve(order.len())
            .map_err(|_| NodeArenaError::AllocationFailed)?;
        for source in order {
            let source_index = source.index().expect("postorder excludes empty lists");
            let row = destination_base
                .checked_add(staged.len())
                .and_then(|index| index.checked_add(1))
                .and_then(|row| u32::try_from(row).ok())
                .ok_or(NodeArenaError::CapacityOverflow)?;
            let destination_id = NodeListId::from_row(row);
            let nodes = self.rows[source_index]
                .iter()
                .cloned()
                .map(|node| {
                    node.map_lists(|child| {
                        child.index().map_or_else(NodeListId::empty, |index| {
                            relocation[index].expect("postorder relocates children before parents")
                        })
                    })
                    .map_payloads(&mut map_glue, &mut map_tokens)
                })
                .collect::<Vec<_>>()
                .into_boxed_slice();
            relocation[source_index] = Some(destination_id);
            staged.push(nodes);
        }

        let promoted_roots = roots
            .iter()
            .map(|root| {
                root.index().map_or_else(NodeListId::empty, |index| {
                    relocation[index].expect("every explicit root was relocated")
                })
            })
            .collect();
        destination.rows.extend(staged);
        Ok(promoted_roots)
    }

    fn postorder(
        &self,
        id: NodeListId<L>,
        state: &mut [u8],
        order: &mut Vec<NodeListId<L>>,
    ) -> Result<(), NodeArenaError> {
        let Some(index) = id.index() else {
            return Ok(());
        };
        let Some(row) = self.rows.get(index) else {
            return Err(NodeArenaError::InvalidList);
        };
        match state[index] {
            2 => return Ok(()),
            1 => return Err(NodeArenaError::CyclicList),
            _ => {}
        }
        state[index] = 1;
        for node in row {
            let mut result = Ok(());
            node.visit_node_lists(|child| {
                if result.is_ok() {
                    result = self.postorder(*child, state, order);
                }
            });
            result?;
        }
        state[index] = 2;
        order.push(id);
        Ok(())
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.rows.len()
    }
}

/// Borrowed node-list view; dropping it has no ownership effect.
#[derive(Clone, Copy)]
pub struct NodeList<'a, L, Glue = GlueSpec, Tokens = NodeTokenList> {
    arena: &'a NodeArena<L, Glue, Tokens>,
    id: NodeListId<L>,
}

impl<'a, L, Glue, Tokens> NodeList<'a, L, Glue, Tokens> {
    /// Returns the immutable logical nodes in source order.
    #[must_use]
    pub fn nodes(self) -> &'a [Node<NodeListId<L>, Glue, Tokens>] {
        self.id
            .index()
            .map_or(&[], |index| self.arena.rows[index].as_ref())
    }

    #[must_use]
    pub fn len(self) -> usize {
        self.nodes().len()
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.id.is_empty()
    }

    #[must_use]
    pub const fn id(self) -> NodeListId<L> {
        self.id
    }

    /// Resolves one child using the same coarse owner borrow.
    pub fn child(self, id: NodeListId<L>) -> Result<Self, NodeArenaError> {
        self.arena.get(id)
    }
}

/// Operation-scratch node storage.
pub type ScratchNodeArena = NodeArena<ScratchLifetime>;

/// Open-mode and page-builder node storage.
pub type PageNodeArena = NodeArena<PageLifetime>;

/// Revision-generation durable node storage.
pub type DurableNodeArena<G> = NodeArena<G, GlueId<G>, TokenListId<G>>;
