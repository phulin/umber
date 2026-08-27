//! Arena-owned immutable node lists.
//!
//! Node-list coordinates never own storage. A scratch, page, or revision
//! generation arena owns complete list rows, and callers resolve coordinates
//! only while borrowing that arena. Lifetime transitions rebrand coordinates
//! while the generation-owned row stays in place.

use ahash::RandomState;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicU64, Ordering};
use smallvec::SmallVec;
use std::hash::{BuildHasher, Hash, Hasher};
use std::rc::Rc;

use crate::durable_arena::{GlueId, TokenListId};
use crate::glue::GlueSpec;
use crate::memory_accounting::MemoryAccounting;
use crate::node::{Node, NodeTokenList};

#[cfg(test)]
#[path = "node_arena/tests.rs"]
mod tests;

static NEXT_ARENA_OWNER: AtomicU64 = AtomicU64::new(1);
static NEXT_LIST_GENERATION: AtomicU64 = AtomicU64::new(1);
const LIST_GENERATION_STRIDE: u64 = 1_u64 << 32;

fn next_list_generation_namespace() -> u64 {
    NEXT_LIST_GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(LIST_GENERATION_STRIDE)
        })
        .expect("node-list generation namespace exhausted")
}

fn semantic_sequence_identity<'a, T: Hash + 'a>(values: impl IntoIterator<Item = &'a T>) -> u64 {
    let state = RandomState::with_seeds(
        0x756d_6265_725f_6e6f,
        0x6465_5f73_656d_616e,
        0x7469_635f_7631_5f66,
        0x6978_6564_5f73_6565,
    );
    let mut sequence = crate::node_sequence::SemanticSequenceIdentity::empty();
    for value in values {
        let mut hasher = state.build_hasher();
        hasher.write(b"umber-node-semantic-identity-v1");
        value.hash(&mut hasher);
        sequence.push_back(hasher.finish());
    }
    match sequence.raw() {
        0 => u64::MAX,
        value => value,
    }
}

/// Operation-local node-list lifetime.
pub enum ScratchLifetime {}

/// Open-mode and page-builder node-list lifetime.
pub enum PageLifetime {}

/// Copyable coordinate of one immutable list row in a matching arena.
///
/// Row zero is the canonical empty list and resolves without storage. The
/// constructor stays private so a coordinate can be obtained only by arena
/// publication or typed relocation.
pub struct NodeListId<L> {
    owner: u64,
    row: u32,
    generation: u64,
    semantic_identity: u64,
    _lifetime: PhantomData<fn(&L) -> &L>,
}

impl<L> NodeListId<L> {
    const EMPTY: Self = Self {
        owner: 0,
        row: 0,
        generation: 0,
        semantic_identity: 0,
        _lifetime: PhantomData,
    };

    const fn from_row(owner: u64, row: u32, generation: u64, semantic_identity: u64) -> Self {
        Self {
            owner,
            row,
            generation,
            semantic_identity,
            _lifetime: PhantomData,
        }
    }

    pub(crate) const fn rebrand<D>(self) -> NodeListId<D> {
        NodeListId::from_row(
            self.owner,
            self.row,
            self.generation,
            self.semantic_identity,
        )
    }

    const fn index(self) -> Option<usize> {
        if self.row == 0 {
            None
        } else {
            Some(self.row as usize - 1)
        }
    }

    /// Returns the canonical empty-list coordinate.
    #[must_use]
    pub const fn empty() -> Self {
        Self::EMPTY
    }

    pub(crate) const fn format_validation_coordinate(row: u32) -> Self {
        if row == 0 {
            Self::EMPTY
        } else {
            Self::from_row(1, row, 1, row as u64)
        }
    }

    /// Whether this coordinate names the canonical empty list.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.row == 0
    }

    pub(crate) const fn semantic_identity(self) -> Option<u64> {
        if self.row == 0 {
            Some(0)
        } else {
            match self.semantic_identity {
                0 => None,
                identity => Some(identity),
            }
        }
    }
}

impl<L> Clone for NodeListId<L> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<L> Copy for NodeListId<L> {}

impl<L> core::fmt::Debug for NodeListId<L> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NodeListId(..)")
    }
}

impl<L> PartialEq for NodeListId<L> {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner && self.row == other.row && self.generation == other.generation
    }
}

impl<L> Eq for NodeListId<L> {}

impl<L> core::hash::Hash for NodeListId<L> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        if self.semantic_identity == 0 {
            self.owner.hash(state);
            self.row.hash(state);
            self.generation.hash(state);
        } else {
            self.semantic_identity.hash(state);
        }
    }
}

/// List coordinate used by operation scratch.
pub type ScratchListId = NodeListId<ScratchLifetime>;

/// List coordinate used by open modes and the page builder.
pub type PageListId = NodeListId<PageLifetime>;

/// List coordinate retained by one revision generation.
pub type DurableListId<G> = NodeListId<G>;

/// Owner-checked suffix watermark for one node arena.
pub struct NodeArenaCursor<L> {
    owner: u64,
    rows: u32,
    next_generation: u64,
    _lifetime: PhantomData<fn(&L) -> &L>,
}

/// Consuming owner of one nested allocation suffix.
///
/// Unlike a rollback cursor, this value is deliberately neither `Clone` nor
/// `Copy`: the structural owner which opened the region must either transfer
/// it to an enclosing publication or consume it when every coordinate in the
/// suffix has crossed its final lifetime boundary.
pub struct NodeArenaRegion<L> {
    cursor: NodeArenaCursor<L>,
}

impl<L> core::fmt::Debug for NodeArenaRegion<L> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NodeArenaRegion(..)")
    }
}

impl<L> Clone for NodeArenaCursor<L> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<L> Copy for NodeArenaCursor<L> {}

impl<L> core::fmt::Debug for NodeArenaCursor<L> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NodeArenaCursor")
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

/// Invalid publication, resolution, relocation, or rollback coordinate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeArenaError {
    CapacityOverflow,
    AllocationFailed,
    ForeignCursor,
    CursorBeyondEnd,
    InvalidList,
    CyclicList,
}

#[derive(Default)]
pub(crate) struct StampedIndexMap {
    keys: Vec<usize>,
    stamps: Vec<u64>,
    states: Vec<u8>,
    stamp: u64,
    len: usize,
}

impl StampedIndexMap {
    pub(crate) fn begin(&mut self) {
        self.stamp = self
            .stamp
            .checked_add(1)
            .expect("memory traversal stamp exhausted");
        self.len = 0;
    }

    #[cfg(test)]
    pub(crate) fn mark(&mut self, key: usize) -> bool {
        if self.state(key) != 0 {
            return false;
        }
        self.set_state(key, 1);
        true
    }

    fn state(&self, key: usize) -> u8 {
        let Some(mut slot) = self.initial_slot(key) else {
            return 0;
        };
        loop {
            if self.stamps[slot] != self.stamp {
                return 0;
            }
            if self.keys[slot] == key {
                return self.states[slot];
            }
            slot = (slot + 1) & (self.keys.len() - 1);
        }
    }

    fn set_state(&mut self, key: usize, state: u8) {
        debug_assert_ne!(state, 0);
        if self.keys.is_empty() || self.len.saturating_add(1) * 2 > self.keys.len() {
            self.grow();
        }
        let mut slot = self.initial_slot(key).expect("grown mark table");
        loop {
            if self.stamps[slot] != self.stamp {
                self.keys[slot] = key;
                self.states[slot] = state;
                self.stamps[slot] = self.stamp;
                self.len += 1;
                return;
            }
            if self.keys[slot] == key {
                self.states[slot] = state;
                return;
            }
            slot = (slot + 1) & (self.keys.len() - 1);
        }
    }

