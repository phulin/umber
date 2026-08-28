//! Exclusive node-closure regions above the shared fixed-chunk pool.
//!
//! Raw list coordinates remain compact implementation details. A
//! `NodeRegion` owns their chunk envelopes, `RegionRoot` records which region
//! admits a top-level coordinate, and `RegionList` binds resolution to an
//! actual borrow of that owner.

use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::fork_arena::{
    ArenaListView, BatchMark, ChunkPool, DetachedBatch, ForkArena, ForkArenaCounters,
    ForkArenaError, PageMaterialLane, RegionValue, SequenceSummaryWork,
};
use crate::node::Node;
use crate::node_sequence::{SemanticSequenceIdentity, semantic_node_identity};
use crate::page_node_arena::PageListId;

#[cfg(test)]
#[path = "node_region/tests.rs"]
mod tests;

type RegionNode = Node<PageListId>;

static NEXT_NODE_POOL_ID: AtomicU64 = AtomicU64::new(1);

/// Node ownership used by page construction and retained page history.
pub enum PageRole {}

/// Node ownership whose lifetime is independent of the current page.
pub enum DurableRole {}

/// Generation-checked identity of one recyclable node-region slot.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct NodeRegionId {
    pool: u64,
    slot: u32,
    generation: u32,
}

impl core::fmt::Debug for NodeRegionId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NodeRegionId(..)")
    }
}

#[derive(Clone, Copy)]
struct RegionSlot {
    generation: u32,
    arena: u32,
    live: bool,
}

/// The one physical page pool shared by all node regions.
///
/// Pool capacity is charged once here. Regions own only their chunk envelopes
/// and canonical descriptors.
pub struct NodePool {
    id: u64,
    pub(crate) chunks: ChunkPool<RegionNode>,
    regions: Vec<RegionSlot>,
    free_regions: Vec<u32>,
    closure_transitions: ClosureTransitionCounters,
}

/// Demand-free observations of explicit closure lifetime transitions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClosureTransitionCounters {
    pub envelope_moves: u64,
    pub rebrand_scan_nodes: u64,
    pub transient_rollbacks: u64,
    pub structural_fallbacks: u64,
    pub interleaved_prefix_fallbacks: u64,
    pub foreign_root_fallbacks: u64,
    pub retained_root_fallbacks: u64,
}

/// Why a caller deliberately selected the bounded recursive-copy seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructuralCopyReason {
    InterleavedPrefixChild,
    ForeignRoot,
    RetainedRoot,
}

impl Default for NodePool {
    fn default() -> Self {
        Self::new()
    }
}

impl NodePool {
    #[must_use]
    pub fn new() -> Self {
        Self::with_chunk_bytes(4 * 1024)
    }

