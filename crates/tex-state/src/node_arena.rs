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
static NEXT_LIST_GENERATION: AtomicU64 = AtomicU64::new(1);

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
    owner: u64,
    row: u32,
    generation: u64,
    _lifetime: PhantomData<fn(&L) -> &L>,
}

impl<L> NodeListId<L> {
    const EMPTY: Self = Self {
        owner: 0,
        row: 0,
        generation: 0,
        _lifetime: PhantomData,
    };

    const fn from_row(owner: u64, row: u32, generation: u64) -> Self {
        Self {
            owner,
            row,
            generation,
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

    pub(crate) const fn format_validation_coordinate(row: u32) -> Self {
        if row == 0 {
            Self::EMPTY
        } else {
            Self::from_row(1, row, 1)
        }
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
        self.owner == other.owner && self.row == other.row && self.generation == other.generation
    }
}

impl<L> Eq for NodeListId<L> {}

impl<L> core::hash::Hash for NodeListId<L> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.owner.hash(state);
        self.row.hash(state);
        self.generation.hash(state);
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
    rows: Vec<Option<NodeArenaRow<L, Glue, Tokens>>>,
}

struct NodeArenaRow<L, Glue, Tokens> {
    generation: u64,
    nodes: Box<[Node<NodeListId<L>, Glue, Tokens>]>,
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

    /// Validates every font coordinate retained in the cursor's immutable
    /// prefix before a rollback can discard a font-store suffix.
    pub(crate) fn font_roots_are_live(
        &self,
        cursor: NodeArenaCursor<L>,
        mut is_live: impl FnMut(crate::ids::FontId) -> bool,
    ) -> Result<bool, NodeArenaError> {
        self.validate_cursor(cursor)?;
        for row in self.rows[..cursor.rows as usize].iter().flatten() {
            for node in row.nodes.iter() {
                let mut live = true;
                node.visit_fonts(|font| live &= is_live(font));
                if !live {
                    return Ok(false);
                }
            }
        }
        Ok(true)
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
        let generation = NEXT_LIST_GENERATION.fetch_add(1, Ordering::Relaxed);
        assert_ne!(generation, 0, "node-list generation exhausted");
        self.rows.push(Some(NodeArenaRow {
            generation,
            nodes: nodes.into_boxed_slice(),
        }));
        Ok(NodeListId::from_row(self.owner, next, generation))
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
        id.index().is_none_or(|index| {
            id.owner == self.owner
                && self
                    .rows
                    .get(index)
                    .and_then(Option::as_ref)
                    .is_some_and(|row| row.generation == id.generation)
        })
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
            for node in self.rows[index]
                .as_ref()
                .map(|row| row.nodes.as_ref())
                .expect("postorder contains only live rows")
            {
                node.visit_payloads(
                    |value| glue.push(value.clone()),
                    |value| tokens.push(value.clone()),
                );
            }
        }
        Ok((glue, tokens))
    }

