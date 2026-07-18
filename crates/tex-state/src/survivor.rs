//! Survivor arena for node lists that escape an epoch.
//!
//! Promotion copies an epoch-rooted node graph into one contiguous allocation
//! and rewrites child spans to be relative to the survivor root.

use crate::glue::GlueSpec;
use crate::ids::{ArenaRef, FontId, GlueId, NodeListId, SurvivorRootId};
#[cfg(debug_assertions)]
use crate::node::Node;
use crate::node_arena::{
    ChildPatch, NodeArena, NodeList, NodeOriginOverlay, NodeSemanticId, NodeStorage,
};
use ahash::AHashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

#[cfg(feature = "profiling-stats")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "profiling-stats")]
use std::time::Instant;

const SURVIVOR_ROOT_MAX: u32 = (1 << 20) - 2;
static NEXT_SURVIVOR_ROOT: AtomicU32 = AtomicU32::new(0);

/// Process-local survivor-operation measurements. Times include the complete
/// promotion, recycling release, or shared-payload drop operation; scratch
/// bytes are allocator payload bytes and exclude allocator metadata and
/// `HashMap` control bytes.
#[cfg(feature = "profiling-stats")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SurvivorMeasurement {
    pub fresh_promotions: u64,
    pub fresh_promotion_nanos: u64,
    pub recycled_promotions: u64,
    pub recycled_promotion_nanos: u64,
    pub releases_to_recycling: u64,
    pub release_nanos: u64,
    pub shared_payload_drops: u64,
    pub shared_payload_drop_nanos: u64,
    pub peak_promotion_scratch_logical_bytes: u64,
    pub peak_promotion_scratch_retained_bytes: u64,
    pub source_words: u64,
    pub child_bearing_nodes: u64,
    pub remap_entries: u64,
    pub pending_entries: u64,
    pub peak_remap_entries: u64,
    pub peak_pending_entries: u64,
}

#[cfg(feature = "profiling-stats")]
mod measurement {
    use super::{AtomicU64, Instant, Ordering, SurvivorMeasurement};

    pub static FRESH_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static FRESH_NANOS: AtomicU64 = AtomicU64::new(0);
    pub static RECYCLED_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static RECYCLED_NANOS: AtomicU64 = AtomicU64::new(0);
    pub static RELEASE_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static RELEASE_NANOS: AtomicU64 = AtomicU64::new(0);
    pub static SHARED_DROP_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static SHARED_DROP_NANOS: AtomicU64 = AtomicU64::new(0);
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
            shared_payload_drops: SHARED_DROP_CALLS.load(Ordering::Relaxed),
            shared_payload_drop_nanos: SHARED_DROP_NANOS.load(Ordering::Relaxed),
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

#[cfg(feature = "profiling-stats")]
#[must_use]
pub fn survivor_measurement() -> SurvivorMeasurement {
    measurement::snapshot()
}

/// Arena for promoted node-list roots.
#[derive(Clone, Debug)]
pub struct SurvivorArena {
    // Root slots are local while immutable semantic payloads may be shared.
    // Packed root keys are process-unique so sibling forks cannot alias one
    // another's new roots.
    slots: Vec<Option<SurvivorRoot>>,
    root_slots: AHashMap<SurvivorRootId, usize>,
    // Node storage is independent of identity and can safely be recycled.
    recycled: Vec<NodeStorage>,
    recycled_buffer_uses: usize,
    promotion_remap: AHashMap<NodeListId, NodeListId>,
    promotion_pending: Vec<ChildPatch>,
}

#[derive(Clone, Debug)]
struct SurvivorRoot {
    payload: Arc<SurvivorPayload>,
    origin_overlay: Option<NodeOriginOverlay>,
    deferred_origins: Vec<DeferredParagraphOrigins>,
    refcount: u32,
}

#[derive(Clone, Debug)]
enum DeferredParagraphOrigins {
    Stable(crate::ParagraphProvenanceRecipe),
    Lazy {
        start: u32,
        end: u32,
        resolver: Arc<crate::ParagraphOriginResolver>,
    },
}

/// Diagnostic provenance selected for one raw character or ligature node.
#[doc(hidden)]
pub enum DeferredNodeOrigins<'a> {
    Stable(&'a crate::ParagraphProvenanceRecipe, std::ops::Range<usize>),
    Lazy(&'a Arc<crate::ParagraphOriginResolver>),
}

/// Monotonic diagnostic-provenance lookup for one emitted survivor list.
///
/// Ordinary shipout visits list words in order. Resolving the survivor root
/// and locating its first sparse provenance entry once keeps the per-glyph
/// path to a single cursor comparison. Direction permutations use the random
/// lookup below because they are deliberately rare.
#[doc(hidden)]
#[derive(Debug)]
pub struct DeferredNodeOriginCursor<'a> {
    entries: &'a [DeferredParagraphOrigins],
    entry: usize,
    node: usize,
    list_start: u32,
    list_end: u32,
}

impl<'a> DeferredNodeOriginCursor<'a> {
    fn empty() -> Self {
        Self {
            entries: &[],
            entry: 0,
            node: 0,
            list_start: 0,
            list_end: 0,
        }
    }

