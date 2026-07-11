//! Survivor arena for node lists that escape an epoch.
//!
//! Promotion copies an epoch-rooted node graph into one contiguous allocation
//! and rewrites child spans to be relative to the survivor root.

use crate::ids::{ArenaRef, NodeListId, SurvivorRootId};
#[cfg(debug_assertions)]
use crate::node::Node;
use crate::node_arena::{ChildPatch, NodeArena, NodeList, NodeStorage};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

#[cfg(feature = "node-stats")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "node-stats")]
use std::time::Instant;

const SURVIVOR_ROOT_MAX: u32 = (1 << 20) - 2;
static NEXT_SURVIVOR_ROOT: AtomicU32 = AtomicU32::new(0);

/// Process-local survivor-operation measurements. Times include the complete
/// promotion/release operation; scratch bytes are allocator payload bytes and
/// exclude allocator metadata and `HashMap` control bytes.
#[cfg(feature = "node-stats")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SurvivorMeasurement {
    pub fresh_promotions: u64,
    pub fresh_promotion_nanos: u64,
    pub recycled_promotions: u64,
    pub recycled_promotion_nanos: u64,
    pub releases_to_recycling: u64,
    pub release_nanos: u64,
    pub peak_promotion_scratch_logical_bytes: u64,
    pub peak_promotion_scratch_retained_bytes: u64,
    pub source_words: u64,
    pub child_bearing_nodes: u64,
    pub remap_entries: u64,
    pub pending_entries: u64,
    pub peak_remap_entries: u64,
    pub peak_pending_entries: u64,
}

#[cfg(feature = "node-stats")]
mod measurement {
    use super::{AtomicU64, Instant, Ordering, SurvivorMeasurement};

    pub static FRESH_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static FRESH_NANOS: AtomicU64 = AtomicU64::new(0);
    pub static RECYCLED_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static RECYCLED_NANOS: AtomicU64 = AtomicU64::new(0);
    pub static RELEASE_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static RELEASE_NANOS: AtomicU64 = AtomicU64::new(0);
    pub static PEAK_SCRATCH_LOGICAL: AtomicU64 = AtomicU64::new(0);
    pub static PEAK_SCRATCH_RETAINED: AtomicU64 = AtomicU64::new(0);
    pub static SOURCE_WORDS: AtomicU64 = AtomicU64::new(0);
    pub static CHILD_BEARING_NODES: AtomicU64 = AtomicU64::new(0);
    pub static REMAP_ENTRIES: AtomicU64 = AtomicU64::new(0);
    pub static PENDING_ENTRIES: AtomicU64 = AtomicU64::new(0);
    pub static PEAK_REMAP_ENTRIES: AtomicU64 = AtomicU64::new(0);
    pub static PEAK_PENDING_ENTRIES: AtomicU64 = AtomicU64::new(0);

    #[allow(
        clippy::disallowed_methods,
        reason = "feature-gated process-local benchmark timing is not semantic engine time"
    )]
    pub fn start_timer() -> Instant {
        Instant::now()
    }

    pub fn snapshot() -> SurvivorMeasurement {
        SurvivorMeasurement {
            fresh_promotions: FRESH_CALLS.load(Ordering::Relaxed),
            fresh_promotion_nanos: FRESH_NANOS.load(Ordering::Relaxed),
            recycled_promotions: RECYCLED_CALLS.load(Ordering::Relaxed),
            recycled_promotion_nanos: RECYCLED_NANOS.load(Ordering::Relaxed),
            releases_to_recycling: RELEASE_CALLS.load(Ordering::Relaxed),
            release_nanos: RELEASE_NANOS.load(Ordering::Relaxed),
            peak_promotion_scratch_logical_bytes: PEAK_SCRATCH_LOGICAL.load(Ordering::Relaxed),
            peak_promotion_scratch_retained_bytes: PEAK_SCRATCH_RETAINED.load(Ordering::Relaxed),
            source_words: SOURCE_WORDS.load(Ordering::Relaxed),
            child_bearing_nodes: CHILD_BEARING_NODES.load(Ordering::Relaxed),
            remap_entries: REMAP_ENTRIES.load(Ordering::Relaxed),
            pending_entries: PENDING_ENTRIES.load(Ordering::Relaxed),
            peak_remap_entries: PEAK_REMAP_ENTRIES.load(Ordering::Relaxed),
            peak_pending_entries: PEAK_PENDING_ENTRIES.load(Ordering::Relaxed),
        }
    }
}