    #[must_use]
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        Self {
            id: NEXT_NODE_POOL_ID.fetch_add(1, Ordering::Relaxed),
            chunks: ChunkPool::with_chunk_bytes(chunk_bytes),
            regions: Vec::new(),
            free_regions: Vec::new(),
            closure_transitions: ClosureTransitionCounters::default(),
        }
    }

    #[must_use]
    pub const fn closure_transition_counters(&self) -> ClosureTransitionCounters {
        self.closure_transitions
    }

    pub(crate) fn start_region<Role>(&mut self) -> Result<NodeRegion<Role>, ForkArenaError> {
        let arena = ForkArena::new();
        let arena_identity = arena.region_identity();
        let (slot, generation) = if let Some(slot) = self.free_regions.pop() {
            let entry = self
                .regions
                .get_mut(slot as usize)
                .ok_or(ForkArenaError::InvalidRegion)?;
            if entry.live {
                return Err(ForkArenaError::InvalidRegion);
            }
            entry.live = true;
            entry.arena = arena_identity;
            (slot, entry.generation)
        } else {
            let slot =
                u32::try_from(self.regions.len()).map_err(|_| ForkArenaError::CapacityOverflow)?;
            self.regions.push(RegionSlot {
                generation: 1,
                arena: arena_identity,
                live: true,
            });
            (slot, 1)
        };
        Ok(NodeRegion {
            id: NodeRegionId {
                pool: self.id,
                slot,
                generation,
            },
            pub_arena: arena,
            next_closure_build: 1,
            _role: PhantomData,
        })
    }

    fn validate_region<Role>(&self, region: &NodeRegion<Role>) -> Result<(), ForkArenaError> {
        if region.id.pool != self.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        let entry = self
            .regions
            .get(region.id.slot as usize)
            .ok_or(ForkArenaError::InvalidRegion)?;
        if !entry.live
            || entry.generation != region.id.generation
            || entry.arena != region.pub_arena.region_identity()
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn validates_id(&self, id: NodeRegionId) -> bool {
        id.pool == self.id
            && self
                .regions
                .get(id.slot as usize)
                .is_some_and(|entry| entry.live && entry.generation == id.generation)
    }

    /// Explicitly retires a region because its chunk keys must be returned to
    /// this separately borrowed pool.
    #[allow(clippy::result_large_err)] // Failure must return the exclusive move-only owner.
    pub(crate) fn retire_region<Role>(
        &mut self,
        mut region: NodeRegion<Role>,
    ) -> Result<(), (ForkArenaError, NodeRegion<Role>)> {
        if let Err(error) = self.validate_region(&region) {
            return Err((error, region));
        }
        if let Err(error) = region.pub_arena.can_retire_region(&self.chunks) {
            return Err((error, region));
        }
        let next_generation = match region.id.generation.checked_add(1) {
            Some(generation) => generation,
            None => return Err((ForkArenaError::CapacityOverflow, region)),
        };
        region
            .pub_arena
            .retire_region(&mut self.chunks)
            .expect("region retirement was completely preflighted");
        let entry = &mut self.regions[region.id.slot as usize];
        entry.live = false;
        entry.arena = 0;
        entry.generation = next_generation;
        self.free_regions.push(region.id.slot);
        Ok(())
    }
}

/// Exclusive, move-only owner of one self-contained node domain.
pub struct NodeRegion<Role> {
    id: NodeRegionId,
    pub(crate) pub_arena: ForkArena<RegionNode, PageMaterialLane>,
    next_closure_build: u64,
    _role: PhantomData<fn(Role) -> Role>,
}

impl<Role> NodeRegion<Role> {
    #[must_use]
    pub const fn id(&self) -> NodeRegionId {
        self.id
    }

    #[cfg(test)]
    pub(crate) fn publish_owned(
        &mut self,
        pool: &mut NodePool,
        nodes: impl IntoIterator<Item = RegionNode>,
    ) -> Result<RegionRoot<Role>, ForkArenaError> {
        pool.validate_region(self)?;
        let mut builder = self.pub_arena.begin_builder(&mut pool.chunks)?;
        for node in nodes {
            builder.push(node)?;
        }
        Ok(RegionRoot {
            region: self.id,
            list: PageListId::from_parts(builder.seal()?, None),
            _role: PhantomData,
        })
    }

    /// Seals the current payload and descriptor tails and opens one fresh
    /// whole-envelope construction suffix. Unlike an operation mark, this
    /// capability can only be consumed by closure sealing.
    pub(crate) fn begin_closure_build(
        &mut self,
        pool: &mut NodePool,
    ) -> Result<ClosureBuildMark<Role>, ForkArenaError> {
        pool.validate_region(self)?;
        let serial = self.next_closure_build;
        self.next_closure_build = serial
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        let batch = self.pub_arena.begin_batch(&mut pool.chunks)?;
        let rollback = self.pub_arena.operation_mark(&pool.chunks);
        Ok(ClosureBuildMark {
            region: self.id,
            serial,
            batch,
            rollback,
            _role: PhantomData,
        })
    }

    pub(crate) fn cancel_closure_build(
        &mut self,
        pool: &mut NodePool,
        mark: ClosureBuildMark<Role>,
    ) -> Result<(), ForkArenaError> {
        pool.validate_region(self)?;
        if mark.region != self.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.pub_arena
            .restore_operation(&mut pool.chunks, mark.rollback)
    }

    pub(crate) fn compatibility_closure_build_receipt(
        &self,
        mark: ClosureBuildMark<Role>,
    ) -> Result<CompatibilityClosureBuildReceipt<Role>, ForkArenaError> {
        if mark.region != self.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok(CompatibilityClosureBuildReceipt {
            region: mark.region,
            serial: mark.serial,
            _role: PhantomData,
        })
    }

