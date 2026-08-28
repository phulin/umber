//! Runtime page-material ownership above the generic coarse fork arena.
//!
//! The generic arena remains coordinate-only. This facade is the semantic
//! boundary that pairs one canonical physical list coordinate with the
//! optional demand-maintained identity used by state hashing.

use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::num::NonZeroU64;
use std::ops::Range;

use crate::fork_arena::{
    ActiveListBuilder, ArenaListId, ArenaListView, ArenaRange, CheckpointMark, ForkArenaCounters,
    ForkArenaError, OperationMark, PageMaterialLane, SealedBoundary,
};
use crate::node::Node;
use crate::node_region::{NodePool, NodeRegion, PageRole};
use crate::node_sequence::{SemanticSequenceIdentity, semantic_node_identity};

type PageMaterialNode = Node<PageListId>;

/// Persistent coordinate-only construction state for one active node list.
#[must_use = "a page-material active list must be finalized or rolled back"]
pub struct PageMaterialActiveListBuilder {
    inner: ActiveListBuilder<PageMaterialNode, PageMaterialLane>,
    identity: Option<SemanticSequenceIdentity>,
}

impl Default for PageMaterialActiveListBuilder {
    fn default() -> Self {
        Self::vacant()
    }
}

impl PageMaterialActiveListBuilder {
    #[must_use]
    pub const fn vacant() -> Self {
        Self {
            inner: ActiveListBuilder::vacant(),
            identity: None,
        }
    }

    #[must_use]
    pub const fn is_vacant(&self) -> bool {
        self.inner.is_vacant()
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.inner.is_open()
    }
}

/// Canonical runtime coordinate plus its demand-maintained semantic scalar.
pub struct PageListId {
    coordinate: ArenaListId<PageMaterialLane>,
    semantic_identity: Option<NonZeroU64>,
}

impl PageListId {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            coordinate: ArenaListId::empty(),
            semantic_identity: None,
        }
    }

    pub(crate) fn from_parts(
        coordinate: ArenaListId<PageMaterialLane>,
        identity: Option<SemanticSequenceIdentity>,
    ) -> Self {
        assert_eq!(
            identity.map(SemanticSequenceIdentity::len),
            identity.map(|_| coordinate.len()),
            "page-list semantic identity length matches its coordinate"
        );
        let semantic_identity = identity
            .and_then(|identity| NonZeroU64::new(identity.raw()).or(NonZeroU64::new(u64::MAX)));
        Self {
            coordinate,
            semantic_identity,
        }
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.coordinate.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.coordinate.is_empty()
    }

    #[must_use]
    pub(crate) const fn coordinate(self) -> ArenaListId<PageMaterialLane> {
        self.coordinate
    }

    #[must_use]
    pub(crate) const fn belongs_to_arena(self, arena: u32) -> bool {
        self.is_empty() || self.coordinate.arena_identity() == arena
    }

    pub(crate) fn rebrand_arena(self, arena: u32) -> Self {
        Self {
            coordinate: self.coordinate.rebrand_arena(arena),
            semantic_identity: self.semantic_identity,
        }
    }

    pub(crate) fn with_coordinate(self, coordinate: ArenaListId<PageMaterialLane>) -> Self {
        Self {
            coordinate,
            semantic_identity: self.semantic_identity,
        }
    }

    #[must_use]
    pub const fn semantic_identity(self) -> Option<u64> {
        if self.is_empty() {
            Some(0)
        } else {
            match self.semantic_identity {
                Some(identity) => Some(identity.get()),
                None => None,
            }
        }
    }

    #[must_use]
    pub const fn list(self) -> Self {
        self
    }

    #[must_use]
    pub const fn sequence(self) -> Self {
        self
    }

    #[must_use]
    fn sequence_identity(self) -> Option<SemanticSequenceIdentity> {
        self.semantic_identity()
            .map(|hash| SemanticSequenceIdentity::from_raw(hash, self.len()))
    }
}

impl Clone for PageListId {
    fn clone(&self) -> Self {
        *self
    }
}