#[cfg(feature = "node-stats")]
#[must_use]
pub fn survivor_measurement() -> SurvivorMeasurement {
    measurement::snapshot()
}

/// Arena for promoted node-list roots.
#[derive(Clone, Debug)]
pub struct SurvivorArena {
    // Storage slots are local and may be cloned, while packed root keys are
    // process-unique so sibling forks cannot alias one another's new roots.
    slots: Vec<Option<SurvivorRoot>>,
    root_slots: HashMap<SurvivorRootId, usize>,
    // Node storage is independent of identity and can safely be recycled.
    recycled: Vec<NodeStorage>,
    recycled_buffer_uses: usize,
    promotion_remap: HashMap<NodeListId, NodeListId>,
    promotion_pending: Vec<ChildPatch>,
}

#[derive(Clone, Debug)]
struct SurvivorRoot {
    storage: NodeStorage,
    refcount: u32,
}

impl SurvivorArena {
    /// Creates an empty survivor arena.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            root_slots: HashMap::new(),
            recycled: Vec::new(),
            recycled_buffer_uses: 0,
            promotion_remap: HashMap::new(),
            promotion_pending: Vec::new(),
        }
    }

    /// Promotes an epoch list into one survivor root with refcount 1.
    pub(crate) fn promote(&mut self, id: NodeListId, epoch: &NodeArena) -> NodeListId {
        #[cfg(feature = "node-stats")]
        let started = measurement::start_timer();
        assert!(
            matches!(id.arena(), ArenaRef::Epoch),
            "only epoch node lists are promoted"
        );
        let root = allocate_survivor_root()
            .map(SurvivorRootId::new)
            .expect("survivor root identity space exhausted");
        let (storage, recycled) = self.take_recycled_buffer();
        #[cfg(not(feature = "node-stats"))]
        let _ = recycled;
        let remapped = core::mem::take(&mut self.promotion_remap);
        let pending = core::mem::take(&mut self.promotion_pending);
        let copied = copy_list_iterative(id, epoch, self, storage, root, remapped, pending);
        #[cfg(feature = "node-stats")]
        {
            measurement::PEAK_SCRATCH_LOGICAL
                .fetch_max(copied.peak_scratch_logical as u64, Ordering::Relaxed);
            measurement::PEAK_SCRATCH_RETAINED
                .fetch_max(copied.peak_scratch_retained as u64, Ordering::Relaxed);
        }
        self.promotion_remap = copied.remapped;
        self.promotion_pending = copied.pending;
        self.allocate_root(root, copied.storage);
        self.debug_assert_no_epoch_ids(copied.promoted);
        #[cfg(feature = "node-stats")]
        {
            let nanos = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            if recycled {
                measurement::RECYCLED_CALLS.fetch_add(1, Ordering::Relaxed);
                measurement::RECYCLED_NANOS.fetch_add(nanos, Ordering::Relaxed);
            } else {
                measurement::FRESH_CALLS.fetch_add(1, Ordering::Relaxed);
                measurement::FRESH_NANOS.fetch_add(nanos, Ordering::Relaxed);
            }
        }
        copied.promoted
    }

    /// Reads a live survivor span.
    #[must_use]
    pub(crate) fn get(&self, id: NodeListId) -> NodeList<'_> {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("survivor arena can only read survivor node-list ids");
        };
        let root = self.root(root);
        let start = id.start() as usize;
        let end = start + id.len() as usize;
        assert!(
            end <= root.storage.len(),
            "survivor node-list id is not live"
        );
        root.storage.view(id.start(), id.len())
    }

    /// Increments the root refcount for a survivor list.
    pub(crate) fn inc_ref(&mut self, id: NodeListId) {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("only survivor node-list ids are refcounted");
        };
        let root = self.root_mut(root);
        root.refcount = root
            .refcount
            .checked_add(1)
            .expect("survivor root refcount overflow");
    }

    /// Decrements the root refcount for a survivor list, freeing the root at zero.
    pub(crate) fn dec_ref(&mut self, id: NodeListId) {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("only survivor node-list ids are refcounted");
        };
        let slot = self.root_mut(root);
        slot.refcount = slot
            .refcount
            .checked_sub(1)
            .expect("survivor root refcount underflow");
        if slot.refcount == 0 {
            #[cfg(feature = "node-stats")]
            let started = measurement::start_timer();
            let index = self
                .root_slots
                .remove(&root)
                .expect("survivor root is not live");
            let mut root = self.slots[index].take().expect("survivor root is not live");
            root.storage.clear();
            self.recycled.push(root.storage);
            #[cfg(feature = "node-stats")]
            {
                measurement::RELEASE_CALLS.fetch_add(1, Ordering::Relaxed);
                let nanos = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
                measurement::RELEASE_NANOS.fetch_add(nanos, Ordering::Relaxed);
            }
        }
    }

    /// Returns whether a survivor list names a live root and span.
    #[must_use]
    pub(crate) fn contains(&self, id: NodeListId) -> bool {
        let ArenaRef::Survivor(root) = id.arena() else {
            return false;
        };
        let Some(index) = self.root_slots.get(&root).copied() else {
            return false;
        };
        let Some(Some(slot)) = self.slots.get(index) else {
            return false;
        };
        (id.start() as usize)
            .checked_add(id.len() as usize)
            .is_some_and(|end| end <= slot.storage.len())
    }

    pub(crate) fn root_storage_len(&self, id: NodeListId) -> usize {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("expected survivor node-list id");
        };
        self.root(root).storage.len()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_live_slot_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_refcount(&self, id: NodeListId) -> u32 {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("expected survivor id");
        };
        self.root(root).refcount
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_recycled_buffer_uses(&self) -> usize {
        self.recycled_buffer_uses
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_root_slot_count(&self) -> usize {
        self.slots.len()
    }

    fn take_recycled_buffer(&mut self) -> (NodeStorage, bool) {
        let Some((index, _)) = self
            .recycled
            .iter()
            .enumerate()
            .max_by_key(|(_, storage)| storage.node_capacity())
        else {
            return (NodeStorage::default(), false);
        };
        self.recycled_buffer_uses += 1;
        (self.recycled.swap_remove(index), true)
    }

    #[cfg(feature = "node-stats")]
    pub(crate) fn memory_columns(&self) -> Vec<crate::node_arena::NodeMemoryColumn> {
        use std::collections::BTreeMap;

        fn add_storage(
            totals: &mut BTreeMap<String, crate::node_arena::NodeMemoryColumn>,
            category: &str,
            storage: &NodeStorage,
        ) {
            for mut column in storage.memory_columns("storage") {
                let suffix = column
                    .name
                    .strip_prefix("storage.")
                    .expect("storage report prefix");
                let name = format!("{category}.{suffix}");
                if let Some(total) = totals.get_mut(&name) {
                    debug_assert_eq!(total.element_bytes, column.element_bytes);
                    total.len += column.len;
                    total.capacity += column.capacity;
                    total.logical_bytes += column.logical_bytes;
                    total.retained_payload_bytes += column.retained_payload_bytes;
                } else {
                    column.name = name.clone();
                    totals.insert(name, column);
                }
            }
        }

        let mut totals = BTreeMap::new();
        for root in self.slots.iter().flatten() {
            add_storage(&mut totals, "survivor.live", &root.storage);
        }
        for storage in &self.recycled {
            add_storage(&mut totals, "survivor.recycled", storage);
        }
        let mut columns: Vec<_> = totals.into_values().collect();
        let element_bytes = core::mem::size_of::<(SurvivorRootId, usize)>();
        columns.push(crate::node_arena::NodeMemoryColumn {
            name: "survivor.root_lookup_entries".to_owned(),
            len: self.root_slots.len(),
            capacity: self.root_slots.capacity(),
            element_bytes,
            logical_bytes: self.root_slots.len() * element_bytes,
            retained_payload_bytes: self.root_slots.capacity() * element_bytes,
        });
        columns
    }

    fn allocate_root(&mut self, root: SurvivorRootId, storage: NodeStorage) {
        let slot = SurvivorRoot {
            storage,
            refcount: 1,
        };
        let index = self.slots.len();
        assert!(
            self.root_slots.insert(root, index).is_none(),
            "survivor root identity was already published"
        );
        self.slots.push(Some(slot));
    }

    fn root(&self, root: SurvivorRootId) -> &SurvivorRoot {
        let index = self
            .root_slots
            .get(&root)
            .copied()
            .expect("survivor root is not live");
        self.slots
            .get(index)
            .and_then(Option::as_ref)
            .expect("survivor root is not live")
    }

    fn root_mut(&mut self, root: SurvivorRootId) -> &mut SurvivorRoot {
        let index = self
            .root_slots
            .get(&root)
            .copied()
            .expect("survivor root is not live");
        self.slots[index]
            .as_mut()
            .expect("survivor root is not live")
    }

    #[cfg(debug_assertions)]
    fn debug_assert_no_epoch_ids(&self, id: NodeListId) {
        for node in self.get(id) {
            debug_assert_no_epoch_ids_in_node(&node.to_owned());
        }
    }

    #[cfg(not(debug_assertions))]
    fn debug_assert_no_epoch_ids(&self, _id: NodeListId) {}
}

