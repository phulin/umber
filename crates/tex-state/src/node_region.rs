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
    AdmittedListChunkCursor, BatchMark, CheckpointMark, ChunkPool, DetachedBatch, ForkArena,
    ForkArenaCounters, ForkArenaError, NodePoolStorageClass, PageMaterialLane, RegionValue,
    SealedBoundary, SequenceSummaryWork, TransferredIntervals,
};

#[cfg(feature = "profiling")]
use crate::fork_arena::ChunkStorageLayoutCensus;
use crate::node::Node;
use crate::node_record::{NodeAnnexView, NodeAnnexWriter, NodeRecord};
use crate::node_sequence::SemanticSequenceIdentity;
use crate::page_node_arena::PageListId;

#[cfg(test)]
#[path = "node_region/tests.rs"]
mod tests;

mod consumed_cut;
mod copy;
pub(crate) use consumed_cut::copy_consumed_direct_cut_into;

#[cfg(any(feature = "profiling", feature = "testing"))]
pub use copy::{ExplicitCopyHarness, ExplicitCopyShape};

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

/// Operation-local authority to return an older, exclusively removed box
/// interval to its page owner if the enclosing command rejects.
pub(crate) struct PageInteriorTransferLoan {
    source: NodeRegionId,
    destination: NodeRegionId,
    chunks: PageInteriorTransferredChunks,
}

enum PageInteriorTransferredChunks {
    Partitioned {
        nodes: TransferredIntervals<PageMaterialLane>,
        annex: TransferredIntervals<NodeAnnexLane>,
    },
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

impl NodeRegionId {
    pub(crate) const fn words(self) -> [u32; 4] {
        [
            self.pool as u32,
            (self.pool >> 32) as u32,
            self.slot,
            self.generation,
        ]
    }

    pub(crate) const fn from_words(words: [u32; 4]) -> Option<Self> {
        let pool = words[0] as u64 | ((words[1] as u64) << 32);
        if pool == 0 || words[3] == 0 {
            return None;
        }
        Some(Self {
            pool,
            slot: words[2],
            generation: words[3],
        })
    }
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
/// `ChunkPool<NodeRecord>` and its paired annex pool own the physical blocks.
/// Page and durable regions own disjoint envelopes over pool-stable logical
/// coordinates; no page root, child, checkpoint, format, or output value
/// exposes a physical block key or retains an owned-node representation.
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
    pub retained_root_fallbacks: u64,
}

/// Why a caller deliberately selected the bounded recursive-copy seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructuralCopyReason {
    InterleavedPrefixChild,
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
                4_096,
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
            mark.batch.payload_start(),
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

    pub(crate) fn can_restore_checkpoint(&self, mark: NodeCheckpointMark) -> bool {
        (self.pub_arena.can_begin_checkpoint_candidate(mark.nodes)
            && self.annex_arena.can_begin_checkpoint_candidate(mark.annex))
            || (self.pub_arena.validates_checkpoint(mark.nodes)
                && self.annex_arena.validates_checkpoint(mark.annex))
    }