    /// Converts the caller's owner-local root audit into the receipt consumed
    /// by closure sealing. Production callers may invoke this only after the
    /// PageBuilder, ModeList, operation journal, and checkpoint owner have
    /// removed every root created after `mark`.
    #[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
    pub(crate) fn consumed_closure_roots_receipt(
        &self,
        mark: &ClosureBuildMark<Role>,
    ) -> Result<ConsumedClosureRootsReceipt<Role>, ForkArenaError> {
        if mark.region != self.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok(ConsumedClosureRootsReceipt {
            region: self.id,
            serial: mark.serial,
            _role: PhantomData,
        })
    }

    /// Preflights and detaches one self-contained recursive closure suffix.
    /// Any failure leaves all chunk envelopes attached to this region.
    #[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
    pub(crate) fn seal_closure(
        &mut self,
        pool: &mut NodePool,
        mark: ClosureBuildMark<Role>,
        root: RegionRoot<Role>,
        receipt: ConsumedClosureRootsReceipt<Role>,
    ) -> Result<SealedNodeClosure<Role>, ClosureSealError<Role>> {
        if let Err(error) = pool.validate_region(self) {
            return Err(ClosureSealError { error, mark });
        }
        if mark.region != self.id
            || receipt.region != self.id
            || receipt.serial != mark.serial
            || root.region != self.id
        {
            return Err(ClosureSealError {
                error: ForkArenaError::InvalidRegion,
                mark,
            });
        }
        if let Err(error) = self.pub_arena.preflight_batch_closure(
            &pool.chunks,
            &mark.batch,
            &[root.list.coordinate()],
        ) {
            return Err(ClosureSealError { error, mark });
        }
        let batch = match self.pub_arena.seal_batch(
            &mut pool.chunks,
            mark.batch,
            vec![root.list.coordinate()],
        ) {
            Ok(batch) => batch,
            Err(error) => return Err(ClosureSealError { error, mark }),
        };
        let batch = match self.pub_arena.detach_batch(&mut pool.chunks, batch) {
            Ok(batch) => batch,
            Err(failure) => {
                self.pub_arena
                    .cancel_batch(failure.batch)
                    .expect("failed closure preflight returns its source authority");
                return Err(ClosureSealError {
                    error: failure.error,
                    mark,
                });
            }
        };
        Ok(SealedNodeClosure {
            source: self.id,
            root,
            batch,
        })
    }

    /// Returns a transient transfer loan to the exact construction suffix.
    #[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
    #[allow(clippy::result_large_err)] // Failure returns the move-only closure loan without allocation.
    pub(crate) fn rollback_closure(
        &mut self,
        pool: &mut NodePool,
        closure: SealedNodeClosure<Role>,
    ) -> Result<(), SealedNodeClosureError<Role>> {
        if closure.source != self.id || closure.root.region != self.id {
            return Err(SealedNodeClosureError {
                error: ForkArenaError::InvalidRegion,
                closure,
            });
        }
        match self.pub_arena.reattach_batch(&pool.chunks, closure.batch) {
            Ok(()) => {
                pool.closure_transitions.transient_rollbacks = pool
                    .closure_transitions
                    .transient_rollbacks
                    .saturating_add(1);
                Ok(())
            }
            Err(failure) => Err(SealedNodeClosureError {
                error: failure.error,
                closure: SealedNodeClosure {
                    source: closure.source,
                    root: closure.root,
                    batch: failure.batch,
                },
            }),
        }
    }

    pub(crate) fn root(
        &self,
        pool: &NodePool,
        list: PageListId,
    ) -> Result<RegionRoot<Role>, ForkArenaError> {
        pool.validate_region(self)?;
        self.pub_arena.list(&pool.chunks, list.coordinate())?;
        Ok(RegionRoot {
            region: self.id,
            list,
            _role: PhantomData,
        })
    }

