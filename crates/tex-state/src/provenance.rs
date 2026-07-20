//! Diagnostic token-provenance storage.
//!
//! Provenance is rollback-coupled with the aggregate store tuple, but remains
//! outside TeX semantic state. Allocation never reports capacity errors:
//! origin-record overflow degrades to [`OriginId::UNKNOWN`], and origin-list
//! overflow degrades to [`OriginListId::EMPTY`].

use crate::identity::{IdentityAllocator, IdentityMark};
use crate::ids::{MacroDefinitionId, OriginListId};
use crate::input::{SourceId, TokenListReplayKind};
use crate::source_map::{SourceMapStats, SourceSpan};
use crate::token::{OriginId, Token, TracedTokenWord};
use crate::world::InputRecordId;
use std::collections::HashSet;
use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_PACKED_ARENA_ORIGIN: AtomicU32 = AtomicU32::new(0);
const ORIGIN_RECORD_ARCHIVE_CHUNK: usize = 1024;

type ArchivedOriginRecord = (u32, OriginRecord);

#[derive(Clone, Debug, Default)]
struct OriginRecordArchive {
    sealed: Arc<Vec<Arc<[ArchivedOriginRecord]>>>,
    tail: Vec<ArchivedOriginRecord>,
}

impl OriginRecordArchive {
    fn append(&mut self, key: u32, record: OriginRecord) {
        self.tail.push((key, record));
        if self.tail.len() == ORIGIN_RECORD_ARCHIVE_CHUNK {
            Arc::make_mut(&mut self.sealed).push(core::mem::take(&mut self.tail).into());
        }
    }

    fn snapshot(&self) -> OriginRecordSnapshot {
        OriginRecordSnapshot {
            sealed: Arc::clone(&self.sealed),
            tail: self.tail.clone().into(),
        }
    }

    fn len(&self) -> usize {
        self.sealed
            .len()
            .saturating_mul(ORIGIN_RECORD_ARCHIVE_CHUNK)
            .saturating_add(self.tail.len())
    }

    fn capacity(&self) -> usize {
        self.sealed
            .len()
            .saturating_mul(ORIGIN_RECORD_ARCHIVE_CHUNK)
            .saturating_add(self.tail.capacity())
    }

    fn get_slot(&self, slot: usize) -> Option<OriginRecord> {
        let chunk = slot / ORIGIN_RECORD_ARCHIVE_CHUNK;
        let offset = slot % ORIGIN_RECORD_ARCHIVE_CHUNK;
        if let Some(chunk) = self.sealed.get(chunk) {
            return chunk.get(offset).map(|(_, record)| *record);
        }
        (chunk == self.sealed.len())
            .then(|| self.tail.get(offset).map(|(_, record)| *record))
            .flatten()
    }

