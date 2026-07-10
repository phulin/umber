//! Survivor arena for node lists that escape an epoch.
//!
//! Promotion copies an epoch-rooted node graph into one contiguous allocation
//! and rewrites child spans to be relative to the survivor root.

use crate::ids::{ArenaRef, NodeListId, SurvivorRootId};
use crate::math::MathField;
use crate::node::{LeaderPayload, Node};
use crate::node_arena::{NodeArena, NodeList, NodeStorage};
use std::collections::HashMap;

#[cfg(feature = "node-stats")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "node-stats")]
use std::time::Instant;

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
    // Root ids are never reused: stale handles must never become live again.
    slots: Vec<Option<SurvivorRoot>>,
    // Node storage is independent of identity and can safely be recycled.
    recycled: Vec<NodeStorage>,
    recycled_buffer_uses: usize,
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
            recycled: Vec::new(),
            recycled_buffer_uses: 0,
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
        assert!(
            self.slots.len() < (1 << 20) - 1,
            "survivor arena exceeds encodable roots"
        );

        let (storage, recycled) = self.take_recycled_buffer();
        #[cfg(not(feature = "node-stats"))]
        let _ = recycled;
        let copied = copy_list_iterative(id, epoch, self, Vec::new());
        #[cfg(feature = "node-stats")]
        {
            measurement::PEAK_SCRATCH_LOGICAL
                .fetch_max(copied.peak_scratch_logical as u64, Ordering::Relaxed);
            measurement::PEAK_SCRATCH_RETAINED
                .fetch_max(copied.peak_scratch_retained as u64, Ordering::Relaxed);
        }
        let (nodes, start, len) = (copied.nodes, copied.start, copied.len);
        let mut storage = storage;
        debug_assert!(storage.is_empty());
        storage.append(&nodes);
        let root = self.allocate_root(storage);
        self.rewrite_root_ids(root);
        let promoted = NodeListId::new_survivor(root, start, len);
        self.debug_assert_no_epoch_ids(promoted);
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
        promoted
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
            let mut root = self.slots[root.raw() as usize]
                .take()
                .expect("survivor root is not live");
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
        let Some(Some(slot)) = self.slots.get(root.raw() as usize) else {
            return false;
        };
        (id.start() as usize)
            .checked_add(id.len() as usize)
            .is_some_and(|end| end <= slot.storage.len())
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
            .max_by_key(|(_, storage)| storage.len())
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
        totals.into_values().collect()
    }

    fn allocate_root(&mut self, storage: NodeStorage) -> SurvivorRootId {
        let slot = SurvivorRoot {
            storage,
            refcount: 1,
        };
        let raw = u32_len(self.slots.len(), "survivor arena exceeds u32 roots");
        assert!(raw < (1 << 20) - 1, "survivor root id exceeds encoding");
        self.slots.push(Some(slot));
        SurvivorRootId::new(raw)
    }

    fn root(&self, root: SurvivorRootId) -> &SurvivorRoot {
        self.slots
            .get(root.raw() as usize)
            .and_then(Option::as_ref)
            .expect("survivor root is not live")
    }

    fn root_mut(&mut self, root: SurvivorRootId) -> &mut SurvivorRoot {
        let index = root.raw() as usize;
        assert!(index < self.slots.len(), "survivor root is not live");
        self.slots[index]
            .as_mut()
            .expect("survivor root is not live")
    }

    fn rewrite_root_ids(&mut self, root: SurvivorRootId) {
        let len = self.root(root).storage.len();
        for index in 0..len {
            let mut node = self
                .root(root)
                .storage
                .all_nodes()
                .get(index)
                .expect("survivor rewrite index is live")
                .to_owned();
            rewrite_node_root_ids(&mut node, root);
            self.root_mut(root).storage.replace_node(index, node);
        }
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

struct PromotionResult {
    nodes: Vec<Node>,
    start: u32,
    len: u32,
    #[cfg(feature = "node-stats")]
    peak_scratch_logical: usize,
    #[cfg(feature = "node-stats")]
    peak_scratch_retained: usize,
}

fn copy_list_iterative(
    id: NodeListId,
    epoch: &NodeArena,
    survivor: &SurvivorArena,
    out: Vec<Node>,
) -> PromotionResult {
    let mut copy = PromotionCopy::new(epoch, survivor, out);
    let root = copy.copy_list(id);
    #[cfg(feature = "node-stats")]
    copy.measure_scratch();

    while let Some(index) = copy.pending.pop() {
        remap_node_children(index, &mut copy);
        #[cfg(feature = "node-stats")]
        copy.measure_scratch();
    }

    PromotionResult {
        nodes: copy.out,
        start: root.start(),
        len: root.len(),
        #[cfg(feature = "node-stats")]
        peak_scratch_logical: copy.peak_scratch_logical,
        #[cfg(feature = "node-stats")]
        peak_scratch_retained: copy.peak_scratch_retained,
    }
}

/// Copies a mixed epoch/survivor DAG into one canonical survivor allocation.
///
/// The source arena is selected from the opaque handle's ownership tag inside
/// the arena implementation. Exact list handles are memoized before their
/// children are traversed, so shared children are copied and remapped once.
struct PromotionCopy<'a> {
    epoch: &'a NodeArena,
    survivor: &'a SurvivorArena,
    out: Vec<Node>,
    remapped: HashMap<NodeListId, NodeListId>,
    pending: Vec<usize>,
    #[cfg(feature = "node-stats")]
    peak_scratch_logical: usize,
    #[cfg(feature = "node-stats")]
    peak_scratch_retained: usize,
}