    /// Returns the stable recipe slots for the next provenance-bearing word.
    #[doc(hidden)]
    pub fn node_origins(&mut self, index: usize, len: usize) -> Option<DeferredNodeOrigins<'a>> {
        let word = self.list_start.checked_add(u32::try_from(index).ok()?)?;
        if word >= self.list_end {
            return None;
        }
        while let Some(deferred) = self.entries.get(self.entry) {
            match deferred {
                DeferredParagraphOrigins::Lazy {
                    start,
                    end,
                    resolver,
                } => {
                    if *end <= word {
                        self.entry += 1;
                        self.node = 0;
                        continue;
                    }
                    if *start > word {
                        return None;
                    }
                    return Some(DeferredNodeOrigins::Lazy(resolver));
                }
                DeferredParagraphOrigins::Stable(recipe) => {
                    let slots = &recipe.node_slots;
                    while let Some(slot) = slots.get(self.node) {
                        if slot.word >= self.list_end {
                            return None;
                        }
                        if slot.word < word {
                            self.node += 1;
                            continue;
                        }
                        if slot.word > word {
                            return None;
                        }
                        self.node += 1;
                        let start = slot.slot as usize;
                        let end = start.checked_add(len)?;
                        return (end <= recipe.origin_slots.len())
                            .then_some(DeferredNodeOrigins::Stable(recipe, start..end));
                    }
                }
            }
            self.entry += 1;
            self.node = 0;
        }
        None
    }
}

#[derive(Debug)]
struct SurvivorPayload {
    storage: NodeStorage,
    semantic_spans: Vec<SurvivorSemanticSpan>,
}

/// Accepted-history ownership of one immutable survivor list and the
/// store-local resources needed to mount it in a related Universe.
///
/// The semantic payload is shared directly. Cloning or dropping this handle
/// never walks the node graph; a live Universe installs a local root slot and
/// ordinary rollback pin only when it actually consumes the mount. Retention
/// also summarizes the immutable graph's supported shape and external resource
/// closure so replay validation never has to decode the graph again.
#[derive(Clone, Debug)]
pub struct RetainedNodeList {
    id: NodeListId,
    payload: Arc<SurvivorPayload>,
    glues: Arc<[(GlueId, GlueSpec)]>,
    fonts: Arc<[FontId]>,
    mountable: bool,
}

impl RetainedNodeList {
    #[must_use]
    pub const fn id(&self) -> NodeListId {
        self.id
    }

    pub(crate) fn glues(&self) -> &[(GlueId, GlueSpec)] {
        &self.glues
    }

    pub(crate) fn fonts(&self) -> &[FontId] {
        &self.fonts
    }

    pub(crate) const fn is_mountable(&self) -> bool {
        self.mountable
    }

    pub(crate) fn resource_retained_bytes(&self) -> usize {
        self.glues
            .len()
            .saturating_mul(core::mem::size_of::<(GlueId, GlueSpec)>())
            .saturating_add(
                self.fonts
                    .len()
                    .saturating_mul(core::mem::size_of::<FontId>()),
            )
    }
}

