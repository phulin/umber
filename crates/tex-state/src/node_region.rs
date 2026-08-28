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
    ArenaListView, ChunkPool, ForkArena, ForkArenaCounters, ForkArenaError, PageMaterialLane,
    RegionValue,
};
use crate::node::Node;
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
}

impl Default for NodePool {
    fn default() -> Self {
        Self::new()
    }
}

impl NodePool {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::with_chunk_bytes(4 * 1024)
    }

    #[must_use]
    pub(crate) fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        Self {
            id: NEXT_NODE_POOL_ID.fetch_add(1, Ordering::Relaxed),
            chunks: ChunkPool::with_chunk_bytes(chunk_bytes),
            regions: Vec::new(),
            free_regions: Vec::new(),
        }
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
    fn validates_id(&self, id: NodeRegionId) -> bool {
        id.pool == self.id
            && self
                .regions
                .get(id.slot as usize)
                .is_some_and(|entry| entry.live && entry.generation == id.generation)
    }

    /// Explicitly retires a region because its chunk keys must be returned to
    /// this separately borrowed pool.
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
    _role: PhantomData<fn(Role) -> Role>,
}

impl<Role> NodeRegion<Role> {
    #[must_use]
    pub const fn id(&self) -> NodeRegionId {
        self.id
    }

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

    pub(crate) const fn counters(&self) -> ForkArenaCounters {
        self.region.counters()
    }
}

/// Failed move which returns the still-exclusive source owner unchanged.
pub(crate) struct ClosureTransferError<Role> {
    pub(crate) error: ForkArenaError,
    pub(crate) closure: OwnedNodeClosure<Role>,
}

/// Moves a whole self-contained closure envelope and rebrands every nested
/// child coordinate without moving any node address.
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
) -> Result<RegionRoot<Destination>, ForkArenaError> {
    pool.validate_region(&source.region)?;
    pool.validate_region(destination)?;
    let operation = destination.pub_arena.operation_mark(&pool.chunks);
    let mut stack = Vec::new();
    let copied = copy_list_recursive::<Source, Destination>(
        &mut pool.chunks,
        &source.region.pub_arena,
        &mut destination.pub_arena,
        source.root.list,
        &mut stack,
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
    Ok(RegionRoot {
        region: destination.id,
        list,
        _role: PhantomData,
    })
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
    Ok(RegionRoot {
        region: destination.id,
        list,
        _role: PhantomData,
    })
}

fn copy_list_recursive<Source, Destination>(
    pool: &mut ChunkPool<RegionNode>,
    source: &ForkArena<RegionNode, PageMaterialLane>,
    destination: &mut ForkArena<RegionNode, PageMaterialLane>,
    list: PageListId,
    stack: &mut Vec<PageListId>,
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
    for node in copied_nodes {
        builder.push(node)?;
    }
    let coordinate = builder.seal()?;
    Ok((list.with_coordinate(coordinate), copied_count))
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