impl<'a> PromotionCopy<'a> {
    fn new(epoch: &'a NodeArena, survivor: &'a SurvivorArena, out: Vec<Node>) -> Self {
        debug_assert!(out.is_empty(), "recycled survivor buffer must be empty");
        Self {
            epoch,
            survivor,
            out,
            remapped: HashMap::new(),
            pending: Vec::new(),
            #[cfg(feature = "node-stats")]
            peak_scratch_logical: 0,
            #[cfg(feature = "node-stats")]
            peak_scratch_retained: 0,
        }
    }

    #[cfg(feature = "node-stats")]
    fn measure_scratch(&mut self) {
        let map_entry = core::mem::size_of::<(NodeListId, NodeListId)>();
        let logical = self.out.len() * core::mem::size_of::<Node>()
            + self.pending.len() * core::mem::size_of::<usize>()
            + self.remapped.len() * map_entry;
        let retained = self.out.capacity() * core::mem::size_of::<Node>()
            + self.pending.capacity() * core::mem::size_of::<usize>()
            + self.remapped.capacity() * map_entry;
        self.peak_scratch_logical = self.peak_scratch_logical.max(logical);
        self.peak_scratch_retained = self.peak_scratch_retained.max(retained);
    }

    fn copy_list(&mut self, id: NodeListId) -> NodeListId {
        if let Some(remapped) = self.remapped.get(&id) {
            return *remapped;
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
        }
        .to_vec();
        let start = u32_len(self.out.len(), "promoted node root exceeds u32 entries");
        let len = id.len();
        self.out.extend(nodes);
        let remapped = NodeListId::new_survivor(SurvivorRootId::new(0), start, len);
        self.remapped.insert(id, remapped);
        self.pending
            .extend((start as usize..start as usize + len as usize).rev());
        remapped
    }
}

fn remap_node_children(index: usize, copy: &mut PromotionCopy<'_>) {
    match copy.out[index].clone() {
        Node::HList(mut box_node) => {
            box_node.children = copy.copy_list(box_node.children);
            copy.out[index] = Node::HList(box_node);
        }
        Node::VList(mut box_node) => {
            box_node.children = copy.copy_list(box_node.children);
            copy.out[index] = Node::VList(box_node);
        }
        Node::Disc {
            kind,
            pre,
            post,
            replace,
        } => {
            copy.out[index] = Node::Disc {
                kind,
                pre: copy.copy_list(pre),
                post: copy.copy_list(post),
                replace: copy.copy_list(replace),
            };
        }
        Node::Ins {
            class,
            size,
            split_top_skip,
            split_max_depth,
            floating_penalty,
            content,
        } => {
            copy.out[index] = Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content: copy.copy_list(content),
            };
        }
        Node::Adjust(content) => {
            copy.out[index] = Node::Adjust(copy.copy_list(content));
        }
        Node::MathNoad(mut noad) => {
            remap_math_field(&mut noad.nucleus, copy);
            remap_math_field(&mut noad.subscript, copy);
            remap_math_field(&mut noad.superscript, copy);
            copy.out[index] = Node::MathNoad(noad);
        }
        Node::FractionNoad(mut fraction) => {
            fraction.numerator = copy.copy_list(fraction.numerator);
            fraction.denominator = copy.copy_list(fraction.denominator);
            copy.out[index] = Node::FractionNoad(fraction);
        }
        Node::MathChoice(mut choice) => {
            choice.display = copy.copy_list(choice.display);
            choice.text = copy.copy_list(choice.text);
            choice.script = copy.copy_list(choice.script);
            choice.script_script = copy.copy_list(choice.script_script);
            copy.out[index] = Node::MathChoice(choice);
        }
        Node::MathList(mut list) => {
            list.content = copy.copy_list(list.content);
            copy.out[index] = Node::MathList(list);
        }
        Node::Glue {
            spec,
            kind,
            leader: Some(payload),
        } => {
            copy.out[index] = Node::Glue {
                spec,
                kind,
                leader: Some(remap_leader_payload(payload, copy)),
            };
        }
        Node::Unset(mut unset) => {
            unset.children = copy.copy_list(unset.children);
            copy.out[index] = Node::Unset(unset);
        }
        Node::Char { .. }
        | Node::Lig { .. }
        | Node::Kern { .. }
        | Node::Glue { .. }
        | Node::Penalty(_)
        | Node::Rule { .. }
        | Node::Mark { .. }
        | Node::Whatsit(_)
        | Node::MathOn(_)
        | Node::MathOff(_)
        | Node::MathStyle(_)
        | Node::Nonscript => {}
    }
}