fn allocate_survivor_root() -> Option<u32> {
    NEXT_SURVIVOR_ROOT
        .fetch_update(
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
            survivor_root_successor,
        )
        .ok()
}

fn survivor_root_successor(next: u32) -> Option<u32> {
    (next <= SURVIVOR_ROOT_MAX).then_some(next + 1)
}

struct PromotionResult {
    storage: NodeStorage,
    promoted: NodeListId,
    remapped: HashMap<NodeListId, NodeListId>,
    pending: Vec<ChildPatch>,
    #[cfg(feature = "node-stats")]
    peak_scratch_logical: usize,
    #[cfg(feature = "node-stats")]
    peak_scratch_retained: usize,
}

fn copy_list_iterative(
    id: NodeListId,
    epoch: &NodeArena,
    survivor: &SurvivorArena,
    storage: NodeStorage,
    root: SurvivorRootId,
    remapped: HashMap<NodeListId, NodeListId>,
    pending: Vec<ChildPatch>,
) -> PromotionResult {
    let mut copy = PromotionCopy::new(epoch, survivor, storage, root, remapped, pending);
    let promoted = copy.copy_list(id);
    #[cfg(feature = "node-stats")]
    copy.measure_scratch();

    while let Some(patch) = copy.pending.pop() {
        let patch = patch.remap(|child| copy.copy_list(child));
        copy.storage.apply_child_patch(patch);
        #[cfg(feature = "node-stats")]
        copy.measure_scratch();
    }

    copy.remapped.clear();
    copy.pending.clear();
    PromotionResult {
        storage: copy.storage,
        promoted,
        remapped: copy.remapped,
        pending: copy.pending,
        #[cfg(feature = "node-stats")]
        peak_scratch_logical: copy.peak_scratch_logical,
        #[cfg(feature = "node-stats")]
        peak_scratch_retained: copy.peak_scratch_retained,
    }
}