    /// Validates the exact closure and reserves its destination rows before
    /// any accompanying durable payload batch is published.
    pub fn reserve_promotion<D, OtherGlue, OtherTokens>(
        &self,
        roots: &[NodeListId<L>],
        destination: &mut NodeArena<D, OtherGlue, OtherTokens>,
    ) -> Result<(), NodeArenaError> {
        let mut state = vec![0_u8; self.rows.len()];
        let mut order = Vec::new();
        for root in roots {
            self.postorder(*root, &mut state, &mut order)?;
        }
        let final_len = destination
            .rows
            .len()
            .checked_add(order.len())
            .ok_or(NodeArenaError::CapacityOverflow)?;
        u32::try_from(final_len).map_err(|_| NodeArenaError::CapacityOverflow)?;
        destination
            .rows
            .try_reserve(order.len())
            .map_err(|_| NodeArenaError::AllocationFailed)
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
            let generation = NEXT_LIST_GENERATION.fetch_add(1, Ordering::Relaxed);
            assert_ne!(generation, 0, "node-list generation exhausted");
            let destination_id = NodeListId::from_row(destination.owner, row, generation);
            let nodes = self.rows[source_index]
                .as_ref()
                .map(|row| row.nodes.as_ref())
                .expect("postorder contains only live rows")
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
            staged.push(Some(NodeArenaRow { generation, nodes }));
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
        if id.owner != self.owner {
            return Err(NodeArenaError::InvalidList);
        }
        let Some(row) = self.rows.get(index).and_then(Option::as_ref) else {
            return Err(NodeArenaError::InvalidList);
        };
        if row.generation != id.generation {
            return Err(NodeArenaError::InvalidList);
        }
        match state[index] {
            2 => return Ok(()),
            1 => return Err(NodeArenaError::CyclicList),
            _ => {}
        }
        state[index] = 1;
        for node in row.nodes.iter() {
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

    /// Drops the exact closure rooted at an exclusively owned completed page.
    ///
    /// The caller must first remove the canonical page root. Unrelated rows
    /// keep their coordinates and storage; released coordinates fail future
    /// resolution instead of becoming aliases for later publications.
    pub fn release_closure(&mut self, root: NodeListId<L>) -> Result<(), NodeArenaError> {
        let mut state = vec![0_u8; self.rows.len()];
        let mut closure = Vec::new();
        self.postorder(root, &mut state, &mut closure)?;
        for id in closure.into_iter().rev() {
            let index = id.index().expect("closure excludes the empty list");
            self.rows[index] = None;
        }
        while self.rows.last().is_some_and(Option::is_none) {
            self.rows.pop();
        }
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

/// Immutable logical node sequence resolved through one arena borrow.
pub type NodeSlice<L, Glue = GlueSpec, Tokens = NodeTokenList> =
    [Node<NodeListId<L>, Glue, Tokens>];

impl<L, Glue, Tokens> core::fmt::Debug for NodeList<'_, L, Glue, Tokens> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NodeList")
            .field("len", &self.nodes().len())
            .finish_non_exhaustive()
    }
}

impl<'a, L, Glue, Tokens> NodeList<'a, L, Glue, Tokens> {
    /// Returns the immutable logical nodes in source order.
    #[must_use]
    pub fn nodes(&self) -> &'a NodeSlice<L, Glue, Tokens> {
        self.id.index().map_or(&[], |index| {
            self.arena.rows[index]
                .as_ref()
                .map(|row| row.nodes.as_ref())
                .expect("validated node-list row is live")
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
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

    /// Resolves one nested coordinate through this list's coarse owner.
    pub fn resolve(self, id: NodeListId<L>) -> Result<Self, NodeArenaError> {
        self.child(id)
    }

    /// Borrows one nested row through this list's coarse owner.
    pub fn child_nodes(
        self,
        id: NodeListId<L>,
    ) -> Result<&'a NodeSlice<L, Glue, Tokens>, NodeArenaError> {
        Ok(self.child(id)?.nodes())
    }
}

impl<'a> NodeList<'a, PageLifetime> {
    #[must_use]
    pub fn get(&self, index: usize) -> Option<NodeRef<'a>> {
        self.nodes().get(index).map(NodeRef::from)
    }

    #[must_use]
    pub fn first(self) -> Option<NodeRef<'a>> {
        self.get(0)
    }

    #[must_use]
    pub fn last(self) -> Option<NodeRef<'a>> {
        self.len().checked_sub(1).and_then(|index| self.get(index))
    }

    pub fn iter(self) -> impl ExactSizeIterator<Item = NodeRef<'a>> {
        self.nodes().iter().map(NodeRef::from)
    }

    #[must_use]
    pub fn contains_direction(self) -> bool {
        self.nodes()
            .iter()
            .any(|node| matches!(node, Node::Direction(_)))
    }

    #[must_use]
    pub fn requires_shipout_normalization(self) -> bool {
        self.nodes().iter().any(node_requires_shipout_normalization)
    }

    #[must_use]
    pub fn node_requires_shipout_normalization(self, index: usize) -> Option<bool> {
        self.nodes()
            .get(index)
            .map(node_requires_shipout_normalization)
    }

    #[must_use]
    pub fn char_run(self, index: usize) -> Option<CharRun<'a>> {
        CharRun::new(self.nodes(), index)
    }

    #[must_use]
    pub fn char_codes(self, index: usize) -> Option<CharCodes<'a>> {
        CharCodes::new(self.nodes(), index)
    }
}

