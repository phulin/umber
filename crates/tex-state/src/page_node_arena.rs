//! Runtime page-material ownership above the generic coarse fork arena.
//!
//! The generic arena remains coordinate-only. This facade is the semantic
//! boundary that pairs one canonical physical list coordinate with the
//! optional demand-maintained identity used by state hashing.

use core::hash::{Hash, Hasher};
use core::num::NonZeroU64;
use std::ops::Range;

use crate::fork_arena::{
    ActiveListBuilder, ArenaListId, ArenaListView, ArenaRange, CheckpointMark, ForkArenaCounters,
    ForkArenaError, OperationMark, PageMaterialLane, SealedBoundary,
};
use crate::node::Node;
use crate::node_region::{
    ClosureBuildMark, CompatibilityClosureBuildReceipt, DurableRole, NodePool, NodeRegion,
    OwnedNodeClosure, PageRole, copy_closure_into, copy_region_root_into, transfer_closure_into,
};
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

/// Region-local page payload state. The physical pool is owned once by the
/// enclosing page-region history and is borrowed explicitly for every access.
pub struct PageMaterialRegion {
    region: NodeRegion<PageRole>,
    range_scratch: Vec<ArenaRange<PageMaterialLane>>,
    coordinate_scratch: Vec<ArenaListId<PageMaterialLane>>,
    semantic_identity_enabled: bool,
    durable_transitions: DurableTransitionCounters,
}

/// Move-only durable node closure owned by an eqtb or PDF carrier.
pub(crate) type DurableNodeClosure = OwnedNodeClosure<DurableRole>;

/// Demand-free observations of explicit durable lifetime transitions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DurableTransitionCounters {
    pub(crate) page_to_durable_nodes_copied: u64,
    pub(crate) tex_copy_nodes_copied: u64,
    pub(crate) history_preservation_nodes_copied: u64,
    pub(crate) nested_closure_nodes_copied: u64,
    pub(crate) node_closure_scan_nodes: u64,
}

/// Exclusive admitted access to one page-material region and the shared pool.
pub struct PageMaterialArena<'a> {
    pool: &'a mut NodePool,
    region: &'a mut NodeRegion<PageRole>,
    range_scratch: &'a mut Vec<ArenaRange<PageMaterialLane>>,
    coordinate_scratch: &'a mut Vec<ArenaListId<PageMaterialLane>>,
    semantic_identity_enabled: &'a mut bool,
    durable_transitions: &'a mut DurableTransitionCounters,
}

impl PageMaterialRegion {
    pub fn new(pool: &mut NodePool) -> Self {
        let region = pool
            .start_region()
            .expect("page-material region identity capacity");
        Self {
            region,
            range_scratch: Vec::new(),
            coordinate_scratch: Vec::new(),
            semantic_identity_enabled: false,
            durable_transitions: DurableTransitionCounters::default(),
        }
    }

    pub(crate) const fn region_id(&self) -> crate::node_region::NodeRegionId {
        self.region.id()
    }

    pub(crate) fn retire(self, pool: &mut NodePool) -> Result<(), ForkArenaError> {
        pool.retire_region(self.region).map_err(|(error, _)| error)
    }

    pub(crate) fn copy_closure_between(
        pool: &mut NodePool,
        destination: &mut Self,
        source: &Self,
        root: PageListId,
    ) -> Result<(PageListId, usize), ForkArenaError> {
        if destination.region.id() == source.region.id()
            || destination.semantic_identity_enabled != source.semantic_identity_enabled
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        let source_root = source.region.root(pool, root)?;
        let before = destination.region.counters().source_nodes_copied;
        let copied = copy_region_root_into(
            pool,
            &source.region,
            source_root,
            &mut destination.region,
            source.semantic_identity_enabled,
        )?;
        let count = destination
            .region
            .counters()
            .source_nodes_copied
            .saturating_sub(before) as usize;
        Ok((copied.page_list(), count))
    }
}

