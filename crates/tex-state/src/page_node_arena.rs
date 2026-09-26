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
    ForkArenaError, OperationMark, PageMaterialLane, RegionValue, UniqueArenaList,
};
use crate::node::Node;
use crate::node_record::{NodeAnnexView, NodeAnnexWriter, NodeRecord};
use crate::node_region::{
    ClosureBuildMark, DurableRole, NodeCheckpointMark, NodePool, NodeRegion, NodeSealedBoundary,
    OwnedNodeClosure, PageRole, StructuralCopyReason, copy_closure_into, copy_region_root_into,
    loan_empty_page_box_body, preflight_empty_page_box_body, preflight_page_interior_closure,
    preflight_page_interior_intervals, rollback_page_interior_closure, structural_copy_fallback,
    transfer_closure_into, transfer_page_interior_closure, transfer_page_interior_intervals,
    transfer_sealed_closure_into,
};
use crate::node_sequence::SemanticSequenceIdentity;

type PageMaterialNode = NodeRecord<PageMaterialLane>;
type OwnedPageMaterialNode = Node<PageListId>;
type SelectedBoxBodyRanges = (Vec<Range<usize>>, Vec<Range<usize>>);
type PreflightedBoxBody = (Node<PageListId>, Vec<Range<usize>>, Vec<Range<usize>>);

mod consumed_source;
pub use consumed_source::{
    ConsumedPageSource, ConsumedPageWindow, GeneratedLineBody, PageDirectChunkSelection,
    PublishedGeneratedBoxBody,
};

/// Opaque typed-annex coordinate for page-owned intervals excluded from a box.
#[derive(Clone, Copy, Debug)]
pub struct PageBoxMigrationKey([u32; 7]);

impl PageBoxMigrationKey {
    pub(crate) const fn from_words(words: [u32; 7]) -> Self {
        Self(words)
    }

    pub(crate) const fn words(self) -> [u32; 7] {
        self.0
    }
}

/// Opaque typed-annex coordinate for independently selected generated-body ranges.
#[derive(Clone, Copy, Debug)]
pub struct PageBoxPositiveKey([u32; 7]);

impl PageBoxPositiveKey {
    pub(crate) const fn from_words(words: [u32; 7]) -> Self {
        Self(words)
    }

    pub(crate) const fn words(self) -> [u32; 7] {
        self.0
    }
}

/// A selected fragment inside one direct node or annex chunk. Only the
/// consumed generated wrapper may authorize its boundary projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageBoxCutRange {
    pub chunk_position: usize,
    pub local: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageBoxPositiveSelection {
    pub nodes: Vec<Range<usize>>,
    pub annex: Vec<Range<usize>>,
    pub node_cuts: Vec<PageBoxCutRange>,
    pub annex_cuts: Vec<PageBoxCutRange>,
}

/// Non-owning description of an original box wrapper's construction ranges.
/// Consuming the unique semantic root is still required to authorize transfer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageBoxMigrationMetadata {
    pub segment: crate::node_region::PageBoxSegment,
    pub exclusions: Vec<crate::node_region::PageBoxSegment>,
    /// Explicit selections have independent node and annex geometry.
    pub positive: Option<PageBoxPositiveSelection>,
    pub sidecar_annex_range: Option<Range<usize>>,
    pub wrapper_rebuild: bool,
}

impl PageBoxMigrationMetadata {
    pub(crate) fn selected_body_ranges(&self) -> Option<SelectedBoxBodyRanges> {
        if let Some(positive) = &self.positive {
            let PageBoxPositiveSelection {
                nodes,
                annex,
                node_cuts,
                annex_cuts,
            } = positive;
            if !self.exclusions.is_empty()
                || !valid_positive_ranges(nodes, self.segment.body_node_range().end)
                || !valid_positive_ranges(annex, self.segment.body_annex_range().end)
                || !valid_positive_cuts(node_cuts, nodes, self.segment.body_node_range().end)
                || !valid_positive_cuts(annex_cuts, annex, self.segment.body_annex_range().end)
            {
                return None;
            }
            return Some((nodes.clone(), annex.clone()));
        }
        fn subtract(
            body: Range<usize>,
            exclusions: impl IntoIterator<Item = Range<usize>>,
        ) -> Option<Vec<Range<usize>>> {
            let mut ranges = Vec::new();
            let mut cursor = body.start;
            for excluded in exclusions {
                if excluded.start < cursor
                    || excluded.end > body.end
                    || excluded.start > excluded.end
                {
                    return None;
                }
                if cursor < excluded.start {
                    ranges.push(cursor..excluded.start);
                }
                cursor = excluded.end;
            }
            if cursor < body.end {
                ranges.push(cursor..body.end);
            }
            Some(ranges)
        }

        if self
            .exclusions
            .iter()
            .any(|excluded| excluded.region() != self.segment.region())
        {
            return None;
        }
        Some((
            subtract(
                self.segment.body_node_range(),
                self.exclusions.iter().map(|x| x.node_range()),
            )?,
            subtract(
                self.segment.body_annex_range(),
                self.exclusions.iter().map(|x| x.annex_range()),
            )?,
        ))
    }
}

pub(crate) fn valid_positive_cuts(
    cuts: &[PageBoxCutRange],
    full: &[Range<usize>],
    limit: usize,
) -> bool {
    let mut previous: Option<&PageBoxCutRange> = None;
    cuts.iter().all(|cut| {
        let intersects_full = full.iter().any(|range| range.contains(&cut.chunk_position));
        let valid = cut.chunk_position < limit
            && cut.local.start < cut.local.end
            && u32::try_from(cut.chunk_position).is_ok()
            && u32::try_from(cut.local.end).is_ok()
            && !intersects_full
            && previous.is_none_or(|prior| {
                prior.chunk_position < cut.chunk_position
                    || (prior.chunk_position == cut.chunk_position
                        && prior.local.end <= cut.local.start)
            });
        previous = Some(cut);
        valid
    })
}

pub(crate) fn valid_positive_ranges(ranges: &[Range<usize>], limit: usize) -> bool {
    let mut previous_end = 0;
    ranges.iter().all(|range| {
        let valid = range.start < range.end
            && range.start >= previous_end
            && range.end <= limit
            && u32::try_from(range.end).is_ok();
        previous_end = range.end;
        valid
    })
}

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

impl<'a> PageMaterialNodeRef<'a> {
    pub(crate) const fn new(record: &'a PageMaterialNode, annex: NodeAnnexView<'a>) -> Self {
        Self { record, annex }
    }

    #[must_use]
    pub fn kind(self) -> Option<crate::node::NodeKind> {
        self.record.kind()
    }

    #[must_use]
    pub fn character(self) -> Option<(crate::ids::FontId, char, crate::token::OriginId)> {
        self.record.character()
    }

    #[must_use]
    pub fn glyph(self) -> Option<(crate::ids::FontId, char)> {
        self.record.glyph(self.annex)
    }

    #[must_use]
    pub fn kern(self) -> Option<(crate::scaled::Scaled, crate::node::KernKind)> {
        self.record.kern()
    }