/// Copies a mixed epoch/survivor DAG into one canonical survivor allocation.
/// Exact list handles are memoized before child patches are traversed, so
/// shared spans copy once while overlapping but unequal spans stay independent.
struct PromotionCopy<'a> {
    epoch: &'a NodeArena,
    survivor: &'a SurvivorArena,
    storage: NodeStorage,
    root: SurvivorRootId,
    remapped: HashMap<NodeListId, NodeListId>,
    pending: Vec<ChildPatch>,
    #[cfg(feature = "node-stats")]
    peak_scratch_logical: usize,
    #[cfg(feature = "node-stats")]
    peak_scratch_retained: usize,
}

impl<'a> PromotionCopy<'a> {
    fn new(
        epoch: &'a NodeArena,
        survivor: &'a SurvivorArena,
        storage: NodeStorage,
        root: SurvivorRootId,
        remapped: HashMap<NodeListId, NodeListId>,
        pending: Vec<ChildPatch>,
    ) -> Self {
        debug_assert!(storage.is_empty(), "recycled survivor buffer must be empty");
        debug_assert!(remapped.is_empty(), "promotion remap scratch must be clear");
        debug_assert!(pending.is_empty(), "promotion patch scratch must be clear");
        Self {
            epoch,
            survivor,
            storage,
            root,
            remapped,
            pending,
            #[cfg(feature = "node-stats")]
            peak_scratch_logical: 0,
            #[cfg(feature = "node-stats")]
            peak_scratch_retained: 0,
        }
    }