    fn grow(&mut self) {
        let old_keys = core::mem::take(&mut self.keys);
        let old_stamps = core::mem::take(&mut self.stamps);
        let old_states = core::mem::take(&mut self.states);
        let old_stamp = self.stamp;
        let old_len = self.len;
        let capacity = old_keys.len().max(8).saturating_mul(2);
        self.keys = vec![0; capacity];
        self.stamps = vec![0; capacity];
        self.states = vec![0; capacity];
        self.len = 0;
        for slot in 0..old_keys.len() {
            if old_stamps[slot] == old_stamp {
                self.set_state(old_keys[slot], old_states[slot]);
            }
        }
        debug_assert_eq!(self.len, old_len);
    }

    fn initial_slot(&self, key: usize) -> Option<usize> {
        if self.keys.is_empty() {
            return None;
        }
        let mut hash = key;
        hash ^= hash >> 16;
        hash = hash.wrapping_mul(0x7feb_352d);
        hash ^= hash >> 15;
        Some(hash & (self.keys.len() - 1))
    }

    #[cfg(test)]
    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    #[cfg(test)]
    pub(crate) const fn capacity(&self) -> usize {
        self.keys.len()
    }
}

struct StampedRelocationMap<L> {
    keys: Vec<usize>,
    stamps: Vec<u64>,
    values: Vec<NodeListId<L>>,
    stamp: u64,
    len: usize,
}

impl<L> Default for StampedRelocationMap<L> {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            stamps: Vec::new(),
            values: Vec::new(),
            stamp: 0,
            len: 0,
        }
    }
}

impl<L> StampedRelocationMap<L> {
    fn begin(&mut self) {
        self.stamp = self
            .stamp
            .checked_add(1)
            .expect("relocation stamp exhausted");
        self.len = 0;
    }

    fn get(&self, key: usize) -> Option<NodeListId<L>> {
        let mut slot = self.initial_slot(key)?;
        loop {
            if self.stamps[slot] != self.stamp {
                return None;
            }
            if self.keys[slot] == key {
                return Some(self.values[slot]);
            }
            slot = (slot + 1) & (self.keys.len() - 1);
        }
    }

    fn insert(&mut self, key: usize, value: NodeListId<L>) {
        if self.keys.is_empty() || self.len.saturating_add(1) * 2 > self.keys.len() {
            self.grow();
        }
        let mut slot = self.initial_slot(key).expect("grown relocation table");
        loop {
            if self.stamps[slot] != self.stamp {
                self.keys[slot] = key;
                self.values[slot] = value;
                self.stamps[slot] = self.stamp;
                self.len += 1;
                return;
            }
            if self.keys[slot] == key {
                self.values[slot] = value;
                return;
            }
            slot = (slot + 1) & (self.keys.len() - 1);
        }
    }

    fn grow(&mut self) {
        let old_keys = core::mem::take(&mut self.keys);
        let old_stamps = core::mem::take(&mut self.stamps);
        let old_values = core::mem::take(&mut self.values);
        let old_stamp = self.stamp;
        let old_len = self.len;
        let capacity = old_keys.len().max(8).saturating_mul(2);
        self.keys = vec![0; capacity];
        self.stamps = vec![0; capacity];
        self.values = vec![NodeListId::empty(); capacity];
        self.len = 0;
        for slot in 0..old_keys.len() {
            if old_stamps[slot] == old_stamp {
                self.insert(old_keys[slot], old_values[slot]);
            }
        }
        debug_assert_eq!(self.len, old_len);
    }

    fn initial_slot(&self, key: usize) -> Option<usize> {
        if self.keys.is_empty() {
            return None;
        }
        let mut hash = key;
        hash ^= hash >> 16;
        hash = hash.wrapping_mul(0x7feb_352d);
        hash ^= hash >> 15;
        Some(hash & (self.keys.len() - 1))
    }
}

pub(crate) struct NodeRelocationScratch<Source, Destination> {
    marks: StampedIndexMap,
    order: Vec<NodeListId<Source>>,
    relocation: StampedRelocationMap<Destination>,
}

impl<Source, Destination> Default for NodeRelocationScratch<Source, Destination> {
    fn default() -> Self {
        Self {
            marks: StampedIndexMap::default(),
            order: Vec::new(),
            relocation: StampedRelocationMap::default(),
        }
    }
}

impl<Source, Destination> NodeRelocationScratch<Source, Destination> {
    fn begin(&mut self) {
        self.marks.begin();
        self.order.clear();
        self.relocation.begin();
    }

    #[cfg(test)]
    fn capacities(&self) -> (usize, usize, usize) {
        (
            self.marks.keys.len(),
            self.order.capacity(),
            self.relocation.keys.len(),
        )
    }
}

pub(crate) struct NodeMemoryScratch<L> {
    marks: StampedIndexMap,
    semantic_order: Vec<NodeListId<L>>,
    traversal_order: Vec<NodeListId<L>>,
    diagnostics: Vec<(NodeListId<L>, u32)>,
}

impl<L> Default for NodeMemoryScratch<L> {
    fn default() -> Self {
        Self {
            marks: StampedIndexMap::default(),
            semantic_order: Vec::new(),
            traversal_order: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

impl<L> NodeMemoryScratch<L> {
    fn next_marks(&mut self) {
        self.marks.begin();
    }

    fn finish(&mut self) {
        self.semantic_order.clear();
        self.traversal_order.clear();
        self.diagnostics.clear();
    }
}

/// One semantic-lifetime owner for immutable node lists.
///
/// Each row owns its node payload directly. Nested lists are copy-only typed
/// coordinates back into this same arena; there is no payload owner, root set,
/// registry lookup, or reference count on a row.
pub struct NodeArena<L, Glue = GlueSpec, Tokens = NodeTokenList> {
    owner: u64,
    next_generation: u64,
    rows: Vec<Option<NodeArenaRow<L, Glue, Tokens>>>,
    segments: Vec<Option<Rc<NodeArenaSegment<L, Glue, Tokens>>>>,
    segment_live_rows: Vec<u32>,
    semantic_identity_enabled: bool,
    accounting: MemoryAccounting,
}

struct NodeArenaRow<L, Glue, Tokens> {
    generation: u64,
    segment: u32,
    offset: u32,
    semantic_identity: u64,
    _lifetime: PhantomData<fn(&L, &Glue, &Tokens)>,
}

const NODE_ARENA_SEGMENT_ROWS: usize = 64;

struct NodeArenaSegment<L, Glue, Tokens> {
    rows: Vec<Option<NodeArenaAllocation<L, Glue, Tokens>>>,
}

struct NodeArenaAllocation<L, Glue, Tokens> {
    nodes: Box<[Node<NodeListId<L>, Glue, Tokens>]>,
    tex82_words: (usize, usize),
    etex_words: (usize, usize),
    accounting: MemoryAccounting,
}

impl<L, Glue, Tokens> Drop for NodeArenaAllocation<L, Glue, Tokens> {
    fn drop(&mut self) {
        self.accounting
            .release_nodes(self.tex82_words, self.etex_words);
    }
}

impl<L, Glue, Tokens> Clone for NodeArenaRow<L, Glue, Tokens> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation,
            segment: self.segment,
            offset: self.offset,
            semantic_identity: self.semantic_identity,
            _lifetime: PhantomData,
        }
    }
}