    fn can_restore_current_checkpoint(&self, mark: NodeCheckpointMark) -> bool {
        self.pub_arena.validates_checkpoint(mark.nodes)
            && self.annex_arena.validates_checkpoint(mark.annex)
            && self.pub_arena.is_forked()
            && self.annex_arena.is_forked()
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
        if self.can_restore_current_checkpoint(mark) {
            self.pub_arena
                .restore_current_checkpoint(&mut pool.chunks, mark.nodes)?;
            self.annex_arena
                .restore_current_checkpoint(&mut pool.annex_chunks, mark.annex)?;
            return Ok(());
        }
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

    #[cfg(any(test, feature = "profiling", feature = "testing"))]
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

    /// Publishes one newly constructed durable wrapper around child storage
    /// already admitted by this region. The source wrapper can have a live
    /// predecessor outside the moved body; no part of it is copied here.
    pub(crate) fn publish_box_wrapper(
        &mut self,
        pool: &mut NodePool,
        node: Node<PageListId>,
        identity: Option<SemanticSequenceIdentity>,
    ) -> Result<RegionRoot<Role>, ForkArenaError> {
        if !matches!(node, Node::HList(_) | Node::VList(_)) {
            return Err(ForkArenaError::InvalidRange);
        }
        pool.validate_region(self)?;
        let annex_operation = self.annex_arena.operation_mark(&pool.annex_chunks);
        let coordinate = (|| {
            let mut builder = self.pub_arena.begin_builder(&mut pool.chunks)?;
            let child_annex_floor = builder.paired_dependency_floor_for(&node)?;
            let (record, annex_floor) = {
                let mut annex = NodeAnnexWriter::new(&mut pool.annex_chunks, &mut self.annex_arena);
                let record = NodeRecord::encode_owned(node.clone(), &mut annex);
                (record, annex.dependency_floor())
            };
            builder.push_with_dependencies(record, &node)?;
            builder.record_paired_dependency(
                [annex_floor, child_annex_floor].into_iter().flatten().min(),
            )?;
            Ok::<_, ForkArenaError>(builder.finish())
        })();
        let coordinate = match coordinate {
            Ok(coordinate) => coordinate,
            Err(error) => {
                self.annex_arena
                    .restore_operation(&mut pool.annex_chunks, annex_operation)?;
                return Err(error);
            }
        };
        Ok(RegionRoot {
            region: self.id,
            list: PageListId::from_parts(coordinate, identity),
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
            mark.batch.payload_start(),
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
        self.pub_arena.can_discard_sealed_batch_suffix(
            &pool.chunks,
            &mark.batch,
            &mark.rollback,
        )?;
        self.annex_arena.can_discard_sealed_batch_suffix(
            &pool.annex_chunks,
            &mark.annex_batch,
            &mark.annex_rollback,
        )?;
        self.pub_arena
            .discard_sealed_batch_suffix(&mut pool.chunks, mark.batch, mark.rollback)?;
        self.annex_arena.discard_sealed_batch_suffix(
            &mut pool.annex_chunks,
            mark.annex_batch,
            mark.annex_rollback,
        )
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
        // Compact node records keep child lists in the paired annex, so
        // RegionValue's generic walk cannot see them. Publication folded
        // those children into each node chunk's dependency floor. A held-over
        // insertion can sit in the build suffix while its content precedes
        // it; adopting only the suffix would leave that content behind.
        self.pub_arena
            .preflight_shared_prefix_metadata(&pool.chunks, &mark.batch, &coordinates)?;
        self.pub_arena.preflight_paired_dependency_floor(
            &pool.chunks,
            &coordinates,
            mark.batch.payload_start(),
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
            mark.batch.payload_start(),
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
    ) -> Result<crate::node_view::NodeCursor<'region>, ForkArenaError> {
        pool.validate_region(self)?;
        if root.region != self.id {
            return Err(ForkArenaError::InvalidRegion);
        }
        let view = self.pub_arena.list(&pool.chunks, root.list.coordinate())?;
        let annex = NodeAnnexView::new(&pool.annex_chunks, &self.annex_arena);
        Ok(crate::node_view::NodeCursor::fork_arena(view, annex))
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

/// Non-owning coordinates of one boxed construction. They become a transfer
/// authority only when the semantic list owner consumes that exact box root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageBoxSegment {
    region: NodeRegionId,
    node_start: u32,
    node_end: u32,
    annex_start: u32,
    annex_end: u32,
}

impl PageBoxSegment {
    pub(crate) fn from_boundaries(
        start: ClosureBuildMark<PageRole>,
        end: ClosureBuildMark<PageRole>,
    ) -> Result<Self, ForkArenaError> {
        if start.region != end.region
            || start.batch.payload_start() > end.batch.payload_start()
            || start.annex_batch.payload_start() > end.annex_batch.payload_start()
        {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok(Self {
            region: start.region,
            node_start: start.batch.payload_start() as u32,
            node_end: end.batch.payload_start() as u32,
            annex_start: start.annex_batch.payload_start() as u32,
            annex_end: end.annex_batch.payload_start() as u32,
        })
    }

    pub(crate) fn from_live_end(
        start: &ClosureBuildMark<PageRole>,
        node_end: usize,
        annex_end: usize,
    ) -> Result<Self, ForkArenaError> {
        let node_start = start.batch.payload_start();
        let annex_start = start.annex_batch.payload_start();
        let node_end = u32::try_from(node_end).map_err(|_| ForkArenaError::CapacityOverflow)?;
        let annex_end = u32::try_from(annex_end).map_err(|_| ForkArenaError::CapacityOverflow)?;
        if node_start >= node_end as usize || annex_start >= annex_end as usize {
            return Err(ForkArenaError::InvalidRegion);
        }
        Ok(Self {
            region: start.region,
            node_start: node_start as u32,
            node_end,
            annex_start: annex_start as u32,
            annex_end,
        })
    }

    pub(crate) const fn words(self) -> [u32; 8] {
        let region = self.region.words();
        [
            region[0],
            region[1],
            region[2],
            region[3],
            self.node_start,
            self.node_end,
            self.annex_start,
            self.annex_end,
        ]
    }

    pub(crate) const fn from_words(words: [u32; 8]) -> Option<Self> {
        let Some(region) = NodeRegionId::from_words([words[0], words[1], words[2], words[3]])
        else {
            return None;
        };
        if words[4] >= words[5] || words[6] >= words[7] {
            return None;
        }
        Some(Self {
            region,
            node_start: words[4],
            node_end: words[5],
            annex_start: words[6],
            annex_end: words[7],
        })
    }

    pub(crate) const fn from_exclusion_bounds(
        region: NodeRegionId,
        bounds: [u32; 4],
    ) -> Option<Self> {
        if bounds[0] > bounds[1]
            || bounds[2] > bounds[3]
            || (bounds[0] == bounds[1] && bounds[2] == bounds[3])
        {
            return None;
        }
        Some(Self {
            region,
            node_start: bounds[0],
            node_end: bounds[1],
            annex_start: bounds[2],
            annex_end: bounds[3],
        })
    }

    pub(crate) const fn exclusion_from_bounds(self, bounds: [u32; 4]) -> Option<Self> {
        Self::from_exclusion_bounds(self.region, bounds)
    }

    /// Rebind construction coordinates to the admitted position of their
    /// original wrapper. Whole-region moves preserve each lane's relative
    /// logical spacing, including vacant positions left by earlier loans.
    pub(crate) fn rebased_to_wrapper(
        self,
        region: NodeRegionId,
        node_wrapper: usize,
        annex_wrapper: usize,
    ) -> Option<Self> {
        let node_shift =
            i64::try_from(node_wrapper).ok()? - i64::from(self.node_end.checked_sub(1)?);
        let annex_shift =
            i64::try_from(annex_wrapper).ok()? - i64::from(self.annex_end.checked_sub(1)?);
        self.shifted(region, node_shift, annex_shift)
    }

    pub(crate) fn shifted_like(self, original: Self, rebased: Self) -> Option<Self> {
        if self.region != original.region {
            return None;
        }
        let node_shift = i64::from(rebased.node_end) - i64::from(original.node_end);
        let annex_shift = i64::from(rebased.annex_end) - i64::from(original.annex_end);
        self.shifted(rebased.region, node_shift, annex_shift)
    }

    fn shifted(self, region: NodeRegionId, node_shift: i64, annex_shift: i64) -> Option<Self> {
        fn offset(position: u32, shift: i64) -> Option<u32> {
            u32::try_from(i64::from(position).checked_add(shift)?).ok()
        }
        Some(Self {
            region,
            node_start: offset(self.node_start, node_shift)?,
            node_end: offset(self.node_end, node_shift)?,
            annex_start: offset(self.annex_start, annex_shift)?,
            annex_end: offset(self.annex_end, annex_shift)?,
        })
    }

    pub const fn is_empty(self) -> bool {
        self.node_start == self.node_end && self.annex_start == self.annex_end
    }

    pub(crate) const fn node_range(self) -> std::ops::Range<usize> {
        self.node_start as usize..self.node_end as usize
    }

    pub(crate) const fn annex_range(self) -> std::ops::Range<usize> {
        self.annex_start as usize..self.annex_end as usize
    }

    /// Construction rotates both tails before publishing the sole wrapper,
    /// so the final logical chunk of each lane belongs to that wrapper.
    pub(crate) const fn body_node_range(self) -> std::ops::Range<usize> {
        self.node_start as usize..(self.node_end - 1) as usize
    }

    pub(crate) const fn body_annex_range(self) -> std::ops::Range<usize> {
        self.annex_start as usize..(self.annex_end - 1) as usize
    }

    pub(crate) const fn region(self) -> NodeRegionId {
        self.region
    }
}

impl<Role> ClosureBuildMark<Role> {
    pub(crate) const fn region_id(&self) -> NodeRegionId {
        self.region
    }
}

/// Consumed proof that no owner-local root outside the closure names its
/// suffix. It is intentionally neither clonable nor constructible from raw
/// coordinates.
pub(crate) struct ConsumedClosureRootsReceipt<Role> {
    region: NodeRegionId,
    serial: u64,
    _role: PhantomData<fn(Role) -> Role>,
}

/// Move-only detached closure suffix. Payload addresses remain stable while
/// this loan is transferred or rolled back.
#[must_use = "a detached closure loan must be transferred or rolled back"]
pub(crate) struct SealedNodeClosure<Role> {
    source: NodeRegionId,
    root: RegionRoot<Role>,
    batch: NodeEnvelopeBatch,
}

pub(crate) struct SealedNodeClosureError<Role> {
    pub(crate) error: ForkArenaError,
    pub(crate) closure: SealedNodeClosure<Role>,
}

impl<Role> SealedNodeClosureError<Role> {
    pub(crate) fn into_parts(self) -> (ForkArenaError, SealedNodeClosure<Role>) {
        (self.error, self.closure)
    }
}

/// Failed seal with the original move-only construction authority restored.
pub(crate) struct ClosureSealError<Role> {
    pub(crate) error: ForkArenaError,
    pub(crate) mark: ClosureBuildMark<Role>,
}

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
    ) -> Result<crate::node_view::NodeCursor<'region>, ForkArenaError> {
        self.region.list(pool, self.root)
    }

    pub(crate) fn child_list<'region>(
        &'region self,
        pool: &'region NodePool,
        list: PageListId,
    ) -> Result<crate::node_view::NodeCursor<'region>, ForkArenaError> {
        let root = self.region.root(pool, list)?;
        self.region.list(pool, root)
    }

