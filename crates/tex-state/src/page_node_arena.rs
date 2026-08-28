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
    ActiveListBuilder, ArenaListId, ArenaListView, ArenaRange, BatchMark, CheckpointMark,
    ForkArena, ForkArenaCounters, ForkArenaError, LocalBatch, OperationMark, PageMaterialLane,
    SealedBoundary,
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
    batch: Option<BatchMark<PageMaterialLane>>,
    dependencies: Vec<u64>,
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
            batch: None,
            dependencies: Vec::new(),
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

struct PageMaterialBatch {
    envelope: LocalBatch<PageMaterialLane>,
    references: u32,
    dependencies: Vec<u64>,
}

/// Move-only owner of one immutable page-material closure. Nested
/// `PageListId` values remain borrowed coordinates; only canonical root slots
/// and explicit TeX box copies construct this coarse lease.
#[must_use = "a page-material root must be moved into a root slot or explicitly released"]
pub struct PageMaterialRoot {
    list: PageListId,
    batch: Option<u64>,
}

impl PageMaterialRoot {
    #[must_use]
    pub const fn list(&self) -> PageListId {
        self.list
    }
}

/// Move-only interval owner retained by one aggregate checkpoint.
#[must_use = "a page-material checkpoint lease must be settled or released"]
pub struct PageMaterialCheckpointLease {
    batch_serial: u64,
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
    batches: Vec<Option<PageMaterialBatch>>,
    descriptor_batches: Vec<Option<(u32, u64)>>,
    checkpoint_intervals: std::collections::BTreeMap<u64, u32>,
    fork_batches: Option<(u64, u64)>,
    collect_queue: Vec<u64>,
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
            batches: Vec::new(),
            descriptor_batches: Vec::new(),
            checkpoint_intervals: std::collections::BTreeMap::new(),
            fork_batches: None,
            collect_queue: Vec::new(),
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
        self.region.pub_arena.counters().identity_nodes_hashed
    }

    /// Stored whole-range and whole-chunk summaries combined for identity.
    #[must_use]
    pub const fn semantic_summary_work(&self) -> u64 {
        self.region
            .pub_arena
            .counters()
            .identity_summaries_combined
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

    fn batch_serial(&self, list: PageListId) -> Result<Option<u64>, ForkArenaError> {
        Self::batch_serial_in(&self.descriptor_batches, list)
    }

    fn batch_serial_in(
        descriptor_batches: &[Option<(u32, u64)>],
        list: PageListId,
    ) -> Result<Option<u64>, ForkArenaError> {
        let Some((slot, generation)) =
            ForkArena::<PageMaterialNode, PageMaterialLane>::list_descriptor_signature(
                list.coordinate(),
            )
        else {
            return Ok(None);
        };
        descriptor_batches
            .get(slot as usize)
            .copied()
            .flatten()
            .filter(|(live_generation, _)| *live_generation == generation)
            .map(|(_, serial)| Some(serial))
            .ok_or(ForkArenaError::InvalidChunk)
    }

    fn add_dependency(dependencies: &mut Vec<u64>, dependency: Option<u64>) {
        if let Some(dependency) = dependency
            && !dependencies.contains(&dependency)
        {
            dependencies.push(dependency);
        }
    }

    fn register_batch(
        &mut self,
        envelope: LocalBatch<PageMaterialLane>,
        list: PageListId,
        dependencies: Vec<u64>,
    ) -> Result<(), ForkArenaError> {
        let Some((slot, generation)) =
            ForkArena::<PageMaterialNode, PageMaterialLane>::list_descriptor_signature(
                list.coordinate(),
            )
        else {
            return Ok(());
        };
        let serial = envelope.serial;
        let index = usize::try_from(serial.saturating_sub(1))
            .map_err(|_| ForkArenaError::CapacityOverflow)?;
        if self.batches.len() <= index {
            self.batches.resize_with(index + 1, || None);
        }
        if self.batches[index].is_some() {
            return Err(ForkArenaError::InvalidRegion);
        }
        for dependency in &dependencies {
            let record = self
                .batches
                .get_mut((*dependency - 1) as usize)
                .and_then(Option::as_mut)
                .ok_or(ForkArenaError::InvalidChunk)?;
            record.references = record
                .references
                .checked_add(1)
                .ok_or(ForkArenaError::CapacityOverflow)?;
        }
        self.batches[index] = Some(PageMaterialBatch {
            envelope,
            references: 0,
            dependencies,
        });
        let slot = slot as usize;
        if self.descriptor_batches.len() <= slot {
            self.descriptor_batches.resize(slot + 1, None);
        }
        self.descriptor_batches[slot] = Some((generation, serial));
        Ok(())
    }

    fn finish_batch(
        &mut self,
        mark: BatchMark<PageMaterialLane>,
        list: PageListId,
        dependencies: Vec<u64>,
    ) -> Result<PageListId, ForkArenaError> {
        let envelope = self
            .region
            .pub_arena
            .finish_local_batch(&mut self.pool.chunks, mark)?;
        self.register_batch(envelope, list, dependencies)?;
        Ok(list)
    }

    /// Acquires one canonical top-level owner. This is constant work; the
    /// immutable closure's dependency charges were installed when its batch
    /// was published.
    pub fn acquire_root(&mut self, list: PageListId) -> Result<PageMaterialRoot, ForkArenaError> {
        let batch = self.batch_serial(list)?;
        if let Some(serial) = batch {
            let record = self
                .batches
                .get_mut((serial - 1) as usize)
                .and_then(Option::as_mut)
                .ok_or(ForkArenaError::InvalidChunk)?;
            record.references = record
                .references
                .checked_add(1)
                .ok_or(ForkArenaError::CapacityOverflow)?;
        }
        Ok(PageMaterialRoot { list, batch })
    }

    /// Implements TeX's explicit `\copy` ownership event.
    pub fn copy_root(
        &mut self,
        root: &PageMaterialRoot,
    ) -> Result<PageMaterialRoot, ForkArenaError> {
        self.acquire_root(root.list)
    }

    pub fn release_root(&mut self, root: PageMaterialRoot) -> Result<(), ForkArenaError> {
        if let Some(serial) = root.batch {
            self.release_reference(serial)?;
            if self.batches[(serial - 1) as usize]
                .as_ref()
                .is_some_and(|batch| batch.references == 0)
            {
                self.collect_queue.push(serial);
            }
            self.collect_unrooted()?;
        }
        Ok(())
    }

    pub fn retain_checkpoint_interval(
        &mut self,
        mark: CheckpointMark<PageMaterialLane>,
    ) -> Result<PageMaterialCheckpointLease, ForkArenaError> {
        if !self.region.pub_arena.validates_checkpoint(mark) {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        let batch_serial =
            ForkArena::<PageMaterialNode, PageMaterialLane>::local_batch_serial(mark);
        *self.checkpoint_intervals.entry(batch_serial).or_default() += 1;
        Ok(PageMaterialCheckpointLease { batch_serial })
    }

    pub fn release_checkpoint_interval(
        &mut self,
        lease: PageMaterialCheckpointLease,
    ) -> Result<(), ForkArenaError> {
        let old_high_water = self
            .checkpoint_intervals
            .last_key_value()
            .map_or(0, |(serial, _)| *serial);
        let count = self
            .checkpoint_intervals
            .get_mut(&lease.batch_serial)
            .ok_or(ForkArenaError::InvalidCheckpoint)?;
        *count -= 1;
        if *count == 0 {
            self.checkpoint_intervals.remove(&lease.batch_serial);
        }
        let new_high_water = self
            .checkpoint_intervals
            .last_key_value()
            .map_or(0, |(serial, _)| *serial);
        for serial in new_high_water.saturating_add(1)..=old_high_water {
            if self
                .batches
                .get((serial - 1) as usize)
                .and_then(Option::as_ref)
                .is_some_and(|batch| batch.references == 0)
            {
                self.collect_queue.push(serial);
            }
        }
        self.collect_unrooted()
    }

    fn release_reference(&mut self, serial: u64) -> Result<(), ForkArenaError> {
        let record = self
            .batches
            .get_mut((serial - 1) as usize)
            .and_then(Option::as_mut)
            .ok_or(ForkArenaError::InvalidChunk)?;
        record.references = record
            .references
            .checked_sub(1)
            .ok_or(ForkArenaError::InvalidRegion)?;
        Ok(())
    }

    fn collect_unrooted(&mut self) -> Result<(), ForkArenaError> {
        if self.fork_batches.is_some() {
            return Ok(());
        }
        let checkpoint_high_water = self
            .checkpoint_intervals
            .last_key_value()
            .map_or(0, |(serial, _)| *serial);
        while let Some(serial) = self.collect_queue.pop() {
            if serial <= checkpoint_high_water
                || self
                    .batches
                    .get((serial - 1) as usize)
                    .and_then(Option::as_ref)
                    .is_none_or(|batch| batch.references != 0)
            {
                continue;
            }
            let Some(batch) = self
                .batches
                .get_mut((serial - 1) as usize)
                .and_then(Option::take)
            else {
                continue;
            };
            self.region
                .pub_arena
                .release_local_batch(&mut self.pool.chunks, &batch.envelope)?;
            for dependency in batch.dependencies {
                self.release_reference(dependency)?;
                let record = self
                    .batches
                    .get((dependency - 1) as usize)
                    .and_then(Option::as_ref)
                    .expect("dependency stays present through its dependent release");
                if record.references == 0 && dependency > checkpoint_high_water {
                    self.collect_queue.push(dependency);
                }
            }
        }
        Ok(())
    }

    fn retire_metadata_range(&mut self, range: std::ops::RangeInclusive<u64>) {
        for serial in range.rev() {
            let Some(batch) = self
                .batches
                .get_mut((serial - 1) as usize)
                .and_then(Option::take)
            else {
                continue;
            };
            for dependency in batch.dependencies {
                let record = self
                    .batches
                    .get_mut((dependency - 1) as usize)
                    .and_then(Option::as_mut)
                    .expect("a retired batch dependency precedes and survives it");
                record.references = record
                    .references
                    .checked_sub(1)
                    .expect("batch dependency charge is balanced");
                if record.references == 0 {
                    self.collect_queue.push(dependency);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.region.pub_arena.live_payload_values(&self.pool.chunks)
    }

    pub fn publish_owned(
        &mut self,
        nodes: impl IntoIterator<Item = PageMaterialNode>,
    ) -> Result<PageListId, ForkArenaError> {
        let batch = self
            .region
            .pub_arena
            .begin_local_batch(&mut self.pool.chunks)?;
        let mut identity = self
            .semantic_identity_enabled
            .then(SemanticSequenceIdentity::empty);
        let arena = self.region.pub_arena.region_identity();
        let mut dependencies = Vec::new();
        let descriptor_batches = &self.descriptor_batches;
        let mut builder = self.region.pub_arena.begin_builder(&mut self.pool.chunks)?;
        for node in nodes {
            if !node_children_belong_to_arena(&node, arena) {
                return Err(ForkArenaError::InvalidRegion);
            }
            node.visit_node_lists(|child| {
                Self::add_dependency(
                    &mut dependencies,
                    Self::batch_serial_in(descriptor_batches, *child)
                        .ok()
                        .flatten(),
                );
            });
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
        let list = PageListId::from_parts(coordinate, identity);
        self.finish_batch(batch, list, dependencies)
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
        builder.batch = Some(
            self.region
                .pub_arena
                .begin_local_batch(&mut self.pool.chunks)?,
        );
        self.region
            .pub_arena
            .open_active_list(&self.pool.chunks, &mut builder.inner)?;
        builder.dependencies.clear();
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
        node.visit_node_lists(|child| {
            Self::add_dependency(
                &mut builder.dependencies,
                self.batch_serial(*child).ok().flatten(),
            );
        });
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
                self.region
                    .pub_arena
                    .record_identity_work(crate::fork_arena::SequenceSummaryWork {
                        hashed_values: 1,
                        ..crate::fork_arena::SequenceSummaryWork::default()
                    });
            }
            result
        } else {
            self.region.pub_arena.push_active_list(
                &mut self.pool.chunks,
                &mut builder.inner,
                node,
            )
        }
    }

    pub fn append_to_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        list: PageListId,
    ) -> Result<(), ForkArenaError> {
        Self::add_dependency(&mut builder.dependencies, self.batch_serial(list)?);
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
        Self::add_dependency(&mut builder.dependencies, self.batch_serial(list)?);
        let selected_identity = if self.semantic_identity_enabled {
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
        let list = PageListId::from_parts(coordinate, builder.identity.take());
        let batch = builder
            .batch
            .take()
            .ok_or(ForkArenaError::InvalidActiveListBuilder)?;
        let dependencies = std::mem::take(&mut builder.dependencies);
        self.finish_batch(batch, list, dependencies)
    }

    pub fn rollback_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
    ) -> Result<(), ForkArenaError> {
        self.region
            .pub_arena
            .rollback_active_list(&mut self.pool.chunks, &mut builder.inner)?;
        builder.identity = None;
        builder.batch = None;
        builder.dependencies.clear();
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
        let batch = self
            .region
            .pub_arena
            .begin_local_batch(&mut self.pool.chunks)?;
        let mut dependencies = Vec::new();
        for list in lists {
            Self::add_dependency(&mut dependencies, self.batch_serial(*list)?);
        }
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
        self.coordinate_scratch.clear();
        self.coordinate_scratch
            .extend(lists.iter().map(|list| list.coordinate()));
        let coordinate = self.region.pub_arena.compose_lists(
            &mut self.pool.chunks,
            &self.coordinate_scratch,
            &mut self.range_scratch,
        )?;
        if identity.is_some() {
            self.region
                .pub_arena
                .record_identity_work(crate::fork_arena::SequenceSummaryWork {
                    combined_summaries: lists.len() as u64,
                    ..crate::fork_arena::SequenceSummaryWork::default()
                });
        }
        let list = PageListId::from_parts(coordinate, identity);
        self.finish_batch(batch, list, dependencies)
    }

    pub fn slice_sequence(
        &mut self,
        list: PageListId,
        selected: Range<usize>,
        _scratch: &mut Vec<PageListId>,
    ) -> Result<PageListId, ForkArenaError> {
        let batch = self
            .region
            .pub_arena
            .begin_local_batch(&mut self.pool.chunks)?;
        let mut dependencies = Vec::new();
        Self::add_dependency(&mut dependencies, self.batch_serial(list)?);
        if self.semantic_identity_enabled {
            let (coordinate, identity, work) = self.region.pub_arena.slice_list_summarized(
                &mut self.pool.chunks,
                list.coordinate(),
                selected,
                &mut self.range_scratch,
                semantic_node_identity,
            )?;
            self.region.pub_arena.record_identity_work(work);
            let list = PageListId::from_parts(coordinate, Some(identity));
            self.finish_batch(batch, list, dependencies)
        } else {
            let coordinate = self.region.pub_arena.slice_list(
                &mut self.pool.chunks,
                list.coordinate(),
                selected,
                &mut self.range_scratch,
            )?;
            let list = PageListId::from_parts(coordinate, None);
            self.finish_batch(batch, list, dependencies)
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
        let cutoff = ForkArena::<PageMaterialNode, PageMaterialLane>::operation_batch_serial(mark);
        let end = self.batches.len() as u64;
        self.region
            .pub_arena
            .restore_operation(&mut self.pool.chunks, mark)?;
        if end > cutoff {
            self.retire_metadata_range(cutoff + 1..=end);
        }
        Ok(())
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
        let cutoff = ForkArena::<PageMaterialNode, PageMaterialLane>::local_batch_serial(mark);
        let prior_end = self.batches.len() as u64;
        self.region.pub_arena.begin_checkpoint_candidate(mark)?;
        self.fork_batches = Some((cutoff, prior_end));
        Ok(())
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
        let (_, prior_end) = self.fork_batches.ok_or(ForkArenaError::NotForked)?;
        let current_end = self.batches.len() as u64;
        self.region
            .pub_arena
            .reject_checkpoint_candidate(&mut self.pool.chunks, boundary)?;
        if current_end > prior_end {
            self.retire_metadata_range(prior_end + 1..=current_end);
        }
        self.fork_batches = None;
        self.collect_unrooted()
    }

    pub fn accept_checkpoint_candidate(
        &mut self,
        boundary: SealedBoundary<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        let (cutoff, prior_end) = self.fork_batches.ok_or(ForkArenaError::NotForked)?;
        self.region
            .pub_arena
            .accept_checkpoint_candidate(&mut self.pool.chunks, boundary)?;
        if prior_end > cutoff {
            self.retire_metadata_range(cutoff + 1..=prior_end);
        }
        self.fork_batches = None;
        self.collect_unrooted()
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