impl<L, Glue, Tokens> NodeArena<L, Glue, Tokens> {
    /// Forks the immutable published rows while preserving their stable
    /// coordinates. The accepted arena is never mutated; subsequent
    /// publication is confined to the destination arena.
    pub(crate) fn fork(&self) -> Self {
        #[cfg(feature = "profiling")]
        crate::measurement::record_node_checkpoint_share(self.rows.iter().flatten().count());
        let mut rows = Vec::with_capacity(self.rows.len().saturating_add(NODE_ARENA_SEGMENT_ROWS));
        rows.extend(self.rows.iter().cloned());
        let mut segments = Vec::with_capacity(self.segments.len().saturating_add(1));
        segments.extend(self.segments.iter().cloned());
        let mut segment_live_rows =
            Vec::with_capacity(self.segment_live_rows.len().saturating_add(1));
        segment_live_rows.extend_from_slice(&self.segment_live_rows);
        Self {
            owner: self.owner,
            next_generation: next_list_generation_namespace(),
            rows,
            segments,
            segment_live_rows,
            semantic_identity_enabled: self.semantic_identity_enabled,
            accounting: self.accounting.clone(),
        }
    }
}

impl<L, Glue, Tokens> core::fmt::Debug for NodeArena<L, Glue, Tokens> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NodeArena")
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}

impl<L, Glue, Tokens> Default for NodeArena<L, Glue, Tokens> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L, Glue, Tokens> NodeArena<L, Glue, Tokens> {
    fn allocation(&self, index: usize) -> Option<&NodeArenaAllocation<L, Glue, Tokens>> {
        let row = self.rows.get(index)?.as_ref()?;
        self.segments
            .get(row.segment as usize)?
            .as_ref()?
            .rows
            .get(row.offset as usize)?
            .as_ref()
    }

    fn append_allocation(
        &mut self,
        generation: u64,
        semantic_identity: u64,
        allocation: NodeArenaAllocation<L, Glue, Tokens>,
    ) -> Result<(), NodeArenaError> {
        let can_append = self
            .segments
            .last()
            .and_then(Option::as_ref)
            .is_some_and(|segment| {
                Rc::strong_count(segment) == 1 && segment.rows.len() < NODE_ARENA_SEGMENT_ROWS
            });
        if !can_append {
            self.segments
                .try_reserve(1)
                .map_err(|_| NodeArenaError::AllocationFailed)?;
            self.segment_live_rows
                .try_reserve(1)
                .map_err(|_| NodeArenaError::AllocationFailed)?;
            self.segments
                .push(Some(Rc::new(NodeArenaSegment { rows: Vec::new() })));
            self.segment_live_rows.push(0);
        }
        let segment_index = self.segments.len() - 1;
        let segment = Rc::get_mut(
            self.segments[segment_index]
                .as_mut()
                .expect("new node segment is present"),
        )
        .expect("appendable node segment is uniquely owned");
        segment
            .rows
            .try_reserve(1)
            .map_err(|_| NodeArenaError::AllocationFailed)?;
        let offset =
            u32::try_from(segment.rows.len()).map_err(|_| NodeArenaError::CapacityOverflow)?;
        segment.rows.push(Some(allocation));
        self.segment_live_rows[segment_index] += 1;
        self.rows.push(Some(NodeArenaRow {
            generation,
            segment: u32::try_from(segment_index).map_err(|_| NodeArenaError::CapacityOverflow)?,
            offset,
            semantic_identity,
            _lifetime: PhantomData,
        }));
        Ok(())
    }

    fn release_row(&mut self, index: usize) {
        let Some(row) = self.rows[index].take() else {
            return;
        };
        let segment_index = row.segment as usize;
        let live = &mut self.segment_live_rows[segment_index];
        *live -= 1;
        if *live == 0 {
            self.segments[segment_index] = None;
        } else if let Some(segment) = self.segments[segment_index].as_mut().and_then(Rc::get_mut) {
            let _ = segment.rows[row.offset as usize].take();
        }
    }

    fn trim_empty_tail_segments(&mut self) {
        while self.segments.last().is_some_and(Option::is_none) {
            self.segments.pop();
            self.segment_live_rows.pop();
        }
    }

    pub(crate) fn semantic_memory_usage(
        &self,
        root: NodeListId<L>,
        etex_node_sizes: bool,
        scratch: &mut NodeMemoryScratch<L>,
        mut visit_tokens: impl FnMut(&Tokens),
    ) -> Result<(usize, usize, usize), NodeArenaError> {
        scratch.semantic_order.clear();
        scratch.diagnostics.clear();
        scratch.next_marks();
        let result = (|| {
            self.postorder_marked(root, &mut scratch.marks, &mut scratch.semantic_order, true)?;
            let mut variable = 0_usize;
            let mut dynamic = 0_usize;
            for offset in 0..scratch.semantic_order.len() {
                let list = scratch.semantic_order[offset];
                let index = list.index().expect("postorder excludes empty lists");
                for node in self
                    .allocation(index)
                    .expect("postorder contains only live rows")
                    .nodes
                    .iter()
                {
                    let (node_variable, node_dynamic) = node.tex_memory_words(etex_node_sizes);
                    variable = variable.saturating_add(node_variable);
                    dynamic = dynamic.saturating_add(node_dynamic);
                    node.visit_payloads(|_| {}, |tokens| visit_tokens(tokens));
                    node.visit_diagnostic_node_lists(|child, overlap| {
                        scratch.diagnostics.push((*child, overlap));
                    });
                }
            }
            let mut diagnostic_extent = 0_usize;
            for offset in 0..scratch.diagnostics.len() {
                let (child, overlap) = scratch.diagnostics[offset];
                scratch.traversal_order.clear();
                scratch.next_marks();
                self.postorder_marked(
                    child,
                    &mut scratch.marks,
                    &mut scratch.traversal_order,
                    false,
                )?;
                let child_dynamic = scratch
                    .traversal_order
                    .iter()
                    .copied()
                    .flat_map(|list| {
                        self.allocation(list.index().expect("postorder excludes empty lists"))
                            .expect("postorder contains only live rows")
                            .nodes
                            .iter()
                    })
                    .fold(0_usize, |words, node| {
                        words.saturating_add(node.tex_memory_words(etex_node_sizes).1)
                    });
                diagnostic_extent =
                    diagnostic_extent.max(child_dynamic.saturating_sub(overlap as usize));
            }
            Ok((variable, dynamic, diagnostic_extent))
        })();
        scratch.finish();
        result
    }