    pub(crate) fn into_region(self) -> NodeRegion<Role> {
        self.region
    }

    pub(crate) fn region_mut(&mut self) -> &mut NodeRegion<Role> {
        &mut self.region
    }

    pub(crate) const fn root(&self) -> RegionRoot<Role> {
        self.root
    }
}

impl OwnedNodeClosure<DurableRole> {
    pub(crate) fn set_root_box_dimension(
        &mut self,
        pool: &mut NodePool,
        dimension: crate::command_context::BoxDimension,
        value: crate::scaled::Scaled,
    ) -> Result<crate::scaled::Scaled, ForkArenaError> {
        pool.validate_region(&self.region)?;
        if self.root.list.len() != 1 {
            return Err(ForkArenaError::InvalidRange);
        }
        let record = *self
            .region
            .pub_arena
            .list(&pool.chunks, self.root.list.coordinate())?
            .get(0)
            .ok_or(ForkArenaError::InvalidRange)?;
        let previous = crate::node_record::set_root_box_dimension(
            record,
            &mut pool.annex_chunks,
            &mut self.region.annex_arena,
            dimension,
            value,
        )?;
        if self.root.list.semantic_identity().is_some() {
            let annex = crate::node_record::NodeAnnexView::new(
                &pool.annex_chunks,
                &self.region.annex_arena,
            );
            let mut identity = crate::node_sequence::SemanticSequenceIdentity::empty();
            identity.push_back(record.semantic_identity(annex));
            self.root.list = PageListId::from_parts(self.root.list.coordinate(), Some(identity));
        }
        Ok(previous)
    }
}