fn node_requires_shipout_normalization(node: &Node) -> bool {
    matches!(
        node,
        Node::HList(_)
            | Node::VList(_)
            | Node::Unset(_)
            | Node::Disc { .. }
            | Node::Ins { .. }
            | Node::Whatsit(_)
            | Node::Direction(_)
            | Node::MathNoad(_)
            | Node::FractionNoad(_)
            | Node::MathStyle(_)
            | Node::MathChoice(_)
            | Node::MathList(_)
            | Node::Nonscript
            | Node::Adjust(_)
            | Node::Glue {
                leader: Some(_),
                ..
            }
    )
}

/// Operation-scratch node storage.
pub type ScratchNodeArena = NodeArena<ScratchLifetime>;

/// Open-mode and page-builder node storage.
pub type PageNodeArena = NodeArena<PageLifetime>;

/// Revision-generation durable node storage.
pub type DurableNodeArena<G> = NodeArena<G, GlueId<G>, TokenListId<G>>;

/// Zero-allocation logical projection of one page-lifetime node.
#[derive(Clone, Debug)]
pub enum NodeRef<'a> {
    Char {
        font: crate::ids::FontId,
        ch: char,
        origin: crate::token::OriginId,
    },
    Lig {
        font: crate::ids::FontId,
        ch: char,
        orig: &'a [char],
        origins: &'a [crate::token::OriginId],
        left_hit: bool,
        right_hit: bool,
    },
    Kern {
        amount: crate::scaled::Scaled,
        kind: crate::node::KernKind,
    },
    MarginKern {
        amount: crate::scaled::Scaled,
        side: crate::node::MarginKernSide,
        font: crate::ids::FontId,
        ch: u8,
    },
    Glue {
        spec: GlueSpec,
        kind: crate::node::GlueKind,
        leader: Option<crate::node::LeaderPayload<PageListId>>,
    },
    Penalty(i32),
    Rule {
        width: Option<crate::scaled::Scaled>,
        height: Option<crate::scaled::Scaled>,
        depth: Option<crate::scaled::Scaled>,
    },
    HList(crate::node::BoxNode<PageListId>),
    VList(crate::node::BoxNode<PageListId>),
    Unset(crate::node::UnsetNode<PageListId>),
    Disc {
        kind: crate::node::DiscKind,
        pre: PageListId,
        post: PageListId,
        replace: PageListId,
        physical_replace_count: u8,
    },
    Mark {
        class: u16,
        tokens: &'a NodeTokenList,
    },
    Ins {
        class: u16,
        size: crate::scaled::Scaled,
        split_top_skip: GlueSpec,
        split_max_depth: crate::scaled::Scaled,
        floating_penalty: i32,
        content: PageListId,
    },
    Whatsit(&'a crate::node::Whatsit),
    MathOn(crate::scaled::Scaled),
    MathOff(crate::scaled::Scaled),
    Direction(crate::node::Direction),
    MathNoad(crate::math::MathNoad<PageListId>),
    FractionNoad(crate::math::MathFraction<PageListId>),
    MathStyle(crate::math::MathStyle),
    MathChoice(crate::math::MathChoice<PageListId>),
    MathList(crate::math::MathListNode<PageListId>),
    Nonscript,
    Adjust(crate::node::AdjustNode<PageListId>),
}