    pub(crate) fn list<'region>(
        &'region self,
        pool: &'region NodePool,
        root: RegionRoot<Role>,
    ) -> Result<RegionList<'region, Role>, ForkArenaError> {
        pool.validate_region(self)?;
        if root.region != self.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok(RegionList {
            view: self.pub_arena.list(&pool.chunks, root.list.coordinate())?,
            _region: PhantomData,
        })
    }

    #[allow(clippy::result_large_err)] // Validation failure must return the exclusive region owner.
    pub(crate) fn into_closure(
        self,
        pool: &NodePool,
        root: RegionRoot<Role>,
    ) -> Result<OwnedNodeClosure<Role>, (ForkArenaError, Self)> {
        if let Err(error) = pool.validate_region(&self) {
            return Err((error, self));
        }
        if root.region != self.id
            || self
                .pub_arena
                .list(&pool.chunks, root.list.coordinate())
                .is_err()
        {
            return Err((ForkArenaError::InvalidRegion, self));
        }
        Ok(OwnedNodeClosure { region: self, root })
    }

    #[must_use]
    pub(crate) const fn counters(&self) -> ForkArenaCounters {
        self.pub_arena.counters()
    }
}

/// Sealed payload-plus-descriptor boundary taken before closure construction.
pub struct ClosureBuildMark<Role> {
    region: NodeRegionId,
    serial: u64,
    #[allow(dead_code)] // Read by closure sealing once the production carrier cutover lands.
    batch: BatchMark<PageMaterialLane>,
    rollback: crate::fork_arena::OperationMark<PageMaterialLane>,
    _role: PhantomData<fn(Role) -> Role>,
}

impl<Role> core::fmt::Debug for ClosureBuildMark<Role> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ClosureBuildMark")
            .field("region", &self.region)
            .field("serial", &self.serial)
            .finish_non_exhaustive()
    }
}

/// Page-owned closure boundary used by execution-facing construction APIs.
pub type PageClosureBuildMark = ClosureBuildMark<PageRole>;

/// Explicit receipt for a legacy owner which intentionally keeps a completed
/// closure in the page region instead of sealing/transferring it.
pub struct CompatibilityClosureBuildReceipt<Role> {
    region: NodeRegionId,
    serial: u64,
    _role: PhantomData<fn(Role) -> Role>,
}

impl<Role> CompatibilityClosureBuildReceipt<Role> {
    #[must_use]
    pub const fn region_id(&self) -> NodeRegionId {
        self.region
    }

    #[must_use]
    pub const fn build_serial(&self) -> u64 {
        self.serial
    }
}

/// Consumed proof that no owner-local root outside the closure names its
/// suffix. It is intentionally neither clonable nor constructible from raw
/// coordinates.
#[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
pub(crate) struct ConsumedClosureRootsReceipt<Role> {
    region: NodeRegionId,
    serial: u64,
    _role: PhantomData<fn(Role) -> Role>,
}

/// Move-only detached closure suffix. Payload addresses remain stable while
/// this loan is transferred or rolled back.
#[must_use = "a detached closure loan must be transferred or rolled back"]
#[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
pub(crate) struct SealedNodeClosure<Role> {
    source: NodeRegionId,
    root: RegionRoot<Role>,
    batch: DetachedBatch<PageMaterialLane>,
}

#[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
pub(crate) struct SealedNodeClosureError<Role> {
    pub(crate) error: ForkArenaError,
    pub(crate) closure: SealedNodeClosure<Role>,
}

#[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
impl<Role> SealedNodeClosureError<Role> {
    pub(crate) fn into_parts(self) -> (ForkArenaError, SealedNodeClosure<Role>) {
        (self.error, self.closure)
    }
}

/// Failed seal with the original move-only construction authority restored.
#[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
pub(crate) struct ClosureSealError<Role> {
    pub(crate) error: ForkArenaError,
    pub(crate) mark: ClosureBuildMark<Role>,
}

#[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
impl<Role> ClosureSealError<Role> {
    pub(crate) fn into_parts(self) -> (ForkArenaError, ClosureBuildMark<Role>) {
        (self.error, self.mark)
    }
}

impl<Role> core::fmt::Debug for ClosureSealError<Role> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ClosureSealError")
            .field("error", &self.error)
            .field("mark", &self.mark)
            .finish()
    }
}

/// Copy-only owner-relative top-level root.
pub struct RegionRoot<Role> {
    region: NodeRegionId,
    list: PageListId,
    _role: PhantomData<fn(Role) -> Role>,
}