    fn postorder_marked(
        &self,
        id: NodeListId<L>,
        marks: &mut StampedIndexMap,
        order: &mut Vec<NodeListId<L>>,
        semantic: bool,
    ) -> Result<(), NodeArenaError> {
        let Some(index) = id.index() else {
            return Ok(());
        };
        if id.owner != self.owner {
            return Err(NodeArenaError::InvalidList);
        }
        let Some(row) = self.rows.get(index).and_then(Option::as_ref) else {
            return Err(NodeArenaError::InvalidList);
        };
        if row.generation != id.generation {
            return Err(NodeArenaError::InvalidList);
        }
        match marks.state(index) {
            2 => return Ok(()),
            1 => return Err(NodeArenaError::CyclicList),
            _ => {}
        }
        marks.set_state(index, 1);
        for node in self
            .allocation(index)
            .expect("validated row retains its allocation")
            .nodes
            .iter()
        {
            let mut result = Ok(());
            let mut visit = |child: &NodeListId<L>| {
                if result.is_ok() {
                    result = self.postorder_marked(*child, marks, order, semantic);
                }
            };
            if semantic {
                node.visit_semantic_node_lists(&mut visit);
            } else {
                node.visit_node_lists(&mut visit);
            }
            result?;
        }
        marks.set_state(index, 2);
        order.push(id);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn semantic_closure_tex_memory_words(
        &self,
        root: NodeListId<L>,
        etex_node_sizes: bool,
    ) -> Result<(usize, usize), NodeArenaError> {
        let mut state = vec![0_u8; self.rows.len()];
        let mut closure = Vec::new();
        self.semantic_postorder(root, &mut state, &mut closure)?;
        Ok(closure
            .into_iter()
            .flat_map(|list| {
                self.allocation(list.index().expect("postorder excludes empty lists"))
                    .expect("postorder contains only live rows")
                    .nodes
                    .iter()
            })
            .fold((0_usize, 0_usize), |words, node| {
                let node_words = node.tex_memory_words(etex_node_sizes);
                (
                    words.0.saturating_add(node_words.0),
                    words.1.saturating_add(node_words.1),
                )
            }))
    }

    /// Creates an empty lifetime owner.
    #[must_use]
    pub fn new() -> Self {
        Self::with_memory_accounting(MemoryAccounting::default())
    }

    pub(crate) fn with_memory_accounting(accounting: MemoryAccounting) -> Self {
        let owner = NEXT_ARENA_OWNER.fetch_add(1, Ordering::Relaxed);
        assert_ne!(owner, 0, "node-arena owner identity exhausted");
        Self {
            owner,
            next_generation: next_list_generation_namespace(),
            rows: Vec::new(),
            segments: Vec::new(),
            segment_live_rows: Vec::new(),
            semantic_identity_enabled: false,
            accounting,
        }
    }

    pub(crate) fn enable_semantic_identity(&mut self) {
        assert!(
            self.rows.is_empty() || self.semantic_identity_enabled,
            "node semantic identity must be selected before publication"
        );
        self.semantic_identity_enabled = true;
    }

    /// Captures the suffix position after canonical roots have been recorded.
    #[must_use]
    pub fn cursor(&self) -> NodeArenaCursor<L> {
        NodeArenaCursor {
            owner: self.owner,
            rows: u32::try_from(self.rows.len()).expect("node arena exceeds u32 rows"),
            next_generation: self.next_generation,
            _lifetime: PhantomData,
        }
    }

    #[must_use]
    pub(crate) fn cursor_is_head(&self, cursor: NodeArenaCursor<L>) -> bool {
        cursor.owner == self.owner && cursor.rows as usize == self.rows.len()
    }

    /// Opens one nested suffix whose coordinates have one structural owner.
    #[must_use]
    pub fn begin_region(&self) -> NodeArenaRegion<L> {
        NodeArenaRegion {
            cursor: self.cursor(),
        }
    }

    /// Drops a complete structural suffix after its owner has published every
    /// surviving value into another lifetime.
    pub fn release_region(&mut self, region: NodeArenaRegion<L>) -> Result<(), NodeArenaError> {
        self.truncate(region.cursor)
    }

    /// Consumes a nested token while retaining its suffix under the enclosing
    /// arena owner because rollback can restore a root into that suffix.
    pub fn retain_region(&self, region: NodeArenaRegion<L>) -> Result<(), NodeArenaError> {
        self.validate_cursor(region.cursor)
    }

    /// Validates a cursor without mutation.
    pub fn validate_cursor(&self, cursor: NodeArenaCursor<L>) -> Result<(), NodeArenaError> {
        if cursor.owner != self.owner {
            return Err(NodeArenaError::ForeignCursor);
        }
        if cursor.rows as usize > self.rows.len() {
            return Err(NodeArenaError::CursorBeyondEnd);
        }
        Ok(())
    }

    pub(crate) fn later_cursor(
        &self,
        left: NodeArenaCursor<L>,
        right: NodeArenaCursor<L>,
    ) -> Result<NodeArenaCursor<L>, NodeArenaError> {
        self.validate_cursor(left)?;
        self.validate_cursor(right)?;
        Ok(if left.rows >= right.rows { left } else { right })
    }

    /// Validates every font coordinate retained in the cursor's immutable
    /// prefix before a rollback can discard a font-store suffix.
    pub(crate) fn font_roots_are_live(
        &self,
        cursor: NodeArenaCursor<L>,
        mut is_live: impl FnMut(crate::ids::FontId) -> bool,
    ) -> Result<bool, NodeArenaError> {
        self.validate_cursor(cursor)?;
        for index in 0..cursor.rows as usize {
            let Some(allocation) = self.allocation(index) else {
                continue;
            };
            for node in allocation.nodes.iter() {
                let mut live = true;
                node.visit_fonts(|font| live &= is_live(font));
                if !live {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Truncates a rejected suffix after callers restore every canonical root.
    pub fn truncate(&mut self, cursor: NodeArenaCursor<L>) -> Result<(), NodeArenaError> {
        self.validate_cursor(cursor)?;
        for index in (cursor.rows as usize..self.rows.len()).rev() {
            self.release_row(index);
        }
        self.rows.truncate(cursor.rows as usize);
        self.trim_empty_tail_segments();
        Ok(())
    }

    pub(crate) fn restore_checkpoint_cursor(
        &mut self,
        cursor: NodeArenaCursor<L>,
    ) -> Result<(), NodeArenaError> {
        self.truncate(cursor)?;
        self.next_generation = cursor.next_generation;
        Ok(())
    }

    /// Publishes one complete list whose children already belong to this arena.
    pub fn publish(
        &mut self,
        nodes: Vec<Node<NodeListId<L>, Glue, Tokens>>,
    ) -> Result<NodeListId<L>, NodeArenaError>
    where
        Glue: Hash,
        Tokens: Hash,
    {
        if nodes.is_empty() {
            return Ok(NodeListId::empty());
        }
        #[cfg(feature = "profiling")]
        crate::measurement::record_node_publication(nodes.len());
        for node in &nodes {
            let mut valid = true;
            node.visit_node_lists(|child| valid &= self.contains(*child));
            if !valid {
                return Err(NodeArenaError::InvalidList);
            }
        }
        let next = self
            .rows
            .len()
            .checked_add(1)
            .and_then(|row| u32::try_from(row).ok())
            .ok_or(NodeArenaError::CapacityOverflow)?;
        self.rows
            .try_reserve(1)
            .map_err(|_| NodeArenaError::AllocationFailed)?;
        let generation = self.next_generation;
        self.next_generation = generation
            .checked_add(1)
            .expect("node-list generation exhausted");
        let tex82_words = node_words(&nodes, false);
        let etex_words = node_words(&nodes, true);
        self.accounting.allocate_nodes(tex82_words, etex_words);
        let semantic_identity = if self.semantic_identity_enabled {
            semantic_sequence_identity(nodes.iter())
        } else {
            0
        };
        self.append_allocation(
            generation,
            semantic_identity,
            NodeArenaAllocation {
                nodes: nodes.into_boxed_slice(),
                tex82_words,
                etex_words,
                accounting: self.accounting.clone(),
            },
        )?;
        Ok(NodeListId::from_row(
            self.owner,
            next,
            generation,
            semantic_identity,
        ))
    }

    /// Borrows one complete list.
    pub fn get(&self, id: NodeListId<L>) -> Result<NodeList<'_, L, Glue, Tokens>, NodeArenaError> {
        if !self.contains(id) {
            return Err(NodeArenaError::InvalidList);
        }
        Ok(NodeList { arena: self, id })
    }

    /// Whether a coordinate resolves directly in this owner.
    #[must_use]
    pub fn contains(&self, id: NodeListId<L>) -> bool {
        id.index().is_none_or(|index| {
            id.owner == self.owner
                && self
                    .rows
                    .get(index)
                    .and_then(Option::as_ref)
                    .is_some_and(|row| row.generation == id.generation)
        })
    }

    /// Copies only the closures reachable from explicit roots into another
    /// lifetime, rewriting every child through a dense relocation vector.
    ///
    /// Staging completes before destination publication. Invalid coordinates
    /// or cycles leave the destination unchanged.
    pub fn promote_into<D>(
        &self,
        roots: &[NodeListId<L>],
        destination: &mut NodeArena<D, Glue, Tokens>,
    ) -> Result<Vec<NodeListId<D>>, NodeArenaError>
    where
        Glue: Clone + Hash,
        Tokens: Clone + Hash,
    {
        self.promote_into_with(
            roots,
            destination,
            core::convert::identity,
            core::convert::identity,
        )
    }

    /// Collects semantic payload roots in the same deterministic order used
    /// by relocation, without scanning unrelated arena rows.
    pub fn escaping_payloads(
        &self,
        roots: &[NodeListId<L>],
    ) -> Result<(Vec<Glue>, Vec<Tokens>), NodeArenaError>
    where
        Glue: Clone,
        Tokens: Clone,
    {
        let mut state = vec![0_u8; self.rows.len()];
        let mut order = Vec::new();
        for root in roots {
            self.postorder(*root, &mut state, &mut order)?;
        }
        let mut glue = Vec::new();
        let mut tokens = Vec::new();
        for list in order {
            let index = list.index().expect("postorder excludes empty lists");
            for node in self
                .allocation(index)
                .map(|allocation| allocation.nodes.as_ref())
                .expect("postorder contains only live rows")
            {
                node.visit_payloads(
                    |value| glue.push(value.clone()),
                    |value| tokens.push(value.clone()),
                );
            }
        }
        Ok((glue, tokens))
    }

    /// Validates the exact closure and reserves its destination rows before
    /// any accompanying durable payload batch is published.
    pub fn reserve_promotion<D, OtherGlue, OtherTokens>(
        &self,
        roots: &[NodeListId<L>],
        destination: &mut NodeArena<D, OtherGlue, OtherTokens>,
    ) -> Result<(), NodeArenaError> {
        let mut state = vec![0_u8; self.rows.len()];
        let mut order = Vec::new();
        for root in roots {
            self.postorder(*root, &mut state, &mut order)?;
        }
        let final_len = destination
            .rows
            .len()
            .checked_add(order.len())
            .ok_or(NodeArenaError::CapacityOverflow)?;
        u32::try_from(final_len).map_err(|_| NodeArenaError::CapacityOverflow)?;
        destination
            .rows
            .try_reserve(order.len())
            .map_err(|_| NodeArenaError::AllocationFailed)
    }

    /// Relocates an exact closure while changing its semantic payload
    /// coordinates, as page values become generation-durable ids.
    pub fn promote_into_with<D, OtherGlue, OtherTokens>(
        &self,
        roots: &[NodeListId<L>],
        destination: &mut NodeArena<D, OtherGlue, OtherTokens>,
        mut map_glue: impl FnMut(Glue) -> OtherGlue,
        mut map_tokens: impl FnMut(Tokens) -> OtherTokens,
    ) -> Result<Vec<NodeListId<D>>, NodeArenaError>
    where
        Glue: Clone,
        Tokens: Clone,
        OtherGlue: Hash,
        OtherTokens: Hash,
    {
        let mut scratch = NodeRelocationScratch::default();
        Ok(self
            .promote_into_with_scratch(
                roots,
                destination,
                &mut scratch,
                &mut map_glue,
                &mut map_tokens,
            )?
            .into_vec())
    }

    pub(crate) fn promote_into_with_scratch<D, OtherGlue, OtherTokens>(
        &self,
        roots: &[NodeListId<L>],
        destination: &mut NodeArena<D, OtherGlue, OtherTokens>,
        scratch: &mut NodeRelocationScratch<L, D>,
        mut map_glue: impl FnMut(Glue) -> OtherGlue,
        mut map_tokens: impl FnMut(Tokens) -> OtherTokens,
    ) -> Result<SmallVec<[NodeListId<D>; 1]>, NodeArenaError>
    where
        Glue: Clone,
        Tokens: Clone,
        OtherGlue: Hash,
        OtherTokens: Hash,
    {
        scratch.begin();
        for root in roots {
            self.postorder_sparse(*root, &mut scratch.marks, &mut scratch.order)?;
        }
        let destination_base = destination.rows.len();
        let final_len = destination_base
            .checked_add(scratch.order.len())
            .ok_or(NodeArenaError::CapacityOverflow)?;
        u32::try_from(final_len).map_err(|_| NodeArenaError::CapacityOverflow)?;
        destination
            .rows
            .try_reserve(scratch.order.len())
            .map_err(|_| NodeArenaError::AllocationFailed)?;

        for source in scratch.order.iter().copied() {
            let source_index = source.index().expect("postorder excludes empty lists");
            let row = destination_base
                .checked_add(destination.rows.len() - destination_base)
                .and_then(|index| index.checked_add(1))
                .and_then(|row| u32::try_from(row).ok())
                .ok_or(NodeArenaError::CapacityOverflow)?;
            let generation = destination.next_generation;
            destination.next_generation = generation
                .checked_add(1)
                .expect("node-list generation exhausted");
            let nodes = self
                .allocation(source_index)
                .map(|allocation| allocation.nodes.as_ref())
                .expect("postorder contains only live rows")
                .iter()
                .cloned()
                .map(|node| {
                    node.map_lists(|child| {
                        child.index().map_or_else(NodeListId::empty, |index| {
                            scratch
                                .relocation
                                .get(index)
                                .expect("postorder relocates children before parents")
                        })
                    })
                    .map_payloads(&mut map_glue, &mut map_tokens)
                })
                .collect::<Vec<_>>()
                .into_boxed_slice();
            let semantic_identity = if destination.semantic_identity_enabled {
                semantic_sequence_identity(nodes.iter())
            } else {
                0
            };
            let destination_id =
                NodeListId::from_row(destination.owner, row, generation, semantic_identity);
            #[cfg(feature = "profiling")]
            crate::measurement::record_node_external_materialization(nodes.len());
            scratch.relocation.insert(source_index, destination_id);
            let tex82_words = node_words(&nodes, false);
            let etex_words = node_words(&nodes, true);
            destination
                .accounting
                .allocate_nodes(tex82_words, etex_words);
            destination.append_allocation(
                generation,
                semantic_identity,
                NodeArenaAllocation {
                    nodes,
                    tex82_words,
                    etex_words,
                    accounting: destination.accounting.clone(),
                },
            )?;
        }

        let promoted_roots = roots
            .iter()
            .map(|root| {
                root.index().map_or_else(NodeListId::empty, |index| {
                    scratch
                        .relocation
                        .get(index)
                        .expect("every explicit root was relocated")
                })
            })
            .collect::<SmallVec<_>>();
        scratch.order.clear();
        Ok(promoted_roots)
    }

    fn postorder_sparse(
        &self,
        id: NodeListId<L>,
        state: &mut StampedIndexMap,
        order: &mut Vec<NodeListId<L>>,
    ) -> Result<(), NodeArenaError> {
        let Some(index) = id.index() else {
            return Ok(());
        };
        if id.owner != self.owner {
            return Err(NodeArenaError::InvalidList);
        }
        let Some(row) = self.rows.get(index).and_then(Option::as_ref) else {
            return Err(NodeArenaError::InvalidList);
        };
        if row.generation != id.generation {
            return Err(NodeArenaError::InvalidList);
        }
        match state.state(index) {
            2 => return Ok(()),
            1 => return Err(NodeArenaError::CyclicList),
            _ => {}
        }
        state.set_state(index, 1);
        for node in self
            .allocation(index)
            .expect("validated row retains its allocation")
            .nodes
            .iter()
        {
            let mut result = Ok(());
            node.visit_node_lists(|child| {
                if result.is_ok() {
                    result = self.postorder_sparse(*child, state, order);
                }
            });
            result?;
        }
        state.set_state(index, 2);
        order.push(id);
        Ok(())
    }

    fn postorder(
        &self,
        id: NodeListId<L>,
        state: &mut [u8],
        order: &mut Vec<NodeListId<L>>,
    ) -> Result<(), NodeArenaError> {
        let Some(index) = id.index() else {
            return Ok(());
        };
        if id.owner != self.owner {
            return Err(NodeArenaError::InvalidList);
        }
        let Some(row) = self.rows.get(index).and_then(Option::as_ref) else {
            return Err(NodeArenaError::InvalidList);
        };
        if row.generation != id.generation {
            return Err(NodeArenaError::InvalidList);
        }
        match state[index] {
            2 => return Ok(()),
            1 => return Err(NodeArenaError::CyclicList),
            _ => {}
        }
        state[index] = 1;
        for node in self
            .allocation(index)
            .expect("validated row retains its allocation")
            .nodes
            .iter()
        {
            let mut result = Ok(());
            node.visit_node_lists(|child| {
                if result.is_ok() {
                    result = self.postorder(*child, state, order);
                }
            });
            result?;
        }
        state[index] = 2;
        order.push(id);
        Ok(())
    }

    #[cfg(test)]
    fn semantic_postorder(
        &self,
        id: NodeListId<L>,
        state: &mut [u8],
        order: &mut Vec<NodeListId<L>>,
    ) -> Result<(), NodeArenaError> {
        let Some(index) = id.index() else {
            return Ok(());
        };
        if id.owner != self.owner {
            return Err(NodeArenaError::InvalidList);
        }
        let Some(row) = self.rows.get(index).and_then(Option::as_ref) else {
            return Err(NodeArenaError::InvalidList);
        };
        if row.generation != id.generation {
            return Err(NodeArenaError::InvalidList);
        }
        match state[index] {
            2 => return Ok(()),
            1 => return Err(NodeArenaError::CyclicList),
            _ => {}
        }
        state[index] = 1;
        for node in self
            .allocation(index)
            .expect("validated row retains its allocation")
            .nodes
            .iter()
        {
            let mut result = Ok(());
            node.visit_semantic_node_lists(|child| {
                if result.is_ok() {
                    result = self.semantic_postorder(*child, state, order);
                }
            });
            result?;
        }
        state[index] = 2;
        order.push(id);
        Ok(())
    }

    /// Drops the exact closure rooted at an exclusively owned completed page.
    ///
    /// The caller must first remove the canonical page root. Unrelated rows
    /// keep their coordinates and storage; released coordinates fail future
    /// resolution instead of becoming aliases for later publications.
    pub fn release_closure(&mut self, root: NodeListId<L>) -> Result<(), NodeArenaError> {
        let mut state = vec![0_u8; self.rows.len()];
        let mut closure = Vec::new();
        self.postorder(root, &mut state, &mut closure)?;
        for id in closure.into_iter().rev() {
            let index = id.index().expect("closure excludes the empty list");
            self.release_row(index);
        }
        while self.rows.last().is_some_and(Option::is_none) {
            self.rows.pop();
        }
        self.trim_empty_tail_segments();
        Ok(())
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.rows.len()
    }
}

fn node_words<L, Glue, Tokens>(
    nodes: &[Node<NodeListId<L>, Glue, Tokens>],
    etex_node_sizes: bool,
) -> (usize, usize) {
    nodes.iter().fold((0, 0), |total, node| {
        let words = node.tex_memory_words(etex_node_sizes);
        (
            total.0.saturating_add(words.0),
            total.1.saturating_add(words.1),
        )
    })
}

/// Borrowed node-list view; dropping it has no ownership effect.
#[derive(Clone, Copy)]
pub struct NodeList<'a, L, Glue = GlueSpec, Tokens = NodeTokenList> {
    arena: &'a NodeArena<L, Glue, Tokens>,
    id: NodeListId<L>,
}

/// Immutable logical node sequence resolved through one arena borrow.
pub type NodeSlice<L, Glue = GlueSpec, Tokens = NodeTokenList> =
    [Node<NodeListId<L>, Glue, Tokens>];

impl<L, Glue, Tokens> core::fmt::Debug for NodeList<'_, L, Glue, Tokens> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NodeList")
            .field("len", &self.nodes().len())
            .finish_non_exhaustive()
    }
}

impl<'a, L, Glue, Tokens> NodeList<'a, L, Glue, Tokens> {
    /// Returns the immutable logical nodes in source order.
    #[must_use]
    pub fn nodes(&self) -> &'a NodeSlice<L, Glue, Tokens> {
        self.id.index().map_or(&[], |index| {
            self.arena
                .allocation(index)
                .map(|allocation| allocation.nodes.as_ref())
                .expect("validated node-list row is live")
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.id.is_empty()
    }

    #[must_use]
    pub const fn id(self) -> NodeListId<L> {
        self.id
    }

    /// Resolves one child using the same coarse owner borrow.
    pub fn child(self, id: NodeListId<L>) -> Result<Self, NodeArenaError> {
        self.arena.get(id)
    }

    /// Resolves one nested coordinate through this list's coarse owner.
    pub fn resolve(self, id: NodeListId<L>) -> Result<Self, NodeArenaError> {
        self.child(id)
    }

    /// Borrows one nested row through this list's coarse owner.
    pub fn child_nodes(
        self,
        id: NodeListId<L>,
    ) -> Result<&'a NodeSlice<L, Glue, Tokens>, NodeArenaError> {
        Ok(self.child(id)?.nodes())
    }
}

impl<'a> NodeList<'a, PageLifetime> {
    #[must_use]
    pub fn get(&self, index: usize) -> Option<NodeRef<'a>> {
        self.nodes().get(index).map(NodeRef::from)
    }