/// Commits a detached construction suffix into a destination region. Failed
/// destination validation returns the move-only suffix loan unchanged.
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

/// Moves a completed, exclusively consumed box interval that precedes the
/// current construction mark. Both typed lanes retain their source-relative
/// logical positions; the returned loan restores them before operation roots.
#[allow(clippy::too_many_arguments)]
pub(crate) fn transfer_page_interior_closure(
    pool: &mut NodePool,
    source: &mut NodeRegion<PageRole>,
    root: RegionRoot<PageRole>,
    node_range: std::ops::Range<usize>,
    annex_range: std::ops::Range<usize>,
    destination: &mut NodeRegion<DurableRole>,
) -> Result<(RegionRoot<DurableRole>, PageInteriorTransferLoan), ForkArenaError> {
    preflight_page_interior_closure(
        pool,
        source,
        root,
        node_range.clone(),
        annex_range.clone(),
        destination,
    )?;
    let node_ranges = (!node_range.is_empty())
        .then_some(node_range)
        .into_iter()
        .collect::<Vec<_>>();
    let annex_ranges = (!annex_range.is_empty())
        .then_some(annex_range)
        .into_iter()
        .collect::<Vec<_>>();
    let loan =
        transfer_page_interior_intervals(pool, source, &node_ranges, &annex_ranges, destination)?;
    Ok((
        RegionRoot {
            region: destination.id,
            list: root.list,
            _role: PhantomData,
        },
        loan,
    ))
}