    #[must_use]
    pub fn margin_kern_amount(self) -> Option<crate::scaled::Scaled> {
        self.record.margin_kern_amount()
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
    pub fn penalty(self) -> Option<i32> {
        self.record.penalty()
    }

    #[must_use]
    pub fn rule_width(self) -> Option<Option<crate::scaled::Scaled>> {
        self.record.rule_width()
    }

    #[must_use]
    pub fn box_width(self) -> Option<crate::scaled::Scaled> {
        self.record.box_width(self.annex)
    }

    #[must_use]
    pub fn box_segment(self) -> Option<crate::node_region::PageBoxSegment> {
        self.record.box_segment(self.annex)
    }

    #[must_use]
    pub fn box_migration_metadata(self) -> Option<PageBoxMigrationMetadata> {
        self.record.box_migration_metadata(self.annex)
    }

    #[must_use]
    pub fn unset_width(self) -> Option<crate::scaled::Scaled> {
        self.record.unset_width(self.annex)
    }

    #[must_use]
    pub fn math_boundary(self) -> Option<(bool, crate::scaled::Scaled)> {
        self.record.math_boundary()
    }

    #[must_use]
    pub fn direction(self) -> Option<crate::node::Direction> {
        self.record.direction()
    }

    #[must_use]
    pub fn pdf_image_width(self) -> Option<crate::scaled::Scaled> {
        self.record.pdf_image_width()
    }

    #[must_use]
    pub fn glue_spec_kind(self) -> Option<(crate::glue::GlueSpec, crate::node::GlueKind)> {
        self.record.glue_spec_kind(self.annex)
    }

    #[must_use]
    pub fn glue_origin(self) -> Option<crate::node::GlueSpecOrigin> {
        self.record.glue_origin()
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

    #[must_use]
    pub fn discretionary_break(self) -> Option<(crate::node::DiscKind, PageListId, PageListId)> {
        self.record.discretionary_break(self.annex)
    }

    #[must_use]
    pub fn discretionary_replace(self) -> Option<PageListId> {
        self.record.discretionary_replace(self.annex)
    }

    pub fn visit_ligature_source(
        self,
        visit: impl FnMut(char, crate::token::OriginId),
    ) -> Option<crate::ids::FontId> {
        self.record.visit_ligature_source(self.annex, visit)
    }

    #[must_use]
    pub(crate) fn tex_memory_words(self, etex_node_sizes: bool) -> (usize, usize) {
        self.record.tex_memory_words(self.annex, etex_node_sizes)
    }

    #[must_use]
    pub(crate) fn retains_node_list(self) -> bool {
        let mut retains = false;
        let _ = self
            .record
            .visit_node_lists(self.annex, |list| retains |= !list.is_empty());
        retains
    }
}

/// Scalar publication evidence derived from the completed resident node.
pub(crate) struct ConstructedNodeMetadata {
    pub(crate) tex82_words: (usize, usize),
    pub(crate) etex_words: (usize, usize),
    pub(crate) font: Option<crate::ids::FontId>,
}

/// Direct child coordinates decoded from one compact resident record.
///
/// Compact records keep list handles in their typed annex, so they cannot
/// implement [`RegionValue`] by themselves without borrowing that annex.  A
/// small fixed projection lets construction validate dependencies after the
/// destination is initialized without expanding the record into an owned
/// [`Node`].
struct NodeRecordDependencies {
    lists: [Option<ArenaListId<PageMaterialLane>>; 12],
    len: usize,
}

impl NodeRecordDependencies {
    fn from_record(
        record: PageMaterialNode,
        annex: NodeAnnexView<'_>,
    ) -> Result<Self, ForkArenaError> {
        let mut dependencies = Self {
            lists: [None; 12],
            len: 0,
        };
        let valid = record.visit_node_lists(annex, |child| {
            if child.is_empty() {
                return;
            }
            let slot = dependencies
                .lists
                .get_mut(dependencies.len)
                .expect("a TeX node has at most twelve direct child lists");
            *slot = Some(child.coordinate());
            dependencies.len += 1;
        });
        valid.map_or(Err(ForkArenaError::InvalidRange), |_| Ok(dependencies))
    }
}

impl RegionValue<PageMaterialLane> for NodeRecordDependencies {
    fn visit_region_lists(&self, visit: &mut dyn FnMut(ArenaListId<PageMaterialLane>)) {
        for list in self.lists[..self.len].iter().flatten().copied() {
            visit(list);
        }
    }

    fn rebrand_region_lists(&mut self, _destination_arena: u32) {}
}

/// Persistent coordinate-only construction state for one active node list.
#[must_use = "a page-material active list must be finalized or rolled back"]
pub struct PageMaterialActiveListBuilder {
    inner: ActiveListBuilder<PageMaterialNode, PageMaterialLane>,
    identity: Option<SemanticSequenceIdentity>,
    identity_work: crate::fork_arena::SequenceSummaryWork,
    fresh_only: bool,
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
            fresh_only: true,
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

/// Freshly finalized active-list segment for a generated semantic tape.
/// Unlike a reclaimed unlinked head, this can only come from a builder that
/// owned and published its direct records in the current operation.
pub struct FreshGeneratedSegment {
    unique: UniquePageList,
}

impl FreshGeneratedSegment {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.unique.is_empty()
    }
}

/// One semantic box consumption grants one shallow projection of its children.
/// A linked physical head is valid here; it only rules out a direct splice.
pub struct ConsumedBoxChildren {
    list: PageListId,
}

impl ConsumedBoxChildren {
    #[must_use]
    pub const fn list(&self) -> PageListId {
        self.list
    }
}

impl UniquePageList {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.coordinate.is_empty()
    }

    pub(crate) fn list(&self) -> PageListId {
        PageListId::from_parts(self.coordinate.coordinate(), self.identity)
    }

    fn publish(self) -> PageListId {
        PageListId::from_parts(self.coordinate.publish(), self.identity)
    }
}

impl PageListId {
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

/// Checked owner-local page-list span for retention in page and mode state.
///
/// The constructor is private to [`PageMaterialArena`]. Operations which
/// traverse repeatedly promote this compact retained root to
/// [`AdmittedPageList`] once rather than inflating every durable root with an
/// endpoint proof. Full chain audits remain at cold transfer and test ingress.
pub struct PageListSpan {
    list: PageListId,
}

/// The executor's actual semantic mode-list slot. Retained journal and
/// checkpoint projections may carry the same copyable span, but they cannot
/// carry this slot's direct-record move authority. Raw restoration marks the
/// slot conservative until a new empty or freshly constructed list replaces
/// that history.
pub struct ModePageListSlot {
    span: PageListSpan,
    move_enabled: bool,
}

impl Default for ModePageListSlot {
    fn default() -> Self {
        Self {
            span: PageListSpan::empty(),
            move_enabled: true,
        }
    }
}

impl core::fmt::Debug for ModePageListSlot {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ModePageListSlot")
            .field("span", &self.span)
            .field("move_enabled", &self.move_enabled)
            .finish()
    }
}

impl PartialEq for ModePageListSlot {
    fn eq(&self, other: &Self) -> bool {
        self.span == other.span
    }
}

impl ModePageListSlot {
    #[must_use]
    pub const fn span(&self) -> PageListSpan {
        self.span
    }

    #[must_use]
    pub const fn list(&self) -> PageListId {
        self.span.list()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.span.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.span.is_empty()
    }

    /// A copyable historical root remains readable but cannot authorize a
    /// later generated-box move merely by being restored to a live mode.
    #[must_use]
    pub const fn retained_snapshot(span: PageListSpan) -> Self {
        Self {
            span,
            move_enabled: false,
        }
    }

    pub fn replace_retained(&mut self, span: PageListSpan) {
        self.span = span;
        self.move_enabled = false;
    }

    #[must_use]
    pub fn take(&mut self) -> PageListSpan {
        let old = self.span;
        *self = Self::default();
        old
    }