    #[must_use]
    pub fn first(self) -> Option<NodeRef<'a>> {
        self.get(0)
    }

    #[must_use]
    pub fn last(self) -> Option<NodeRef<'a>> {
        self.len().checked_sub(1).and_then(|index| self.get(index))
    }

    pub fn iter(self) -> impl ExactSizeIterator<Item = NodeRef<'a>> {
        self.nodes().iter().map(NodeRef::from)
    }

    #[must_use]
    pub fn contains_direction(self) -> bool {
        self.nodes()
            .iter()
            .any(|node| matches!(node, Node::Direction(_)))
    }

    #[must_use]
    pub fn requires_shipout_normalization(self) -> bool {
        self.nodes().iter().any(node_requires_shipout_normalization)
    }

    #[must_use]
    pub fn node_requires_shipout_normalization(self, index: usize) -> Option<bool> {
        self.nodes()
            .get(index)
            .map(node_requires_shipout_normalization)
    }

    #[must_use]
    pub fn char_run(self, index: usize) -> Option<CharRun<'a>> {
        CharRun::new(self.nodes(), index)
    }

    #[must_use]
    pub fn char_codes(self, index: usize) -> Option<CharCodes<'a>> {
        CharCodes::new(self.nodes(), index)
    }
}

fn node_requires_shipout_normalization(node: &Node) -> bool {
    matches!(
        node,
        Node::HList(_)
            | Node::VList(_)
            | Node::Unset(_)
            | Node::Disc { .. }
            | Node::Ins { .. }
            | Node::Whatsit(_)
            | Node::Direction(_)
            | Node::MathNoad(_)
            | Node::FractionNoad(_)
            | Node::MathStyle(_)
            | Node::MathChoice(_)
            | Node::MathList(_)
            | Node::Nonscript
            | Node::Adjust(_)
            | Node::Glue {
                leader: Some(_),
                ..
            }
    )
}

