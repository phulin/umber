//! Diagnostic token-provenance storage.
//!
//! Provenance is rollback-coupled with the aggregate store tuple, but remains
//! outside TeX semantic state. Allocation never reports capacity errors:
//! origin-record overflow degrades to [`OriginId::UNKNOWN`], and origin-list
//! overflow degrades to [`OriginListId::EMPTY`].

use crate::ids::{MacroDefinitionId, OriginListId};
use crate::input::{SourceId, TokenListReplayKind};
use crate::source_map::{SourceMapStats, SourceSpan};
use crate::token::{OriginId, Token};
use crate::world::InputRecordId;
use std::mem;

/// A rollback watermark for the provenance store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProvenanceStoreMark {
    records: u32,
    spans: u32,
    origins: u32,
}

/// Live provenance arena size counters.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProvenanceStats {
    origin_records: usize,
    origin_list_spans: usize,
    origin_list_entries: usize,
    origin_record_capacity: usize,
    origin_list_span_capacity: usize,
    origin_list_entry_capacity: usize,
    source_regions: usize,
    generated_source_backings: usize,
    source_map_bytes: usize,
    source_map_retained_bytes: usize,
}

impl PartialEq for ProvenanceStats {
    fn eq(&self, other: &Self) -> bool {
        self.origin_records == other.origin_records
            && self.origin_list_spans == other.origin_list_spans
            && self.origin_list_entries == other.origin_list_entries
            && self.source_regions == other.source_regions
            && self.generated_source_backings == other.generated_source_backings
            && self.source_map_bytes == other.source_map_bytes
    }
}

impl Eq for ProvenanceStats {}

impl ProvenanceStats {
    #[must_use]
    pub const fn new(
        origin_records: usize,
        origin_list_spans: usize,
        origin_list_entries: usize,
    ) -> Self {
        Self {
            origin_records,
            origin_list_spans,
            origin_list_entries,
            origin_record_capacity: 0,
            origin_list_span_capacity: 0,
            origin_list_entry_capacity: 0,
            source_regions: 0,
            generated_source_backings: 0,
            source_map_bytes: 0,
            source_map_retained_bytes: 0,
        }
    }

    const fn with_capacities(
        origin_records: usize,
        origin_list_spans: usize,
        origin_list_entries: usize,
        origin_record_capacity: usize,
        origin_list_span_capacity: usize,
        origin_list_entry_capacity: usize,
    ) -> Self {
        Self {
            origin_records,
            origin_list_spans,
            origin_list_entries,
            origin_record_capacity,
            origin_list_span_capacity,
            origin_list_entry_capacity,
            source_regions: 0,
            generated_source_backings: 0,
            source_map_bytes: 0,
            source_map_retained_bytes: 0,
        }
    }

    pub(crate) const fn with_source_map(mut self, stats: SourceMapStats) -> Self {
        self.source_regions = stats.regions;
        self.generated_source_backings = stats.generated_backings;
        self.source_map_bytes = stats.live_bytes;
        self.source_map_retained_bytes = stats.retained_bytes;
        self
    }

    #[must_use]
    pub const fn origin_records(self) -> usize {
        self.origin_records
    }

    #[must_use]
    pub const fn origin_list_spans(self) -> usize {
        self.origin_list_spans
    }

    #[must_use]
    pub const fn origin_list_entries(self) -> usize {
        self.origin_list_entries
    }

    #[must_use]
    pub const fn source_regions(self) -> usize {
        self.source_regions
    }

    #[must_use]
    pub const fn generated_source_backings(self) -> usize {
        self.generated_source_backings
    }

    #[must_use]
    pub const fn source_map_bytes(self) -> usize {
        self.source_map_bytes
    }

    #[must_use]
    pub const fn estimated_bytes(self) -> usize {
        self.origin_records * mem::size_of::<OriginRecord>()
            + self.origin_list_spans * mem::size_of::<(u32, u32)>()
            + self.origin_list_entries * mem::size_of::<OriginId>()
            + self.source_map_bytes
    }

