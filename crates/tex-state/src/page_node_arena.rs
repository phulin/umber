//! Runtime page-material ownership above the generic coarse fork arena.
//!
//! The generic arena remains coordinate-only. This facade is the semantic
//! boundary that pairs one canonical physical list coordinate with the
//! optional demand-maintained identity used by state hashing.

use core::hash::{Hash, Hasher};
use core::num::NonZeroU64;
use std::ops::Range;

use crate::fork_arena::{
    ActiveListBuilder, AdmittedListChunkCursor, ArenaListId, ArenaListView, ForkArenaCounters,
    ForkArenaError, OperationMark, PageMaterialLane, UniqueArenaList,
};
use crate::node::Node;
use crate::node_record::{NodeAnnexView, NodeAnnexWriter, NodeRecord};
use crate::node_region::{
    ClosureBuildMark, DurableRole, NodeCheckpointMark, NodePool, NodeRegion, NodeSealedBoundary,
    OwnedNodeClosure, PageRole, StructuralCopyReason, copy_closure_into, copy_region_root_into,
    structural_copy_fallback, transfer_closure_into, transfer_sealed_closure_into,
};
use crate::node_sequence::{SemanticSequenceIdentity, semantic_node_identity};

type PageMaterialNode = NodeRecord<PageMaterialLane>;
type OwnedPageMaterialNode = Node<PageListId>;

/// Short-lived projection of one admitted compact page-material record.
///
/// The projection retains the record and annex borrows instead of expanding
/// the record into the former 168-byte owned node carrier. Callers decode only
/// the fields needed by their scan.
#[derive(Clone, Copy)]
pub struct PageMaterialNodeRef<'a> {
    record: &'a PageMaterialNode,
    annex: NodeAnnexView<'a>,
}

impl PageMaterialNodeRef<'_> {
    #[must_use]
    pub fn character(self) -> Option<(crate::ids::FontId, char, crate::token::OriginId)> {
        self.record.character()
    }

    #[must_use]
    pub fn is_font_kern(self) -> bool {
        self.record.is_font_kern()
    }

    #[must_use]
    pub fn is_glue(self) -> bool {
        self.record.is_glue()
    }

    #[must_use]
    pub fn glue_spec_kind(self) -> Option<(crate::glue::GlueSpec, crate::node::GlueKind)> {
        self.record.glue_spec_kind(self.annex)
    }

    #[must_use]
    pub fn glue_leader(self) -> Option<Option<crate::node::LeaderPayload<PageListId>>> {
        self.record.glue_leader(self.annex)
    }

    #[must_use]
    pub fn is_math_on(self) -> bool {
        self.record.is_math_on()
    }

    #[must_use]
    pub fn is_math_off(self) -> bool {
        self.record.is_math_off()
    }

    #[must_use]
    pub fn language(self) -> Option<(u8, u8, u8)> {
        self.record.language()
    }

    #[must_use]
    pub fn math_list(self) -> Option<crate::math::MathListNode<PageListId>> {
        self.record.math_list(self.annex)
    }

    #[must_use]
    pub fn discretionary(
        self,
    ) -> Option<(
        crate::node::DiscKind,
        PageListId,
        PageListId,
        PageListId,
        u8,
    )> {
        self.record.discretionary(self.annex)
    }

    pub fn visit_ligature_source(
        self,
        visit: impl FnMut(char, crate::token::OriginId),
    ) -> Option<crate::ids::FontId> {
        self.record.visit_ligature_source(self.annex, visit)
    }
}

/// Scalar publication evidence derived from the completed resident node.
pub(crate) struct ConstructedNodeMetadata {
    pub(crate) tex82_words: (usize, usize),
    pub(crate) etex_words: (usize, usize),
    pub(crate) font: Option<crate::ids::FontId>,
}

/// Persistent coordinate-only construction state for one active node list.
#[must_use = "a page-material active list must be finalized or rolled back"]
pub struct PageMaterialActiveListBuilder {
    inner: ActiveListBuilder<PageMaterialNode, PageMaterialLane>,
    identity: Option<SemanticSequenceIdentity>,
    identity_work: crate::fork_arena::SequenceSummaryWork,
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
            identity_work: crate::fork_arena::SequenceSummaryWork {
                hashed_values: 0,
                combined_summaries: 0,
            },
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

/// Move-only whole-list result whose head predecessor has not been published.
///
/// Page journals retain copyable [`PageListSpan`] roots. Fresh builders use
/// this capability only for the right suffix of an append, where consuming it
/// permits one O(1) direct-chain splice without weakening retained roots.
pub struct UniquePageList {
    coordinate: UniqueArenaList<PageMaterialLane>,
    identity: Option<SemanticSequenceIdentity>,
}

impl UniquePageList {
    pub(crate) fn list(&self) -> PageListId {
        PageListId::from_parts(self.coordinate.coordinate(), self.identity)
    }

    fn publish(self) -> PageListId {
        PageListId::from_parts(self.coordinate.publish(), self.identity)
    }
}

impl PageListId {
    #[allow(dead_code)] // Used by the nonresident compact-node codec until its atomic cutover.
    pub(crate) const fn words(self) -> [u32; 10] {
        let coordinate = self.coordinate.words();
        let identity = match (self.coordinate.is_empty(), self.semantic_identity) {
            (true, _) | (false, None) => 0,
            (false, Some(identity)) => identity.get(),
        };
        [
            coordinate[0],
            coordinate[1],
            coordinate[2],
            coordinate[3],
            coordinate[4],
            coordinate[5],
            coordinate[6],
            coordinate[7],
            identity as u32,
            (identity >> 32) as u32,
        ]
    }

    #[allow(dead_code)] // Used by the nonresident compact-node codec until its atomic cutover.
    pub(crate) fn from_words(words: [u32; 10]) -> Option<Self> {
        let coordinate = ArenaListId::from_words([
            words[0], words[1], words[2], words[3], words[4], words[5], words[6], words[7],
        ])?;
        let raw_identity = (words[8] as u64) | ((words[9] as u64) << 32);
        if coordinate.is_empty() {
            return (raw_identity == 0).then_some(Self::empty());
        }
        let identity = (raw_identity != 0).then_some(SemanticSequenceIdentity::from_raw(
            raw_identity,
            coordinate.len(),
        ));
        Some(Self::from_parts(coordinate, identity))
    }

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