/// Creates an exact reversible paired loan for a box with no resident body.
/// The vacant destination coordinates position its newly built wrapper after
/// the source wrapper's construction boundary without moving source data.
pub(crate) fn loan_empty_page_box_body(
    pool: &mut NodePool,
    source: &mut NodeRegion<PageRole>,
    node_position: usize,
    annex_position: usize,
    destination: &mut NodeRegion<DurableRole>,
) -> Result<PageInteriorTransferLoan, ForkArenaError> {
    preflight_empty_page_box_body(pool, source, node_position, annex_position, destination)?;
    transfer_page_interior_intervals(pool, source, &[], &[], destination)
}

pub(crate) fn preflight_empty_page_box_body(
    pool: &NodePool,
    source: &NodeRegion<PageRole>,
    node_position: usize,
    annex_position: usize,
    destination: &NodeRegion<DurableRole>,
) -> Result<(), ForkArenaError> {
    pool.validate_region(source)?;
    pool.validate_region(destination)?;
    let _ = (node_position, annex_position);
    preflight_page_interior_intervals(pool, source, &[], &[], destination)
}

pub(crate) fn preflight_page_interior_closure(
    pool: &NodePool,
    source: &NodeRegion<PageRole>,
    root: RegionRoot<PageRole>,
    node_range: std::ops::Range<usize>,
    annex_range: std::ops::Range<usize>,
    destination: &NodeRegion<DurableRole>,
) -> Result<(), ForkArenaError> {
    pool.validate_region(source)?;
    pool.validate_region(destination)?;
    if root.region != source.id {
        return Err(ForkArenaError::InvalidRegion);
    }
    source.pub_arena.preflight_interval_root(
        &pool.chunks,
        root.list.coordinate(),
        node_range.start,
        node_range.end,
    )?;
    let node_ranges = (!node_range.is_empty())
        .then_some(node_range)
        .into_iter()
        .collect::<Vec<_>>();
    let annex_ranges = (!annex_range.is_empty())
        .then_some(annex_range)
        .into_iter()
        .collect::<Vec<_>>();
    preflight_page_interior_intervals(pool, source, &node_ranges, &annex_ranges, destination)
}