impl PartialEq for RetainedNodeList {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RetainedNodeList {}

#[derive(Clone, Copy, Debug)]
struct SurvivorSemanticSpan {
    start: u32,
    len: u32,
    semantic_id: NodeSemanticId,
}

impl SurvivorArena {
    /// Reserves a process-unique root for a fully validated frozen format
    /// graph. The caller must publish it before exposing any derived handles.
    pub(crate) fn reserve_frozen_root(&self) -> SurvivorRootId {
        allocate_survivor_root()
            .map(SurvivorRootId::new)
            .expect("survivor root identity space exhausted")
    }

    /// Publishes a portable frozen graph as one immutable survivor root.
    pub(crate) fn publish_frozen_root(
        &mut self,
        root: SurvivorRootId,
        storage: NodeStorage,
        spans: Vec<(u32, u32, NodeSemanticId)>,
    ) {
        let spans = spans
            .into_iter()
            .filter(|(_, len, _)| *len != 0)
            .map(|(start, len, semantic_id)| SurvivorSemanticSpan {
                start,
                len,
                semantic_id,
            })
            .collect();
        self.allocate_root(root, storage, spans);
    }

    pub(crate) fn retained_payload_bytes(&self) -> usize {
        let root_storage = self
            .slots
            .iter()
            .flatten()
            .map(|root| {
                root.payload
                    .storage
                    .retained_payload_bytes()
                    .saturating_add(
                        root.payload
                            .semantic_spans
                            .capacity()
                            .saturating_mul(core::mem::size_of::<SurvivorSemanticSpan>()),
                    )
            })
            .sum::<usize>();
        let recycled = self
            .recycled
            .iter()
            .map(NodeStorage::retained_payload_bytes)
            .sum::<usize>();
        root_storage
            .saturating_add(recycled)
            .saturating_add(
                self.slots
                    .capacity()
                    .saturating_mul(core::mem::size_of::<Option<SurvivorRoot>>()),
            )
            .saturating_add(
                self.root_slots
                    .capacity()
                    .saturating_mul(core::mem::size_of::<(SurvivorRootId, usize)>()),
            )
    }