impl<'a> From<&'a Node> for NodeRef<'a> {
    fn from(node: &'a Node) -> Self {
        match node {
            Node::Char { font, ch, origin } => Self::Char {
                font: *font,
                ch: *ch,
                origin: *origin,
            },
            Node::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                origins,
            } => Self::Lig {
                font: *font,
                ch: *ch,
                orig,
                origins,
                left_hit: *left_hit,
                right_hit: *right_hit,
            },
            Node::Kern { amount, kind } => Self::Kern {
                amount: *amount,
                kind: *kind,
            },
            Node::MarginKern {
                amount,
                side,
                font,
                ch,
            } => Self::MarginKern {
                amount: *amount,
                side: *side,
                font: *font,
                ch: *ch,
            },
            Node::Glue { spec, kind, leader } => Self::Glue {
                spec: *spec,
                kind: *kind,
                leader: *leader,
            },
            Node::Penalty(value) => Self::Penalty(*value),
            Node::Rule {
                width,
                height,
                depth,
            } => Self::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Node::HList(value) => Self::HList(*value),
            Node::VList(value) => Self::VList(*value),
            Node::Unset(value) => Self::Unset(*value),
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Self::Disc {
                kind: *kind,
                pre: *pre,
                post: *post,
                replace: *replace,
                physical_replace_count: *physical_replace_count,
            },
            Node::Mark { class, tokens } => Self::Mark {
                class: *class,
                tokens,
            },
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Self::Ins {
                class: *class,
                size: *size,
                split_top_skip: *split_top_skip,
                split_max_depth: *split_max_depth,
                floating_penalty: *floating_penalty,
                content: *content,
            },
            Node::Whatsit(value) => Self::Whatsit(value),
            Node::MathOn(value) => Self::MathOn(*value),
            Node::MathOff(value) => Self::MathOff(*value),
            Node::Direction(value) => Self::Direction(*value),
            Node::MathNoad(value) => Self::MathNoad(value.clone()),
            Node::FractionNoad(value) => Self::FractionNoad(*value),
            Node::MathStyle(value) => Self::MathStyle(*value),
            Node::MathChoice(value) => Self::MathChoice(*value),
            Node::MathList(value) => Self::MathList(*value),
            Node::Nonscript => Self::Nonscript,
            Node::Adjust(value) => Self::Adjust(*value),
        }
    }
}

/// Dimension-bearing projection shared by page-arena and operation buffers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PackedNode<'a> {
    Glyph {
        font: crate::ids::FontId,
        ch: char,
    },
    Kern {
        amount: crate::scaled::Scaled,
        kind: Option<crate::node::KernKind>,
    },
    Glue {
        spec: GlueSpec,
        leader: Option<&'a crate::node::LeaderPayload<PageListId>>,
    },
    Rule {
        width: Option<crate::scaled::Scaled>,
        height: Option<crate::scaled::Scaled>,
        depth: Option<crate::scaled::Scaled>,
    },
    Box(crate::node::BoxNode<PageListId>),
    Unset(crate::node::UnsetNode<PageListId>),
    Disc(PageListId),
    Image {
        width: crate::scaled::Scaled,
        height: crate::scaled::Scaled,
        depth: crate::scaled::Scaled,
    },
    Math(crate::scaled::Scaled),
    Ignored,
}