fn rewrite_node_root_ids(node: &mut Node, root: SurvivorRootId) {
    match node {
        Node::HList(box_node) | Node::VList(box_node) => {
            box_node.children = with_root(box_node.children, root);
        }
        Node::Disc {
            pre, post, replace, ..
        } => {
            *pre = with_root(*pre, root);
            *post = with_root(*post, root);
            *replace = with_root(*replace, root);
        }
        Node::Ins { content, .. } | Node::Adjust(content) => {
            *content = with_root(*content, root);
        }
        Node::MathNoad(noad) => {
            rewrite_math_field_root(&mut noad.nucleus, root);
            rewrite_math_field_root(&mut noad.subscript, root);
            rewrite_math_field_root(&mut noad.superscript, root);
        }
        Node::FractionNoad(fraction) => {
            fraction.numerator = with_root(fraction.numerator, root);
            fraction.denominator = with_root(fraction.denominator, root);
        }
        Node::MathChoice(choice) => {
            choice.display = with_root(choice.display, root);
            choice.text = with_root(choice.text, root);
            choice.script = with_root(choice.script, root);
            choice.script_script = with_root(choice.script_script, root);
        }
        Node::MathList(list) => {
            list.content = with_root(list.content, root);
        }
        Node::Glue {
            leader: Some(payload),
            ..
        } => rewrite_leader_payload_root(payload, root),
        Node::Unset(unset) => {
            unset.children = with_root(unset.children, root);
        }
        Node::Char { .. }
        | Node::Lig { .. }
        | Node::Kern { .. }
        | Node::Glue { .. }
        | Node::Penalty(_)
        | Node::Rule { .. }
        | Node::Mark { .. }
        | Node::Whatsit(_)
        | Node::MathOn(_)
        | Node::MathOff(_)
        | Node::MathStyle(_)
        | Node::Nonscript => {}
    }
}

fn remap_math_field(field: &mut MathField, copy: &mut PromotionCopy<'_>) {
    if let MathField::SubBox(list) | MathField::SubMlist(list) = field {
        *list = copy.copy_list(*list);
    }
}

fn remap_leader_payload(payload: LeaderPayload, copy: &mut PromotionCopy<'_>) -> LeaderPayload {
    match payload {
        LeaderPayload::HList(mut box_node) => {
            box_node.children = copy.copy_list(box_node.children);
            LeaderPayload::HList(box_node)
        }
        LeaderPayload::VList(mut box_node) => {
            box_node.children = copy.copy_list(box_node.children);
            LeaderPayload::VList(box_node)
        }
        LeaderPayload::Rule {
            width,
            height,
            depth,
        } => LeaderPayload::Rule {
            width,
            height,
            depth,
        },
    }
}

fn rewrite_leader_payload_root(payload: &mut LeaderPayload, root: SurvivorRootId) {
    match payload {
        LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node) => {
            box_node.children = with_root(box_node.children, root);
        }
        LeaderPayload::Rule { .. } => {}
    }
}

fn rewrite_math_field_root(field: &mut MathField, root: SurvivorRootId) {
    if let MathField::SubBox(list) | MathField::SubMlist(list) = field {
        *list = with_root(*list, root);
    }
}

fn with_root(id: NodeListId, root: SurvivorRootId) -> NodeListId {
    let ArenaRef::Survivor(_) = id.arena() else {
        panic!("promoted child should already be survivor-rooted");
    };
    NodeListId::new_survivor(root, id.start(), id.len())
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