fn interval_contains(ranges: &[std::ops::Range<usize>], position: usize) -> bool {
    let index = ranges.partition_point(|range| range.end <= position);
    ranges
        .get(index)
        .is_some_and(|range| range.start <= position && position < range.end)
}

fn interval_contains_range(
    ranges: &[std::ops::Range<usize>],
    dependency: std::ops::Range<usize>,
) -> bool {
    let index = ranges.partition_point(|range| range.end <= dependency.start);
    ranges
        .get(index)
        .is_some_and(|range| range.start <= dependency.start && dependency.end <= range.end)
}

/// Proves exact direct closure over both selected lanes before a disjoint box
/// body move. The generic arena checks selected chunk ownership/predecessors;
/// this typed pass checks each resident record's direct child and annex keys.
pub(crate) fn preflight_page_interior_intervals(
    pool: &NodePool,
    source: &NodeRegion<PageRole>,
    node_ranges: &[std::ops::Range<usize>],
    annex_ranges: &[std::ops::Range<usize>],
    destination: &NodeRegion<DurableRole>,
) -> Result<(), ForkArenaError> {
    pool.validate_region(source)?;
    pool.validate_region(destination)?;
    source.pub_arena.preflight_interior_intervals(
        &pool.chunks,
        &destination.pub_arena,
        node_ranges,
    )?;
    source.annex_arena.preflight_interior_intervals(
        &pool.annex_chunks,
        &destination.annex_arena,
        annex_ranges,
    )?;
    let annex = NodeAnnexView::new(&pool.annex_chunks, &source.annex_arena);
    source
        .pub_arena
        .visit_interval_values(&pool.chunks, node_ranges, |record| {
            let mut closed = true;
            record
                .visit_node_lists(annex, |child| {
                    if child.is_empty() {
                        return;
                    }
                    let admitted = source
                        .pub_arena
                        .owner_relative_list_block_range(&pool.chunks, child.coordinate())
                        .ok();
                    closed &= admitted.is_some_and(|range| {
                        interval_contains(node_ranges, range.start)
                            && interval_contains(node_ranges, range.end - 1)
                    });
                })
                .ok_or(ForkArenaError::InvalidRange)?;
            record
                .visit_annex_block_ranges(annex, |range| {
                    closed &= interval_contains_range(annex_ranges, range);
                })
                .ok_or(ForkArenaError::InvalidRange)?;
            closed.then_some(()).ok_or(ForkArenaError::InvalidRegion)
        })
}

pub(crate) fn transfer_page_interior_intervals(
    pool: &mut NodePool,
    source: &mut NodeRegion<PageRole>,
    node_ranges: &[std::ops::Range<usize>],
    annex_ranges: &[std::ops::Range<usize>],
    destination: &mut NodeRegion<DurableRole>,
) -> Result<PageInteriorTransferLoan, ForkArenaError> {
    preflight_page_interior_intervals(pool, source, node_ranges, annex_ranges, destination)?;
    let nodes = source.pub_arena.transfer_interior_intervals(
        &mut pool.chunks,
        &mut destination.pub_arena,
        node_ranges,
    )?;
    let annex = source.annex_arena.transfer_interior_intervals(
        &mut pool.annex_chunks,
        &mut destination.annex_arena,
        annex_ranges,
    )?;
    pool.closure_transitions.envelope_moves =
        pool.closure_transitions.envelope_moves.saturating_add(1);
    Ok(PageInteriorTransferLoan {
        source: source.id,
        destination: destination.id,
        chunks: PageInteriorTransferredChunks::Partitioned { nodes, annex },
    })
}