    fn truncate(&mut self, records: usize) {
        let full = records / ORIGIN_RECORD_ARCHIVE_CHUNK;
        let remainder = records % ORIGIN_RECORD_ARCHIVE_CHUNK;
        let sealed = Arc::make_mut(&mut self.sealed);
        if full < sealed.len() {
            self.tail = if remainder == 0 {
                Vec::new()
            } else {
                sealed[full][..remainder].to_vec()
            };
            sealed.truncate(full);
        } else {
            debug_assert_eq!(full, sealed.len());
            self.tail.truncate(remainder);
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OriginRecordSnapshot {
    sealed: Arc<Vec<Arc<[ArchivedOriginRecord]>>>,
    tail: Arc<[ArchivedOriginRecord]>,
}

impl OriginRecordSnapshot {
    fn get(&self, origin: OriginId) -> Option<OriginRecord> {
        let crate::token::OriginEncoding::Arena(key) = origin.decode() else {
            return None;
        };
        if let Some(&(found, record)) = self
            .tail
            .binary_search_by_key(&key, |(key, _)| *key)
            .ok()
            .and_then(|index| self.tail.get(index))
        {
            debug_assert_eq!(found, key);
            return Some(record);
        }
        let index = self
            .sealed
            .partition_point(|chunk| chunk.first().is_some_and(|(first, _)| *first <= key))
            .checked_sub(1)?;
        let chunk = &self.sealed[index];
        let index = chunk.binary_search_by_key(&key, |(key, _)| *key).ok()?;
        Some(chunk[index].1)
    }
}

/// Immutable accepted-generation resolver for diagnostic-only paragraph
/// origins. Cloning this handle is constant time; origin chains are followed
/// only when a diagnostic consumer requests a source location.
#[derive(Clone, Debug)]
pub struct ParagraphOriginResolver {
    provenance: OriginRecordSnapshot,
    fragments: crate::source_fragments::FragmentStore,
}

impl ParagraphOriginResolver {
    pub(crate) fn new(
        provenance: OriginRecordSnapshot,
        fragments: crate::source_fragments::FragmentStore,
    ) -> Self {
        Self {
            provenance,
            fragments,
        }
    }

    /// Resolves one retained raw origin to stable editor backing without
    /// allocating a live provenance record.
    #[must_use]
    pub fn stable_span(&self, origin: OriginId) -> Option<crate::RootSpanId> {
        let mut current = origin;
        for _ in 0..68 {
            if let Some(span) = self.fragments.direct_root_span_id(current) {
                return Some(span);
            }
            current = match current.decode() {
                crate::token::OriginEncoding::Arena(_) => match self.provenance.get(current)? {
                    OriginRecord::MacroInvocation(invocation) => invocation.invocation(),
                    OriginRecord::Inserted(inserted) => inserted.parent(),
                    OriginRecord::Synthesized(synthesized) => synthesized.parent(),
                    OriginRecord::Source(_)
                    | OriginRecord::SourceSpan(_)
                    | OriginRecord::UnknownBootstrap
                    | OriginRecord::Synthetic(_) => return None,
                },
                crate::token::OriginEncoding::Unknown
                | crate::token::OriginEncoding::NoExpandFallback
                | crate::token::OriginEncoding::DirectSource(_) => return None,
            };
        }
        None
    }
}

/// A rollback watermark for the provenance store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProvenanceStoreMark {
    records: u32,
    spans: u32,
    origins: u32,
    list_identities: IdentityMark,
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

    /// Appends a contiguous immutable origin span.
    pub fn extend_from_slice(&mut self, origins: &[OriginId]) {
        self.buf.extend_from_slice(origins);
    }

    /// Appends one origin for a complete literal token span.
    pub fn extend_repeated(&mut self, origin: OriginId, len: usize) {
        self.buf.extend(std::iter::repeat_n(origin, len));
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
    Unexpanded,
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

/// One consecutive process-global origin-key range mapped onto consecutive
/// dense record slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OriginKeyRun {
    first_key: u32,
    first_slot: u32,
    len: u32,
}

impl OriginKeyRun {
    fn end_key(self) -> u32 {
        self.first_key
            .checked_add(self.len)
            .expect("origin key run exceeds u32")
    }

    fn end_slot(self) -> u32 {
        self.first_slot
            .checked_add(self.len)
            .expect("origin key run exceeds record slots")
    }

    fn slot(self, key: u32) -> Option<u32> {
        let offset = key.checked_sub(self.first_key)?;
        (offset < self.len).then(|| self.first_slot + offset)
    }
}

/// Sparse affine index for the globally unique keys present in one timeline.
/// A normal unbranched timeline occupies one run regardless of record count.
#[derive(Clone, Debug, Default)]
struct OriginKeyRuns {
    runs: Vec<OriginKeyRun>,
}

impl OriginKeyRuns {
    fn append(&mut self, key: u32, slot: u32) {
        let Some(last) = self.runs.last_mut() else {
            assert_eq!(slot, 0, "first provenance record slot must be zero");
            self.runs.push(OriginKeyRun {
                first_key: key,
                first_slot: slot,
                len: 1,
            });
            return;
        };
        assert_eq!(
            slot,
            last.end_slot(),
            "provenance records must occupy consecutive slots"
        );
        assert!(
            key >= last.end_key(),
            "process-global provenance keys must increase"
        );
        if key == last.end_key() {
            last.len = last.len.checked_add(1).expect("origin key run overflow");
        } else {
            self.runs.push(OriginKeyRun {
                first_key: key,
                first_slot: slot,
                len: 1,
            });
        }
    }

    fn slot(&self, key: u32) -> Option<u32> {
        if let Some(slot) = self.runs.last().and_then(|run| run.slot(key)) {
            return Some(slot);
        }
        let index = self
            .runs
            .partition_point(|run| run.first_key <= key)
            .checked_sub(1)?;
        self.runs[index].slot(key)
    }

    fn truncate(&mut self, records: u32) {
        let keep = self.runs.partition_point(|run| run.first_slot < records);
        self.runs.truncate(keep);
        if let Some(last) = self.runs.last_mut() {
            last.len = records
                .checked_sub(last.first_slot)
                .expect("retained origin run starts beyond record mark");
            debug_assert!(last.len > 0);
        } else {
            debug_assert_eq!(records, 0);
        }
    }
}

const DEFAULT_ORIGIN_RECORD_LIMIT: usize = 1_048_576;
const DEFAULT_ORIGIN_LIST_SPAN_LIMIT: usize = 262_144;
const DEFAULT_ORIGIN_LIST_ENTRY_LIMIT: usize = 2_097_152;

/// Append-only origin-record and origin-list arenas.
#[derive(Debug)]
pub(crate) struct ProvenanceStore {
    records: OriginRecordArchive,
    spans: Vec<(u32, u32)>,
    origins: Vec<OriginId>,
    list_identities: IdentityAllocator,
    record_keys: OriginKeyRuns,
    record_limit: usize,
    list_span_limit: usize,
    list_entry_limit: usize,
}

impl Clone for ProvenanceStore {
    fn clone(&self) -> Self {
        Self {
            records: self.records.clone(),
            spans: self.spans.clone(),
            origins: self.origins.clone(),
            list_identities: self.list_identities.fork(),
            record_keys: self.record_keys.clone(),
            record_limit: self.record_limit,
            list_span_limit: self.list_span_limit,
            list_entry_limit: self.list_entry_limit,
        }
    }
}

impl ProvenanceStore {
    /// Creates a provenance store with reserved unknown and empty records.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            records: OriginRecordArchive::default(),
            spans: vec![(0, 0)],
            origins: Vec::new(),
            list_identities: IdentityAllocator::new(1),
            record_keys: OriginKeyRuns::default(),
            record_limit: DEFAULT_ORIGIN_RECORD_LIMIT,
            list_span_limit: DEFAULT_ORIGIN_LIST_SPAN_LIMIT,
            list_entry_limit: DEFAULT_ORIGIN_LIST_ENTRY_LIMIT,
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
        if self.records.len() >= self.record_limit {
            return match record {
                OriginRecord::Inserted(inserted)
                    if inserted.kind() == InsertedOriginKind::NoExpand =>
                {
                    OriginId::NOEXPAND_FALLBACK
                }
                _ => OriginId::UNKNOWN,
            };
        }
        let Some(key) = next_packed_arena_origin() else {
            return OriginId::UNKNOWN;
        };
        let slot = u32::try_from(self.records.len())
            .expect("global origin key capacity bounds provenance record slots");
        self.records.append(key, record);
        self.record_keys.append(key, slot);
        OriginId::arena(key).expect("global packed provenance key is representable")
    }

    /// Retains the arena-record graph reachable from diagnostic roots in a
    /// related fork. Process-global arena keys make the imported records
    /// addressable by the artifact's existing `OriginId`s.
    pub(crate) fn retain_origin_graph_from(&mut self, fork: &Self, roots: &[OriginId]) {
        let mut pending = roots.to_vec();
        let mut imported = Vec::new();
        let mut seen = HashSet::with_capacity(roots.len());
        while let Some(origin) = pending.pop() {
            let crate::token::OriginEncoding::Arena(key) = origin.decode() else {
                continue;
            };
            if self.record_keys.slot(key).is_some() || !seen.insert(key) {
                continue;
            }
            let Some(slot) = fork.record_keys.slot(key) else {
                continue;
            };
            let record = fork
                .records
                .get_slot(slot as usize)
                .expect("fork provenance slot is live");
            match record {
                OriginRecord::MacroInvocation(invocation) => {
                    pending.push(invocation.invocation());
                    pending.push(invocation.definition_origin());
                    pending.push(invocation.parent_invocation());
                }
                OriginRecord::Inserted(inserted) => pending.push(inserted.parent()),
                OriginRecord::Synthesized(synthesized) => pending.push(synthesized.parent()),
                OriginRecord::UnknownBootstrap
                | OriginRecord::Source(_)
                | OriginRecord::SourceSpan(_)
                | OriginRecord::Synthetic(_) => {}
            }
            imported.push((key, record));
            if self.records.len() >= self.record_limit {
                break;
            }
        }
        imported.sort_unstable_by_key(|(key, _)| *key);
        for (key, record) in imported {
            let slot = u32::try_from(self.records.len())
                .expect("global origin key capacity bounds provenance record slots");
            self.records.append(key, record);
            self.record_keys.append(key, slot);
        }
    }

    pub(crate) fn record_snapshot(&self) -> OriginRecordSnapshot {
        self.records.snapshot()
    }

    fn list_budget_allows(&self, len: usize) -> bool {
        self.spans.len() < self.list_span_limit
            && self
                .origins
                .len()
                .checked_add(len)
                .is_some_and(|end| end <= self.list_entry_limit)
    }

    /// Allocates an origin-list span, saturating capacity overflow to empty.
    pub(crate) fn allocate_list(&mut self, origins: &[OriginId]) -> OriginListId {
        if origins.is_empty() || !self.list_budget_allows(origins.len()) {
            return OriginListId::EMPTY;
        }
        let (Some(start), Some(len), Some(_raw)) = (
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
        OriginListId::from_identity(
            self.list_identities
                .allocate()
                .expect("origin-list span capacity checked"),
        )
    }

    /// Allocates the origin projection of an already-validated traced slice.
    pub(crate) fn allocate_traced_list(&mut self, traced: &[TracedTokenWord]) -> OriginListId {
        if traced.is_empty() || !self.list_budget_allows(traced.len()) {
            return OriginListId::EMPTY;
        }
        let (Some(start), Some(len), Some(_raw)) = (
            u32_len(self.origins.len()),
            u32_len(traced.len()),
            u32_index(self.spans.len()),
        ) else {
            return OriginListId::EMPTY;
        };
        let Some(_end) = start.checked_add(len) else {
            return OriginListId::EMPTY;
        };
        self.origins.reserve(traced.len());
        self.spans.reserve(1);
        self.origins.extend(traced.iter().map(|word| word.origin()));
        self.spans.push((start, len));
        OriginListId::from_identity(
            self.list_identities
                .allocate()
                .expect("origin-list span capacity checked"),
        )
    }

    /// Allocates an origin-list span by repeating one live origin.
    pub(crate) fn allocate_repeated_list(&mut self, origin: OriginId, len: usize) -> OriginListId {
        if len == 0 || !self.list_budget_allows(len) {
            return OriginListId::EMPTY;
        }
        let (Some(start), Some(len), Some(_raw)) = (
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
        OriginListId::from_identity(
            self.list_identities
                .allocate()
                .expect("origin-list span capacity checked"),
        )
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
        let index = self.record_keys.slot(index).expect("origin id is not live") as usize;
        self.records
            .get_slot(index)
            .expect("live provenance slot exists")
    }

    /// Reads a live origin-list span.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn list(&self, id: OriginListId) -> &[OriginId] {
        assert!(self.contains_list(id), "origin list id is not live");
        self.list_span(id)
    }

    /// Resolves a stored origin-list id and reads its live span.
    #[must_use]
    pub(crate) fn resolve_list(&self, id: OriginListId) -> Option<&[OriginId]> {
        self.resolve_stored_list(id).map(|id| self.list_span(id))
    }

    fn list_span(&self, id: OriginListId) -> &[OriginId] {
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
            crate::token::OriginEncoding::NoExpandFallback => true,
            crate::token::OriginEncoding::Arena(index) => self.record_keys.slot(index).is_some(),
            crate::token::OriginEncoding::DirectSource(_) => false,
        }
    }

    /// Returns whether `id` names a currently-live origin-list span.
    #[must_use]
    pub(crate) fn contains_list(&self, id: OriginListId) -> bool {
        self.list_identities.contains(id.identity())
    }

    pub(crate) fn resolve_stored_list(&self, id: OriginListId) -> Option<OriginListId> {
        if self.contains_list(id) {
            return Some(id);
        }
        if !id.is_stored() {
            return None;
        }
        self.list_identities
            .identity_at(id.raw())
            .map(OriginListId::from_identity)
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
            list_identities: self.list_identities.watermark(),
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

        self.list_identities
            .rollback(mark.list_identities)
            .expect("provenance mark is not an ancestor");
        self.record_keys.truncate(mark.records);
        self.records.truncate(records);
        self.spans.truncate(spans);
        self.origins.truncate(origins);
    }
}

fn u32_len(value: usize) -> Option<u32> {
    u32::try_from(value).ok()
}

fn next_packed_arena_origin() -> Option<u32> {
    NEXT_PACKED_ARENA_ORIGIN
        .fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            packed_origin_successor,
        )
        .ok()
}

fn packed_origin_successor(next: u32) -> Option<u32> {
    (next <= 0x7fff_ffff).then_some(next + 1)
}

fn u32_index(value: usize) -> Option<u32> {
    let value = u32_len(value)?;
    (value < u32::MAX).then_some(value)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
fn arena_index(value: usize) -> Option<u32> {
    let value = u32::try_from(value).ok()?;
    (value <= 0x7fff_ffff).then_some(value)
}

#[cfg(test)]
mod tests;