    #[must_use]
    pub const fn retained_bytes(self) -> usize {
        self.origin_record_capacity * mem::size_of::<OriginRecord>()
            + self.origin_list_span_capacity * mem::size_of::<(u32, u32)>()
            + self.origin_list_entry_capacity * mem::size_of::<OriginId>()
            + self.source_map_retained_bytes
    }

    #[must_use]
    pub const fn origin_record_capacity(self) -> usize {
        self.origin_record_capacity
    }

    #[must_use]
    pub const fn origin_list_span_capacity(self) -> usize {
        self.origin_list_span_capacity
    }

    #[must_use]
    pub const fn origin_list_entry_capacity(self) -> usize {
        self.origin_list_entry_capacity
    }

    #[must_use]
    pub const fn source_map_retained_bytes(self) -> usize {
        self.source_map_retained_bytes
    }

    #[must_use]
    pub const fn saturating_sub(self, baseline: Self) -> Self {
        Self {
            origin_records: self.origin_records.saturating_sub(baseline.origin_records),
            origin_list_spans: self
                .origin_list_spans
                .saturating_sub(baseline.origin_list_spans),
            origin_list_entries: self
                .origin_list_entries
                .saturating_sub(baseline.origin_list_entries),
            origin_record_capacity: self
                .origin_record_capacity
                .saturating_sub(baseline.origin_record_capacity),
            origin_list_span_capacity: self
                .origin_list_span_capacity
                .saturating_sub(baseline.origin_list_span_capacity),
            origin_list_entry_capacity: self
                .origin_list_entry_capacity
                .saturating_sub(baseline.origin_list_entry_capacity),
            source_regions: self.source_regions.saturating_sub(baseline.source_regions),
            generated_source_backings: self
                .generated_source_backings
                .saturating_sub(baseline.generated_source_backings),
            source_map_bytes: self
                .source_map_bytes
                .saturating_sub(baseline.source_map_bytes),
            source_map_retained_bytes: self
                .source_map_retained_bytes
                .saturating_sub(baseline.source_map_retained_bytes),
        }
    }
}

/// An owned scratch buffer for building an origin list before freezing it.
#[derive(Clone, Debug)]
pub struct OriginListBuilder {
    buf: Vec<OriginId>,
}

impl OriginListBuilder {
    /// Creates an empty reusable origin-list builder.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Appends one origin to the unfinished list.
    pub fn push(&mut self, origin: OriginId) {
        self.buf.push(origin);
    }

    /// Reserves capacity when the caller already knows the remaining size.
    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional);
    }

    /// Returns the number of origins currently buffered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns whether the builder currently holds no origins.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Clears the unfinished list without allocating a span.
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    #[cfg(feature = "node-stats")]
    #[must_use]
    pub(crate) fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    #[must_use]
    pub(crate) fn as_slice(&self) -> &[OriginId] {
        &self.buf
    }

    /// Allocates the current origin list and clears the builder for reuse.
    pub(crate) fn finish(&mut self, store: &mut ProvenanceStore) -> OriginListId {
        let id = store.allocate_list(&self.buf);
        self.buf.clear();
        id
    }
}

/// Source coordinate for a token read from an input source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceOrigin {
    byte_offset: u64,
    source: SourceId,
    input_record: Option<InputRecordId>,
    line: u32,
    column: u32,
}

impl SourceOrigin {
    /// Creates a source-origin coordinate.
    #[must_use]
    pub const fn new(source: SourceId, byte_offset: u64, line: u32, column: u32) -> Self {
        Self {
            byte_offset,
            source,
            input_record: None,
            line,
            column,
        }
    }

    /// Attaches the `World` record that owns the source's path and bytes.
    #[must_use]
    pub const fn with_input_record(mut self, input_record: InputRecordId) -> Self {
        self.input_record = Some(input_record);
        self
    }

    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    #[must_use]
    pub const fn input_record(self) -> Option<InputRecordId> {
        self.input_record
    }

    #[must_use]
    pub const fn byte_offset(self) -> u64 {
        self.byte_offset
    }

    #[must_use]
    pub const fn line(self) -> u32 {
        self.line
    }

    #[must_use]
    pub const fn column(self) -> u32 {
        self.column
    }
}