    pub(crate) const fn rebrand_arena(self, _arena: u32) -> Self {
        // Page-list identity is pool-stable. Semantic transfer changes the
        // admitting NodeRegion, never the stored coordinate.
        self
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

const _: () = assert!(core::mem::size_of::<PageListId>() <= 40);

/// Checked owner-local page-list span for repeated traversal and retention.
///
/// The constructor is private to [`PageMaterialArena`]. A span carries the
/// constant-time direct-root admission proven from its opaque construction
/// facts and never owns or copies node payload. Full chain audits remain at
/// cold region-transfer and test ingress seams.
pub struct PageListSpan {
    list: PageListId,
}

/// Stack-resident continuation for direct traversal of one admitted page span.
///
/// The cursor contains only owner-relative chunk coordinates. It neither
/// borrows nor copies node payload, so a caller may retain it across appends
/// to the same generation-owned arena while the source span remains sealed.
pub struct PageListChunkCursor {
    span: PageListSpan,
    inner: AdmittedListChunkCursor<PageMaterialLane>,
}

/// Cold materialized compatibility view for positional diagnostics and tests.
/// Routine semantic traversal uses [`crate::node_arena::NodeCursor`] directly
/// over resident compact records.
pub struct PageMaterialListView {
    nodes: Vec<OwnedPageMaterialNode>,
    #[cfg(test)]
    resident_addresses: Vec<*const OwnedPageMaterialNode>,
    #[allow(dead_code)]
    #[cfg(any(test, feature = "testing"))]
    traversal_counters: (u64, u64, u64),
}

impl PageMaterialListView {
    pub fn nodes(&self) -> &[OwnedPageMaterialNode] {
        &self.nodes
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn get(&self, index: usize) -> Option<&OwnedPageMaterialNode> {
        self.nodes.get(index)
    }
    pub fn iter(&self) -> core::slice::Iter<'_, OwnedPageMaterialNode> {
        self.nodes.iter()
    }
    pub fn for_each(&self, visit: impl FnMut(&OwnedPageMaterialNode)) {
        self.nodes.iter().for_each(visit);
    }
    #[cfg(test)]
    pub(crate) fn testing_node_address(
        &self,
        index: usize,
    ) -> Option<*const OwnedPageMaterialNode> {
        self.resident_addresses.get(index).copied()
    }
    #[allow(dead_code)]
    #[cfg(any(test, feature = "testing"))]
    pub(crate) const fn traversal_counters(&self) -> (u64, u64, u64) {
        self.traversal_counters
    }
}

impl PageListChunkCursor {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub const fn logical_start(&self) -> usize {
        self.inner.logical_start()
    }
}

impl Clone for PageListSpan {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for PageListSpan {}

impl core::fmt::Debug for PageListSpan {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PageListSpan")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl PageListSpan {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            list: PageListId::empty(),
        }
    }

    #[must_use]
    pub const fn list(self) -> PageListId {
        self.list
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.list.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.list.is_empty()
    }
}

impl Default for PageListSpan {
    fn default() -> Self {
        Self::empty()
    }
}

impl PartialEq for PageListSpan {
    fn eq(&self, other: &Self) -> bool {
        self.list == other.list
    }
}

impl Eq for PageListSpan {}

impl Hash for PageListSpan {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.list.hash(state);
    }
}

const _: () = assert!(core::mem::size_of::<PageListSpan>() <= 64);

/// Region-local page payload state. The physical pool is owned once by the
/// enclosing page-region history and is borrowed explicitly for every access.
pub struct PageMaterialRegion {
    region: NodeRegion<PageRole>,
    list_scratch: Vec<()>,
    semantic_identity_enabled: bool,
    durable_transitions: DurableTransitionCounters,
}

/// Move-only durable node closure owned by an eqtb or PDF carrier.
pub(crate) type DurableNodeClosure = OwnedNodeClosure<DurableRole>;

/// Rollback authority for a unique durable closure temporarily moved into
/// page ownership by one active command operation.
pub(crate) struct DurableTransferLoan {
    build: ClosureBuildMark<PageRole>,
    root: PageListId,
    settled: OperationMark<PageMaterialLane>,
}

type BuiltClosureMoveResult =
    Result<(PageListId, u64), (ForkArenaError, Option<ClosureBuildMark<PageRole>>)>;

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
    list_scratch: &'a mut Vec<()>,
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
            list_scratch: Vec::new(),
            semantic_identity_enabled: false,
            durable_transitions: DurableTransitionCounters::default(),
        }
    }

    pub(crate) const fn region_id(&self) -> crate::node_region::NodeRegionId {
        self.region.id()
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn profiling_physical_ownership(
        &self,
        pool: &NodePool,
    ) -> crate::node_region::NodeRegionPhysicalOwnership {
        self.region.profiling_physical_ownership(pool)
    }

    pub(crate) const fn durable_transition_counters(&self) -> DurableTransitionCounters {
        self.durable_transitions
    }

    pub(crate) const fn counters(&self) -> ForkArenaCounters {
        self.region.pub_arena.counters()
    }

    pub(crate) fn inherit_durable_transition_counters_from(&mut self, source: &Self) {
        self.durable_transitions = source.durable_transitions;
    }

    pub(crate) fn retire(self, pool: &mut NodePool) -> Result<(), ForkArenaError> {
        pool.retire_region(self.region).map_err(|(error, _)| error)
    }

    pub(crate) fn release_rootless_suffix(
        &mut self,
        pool: &mut NodePool,
        retained: Option<NodeCheckpointMark>,
    ) -> Result<usize, ForkArenaError> {
        self.region.release_rootless_suffix(pool, retained)
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

    pub(crate) fn can_share_sealed_prefix<const N: usize>(
        &self,
        pool: &NodePool,
        mark: &ClosureBuildMark<PageRole>,
        roots: [PageListId; N],
    ) -> Result<(), ForkArenaError> {
        self.region.can_share_sealed_prefix(pool, mark, roots)
    }

    pub(crate) fn share_sealed_prefix_from<const N: usize>(
        pool: &mut NodePool,
        source: &mut Self,
        mark: ClosureBuildMark<PageRole>,
        roots: [PageListId; N],
    ) -> Result<Self, ForkArenaError> {
        let region = source.region.share_sealed_prefix(pool, mark, roots)?;
        Ok(Self {
            region,
            list_scratch: Vec::new(),
            semantic_identity_enabled: source.semantic_identity_enabled,
            durable_transitions: source.durable_transitions,
        })
    }

    /// Transfers one self-contained construction suffix between page owners.
    /// A seal rejection returns the still-live build authority so the caller
    /// can roll it back before selecting an exact structural-copy fallback.
    #[allow(clippy::result_large_err)]
    pub(crate) fn move_built_closure_between(
        pool: &mut NodePool,
        destination: &mut Self,
        source: &mut Self,
        mark: ClosureBuildMark<PageRole>,
        root: PageListId,
    ) -> BuiltClosureMoveResult {
        let source_root = match source.region.root(pool, root) {
            Ok(root) => root,
            Err(error) => return Err((error, Some(mark))),
        };
        let receipt = match source.region.consumed_closure_roots_receipt(&mark) {
            Ok(receipt) => receipt,
            Err(error) => return Err((error, Some(mark))),
        };
        let sealed = source
            .region
            .seal_closure(pool, mark, source_root, receipt)
            .map_err(|failure| {
                let (error, mark) = failure.into_parts();
                (error, Some(mark))
            })?;
        let before = pool.closure_transition_counters().rebrand_scan_nodes;
        match transfer_sealed_closure_into(
            pool,
            &mut source.region,
            sealed,
            &mut destination.region,
        ) {
            Ok(root) => Ok((
                root.page_list(),
                pool.closure_transition_counters()
                    .rebrand_scan_nodes
                    .saturating_sub(before),
            )),
            Err(failure) => {
                let (error, sealed) = failure.into_parts();
                assert!(
                    source.region.rollback_closure(pool, sealed).is_ok(),
                    "failed page transfer returns its exact suffix"
                );
                Err((error, None))
            }
        }
    }

    pub(crate) fn cancel_closure_build(
        &mut self,
        pool: &mut NodePool,
        mark: ClosureBuildMark<PageRole>,
    ) -> Result<(), ForkArenaError> {
        self.region.cancel_closure_build(pool, mark)
    }

    pub(crate) fn preflight_unique_successor_adoption<const N: usize>(
        &self,
        pool: &NodePool,
        mark: &ClosureBuildMark<PageRole>,
        roots: [PageListId; N],
    ) -> Result<(), ForkArenaError> {
        self.region
            .preflight_unique_successor_adoption(pool, mark, roots)
    }

    pub(crate) fn adopt_unique_successor<const N: usize>(
        &mut self,
        pool: &mut NodePool,
        mark: ClosureBuildMark<PageRole>,
        roots: [PageListId; N],
    ) -> Result<(), ForkArenaError> {
        self.region.adopt_unique_successor(pool, mark, roots)
    }
}

impl<'a> PageMaterialArena<'a> {
    pub fn new(pool: &'a mut NodePool, state: &'a mut PageMaterialRegion) -> Self {
        Self {
            pool,
            region: &mut state.region,
            list_scratch: &mut state.list_scratch,
            semantic_identity_enabled: &mut state.semantic_identity_enabled,
            durable_transitions: &mut state.durable_transitions,
        }
    }

    fn annex_view(&self) -> NodeAnnexView<'_> {
        NodeAnnexView::new(&self.pool.annex_chunks, &self.region.annex_arena)
    }

    pub fn enable_semantic_identity(&mut self) {
        assert!(
            self.region.pub_arena.counters().new_semantic_nodes == 0
                || *self.semantic_identity_enabled,
            "semantic identity demand starts before page-node publication"
        );
        *self.semantic_identity_enabled = true;
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn record_page_output_pool_census(&self) {
        use std::collections::BTreeSet;

        use crate::measurement::{PageOutputPoolCensus, PageOutputPoolLaneCensus};

        fn lane(
            layout: crate::fork_arena::ChunkStorageLayoutCensus,
            current: Vec<u64>,
            prior: Vec<u64>,
        ) -> PageOutputPoolLaneCensus {
            let current = current.into_iter().collect::<BTreeSet<_>>();
            let prior = prior.into_iter().collect::<BTreeSet<_>>();
            let page_union = current.union(&prior).copied().collect::<BTreeSet<_>>();
            PageOutputPoolLaneCensus {
                live_blocks: layout.live_blocks,
                used_records: layout.used_records,
                stranded_records: layout.stranded_records,
                partial_blocks: layout.partial_blocks,
                physically_shared_blocks: layout.physically_shared_blocks,
                output_region_blocks: current.len() as u64,
                durable_or_other_blocks: layout.live_blocks.saturating_sub(page_union.len() as u64),
            }
        }

        let (nodes, annexes) = self.pool.profiling_storage_layout();
        let owners = self.region.profiling_physical_ownership(self.pool);
        crate::measurement::record_page_output_pool_census(PageOutputPoolCensus {
            nodes: lane(nodes, owners.current_nodes, owners.prior_nodes),
            annexes: lane(annexes, owners.current_annexes, owners.prior_annexes),
            ..PageOutputPoolCensus::default()
        });
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

    #[cfg(test)]
    pub(crate) fn allocated_heap_bytes(&self) -> usize {
        self.pool.chunks.allocated_heap_bytes()
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn payload_chunk_capacity(&self) -> usize {
        self.region
            .pub_arena
            .payload_chunk_capacity(&self.pool.chunks)
    }

    pub fn publish_owned(
        &mut self,
        nodes: impl IntoIterator<Item = OwnedPageMaterialNode>,
    ) -> Result<PageListId, ForkArenaError> {
        Ok(self.publish_owned_unique(nodes)?.publish())
    }

    pub fn publish_owned_unique(
        &mut self,
        nodes: impl IntoIterator<Item = OwnedPageMaterialNode>,
    ) -> Result<UniquePageList, ForkArenaError> {
        let mut builder = PageMaterialActiveListBuilder::vacant();
        self.open_active_list(&mut builder)?;
        for node in nodes {
            if let Err(error) = self.push_active_list(&mut builder, node) {
                self.rollback_active_list(&mut builder)
                    .expect("failed page publication returns its exact suffix");
                return Err(error);
            }
        }
        self.finalize_unique_active_list(&mut builder)
    }

    pub fn publish_owned_span(
        &mut self,
        nodes: impl IntoIterator<Item = OwnedPageMaterialNode>,
    ) -> Result<PageListSpan, ForkArenaError> {
        let list = self.publish_owned(nodes)?;
        self.admit_span(list)
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

    /// Seals the suffix opened before ordinary box/form construction and
    /// moves its envelopes into a fresh durable owner. Addresses stay stable;
    /// the only traversal is the bounded child-coordinate rebrand scan.
    ///
    /// If a nested child predates the build mark, the suffix is not
    /// self-contained. That explicit structural case copies exactly the
    /// selected recursive closure, then rolls the construction suffix back.
    pub(crate) fn finish_built_page_root_to_durable(
        &mut self,
        mark: ClosureBuildMark<PageRole>,
        root: PageListId,
    ) -> Result<DurableNodeClosure, ForkArenaError> {
        let source_root = self.region.root(self.pool, root)?;
        let receipt = self.region.consumed_closure_roots_receipt(&mark)?;
        let sealed = match self
            .region
            .seal_closure(self.pool, mark, source_root, receipt)
        {
            Ok(sealed) => sealed,
            Err(failure) => {
                let (error, mark) = failure.into_parts();
                if error != ForkArenaError::InvalidRegion {
                    self.region.cancel_closure_build(self.pool, mark)?;
                    return Err(error);
                }
                let mut durable = self.pool.start_region::<DurableRole>()?;
                let before = durable.counters().source_nodes_copied;
                let copied = match structural_copy_fallback(
                    self.pool,
                    self.region,
                    source_root,
                    &mut durable,
                    StructuralCopyReason::InterleavedPrefixChild,
                ) {
                    Ok(copied) => copied,
                    Err(error) => {
                        assert!(
                            self.pool.retire_region(durable).is_ok(),
                            "failed structural destination remains quiescent"
                        );
                        self.region.cancel_closure_build(self.pool, mark)?;
                        return Err(error);
                    }
                };
                self.region.cancel_closure_build(self.pool, mark)?;
                let copied_nodes = durable
                    .counters()
                    .source_nodes_copied
                    .saturating_sub(before);
                let owner =
                    durable
                        .into_closure(self.pool, copied)
                        .map_err(|(error, region)| {
                            assert!(
                                self.pool.retire_region(region).is_ok(),
                                "validated structural destination retires"
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
                return Ok(owner);
            }
        };

        let mut durable = self.pool.start_region::<DurableRole>()?;
        let before = self.pool.closure_transition_counters().rebrand_scan_nodes;
        let durable_root =
            match transfer_sealed_closure_into(self.pool, self.region, sealed, &mut durable) {
                Ok(root) => root,
                Err(failure) => {
                    let (error, sealed) = failure.into_parts();
                    self.region
                        .rollback_closure(self.pool, sealed)
                        .map_err(|failure| failure.into_parts().0)?;
                    assert!(
                        self.pool.retire_region(durable).is_ok(),
                        "failed transfer destination remains quiescent"
                    );
                    return Err(error);
                }
            };
        let scanned = self
            .pool
            .closure_transition_counters()
            .rebrand_scan_nodes
            .saturating_sub(before);
        self.durable_transitions.node_closure_scan_nodes = self
            .durable_transitions
            .node_closure_scan_nodes
            .saturating_add(scanned);
        durable
            .into_closure(self.pool, durable_root)
            .map_err(|(error, region)| {
                assert!(
                    self.pool.retire_region(region).is_ok(),
                    "validated durable destination retires"
                );
                error
            })
    }

    /// Publishes a built closure without detaching a construction-suffix root
    /// still owned by the page builder.
    pub(crate) fn finish_built_page_root_to_durable_preserving_roots<const N: usize>(
        &mut self,
        mark: ClosureBuildMark<PageRole>,
        root: PageListId,
        retained_roots: [PageListId; N],
    ) -> Result<DurableNodeClosure, ForkArenaError> {
        if !self
            .region
            .build_suffix_contains_any_root(self.pool, &mark, retained_roots)?
        {
            return self.finish_built_page_root_to_durable(mark, root);
        }

        let source_root = self.region.root(self.pool, root)?;
        let mut durable = self.pool.start_region::<DurableRole>()?;
        let before = durable.counters().source_nodes_copied;
        let copied = match structural_copy_fallback(
            self.pool,
            self.region,
            source_root,
            &mut durable,
            StructuralCopyReason::RetainedRoot,
        ) {
            Ok(copied) => copied,
            Err(error) => {
                assert!(
                    self.pool.retire_region(durable).is_ok(),
                    "failed retained-root destination remains quiescent"
                );
                return Err(error);
            }
        };
        let copied_nodes = durable
            .counters()
            .source_nodes_copied
            .saturating_sub(before);
        let owner = durable
            .into_closure(self.pool, copied)
            .map_err(|(error, region)| {
                assert!(
                    self.pool.retire_region(region).is_ok(),
                    "validated retained-root destination retires"
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
        Ok(owner)
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

    /// Moves a unique durable owner into the page region while retaining the
    /// exact suffix authority needed to reverse the move on operation
    /// rollback. No history-preservation copy is made merely because the
    /// command operation is live.
    #[allow(clippy::result_large_err)]
    pub(crate) fn loan_durable_to_page(
        &mut self,
        closure: DurableNodeClosure,
    ) -> Result<(PageListId, DurableTransferLoan), (ForkArenaError, DurableNodeClosure)> {
        let build = match self.region.begin_closure_build(self.pool) {
            Ok(build) => build,
            Err(error) => return Err((error, closure)),
        };
        let root = match transfer_closure_into(self.pool, closure, self.region) {
            Ok(root) => root.list(),
            Err(failure) => {
                self.region
                    .cancel_closure_build(self.pool, build)
                    .expect("empty failed transfer suffix rolls back");
                return Err((failure.error, failure.closure));
            }
        };
        let settled = self.region.pub_arena.operation_mark(&self.pool.chunks);
        Ok((
            root,
            DurableTransferLoan {
                build,
                root,
                settled,
            },
        ))
    }

    pub(crate) fn commit_durable_transfer_loan(&mut self, _loan: DurableTransferLoan) {
        // The page carrier may already have nested the root, shipped it, or
        // rotated the complete page region before the command commit barrier.
        // Committing consumes rollback authority; it does not need a second
        // liveness scan or representation.
    }

    pub(crate) fn rollback_durable_transfer_loan(
        &mut self,
        loan: DurableTransferLoan,
    ) -> Result<DurableNodeClosure, ForkArenaError> {
        self.region
            .pub_arena
            .restore_operation(&mut self.pool.chunks, loan.settled)?;
        self.finish_built_page_root_to_durable(loan.build, loan.root)
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
        let annex_operation = self
            .region
            .annex_arena
            .operation_mark(&self.pool.annex_chunks);
        self.region
            .pub_arena
            .open_active_list(&self.pool.chunks, &mut builder.inner)?;
        self.region.active_annex_operation = Some(annex_operation);
        builder.identity = (*self.semantic_identity_enabled).then(SemanticSequenceIdentity::empty);
        builder.identity_work = crate::fork_arena::SequenceSummaryWork::default();
        Ok(())
    }

    pub fn push_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        node: OwnedPageMaterialNode,
    ) -> Result<(), ForkArenaError> {
        if !owned_node_children_are_live(&self.region.pub_arena, &self.pool.chunks, &node) {
            return Err(ForkArenaError::InvalidRegion);
        }
        let child_annex_dependency_floor = self
            .region
            .pub_arena
            .paired_dependency_floor_for(&self.pool.chunks, &node)?;
        let item_identity = builder
            .identity
            .as_ref()
            .map(|_| semantic_node_identity(&node));
        let (record, annex_dependency_floor) = {
            let mut annex =
                NodeAnnexWriter::new(&mut self.pool.annex_chunks, &mut self.region.annex_arena);
            let record = NodeRecord::encode_owned(node.clone(), &mut annex);
            (record, annex.dependency_floor())
        };
        let slot = self
            .region
            .pub_arena
            .reserve_region_value_active_list_slot(
                &mut self.pool.chunks,
                &mut builder.inner,
                &node,
                item_identity,
            )?;
        assert!(slot.is_none(), "reserved page-node destination is vacant");
        *slot = Some(record);
        self.region.pub_arena.record_active_paired_dependency(
            &mut self.pool.chunks,
            &mut builder.inner,
            [annex_dependency_floor, child_annex_dependency_floor]
                .into_iter()
                .flatten()
                .min(),
        )?;
        if let Some(item_identity) = item_identity {
            if let Some(identity) = &mut builder.identity {
                identity.push_back(item_identity);
            }
            builder.identity_work.hashed_values =
                builder.identity_work.hashed_values.saturating_add(1);
        }
        Ok(())
    }

    /// Constructs one generated node directly in its final checked arena slot.
    pub(crate) fn construct_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        initialize: impl FnOnce(crate::NodeDestination<'_>),
    ) -> Result<ConstructedNodeMetadata, ForkArenaError> {
        let identity_enabled = builder.identity.is_some();
        let annex_operation = self
            .region
            .annex_arena
            .operation_mark(&self.pool.annex_chunks);
        let (constructed, annex_dependency_floor) = {
            let mut annex =
                NodeAnnexWriter::new(&mut self.pool.annex_chunks, &mut self.region.annex_arena);
            let constructed = self.region.pub_arena.construct_region_value_active_list(
                &mut self.pool.chunks,
                &mut builder.inner,
                identity_enabled,
                &mut annex,
                |slot, annex| initialize(crate::NodeDestination::new_record(slot, annex)),
                |record, annex| {
                    let node = record
                        .decode_owned(annex.view())
                        .ok_or(ForkArenaError::InvalidRange)?;
                    let mut font = None;
                    node.visit_fonts(|value| {
                        assert!(
                            font.replace(value).is_none(),
                            "one node has at most one direct font"
                        );
                    });
                    let metadata = ConstructedNodeMetadata {
                        tex82_words: node.tex_memory_words(false),
                        etex_words: node.tex_memory_words(true),
                        font,
                    };
                    Ok((semantic_node_identity(&node), metadata, node))
                },
            );
            (constructed, annex.dependency_floor())
        };
        let (item_identity, metadata) = match constructed {
            Ok(constructed) => constructed,
            Err(error) => {
                self.region
                    .annex_arena
                    .restore_operation(&mut self.pool.annex_chunks, annex_operation)
                    .expect("failed destination construction restores its annex suffix");
                return Err(error);
            }
        };
        self.region.pub_arena.record_active_paired_dependency(
            &mut self.pool.chunks,
            &mut builder.inner,
            annex_dependency_floor,
        )?;
        if let Some(item_identity) = item_identity {
            if let Some(identity) = &mut builder.identity {
                identity.push_back(item_identity);
            }
            builder.identity_work.hashed_values =
                builder.identity_work.hashed_values.saturating_add(1);
        }
        Ok(metadata)
    }

    pub fn append_to_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        list: PageListId,
    ) -> Result<(), ForkArenaError> {
        let span = self.admit_span(list)?;
        self.append_span_to_active_list(builder, span)
    }

    pub fn append_span_to_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        span: PageListSpan,
    ) -> Result<(), ForkArenaError> {
        self.append_reencoded_span_range(builder, span, 0..span.len(), true)
    }

    /// Moves one unpublished whole chain into the builder's private suffix.
    pub fn append_unique_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        list: UniquePageList,
    ) -> Result<(), ForkArenaError> {
        let UniquePageList {
            coordinate,
            identity: appended_identity,
        } = list;
        self.region.pub_arena.append_unique_active_list(
            &mut self.pool.chunks,
            &mut builder.inner,
            coordinate,
        )?;
        if let Some(identity) = &mut builder.identity {
            *identity = identity
                .concat(appended_identity.expect("demand-enabled unique list carries identity"));
            builder.identity_work.combined_summaries =
                builder.identity_work.combined_summaries.saturating_add(1);
        }
        Ok(())
    }

    pub fn append_range_to_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        list: PageListId,
        selected: Range<usize>,
    ) -> Result<(), ForkArenaError> {
        let span = self.admit_span(list)?;
        self.append_span_range_to_active_list(builder, span, selected)
    }

    pub fn append_span_range_to_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        span: PageListSpan,
        selected: Range<usize>,
    ) -> Result<(), ForkArenaError> {
        if self
            .region
            .pub_arena
            .paired_dependency_floor_for_list(&self.pool.chunks, span.list.coordinate())?
            .is_none()
        {
            let selected_identity = if *self.semantic_identity_enabled {
                let annex = NodeAnnexView::new(&self.pool.annex_chunks, &self.region.annex_arena);
                let (identity, work) = self
                    .region
                    .pub_arena
                    .append_validated_active_list_range_summarized(
                        &mut self.pool.chunks,
                        &mut builder.inner,
                        span.list.coordinate(),
                        selected,
                        |record| semantic_record_identity(record, annex),
                    )?;
                builder.identity_work.hashed_values = builder
                    .identity_work
                    .hashed_values
                    .saturating_add(work.hashed_values);
                builder.identity_work.combined_summaries = builder
                    .identity_work
                    .combined_summaries
                    .saturating_add(work.combined_summaries);
                Some(identity)
            } else {
                self.region.pub_arena.append_validated_active_list_range(
                    &mut self.pool.chunks,
                    &mut builder.inner,
                    span.list.coordinate(),
                    selected,
                )?;
                None
            };
            if let (Some(identity), Some(selected_identity)) =
                (&mut builder.identity, selected_identity)
            {
                *identity = identity.concat(selected_identity);
            }
            return Ok(());
        }
        self.append_reencoded_span_range(builder, span, selected, false)
    }

    fn append_reencoded_span_range(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        span: PageListSpan,
        selected: Range<usize>,
        whole_source: bool,
    ) -> Result<(), ForkArenaError> {
        if selected.start > selected.end || selected.end > span.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        self.region
            .pub_arena
            .begin_reencoded_active_list_copy(span.len(), selected.len());
        let mut selected_identity = builder
            .identity
            .as_ref()
            .map(|_| SemanticSequenceIdentity::empty());
        let mut copied = 0_usize;
        if let Some(tail) = self.span_tail_chunk(span)? {
            self.append_reencoded_chunk_range(
                builder,
                span,
                &tail,
                &selected,
                &mut selected_identity,
                &mut copied,
            )?;
        }
        if copied != selected.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        if selected_identity.is_some() {
            builder.identity_work.hashed_values = builder
                .identity_work
                .hashed_values
                .saturating_add(copied as u64);
            builder.identity_work.combined_summaries = builder
                .identity_work
                .combined_summaries
                .saturating_add(u64::from(whole_source));
        }
        if let (Some(identity), Some(selected_identity)) =
            (&mut builder.identity, selected_identity)
        {
            *identity = identity.concat(selected_identity);
        }
        Ok(())
    }

    fn append_reencoded_chunk_range(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        span: PageListSpan,
        cursor: &PageListChunkCursor,
        selected: &Range<usize>,
        selected_identity: &mut Option<SemanticSequenceIdentity>,
        copied: &mut usize,
    ) -> Result<(), ForkArenaError> {
        if let Some(previous) = self.span_previous_chunk(span, cursor)? {
            self.append_reencoded_chunk_range(
                builder,
                span,
                &previous,
                selected,
                selected_identity,
                copied,
            )?;
        }
        for offset in 0..cursor.len() {
            let (index, node) = self.span_chunk_node(span, cursor, offset)?;
            if !selected.contains(&index) {
                continue;
            }
            let record = *node.record;
            let annex = node.annex;
            let (dependency_floor, child_annex_dependency_floor) = self
                .region
                .pub_arena
                .dependency_floors_for_region_lists(&self.pool.chunks, |visit| {
                    record.visit_node_lists(annex, |child| visit(child.coordinate()))
                })?;
            let item_identity = selected_identity
                .as_ref()
                .map(|_| semantic_record_identity(&record, annex));
            if let (Some(identity), Some(item_identity)) =
                (selected_identity.as_mut(), item_identity)
            {
                identity.push_back(item_identity);
            }
            let (record, annex_dependency_floor) = record
                .reencode_same_region(
                    &mut self.pool.annex_chunks,
                    &mut self.region.annex_arena,
                    Some,
                )
                .ok_or(ForkArenaError::InvalidRange)?;
            self.region
                .pub_arena
                .append_reencoded_active_list_copy_value(
                    &mut self.pool.chunks,
                    &mut builder.inner,
                    record,
                    item_identity,
                    dependency_floor,
                    [annex_dependency_floor, child_annex_dependency_floor]
                        .into_iter()
                        .flatten()
                        .min(),
                )?;
            *copied += 1;
        }
        Ok(())
    }

    pub fn finalize_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
    ) -> Result<PageListId, ForkArenaError> {
        Ok(self.finalize_unique_active_list(builder)?.publish())
    }

    pub fn finalize_unique_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
    ) -> Result<UniquePageList, ForkArenaError> {
        self.region
            .pub_arena
            .finalize_active_list(&mut self.pool.chunks, &mut builder.inner)?;
        let coordinate = builder.inner.take_unique_sealed()?;
        self.region.active_annex_operation = None;
        self.region
            .pub_arena
            .record_identity_work(builder.identity_work);
        builder.identity_work = crate::fork_arena::SequenceSummaryWork::default();
        Ok(UniquePageList {
            coordinate,
            identity: builder.identity.take(),
        })
    }

    /// Publishes a move-only list without copying it.
    ///
    /// This is reserved for semantic ownership boundaries that need to place
    /// the finished coordinate inside another immutable node rather than
    /// splice it into a list chain.
    pub fn publish_unique_list(&self, list: UniquePageList) -> PageListId {
        list.publish()
    }

    pub fn finalize_active_span(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
    ) -> Result<PageListSpan, ForkArenaError> {
        let list = self.finalize_active_list(builder)?;
        self.admit_span(list)
    }

    pub fn rollback_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
    ) -> Result<(), ForkArenaError> {
        self.region
            .pub_arena
            .rollback_active_list(&mut self.pool.chunks, &mut builder.inner)?;
        let annex_operation = self
            .region
            .active_annex_operation
            .take()
            .ok_or(ForkArenaError::InvalidActiveListBuilder)?;
        self.region
            .annex_arena
            .restore_operation(&mut self.pool.annex_chunks, annex_operation)?;
        builder.identity = None;
        builder.identity_work = crate::fork_arena::SequenceSummaryWork::default();
        Ok(())
    }

    pub fn publish_range(
        &mut self,
        nodes: Vec<OwnedPageMaterialNode>,
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
        let coordinate = self.compose_reencoded_lists(lists.iter().copied())?;
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

    /// Consumes a freshly built whole right suffix after one admitted shared
    /// left root. This is the production O(1) append seam used by page and
    /// mode owners; it neither copies nodes nor walks the existing chain.
    pub fn append_unique_to_span(
        &mut self,
        left: PageListSpan,
        right: UniquePageList,
    ) -> Result<PageListSpan, ForkArenaError> {
        let UniquePageList {
            coordinate: right_coordinate,
            identity: right_identity,
        } = right;
        let identity = match *self.semantic_identity_enabled {
            true => Some(
                left.list
                    .sequence_identity()
                    .expect("demand-enabled left span carries identity")
                    .concat(right_identity.expect("demand-enabled unique suffix carries identity")),
            ),
            false => None,
        };
        let coordinate = self.region.pub_arena.append_unique_to_validated_list(
            &mut self.pool.chunks,
            left.list.coordinate(),
            right_coordinate,
        )?;
        if identity.is_some() {
            self.region
                .pub_arena
                .record_identity_work(crate::fork_arena::SequenceSummaryWork {
                    combined_summaries: 1,
                    ..crate::fork_arena::SequenceSummaryWork::default()
                });
        }
        Ok(PageListSpan {
            list: PageListId::from_parts(coordinate, identity),
        })
    }

    /// Converts a removed semantic owner into move-only direct-chain
    /// authority. The root must still have its original unlinked head.
    pub fn reclaim_unique_span(
        &self,
        span: PageListSpan,
    ) -> Result<UniquePageList, ForkArenaError> {
        let coordinate = self
            .region
            .pub_arena
            .reclaim_unlinked_validated_list(&self.pool.chunks, span.list.coordinate())?;
        Ok(UniquePageList {
            coordinate,
            identity: span.list.sequence_identity(),
        })
    }

    pub fn compose_spans(
        &mut self,
        spans: &[PageListSpan],
    ) -> Result<PageListSpan, ForkArenaError> {
        let identity = if *self.semantic_identity_enabled {
            let mut identity = SemanticSequenceIdentity::empty();
            for span in spans {
                identity = identity.concat(
                    span.list
                        .sequence_identity()
                        .expect("demand-enabled page span carries identity"),
                );
            }
            Some(identity)
        } else {
            None
        };
        let coordinate = self.compose_reencoded_lists(spans.iter().map(|span| span.list))?;
        if identity.is_some() {
            self.region
                .pub_arena
                .record_identity_work(crate::fork_arena::SequenceSummaryWork {
                    combined_summaries: spans.len() as u64,
                    ..crate::fork_arena::SequenceSummaryWork::default()
                });
        }
        let list = PageListId::from_parts(coordinate, identity);
        self.admit_span(list)
    }

    fn compose_reencoded_lists(
        &mut self,
        lists: impl IntoIterator<Item = PageListId>,
    ) -> Result<ArenaListId<PageMaterialLane>, ForkArenaError> {
        let mut root = ArenaListId::empty();
        for list in lists {
            if list.is_empty() {
                continue;
            }
            self.region
                .pub_arena
                .admit_list(&self.pool.chunks, list.coordinate())?;
            if root.is_empty() {
                root = list.coordinate();
                continue;
            }
            let span = PageListSpan { list };
            let mut builder = PageMaterialActiveListBuilder::vacant();
            self.open_active_list(&mut builder)?;
            builder.identity = None;
            self.append_reencoded_span_range(&mut builder, span, 0..span.len(), false)?;
            let copied = self.finalize_unique_active_list(&mut builder)?;
            root = self.region.pub_arena.append_unique_to_validated_list(
                &mut self.pool.chunks,
                root,
                copied.coordinate,
            )?;
        }
        Ok(root)
    }

    pub fn slice_sequence(
        &mut self,
        list: PageListId,
        selected: Range<usize>,
        _scratch: &mut Vec<PageListId>,
    ) -> Result<PageListId, ForkArenaError> {
        if *self.semantic_identity_enabled {
            let annex = NodeAnnexView::new(&self.pool.annex_chunks, &self.region.annex_arena);
            let (coordinate, identity, work) = self.region.pub_arena.slice_list_summarized(
                &mut self.pool.chunks,
                list.coordinate(),
                selected,
                self.list_scratch,
                |record| semantic_record_identity(record, annex),
            )?;
            self.region.pub_arena.record_identity_work(work);
            Ok(PageListId::from_parts(coordinate, Some(identity)))
        } else {
            let coordinate = self.region.pub_arena.slice_list(
                &mut self.pool.chunks,
                list.coordinate(),
                selected,
                self.list_scratch,
            )?;
            Ok(PageListId::from_parts(coordinate, None))
        }
    }

    pub fn slice_span(
        &mut self,
        span: PageListSpan,
        selected: Range<usize>,
    ) -> Result<PageListSpan, ForkArenaError> {
        let list = if *self.semantic_identity_enabled {
            let annex = NodeAnnexView::new(&self.pool.annex_chunks, &self.region.annex_arena);
            let (coordinate, identity, work) =
                self.region.pub_arena.slice_validated_list_summarized(
                    &mut self.pool.chunks,
                    span.list.coordinate(),
                    selected,
                    self.list_scratch,
                    |record| semantic_record_identity(record, annex),
                )?;
            self.region.pub_arena.record_identity_work(work);
            PageListId::from_parts(coordinate, Some(identity))
        } else {
            let coordinate = self.region.pub_arena.slice_validated_list(
                &mut self.pool.chunks,
                span.list.coordinate(),
                selected,
                self.list_scratch,
            )?;
            PageListId::from_parts(coordinate, None)
        };
        self.admit_span(list)
    }

    pub fn list(&self, list: PageListId) -> Result<PageMaterialListView, ForkArenaError> {
        let view = self
            .region
            .pub_arena
            .list(&self.pool.chunks, list.coordinate())?;
        materialize_page_list(view, self.annex_view())
    }

    pub fn admit_span(&self, list: PageListId) -> Result<PageListSpan, ForkArenaError> {
        self.region
            .pub_arena
            .admit_owned_list(&self.pool.chunks, list.coordinate())?;
        Ok(PageListSpan { list })
    }

    pub fn span_list(&self, span: PageListSpan) -> Result<PageMaterialListView, ForkArenaError> {
        let view = self
            .region
            .pub_arena
            .validated_list(&self.pool.chunks, span.list.coordinate())?;
        materialize_page_list(view, self.annex_view())
    }

    pub fn get(&self, list: PageListId) -> Result<PageMaterialListView, ForkArenaError> {
        self.list(list)
    }

    pub fn node_cursor(
        &self,
        list: PageListId,
    ) -> Result<crate::node_arena::NodeCursor<'_>, ForkArenaError> {
        self.admit_span(list)
            .and_then(|span| self.span_node_cursor(span))
    }

    pub fn span_node_cursor(
        &self,
        span: PageListSpan,
    ) -> Result<crate::node_arena::NodeCursor<'_>, ForkArenaError> {
        self.region
            .pub_arena
            .validated_list(&self.pool.chunks, span.list.coordinate())
            .map(|view| crate::node_arena::NodeCursor::fork_arena(view, self.annex_view()))
    }

    /// Starts a mutation-compatible direct chunk walk after one span admission.
    pub fn span_tail_chunk(
        &self,
        span: PageListSpan,
    ) -> Result<Option<PageListChunkCursor>, ForkArenaError> {
        self.region
            .pub_arena
            .admitted_tail_chunk(&self.pool.chunks, span.list.coordinate())
            .map(|cursor| cursor.map(|inner| PageListChunkCursor { span, inner }))
    }

    /// Returns the preceding source chunk through its sole persistent edge.
    pub fn span_previous_chunk(
        &self,
        span: PageListSpan,
        cursor: &PageListChunkCursor,
    ) -> Result<Option<PageListChunkCursor>, ForkArenaError> {
        if cursor.span != span {
            return Err(ForkArenaError::InvalidRange);
        }
        self.region
            .pub_arena
            .admitted_previous_chunk(&self.pool.chunks, &cursor.inner)
            .map(|previous| previous.map(|inner| PageListChunkCursor { span, inner }))
    }

    /// Borrows one node directly from a retained packed-chunk coordinate.
    pub fn span_chunk_node(
        &self,
        span: PageListSpan,
        cursor: &PageListChunkCursor,
        offset: usize,
    ) -> Result<(usize, PageMaterialNodeRef<'_>), ForkArenaError> {
        if cursor.span != span {
            return Err(ForkArenaError::InvalidRange);
        }
        let (index, record) = self.region.pub_arena.admitted_chunk_value(
            &self.pool.chunks,
            span.list.coordinate(),
            &cursor.inner,
            offset,
        )?;
        Ok((
            index,
            PageMaterialNodeRef {
                record,
                annex: self.annex_view(),
            },
        ))
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

    pub fn seal_boundary(&mut self) -> Result<NodeSealedBoundary, ForkArenaError> {
        self.region.seal_checkpoint_boundary(self.pool)
    }

    pub fn checkpoint_mark(
        &self,
        boundary: NodeSealedBoundary,
    ) -> Result<NodeCheckpointMark, ForkArenaError> {
        self.region.checkpoint_mark(boundary)
    }

    pub fn begin_checkpoint_candidate(
        &mut self,
        mark: NodeCheckpointMark,
    ) -> Result<(), ForkArenaError> {
        self.region.begin_checkpoint_candidate(self.pool, mark)
    }

    #[must_use]
    pub fn validates_checkpoint(&self, mark: NodeCheckpointMark) -> bool {
        self.region.validates_checkpoint(mark)
    }

    #[must_use]
    pub fn can_restore_checkpoint(&self, mark: NodeCheckpointMark) -> bool {
        self.region.can_begin_checkpoint_candidate(mark)
    }

    pub fn restore_checkpoint(&mut self, mark: NodeCheckpointMark) -> Result<(), ForkArenaError> {
        self.region.restore_checkpoint(self.pool, mark)
    }

    pub fn reject_checkpoint_candidate(
        &mut self,
        boundary: NodeSealedBoundary,
    ) -> Result<(), ForkArenaError> {
        self.region.reject_checkpoint_candidate(self.pool, boundary)
    }

    pub fn accept_checkpoint_candidate(
        &mut self,
        boundary: NodeSealedBoundary,
    ) -> Result<(), ForkArenaError> {
        self.region.accept_checkpoint_candidate(self.pool, boundary)
    }
}

fn owned_node_children_are_live(
    arena: &crate::fork_arena::ForkArena<PageMaterialNode, PageMaterialLane>,
    pool: &crate::fork_arena::ChunkPool<PageMaterialNode>,
    node: &OwnedPageMaterialNode,
) -> bool {
    let mut valid = true;
    node.visit_node_lists(|child| {
        valid &= arena.admit_list(pool, child.coordinate()).is_ok();
    });
    valid
}

fn semantic_record_identity(record: &PageMaterialNode, annex: NodeAnnexView<'_>) -> u64 {
    record.semantic_identity(annex)
}

fn materialize_page_list(
    view: ArenaListView<'_, PageMaterialNode, PageMaterialLane>,
    annex: NodeAnnexView<'_>,
) -> Result<PageMaterialListView, ForkArenaError> {
    #[cfg(any(test, feature = "testing"))]
    let traversal_counters = view.traversal_counters();
    let mut nodes = Vec::with_capacity(view.len());
    #[cfg(test)]
    let mut resident_addresses = Vec::with_capacity(view.len());
    let mut invalid = false;
    view.for_each(|record| match record.decode_owned(annex) {
        Some(node) => {
            #[cfg(test)]
            resident_addresses.push(core::ptr::from_ref(record).cast());
            nodes.push(node);
        }
        None => invalid = true,
    });
    if invalid {
        return Err(ForkArenaError::InvalidRange);
    }
    Ok(PageMaterialListView {
        nodes,
        #[cfg(test)]
        resident_addresses,
        #[cfg(any(test, feature = "testing"))]
        traversal_counters,
    })
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

    pub fn list(&self, list: PageListId) -> Result<PageMaterialListView, ForkArenaError> {
        let view = self
            .state
            .region
            .pub_arena
            .list(&self.pool.chunks, list.coordinate())?;
        materialize_page_list(
            view,
            NodeAnnexView::new(&self.pool.annex_chunks, &self.state.region.annex_arena),
        )
    }

    pub fn span_list(&self, span: PageListSpan) -> Result<PageMaterialListView, ForkArenaError> {
        let view = self
            .state
            .region
            .pub_arena
            .validated_list(&self.pool.chunks, span.list.coordinate())?;
        materialize_page_list(
            view,
            NodeAnnexView::new(&self.pool.annex_chunks, &self.state.region.annex_arena),
        )
    }

    pub fn get(&self, list: PageListId) -> Result<PageMaterialListView, ForkArenaError> {
        self.list(list)
    }

    pub fn node_cursor(
        &self,
        list: PageListId,
    ) -> Result<crate::node_arena::NodeCursor<'a>, ForkArenaError> {
        self.state
            .region
            .pub_arena
            .validated_list(&self.pool.chunks, list.coordinate())
            .map(|view| {
                crate::node_arena::NodeCursor::fork_arena(
                    view,
                    NodeAnnexView::new(&self.pool.annex_chunks, &self.state.region.annex_arena),
                )
            })
    }

    pub fn span_node_cursor(
        &self,
        span: PageListSpan,
    ) -> Result<crate::node_arena::NodeCursor<'a>, ForkArenaError> {
        self.state
            .region
            .pub_arena
            .validated_list(&self.pool.chunks, span.list.coordinate())
            .map(|view| {
                crate::node_arena::NodeCursor::fork_arena(
                    view,
                    NodeAnnexView::new(&self.pool.annex_chunks, &self.state.region.annex_arena),
                )
            })
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
    pub fn validates_checkpoint(&self, mark: NodeCheckpointMark) -> bool {
        self.state.region.validates_checkpoint(mark)
    }

    #[must_use]
    pub fn can_restore_checkpoint(&self, mark: NodeCheckpointMark) -> bool {
        self.state.region.can_begin_checkpoint_candidate(mark)
    }

    pub fn checkpoint_mark(
        &self,
        boundary: NodeSealedBoundary,
    ) -> Result<NodeCheckpointMark, ForkArenaError> {
        self.state.region.checkpoint_mark(boundary)
    }
}

#[cfg(test)]
#[path = "page_node_arena/tests.rs"]
mod tests;