pub(crate) fn rollback_page_interior_closure(
    pool: &mut NodePool,
    source: &mut NodeRegion<PageRole>,
    destination: &mut NodeRegion<DurableRole>,
    loan: PageInteriorTransferLoan,
) -> Result<(), ForkArenaError> {
    pool.validate_region(source)?;
    pool.validate_region(destination)?;
    if loan.source != source.id || loan.destination != destination.id {
        return Err(ForkArenaError::InvalidRegion);
    }
    match loan.chunks {
        PageInteriorTransferredChunks::Partitioned { nodes, annex } => {
            source.pub_arena.preflight_rollback_interior_intervals(
                &pool.chunks,
                &destination.pub_arena,
                &nodes,
            )?;
            source.annex_arena.preflight_rollback_interior_intervals(
                &pool.annex_chunks,
                &destination.annex_arena,
                &annex,
            )?;
            source.pub_arena.rollback_interior_intervals(
                &mut pool.chunks,
                &mut destination.pub_arena,
                nodes,
            )?;
            source.annex_arena.rollback_interior_intervals(
                &mut pool.annex_chunks,
                &mut destination.annex_arena,
                annex,
            )
        }
    }
}

/// Moves a whole self-contained closure envelope and rebrands every nested
/// child coordinate without moving any node address.
pub(crate) fn transfer_closure_into<Source, Destination>(
    pool: &mut NodePool,
    closure: &mut OwnedNodeClosure<Source>,
    destination: &mut NodeRegion<Destination>,
) -> Result<RegionRoot<Destination>, ForkArenaError> {
    let source_node_base = closure.region.pub_arena.payload_base_position();
    let source_annex_base = closure.region.annex_arena.payload_base_position();
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
            )?;
            closure.region.pub_arena.preflight_paired_dependency_floor(
                &pool.chunks,
                &[closure.root.list.coordinate()],
                source_node_base,
                source_annex_base,
            )?;
            let nodes = closure.region.pub_arena.payload_position_end() - source_node_base;
            let annex = closure.region.annex_arena.payload_position_end() - source_annex_base;
            if destination
                .pub_arena
                .payload_position_end()
                .checked_add(nodes)
                .is_none_or(|end| end > u32::MAX as usize)
                || destination
                    .annex_arena
                    .payload_position_end()
                    .checked_add(annex)
                    .is_none_or(|end| end > u32::MAX as usize)
            {
                return Err(ForkArenaError::CapacityOverflow);
            }
            Ok(())
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
        .rebase_dependency_suffix(&mut pool.chunks, destination_node_start, source_node_base)
        .expect("whole-region node dependency floors were preflighted");
    destination
        .annex_arena
        .rebase_dependency_suffix(
            &mut pool.annex_chunks,
            destination_annex_start,
            source_annex_base,
        )
        .expect("whole-region annex dependency floors were preflighted");
    destination
        .pub_arena
        .rebase_paired_dependency_suffix(
            &mut pool.chunks,
            destination_node_start,
            source_annex_base,
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
    let copied = copy::CopyContext::new(pool, source, destination, semantic_identity_enabled)
        .copy_list(root.list);
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
        StructuralCopyReason::RetainedRoot => &mut pool.closure_transitions.retained_root_fallbacks,
    };
    *reason_counter = reason_counter.saturating_add(1);
    Ok(copied)
}

impl RegionValue<PageMaterialLane> for RegionNode {
    const HAS_INLINE_REGION_LISTS: bool = false;

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
    const HAS_INLINE_REGION_LISTS: bool = false;

    fn visit_region_lists(
        &self,
        _visit: &mut dyn FnMut(crate::fork_arena::ArenaListId<NodeAnnexLane>),
    ) {
    }

    fn rebrand_region_lists(&mut self, _destination_arena: u32) {}
}