    /// Creates an empty survivor arena.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            root_slots: AHashMap::new(),
            recycled: Vec::new(),
            recycled_buffer_uses: 0,
            promotion_remap: AHashMap::new(),
            promotion_pending: Vec::new(),
        }
    }

    /// Promotes an epoch list into one survivor root with refcount 1.
    pub(crate) fn promote(&mut self, id: NodeListId, epoch: &NodeArena) -> NodeListId {
        #[cfg(feature = "profiling-stats")]
        let started = measurement::start_timer();
        assert!(
            matches!(id.arena(), ArenaRef::Epoch),
            "only epoch node lists are promoted"
        );
        let root = allocate_survivor_root()
            .map(SurvivorRootId::new)
            .expect("survivor root identity space exhausted");
        let (storage, recycled) = self.take_recycled_buffer();
        #[cfg(not(feature = "profiling-stats"))]
        let _ = recycled;
        let remapped = core::mem::take(&mut self.promotion_remap);
        let pending = core::mem::take(&mut self.promotion_pending);
        let copied = copy_list_iterative(id, epoch, self, storage, root, remapped, pending);
        #[cfg(feature = "profiling-stats")]
        {
            measurement::PEAK_SCRATCH_LOGICAL
                .fetch_max(copied.peak_scratch_logical as u64, Ordering::Relaxed);
            measurement::PEAK_SCRATCH_RETAINED
                .fetch_max(copied.peak_scratch_retained as u64, Ordering::Relaxed);
        }
        self.promotion_remap = copied.remapped;
        self.promotion_pending = copied.pending;
        self.allocate_root(
            root,
            copied.storage,
            copied.semantic_spans,
            copied.deferred_origins,
        );
        self.debug_assert_no_epoch_ids(copied.promoted);
        #[cfg(feature = "profiling-stats")]
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
            end <= root.payload.storage.len(),
            "survivor node-list id is not live"
        );
        root.payload
            .storage
            .view_with_origins(id.start(), id.len(), root.origin_overlay.as_ref())
    }

    #[must_use]
    pub(crate) fn semantic_id(&self, id: NodeListId) -> NodeSemanticId {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("survivor arena can only resolve survivor semantic ids");
        };
        if id.len() == 0 {
            return NodeSemanticId::empty();
        }
        let root = self.root(root);
        let index = root
            .payload
            .semantic_spans
            .binary_search_by_key(&id.start(), |span| span.start)
            .expect("survivor node-list semantic id is not live");
        let span = root.payload.semantic_spans[index];
        assert_eq!(span.len, id.len(), "survivor semantic span length mismatch");
        span.semantic_id
    }

    /// Replaces one placeholder identity while atomically installing a frozen
    /// graph. Callers must finish validating every span before exposing the
    /// restored stores.
    pub(crate) fn set_frozen_semantic_id(&mut self, id: NodeListId, semantic_id: NodeSemanticId) {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("frozen semantic ids belong to survivor roots");
        };
        let root = self.root_mut(root);
        let payload = Arc::get_mut(&mut root.payload)
            .expect("a frozen root cannot be shared before identity validation");
        let index = payload
            .semantic_spans
            .binary_search_by_key(&id.start(), |span| span.start)
            .expect("frozen node-list semantic id is not live");
        let span = &mut payload.semantic_spans[index];
        assert_eq!(span.len, id.len(), "frozen semantic span length mismatch");
        span.semantic_id = semantic_id;
    }

    /// Captures accepted-history ownership of an already promoted list.
    pub(crate) fn retain(
        &self,
        id: NodeListId,
        glues: Vec<(GlueId, GlueSpec)>,
        fonts: Vec<FontId>,
        mountable: bool,
    ) -> RetainedNodeList {
        let ArenaRef::Survivor(_) = id.arena() else {
            panic!("only survivor node-list ids can be retained");
        };
        let root = self.root_for_retained(id);
        RetainedNodeList {
            id,
            payload: Arc::clone(&root.payload),
            glues: glues.into(),
            fonts: fonts.into(),
            mountable,
        }
    }

    /// Installs one accepted-history payload as a local root. The returned
    /// flag says whether the new slot's initial refcount must be adopted by
    /// the caller's ordinary rollback pin log.
    pub(crate) fn mount(&mut self, retained: &RetainedNodeList) -> Option<bool> {
        let ArenaRef::Survivor(root_id) = retained.id.arena() else {
            return None;
        };
        if let Some(index) = self.root_slots.get(&root_id).copied() {
            let root = self.slots.get(index)?.as_ref()?;
            return Arc::ptr_eq(&root.payload, &retained.payload).then_some(false);
        }
        let index = self.slots.len();
        self.slots.push(Some(SurvivorRoot {
            payload: Arc::clone(&retained.payload),
            origin_overlay: None,
            deferred_origins: Vec::new(),
            refcount: 1,
        }));
        assert!(
            self.root_slots.insert(root_id, index).is_none(),
            "retained survivor root was published concurrently"
        );
        Some(true)
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
            #[cfg(feature = "profiling-stats")]
            let started = measurement::start_timer();
            let index = self
                .root_slots
                .remove(&root)
                .expect("survivor root is not live");
            let root = self.slots[index].take().expect("survivor root is not live");
            let recycled = if let Ok(mut payload) = Arc::try_unwrap(root.payload) {
                payload.storage.clear();
                self.recycled.push(payload.storage);
                true
            } else {
                false
            };
            #[cfg(feature = "profiling-stats")]
            if recycled {
                measurement::RELEASE_CALLS.fetch_add(1, Ordering::Relaxed);
                let nanos = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
                measurement::RELEASE_NANOS.fetch_add(nanos, Ordering::Relaxed);
            } else {
                measurement::SHARED_DROP_CALLS.fetch_add(1, Ordering::Relaxed);
                let nanos = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
                measurement::SHARED_DROP_NANOS.fetch_add(nanos, Ordering::Relaxed);
            }
            #[cfg(not(feature = "profiling-stats"))]
            let _ = recycled;
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
            .is_some_and(|end| end <= slot.payload.storage.len())
    }

    /// Mounts current-revision diagnostic provenance over one immutable
    /// survivor payload. The semantic graph remains shared across arena clones.
    pub(crate) fn mount_paragraph_origins(
        &mut self,
        id: NodeListId,
        root_origins: &[crate::token::OriginId],
        origin_slots: &[u32],
    ) -> bool {
        let ArenaRef::Survivor(root_id) = id.arena() else {
            return false;
        };
        let Some(index) = self.root_slots.get(&root_id).copied() else {
            return false;
        };
        let Some(Some(root)) = self.slots.get(index) else {
            return false;
        };
        let Some(overlay) =
            root.payload
                .storage
                .paragraph_origin_overlay(id, root_origins, origin_slots)
        else {
            return false;
        };
        self.slots[index]
            .as_mut()
            .expect("survivor root is live")
            .origin_overlay = Some(overlay);
        self.slots[index]
            .as_mut()
            .expect("survivor root is live")
            .deferred_origins
            .clear();
        true
    }

    /// Attaches an allocation-free diagnostic recipe to one mounted root.
    pub(crate) fn mount_deferred_paragraph_origins(
        &mut self,
        id: NodeListId,
        recipe: crate::ParagraphProvenanceRecipe,
    ) -> bool {
        let ArenaRef::Survivor(root_id) = id.arena() else {
            return false;
        };
        let Some(index) = self.root_slots.get(&root_id).copied() else {
            return false;
        };
        let Some(Some(root)) = self.slots.get_mut(index) else {
            return false;
        };
        root.origin_overlay = None;
        root.deferred_origins.clear();
        root.deferred_origins
            .push(DeferredParagraphOrigins::Stable(recipe));
        true
    }

    /// Attaches one accepted-generation resolver to the raw origins already
    /// embedded throughout a retained paragraph graph.
    pub(crate) fn mount_lazy_paragraph_origins(
        &mut self,
        id: NodeListId,
        resolver: Arc<crate::ParagraphOriginResolver>,
    ) -> bool {
        let ArenaRef::Survivor(root_id) = id.arena() else {
            return false;
        };
        let Some(index) = self.root_slots.get(&root_id).copied() else {
            return false;
        };
        let Some(Some(root)) = self.slots.get_mut(index) else {
            return false;
        };
        root.origin_overlay = None;
        root.deferred_origins.clear();
        root.deferred_origins.push(DeferredParagraphOrigins::Lazy {
            start: 0,
            end: u32::try_from(root.payload.storage.len()).unwrap_or(u32::MAX),
            resolver,
        });
        true
    }

    fn deferred_paragraph_origins_ref(&self, id: NodeListId) -> &[DeferredParagraphOrigins] {
        let ArenaRef::Survivor(root_id) = id.arena() else {
            return &[];
        };
        let Some(index) = self.root_slots.get(&root_id).copied() else {
            return &[];
        };
        self.slots
            .get(index)
            .and_then(Option::as_ref)
            .map_or(&[], |root| root.deferred_origins.as_slice())
    }

    pub(crate) fn deferred_node_origin_cursor(
        &self,
        list: NodeListId,
    ) -> DeferredNodeOriginCursor<'_> {
        if !matches!(list.arena(), ArenaRef::Survivor(_)) {
            return DeferredNodeOriginCursor::empty();
        }
        let Some(list_end) = list.start().checked_add(list.len()) else {
            return DeferredNodeOriginCursor::empty();
        };
        let entries = self.deferred_paragraph_origins_ref(list);
        let entry = entries.partition_point(|deferred| match deferred {
            DeferredParagraphOrigins::Stable(recipe) => recipe
                .node_slots
                .last()
                .is_none_or(|slot| slot.word < list.start()),
            DeferredParagraphOrigins::Lazy { end, .. } => *end <= list.start(),
        });
        let node = entries.get(entry).map_or(0, |deferred| match deferred {
            DeferredParagraphOrigins::Stable(recipe) => recipe
                .node_slots
                .partition_point(|slot| slot.word < list.start()),
            DeferredParagraphOrigins::Lazy { .. } => 0,
        });
        DeferredNodeOriginCursor {
            entries,
            entry,
            node,
            list_start: list.start(),
            list_end,
        }
    }

    pub(crate) fn deferred_node_origins(
        &self,
        list: NodeListId,
        index: usize,
        len: usize,
    ) -> Option<DeferredNodeOrigins<'_>> {
        let ArenaRef::Survivor(root_id) = list.arena() else {
            return None;
        };
        let slot = self.root_slots.get(&root_id).copied()?;
        let deferred = self
            .slots
            .get(slot)?
            .as_ref()?
            .deferred_origins
            .iter()
            .find(|deferred| {
                let Some(word) = list
                    .start()
                    .checked_add(u32::try_from(index).ok().unwrap_or(u32::MAX))
                else {
                    return false;
                };
                match deferred {
                    DeferredParagraphOrigins::Stable(recipe) => recipe
                        .node_slots
                        .binary_search_by_key(&word, |node| node.word)
                        .is_ok(),
                    DeferredParagraphOrigins::Lazy { start, end, .. } => {
                        (*start..*end).contains(&word)
                    }
                }
            })?;
        let word = list.start().checked_add(u32::try_from(index).ok()?)?;
        match deferred {
            DeferredParagraphOrigins::Stable(recipe) => {
                let range = recipe.node_origin_slots(word, len)?;
                Some(DeferredNodeOrigins::Stable(recipe, range))
            }
            DeferredParagraphOrigins::Lazy { resolver, .. } => {
                Some(DeferredNodeOrigins::Lazy(resolver))
            }
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn testing_payload_strong_count(retained: &RetainedNodeList) -> usize {
        Arc::strong_count(&retained.payload)
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

    #[cfg(feature = "profiling-stats")]
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
            add_storage(&mut totals, "survivor.live", &root.payload.storage);
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
        let (len, capacity) = self
            .slots
            .iter()
            .flatten()
            .map(|root| {
                (
                    root.payload.semantic_spans.len(),
                    root.payload.semantic_spans.capacity(),
                )
            })
            .fold((0, 0), |(len, capacity), current| {
                (len + current.0, capacity + current.1)
            });
        let element_bytes = core::mem::size_of::<SurvivorSemanticSpan>();
        columns.push(crate::node_arena::NodeMemoryColumn {
            name: "survivor.live.semantic_spans".to_owned(),
            len,
            capacity,
            element_bytes,
            logical_bytes: len * element_bytes,
            retained_payload_bytes: capacity * element_bytes,
        });
        columns
    }

    fn allocate_root(
        &mut self,
        root: SurvivorRootId,
        storage: NodeStorage,
        semantic_spans: Vec<SurvivorSemanticSpan>,
        deferred_origins: Vec<DeferredParagraphOrigins>,
    ) {
        let slot = SurvivorRoot {
            payload: Arc::new(SurvivorPayload {
                storage,
                semantic_spans,
            }),
            origin_overlay: None,
            deferred_origins,
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

    fn root_for_retained(&self, id: NodeListId) -> &SurvivorRoot {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("expected survivor id");
        };
        let root = self.root(root);
        let end = (id.start() as usize)
            .checked_add(id.len() as usize)
            .expect("survivor node-list span overflow");
        assert!(
            end <= root.payload.storage.len(),
            "survivor node-list id is not live"
        );
        root
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
    semantic_spans: Vec<SurvivorSemanticSpan>,
    remapped: AHashMap<NodeListId, NodeListId>,
    pending: Vec<ChildPatch>,
    deferred_origins: Vec<DeferredParagraphOrigins>,
    #[cfg(feature = "profiling-stats")]
    peak_scratch_logical: usize,
    #[cfg(feature = "profiling-stats")]
    peak_scratch_retained: usize,
}

fn copy_list_iterative(
    id: NodeListId,
    epoch: &NodeArena,
    survivor: &SurvivorArena,
    storage: NodeStorage,
    root: SurvivorRootId,
    remapped: AHashMap<NodeListId, NodeListId>,
    pending: Vec<ChildPatch>,
) -> PromotionResult {
    let mut copy = PromotionCopy::new(epoch, survivor, storage, root, remapped, pending);
    let promoted = copy.copy_list(id);
    #[cfg(feature = "profiling-stats")]
    copy.measure_scratch();

    while let Some(patch) = copy.pending.pop() {
        let patch = patch.remap(|child| copy.copy_list(child));
        copy.storage.apply_child_patch(patch);
        #[cfg(feature = "profiling-stats")]
        copy.measure_scratch();
    }

    copy.remapped.clear();
    copy.pending.clear();
    PromotionResult {
        storage: copy.storage,
        promoted,
        semantic_spans: copy.semantic_spans,
        remapped: copy.remapped,
        pending: copy.pending,
        deferred_origins: copy.deferred_origins,
        #[cfg(feature = "profiling-stats")]
        peak_scratch_logical: copy.peak_scratch_logical,
        #[cfg(feature = "profiling-stats")]
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
    remapped: AHashMap<NodeListId, NodeListId>,
    pending: Vec<ChildPatch>,
    semantic_spans: Vec<SurvivorSemanticSpan>,
    deferred_origins: Vec<DeferredParagraphOrigins>,
    #[cfg(feature = "profiling-stats")]
    peak_scratch_logical: usize,
    #[cfg(feature = "profiling-stats")]
    peak_scratch_retained: usize,
}

impl<'a> PromotionCopy<'a> {
    fn new(
        epoch: &'a NodeArena,
        survivor: &'a SurvivorArena,
        storage: NodeStorage,
        root: SurvivorRootId,
        remapped: AHashMap<NodeListId, NodeListId>,
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
            semantic_spans: Vec::new(),
            deferred_origins: Vec::new(),
            #[cfg(feature = "profiling-stats")]
            peak_scratch_logical: 0,
            #[cfg(feature = "profiling-stats")]
            peak_scratch_retained: 0,
        }
    }

    #[cfg(feature = "profiling-stats")]
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

        let semantic_id = match id.arena() {
            ArenaRef::Epoch => self.epoch.epoch_semantic_id(id),
            ArenaRef::Survivor(_) => self.survivor.semantic_id(id),
        };
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
        if matches!(id.arena(), ArenaRef::Survivor(_)) {
            let source_start = id.start();
            let source_end = source_start
                .checked_add(id.len())
                .expect("source survivor span fits u32");
            for deferred in self.survivor.deferred_paragraph_origins_ref(id) {
                match deferred {
                    DeferredParagraphOrigins::Stable(recipe) => {
                        let node_slots = recipe
                            .node_slots
                            .iter()
                            .filter(|node| (source_start..source_end).contains(&node.word))
                            .map(|node| crate::ParagraphProvenanceNode {
                                word: start
                                    .checked_add(node.word - source_start)
                                    .expect("promoted provenance word fits u32"),
                                slot: node.slot,
                            })
                            .collect::<Vec<_>>();
                        if !node_slots.is_empty() {
                            let mut recipe = recipe.clone();
                            recipe.node_slots = node_slots.into();
                            self.deferred_origins
                                .push(DeferredParagraphOrigins::Stable(recipe));
                        }
                    }
                    DeferredParagraphOrigins::Lazy {
                        start: lazy_start,
                        end: lazy_end,
                        resolver,
                    } if *lazy_start < source_end && *lazy_end > source_start => {
                        self.deferred_origins.push(DeferredParagraphOrigins::Lazy {
                            start,
                            end: start.checked_add(len).expect("promoted list span fits u32"),
                            resolver: Arc::clone(resolver),
                        });
                    }
                    DeferredParagraphOrigins::Lazy { .. } => {}
                }
            }
        }
        if len != 0 {
            self.semantic_spans.push(SurvivorSemanticSpan {
                start,
                len,
                semantic_id,
            });
        }
        #[cfg(feature = "profiling-stats")]
        let pending_before = self.pending.len();
        let appended = self.storage.append_compact(nodes, &mut self.pending);
        assert_eq!(appended, (start, len));
        #[cfg(feature = "profiling-stats")]
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