/// Provenance for one live macro invocation frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroInvocationOrigin {
    definition: MacroDefinitionId,
    invocation: OriginId,
    definition_origin: OriginId,
    parent_invocation: OriginId,
}

impl MacroInvocationOrigin {
    /// Creates a macro-invocation origin record.
    #[must_use]
    pub const fn new(
        definition: MacroDefinitionId,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> Self {
        Self {
            definition,
            invocation,
            definition_origin,
            parent_invocation,
        }
    }

    #[must_use]
    pub const fn definition(self) -> MacroDefinitionId {
        self.definition
    }

    #[must_use]
    pub const fn invocation(self) -> OriginId {
        self.invocation
    }

    #[must_use]
    pub const fn definition_origin(self) -> OriginId {
        self.definition_origin
    }

    #[must_use]
    pub const fn parent_invocation(self) -> OriginId {
        self.parent_invocation
    }
}

/// Provenance for a token inserted into the input stream by TeX machinery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InsertedOrigin {
    kind: InsertedOriginKind,
    token: Token,
    parent: OriginId,
}

impl InsertedOrigin {
    /// Creates an inserted-token origin.
    #[must_use]
    pub const fn new(kind: InsertedOriginKind, token: Token, parent: OriginId) -> Self {
        Self {
            kind,
            token,
            parent,
        }
    }

    #[must_use]
    pub const fn kind(self) -> InsertedOriginKind {
        self.kind
    }

    #[must_use]
    pub const fn token(self) -> Token {
        self.token
    }

    #[must_use]
    pub const fn parent(self) -> OriginId {
        self.parent
    }
}

/// The source of an inserted token.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InsertedOriginKind {
    EndLine,
    Paragraph,
    AfterGroup,
    AfterAssignment,
    NoExpand,
    ExpandAfter,
    Unread,
    TokenListReplay(TokenListReplayKind),
    ErrorRecovery,
}

/// Provenance for a token synthesized from semantic state rather than copied
/// from a source or token list.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SynthesizedOrigin {
    kind: SynthesizedOriginKind,
    parent: OriginId,
}

impl SynthesizedOrigin {
    /// Creates a synthesized-token origin.
    #[must_use]
    pub const fn new(kind: SynthesizedOriginKind, parent: OriginId) -> Self {
        Self { kind, parent }
    }

    #[must_use]
    pub const fn kind(self) -> SynthesizedOriginKind {
        self.kind
    }

    #[must_use]
    pub const fn parent(self) -> OriginId {
        self.parent
    }
}

/// The operation that synthesized a token.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SynthesizedOriginKind {
    Expansion,
    Scanner,
    ValueRendering,
    NoExpand,
    ErrorRecovery,
}

/// Provenance for bootstrap or engine-owned tokens with no source coordinate.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SyntheticOrigin {
    kind: SyntheticOriginKind,
}

impl SyntheticOrigin {
    /// Creates a synthetic/bootstrap origin.
    #[must_use]
    pub const fn new(kind: SyntheticOriginKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> SyntheticOriginKind {
        self.kind
    }
}

/// The family of a synthetic/bootstrap origin.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SyntheticOriginKind {
    Bootstrap,
    Primitive,
    Format,
    Engine,
    Test,
}

/// The semantic role of a secondary diagnostic location.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RelatedLocationRole {
    Invocation,
    Definition,
    RecoveryFrontier,
    SecondarySpelling,
}

impl RelatedLocationRole {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Invocation => "invoked here",
            Self::Definition => "defined here",
            Self::RecoveryFrontier => "recovery begins here",
            Self::SecondarySpelling => "also consumed here",
        }
    }
}

/// One labeled secondary location captured when a diagnostic is created.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RelatedLocation {
    role: RelatedLocationRole,
    origin: OriginId,
}

impl RelatedLocation {
    #[must_use]
    pub const fn new(role: RelatedLocationRole, origin: OriginId) -> Self {
        Self { role, origin }
    }

    #[must_use]
    pub const fn role(self) -> RelatedLocationRole {
        self.role
    }