impl<'a> PageMaterialArena<'a> {
    pub fn new(pool: &'a mut NodePool, state: &'a mut PageMaterialRegion) -> Self {
        Self {
            pool,
            region: &mut state.region,
            range_scratch: &mut state.range_scratch,
            coordinate_scratch: &mut state.coordinate_scratch,
            semantic_identity_enabled: &mut state.semantic_identity_enabled,
            durable_transitions: &mut state.durable_transitions,
        }
    }

    pub fn enable_semantic_identity(&mut self) {
        assert!(
            self.region.pub_arena.counters().new_semantic_nodes == 0
                || *self.semantic_identity_enabled,
            "semantic identity demand starts before page-node publication"
        );
        *self.semantic_identity_enabled = true;
    }

    #[must_use]
    pub const fn semantic_hash_work(&self) -> u64 {
        self.region.pub_arena.counters().identity_nodes_hashed
    }

    /// Stored whole-range and whole-chunk summaries combined for identity.
    #[must_use]
    pub const fn semantic_summary_work(&self) -> u64 {
        self.region.pub_arena.counters().identity_summaries_combined
    }

    #[must_use]
    pub fn semantic_identity_enabled(&self) -> bool {
        *self.semantic_identity_enabled
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
    #[must_use]
    pub(crate) const fn durable_transition_counters(&self) -> DurableTransitionCounters {
        *self.durable_transitions
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.region.pub_arena.live_payload_values(&self.pool.chunks)
    }

    pub fn publish_owned(
        &mut self,
        nodes: impl IntoIterator<Item = PageMaterialNode>,
    ) -> Result<PageListId, ForkArenaError> {
        let mut identity = (*self.semantic_identity_enabled).then(SemanticSequenceIdentity::empty);
        let arena = self.region.pub_arena.region_identity();
        let mut builder = self.region.pub_arena.begin_builder(&mut self.pool.chunks)?;
        for node in nodes {
            if !node_children_belong_to_arena(&node, arena) {
                return Err(ForkArenaError::InvalidRegion);
            }
            if let Some(identity) = &mut identity {
                let item_identity = semantic_node_identity(&node);
                identity.push_back(item_identity);
                builder.push_summarized(node, item_identity)?;
            } else {
                builder.push(node)?;
            }
        }
        let coordinate = builder.seal()?;
        if let Some(identity) = identity {
            self.region
                .pub_arena
                .record_identity_work(crate::fork_arena::SequenceSummaryWork {
                    hashed_values: identity.len() as u64,
                    ..crate::fork_arena::SequenceSummaryWork::default()
                });
        }
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

    /// Cold-copies a page root into a fresh self-contained durable region.
    ///
    /// Page-region carrier migration will replace this explicit transition
    /// with whole-envelope movement when the selected closure is independently
    /// transferable. The current compatibility page region is larger than one
    /// box, so treating this as a move would create a cross-owner coordinate.
    pub(crate) fn copy_page_root_to_durable(
        &mut self,
        root: PageListId,
    ) -> Result<DurableNodeClosure, ForkArenaError> {
        let source = self.region.root(self.pool, root)?;
        let mut durable = self.pool.start_region::<DurableRole>()?;
        let copied = match copy_region_root_into(
            self.pool,
            self.region,
            source,
            &mut durable,
            *self.semantic_identity_enabled,
        ) {
            Ok(root) => root,
            Err(error) => {
                assert!(
                    self.pool.retire_region(durable).is_ok(),
                    "empty durable copy destination retires"
                );
                return Err(error);
            }
        };
        let copied_nodes = durable.counters().source_nodes_copied;
        let closure = durable
            .into_closure(self.pool, copied)
            .map_err(|(error, region)| {
                assert!(
                    self.pool.retire_region(region).is_ok(),
                    "validated durable copy destination retires"
                );
                error
            })?;
        self.durable_transitions.page_to_durable_nodes_copied = self
            .durable_transitions
            .page_to_durable_nodes_copied
            .saturating_add(copied_nodes);
        self.durable_transitions.node_closure_scan_nodes = self
            .durable_transitions
            .node_closure_scan_nodes
            .saturating_add(copied_nodes);
        Ok(closure)
    }

    /// Consumes a unique durable owner and transfers its exact addresses into
    /// the current page region.
    #[allow(clippy::result_large_err)] // Failed transfer must return the move-only durable owner.
    pub(crate) fn move_durable_to_page(
        &mut self,
        closure: DurableNodeClosure,
    ) -> Result<PageListId, (ForkArenaError, DurableNodeClosure)> {
        transfer_closure_into(self.pool, closure, self.region)
            .map(|root| root.list())
            .map_err(|failure| (failure.error, failure.closure))
    }

    /// Implements TeX's explicit recursive copy while retaining the source
    /// durable owner.
    pub(crate) fn copy_durable_to_page(
        &mut self,
        closure: &DurableNodeClosure,
    ) -> Result<PageListId, ForkArenaError> {
        let before = self.region.counters().source_nodes_copied;
        let root = copy_closure_into(
            self.pool,
            closure,
            self.region,
            *self.semantic_identity_enabled,
        )?;
        let copied = self
            .region
            .counters()
            .source_nodes_copied
            .saturating_sub(before);
        self.durable_transitions.tex_copy_nodes_copied = self
            .durable_transitions
            .tex_copy_nodes_copied
            .saturating_add(copied);
        self.durable_transitions.node_closure_scan_nodes = self
            .durable_transitions
            .node_closure_scan_nodes
            .saturating_add(copied);
        Ok(root.list())
    }

    pub(crate) fn copy_history_preserved_to_page(
        &mut self,
        closure: &DurableNodeClosure,
    ) -> Result<PageListId, ForkArenaError> {
        let before = self.region.counters().source_nodes_copied;
        let root = copy_closure_into(
            self.pool,
            closure,
            self.region,
            *self.semantic_identity_enabled,
        )?;
        let copied = self
            .region
            .counters()
            .source_nodes_copied
            .saturating_sub(before);
        self.durable_transitions.history_preservation_nodes_copied = self
            .durable_transitions
            .history_preservation_nodes_copied
            .saturating_add(copied);
        self.durable_transitions.node_closure_scan_nodes = self
            .durable_transitions
            .node_closure_scan_nodes
            .saturating_add(copied);
        Ok(root.list())
    }

    /// Copies one durable closure into a fresh independently owned durable
    /// region for a semantically required historical or nested owner.
    pub(crate) fn copy_durable_owner(
        &mut self,
        closure: &DurableNodeClosure,
    ) -> Result<DurableNodeClosure, ForkArenaError> {
        let mut destination = self.pool.start_region::<DurableRole>()?;
        let copied = match copy_closure_into(
            self.pool,
            closure,
            &mut destination,
            *self.semantic_identity_enabled,
        ) {
            Ok(root) => root,
            Err(error) => {
                assert!(
                    self.pool.retire_region(destination).is_ok(),
                    "empty durable copy destination retires"
                );
                return Err(error);
            }
        };
        let copied_nodes = destination.counters().source_nodes_copied;
        let closure = destination
            .into_closure(self.pool, copied)
            .map_err(|(error, region)| {
                assert!(
                    self.pool.retire_region(region).is_ok(),
                    "validated durable copy destination retires"
                );
                error
            })?;
        self.durable_transitions.history_preservation_nodes_copied = self
            .durable_transitions
            .history_preservation_nodes_copied
            .saturating_add(copied_nodes);
        self.durable_transitions.node_closure_scan_nodes = self
            .durable_transitions
            .node_closure_scan_nodes
            .saturating_add(copied_nodes);
        Ok(closure)
    }

    /// Borrows one durable closure under the matching pool owner.
    pub(crate) fn durable_list<'b>(
        &'b self,
        closure: &'b DurableNodeClosure,
    ) -> Result<crate::node_region::RegionList<'b, DurableRole>, ForkArenaError> {
        closure.list(self.pool)
    }

