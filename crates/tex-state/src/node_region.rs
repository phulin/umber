//! Exclusive node-closure regions above the shared fixed-chunk pool.
//!
//! Raw list coordinates remain compact implementation details. A
//! `NodeRegion` owns their chunk envelopes, `RegionRoot` records which region
//! admits a top-level coordinate, and `NodeCursor` binds resolution to an
//! actual borrow of that owner without reconstructing resident nodes.

use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::fork_arena::{
    ActiveListBuilder, AdmittedListChunkCursor, BatchMark, CheckpointMark, ChunkPool,
    DetachedBatch, ForkArena, ForkArenaCounters, ForkArenaError, NodePoolStorageClass,
    PageMaterialLane, RegionValue, SealedBoundary, SequenceSummaryWork,
};

#[cfg(feature = "profiling")]
use crate::fork_arena::ChunkStorageLayoutCensus;
use crate::node::Node;
#[cfg(test)]
use crate::node_record::NodeAnnexWriter;
use crate::node_record::{NodeAnnexView, NodeRecord};
use crate::node_sequence::SemanticSequenceIdentity;
use crate::page_node_arena::PageListId;

#[cfg(test)]
#[path = "node_region/tests.rs"]
mod tests;

pub(crate) type RegionNode = NodeRecord<PageMaterialLane>;

pub(crate) enum NodeAnnexLane {}

pub struct NodeSealedBoundary {
    nodes: SealedBoundary<PageMaterialLane>,
    annex: SealedBoundary<NodeAnnexLane>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeCheckpointMark {
    nodes: CheckpointMark<PageMaterialLane>,
    annex: CheckpointMark<NodeAnnexLane>,
}

struct NodeEnvelopeBatch {
    nodes: DetachedBatch<PageMaterialLane>,
    annex: DetachedBatch<NodeAnnexLane>,
}

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
    annex_arena: u32,
    live: bool,
}

/// The one pool-stable logical node space shared by all node regions.
///
/// During the owned-enum transition, `ChunkPool` is a private resolver from
/// logical positions to the existing stable chunk allocation. No page root,
/// child, predecessor, checkpoint, format, or output value can observe its
/// physical key. The atomic compact-record cutover replaces that adapter with
/// one `BlockStore<NodeRecord>` and `AcceptedBlockTable<NodeRecord>`; it does
/// not add another resident node representation.
pub struct NodePool {
    id: u64,
    pub(crate) chunks: ChunkPool<RegionNode>,
    pub(crate) annex_chunks: ChunkPool<u32>,
    regions: Vec<RegionSlot>,
    free_regions: Vec<u32>,
    closure_transitions: ClosureTransitionCounters,
}