impl Default for PageListId {
    fn default() -> Self {
        Self::empty()
    }
}

impl Copy for PageListId {}

impl core::fmt::Debug for PageListId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PageListId")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl PartialEq for PageListId {
    fn eq(&self, other: &Self) -> bool {
        self.coordinate == other.coordinate
    }
}

impl Eq for PageListId {}

impl Hash for PageListId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if let Some(identity) = self.semantic_identity {
            identity.hash(state);
        } else {
            self.coordinate.hash(state);
        }
    }
}

const _: () = assert!(core::mem::size_of::<PageListId>() <= 32);

/// Generation-branded durable root into the same runtime page-material arena.
pub struct DurableListId<G> {
    page: PageListId,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> DurableListId<G> {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            page: PageListId::empty(),
            _generation: PhantomData,
        }
    }

    #[must_use]
    pub const fn page(self) -> PageListId {
        self.page
    }

    #[must_use]
    pub const fn rebrand(self) -> PageListId {
        self.page
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.page.is_empty()
    }

    #[must_use]
    pub const fn semantic_identity(self) -> Option<u64> {
        self.page.semantic_identity()
    }
}

impl PageListId {
    #[must_use]
    pub const fn rebrand<G>(self) -> DurableListId<G> {
        DurableListId {
            page: self,
            _generation: PhantomData,
        }
    }
}

impl<G> Clone for DurableListId<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for DurableListId<G> {}

impl<G> core::fmt::Debug for DurableListId<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DurableListId(..)")
    }
}

impl<G> PartialEq for DurableListId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.page == other.page
    }
}

impl<G> Eq for DurableListId<G> {}

impl<G> Hash for DurableListId<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.page.hash(state);
    }
}

/// Runtime page payload owner. Every `Node` is appended exactly once.
pub struct PageMaterialArena {
    pool: NodePool,
    region: NodeRegion<PageRole>,
    range_scratch: Vec<ArenaRange<PageMaterialLane>>,
    coordinate_scratch: Vec<ArenaListId<PageMaterialLane>>,
    semantic_identity_enabled: bool,
    semantic_hash_work: u64,
}

impl Default for PageMaterialArena {
    fn default() -> Self {
        Self::new()
    }
}

impl PageMaterialArena {
    #[must_use]
    pub fn new() -> Self {
        Self::with_chunk_bytes(4 * 1024)
    }