    #[must_use]
    pub const fn origin(self) -> OriginId {
        self.origin
    }
}

/// Origins retained by an error independently of mutable input-stack state.
///
/// The expansion head names a persistent parent-linked macro invocation
/// chain. Presentation decides how much of that chain to render.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticSite {
    primary: Option<OriginId>,
    related: Box<[RelatedLocation]>,
    expansion_head: Option<OriginId>,
}

impl DiagnosticSite {
    pub const MAX_RELATED: usize = 8;

    #[must_use]
    pub fn new(
        primary: Option<OriginId>,
        related: impl IntoIterator<Item = RelatedLocation>,
        expansion_head: Option<OriginId>,
    ) -> Self {
        Self {
            primary,
            related: related.into_iter().take(Self::MAX_RELATED).collect(),
            expansion_head: expansion_head.filter(|origin| *origin != OriginId::UNKNOWN),
        }
    }

    #[must_use]
    pub fn primary(primary: OriginId) -> Self {
        Self::new(Some(primary), [], None)
    }

    #[must_use]
    pub fn unknown() -> Self {
        Self::new(None, [], None)
    }

    #[must_use]
    pub const fn primary_origin(&self) -> Option<OriginId> {
        self.primary
    }

    #[must_use]
    pub fn related(&self) -> &[RelatedLocation] {
        &self.related
    }

    #[must_use]
    pub const fn expansion_head(&self) -> Option<OriginId> {
        self.expansion_head
    }
}

/// One lazily-resolved token-origin record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OriginRecord {
    /// Reserved record for unknown, bootstrap, or lost provenance.
    UnknownBootstrap,
    Source(SourceOrigin),
    /// A validated source-map range, used by tagged direct/fallback origins.
    SourceSpan(SourceSpan),
    MacroInvocation(MacroInvocationOrigin),
    Inserted(InsertedOrigin),
    Synthesized(SynthesizedOrigin),
    Synthetic(SyntheticOrigin),
}

/// Append-only origin-record and origin-list arenas.
#[derive(Clone, Debug)]
pub(crate) struct ProvenanceStore {
    records: Vec<OriginRecord>,
    spans: Vec<(u32, u32)>,
    origins: Vec<OriginId>,
}