    #[cfg(feature = "node-stats")]
    fn measure_scratch(&mut self) {
        let map_entry = core::mem::size_of::<(NodeListId, NodeListId)>();
        let patch = core::mem::size_of::<ChildPatch>();
        let logical = self.pending.len() * patch + self.remapped.len() * map_entry;
        let retained = self.pending.capacity() * patch + self.remapped.capacity() * map_entry;
        self.peak_scratch_logical = self.peak_scratch_logical.max(logical);
        self.peak_scratch_retained = self.peak_scratch_retained.max(retained);
        measurement::PEAK_REMAP_ENTRIES.fetch_max(self.remapped.len() as u64, Ordering::Relaxed);
        measurement::PEAK_PENDING_ENTRIES.fetch_max(self.pending.len() as u64, Ordering::Relaxed);
    }

    fn copy_list(&mut self, id: NodeListId) -> NodeListId {
        if let Some(&remapped) = self.remapped.get(&id) {
            return remapped;
        }

        let nodes = match id.arena() {
            ArenaRef::Epoch => self.epoch.get_epoch(id),
            ArenaRef::Survivor(_) => {
                assert!(
                    self.survivor.contains(id),
                    "promotion source survivor node-list id is not live: {id:?}"
                );
                self.survivor.get(id)
            }
        };
        let len = u32_len(nodes.len(), "promoted node list exceeds u32 entries");
        let start = u32_len(self.storage.len(), "promoted node root exceeds u32 entries");
        let remapped = NodeListId::new_survivor(self.root, start, len);
        self.remapped.insert(id, remapped);
        #[cfg(feature = "node-stats")]
        let pending_before = self.pending.len();
        let appended = self.storage.append_compact(nodes, &mut self.pending);
        assert_eq!(appended, (start, len));
        #[cfg(feature = "node-stats")]
        {
            let child_patches = self.pending.len() - pending_before;
            measurement::SOURCE_WORDS.fetch_add(u64::from(len), Ordering::Relaxed);
            measurement::CHILD_BEARING_NODES.fetch_add(child_patches as u64, Ordering::Relaxed);
            measurement::REMAP_ENTRIES.fetch_add(1, Ordering::Relaxed);
            measurement::PENDING_ENTRIES.fetch_add(child_patches as u64, Ordering::Relaxed);
        }
        remapped
    }
}

#[cfg(debug_assertions)]
fn debug_assert_no_epoch_ids_in_node(node: &Node) {
    let mut children = Vec::new();
    node.child_lists(&mut children);
    for child in children {
        debug_assert!(
            matches!(child.arena(), ArenaRef::Survivor(_)),
            "promoted survivor root contains epoch node-list id"
        );
    }
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{SURVIVOR_ROOT_MAX, SurvivorArena, survivor_root_successor};
    use crate::node::Node;
    use crate::node_arena::NodeStorage;

    #[test]
    fn survivor_root_namespace_includes_its_last_packed_key() {
        assert_eq!(
            survivor_root_successor(SURVIVOR_ROOT_MAX - 1),
            Some(SURVIVOR_ROOT_MAX)
        );
        assert_eq!(
            survivor_root_successor(SURVIVOR_ROOT_MAX),
            Some(SURVIVOR_ROOT_MAX + 1)
        );
        assert_eq!(survivor_root_successor(SURVIVOR_ROOT_MAX + 1), None);
    }

    #[test]
    fn recycled_buffer_selection_prefers_largest_capacity() {
        fn cleared_storage(len: usize) -> NodeStorage {
            let mut storage = NodeStorage::default();
            storage.append(&vec![Node::Penalty(0); len]);
            storage.clear();
            storage
        }

        let large = cleared_storage(256);
        let large_capacity = large.node_capacity();
        let small = cleared_storage(8);
        assert!(large_capacity > small.node_capacity());

        let mut arena = SurvivorArena::new();
        arena.recycled = vec![large, small];

        let (selected, recycled) = arena.take_recycled_buffer();
        assert!(recycled);
        assert_eq!(selected.node_capacity(), large_capacity);
        assert!(selected.is_empty());
    }
}