    #[must_use]
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        let mut pool = NodePool::with_chunk_bytes(chunk_bytes);
        let region = pool
            .start_region()
            .expect("page-material region identity capacity");
        Self {
            pool,
            region,
            range_scratch: Vec::new(),
            coordinate_scratch: Vec::new(),
            semantic_identity_enabled: false,
            semantic_hash_work: 0,
        }
    }

    pub fn enable_semantic_identity(&mut self) {
        assert!(
            self.region.pub_arena.counters().new_semantic_nodes == 0
                || self.semantic_identity_enabled,
            "semantic identity demand starts before page-node publication"
        );
        self.semantic_identity_enabled = true;
    }

    #[must_use]
    pub const fn semantic_hash_work(&self) -> u64 {
        self.semantic_hash_work
    }

    #[must_use]
    pub const fn semantic_identity_enabled(&self) -> bool {
        self.semantic_identity_enabled
    }

    #[must_use]
    pub const fn counters(&self) -> ForkArenaCounters {
        self.region.pub_arena.counters()
    }

    /// Returns the generation-checked identity of the exclusive region which
    /// owns every coordinate admitted by this arena.
    #[must_use]
    pub(crate) const fn region_id(&self) -> crate::node_region::NodeRegionId {
        self.region.id()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.region.pub_arena.live_payload_values(&self.pool.chunks)
    }

    pub fn publish_owned(
        &mut self,
        nodes: impl IntoIterator<Item = PageMaterialNode>,
    ) -> Result<PageListId, ForkArenaError> {
        let mut identity = self
            .semantic_identity_enabled
            .then(SemanticSequenceIdentity::empty);
        let mut semantic_hash_work = 0_u64;
        let arena = self.region.pub_arena.region_identity();
        let mut builder = self.region.pub_arena.begin_builder(&mut self.pool.chunks)?;
        for node in nodes {
            if !node_children_belong_to_arena(&node, arena) {
                return Err(ForkArenaError::InvalidRegion);
            }
            if let Some(identity) = &mut identity {
                identity.push_back(semantic_node_identity(&node));
                semantic_hash_work = semantic_hash_work.saturating_add(1);
            }
            builder.push(node)?;
        }
        let coordinate = builder.seal()?;
        self.semantic_hash_work = self.semantic_hash_work.saturating_add(semantic_hash_work);
        Ok(PageListId::from_parts(coordinate, identity))
    }

    /// Test-only negative control for the source-copy counter. Production
    /// transforms have no copy-published entry point and append ranges instead.
    #[cfg(test)]
    pub(crate) fn publish_source_copy(
        &mut self,
        source: PageListId,
    ) -> Result<PageListId, ForkArenaError> {
        let nodes = self.list(source)?.iter().cloned().collect::<Vec<_>>();
        self.region
            .pub_arena
            .record_source_nodes_copied(nodes.len());
        self.publish_owned(nodes)
    }

    pub fn open_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
    ) -> Result<(), ForkArenaError> {
        self.region
            .pub_arena
            .open_active_list(&self.pool.chunks, &mut builder.inner)?;
        builder.identity = self
            .semantic_identity_enabled
            .then(SemanticSequenceIdentity::empty);
        Ok(())
    }

    pub fn push_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        node: PageMaterialNode,
    ) -> Result<(), ForkArenaError> {
        if !node_children_belong_to_arena(&node, self.region.pub_arena.region_identity()) {
            return Err(ForkArenaError::InvalidRegion);
        }
        if let Some(identity) = &mut builder.identity {
            identity.push_back(semantic_node_identity(&node));
            self.semantic_hash_work = self.semantic_hash_work.saturating_add(1);
        }
        self.region
            .pub_arena
            .push_active_list(&mut self.pool.chunks, &mut builder.inner, node)
    }

    pub fn append_to_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        list: PageListId,
    ) -> Result<(), ForkArenaError> {
        self.region.pub_arena.append_active_list(
            &mut self.pool.chunks,
            &mut builder.inner,
            list.coordinate(),
        )?;
        if let Some(identity) = &mut builder.identity {
            *identity = identity.concat(
                list.sequence_identity()
                    .expect("demand-enabled page list carries identity"),
            );
        }
        Ok(())
    }

    pub fn append_range_to_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        list: PageListId,
        selected: Range<usize>,
    ) -> Result<(), ForkArenaError> {
        let selected_identity = if self.semantic_identity_enabled {
            let view = self.list(list)?;
            Some(SemanticSequenceIdentity::from_nodes(
                selected
                    .clone()
                    .map(|index| view.get(index).expect("validated list range")),
            ))
        } else {
            None
        };
        self.region.pub_arena.append_active_list_range(
            &mut self.pool.chunks,
            &mut builder.inner,
            list.coordinate(),
            selected,
        )?;
        if let (Some(identity), Some(selected_identity)) =
            (&mut builder.identity, selected_identity)
        {
            *identity = identity.concat(selected_identity);
        }
        Ok(())
    }

    pub fn finalize_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
    ) -> Result<PageListId, ForkArenaError> {
        self.region
            .pub_arena
            .finalize_active_list(&mut self.pool.chunks, &mut builder.inner)?;
        let coordinate = builder.inner.take_sealed()?;
        Ok(PageListId::from_parts(coordinate, builder.identity.take()))
    }

    pub fn rollback_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
    ) -> Result<(), ForkArenaError> {
        self.region
            .pub_arena
            .rollback_active_list(&mut self.pool.chunks, &mut builder.inner)?;
        builder.identity = None;
        Ok(())
    }

    pub fn publish_range(
        &mut self,
        nodes: Vec<PageMaterialNode>,
    ) -> Result<PageListId, ForkArenaError> {
        self.publish_owned(nodes)
    }

    pub fn compose_sequences(
        &mut self,
        lists: &[PageListId],
    ) -> Result<PageListId, ForkArenaError> {
        let identity = if self.semantic_identity_enabled {
            let mut identity = SemanticSequenceIdentity::empty();
            for list in lists {
                identity = identity.concat(
                    list.sequence_identity()
                        .expect("demand-enabled page list carries identity"),
                );
            }
            Some(identity)
        } else {
            None
        };
        self.compose_with_identity(lists, identity)
    }

    pub fn slice_sequence(
        &mut self,
        list: PageListId,
        selected: Range<usize>,
        _scratch: &mut Vec<PageListId>,
    ) -> Result<PageListId, ForkArenaError> {
        let identity = if self.semantic_identity_enabled {
            let view = self.list(list)?;
            Some(SemanticSequenceIdentity::from_nodes(
                selected
                    .clone()
                    .map(|index| view.get(index).expect("validated list range")),
            ))
        } else {
            None
        };
        self.slice_with_identity(list, selected, identity)
    }

    pub fn slice_with_identity(
        &mut self,
        list: PageListId,
        selected: Range<usize>,
        identity: Option<SemanticSequenceIdentity>,
    ) -> Result<PageListId, ForkArenaError> {
        assert_eq!(self.semantic_identity_enabled, identity.is_some());
        let coordinate = self.region.pub_arena.slice_list(
            &mut self.pool.chunks,
            list.coordinate(),
            selected,
            &mut self.range_scratch,
        )?;
        Ok(PageListId::from_parts(coordinate, identity))
    }

    pub fn compose_with_identity(
        &mut self,
        lists: &[PageListId],
        identity: Option<SemanticSequenceIdentity>,
    ) -> Result<PageListId, ForkArenaError> {
        assert_eq!(self.semantic_identity_enabled, identity.is_some());
        self.coordinate_scratch.clear();
        self.coordinate_scratch
            .extend(lists.iter().map(|list| list.coordinate()));
        let coordinate = self.region.pub_arena.compose_lists(
            &mut self.pool.chunks,
            &self.coordinate_scratch,
            &mut self.range_scratch,
        )?;
        Ok(PageListId::from_parts(coordinate, identity))
    }

    pub fn list(
        &self,
        list: PageListId,
    ) -> Result<ArenaListView<'_, PageMaterialNode, PageMaterialLane>, ForkArenaError> {
        self.region
            .pub_arena
            .list(&self.pool.chunks, list.coordinate())
    }

    pub fn get(
        &self,
        list: PageListId,
    ) -> Result<ArenaListView<'_, PageMaterialNode, PageMaterialLane>, ForkArenaError> {
        self.list(list)
    }

    pub fn node_cursor(
        &self,
        list: PageListId,
    ) -> Result<crate::node_arena::NodeCursor<'_>, ForkArenaError> {
        self.list(list)
            .map(crate::node_arena::NodeCursor::fork_arena)
    }

    pub fn get_sequence(
        &self,
        list: PageListId,
    ) -> Result<crate::node_arena::NodeCursor<'_>, ForkArenaError> {
        self.node_cursor(list)
    }

    #[must_use]
    pub fn contains(&self, list: PageListId) -> bool {
        self.list(list).is_ok()
    }

    #[must_use]
    pub fn operation_mark(&self) -> OperationMark<PageMaterialLane> {
        self.region.pub_arena.operation_mark(&self.pool.chunks)
    }

    pub fn restore_operation(
        &mut self,
        mark: OperationMark<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        self.region
            .pub_arena
            .restore_operation(&mut self.pool.chunks, mark)
    }

    pub fn seal_boundary(&mut self) -> Result<SealedBoundary<PageMaterialLane>, ForkArenaError> {
        self.region.pub_arena.seal_boundary(&mut self.pool.chunks)
    }

    pub fn checkpoint_mark(
        &self,
        boundary: SealedBoundary<PageMaterialLane>,
    ) -> Result<CheckpointMark<PageMaterialLane>, ForkArenaError> {
        self.region.pub_arena.checkpoint_mark(boundary)
    }

    pub fn begin_checkpoint_candidate(
        &mut self,
        mark: CheckpointMark<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        self.region.pub_arena.begin_checkpoint_candidate(mark)
    }

    #[must_use]
    pub fn validates_checkpoint(&self, mark: CheckpointMark<PageMaterialLane>) -> bool {
        self.region.pub_arena.validates_checkpoint(mark)
    }

    #[must_use]
    pub fn can_restore_checkpoint(&self, mark: CheckpointMark<PageMaterialLane>) -> bool {
        self.region.pub_arena.can_begin_checkpoint_candidate(mark)
    }

    pub fn restore_checkpoint(
        &mut self,
        mark: CheckpointMark<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        self.region
            .pub_arena
            .restore_accepted_checkpoint(&mut self.pool.chunks, mark)
    }

    pub fn reject_checkpoint_candidate(
        &mut self,
        boundary: SealedBoundary<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        self.region
            .pub_arena
            .reject_checkpoint_candidate(&mut self.pool.chunks, boundary)
    }

    pub fn accept_checkpoint_candidate(
        &mut self,
        boundary: SealedBoundary<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        self.region
            .pub_arena
            .accept_checkpoint_candidate(&mut self.pool.chunks, boundary)
    }

    /// Recursively copies one exact closure from another page region.
    ///
    /// This is the shared semantic-copy primitive for explicit lifetime
    /// transitions.  Ordinary appends continue to use `publish_owned`; this
    /// path additionally records the exact source-node copy count and rewrites
    /// every child coordinate into the destination region.
    pub(crate) fn copy_closure_from(
        &mut self,
        source: &Self,
        root: PageListId,
    ) -> Result<(PageListId, usize), ForkArenaError> {
        if self.region.id() == source.region.id()
            || self.semantic_identity_enabled != source.semantic_identity_enabled
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        let operation = self.operation_mark();
        let result = copy_closure_between_page_arenas(self, source, root, &mut Vec::new());
        match result {
            Ok((copied, count)) => {
                self.region.pub_arena.record_source_nodes_copied(count);
                Ok((copied, count))
            }
            Err(error) => {
                self.restore_operation(operation)
                    .expect("semantic-copy rollback mark remains valid");
                Err(error)
            }
        }
    }
}