/// Operation-scratch node storage.
pub type ScratchNodeArena = NodeArena<ScratchLifetime>;

/// Open-mode and page-builder node storage.
pub type PageNodeArena = NodeArena<PageLifetime>;

/// Revision-generation durable node storage.
pub type DurableNodeArena<G> = NodeArena<G, GlueId<G>, TokenListId<G>>;

/// Zero-allocation logical projection of one immutable node.
#[derive(Clone, Debug)]
pub enum NodeRef<'a, List = PageListId, Glue = GlueSpec, Tokens = NodeTokenList> {
    Char {
        font: crate::ids::FontId,
        ch: char,
        origin: crate::token::OriginId,
    },
    Lig {
        font: crate::ids::FontId,
        ch: char,
        orig: &'a [char],
        origins: &'a [crate::token::OriginId],
        left_hit: bool,
        right_hit: bool,
    },
    Kern {
        amount: crate::scaled::Scaled,
        kind: crate::node::KernKind,
    },
    MarginKern {
        amount: crate::scaled::Scaled,
        side: crate::node::MarginKernSide,
        font: crate::ids::FontId,
        ch: u8,
    },
    Glue {
        spec: Glue,
        kind: crate::node::GlueKind,
        leader: Option<crate::node::LeaderPayload<List>>,
    },
    Penalty(i32),
    Rule {
        width: Option<crate::scaled::Scaled>,
        height: Option<crate::scaled::Scaled>,
        depth: Option<crate::scaled::Scaled>,
    },
    HList(crate::node::BoxNode<List>),
    VList(crate::node::BoxNode<List>),
    Unset(crate::node::UnsetNode<List>),
    Disc {
        kind: crate::node::DiscKind,
        pre: List,
        post: List,
        replace: List,
        physical_replace_count: u8,
    },
    Mark {
        class: u16,
        tokens: &'a Tokens,
    },
    Ins {
        class: u16,
        size: crate::scaled::Scaled,
        split_top_skip: Glue,
        split_max_depth: crate::scaled::Scaled,
        floating_penalty: i32,
        content: List,
    },
    Whatsit(&'a crate::node::Whatsit<Glue, Tokens>),
    MathOn(crate::scaled::Scaled),
    MathOff(crate::scaled::Scaled),
    Direction(crate::node::Direction),
    MathNoad(crate::math::MathNoad<List>),
    FractionNoad(crate::math::MathFraction<List>),
    MathStyle(crate::math::MathStyle),
    MathChoice(crate::math::MathChoice<List>),
    MathList(crate::math::MathListNode<List>),
    Nonscript,
    Adjust(crate::node::AdjustNode<List>),
}