impl<Role> Clone for RegionRoot<Role> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Role> Copy for RegionRoot<Role> {}

impl<Role> core::fmt::Debug for RegionRoot<Role> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RegionRoot")
            .field("len", &self.list.len())
            .finish_non_exhaustive()
    }
}

impl<Role> PartialEq for RegionRoot<Role> {
    fn eq(&self, other: &Self) -> bool {
        self.region == other.region && self.list == other.list
    }
}

impl<Role> Eq for RegionRoot<Role> {}

impl<Role> Hash for RegionRoot<Role> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.region.hash(state);
        self.list.hash(state);
    }
}

impl<Role> RegionRoot<Role> {
    pub(crate) const fn list(self) -> PageListId {
        self.list
    }

    #[must_use]
    pub const fn region_id(self) -> NodeRegionId {
        self.region
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.list.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.list.is_empty()
    }

    #[must_use]
    pub(crate) const fn page_list(self) -> PageListId {
        self.list
    }
}

/// Borrowed list capability tied to the matching `NodeRegion` borrow.
pub struct RegionList<'region, Role> {
    view: ArenaListView<'region, RegionNode, PageMaterialLane>,
    _region: PhantomData<&'region NodeRegion<Role>>,
}

impl<Role> RegionList<'_, Role> {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.view.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.view.is_empty()
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&RegionNode> {
        self.view.get(index)
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &RegionNode> + ExactSizeIterator {
        self.view.iter()
    }
}

/// Move-only owner-plus-root aggregate used by durable and semantic-copy
/// transitions.
pub struct OwnedNodeClosure<Role> {
    region: NodeRegion<Role>,
    root: RegionRoot<Role>,
}

impl<Role> OwnedNodeClosure<Role> {
    #[must_use]
    pub const fn region_id(&self) -> NodeRegionId {
        self.region.id
    }

    pub(crate) fn list<'region>(
        &'region self,
        pool: &'region NodePool,
    ) -> Result<RegionList<'region, Role>, ForkArenaError> {
        self.region.list(pool, self.root)
    }

    pub(crate) fn child_list<'region>(
        &'region self,
        pool: &'region NodePool,
        list: PageListId,
    ) -> Result<RegionList<'region, Role>, ForkArenaError> {
        let root = self.region.root(pool, list)?;
        self.region.list(pool, root)
    }

    pub(crate) fn into_region(self) -> NodeRegion<Role> {
        self.region
    }

    pub(crate) const fn root(&self) -> RegionRoot<Role> {
        self.root
    }
}

/// Failed move which returns the still-exclusive source owner unchanged.
pub(crate) struct ClosureTransferError<Role> {
    pub(crate) error: ForkArenaError,
    pub(crate) closure: OwnedNodeClosure<Role>,
}

/// Commits a detached construction suffix into a destination region. Failed
/// destination validation returns the move-only suffix loan unchanged.
#[allow(dead_code)] // Production carriers currently retain the compatibility receipt.
#[allow(clippy::result_large_err)] // Failure returns the move-only closure loan without allocation.
pub(crate) fn transfer_sealed_closure_into<Source, Destination>(
    pool: &mut NodePool,
    source: &mut NodeRegion<Source>,
    closure: SealedNodeClosure<Source>,
    destination: &mut NodeRegion<Destination>,
) -> Result<RegionRoot<Destination>, SealedNodeClosureError<Source>> {
    if pool.validate_region(source).is_err()
        || pool.validate_region(destination).is_err()
        || closure.source != source.id
        || closure.root.region != source.id
    {
        return Err(SealedNodeClosureError {
            error: ForkArenaError::InvalidRegion,
            closure,
        });
    }
    let original_root = closure.root;
    match source.pub_arena.promote_detached_batch_into(
        &mut pool.chunks,
        &mut destination.pub_arena,
        closure.batch,
    ) {
        Ok((coordinates, scanned)) => {
            let [coordinate]: [_; 1] = coordinates
                .try_into()
                .expect("one sealed closure root produces one transferred root");
            pool.closure_transitions.envelope_moves =
                pool.closure_transitions.envelope_moves.saturating_add(1);
            pool.closure_transitions.rebrand_scan_nodes = pool
                .closure_transitions
                .rebrand_scan_nodes
                .saturating_add(scanned);
            Ok(RegionRoot {
                region: destination.id,
                list: original_root.list.with_coordinate(coordinate),
                _role: PhantomData,
            })
        }
        Err(failure) => Err(SealedNodeClosureError {
            error: failure.error,
            closure: SealedNodeClosure {
                source: source.id,
                root: original_root,
                batch: failure.batch,
            },
        }),
    }
}