fn node_children_belong_to_arena(node: &PageMaterialNode, arena: u32) -> bool {
    let mut valid = true;
    node.visit_node_lists(|child| valid &= child.belongs_to_arena(arena));
    valid
}

fn copy_closure_between_page_arenas(
    destination: &mut PageMaterialArena,
    source: &PageMaterialArena,
    root: PageListId,
    stack: &mut Vec<PageListId>,
) -> Result<(PageListId, usize), ForkArenaError> {
    if root.is_empty() {
        return Ok((PageListId::empty(), 0));
    }
    if stack.contains(&root) {
        return Err(ForkArenaError::InvalidRegion);
    }
    let nodes = source.list(root)?.iter().cloned().collect::<Vec<_>>();
    stack.push(root);
    let mut copied = Vec::with_capacity(nodes.len());
    let mut count = nodes.len();
    for mut node in nodes {
        let mut child_result = Ok(());
        node.visit_node_lists_mut(|child| {
            if child_result.is_err() {
                return;
            }
            match copy_closure_between_page_arenas(destination, source, *child, stack) {
                Ok((copied, copied_count)) => {
                    *child = copied;
                    count = count.saturating_add(copied_count);
                }
                Err(error) => child_result = Err(error),
            }
        });
        if let Err(error) = child_result {
            stack.pop();
            return Err(error);
        }
        copied.push(node);
    }
    stack.pop();
    destination
        .publish_owned(copied)
        .map(|copied| (copied, count))
}

#[cfg(test)]
#[path = "page_node_arena/tests.rs"]
mod tests;