impl NodeRef<'_> {
    #[must_use]
    pub const fn kind(&self) -> crate::node::NodeKind {
        use crate::node::NodeKind;
        match self {
            Self::Char { .. } => NodeKind::Char,
            Self::Lig { .. } => NodeKind::Lig,
            Self::Kern { .. } => NodeKind::Kern,
            Self::MarginKern { .. } => NodeKind::MarginKern,
            Self::Glue { .. } => NodeKind::Glue,
            Self::Penalty(_) => NodeKind::Penalty,
            Self::Rule { .. } => NodeKind::Rule,
            Self::HList(_) => NodeKind::HList,
            Self::VList(_) => NodeKind::VList,
            Self::Unset(_) => NodeKind::Unset,
            Self::Disc { .. } => NodeKind::Disc,
            Self::Mark { .. } => NodeKind::Mark,
            Self::Ins { .. } => NodeKind::Ins,
            Self::Whatsit(_) => NodeKind::Whatsit,
            Self::MathOn(_) => NodeKind::MathOn,
            Self::MathOff(_) => NodeKind::MathOff,
            Self::Direction(_) => NodeKind::Direction,
            Self::MathNoad(_) => NodeKind::MathNoad,
            Self::FractionNoad(_) => NodeKind::FractionNoad,
            Self::MathStyle(_) => NodeKind::MathStyle,
            Self::MathChoice(_) => NodeKind::MathChoice,
            Self::MathList(_) => NodeKind::MathList,
            Self::Nonscript => NodeKind::Nonscript,
            Self::Adjust(_) => NodeKind::Adjust,
        }
    }

    #[must_use]
    pub const fn etex_type(&self) -> i32 {
        self.kind().etex_type()
    }

    #[must_use]
    pub fn packed(&self) -> PackedNode<'_> {
        match self {
            Self::Char { font, ch, .. } | Self::Lig { font, ch, .. } => PackedNode::Glyph {
                font: *font,
                ch: *ch,
            },
            Self::Kern { amount, kind } => PackedNode::Kern {
                amount: *amount,
                kind: Some(*kind),
            },
            Self::MarginKern { amount, .. } => PackedNode::Kern {
                amount: *amount,
                kind: None,
            },
            Self::Glue { spec, leader, .. } => PackedNode::Glue {
                spec: *spec,
                leader: leader.as_ref(),
            },
            Self::Rule {
                width,
                height,
                depth,
            } => PackedNode::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::HList(value) | Self::VList(value) => PackedNode::Box(*value),
            Self::Unset(value) => PackedNode::Unset(*value),
            Self::Disc { replace, .. } => PackedNode::Disc(*replace),
            Self::Whatsit(
                crate::node::Whatsit::PdfRefXForm {
                    width,
                    height,
                    depth,
                    ..
                }
                | crate::node::Whatsit::PdfRefXImage {
                    width,
                    height,
                    depth,
                    ..
                },
            ) => PackedNode::Image {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::MathOn(value) | Self::MathOff(value) => PackedNode::Math(*value),
            _ => PackedNode::Ignored,
        }
    }

    #[must_use]
    pub fn vertical_dimensions(&self) -> Option<(crate::scaled::Scaled, crate::scaled::Scaled)> {
        match self.packed() {
            PackedNode::Box(node) => Some((node.height, node.depth)),
            PackedNode::Unset(node) => Some((node.height, node.depth)),
            PackedNode::Rule { height, depth, .. } => Some((
                height.unwrap_or(crate::scaled::Scaled::from_raw(0)),
                depth.unwrap_or(crate::scaled::Scaled::from_raw(0)),
            )),
            _ => None,
        }
    }

    #[must_use]
    pub fn box_node(&self) -> Option<crate::node::BoxNode<PageListId>> {
        match self.packed() {
            PackedNode::Box(node) => Some(node),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_owned_with(&self, _resolve: impl FnMut(PageListId) -> PageListId) -> Node {
        match self {
            Self::Char { font, ch, origin } => Node::Char {
                font: *font,
                ch: *ch,
                origin: *origin,
            },
            Self::Lig {
                font,
                ch,
                orig,
                origins,
                left_hit,
                right_hit,
            } => Node::Lig {
                font: *font,
                ch: *ch,
                orig: orig.to_vec(),
                left_hit: *left_hit,
                right_hit: *right_hit,
                origins: origins.to_vec(),
            },
            Self::Kern { amount, kind } => Node::Kern {
                amount: *amount,
                kind: *kind,
            },
            Self::MarginKern {
                amount,
                side,
                font,
                ch,
            } => Node::MarginKern {
                amount: *amount,
                side: *side,
                font: *font,
                ch: *ch,
            },
            Self::Glue { spec, kind, leader } => Node::Glue {
                spec: *spec,
                kind: *kind,
                leader: *leader,
            },
            Self::Penalty(value) => Node::Penalty(*value),
            Self::Rule {
                width,
                height,
                depth,
            } => Node::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::HList(value) => Node::HList(*value),
            Self::VList(value) => Node::VList(*value),
            Self::Unset(value) => Node::Unset(*value),
            Self::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Node::Disc {
                kind: *kind,
                pre: *pre,
                post: *post,
                replace: *replace,
                physical_replace_count: *physical_replace_count,
            },
            Self::Mark { class, tokens } => Node::Mark {
                class: *class,
                tokens: (*tokens).clone(),
            },
            Self::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Node::Ins {
                class: *class,
                size: *size,
                split_top_skip: *split_top_skip,
                split_max_depth: *split_max_depth,
                floating_penalty: *floating_penalty,
                content: *content,
            },
            Self::Whatsit(value) => Node::Whatsit((*value).clone()),
            Self::MathOn(value) => Node::MathOn(*value),
            Self::MathOff(value) => Node::MathOff(*value),
            Self::Direction(value) => Node::Direction(*value),
            Self::MathNoad(value) => Node::MathNoad(value.clone()),
            Self::FractionNoad(value) => Node::FractionNoad(*value),
            Self::MathStyle(value) => Node::MathStyle(*value),
            Self::MathChoice(value) => Node::MathChoice(*value),
            Self::MathList(value) => Node::MathList(*value),
            Self::Nonscript => Node::Nonscript,
            Self::Adjust(value) => Node::Adjust(*value),
        }
    }
}

/// Unified read cursor for operation buffers and immutable page rows.
#[derive(Clone, Copy)]
pub struct NodeCursor<'a> {
    nodes: &'a [Node],
}