#[cfg(feature = "profiling")]
pub(crate) struct NodeRegionPhysicalOwnership {
    pub(crate) current_nodes: Vec<u64>,
    pub(crate) prior_nodes: Vec<u64>,
    pub(crate) current_annexes: Vec<u64>,
    pub(crate) prior_annexes: Vec<u64>,
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
        // Sixteen-record logical chunks pack into the exact 64-KiB physical
        // superblocks. Small TeX lists therefore share backing without moving
        // stable coordinates, while an interior fork copies at most 15 records
        // and physical allocation remains one block per 2,048 packed records.
        Self::with_chunk_bytes(512)
    }

    #[must_use]
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        Self {
            id: NEXT_NODE_POOL_ID.fetch_add(1, Ordering::Relaxed),
            chunks: ChunkPool::with_node_pool_chunk_bytes(chunk_bytes, NodePoolStorageClass::Node),
            annex_chunks: ChunkPool::with_node_pool_packed_chunk_bytes(
                65_536,
                NodePoolStorageClass::Annex,
            ),
            regions: Vec::new(),
            free_regions: Vec::new(),
            closure_transitions: ClosureTransitionCounters::default(),
        }
    }

    #[must_use]
    pub const fn closure_transition_counters(&self) -> ClosureTransitionCounters {
        self.closure_transitions
    }

    /// Heap capacity owned by the one shared node/annex pool.
    ///
    /// The page-history retention owner charges this aggregate once. Individual
    /// page checkpoints and durable closures must not charge the same backing
    /// again merely because their disjoint envelopes resolve through it.
    pub(crate) fn retained_owner_bytes(&self) -> usize {
        self.chunks
            .live_owner_heap_bytes()
            .saturating_add(self.annex_chunks.live_owner_heap_bytes())
            .saturating_add(
                self.regions
                    .capacity()
                    .saturating_mul(core::mem::size_of::<RegionSlot>()),
            )
            .saturating_add(
                self.free_regions
                    .capacity()
                    .saturating_mul(core::mem::size_of::<u32>()),
            )
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn profiling_live_physical_tokens(&self) -> (Vec<u64>, Vec<u64>) {
        (
            self.chunks.profiling_live_physical_tokens(),
            self.annex_chunks.profiling_live_physical_tokens(),
        )
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn profiling_storage_layout(
        &self,
    ) -> (ChunkStorageLayoutCensus, ChunkStorageLayoutCensus) {
        (
            self.chunks.profiling_layout_census(),
            self.annex_chunks.profiling_layout_census(),
        )
    }

    pub(crate) fn start_region<Role>(&mut self) -> Result<NodeRegion<Role>, ForkArenaError> {
        let arena = ForkArena::new();
        let annex_arena = ForkArena::new();
        self.install_region(arena, annex_arena)
    }

    fn install_region<Role>(
        &mut self,
        arena: ForkArena<RegionNode, PageMaterialLane>,
        annex_arena: ForkArena<u32, NodeAnnexLane>,
    ) -> Result<NodeRegion<Role>, ForkArenaError> {
        let arena_identity = arena.region_identity();
        let annex_arena_identity = annex_arena.region_identity();
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
            entry.annex_arena = annex_arena_identity;
            (slot, entry.generation)
        } else {
            let slot =
                u32::try_from(self.regions.len()).map_err(|_| ForkArenaError::CapacityOverflow)?;
            self.regions.push(RegionSlot {
                generation: 1,
                arena: arena_identity,
                annex_arena: annex_arena_identity,
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
            annex_arena,
            active_annex_operation: None,
            next_closure_build: 1,
            _role: PhantomData,
        })
    }

    fn share_region<Role, const N: usize>(
        &mut self,
        source: &mut NodeRegion<Role>,
        mark: ClosureBuildMark<Role>,
        roots: [PageListId; N],
    ) -> Result<NodeRegion<Role>, ForkArenaError> {
        self.validate_region(source)?;
        if mark.region != source.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        let coordinates = roots.map(PageListId::coordinate);
        source
            .pub_arena
            .can_share_sealed_prefix(&self.chunks, &mark.batch, &coordinates)?;
        source
            .annex_arena
            .can_share_sealed_prefix(&self.annex_chunks, &mark.annex_batch, &[])?;
        source.pub_arena.preflight_paired_dependency_floor(
            &self.chunks,
            &coordinates,
            mark.annex_batch.payload_start(),
        )?;
        let arena = source
            .pub_arena
            .share_sealed_prefix(&mut self.chunks, mark.batch, &coordinates)
            .expect("paired node prefix sharing was preflighted");
        let annex_arena = source
            .annex_arena
            .share_sealed_prefix(&mut self.annex_chunks, mark.annex_batch, &[])
            .expect("paired annex prefix sharing was preflighted");
        self.install_region(arena, annex_arena)
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
            || entry.annex_arena != region.annex_arena.region_identity()
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok(())
    }

    fn can_advance_region<Role>(&self, region: &NodeRegion<Role>) -> Result<u32, ForkArenaError> {
        self.validate_region(region)?;
        region
            .id
            .generation
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)
    }

    /// Gives a consumed semantic predecessor a fresh region generation while
    /// retaining its physical arena and chunk addresses.
    fn advance_region<Role>(
        &mut self,
        region: &mut NodeRegion<Role>,
    ) -> Result<(), ForkArenaError> {
        let generation = self.can_advance_region(region)?;
        let entry = self
            .regions
            .get_mut(region.id.slot as usize)
            .ok_or(ForkArenaError::InvalidRegion)?;
        entry.generation = generation;
        region.id.generation = generation;
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
        if let Err(error) = self.retire_region_in_place(&mut region) {
            return Err((error, region));
        }
        Ok(())
    }

    /// Retires one authoritative region slot without transporting its complete
    /// envelope through a temporary return value. The now-empty value may be
    /// dropped in place by a higher-level owner store.
    pub(crate) fn retire_region_in_place<Role>(
        &mut self,
        region: &mut NodeRegion<Role>,
    ) -> Result<(), ForkArenaError> {
        self.validate_region(region)?;
        region.pub_arena.can_retire_region(&self.chunks)?;
        region.annex_arena.can_retire_region(&self.annex_chunks)?;
        let next_generation = match region.id.generation.checked_add(1) {
            Some(generation) => generation,
            None => return Err(ForkArenaError::CapacityOverflow),
        };
        region
            .pub_arena
            .retire_region(&mut self.chunks)
            .expect("region retirement was completely preflighted");
        region
            .annex_arena
            .retire_region(&mut self.annex_chunks)
            .expect("annex retirement was completely preflighted");
        let entry = &mut self.regions[region.id.slot as usize];
        entry.live = false;
        entry.arena = 0;
        entry.annex_arena = 0;
        entry.generation = next_generation;
        self.free_regions.push(region.id.slot);
        Ok(())
    }

    pub(crate) fn retire_closure_in_place<Role>(
        &mut self,
        closure: &mut OwnedNodeClosure<Role>,
    ) -> Result<(), ForkArenaError> {
        self.retire_region_in_place(&mut closure.region)
    }
}