impl ProvenanceStore {
    /// Creates a provenance store with reserved unknown and empty records.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            records: Vec::new(),
            spans: vec![(0, 0)],
            origins: Vec::new(),
        }
    }

    /// Returns the reserved unknown/bootstrap origin id.
    #[must_use]
    pub(crate) const fn unknown_id() -> OriginId {
        OriginId::UNKNOWN
    }

    /// Creates a fresh owned scratch origin-list builder.
    #[must_use]
    pub(crate) fn builder() -> OriginListBuilder {
        OriginListBuilder::new()
    }

    /// Allocates a new origin record, saturating capacity overflow to unknown.
    pub(crate) fn allocate(&mut self, record: OriginRecord) -> OriginId {
        let Some(index) = arena_index(self.records.len()) else {
            return OriginId::UNKNOWN;
        };
        self.records.push(record);
        OriginId::arena(index).expect("checked provenance arena index")
    }

    /// Allocates an origin-list span, saturating capacity overflow to empty.
    pub(crate) fn allocate_list(&mut self, origins: &[OriginId]) -> OriginListId {
        if origins.is_empty() {
            return OriginListId::EMPTY;
        }
        let (Some(start), Some(len), Some(raw)) = (
            u32_len(self.origins.len()),
            u32_len(origins.len()),
            u32_index(self.spans.len()),
        ) else {
            return OriginListId::EMPTY;
        };
        let Some(_end) = start.checked_add(len) else {
            return OriginListId::EMPTY;
        };
        self.origins.extend_from_slice(origins);
        self.spans.push((start, len));
        OriginListId::new(raw)
    }

    /// Allocates an origin-list span by repeating one live origin.
    pub(crate) fn allocate_repeated_list(&mut self, origin: OriginId, len: usize) -> OriginListId {
        if len == 0 {
            return OriginListId::EMPTY;
        }
        let (Some(start), Some(len), Some(raw)) = (
            u32_len(self.origins.len()),
            u32_len(len),
            u32_index(self.spans.len()),
        ) else {
            return OriginListId::EMPTY;
        };
        let Some(_end) = start.checked_add(len) else {
            return OriginListId::EMPTY;
        };
        self.origins
            .resize(self.origins.len() + len as usize, origin);
        self.spans.push((start, len));
        OriginListId::new(raw)
    }

    /// Reads a live origin record.
    #[must_use]
    pub(crate) fn get(&self, id: OriginId) -> OriginRecord {
        if id == OriginId::UNKNOWN {
            return OriginRecord::UnknownBootstrap;
        }
        let crate::token::OriginEncoding::Arena(index) = id.decode() else {
            panic!("direct source origin has no provenance arena record");
        };
        let index = index as usize;
        assert!(index < self.records.len(), "origin id is not live");
        self.records[index]
    }

    /// Reads a live origin-list span.
    #[must_use]
    pub(crate) fn list(&self, id: OriginListId) -> &[OriginId] {
        let index = id.raw() as usize;
        assert!(index < self.spans.len(), "origin list id is not live");
        let (start, len) = self.spans[index];
        let start = start as usize;
        let end = start + len as usize;
        assert!(end <= self.origins.len(), "origin-list span exceeds arena");
        &self.origins[start..end]
    }

    /// Returns whether `id` names a currently-live origin record.
    #[must_use]
    pub(crate) fn contains_origin(&self, id: OriginId) -> bool {
        match id.decode() {
            crate::token::OriginEncoding::Unknown => true,
            crate::token::OriginEncoding::Arena(index) => (index as usize) < self.records.len(),
            crate::token::OriginEncoding::DirectSource(_) => false,
        }
    }

    /// Returns whether `id` names a currently-live origin-list span.
    #[must_use]
    pub(crate) fn contains_list(&self, id: OriginListId) -> bool {
        (id.raw() as usize) < self.spans.len()
    }

    /// Returns live arena length counters.
    #[must_use]
    pub(crate) fn stats(&self) -> ProvenanceStats {
        ProvenanceStats::with_capacities(
            self.records.len(),
            self.spans.len(),
            self.origins.len(),
            self.records.capacity(),
            self.spans.capacity(),
            self.origins.capacity(),
        )
    }

    /// Takes a rollback watermark for aggregate snapshots.
    #[must_use]
    pub(crate) fn watermark(&self) -> ProvenanceStoreMark {
        ProvenanceStoreMark {
            records: u32_len(self.records.len())
                .expect("provenance record arena exceeded representable mark"),
            spans: u32_len(self.spans.len())
                .expect("provenance span arena exceeded representable mark"),
            origins: u32_len(self.origins.len())
                .expect("provenance origin arena exceeded representable mark"),
        }
    }

    /// Truncates to a previously-taken aggregate snapshot watermark.
    pub(crate) fn truncate_to(&mut self, mark: ProvenanceStoreMark) {
        let records = mark.records as usize;
        let spans = mark.spans as usize;
        let origins = mark.origins as usize;
        assert!(spans >= 1, "provenance mark removes empty origin list");
        assert!(
            records <= self.records.len(),
            "provenance mark has too many records"
        );
        assert!(
            spans <= self.spans.len(),
            "provenance mark has too many spans"
        );
        assert!(
            origins <= self.origins.len(),
            "provenance mark has too many origins"
        );
        assert!(
            self.spans[..spans]
                .last()
                .is_some_and(|&(start, len)| start + len == mark.origins),
            "provenance mark does not point to an origin-list boundary"
        );

        self.records.truncate(records);
        self.spans.truncate(spans);
        self.origins.truncate(origins);
    }
}

fn u32_len(value: usize) -> Option<u32> {
    u32::try_from(value).ok()
}

fn u32_index(value: usize) -> Option<u32> {
    let value = u32_len(value)?;
    (value < u32::MAX).then_some(value)
}

fn arena_index(value: usize) -> Option<u32> {
    let value = u32::try_from(value).ok()?;
    (value <= 0x7fff_ffff).then_some(value)
}

#[cfg(test)]
mod tests;