impl<'a> NodeCursor<'a> {
    #[must_use]
    pub const fn owned(nodes: &'a [Node]) -> Self {
        Self { nodes }
    }
    #[must_use]
    pub const fn compact(nodes: &'a [Node]) -> Self {
        Self { nodes }
    }
    #[must_use]
    pub const fn len(&self) -> usize {
        self.nodes.len()
    }
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    #[must_use]
    pub fn get(&self, index: usize) -> Option<NodeRef<'a>> {
        self.nodes.get(index).map(NodeRef::from)
    }
    #[must_use]
    pub fn owned_node(&self, index: usize) -> Option<&'a Node> {
        self.nodes.get(index)
    }
    #[must_use]
    pub fn char_codes(&self, index: usize) -> Option<CharCodes<'a>> {
        CharCodes::new(self.nodes, index)
    }
}

/// Lazy same-font byte-character run.
pub struct CharCodes<'a> {
    nodes: &'a [Node],
    next: usize,
    font: crate::ids::FontId,
}

impl<'a> CharCodes<'a> {
    fn new(nodes: &'a [Node], index: usize) -> Option<Self> {
        let Node::Char { font, ch, .. } = nodes.get(index)? else {
            return None;
        };
        u8::try_from(*ch as u32).ok()?;
        Some(Self {
            nodes,
            next: index,
            font: *font,
        })
    }
    #[must_use]
    pub const fn font(&self) -> crate::ids::FontId {
        self.font
    }
}

impl Iterator for CharCodes<'_> {
    type Item = u8;
    fn next(&mut self) -> Option<Self::Item> {
        let Node::Char { font, ch, .. } = self.nodes.get(self.next)? else {
            return None;
        };
        if *font != self.font {
            return None;
        }
        let code = u8::try_from(*ch as u32).ok()?;
        self.next += 1;
        Some(code)
    }
}

/// Borrowed maximal run of page-arena byte characters with one font.
#[derive(Clone, Copy, Debug)]
pub struct CharRun<'a> {
    nodes: &'a [Node],
    start: usize,
    end: usize,
    font: crate::ids::FontId,
}

impl<'a> CharRun<'a> {
    fn new(nodes: &'a [Node], start: usize) -> Option<Self> {
        let Node::Char { font, ch, .. } = nodes.get(start)? else {
            return None;
        };
        u8::try_from(*ch as u32).ok()?;
        let mut end = start + 1;
        while let Some(Node::Char {
            font: candidate,
            ch,
            ..
        }) = nodes.get(end)
        {
            if candidate != font || u8::try_from(*ch as u32).is_err() {
                break;
            }
            end += 1;
        }
        Some(Self {
            nodes,
            start,
            end,
            font: *font,
        })
    }

    #[must_use]
    pub const fn font(self) -> crate::ids::FontId {
        self.font
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.end - self.start
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn codes(self) -> impl ExactSizeIterator<Item = u8> + 'a {
        self.nodes[self.start..self.end].iter().map(|node| {
            let Node::Char { ch, .. } = node else {
                unreachable!("character-run bounds contain only characters")
            };
            u8::try_from(*ch as u32).expect("character-run bounds contain only byte characters")
        })
    }

    pub fn origins(self) -> impl ExactSizeIterator<Item = crate::token::OriginId> + 'a {
        self.nodes[self.start..self.end].iter().map(|node| {
            let Node::Char { origin, .. } = node else {
                unreachable!("character-run bounds contain only characters")
            };
            *origin
        })
    }
}