/// Exclusive, move-only owner of one self-contained node domain.
pub struct NodeRegion<Role> {
    id: NodeRegionId,
    pub(crate) pub_arena: ForkArena<RegionNode, PageMaterialLane>,
    pub(crate) annex_arena: ForkArena<u32, NodeAnnexLane>,
    pub(crate) active_annex_operation: Option<crate::fork_arena::OperationMark<NodeAnnexLane>>,
    next_closure_build: u64,
    _role: PhantomData<fn(Role) -> Role>,
}

impl<Role> NodeRegion<Role> {
    #[must_use]
    pub const fn id(&self) -> NodeRegionId {
        self.id
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn profiling_physical_ownership(
        &self,
        pool: &NodePool,
    ) -> NodeRegionPhysicalOwnership {
        NodeRegionPhysicalOwnership {
            current_nodes: self
                .pub_arena
                .profiling_current_physical_tokens(&pool.chunks),
            prior_nodes: self.pub_arena.profiling_prior_physical_tokens(&pool.chunks),
            current_annexes: self
                .annex_arena
                .profiling_current_physical_tokens(&pool.annex_chunks),
            prior_annexes: self
                .annex_arena
                .profiling_prior_physical_tokens(&pool.annex_chunks),
        }
    }

    pub(crate) fn seal_checkpoint_boundary(
        &mut self,
        pool: &mut NodePool,
    ) -> Result<NodeSealedBoundary, ForkArenaError> {
        pool.validate_region(self)?;
        self.pub_arena.can_seal_boundary(&pool.chunks)?;
        self.annex_arena.can_seal_boundary(&pool.annex_chunks)?;
        let nodes = self
            .pub_arena
            .seal_boundary(&mut pool.chunks)
            .expect("paired node boundary was preflighted");
        let annex = self
            .annex_arena
            .seal_boundary(&mut pool.annex_chunks)
            .expect("paired annex boundary was preflighted");
        Ok(NodeSealedBoundary { nodes, annex })
    }

    pub(crate) fn checkpoint_mark(
        &self,
        boundary: NodeSealedBoundary,
    ) -> Result<NodeCheckpointMark, ForkArenaError> {
        let nodes = self.pub_arena.checkpoint_mark(boundary.nodes)?;
        let annex = self
            .annex_arena
            .checkpoint_mark(boundary.annex)
            .expect("paired annex boundary was sealed");
        Ok(NodeCheckpointMark { nodes, annex })
    }

    pub(crate) fn release_rootless_suffix(
        &mut self,
        pool: &mut NodePool,
        retained: Option<NodeCheckpointMark>,
    ) -> Result<usize, ForkArenaError> {
        let boundary = self.seal_checkpoint_boundary(pool)?;
        let nodes = self.pub_arena.release_rootless_current_suffix(
            &mut pool.chunks,
            boundary.nodes,
            retained.map(|mark| mark.nodes),
        )?;
        let annex = self
            .annex_arena
            .release_rootless_current_suffix(
                &mut pool.annex_chunks,
                boundary.annex,
                retained.map(|mark| mark.annex),
            )
            .expect("paired annex rootless suffix was preflighted");
        Ok(nodes.saturating_add(annex))
    }

    pub(crate) fn validates_checkpoint(&self, mark: NodeCheckpointMark) -> bool {
        self.pub_arena.validates_checkpoint(mark.nodes)
            && self.annex_arena.validates_checkpoint(mark.annex)
    }

    pub(crate) fn can_begin_checkpoint_candidate(&self, mark: NodeCheckpointMark) -> bool {
        self.pub_arena.can_begin_checkpoint_candidate(mark.nodes)
            && self.annex_arena.can_begin_checkpoint_candidate(mark.annex)
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        pool: &mut NodePool,
        mark: NodeCheckpointMark,
    ) -> Result<(), ForkArenaError> {
        if !self.can_begin_checkpoint_candidate(mark) {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        self.pub_arena
            .begin_checkpoint_candidate(&mut pool.chunks, mark.nodes)
            .expect("paired node checkpoint was preflighted");
        self.annex_arena
            .begin_checkpoint_candidate(&mut pool.annex_chunks, mark.annex)
            .expect("paired annex checkpoint was preflighted");
        Ok(())
    }

    pub(crate) fn restore_checkpoint(
        &mut self,
        pool: &mut NodePool,
        mark: NodeCheckpointMark,
    ) -> Result<(), ForkArenaError> {
        self.begin_checkpoint_candidate(pool, mark)?;
        let boundary = self.seal_checkpoint_boundary(pool)?;
        self.accept_checkpoint_candidate(pool, boundary)
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        pool: &mut NodePool,
        boundary: NodeSealedBoundary,
    ) -> Result<(), ForkArenaError> {
        self.pub_arena
            .can_settle_checkpoint_candidate(&boundary.nodes)?;
        self.annex_arena
            .can_settle_checkpoint_candidate(&boundary.annex)?;
        self.pub_arena
            .reject_checkpoint_candidate(&mut pool.chunks, boundary.nodes)
            .expect("paired node rejection was preflighted");
        self.annex_arena
            .reject_checkpoint_candidate(&mut pool.annex_chunks, boundary.annex)
            .expect("paired annex rejection was preflighted");
        Ok(())
    }

    pub(crate) fn accept_checkpoint_candidate(
        &mut self,
        pool: &mut NodePool,
        boundary: NodeSealedBoundary,
    ) -> Result<(), ForkArenaError> {
        self.pub_arena
            .can_settle_checkpoint_candidate(&boundary.nodes)?;
        self.annex_arena
            .can_settle_checkpoint_candidate(&boundary.annex)?;
        self.pub_arena
            .accept_checkpoint_candidate(&mut pool.chunks, boundary.nodes)
            .expect("paired node acceptance was preflighted");
        self.annex_arena
            .accept_checkpoint_candidate(&mut pool.annex_chunks, boundary.annex)
            .expect("paired annex acceptance was preflighted");
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn publish_owned(
        &mut self,
        pool: &mut NodePool,
        nodes: impl IntoIterator<Item = Node<PageListId>>,
    ) -> Result<RegionRoot<Role>, ForkArenaError> {
        pool.validate_region(self)?;
        let mut builder = self.pub_arena.begin_builder(&mut pool.chunks)?;
        for node in nodes {
            let child_annex_dependency_floor = builder.paired_dependency_floor_for(&node)?;
            let (record, annex_dependency_floor) = {
                let mut annex = NodeAnnexWriter::new(&mut pool.annex_chunks, &mut self.annex_arena);
                let record = NodeRecord::encode_owned(node.clone(), &mut annex);
                (record, annex.dependency_floor())
            };
            builder.push_with_dependencies(record, &node)?;
            builder.record_paired_dependency(
                [annex_dependency_floor, child_annex_dependency_floor]
                    .into_iter()
                    .flatten()
                    .min(),
            )?;
        }
        Ok(RegionRoot {
            region: self.id,
            list: PageListId::from_parts(builder.finish(), None),
            _role: PhantomData,
        })
    }

    /// Seals the current payload tail and opens one fresh
    /// whole-envelope construction suffix. Unlike an operation mark, this
    /// capability can only be consumed by closure sealing.
    pub(crate) fn begin_closure_build(
        &mut self,
        pool: &mut NodePool,
    ) -> Result<ClosureBuildMark<Role>, ForkArenaError> {
        pool.validate_region(self)?;
        self.pub_arena.can_seal_boundary(&pool.chunks)?;
        self.annex_arena.can_seal_boundary(&pool.annex_chunks)?;
        let serial = self.next_closure_build;
        self.next_closure_build = serial
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        let batch = self
            .pub_arena
            .begin_batch(&mut pool.chunks)
            .expect("paired node batch was preflighted");
        let annex_batch = self
            .annex_arena
            .begin_batch(&mut pool.annex_chunks)
            .expect("paired annex batch was preflighted");
        let rollback = self.pub_arena.operation_mark(&pool.chunks);
        let annex_rollback = self.annex_arena.operation_mark(&pool.annex_chunks);
        Ok(ClosureBuildMark {
            region: self.id,
            serial,
            batch,
            annex_batch,
            rollback,
            annex_rollback,
            _role: PhantomData,
        })
    }

    pub(crate) fn can_share_sealed_prefix<const N: usize>(
        &self,
        pool: &NodePool,
        mark: &ClosureBuildMark<Role>,
        roots: [PageListId; N],
    ) -> Result<(), ForkArenaError> {
        pool.validate_region(self)?;
        if mark.region != self.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        let coordinates = roots.map(PageListId::coordinate);
        self.pub_arena
            .can_share_sealed_prefix(&pool.chunks, &mark.batch, &coordinates)?;
        self.annex_arena
            .can_share_sealed_prefix(&pool.annex_chunks, &mark.annex_batch, &[])?;
        self.pub_arena.preflight_paired_dependency_floor(
            &pool.chunks,
            &coordinates,
            mark.annex_batch.payload_start(),
        )
    }

    pub(crate) fn share_sealed_prefix<const N: usize>(
        &mut self,
        pool: &mut NodePool,
        mark: ClosureBuildMark<Role>,
        roots: [PageListId; N],
    ) -> Result<NodeRegion<Role>, ForkArenaError> {
        pool.share_region(self, mark, roots)
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
            .restore_operation(&mut pool.chunks, mark.rollback)?;
        self.annex_arena
            .restore_operation(&mut pool.annex_chunks, mark.annex_rollback)
    }

    pub(crate) fn build_suffix_contains_any_root<const N: usize>(
        &self,
        pool: &NodePool,
        mark: &ClosureBuildMark<Role>,
        roots: [PageListId; N],
    ) -> Result<bool, ForkArenaError> {
        pool.validate_region(self)?;
        if mark.region != self.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        for root in roots {
            if self.pub_arena.list_is_in_batch_suffix(
                &pool.chunks,
                &mark.batch,
                root.coordinate(),
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn preflight_unique_successor_adoption<const N: usize>(
        &self,
        pool: &NodePool,
        mark: &ClosureBuildMark<Role>,
        roots: [PageListId; N],
    ) -> Result<(), ForkArenaError> {
        pool.can_advance_region(self)?;
        if mark.region != self.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        let coordinates = roots.map(PageListId::coordinate);
        self.pub_arena.preflight_unique_successor_adoption(
            &pool.chunks,
            &mark.batch,
            &coordinates,
        )?;
        self.pub_arena.preflight_paired_dependency_floor(
            &pool.chunks,
            &coordinates,
            mark.annex_batch.payload_start(),
        )?;
        self.annex_arena.preflight_unique_successor_adoption(
            &pool.annex_chunks,
            &mark.annex_batch,
            &[],
        )
    }

    pub(crate) fn adopt_unique_successor<const N: usize>(
        &mut self,
        pool: &mut NodePool,
        mark: ClosureBuildMark<Role>,
        roots: [PageListId; N],
    ) -> Result<(), ForkArenaError> {
        self.preflight_unique_successor_adoption(pool, &mark, roots)?;
        let coordinates = roots.map(PageListId::coordinate);
        self.pub_arena
            .adopt_unique_successor_suffix(&mut pool.chunks, mark.batch, &coordinates)?;
        self.annex_arena
            .adopt_unique_successor_suffix(&mut pool.annex_chunks, mark.annex_batch, &[])
            .expect("paired successor annex adoption was preflighted");
        pool.advance_region(self)
            .expect("unique-successor region generation was preflighted");
        Ok(())
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
    #[allow(clippy::result_large_err)] // Failure returns the sole move-only build authority.
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
        if let Err(error) =
            self.annex_arena
                .preflight_batch_closure(&pool.annex_chunks, &mark.annex_batch, &[])
        {
            return Err(ClosureSealError { error, mark });
        }
        if let Err(error) = self.pub_arena.preflight_paired_dependency_floor(
            &pool.chunks,
            &[root.list.coordinate()],
            mark.annex_batch.payload_start(),
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
        let annex_batch = self
            .annex_arena
            .seal_batch(&mut pool.annex_chunks, mark.annex_batch, Vec::new())
            .expect("paired empty annex suffix sealing is infallible");
        let annex_batch = self
            .annex_arena
            .detach_batch(&mut pool.annex_chunks, annex_batch)
            .unwrap_or_else(|_| unreachable!("paired annex suffix was just sealed"));
        Ok(SealedNodeClosure {
            source: self.id,
            root,
            batch: NodeEnvelopeBatch {
                nodes: batch,
                annex: annex_batch,
            },
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
        if let Err(error) = self
            .pub_arena
            .can_reattach_batch(&pool.chunks, &closure.batch.nodes)
        {
            return Err(SealedNodeClosureError { error, closure });
        }
        if let Err(error) = self
            .annex_arena
            .can_reattach_batch(&pool.annex_chunks, &closure.batch.annex)
        {
            return Err(SealedNodeClosureError { error, closure });
        }
        self.pub_arena
            .reattach_batch(&mut pool.chunks, closure.batch.nodes)
            .unwrap_or_else(|_| unreachable!("paired node rollback was preflighted"));
        self.annex_arena
            .reattach_batch(&mut pool.annex_chunks, closure.batch.annex)
            .unwrap_or_else(|_| unreachable!("paired annex rollback was preflighted"));
        pool.closure_transitions.transient_rollbacks = pool
            .closure_transitions
            .transient_rollbacks
            .saturating_add(1);
        Ok(())
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
    ) -> Result<crate::node_arena::NodeCursor<'region>, ForkArenaError> {
        pool.validate_region(self)?;
        if root.region != self.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        let view = self.pub_arena.list(&pool.chunks, root.list.coordinate())?;
        let annex = NodeAnnexView::new(&pool.annex_chunks, &self.annex_arena);
        Ok(crate::node_arena::NodeCursor::fork_arena(view, annex))
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

/// Paired node-plus-annex boundary taken before closure construction.
/// It is consumed either by exact suffix transfer or by the retained-root
/// structural-copy path that deliberately keeps the suffix page-owned.
pub struct ClosureBuildMark<Role> {
    region: NodeRegionId,
    serial: u64,
    #[allow(dead_code)] // Read by closure sealing once the production carrier cutover lands.
    batch: BatchMark<PageMaterialLane>,
    annex_batch: BatchMark<NodeAnnexLane>,
    rollback: crate::fork_arena::OperationMark<PageMaterialLane>,
    annex_rollback: crate::fork_arena::OperationMark<NodeAnnexLane>,
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
    batch: NodeEnvelopeBatch,
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
    ) -> Result<crate::node_arena::NodeCursor<'region>, ForkArenaError> {
        self.region.list(pool, self.root)
    }

    pub(crate) fn child_list<'region>(
        &'region self,
        pool: &'region NodePool,
        list: PageListId,
    ) -> Result<crate::node_arena::NodeCursor<'region>, ForkArenaError> {
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
    if let Err(error) = source.pub_arena.can_promote_detached_batch_into(
        &pool.chunks,
        &destination.pub_arena,
        &closure.batch.nodes,
    ) {
        return Err(SealedNodeClosureError { error, closure });
    }
    if let Err(error) = source.annex_arena.can_promote_detached_batch_into(
        &pool.annex_chunks,
        &destination.annex_arena,
        &closure.batch.annex,
    ) {
        return Err(SealedNodeClosureError { error, closure });
    }
    let source_annex_start = closure.batch.annex.payload_start();
    let destination_node_start = destination.pub_arena.live_payload_chunks();
    let destination_annex_start = destination.annex_arena.live_payload_chunks();
    let (coordinates, scanned) = source
        .pub_arena
        .promote_detached_batch_into(
            &mut pool.chunks,
            &mut destination.pub_arena,
            closure.batch.nodes,
        )
        .unwrap_or_else(|_| unreachable!("paired node transfer was preflighted"));
    let (annex_roots, annex_scanned) = source
        .annex_arena
        .promote_detached_batch_into(
            &mut pool.annex_chunks,
            &mut destination.annex_arena,
            closure.batch.annex,
        )
        .unwrap_or_else(|_| unreachable!("paired annex transfer was preflighted"));
    debug_assert!(annex_roots.is_empty());
    debug_assert_eq!(annex_scanned, 0);
    destination
        .pub_arena
        .rebase_paired_dependency_suffix(
            &mut pool.chunks,
            destination_node_start,
            source_annex_start,
            destination_annex_start,
        )
        .expect("paired detached transfer preserves relative annex floors");
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

/// Moves a whole self-contained closure envelope and rebrands every nested
/// child coordinate without moving any node address.
pub(crate) fn transfer_closure_into<Source, Destination>(
    pool: &mut NodePool,
    closure: &mut OwnedNodeClosure<Source>,
    destination: &mut NodeRegion<Destination>,
) -> Result<RegionRoot<Destination>, ForkArenaError> {
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
                Some(closure.root.list.coordinate()),
            )?;
            closure.region.annex_arena.preflight_whole_region_transfer(
                &pool.annex_chunks,
                &destination.annex_arena,
                None,
            )
        });
    preflight?;

    let batch = closure
        .region
        .pub_arena
        .seal_whole_region_batch(&mut pool.chunks, Some(closure.root.list.coordinate()))
        .expect("whole-region transfer was preflighted");
    let annex_batch = closure
        .region
        .annex_arena
        .seal_whole_region_batch(&mut pool.annex_chunks, None)
        .expect("whole-region annex transfer was preflighted");
    let destination_node_start = destination.pub_arena.live_payload_chunks();
    let destination_annex_start = destination.annex_arena.live_payload_chunks();
    let promoted = closure
        .region
        .pub_arena
        .promote_whole_region_into(&mut pool.chunks, &mut destination.pub_arena, batch)
        .expect("whole-region promotion was preflighted");
    let annex_promoted = closure
        .region
        .annex_arena
        .promote_whole_region_into(
            &mut pool.annex_chunks,
            &mut destination.annex_arena,
            annex_batch,
        )
        .expect("whole-region annex promotion was preflighted");
    debug_assert!(annex_promoted.is_none());
    destination
        .pub_arena
        .rebase_paired_dependency_suffix(
            &mut pool.chunks,
            destination_node_start,
            0,
            destination_annex_start,
        )
        .expect("paired whole-region transfer preserves relative annex floors");
    let coordinate = promoted.expect("one declared closure root produces one promoted root");
    let root = RegionRoot {
        region: destination.id,
        list: closure.root.list.with_coordinate(coordinate),
        _role: PhantomData,
    };
    pool.retire_region_in_place(&mut closure.region)
        .unwrap_or_else(|_| unreachable!("empty transferred region retires infallibly"));
    Ok(root)
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
    let annex_operation = destination.annex_arena.operation_mark(&pool.annex_chunks);
    let copied = copy_list_recursive::<Source, Destination>(
        &mut pool.chunks,
        &mut pool.annex_chunks,
        &source.pub_arena,
        &source.annex_arena,
        &mut destination.pub_arena,
        &mut destination.annex_arena,
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
            destination
                .annex_arena
                .restore_operation(&mut pool.annex_chunks, annex_operation)
                .expect("copy annex rollback mark remains valid");
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

#[allow(clippy::too_many_arguments)] // Keeps both paired stores and owners explicit during recursive copy.
fn copy_list_recursive<Source, Destination>(
    pool: &mut ChunkPool<RegionNode>,
    annex_pool: &mut ChunkPool<u32>,
    source: &ForkArena<RegionNode, PageMaterialLane>,
    source_annex: &ForkArena<u32, NodeAnnexLane>,
    destination: &mut ForkArena<RegionNode, PageMaterialLane>,
    destination_annex: &mut ForkArena<u32, NodeAnnexLane>,
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
    let admitted = source.admit_owned_root(pool, list.coordinate())?;
    let mut source_children = Vec::new();
    if let Some(tail) = source.admitted_tail_chunk_from_root(pool, list.coordinate(), admitted)? {
        collect_copy_children(
            pool,
            annex_pool,
            source,
            source_annex,
            tail,
            &mut source_children,
        )?;
    }
    stack.push(list);
    let mut copied_count = list.len();
    for child in &mut source_children {
        match copy_list_recursive::<Source, Destination>(
            pool,
            annex_pool,
            source,
            source_annex,
            destination,
            destination_annex,
            *child,
            stack,
            semantic_identity_enabled,
        ) {
            Ok((copied, count)) => {
                *child = copied;
                copied_count = copied_count.saturating_add(count);
            }
            Err(error) => {
                stack.pop();
                return Err(error);
            }
        }
    }
    stack.pop();

    let mut copied_children = source_children.into_iter();
    let mut builder = ActiveListBuilder::vacant();
    destination.open_active_list(pool, &mut builder)?;
    if let Some(tail) = source.admitted_tail_chunk_from_root(pool, list.coordinate(), admitted)? {
        copy_record_chunk_prefix(
            pool,
            annex_pool,
            source,
            source_annex,
            destination,
            destination_annex,
            tail,
            &mut builder,
            &mut copied_children,
        )?;
    }
    let coordinate = destination.finish_active_list(pool, &mut builder).publish();
    if copied_children.next().is_some() {
        return Err(ForkArenaError::InvalidRegion);
    }
    let identity = if semantic_identity_enabled {
        match list.semantic_identity() {
            Some(hash) => Some(SemanticSequenceIdentity::from_raw(hash, list.len())),
            None => {
                let annex = NodeAnnexView::new(annex_pool, destination_annex);
                let mut identity = SemanticSequenceIdentity::empty();
                destination.list(pool, coordinate)?.for_each(|record| {
                    identity.push_back(record.semantic_identity(annex));
                });
                Some(identity)
            }
        }
    } else {
        None
    };
    Ok((PageListId::from_parts(coordinate, identity), copied_count))
}

#[allow(clippy::too_many_arguments)]
fn collect_copy_children(
    pool: &ChunkPool<RegionNode>,
    annex_pool: &ChunkPool<u32>,
    source: &ForkArena<RegionNode, PageMaterialLane>,
    source_annex: &ForkArena<u32, NodeAnnexLane>,
    mut cursor: AdmittedListChunkCursor<PageMaterialLane>,
    children: &mut Vec<PageListId>,
) -> Result<(), ForkArenaError> {
    if let Some(previous) = source.admitted_previous_chunk(pool, &cursor)? {
        collect_copy_children(pool, annex_pool, source, source_annex, previous, children)?;
    }
    let annex = NodeAnnexView::new(annex_pool, source_annex);
    let Some((_, records)) = source.admitted_remaining_chunk(pool, &mut cursor) else {
        return Ok(());
    };
    let mut valid = true;
    records.for_each(|record| {
        valid &= record
            .visit_node_lists(annex, |child| children.push(child))
            .is_some();
    });
    if !valid {
        return Err(ForkArenaError::InvalidRange);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn copy_record_chunk_prefix(
    pool: &mut ChunkPool<RegionNode>,
    annex_pool: &mut ChunkPool<u32>,
    source: &ForkArena<RegionNode, PageMaterialLane>,
    source_annex: &ForkArena<u32, NodeAnnexLane>,
    destination: &mut ForkArena<RegionNode, PageMaterialLane>,
    destination_annex: &mut ForkArena<u32, NodeAnnexLane>,
    mut cursor: AdmittedListChunkCursor<PageMaterialLane>,
    builder: &mut ActiveListBuilder<RegionNode, PageMaterialLane>,
    copied_children: &mut impl Iterator<Item = PageListId>,
) -> Result<(), ForkArenaError> {
    if let Some(previous) = source.admitted_previous_chunk(pool, &cursor)? {
        copy_record_chunk_prefix(
            pool,
            annex_pool,
            source,
            source_annex,
            destination,
            destination_annex,
            previous,
            builder,
            copied_children,
        )?;
    }
    while let Some((_, record)) = source.admitted_next_chunk_value(pool, &mut cursor) {
        let record = *record;
        let (record, annex_dependency_floor) = record
            .reencode_between_regions(annex_pool, source_annex, destination_annex, |_| {
                copied_children.next()
            })
            .ok_or(ForkArenaError::InvalidRange)?;
        let destination_annex_view = NodeAnnexView::new(annex_pool, destination_annex);
        let (dependency_floor, child_annex_dependency_floor) = destination
            .dependency_floors_for_region_lists(pool, |visit| {
                record.visit_node_lists(destination_annex_view, |child| visit(child.coordinate()))
            })?;
        destination.append_constructed_active_list_value(
            pool,
            builder,
            record,
            dependency_floor,
            [annex_dependency_floor, child_annex_dependency_floor]
                .into_iter()
                .flatten()
                .min(),
        )?;
    }
    Ok(())
}

impl RegionValue<PageMaterialLane> for RegionNode {
    fn visit_region_lists(
        &self,
        visit: &mut dyn FnMut(crate::fork_arena::ArenaListId<PageMaterialLane>),
    ) {
        let _ = visit;
    }

    fn rebrand_region_lists(&mut self, destination_arena: u32) {
        let _ = destination_arena;
    }
}

impl RegionValue<PageMaterialLane> for Node<PageListId> {
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

impl RegionValue<NodeAnnexLane> for u32 {
    fn visit_region_lists(
        &self,
        _visit: &mut dyn FnMut(crate::fork_arena::ArenaListId<NodeAnnexLane>),
    ) {
    }

    fn rebrand_region_lists(&mut self, _destination_arena: u32) {}
}