    #[cfg(test)]
    pub(crate) const fn synthetic_semantic_source(span: PageListSpan) -> Self {
        Self {
            span,
            move_enabled: true,
        }
    }
}

/// Operation-local page-list admission. Unlike retained [`PageListSpan`]
/// roots, this value carries the resolved compact endpoint proof through one
/// traversal without inflating mode, page, or rollback state.
#[derive(Clone, Copy)]
pub struct AdmittedPageList {
    span: PageListSpan,
    admission: crate::fork_arena::AdmittedListRoot<PageMaterialLane>,
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

    #[must_use]
    pub const fn owner_position(&self) -> usize {
        self.inner.owner_position()
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

impl AdmittedPageList {
    #[must_use]
    pub const fn span(self) -> PageListSpan {
        self.span
    }

    #[must_use]
    pub const fn list(self) -> PageListId {
        self.span.list()
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.span.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.span.is_empty()
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

/// Construction path for profiling a box whose child predates its build mark.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltBoxOrigin {
    SetBoxConstruction,
    SetBoxRegisterTake,
    SetBoxRegisterCopy,
    SetBoxVSplit,
    LastBox,
    PdfForm,
    Other,
    #[cfg(feature = "profiling")]
    SetBoxRegisterTakeOutputCarrier,
    #[cfg(feature = "profiling")]
    SetBoxRegisterTakeOutputRetainedPage,
    #[cfg(feature = "profiling")]
    SetBoxRegisterTakeOutputPendingSuccessor,
    #[cfg(feature = "profiling")]
    SetBoxRegisterTakeOutputModeRoots,
    #[cfg(feature = "profiling")]
    SetBoxRegisterTakeOutputActiveBox,
    #[cfg(feature = "profiling")]
    SetBoxRegisterTakeOutputPendingLoan,
    #[cfg(feature = "profiling")]
    SetBoxRegisterTakeOutputReadyArmed,
    #[cfg(feature = "profiling")]
    SetBoxRegisterTakeOutputReadyUnarmed,
    #[cfg(feature = "profiling")]
    SetBoxRegisterTakeRetainedDurable,
    #[cfg(feature = "profiling")]
    SetBoxRegisterTakeMissingSource,
}

/// Demand-free observations of explicit durable lifetime transitions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DurableTransitionCounters {
    pub(crate) page_to_durable_nodes_copied: u64,
    pub(crate) interleaved_box_wrappers_built: u64,
    pub(crate) tex_copy_nodes_copied: u64,
    pub(crate) history_preservation_nodes_copied: u64,
    pub(crate) nested_closure_nodes_copied: u64,
    pub(crate) node_closure_scan_nodes: u64,
}

/// Exclusive admitted access to one page-material region and the shared pool.
pub struct PageMaterialArena<'a> {
    pool: &'a mut NodePool,
    region: &'a mut NodeRegion<PageRole>,
    semantic_identity_enabled: &'a mut bool,
    durable_transitions: &'a mut DurableTransitionCounters,
    #[cfg(feature = "profiling")]
    output_history_probe: PageOutputHistoryProbe,
}

/// Read-only state of the actual page-history owner at its command lend.
/// This is diagnostic data, never transfer authority.
#[cfg(feature = "profiling")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageOutputHistoryProbe {
    Unavailable,
    RetainedPage,
    PendingSuccessor,
    Ready,
}

impl PageMaterialRegion {
    pub fn new(pool: &mut NodePool) -> Self {
        let region = pool
            .start_region()
            .expect("page-material region identity capacity");
        Self {
            region,
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
    pub fn finalize_generated_active_segment(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
    ) -> Result<FreshGeneratedSegment, ForkArenaError> {
        if !builder.fresh_only {
            return Err(ForkArenaError::InvalidActiveListBuilder);
        }
        Ok(FreshGeneratedSegment {
            unique: self.finalize_unique_active_list(builder)?,
        })
    }

    pub fn closure_transition_counters(&self) -> crate::node_region::ClosureTransitionCounters {
        self.pool.closure_transition_counters()
    }

    pub fn new(pool: &'a mut NodePool, state: &'a mut PageMaterialRegion) -> Self {
        Self {
            pool,
            region: &mut state.region,
            semantic_identity_enabled: &mut state.semantic_identity_enabled,
            durable_transitions: &mut state.durable_transitions,
            #[cfg(feature = "profiling")]
            output_history_probe: PageOutputHistoryProbe::Unavailable,
        }
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn with_output_history_probe(mut self, probe: PageOutputHistoryProbe) -> Self {
        self.output_history_probe = probe;
        self
    }

    #[cfg(feature = "profiling")]
    pub(crate) const fn output_history_probe(&self) -> PageOutputHistoryProbe {
        self.output_history_probe
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
        let nodes = self
            .node_cursor(source)?
            .iter()
            .map(|node| node.to_owned())
            .collect::<Vec<_>>();
        self.region
            .pub_arena
            .record_source_nodes_copied(nodes.len());
        self.publish_owned(nodes)
    }

    /// Cold-copies a page root into a fresh self-contained durable region.
    ///
    /// Callers use this when the selected list is already part of the shared
    /// page region and cannot be detached independently. Fresh box/form
    /// construction instead opens a closure suffix and transfers its paired
    /// node-and-annex envelope without copying.
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
        let mut closure = durable
            .into_closure(self.pool, copied)
            .map_err(|(error, region)| {
                assert!(
                    self.pool.retire_region(region).is_ok(),
                    "validated durable copy destination retires"
                );
                error
            })?;
        closure.profiling_mark_fresh_recursive_copy();
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
        self.finish_built_page_root_to_durable_from(mark, root, BuiltBoxOrigin::Other)
    }

    fn finish_built_page_root_to_durable_from(
        &mut self,
        mark: ClosureBuildMark<PageRole>,
        root: PageListId,
        _origin: BuiltBoxOrigin,
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
                #[cfg(feature = "profiling")]
                let shape = crate::measurement::box_fallback_census_enabled()
                    .then(|| self.profile_built_fallback_shape(root));
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
                let mut owner =
                    durable
                        .into_closure(self.pool, copied)
                        .map_err(|(error, region)| {
                            assert!(
                                self.pool.retire_region(region).is_ok(),
                                "validated structural destination retires"
                            );
                            error
                        })?;
                owner.profiling_mark_fresh_recursive_copy();
                self.durable_transitions.page_to_durable_nodes_copied = self
                    .durable_transitions
                    .page_to_durable_nodes_copied
                    .saturating_add(copied_nodes);
                self.durable_transitions.node_closure_scan_nodes = self
                    .durable_transitions
                    .node_closure_scan_nodes
                    .saturating_add(copied_nodes);
                #[cfg(feature = "profiling")]
                if let Some(shape) = shape {
                    crate::measurement::record_box_fallback(_origin, shape, copied_nodes);
                }
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

    /// Moves an exclusively consumed anonymous box from an older page
    /// construction interval. Its loan is retained by the operation journal
    /// until the assignment commits or is rolled back.
    pub(crate) fn finish_interleaved_page_box(
        &mut self,
        root: PageListId,
        metadata: PageBoxMigrationMetadata,
        reset_shift: bool,
    ) -> Result<
        (
            DurableNodeClosure,
            crate::node_region::PageInteriorTransferLoan,
        ),
        ForkArenaError,
    > {
        if metadata.positive.is_some() {
            let mut durable = self.pool.start_region::<DurableRole>()?;
            let result =
                self.finish_generated_inline_box(root, &metadata, reset_shift, &mut durable);
            let (durable_root, loan) = match result {
                Ok(value) => value,
                Err(error) => {
                    assert!(self.pool.retire_region(durable).is_ok());
                    return Err(error);
                }
            };
            self.durable_transitions.interleaved_box_wrappers_built = self
                .durable_transitions
                .interleaved_box_wrappers_built
                .saturating_add(1);
            let owner = durable
                .into_closure(self.pool, durable_root)
                .unwrap_or_else(|(error, _)| panic!("preflighted generated root: {error:?}"));
            return Ok((owner, loan));
        }
        if self.box_migration_metadata(root).as_ref() != Some(&metadata) {
            return Err(ForkArenaError::InvalidRegion);
        }
        let segment = metadata.segment;
        let source_root = self.region.root(self.pool, root)?;
        let mut durable = self.pool.start_region::<DurableRole>()?;
        let partitioned =
            reset_shift || metadata.wrapper_rebuild || !metadata.exclusions.is_empty();
        let whole = if partitioned {
            Err(ForkArenaError::InvalidRegion)
        } else {
            preflight_page_interior_closure(
                self.pool,
                self.region,
                source_root,
                segment.node_range(),
                segment.annex_range(),
                &durable,
            )
        };
        let result = if partitioned {
            let (mut wrapper, nodes, annex) =
                match self.preflight_partitioned_box_body(root, &metadata, &durable) {
                    Ok(value) => value,
                    Err(error) => {
                        assert!(self.pool.retire_region(durable).is_ok());
                        return Err(error);
                    }
                };
            if reset_shift {
                match &mut wrapper {
                    Node::HList(boxed) | Node::VList(boxed) => {
                        boxed.shift = crate::scaled::Scaled::from_raw(0);
                    }
                    _ => unreachable!("preflighted box wrapper"),
                }
            }
            transfer_page_interior_intervals(self.pool, self.region, &nodes, &annex, &mut durable)
                .and_then(|loan| {
                    match durable.publish_box_wrapper(self.pool, wrapper, root.sequence_identity())
                    {
                        Ok(root) => Ok((root, loan)),
                        Err(error) => {
                            rollback_page_interior_closure(
                                self.pool,
                                self.region,
                                &mut durable,
                                loan,
                            )
                            .expect("failed wrapper construction returns its partitioned loan");
                            Err(error)
                        }
                    }
                })
        } else if whole.is_ok() {
            transfer_page_interior_closure(
                self.pool,
                self.region,
                source_root,
                segment.node_range(),
                segment.annex_range(),
                &mut durable,
            )
        } else {
            let (wrapper, child) =
                match self.preflight_interleaved_box_body(root, segment, &durable) {
                    Ok(value) => value,
                    Err(error) => {
                        assert!(self.pool.retire_region(durable).is_ok());
                        return Err(error);
                    }
                };
            let loaned = if let Some(child) = child {
                let source_child = self.region.root(self.pool, child)?;
                transfer_page_interior_closure(
                    self.pool,
                    self.region,
                    source_child,
                    segment.body_node_range(),
                    segment.body_annex_range(),
                    &mut durable,
                )
                .map(|(_, loan)| loan)
            } else {
                loan_empty_page_box_body(
                    self.pool,
                    self.region,
                    segment.body_node_range().start,
                    segment.body_annex_range().start,
                    &mut durable,
                )
            };
            loaned.and_then(|loan| {
                match durable.publish_box_wrapper(self.pool, wrapper, root.sequence_identity()) {
                    Ok(root) => Ok((root, loan)),
                    Err(error) => {
                        rollback_page_interior_closure(self.pool, self.region, &mut durable, loan)
                            .expect("failed wrapper construction returns its exact body loan");
                        Err(error)
                    }
                }
            })
        };
        let (durable_root, loan) = match result {
            Ok(result) => result,
            Err(error) => {
                assert!(self.pool.retire_region(durable).is_ok());
                return Err(error);
            }
        };
        if whole.is_err() {
            self.durable_transitions.interleaved_box_wrappers_built = self
                .durable_transitions
                .interleaved_box_wrappers_built
                .saturating_add(1);
        }
        let owner = durable
            .into_closure(self.pool, durable_root)
            .unwrap_or_else(|(error, _)| panic!("preflighted durable root: {error:?}"));
        Ok((owner, loan))
    }

    /// Determines whether this consumed box's exact sealed interval can be
    /// detached. This check runs before assignment tracing, which may itself
    /// publish page material for the old value.
    pub(crate) fn can_finish_interleaved_page_box(
        &mut self,
        root: PageListId,
        metadata: &PageBoxMigrationMetadata,
        reset_shift: bool,
    ) -> bool {
        if metadata.positive.is_some() {
            let _ = reset_shift;
            return self.preflight_generated_inline_box(root, metadata).is_ok();
        }
        if self.box_migration_metadata(root).as_ref() != Some(metadata) {
            return false;
        }
        let segment = metadata.segment;
        let Ok(source_root) = self.region.root(self.pool, root) else {
            return false;
        };
        let Ok(destination) = self.pool.start_region::<DurableRole>() else {
            return false;
        };
        let result = if !reset_shift && !metadata.wrapper_rebuild && metadata.exclusions.is_empty()
        {
            preflight_page_interior_closure(
                self.pool,
                self.region,
                source_root,
                segment.node_range(),
                segment.annex_range(),
                &destination,
            )
        } else {
            Err(ForkArenaError::InvalidRegion)
        };
        let eligible =
            ((reset_shift || metadata.wrapper_rebuild || !metadata.exclusions.is_empty())
                && self
                    .preflight_partitioned_box_body(root, metadata, &destination)
                    .is_ok())
                || result.is_ok()
                || self
                    .preflight_interleaved_box_body(root, segment, &destination)
                    .is_ok()
                    && metadata.exclusions.is_empty()
                    && !metadata.wrapper_rebuild
                    && !reset_shift;
        assert!(self.pool.retire_region(destination).is_ok());
        eligible
    }

    fn preflight_interleaved_box_body(
        &self,
        root: PageListId,
        segment: crate::node_region::PageBoxSegment,
        destination: &NodeRegion<DurableRole>,
    ) -> Result<(Node<PageListId>, Option<PageListId>), ForkArenaError> {
        if root.len() != 1 || segment.region() != self.region.id() {
            return Err(ForkArenaError::InvalidRegion);
        }
        let wrapper_range = segment.node_range().end - 1..segment.node_range().end;
        self.region.pub_arena.preflight_interval_root(
            &self.pool.chunks,
            root.coordinate(),
            wrapper_range.start,
            wrapper_range.end,
        )?;
        let wrapper = self
            .node_cursor(root)?
            .first()
            .ok_or(ForkArenaError::InvalidRange)?
            .to_owned();
        let boxed = match &wrapper {
            Node::HList(boxed) | Node::VList(boxed) => boxed,
            _ => return Err(ForkArenaError::InvalidRange),
        };
        let body_nodes = segment.body_node_range();
        let body_annex = segment.body_annex_range();
        let diagnostic = boxed.diagnostic_children.filter(|root| !root.is_empty());
        let child = (!boxed.children.is_empty())
            .then_some(boxed.children)
            .or(diagnostic);
        let Some(child) = child else {
            if !body_nodes.is_empty() || !body_annex.is_empty() {
                return Err(ForkArenaError::InvalidRegion);
            }
            preflight_empty_page_box_body(
                self.pool,
                self.region,
                body_nodes.start,
                body_annex.start,
                destination,
            )?;
            return Ok((wrapper, None));
        };
        preflight_page_interior_closure(
            self.pool,
            self.region,
            self.region.root(self.pool, child)?,
            body_nodes.clone(),
            body_annex,
            destination,
        )?;
        if let Some(diagnostic) = diagnostic {
            self.region.pub_arena.preflight_interval_root(
                &self.pool.chunks,
                diagnostic.coordinate(),
                body_nodes.start,
                body_nodes.end,
            )?;
        }
        Ok((wrapper, Some(child)))
    }

    fn preflight_partitioned_box_body(
        &self,
        root: PageListId,
        metadata: &PageBoxMigrationMetadata,
        destination: &NodeRegion<DurableRole>,
    ) -> Result<PreflightedBoxBody, ForkArenaError> {
        let segment = metadata.segment;
        if root.len() != 1 || segment.region() != self.region.id() {
            return Err(ForkArenaError::InvalidRegion);
        }
        let wrapper_range = segment.node_range().end - 1..segment.node_range().end;
        self.region.pub_arena.preflight_interval_root(
            &self.pool.chunks,
            root.coordinate(),
            wrapper_range.start,
            wrapper_range.end,
        )?;
        let wrapper = self
            .node_cursor(root)?
            .first()
            .ok_or(ForkArenaError::InvalidRange)?
            .to_owned();
        let boxed = match &wrapper {
            Node::HList(boxed) | Node::VList(boxed) => boxed,
            _ => return Err(ForkArenaError::InvalidRange),
        };
        let (mut nodes, mut annex) = metadata
            .selected_body_ranges()
            .ok_or(ForkArenaError::InvalidRange)?;
        let child = (!boxed.children.is_empty())
            .then_some(boxed.children)
            .or(boxed.diagnostic_children.filter(|root| !root.is_empty()));
        if child.is_none() {
            nodes.clear();
            annex.clear();
        }
        for child in [Some(boxed.children), boxed.diagnostic_children]
            .into_iter()
            .flatten()
        {
            if child.is_empty() {
                continue;
            }
            let range = self
                .region
                .pub_arena
                .owner_relative_list_block_range(&self.pool.chunks, child.coordinate())?;
            let selected = |position| {
                let index = nodes.partition_point(|candidate| candidate.end <= position);
                nodes.get(index).is_some_and(|candidate| {
                    candidate.start <= position && position < candidate.end
                })
            };
            if !selected(range.start) || !selected(range.end - 1) {
                return Err(ForkArenaError::InvalidRegion);
            }
        }
        preflight_page_interior_intervals(self.pool, self.region, &nodes, &annex, destination)?;
        Ok((wrapper, nodes, annex))
    }

    pub(crate) fn rollback_interleaved_page_box(
        &mut self,
        owner: &mut Option<DurableNodeClosure>,
        loan: crate::node_region::PageInteriorTransferLoan,
    ) -> Result<(), ForkArenaError> {
        let region = owner
            .as_mut()
            .ok_or(ForkArenaError::InvalidRegion)?
            .region_mut();
        rollback_page_interior_closure(self.pool, self.region, region, loan)?;
        self.retire_durable_in_place(owner)
    }

    /// Publishes a built closure without detaching a construction-suffix root
    /// still owned by the page builder.
    pub(crate) fn finish_built_page_root_to_durable_preserving_roots<const N: usize>(
        &mut self,
        mark: ClosureBuildMark<PageRole>,
        root: PageListId,
        retained_roots: [PageListId; N],
        origin: BuiltBoxOrigin,
    ) -> Result<DurableNodeClosure, ForkArenaError> {
        if !self
            .region
            .build_suffix_contains_any_root(self.pool, &mark, retained_roots)?
        {
            return self.finish_built_page_root_to_durable_from(mark, root, origin);
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
        let mut owner = durable
            .into_closure(self.pool, copied)
            .map_err(|(error, region)| {
                assert!(
                    self.pool.retire_region(region).is_ok(),
                    "validated retained-root destination retires"
                );
                error
            })?;
        owner.profiling_mark_fresh_recursive_copy();
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

    pub(crate) fn move_durable_to_page_in_place(
        &mut self,
        closure: &mut Option<DurableNodeClosure>,
    ) -> Result<PageListId, ForkArenaError> {
        let owner = closure.as_mut().ok_or(ForkArenaError::InvalidRegion)?;
        let root = transfer_closure_into(self.pool, owner, self.region)?.list();
        *closure = None;
        Ok(root)
    }

    pub(crate) fn loan_durable_to_page_in_place(
        &mut self,
        closure: &mut Option<DurableNodeClosure>,
    ) -> Result<(PageListId, DurableTransferLoan), ForkArenaError> {
        let build = self.region.begin_closure_build(self.pool)?;
        let owner = closure.as_mut().ok_or(ForkArenaError::InvalidRegion)?;
        let root = match transfer_closure_into(self.pool, owner, self.region) {
            Ok(root) => root.list(),
            Err(error) => {
                self.region
                    .cancel_closure_build(self.pool, build)
                    .expect("empty failed transfer suffix rolls back");
                return Err(error);
            }
        };
        *closure = None;
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
        #[cfg(feature = "profiling")]
        let marked = closure.profiling_is_fresh_recursive_copy();
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
        #[cfg(feature = "profiling")]
        crate::measurement::record_durable_source_copy(
            crate::measurement::DurableSourceCopyKind::ExplicitToPage,
            marked,
            copied,
        );
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
        #[cfg(feature = "profiling")]
        let marked = closure.profiling_is_fresh_recursive_copy();
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
        #[cfg(feature = "profiling")]
        crate::measurement::record_durable_source_copy(
            crate::measurement::DurableSourceCopyKind::HistoryToPage,
            marked,
            copied,
        );
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
        #[cfg(feature = "profiling")]
        let marked = closure.profiling_is_fresh_recursive_copy();
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
        let mut closure =
            destination
                .into_closure(self.pool, copied)
                .map_err(|(error, region)| {
                    assert!(
                        self.pool.retire_region(region).is_ok(),
                        "validated durable copy destination retires"
                    );
                    error
                })?;
        #[cfg(feature = "profiling")]
        crate::measurement::record_durable_source_copy(
            crate::measurement::DurableSourceCopyKind::DurableOwner,
            marked,
            copied_nodes,
        );
        closure.profiling_mark_fresh_recursive_copy();
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
    ) -> Result<crate::node_view::NodeCursor<'b>, ForkArenaError> {
        closure.list(self.pool)
    }

    pub(crate) fn set_durable_root_box_dimension(
        &mut self,
        closure: &mut DurableNodeClosure,
        dimension: crate::command_context::BoxDimension,
        value: crate::scaled::Scaled,
    ) -> Result<crate::scaled::Scaled, ForkArenaError> {
        closure.set_root_box_dimension(self.pool, dimension, value)
    }

    /// Edits the page-owned output wrapper, leaving its child closure and
    /// stable coordinate in the page region. A retained fork cannot edit a
    /// shared annex chunk: the fixed-word mutation checks exclusive lineage.
    pub(crate) fn set_page_root_box_dimension(
        &mut self,
        root: PageListId,
        dimension: crate::command_context::BoxDimension,
        value: crate::scaled::Scaled,
    ) -> Result<(crate::scaled::Scaled, PageListId), ForkArenaError> {
        if root.len() != 1 {
            return Err(ForkArenaError::InvalidRange);
        }
        let record = *self
            .region
            .pub_arena
            .list(&self.pool.chunks, root.coordinate())?
            .get(0)
            .ok_or(ForkArenaError::InvalidRange)?;
        let previous = crate::node_record::set_root_box_dimension(
            record,
            &mut self.pool.annex_chunks,
            &mut self.region.annex_arena,
            dimension,
            value,
        )?;
        let identity = root.semantic_identity().map(|_| {
            let annex = NodeAnnexView::new(&self.pool.annex_chunks, &self.region.annex_arena);
            let mut identity = SemanticSequenceIdentity::empty();
            identity.push_back(record.semantic_identity(annex));
            identity
        });
        Ok((
            previous,
            PageListId::from_parts(root.coordinate(), identity),
        ))
    }

    pub(crate) fn durable_child_list<'b>(
        &'b self,
        closure: &'b DurableNodeClosure,
        child: PageListId,
    ) -> Result<crate::node_view::NodeCursor<'b>, ForkArenaError> {
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

    /// Retires a closure in its authoritative owner slot. This avoids moving
    /// the complete region envelope through every journal retirement; after
    /// success the slot is vacant and may be reused by the durable owner store.
    pub(crate) fn retire_durable_in_place(
        &mut self,
        closure: &mut Option<DurableNodeClosure>,
    ) -> Result<(), ForkArenaError> {
        let Some(owner) = closure.as_mut() else {
            return Ok(());
        };
        self.pool.retire_closure_in_place(owner)?;
        *closure = None;
        Ok(())
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
        builder.fresh_only = true;
        Ok(())
    }

    pub fn push_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        node: OwnedPageMaterialNode,
    ) -> Result<(), ForkArenaError> {
        self.construct_active_list(builder, |destination| destination.owned(node))
            .map(|_| ())
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
        let constructed = {
            let mut annex =
                NodeAnnexWriter::new(&mut self.pool.annex_chunks, &mut self.region.annex_arena);
            self.region.pub_arena.construct_region_value_active_list(
                &mut self.pool.chunks,
                &mut builder.inner,
                identity_enabled,
                &mut annex,
                |slot, annex| initialize(crate::NodeDestination::new_record(slot, annex)),
                |record, annex| {
                    let annex_dependency_floor = annex.dependency_floor();
                    let annex = annex.view();
                    let font = record.direct_font(annex);
                    let dependencies = NodeRecordDependencies::from_record(*record, annex)?;
                    let metadata = ConstructedNodeMetadata {
                        tex82_words: record.tex_memory_words(annex, false),
                        etex_words: record.tex_memory_words(annex, true),
                        font,
                    };
                    let identity = if identity_enabled {
                        record.semantic_identity(annex)
                    } else {
                        0
                    };
                    Ok((identity, metadata, dependencies, annex_dependency_floor))
                },
            )
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
        self.append_reencoded_span_range(builder, span, 0..span.len(), true, false, false)
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
        builder.fresh_only = false;
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
        self.append_reencoded_span_range(builder, span, selected, false, false, false)
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
        let coordinate = self
            .region
            .pub_arena
            .finish_active_list(&mut self.pool.chunks, &mut builder.inner);
        self.region.active_annex_operation = None;
        self.region
            .pub_arena
            .record_identity_work(builder.identity_work);
        builder.identity_work = crate::fork_arena::SequenceSummaryWork::default();
        builder.fresh_only = true;
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
        builder.fresh_only = true;
        Ok(())
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
            self.append_reencoded_span_range(
                &mut builder,
                span,
                0..span.len(),
                false,
                false,
                false,
            )?;
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
    ) -> Result<PageListId, ForkArenaError> {
        if *self.semantic_identity_enabled {
            let annex = NodeAnnexView::new(&self.pool.annex_chunks, &self.region.annex_arena);
            let (coordinate, identity, work) = self.region.pub_arena.slice_list_summarized(
                &mut self.pool.chunks,
                list.coordinate(),
                selected,
                |record| semantic_record_identity(record, annex),
            )?;
            self.region.pub_arena.record_identity_work(work);
            Ok(PageListId::from_parts(coordinate, Some(identity)))
        } else {
            let coordinate = self.region.pub_arena.slice_list(
                &mut self.pool.chunks,
                list.coordinate(),
                selected,
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
                    |record| semantic_record_identity(record, annex),
                )?;
            self.region.pub_arena.record_identity_work(work);
            PageListId::from_parts(coordinate, Some(identity))
        } else {
            let coordinate = self.region.pub_arena.slice_validated_list(
                &mut self.pool.chunks,
                span.list.coordinate(),
                selected,
            )?;
            PageListId::from_parts(coordinate, None)
        };
        self.admit_span(list)
    }

    pub fn admit_span(&self, list: PageListId) -> Result<PageListSpan, ForkArenaError> {
        self.region
            .pub_arena
            .admit_owned_list(&self.pool.chunks, list.coordinate())?;
        Ok(PageListSpan { list })
    }

    pub fn admit_page_list(&self, list: PageListId) -> Result<AdmittedPageList, ForkArenaError> {
        let admission = self
            .region
            .pub_arena
            .admit_owned_root(&self.pool.chunks, list.coordinate())?;
        Ok(AdmittedPageList {
            span: PageListSpan { list },
            admission,
        })
    }

    pub fn node_cursor(
        &self,
        list: PageListId,
    ) -> Result<crate::node_view::NodeCursor<'_>, ForkArenaError> {
        self.admit_span(list)
            .and_then(|span| self.span_node_cursor(span))
    }

    pub fn span_node_cursor(
        &self,
        span: PageListSpan,
    ) -> Result<crate::node_view::NodeCursor<'_>, ForkArenaError> {
        self.region
            .pub_arena
            .validated_list(&self.pool.chunks, span.list.coordinate())
            .map(|view| crate::node_view::NodeCursor::fork_arena(view, self.annex_view()))
    }

    pub fn admitted_node_cursor(
        &self,
        list: AdmittedPageList,
    ) -> Result<crate::node_view::NodeCursor<'_>, ForkArenaError> {
        let view = self.region.pub_arena.admitted_view(
            &self.pool.chunks,
            list.span.list.coordinate(),
            list.admission,
        )?;
        Ok(crate::node_view::NodeCursor::fork_arena(
            view,
            self.annex_view(),
        ))
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

    /// Walks only the top-level records of one authenticated list. Hpack
    /// consumes this input list before publishing its retained projection;
    /// its former chunks can then remain page-owned across a box-body loan.
    pub fn list_chunk_positions(&self, list: PageListId) -> Result<Vec<usize>, ForkArenaError> {
        let span = self.admit_span(list)?;
        let mut positions = Vec::new();
        let mut cursor = self.span_tail_chunk(span)?;
        while let Some(chunk) = cursor {
            positions.push(chunk.owner_position());
            cursor = self.span_previous_chunk(&chunk)?;
        }
        positions.reverse();
        positions.dedup();
        Ok(positions)
    }

    pub fn admitted_tail_chunk(
        &self,
        list: AdmittedPageList,
    ) -> Result<Option<PageListChunkCursor>, ForkArenaError> {
        self.region
            .pub_arena
            .admitted_tail_chunk_from_root(
                &self.pool.chunks,
                list.span.list.coordinate(),
                list.admission,
            )
            .map(|cursor| {
                cursor.map(|inner| PageListChunkCursor {
                    span: list.span,
                    inner,
                })
            })
    }

    /// Returns the preceding source chunk through its sole persistent edge.
    pub fn span_previous_chunk(
        &self,
        cursor: &PageListChunkCursor,
    ) -> Result<Option<PageListChunkCursor>, ForkArenaError> {
        self.region
            .pub_arena
            .admitted_previous_chunk(&self.pool.chunks, &cursor.inner)
            .map(|previous| {
                previous.map(|inner| PageListChunkCursor {
                    span: cursor.span,
                    inner,
                })
            })
    }

    /// Borrows one node directly from a retained packed-chunk coordinate.
    pub fn span_chunk_node_at(
        &self,
        cursor: &PageListChunkCursor,
        offset: usize,
    ) -> (usize, PageMaterialNodeRef<'_>) {
        let (index, record) =
            self.region
                .pub_arena
                .admitted_chunk_value_at(&self.pool.chunks, &cursor.inner, offset);
        (
            index,
            PageMaterialNodeRef {
                record,
                annex: self.annex_view(),
            },
        )
    }

    /// Advances one admitted packed-chunk cursor without replaying root or
    /// range validation.
    pub fn span_next_chunk_node(
        &self,
        cursor: &mut PageListChunkCursor,
    ) -> Option<(usize, PageMaterialNodeRef<'_>)> {
        self.region
            .pub_arena
            .admitted_next_chunk_value(&self.pool.chunks, &mut cursor.inner)
            .map(|(index, record)| {
                (
                    index,
                    PageMaterialNodeRef {
                        record,
                        annex: self.annex_view(),
                    },
                )
            })
    }

    #[must_use]
    pub fn contains(&self, list: PageListId) -> bool {
        self.region
            .pub_arena
            .validated_list(&self.pool.chunks, list.coordinate())
            .is_ok_and(|view| page_list_is_decodable(view, self.annex_view()))
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

    pub fn close_box_segment(
        &mut self,
        start: ClosureBuildMark<PageRole>,
    ) -> Result<crate::node_region::PageBoxSegment, ForkArenaError> {
        if start.region_id() != self.region.id() {
            return Err(ForkArenaError::InvalidRegion);
        }
        let end = self.region.begin_closure_build(self.pool)?;
        crate::node_region::PageBoxSegment::from_boundaries(start, end)
    }

    pub fn box_segment_at_current_end(
        &self,
        start: &ClosureBuildMark<PageRole>,
    ) -> Result<crate::node_region::PageBoxSegment, ForkArenaError> {
        if start.region_id() != self.region.id() {
            return Err(ForkArenaError::InvalidRegion);
        }
        crate::node_region::PageBoxSegment::from_live_end(
            start,
            self.region.pub_arena.payload_position_end(),
            self.region.annex_arena.payload_position_end(),
        )
    }

    /// Seals the body before the one-record wrapper is published. This
    /// guarantees the stamp's fixed payload remains in that wrapper's own
    /// annex chunk, with an end coordinate known before final sealing.
    pub fn rotate_box_wrapper_tail(&mut self) -> Result<(), ForkArenaError> {
        self.region.seal_checkpoint_boundary(self.pool).map(|_| ())
    }

    pub fn publish_box_migration_segments(
        &mut self,
        exclusions: &[crate::node_region::PageBoxSegment],
        obsolete_input_positions: &[usize],
    ) -> Result<Option<PageBoxMigrationKey>, ForkArenaError> {
        if exclusions.is_empty() && obsolete_input_positions.is_empty() {
            return Ok(None);
        }
        if !crate::node_record::valid_box_exclusions(self.region.id().words(), exclusions)
            || obsolete_input_positions
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(ForkArenaError::InvalidRange);
        }
        // The consumed hpack source list's top-level chunks are obsolete
        // after its retained projection is published. Leave exactly those
        // chunks on the page, without excluding their nested child closures.
        let mut merged = Vec::with_capacity(exclusions.len() + obsolete_input_positions.len());
        let mut excluded = exclusions.iter().copied().peekable();
        let mut annex_anchor = exclusions
            .first()
            .map_or(self.region.annex_arena.payload_position_end(), |first| {
                first.annex_range().start
            });
        for &position in obsolete_input_positions {
            while excluded
                .peek()
                .is_some_and(|segment| segment.node_range().end <= position)
            {
                let segment = excluded.next().expect("peeked segment exists");
                annex_anchor = segment.annex_range().end;
                merged.push(segment);
            }
            if excluded.peek().is_some_and(|segment| {
                let range = segment.node_range();
                range.start <= position && position < range.end
            }) {
                continue;
            }
            let bounds = [
                u32::try_from(position).map_err(|_| ForkArenaError::CapacityOverflow)?,
                u32::try_from(position + 1).map_err(|_| ForkArenaError::CapacityOverflow)?,
                u32::try_from(annex_anchor).map_err(|_| ForkArenaError::CapacityOverflow)?,
                u32::try_from(annex_anchor).map_err(|_| ForkArenaError::CapacityOverflow)?,
            ];
            merged.push(
                crate::node_region::PageBoxSegment::from_exclusion_bounds(self.region.id(), bounds)
                    .ok_or(ForkArenaError::InvalidRange)?,
            );
        }
        merged.extend(excluded);
        if !crate::node_record::valid_box_exclusions(self.region.id().words(), &merged) {
            return Err(ForkArenaError::InvalidRange);
        }
        // The sidecar remains page-owned when the box body moves. Isolate its
        // paired chunk before publication; wrapper rotation seals it again.
        self.region.seal_checkpoint_boundary(self.pool)?;
        let mut annex =
            NodeAnnexWriter::new(&mut self.pool.annex_chunks, &mut self.region.annex_arena);
        Ok(Some(annex.publish_box_migration_segments(&merged)))
    }

    /// Publishes non-owning selected ranges for a generated box. The source
    /// may share cut boundary chunks; ownership is proved only when the
    /// consumed wrapper enters prepared transfer.
    pub(crate) fn publish_generated_box_body_ranges(
        &mut self,
        nodes: &[Range<usize>],
        annex: &[Range<usize>],
        node_cuts: &[PageBoxCutRange],
        annex_cuts: &[PageBoxCutRange],
    ) -> Result<PageBoxPositiveKey, ForkArenaError> {
        if !valid_positive_ranges(nodes, self.region.pub_arena.payload_position_end())
            || !valid_positive_ranges(annex, self.region.annex_arena.payload_position_end())
            || !valid_positive_cuts(
                node_cuts,
                nodes,
                self.region.pub_arena.payload_position_end(),
            )
            || !valid_positive_cuts(
                annex_cuts,
                annex,
                self.region.annex_arena.payload_position_end(),
            )
            || u32::try_from(nodes.len()).is_err()
            || u32::try_from(annex.len()).is_err()
            || u32::try_from(node_cuts.len()).is_err()
            || u32::try_from(annex_cuts.len()).is_err()
        {
            return Err(ForkArenaError::InvalidRange);
        }
        self.region.seal_checkpoint_boundary(self.pool)?;
        Ok(
            NodeAnnexWriter::new(&mut self.pool.annex_chunks, &mut self.region.annex_arena)
                .publish_box_positive_ranges(nodes, annex, node_cuts, annex_cuts),
        )
    }

    /// Binds a published positive sidecar to the actual isolated wrapper.
    /// A stamp is provenance only: consuming the wrapper and preparing the
    /// cut-boundary transfer are separate ownership requirements.
    pub(crate) fn stamp_generated_box_body(
        &mut self,
        root: PageListId,
        key: PageBoxPositiveKey,
    ) -> Result<PageBoxMigrationMetadata, ForkArenaError> {
        if root.len() != 1 {
            return Err(ForkArenaError::InvalidRange);
        }
        let record = *self
            .region
            .pub_arena
            .validated_list(&self.pool.chunks, root.coordinate())?
            .first()
            .ok_or(ForkArenaError::InvalidRange)?;
        let node_wrapper = self
            .region
            .pub_arena
            .owner_relative_list_block_range(&self.pool.chunks, root.coordinate())?;
        let annex_view = self.annex_view();
        let annex_wrapper = record
            .box_payload_block_range(annex_view)
            .ok_or(ForkArenaError::InvalidRange)?;
        if node_wrapper.end != node_wrapper.start + 1
            || annex_wrapper.end != annex_wrapper.start + 1
            || annex_view
                .box_positive_ranges(key, annex_wrapper.start, 0, 0, node_wrapper.start)
                .is_none()
        {
            return Err(ForkArenaError::InvalidRange);
        }
        let region = self.region.id();
        let mut writer =
            NodeAnnexWriter::new(&mut self.pool.annex_chunks, &mut self.region.annex_arena);
        record
            .stamp_box_positive(
                &mut writer,
                region,
                key,
                node_wrapper.start,
                annex_wrapper.start,
            )
            .ok_or(ForkArenaError::InvalidRange)?;
        self.box_migration_metadata(root)
            .ok_or(ForkArenaError::InvalidRange)
    }

    pub fn stamp_box_segment(
        &mut self,
        start: &ClosureBuildMark<PageRole>,
        root: PageListId,
        migrations: Option<PageBoxMigrationKey>,
    ) -> Result<crate::node_region::PageBoxSegment, ForkArenaError> {
        let segment = self.box_segment_at_current_end(start)?;
        let record = *self
            .region
            .pub_arena
            .validated_list(&self.pool.chunks, root.coordinate())?
            .first()
            .ok_or(ForkArenaError::InvalidRange)?;
        if root.len() != 1 {
            return Err(ForkArenaError::InvalidRange);
        }
        {
            let mut annex =
                NodeAnnexWriter::new(&mut self.pool.annex_chunks, &mut self.region.annex_arena);
            record
                .stamp_box_segment(&mut annex, segment, migrations)
                .ok_or(ForkArenaError::InvalidRange)?;
        }
        debug_assert_eq!(
            self.region.annex_arena.payload_position_end(),
            segment.annex_range().end,
            "box stamp must fit its isolated wrapper chunk"
        );
        Ok(segment)
    }

    pub fn box_segment(&self, root: PageListId) -> Option<crate::node_region::PageBoxSegment> {
        if root.len() != 1 {
            return None;
        }
        self.region
            .pub_arena
            .validated_list(&self.pool.chunks, root.coordinate())
            .ok()?
            .first()?
            .box_segment(self.annex_view())
    }

    pub fn box_migration_metadata(&self, root: PageListId) -> Option<PageBoxMigrationMetadata> {
        if root.len() != 1 {
            return None;
        }
        let record = self
            .region
            .pub_arena
            .validated_list(&self.pool.chunks, root.coordinate())
            .ok()?
            .first()?;
        let node_wrapper = self
            .region
            .pub_arena
            .owner_relative_list_block_range(&self.pool.chunks, root.coordinate())
            .ok()?;
        if node_wrapper.end != node_wrapper.start + 1 {
            return None;
        }
        box_migration_metadata_at_record(
            *record,
            self.annex_view(),
            self.region.id(),
            node_wrapper.start,
        )
    }

    /// Claims the child list of the page wrapper just consumed by `\unhbox`
    /// or `\unvbox`. The caller must have removed or independently copied that
    /// semantic wrapper before asking for this one-shot projection authority.
    pub fn consumed_box_children(
        &self,
        wrapper: PageListId,
    ) -> Result<ConsumedBoxChildren, ForkArenaError> {
        if wrapper.len() != 1 {
            return Err(ForkArenaError::InvalidRange);
        }
        let child = match self
            .node_cursor(wrapper)?
            .get(0)
            .ok_or(ForkArenaError::InvalidRange)?
        {
            crate::node_view::NodeView::HList(node) | crate::node_view::NodeView::VList(node) => {
                node.children
            }
            _ => return Err(ForkArenaError::InvalidRange),
        };
        self.admit_span(child)?;
        Ok(ConsumedBoxChildren { list: child })
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
        self.region.can_restore_checkpoint(mark)
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

fn semantic_record_identity(record: &PageMaterialNode, annex: NodeAnnexView<'_>) -> u64 {
    record.semantic_identity(annex)
}

fn is_unbox_margin_kern(record: &PageMaterialNode) -> bool {
    record.kind() == Some(crate::node::NodeKind::MarginKern)
        || record.kern().is_some_and(|(_, kind)| {
            matches!(
                kind,
                crate::node::KernKind::LeftMargin | crate::node::KernKind::RightMargin
            )
        })
}

fn box_migration_metadata_at_record(
    record: PageMaterialNode,
    annex: NodeAnnexView<'_>,
    region: crate::node_region::NodeRegionId,
    node_wrapper: usize,
) -> Option<PageBoxMigrationMetadata> {
    let annex_wrapper = record.box_payload_block_range(annex)?;
    if annex_wrapper.end != annex_wrapper.start + 1 {
        return None;
    }
    if let Some(stamp) = record.copied_box_body_stamp(annex) {
        return stamp.metadata_at_wrapper(region, node_wrapper, annex_wrapper.start);
    }
    if let Some(metadata) =
        record.positive_box_metadata_at_wrapper(annex, region, node_wrapper, annex_wrapper.start)
    {
        return Some(metadata);
    }
    let original = record.box_segment(annex)?;
    let segment = original.rebased_to_wrapper(region, node_wrapper, annex_wrapper.start)?;
    record.box_migration_metadata_rebased(annex, segment)
}

/// Checks the admitted root and every compact record without collecting nodes.
fn page_list_is_decodable(
    view: ArenaListView<'_, PageMaterialNode, PageMaterialLane>,
    annex: NodeAnnexView<'_>,
) -> bool {
    // Decodability is order-independent. Follow the stored predecessor
    // direction without allocating forward-traversal scratch.
    view.iter()
        .rev()
        .all(|record| record.decode_owned(annex).is_some())
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

    pub fn node_cursor(
        &self,
        list: PageListId,
    ) -> Result<crate::node_view::NodeCursor<'a>, ForkArenaError> {
        self.state
            .region
            .pub_arena
            .validated_list(&self.pool.chunks, list.coordinate())
            .map(|view| {
                crate::node_view::NodeCursor::fork_arena(
                    view,
                    NodeAnnexView::new(&self.pool.annex_chunks, &self.state.region.annex_arena),
                )
            })
    }

    pub fn span_node_cursor(
        &self,
        span: PageListSpan,
    ) -> Result<crate::node_view::NodeCursor<'a>, ForkArenaError> {
        self.state
            .region
            .pub_arena
            .validated_list(&self.pool.chunks, span.list.coordinate())
            .map(|view| {
                crate::node_view::NodeCursor::fork_arena(
                    view,
                    NodeAnnexView::new(&self.pool.annex_chunks, &self.state.region.annex_arena),
                )
            })
    }

    pub(crate) fn durable_list(
        &self,
        closure: &'a DurableNodeClosure,
    ) -> Result<crate::node_view::NodeCursor<'a>, ForkArenaError> {
        closure.list(self.pool)
    }

    pub(crate) fn durable_child_list(
        &self,
        closure: &'a DurableNodeClosure,
        child: PageListId,
    ) -> Result<crate::node_view::NodeCursor<'a>, ForkArenaError> {
        closure.child_list(self.pool, child)
    }

    #[must_use]
    pub fn contains(&self, list: PageListId) -> bool {
        self.state
            .region
            .pub_arena
            .validated_list(&self.pool.chunks, list.coordinate())
            .is_ok_and(|view| {
                page_list_is_decodable(
                    view,
                    NodeAnnexView::new(&self.pool.annex_chunks, &self.state.region.annex_arena),
                )
            })
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
        self.state.region.can_restore_checkpoint(mark)
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

mod consumed_box_projection;
#[cfg(feature = "profiling")]
mod fallback_profile;