/// Moves a whole self-contained closure envelope and rebrands every nested
/// child coordinate without moving any node address.
#[allow(clippy::result_large_err)] // Transfer failure must return the exclusive closure owner.
pub(crate) fn transfer_closure_into<Source, Destination>(
    pool: &mut NodePool,
    mut closure: OwnedNodeClosure<Source>,
    destination: &mut NodeRegion<Destination>,
) -> Result<RegionRoot<Destination>, ClosureTransferError<Source>> {
    let preflight = pool
        .validate_region(&closure.region)
        .and_then(|()| pool.validate_region(destination))
        .and_then(|()| {
            if closure.root.region != closure.region.id {
                return Err(ForkArenaError::InvalidRegion);
            }
            closure.region.pub_arena.preflight_whole_region_transfer(
                &pool.chunks,
                &destination.pub_arena,
                &[closure.root.list.coordinate()],
            )
        });
    if let Err(error) = preflight {
        return Err(ClosureTransferError { error, closure });
    }

    let batch = closure
        .region
        .pub_arena
        .seal_whole_region_batch(&mut pool.chunks, vec![closure.root.list.coordinate()])
        .expect("whole-region transfer was preflighted");
    let promoted = closure
        .region
        .pub_arena
        .promote_batch_into(&mut pool.chunks, &mut destination.pub_arena, batch)
        .expect("whole-region promotion was preflighted");
    let [coordinate]: [_; 1] = promoted
        .try_into()
        .expect("one declared closure root produces one promoted root");
    let root = RegionRoot {
        region: destination.id,
        list: closure.root.list.with_coordinate(coordinate),
        _role: PhantomData,
    };
    match pool.retire_region(closure.region) {
        Ok(()) => Ok(root),
        Err((_error, _region)) => unreachable!("empty transferred region retires infallibly"),
    }
}

/// Recursively copies one exact node closure into an independently owned
/// destination region while keeping the source owner and addresses live.
pub(crate) fn copy_closure_into<Source, Destination>(
    pool: &mut NodePool,
    source: &OwnedNodeClosure<Source>,
    destination: &mut NodeRegion<Destination>,
    semantic_identity_enabled: bool,
) -> Result<RegionRoot<Destination>, ForkArenaError> {
    copy_region_root_into(
        pool,
        &source.region,
        source.root,
        destination,
        semantic_identity_enabled,
    )
}

/// Recursively copies one owner-relative root between live regions.
///
/// This is the cold transition seam used while a source carrier still owns a
/// larger coarse region than the selected closure. Unlike [`copy_closure_into`],
/// it does not pretend that the source root is independently movable.
pub(crate) fn copy_region_root_into<Source, Destination>(
    pool: &mut NodePool,
    source: &NodeRegion<Source>,
    root: RegionRoot<Source>,
    destination: &mut NodeRegion<Destination>,
    semantic_identity_enabled: bool,
) -> Result<RegionRoot<Destination>, ForkArenaError> {
    pool.validate_region(source)?;
    pool.validate_region(destination)?;
    if root.region != source.id {
        return Err(ForkArenaError::InvalidRegion);
    }
    let operation = destination.pub_arena.operation_mark(&pool.chunks);
    let copied = copy_list_recursive::<Source, Destination>(
        &mut pool.chunks,
        &source.pub_arena,
        &mut destination.pub_arena,
        root.list,
        &mut Vec::new(),
        semantic_identity_enabled,
    );
    let (list, count) = match copied {
        Ok(copied) => copied,
        Err(error) => {
            destination
                .pub_arena
                .restore_operation(&mut pool.chunks, operation)
                .expect("copy destination rollback mark remains valid");
            return Err(error);
        }
    };
    destination.pub_arena.record_source_nodes_copied(count);
    if semantic_identity_enabled {
        destination
            .pub_arena
            .record_identity_work(SequenceSummaryWork {
                hashed_values: count as u64,
                ..SequenceSummaryWork::default()
            });
    }
    Ok(RegionRoot {
        region: destination.id,
        list,
        _role: PhantomData,
    })
}