    pub(crate) fn durable_child_list<'b>(
        &'b self,
        closure: &'b DurableNodeClosure,
        child: PageListId,
    ) -> Result<crate::node_region::RegionList<'b, DurableRole>, ForkArenaError> {
        closure.child_list(self.pool, child)
    }

    /// Drops one exact durable owner and returns its envelopes to the pool.
    pub(crate) fn retire_durable(
        &mut self,
        closure: DurableNodeClosure,
    ) -> Result<(), ForkArenaError> {
        self.pool
            .retire_region(closure.into_region())
            .map_err(|(error, _)| error)
    }

    #[cfg(test)]
    pub(crate) fn durable_region_is_live(&self, id: crate::node_region::NodeRegionId) -> bool {
        self.pool.validates_id(id)
    }

    pub fn open_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
    ) -> Result<(), ForkArenaError> {
        self.region
            .pub_arena
            .open_active_list(&self.pool.chunks, &mut builder.inner)?;
        builder.identity = (*self.semantic_identity_enabled).then(SemanticSequenceIdentity::empty);
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
            let item_identity = semantic_node_identity(&node);
            identity.push_back(item_identity);
            let result = self.region.pub_arena.push_active_list_summarized(
                &mut self.pool.chunks,
                &mut builder.inner,
                node,
                item_identity,
            );
            if result.is_ok() {
                self.region.pub_arena.record_identity_work(
                    crate::fork_arena::SequenceSummaryWork {
                        hashed_values: 1,
                        ..crate::fork_arena::SequenceSummaryWork::default()
                    },
                );
            }
            result
        } else {
            self.region
                .pub_arena
                .push_active_list(&mut self.pool.chunks, &mut builder.inner, node)
        }
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
            self.region
                .pub_arena
                .record_identity_work(crate::fork_arena::SequenceSummaryWork {
                    combined_summaries: 1,
                    ..crate::fork_arena::SequenceSummaryWork::default()
                });
        }
        Ok(())
    }

    pub fn append_range_to_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        list: PageListId,
        selected: Range<usize>,
    ) -> Result<(), ForkArenaError> {
        let selected_identity = if *self.semantic_identity_enabled {
            let (identity, work) = self.region.pub_arena.append_active_list_range_summarized(
                &mut self.pool.chunks,
                &mut builder.inner,
                list.coordinate(),
                selected,
                semantic_node_identity,
            )?;
            self.region.pub_arena.record_identity_work(work);
            Some(identity)
        } else {
            self.region.pub_arena.append_active_list_range(
                &mut self.pool.chunks,
                &mut builder.inner,
                list.coordinate(),
                selected,
            )?;
            None
        };
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
        let identity = if *self.semantic_identity_enabled {
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
        self.coordinate_scratch.clear();
        self.coordinate_scratch
            .extend(lists.iter().map(|list| list.coordinate()));
        let coordinate = self.region.pub_arena.compose_lists(
            &mut self.pool.chunks,
            self.coordinate_scratch,
            self.range_scratch,
        )?;
        if identity.is_some() {
            self.region
                .pub_arena
                .record_identity_work(crate::fork_arena::SequenceSummaryWork {
                    combined_summaries: lists.len() as u64,
                    ..crate::fork_arena::SequenceSummaryWork::default()
                });
        }
        Ok(PageListId::from_parts(coordinate, identity))
    }

    pub fn slice_sequence(
        &mut self,
        list: PageListId,
        selected: Range<usize>,
        _scratch: &mut Vec<PageListId>,
    ) -> Result<PageListId, ForkArenaError> {
        if *self.semantic_identity_enabled {
            let (coordinate, identity, work) = self.region.pub_arena.slice_list_summarized(
                &mut self.pool.chunks,
                list.coordinate(),
                selected,
                self.range_scratch,
                semantic_node_identity,
            )?;
            self.region.pub_arena.record_identity_work(work);
            Ok(PageListId::from_parts(coordinate, Some(identity)))
        } else {
            let coordinate = self.region.pub_arena.slice_list(
                &mut self.pool.chunks,
                list.coordinate(),
                selected,
                self.range_scratch,
            )?;
            Ok(PageListId::from_parts(coordinate, None))
        }
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

    pub fn begin_closure_build(&mut self) -> Result<ClosureBuildMark<PageRole>, ForkArenaError> {
        self.region.begin_closure_build(self.pool)
    }

    pub fn cancel_closure_build(
        &mut self,
        mark: ClosureBuildMark<PageRole>,
    ) -> Result<(), ForkArenaError> {
        self.region.cancel_closure_build(self.pool, mark)
    }

    pub fn compatibility_closure_build_receipt(
        &self,
        mark: ClosureBuildMark<PageRole>,
    ) -> Result<CompatibilityClosureBuildReceipt<PageRole>, ForkArenaError> {
        self.region.compatibility_closure_build_receipt(mark)
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
}

fn node_children_belong_to_arena(node: &PageMaterialNode, arena: u32) -> bool {
    let mut valid = true;
    node.visit_node_lists(|child| valid &= child.belongs_to_arena(arena));
    valid
}

/// Read-only admitted access used by retained history and format capture.
pub struct PageMaterialView<'a> {
    pool: &'a NodePool,
    state: &'a PageMaterialRegion,
}

impl<'a> PageMaterialView<'a> {
    pub const fn new(pool: &'a NodePool, state: &'a PageMaterialRegion) -> Self {
        Self { pool, state }
    }

    #[must_use]
    pub const fn semantic_identity_enabled(&self) -> bool {
        self.state.semantic_identity_enabled
    }

    #[must_use]
    pub const fn semantic_hash_work(&self) -> u64 {
        self.state.region.pub_arena.counters().identity_nodes_hashed
    }

    #[must_use]
    pub const fn semantic_summary_work(&self) -> u64 {
        self.state
            .region
            .pub_arena
            .counters()
            .identity_summaries_combined
    }

    #[must_use]
    pub const fn counters(&self) -> ForkArenaCounters {
        self.state.region.pub_arena.counters()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.state
            .region
            .pub_arena
            .live_payload_values(&self.pool.chunks)
    }

    pub fn list(
        &self,
        list: PageListId,
    ) -> Result<ArenaListView<'a, PageMaterialNode, PageMaterialLane>, ForkArenaError> {
        self.state
            .region
            .pub_arena
            .list(&self.pool.chunks, list.coordinate())
    }

    pub fn get(
        &self,
        list: PageListId,
    ) -> Result<ArenaListView<'a, PageMaterialNode, PageMaterialLane>, ForkArenaError> {
        self.list(list)
    }

    pub fn node_cursor(
        &self,
        list: PageListId,
    ) -> Result<crate::node_arena::NodeCursor<'a>, ForkArenaError> {
        self.list(list)
            .map(crate::node_arena::NodeCursor::fork_arena)
    }

    pub fn get_sequence(
        &self,
        list: PageListId,
    ) -> Result<crate::node_arena::NodeCursor<'a>, ForkArenaError> {
        self.node_cursor(list)
    }

    pub(crate) fn durable_list(
        &self,
        closure: &'a DurableNodeClosure,
    ) -> Result<crate::node_region::RegionList<'a, DurableRole>, ForkArenaError> {
        closure.list(self.pool)
    }

    pub(crate) fn durable_child_list(
        &self,
        closure: &'a DurableNodeClosure,
        child: PageListId,
    ) -> Result<crate::node_region::RegionList<'a, DurableRole>, ForkArenaError> {
        closure.child_list(self.pool, child)
    }

    #[must_use]
    pub fn contains(&self, list: PageListId) -> bool {
        self.list(list).is_ok()
    }

    #[must_use]
    pub fn operation_mark(&self) -> OperationMark<PageMaterialLane> {
        self.state
            .region
            .pub_arena
            .operation_mark(&self.pool.chunks)
    }

    #[must_use]
    pub fn validates_checkpoint(&self, mark: CheckpointMark<PageMaterialLane>) -> bool {
        self.state.region.pub_arena.validates_checkpoint(mark)
    }

    #[must_use]
    pub fn can_restore_checkpoint(&self, mark: CheckpointMark<PageMaterialLane>) -> bool {
        self.state
            .region
            .pub_arena
            .can_begin_checkpoint_candidate(mark)
    }

    pub fn checkpoint_mark(
        &self,
        boundary: SealedBoundary<PageMaterialLane>,
    ) -> Result<CheckpointMark<PageMaterialLane>, ForkArenaError> {
        self.state.region.pub_arena.checkpoint_mark(boundary)
    }
}

#[cfg(test)]
#[path = "page_node_arena/tests.rs"]
mod tests;