impl<'a, List: Copy, Glue: Copy, Tokens> From<&'a Node<List, Glue, Tokens>>
    for NodeRef<'a, List, Glue, Tokens>
{
    fn from(node: &'a Node<List, Glue, Tokens>) -> Self {
        match node {
            Node::Char { font, ch, origin } => Self::Char {
                font: *font,
                ch: *ch,
                origin: *origin,
            },
            Node::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                origins,
            } => Self::Lig {
                font: *font,
                ch: *ch,
                orig,
                origins,
                left_hit: *left_hit,
                right_hit: *right_hit,
            },
            Node::Kern { amount, kind } => Self::Kern {
                amount: *amount,
                kind: *kind,
            },
            Node::MarginKern {
                amount,
                side,
                font,
                ch,
            } => Self::MarginKern {
                amount: *amount,
                side: *side,
                font: *font,
                ch: *ch,
            },
            Node::Glue { spec, kind, leader } => Self::Glue {
                spec: *spec,
                kind: *kind,
                leader: *leader,
            },
            Node::Penalty(value) => Self::Penalty(*value),
            Node::Rule {
                width,
                height,
                depth,
            } => Self::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Node::HList(value) => Self::HList(*value),
            Node::VList(value) => Self::VList(*value),
            Node::Unset(value) => Self::Unset(*value),
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Self::Disc {
                kind: *kind,
                pre: *pre,
                post: *post,
                replace: *replace,
                physical_replace_count: *physical_replace_count,
            },
            Node::Mark { class, tokens } => Self::Mark {
                class: *class,
                tokens,
            },
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Self::Ins {
                class: *class,
                size: *size,
                split_top_skip: *split_top_skip,
                split_max_depth: *split_max_depth,
                floating_penalty: *floating_penalty,
                content: *content,
            },
            Node::Whatsit(value) => Self::Whatsit(value),
            Node::MathOn(value) => Self::MathOn(*value),
            Node::MathOff(value) => Self::MathOff(*value),
            Node::Direction(value) => Self::Direction(*value),
            Node::MathNoad(value) => Self::MathNoad(value.clone()),
            Node::FractionNoad(value) => Self::FractionNoad(*value),
            Node::MathStyle(value) => Self::MathStyle(*value),
            Node::MathChoice(value) => Self::MathChoice(*value),
            Node::MathList(value) => Self::MathList(*value),
            Node::Nonscript => Self::Nonscript,
            Node::Adjust(value) => Self::Adjust(*value),
        }
    }
}

/// Dimension-bearing projection shared by page-arena and operation buffers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PackedNode<'a> {
    Glyph {
        font: crate::ids::FontId,
        ch: char,
    },
    Kern {
        amount: crate::scaled::Scaled,
        kind: Option<crate::node::KernKind>,
    },
    Glue {
        spec: GlueSpec,
        leader: Option<&'a crate::node::LeaderPayload<PageListId>>,
    },
    Rule {
        width: Option<crate::scaled::Scaled>,
        height: Option<crate::scaled::Scaled>,
        depth: Option<crate::scaled::Scaled>,
    },
    Box(crate::node::BoxNode<PageListId>),
    Unset(crate::node::UnsetNode<PageListId>),
    Disc(PageListId),
    Image {
        width: crate::scaled::Scaled,
        height: crate::scaled::Scaled,
        depth: crate::scaled::Scaled,
    },
    Math(crate::scaled::Scaled),
    Ignored,
}