/// Explicit bounded structural-copy fallback. The reason is observed but
/// never used as liveness authority or to select another representation.
#[allow(dead_code)] // The current production carrier path has no structural fallback call.
pub(crate) fn structural_copy_fallback<Source, Destination>(
    pool: &mut NodePool,
    source: &NodeRegion<Source>,
    root: RegionRoot<Source>,
    destination: &mut NodeRegion<Destination>,
    reason: StructuralCopyReason,
) -> Result<RegionRoot<Destination>, ForkArenaError> {
    let copied = copy_region_root_into(
        pool,
        source,
        root,
        destination,
        root.list.semantic_identity().is_some(),
    )?;
    pool.closure_transitions.structural_fallbacks = pool
        .closure_transitions
        .structural_fallbacks
        .saturating_add(1);
    let reason_counter = match reason {
        StructuralCopyReason::InterleavedPrefixChild => {
            &mut pool.closure_transitions.interleaved_prefix_fallbacks
        }
        StructuralCopyReason::ForeignRoot => &mut pool.closure_transitions.foreign_root_fallbacks,
        StructuralCopyReason::RetainedRoot => &mut pool.closure_transitions.retained_root_fallbacks,
    };
    *reason_counter = reason_counter.saturating_add(1);
    Ok(copied)
}

fn copy_list_recursive<Source, Destination>(
    pool: &mut ChunkPool<RegionNode>,
    source: &ForkArena<RegionNode, PageMaterialLane>,
    destination: &mut ForkArena<RegionNode, PageMaterialLane>,
    list: PageListId,
    stack: &mut Vec<PageListId>,
    semantic_identity_enabled: bool,
) -> Result<(PageListId, usize), ForkArenaError> {
    let _roles = PhantomData::<fn(Source) -> Destination>;
    if list.is_empty() {
        return Ok((PageListId::empty(), 0));
    }
    if stack.contains(&list) {
        return Err(ForkArenaError::InvalidRegion);
    }
    let nodes = source
        .list(pool, list.coordinate())?
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    stack.push(list);
    let mut copied_nodes = Vec::with_capacity(nodes.len());
    let mut copied_count = nodes.len();
    for mut node in nodes {
        let mut child_error = None;
        node.visit_node_lists_mut(|child| {
            if child_error.is_some() {
                return;
            }
            match copy_list_recursive::<Source, Destination>(
                pool,
                source,
                destination,
                *child,
                stack,
                semantic_identity_enabled,
            ) {
                Ok((copied, count)) => {
                    *child = copied;
                    copied_count = copied_count.saturating_add(count);
                }
                Err(error) => child_error = Some(error),
            }
        });
        if let Some(error) = child_error {
            stack.pop();
            return Err(error);
        }
        copied_nodes.push(node);
    }
    stack.pop();
    let mut builder = destination.begin_builder(pool)?;
    let mut identity = semantic_identity_enabled.then(SemanticSequenceIdentity::empty);
    for node in copied_nodes {
        if let Some(sequence_identity) = &mut identity {
            let node_identity = semantic_node_identity(&node);
            sequence_identity.push_back(node_identity);
            builder.push_summarized(node, node_identity)?;
        } else {
            builder.push(node)?;
        }
    }
    let coordinate = builder.seal()?;
    Ok((PageListId::from_parts(coordinate, identity), copied_count))
}

impl RegionValue<PageMaterialLane> for RegionNode {
    fn visit_region_lists(
        &self,
        visit: &mut dyn FnMut(crate::fork_arena::ArenaListId<PageMaterialLane>),
    ) {
        self.visit_node_lists(|list| visit(list.coordinate()));
    }

    fn rebrand_region_lists(&mut self, destination_arena: u32) {
        self.visit_node_lists_mut(|list| *list = list.rebrand_arena(destination_arena));
    }
}