impl NodeRef<'_> {
    #[must_use]
    pub const fn kind(&self) -> crate::node::NodeKind {
        use crate::node::NodeKind;
        match self {
            Self::Char { .. } => NodeKind::Char,
            Self::Lig { .. } => NodeKind::Lig,
            Self::Kern { .. } => NodeKind::Kern,
            Self::MarginKern { .. } => NodeKind::MarginKern,
            Self::Glue { .. } => NodeKind::Glue,
            Self::Penalty(_) => NodeKind::Penalty,
            Self::Rule { .. } => NodeKind::Rule,
            Self::HList(_) => NodeKind::HList,
            Self::VList(_) => NodeKind::VList,
            Self::Unset(_) => NodeKind::Unset,
            Self::Disc { .. } => NodeKind::Disc,
            Self::Mark { .. } => NodeKind::Mark,
            Self::Ins { .. } => NodeKind::Ins,
            Self::Whatsit(_) => NodeKind::Whatsit,
            Self::MathOn(_) => NodeKind::MathOn,
            Self::MathOff(_) => NodeKind::MathOff,
            Self::Direction(_) => NodeKind::Direction,
            Self::MathNoad(_) => NodeKind::MathNoad,
            Self::FractionNoad(_) => NodeKind::FractionNoad,
            Self::MathStyle(_) => NodeKind::MathStyle,
            Self::MathChoice(_) => NodeKind::MathChoice,
            Self::MathList(_) => NodeKind::MathList,
            Self::Nonscript => NodeKind::Nonscript,
            Self::Adjust(_) => NodeKind::Adjust,
        }
    }

    #[must_use]
    pub const fn etex_type(&self) -> i32 {
        self.kind().etex_type()
    }

    #[must_use]
    pub fn packed(&self) -> PackedNode<'_> {
        match self {
            Self::Char { font, ch, .. } | Self::Lig { font, ch, .. } => PackedNode::Glyph {
                font: *font,
                ch: *ch,
            },
            Self::Kern { amount, kind } => PackedNode::Kern {
                amount: *amount,
                kind: Some(*kind),
            },
            Self::MarginKern { amount, .. } => PackedNode::Kern {
                amount: *amount,
                kind: None,
            },
            Self::Glue { spec, leader, .. } => PackedNode::Glue {
                spec: *spec,
                leader: leader.as_ref(),
            },
            Self::Rule {
                width,
                height,
                depth,
            } => PackedNode::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::HList(value) | Self::VList(value) => PackedNode::Box(*value),
            Self::Unset(value) => PackedNode::Unset(*value),
            Self::Disc { replace, .. } => PackedNode::Disc(*replace),
            Self::Whatsit(
                crate::node::Whatsit::PdfRefXForm {
                    width,
                    height,
                    depth,
                    ..
                }
                | crate::node::Whatsit::PdfRefXImage {
                    width,
                    height,
                    depth,
                    ..
                },
            ) => PackedNode::Image {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::MathOn(value) | Self::MathOff(value) => PackedNode::Math(*value),
            _ => PackedNode::Ignored,
        }
    }

    #[must_use]
    pub fn vertical_dimensions(&self) -> Option<(crate::scaled::Scaled, crate::scaled::Scaled)> {
        match self.packed() {
            PackedNode::Box(node) => Some((node.height, node.depth)),
            PackedNode::Unset(node) => Some((node.height, node.depth)),
            PackedNode::Rule { height, depth, .. } => Some((
                height.unwrap_or(crate::scaled::Scaled::from_raw(0)),
                depth.unwrap_or(crate::scaled::Scaled::from_raw(0)),
            )),
            _ => None,
        }
    }

    #[must_use]
    pub fn box_node(&self) -> Option<crate::node::BoxNode<PageListId>> {
        match self.packed() {
            PackedNode::Box(node) => Some(node),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_owned_with(&self, _resolve: impl FnMut(PageListId) -> PageListId) -> Node {
        match self {
            Self::Char { font, ch, origin } => Node::Char {
                font: *font,
                ch: *ch,
                origin: *origin,
            },
            Self::Lig {
                font,
                ch,
                orig,
                origins,
                left_hit,
                right_hit,
            } => Node::Lig {
                font: *font,
                ch: *ch,
                orig: orig.to_vec(),
                left_hit: *left_hit,
                right_hit: *right_hit,
                origins: origins.to_vec(),
            },
            Self::Kern { amount, kind } => Node::Kern {
                amount: *amount,
                kind: *kind,
            },
            Self::MarginKern {
                amount,
                side,
                font,
                ch,
            } => Node::MarginKern {
                amount: *amount,
                side: *side,
                font: *font,
                ch: *ch,
            },
            Self::Glue { spec, kind, leader } => Node::Glue {
                spec: *spec,
                kind: *kind,
                leader: *leader,
            },
            Self::Penalty(value) => Node::Penalty(*value),
            Self::Rule {
                width,
                height,
                depth,
            } => Node::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::HList(value) => Node::HList(*value),
            Self::VList(value) => Node::VList(*value),
            Self::Unset(value) => Node::Unset(*value),
            Self::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Node::Disc {
                kind: *kind,
                pre: *pre,
                post: *post,
                replace: *replace,
                physical_replace_count: *physical_replace_count,
            },
            Self::Mark { class, tokens } => Node::Mark {
                class: *class,
                tokens: (*tokens).clone(),
            },
            Self::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Node::Ins {
                class: *class,
                size: *size,
                split_top_skip: *split_top_skip,
                split_max_depth: *split_max_depth,
                floating_penalty: *floating_penalty,
                content: *content,
            },
            Self::Whatsit(value) => Node::Whatsit((*value).clone()),
            Self::MathOn(value) => Node::MathOn(*value),
            Self::MathOff(value) => Node::MathOff(*value),
            Self::Direction(value) => Node::Direction(*value),
            Self::MathNoad(value) => Node::MathNoad(value.clone()),
            Self::FractionNoad(value) => Node::FractionNoad(*value),
            Self::MathStyle(value) => Node::MathStyle(*value),
            Self::MathChoice(value) => Node::MathChoice(*value),
            Self::MathList(value) => Node::MathList(*value),
            Self::Nonscript => Node::Nonscript,
            Self::Adjust(value) => Node::Adjust(*value),
        }
    }
}

/// Unified read cursor for operation buffers and immutable page rows.
#[derive(Clone, Copy)]
pub struct NodeCursor<'a> {
    nodes: &'a [Node],
}

impl<'a> NodeCursor<'a> {
    #[must_use]
    pub const fn owned(nodes: &'a [Node]) -> Self {
        Self { nodes }
    }
    #[must_use]
    pub const fn compact(nodes: &'a [Node]) -> Self {
        Self { nodes }
    }
    #[must_use]
    pub const fn len(&self) -> usize {
        self.nodes.len()
    }
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    #[must_use]
    pub fn get(&self, index: usize) -> Option<NodeRef<'a>> {
        self.nodes.get(index).map(NodeRef::from)
    }
    #[must_use]
    pub fn owned_node(&self, index: usize) -> Option<&'a Node> {
        self.nodes.get(index)
    }
    #[must_use]
    pub fn char_codes(&self, index: usize) -> Option<CharCodes<'a>> {
        CharCodes::new(self.nodes, index)
    }
}

/// Lazy same-font byte-character run.
pub struct CharCodes<'a> {
    nodes: &'a [Node],
    next: usize,
    font: crate::ids::FontId,
}

impl<'a> CharCodes<'a> {
    fn new(nodes: &'a [Node], index: usize) -> Option<Self> {
        let Node::Char { font, ch, .. } = nodes.get(index)? else {
            return None;
        };
        u8::try_from(*ch as u32).ok()?;
        Some(Self {
            nodes,
            next: index,
            font: *font,
        })
    }
    #[must_use]
    pub const fn font(&self) -> crate::ids::FontId {
        self.font
    }
}

impl Iterator for CharCodes<'_> {
    type Item = u8;
    fn next(&mut self) -> Option<Self::Item> {
        let Node::Char { font, ch, .. } = self.nodes.get(self.next)? else {
            return None;
        };
        if *font != self.font {
            return None;
        }
        let code = u8::try_from(*ch as u32).ok()?;
        self.next += 1;
        Some(code)
    }
}

/// Borrowed maximal run of page-arena byte characters with one font.
#[derive(Clone, Copy, Debug)]
pub struct CharRun<'a> {
    nodes: &'a [Node],
    start: usize,
    end: usize,
    font: crate::ids::FontId,
}

impl<'a> CharRun<'a> {
    fn new(nodes: &'a [Node], start: usize) -> Option<Self> {
        let Node::Char { font, ch, .. } = nodes.get(start)? else {
            return None;
        };
        u8::try_from(*ch as u32).ok()?;
        let mut end = start + 1;
        while let Some(Node::Char {
            font: candidate,
            ch,
            ..
        }) = nodes.get(end)
        {
            if candidate != font || u8::try_from(*ch as u32).is_err() {
                break;
            }
            end += 1;
        }
        Some(Self {
            nodes,
            start,
            end,
            font: *font,
        })
    }

    #[must_use]
    pub const fn font(self) -> crate::ids::FontId {
        self.font
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.end - self.start
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn codes(self) -> impl ExactSizeIterator<Item = u8> + 'a {
        self.nodes[self.start..self.end].iter().map(|node| {
            let Node::Char { ch, .. } = node else {
                unreachable!("character-run bounds contain only characters")
            };
            u8::try_from(*ch as u32).expect("character-run bounds contain only byte characters")
        })
    }

    pub fn origins(self) -> impl ExactSizeIterator<Item = crate::token::OriginId> + 'a {
        self.nodes[self.start..self.end].iter().map(|node| {
            let Node::Char { origin, .. } = node else {
                unreachable!("character-run bounds contain only characters")
            };
            *origin
        })
    }
}
