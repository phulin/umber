//! Diagnostic token-provenance storage.
//!
//! Provenance remains outside TeX semantic state. Origin records retain their
//! rollback-coupled compatibility archive, while exact origin lists live in
//! the aggregate runtime value registry. Allocation never reports capacity
//! errors: origin-record overflow degrades to [`OriginId::UNKNOWN`], and
//! origin-list overflow degrades to [`OriginListId::EMPTY`].

use crate::hot_core::arena::store::RuntimeOriginListView;
use crate::ids::{MacroDefinitionId, OriginListId};
use crate::input::{SourceId, TokenListReplayKind};
use crate::source_map::{SourceMap, SourceMapStats, SourceRegistrationRef, SourceSpan};
use crate::token::{OriginEncoding, OriginId, Token};
use crate::world::InputRecordId;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_PACKED_ARENA_ORIGIN: AtomicU32 = AtomicU32::new(0);
const ORIGIN_RECORD_ARCHIVE_CHUNK: usize = 1024;
const ORIGIN_KEY_LEASE_LEN: u32 = 256;
const PACKED_ARENA_ORIGIN_END: u32 = 0x8000_0000;

/// Optional provenance surfaces selected once for an engine job.
///
/// Source registration and compact token positions are unconditional engine
/// state. This policy controls only consumers which retain additional roots at
/// an output boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvenanceDemand {
    diagnostics: bool,
    rendered_source: bool,
}

impl ProvenanceDemand {
    /// Ordinary batch execution: diagnostics remain exact, but shipped pages
    /// do not retain rendered-source sidecars.
    pub const DIAGNOSTICS: Self = Self {
        diagnostics: true,
        rendered_source: false,
    };

    /// Editor execution with both diagnostic and rendered-source consumers.
    pub const DIAGNOSTICS_AND_RENDERED_SOURCE: Self = Self {
        diagnostics: true,
        rendered_source: true,
    };

    /// Whether an error consumer may capture diagnostic roots.
    #[must_use]
    pub const fn diagnostics(self) -> bool {
        self.diagnostics
    }

    /// Whether shipout retains node-to-source roots and recipes.
    #[must_use]
    pub const fn rendered_source(self) -> bool {
        self.rendered_source
    }

    /// Returns the same policy with rendered-source consumption enabled.
    #[must_use]
    pub const fn with_rendered_source(self) -> Self {
        Self {
            rendered_source: true,
            ..self
        }
    }
}

impl Default for ProvenanceDemand {
    fn default() -> Self {
        Self::DIAGNOSTICS
    }
}

/// Independent production admission limits for retained provenance.
///
/// Exhaustion degrades only optional provenance to unknown. It never aborts
/// TeX execution or changes artifact bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvenanceBudgets {
    pub live_atoms: usize,
    pub live_origin_lists: usize,
    pub origin_list_entries: usize,
    pub weak_atom_slots: usize,
    pub weak_atom_candidate_keys: usize,
    pub detached_artifact_recipe_bytes: usize,
}

impl Default for ProvenanceBudgets {
    fn default() -> Self {
        Self {
            live_atoms: DEFAULT_ORIGIN_RECORD_LIMIT,
            live_origin_lists: DEFAULT_ORIGIN_LIST_SPAN_LIMIT,
            origin_list_entries: DEFAULT_ORIGIN_LIST_ENTRY_LIMIT,
            weak_atom_slots: DEFAULT_ORIGIN_RECORD_LIMIT,
            weak_atom_candidate_keys: RECORD_CANDIDATE_KEY_BUDGET,
            detached_artifact_recipe_bytes: 16 * 1024 * 1024,
        }
    }
}

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

    fn retained_metadata_bytes(&self) -> usize {
        self.sealed
            .capacity()
            .saturating_mul(mem::size_of::<Arc<[ArchivedOriginRecord]>>())
    }

    fn macro_invocation_records(&self) -> usize {
        self.sealed
            .iter()
            .flat_map(|chunk| chunk.iter())
            .chain(self.tail.iter())
            .filter(|(_, record)| matches!(record, OriginRecord::MacroInvocation(_)))
            .count()
    }

    #[cfg(any(test, feature = "testing"))]
    fn macro_invocation_origins(&self) -> Vec<OriginId> {
        self.sealed
            .iter()
            .flat_map(|chunk| chunk.iter())
            .chain(self.tail.iter())
            .filter(|(_, record)| matches!(record, OriginRecord::MacroInvocation(_)))
            .map(|(key, _)| {
                OriginId::arena(*key).expect("stored provenance key remains representable")
            })
            .collect()
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

    fn get_entry(&self, slot: usize) -> Option<ArchivedOriginRecord> {
        let chunk = slot / ORIGIN_RECORD_ARCHIVE_CHUNK;
        let offset = slot % ORIGIN_RECORD_ARCHIVE_CHUNK;
        if let Some(chunk) = self.sealed.get(chunk) {
            return chunk.get(offset).copied();
        }
        (chunk == self.sealed.len())
            .then(|| self.tail.get(offset).copied())
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

/// A rollback watermark for the provenance store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProvenanceStoreMark {
    records: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct OriginRecordStorageStats {
    capacity: usize,
    archive_metadata_retained_bytes: usize,
    key_runs: usize,
    key_run_capacity: usize,
}

/// Live compatibility-record and structural provenance size counters.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProvenanceStats {
    origin_records: usize,
    origin_list_spans: usize,
    origin_list_entries: usize,
    origin_record_capacity: usize,
    origin_record_archive_metadata_retained_bytes: usize,
    origin_key_runs: usize,
    origin_key_run_capacity: usize,
    origin_list_span_capacity: usize,
    origin_list_entry_capacity: usize,
    origin_list_retained_bytes: usize,
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
            origin_record_archive_metadata_retained_bytes: 0,
            origin_key_runs: 0,
            origin_key_run_capacity: 0,
            origin_list_span_capacity: 0,
            origin_list_entry_capacity: 0,
            origin_list_retained_bytes: 0,
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
        origin_records_storage: OriginRecordStorageStats,
        origin_list_span_capacity: usize,
        origin_list_entry_capacity: usize,
        origin_list_retained_bytes: usize,
    ) -> Self {
        Self {
            origin_records,
            origin_list_spans,
            origin_list_entries,
            origin_record_capacity: origin_records_storage.capacity,
            origin_record_archive_metadata_retained_bytes: origin_records_storage
                .archive_metadata_retained_bytes,
            origin_key_runs: origin_records_storage.key_runs,
            origin_key_run_capacity: origin_records_storage.key_run_capacity,
            origin_list_span_capacity,
            origin_list_entry_capacity,
            origin_list_retained_bytes,
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

    pub(crate) const fn with_origin_lists(
        mut self,
        lists: usize,
        entries: usize,
        retained_list_slots: usize,
        retained_entry_slots: usize,
        retained_bytes: usize,
    ) -> Self {
        self.origin_list_spans = lists;
        self.origin_list_entries = entries;
        self.origin_list_span_capacity = retained_list_slots;
        self.origin_list_entry_capacity = retained_entry_slots;
        self.origin_list_retained_bytes = retained_bytes;
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
        self.origin_records * mem::size_of::<ArchivedOriginRecord>()
            + self.origin_list_spans * mem::size_of::<OriginListRef>()
            + self.origin_list_entries * mem::size_of::<OriginId>()
            + self.source_map_bytes
    }

    #[must_use]
    pub const fn retained_bytes(self) -> usize {
        self.origin_record_capacity * mem::size_of::<ArchivedOriginRecord>()
            + self.origin_record_archive_metadata_retained_bytes
            + self.origin_key_run_capacity * mem::size_of::<OriginKeyRun>()
            + self.origin_list_retained_bytes
            + self.source_map_retained_bytes
    }

    /// Retained bytes charged to the fixed-width origin-record arena and its
    /// packed-key index, excluding origin lists and source-map storage.
    #[must_use]
    pub const fn origin_record_retained_bytes(self) -> usize {
        self.origin_record_capacity * mem::size_of::<ArchivedOriginRecord>()
            + self.origin_record_archive_metadata_retained_bytes
            + self.origin_key_run_capacity * mem::size_of::<OriginKeyRun>()
    }

    /// Fixed width of one archived `(key, record)` slot. This is the layout
    /// quantity governed by the 64-byte per-record production admission charge.
    #[must_use]
    pub const fn origin_record_slot_bytes(self) -> usize {
        mem::size_of::<ArchivedOriginRecord>()
    }

    /// Number of record slots sealed into one immutable archive chunk.
    #[must_use]
    pub const fn origin_record_archive_chunk_slots(self) -> usize {
        ORIGIN_RECORD_ARCHIVE_CHUNK
    }

    /// Number of consecutive process-global keys reserved by one arena lease.
    #[must_use]
    pub const fn origin_key_lease_slots(self) -> usize {
        ORIGIN_KEY_LEASE_LEN as usize
    }

    #[must_use]
    pub const fn origin_record_archive_metadata_retained_bytes(self) -> usize {
        self.origin_record_archive_metadata_retained_bytes
    }

    #[must_use]
    pub const fn origin_key_runs(self) -> usize {
        self.origin_key_runs
    }

    #[must_use]
    pub const fn origin_key_run_capacity(self) -> usize {
        self.origin_key_run_capacity
    }

    /// Maximum retained bytes implied by the archive's 1,024-slot chunks and
    /// the geometric growth policy of its three `Vec` allocations.
    ///
    /// This is a container-layout bound, not an empirical workload ceiling.
    /// It deliberately includes unused tail and index capacity.
    #[must_use]
    pub fn origin_record_layout_budget_bytes(self) -> usize {
        let sealed_chunks = self.origin_records / ORIGIN_RECORD_ARCHIVE_CHUNK;
        let tail_len = self.origin_records % ORIGIN_RECORD_ARCHIVE_CHUNK;
        let tail_capacity = if tail_len == 0 {
            0
        } else {
            tail_len
                .checked_next_power_of_two()
                .unwrap_or(usize::MAX)
                .max(4)
        };
        let record_capacity = sealed_chunks
            .saturating_mul(ORIGIN_RECORD_ARCHIVE_CHUNK)
            .saturating_add(tail_capacity);
        let sealed_capacity = if sealed_chunks == 0 {
            0
        } else {
            sealed_chunks
                .checked_next_power_of_two()
                .unwrap_or(usize::MAX)
                .max(4)
        };
        let run_capacity = if self.origin_key_runs == 0 {
            0
        } else {
            self.origin_key_runs
                .checked_next_power_of_two()
                .unwrap_or(usize::MAX)
                .max(4)
        };
        record_capacity
            .saturating_mul(mem::size_of::<ArchivedOriginRecord>())
            .saturating_add(
                sealed_capacity.saturating_mul(mem::size_of::<Arc<[ArchivedOriginRecord]>>()),
            )
            .saturating_add(run_capacity.saturating_mul(mem::size_of::<OriginKeyRun>()))
    }

    /// Whether two observations have identical logical and retained storage.
    /// Unlike `PartialEq`, this includes allocation capacity and source-map
    /// retention and is intended for fresh-job/cache-isolation controls.
    #[must_use]
    pub const fn retained_layout_eq(self, other: Self) -> bool {
        self.origin_records == other.origin_records
            && self.origin_list_spans == other.origin_list_spans
            && self.origin_list_entries == other.origin_list_entries
            && self.origin_record_capacity == other.origin_record_capacity
            && self.origin_record_archive_metadata_retained_bytes
                == other.origin_record_archive_metadata_retained_bytes
            && self.origin_key_runs == other.origin_key_runs
            && self.origin_key_run_capacity == other.origin_key_run_capacity
            && self.origin_list_span_capacity == other.origin_list_span_capacity
            && self.origin_list_entry_capacity == other.origin_list_entry_capacity
            && self.origin_list_retained_bytes == other.origin_list_retained_bytes
            && self.source_regions == other.source_regions
            && self.generated_source_backings == other.generated_source_backings
            && self.source_map_bytes == other.source_map_bytes
            && self.source_map_retained_bytes == other.source_map_retained_bytes
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
            origin_record_archive_metadata_retained_bytes: self
                .origin_record_archive_metadata_retained_bytes
                .saturating_sub(baseline.origin_record_archive_metadata_retained_bytes),
            origin_key_runs: self
                .origin_key_runs
                .saturating_sub(baseline.origin_key_runs),
            origin_key_run_capacity: self
                .origin_key_run_capacity
                .saturating_sub(baseline.origin_key_run_capacity),
            origin_list_span_capacity: self
                .origin_list_span_capacity
                .saturating_sub(baseline.origin_list_span_capacity),
            origin_list_entry_capacity: self
                .origin_list_entry_capacity
                .saturating_sub(baseline.origin_list_entry_capacity),
            origin_list_retained_bytes: self
                .origin_list_retained_bytes
                .saturating_sub(baseline.origin_list_retained_bytes),
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

/// On-demand retained-memory accounting for macro-invocation provenance.
///
/// Computing this report scans fixed-width origin records and is intentionally
/// separate from [`ProvenanceStats`] so ordinary statistics stay O(1) and
/// macro expansion does not write profiling-only counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MacroInvocationProvenanceStats {
    invocations: usize,
    retained_bytes: usize,
}

impl MacroInvocationProvenanceStats {
    #[must_use]
    pub const fn invocations(self) -> usize {
        self.invocations
    }

    #[must_use]
    pub const fn retained_bytes(self) -> usize {
        self.retained_bytes
    }

    #[must_use]
    pub const fn bytes_per_invocation(self) -> usize {
        if self.invocations == 0 {
            0
        } else {
            self.retained_bytes.div_ceil(self.invocations)
        }
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
    definition_operand: u64,
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
            definition_operand: definition.raw() as u64,
            invocation,
            definition_origin,
            parent_invocation,
        }
    }

    #[must_use]
    pub const fn definition_operand(self) -> u64 {
        self.definition_operand
    }

    pub(crate) const fn from_nonowning_operand(
        definition_operand: u64,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> Self {
        Self {
            definition_operand,
            invocation,
            definition_origin,
            parent_invocation,
        }
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
    roots: Box<[OriginRef]>,
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
            roots: Box::default(),
        }
    }

    /// Captures a diagnostic from typed provenance roots. Raw ids remain the
    /// compact presentation projection; the roots alone own runtime values.
    #[must_use]
    pub fn rooted(
        primary: Option<OriginRef>,
        related: impl IntoIterator<Item = (RelatedLocationRole, OriginRef)>,
        expansion_head: Option<ExpansionFrameRef>,
    ) -> Self {
        let mut roots = Vec::new();
        let primary_id = primary.as_ref().map(OriginRef::id);
        roots.extend(primary);
        let related = related
            .into_iter()
            .take(Self::MAX_RELATED)
            .map(|(role, origin)| {
                let location = RelatedLocation::new(role, origin.id());
                roots.push(origin);
                location
            })
            .collect();
        let expansion_head_id = expansion_head.as_ref().map(ExpansionFrameRef::id);
        roots.extend(expansion_head.map(ExpansionFrameRef::into_origin));
        Self {
            primary: primary_id,
            related,
            expansion_head: expansion_head_id,
            roots: roots.into_boxed_slice(),
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

    /// Borrows the exact typed roots owned by this diagnostic site.
    #[must_use]
    pub fn roots(&self) -> &[OriginRef] {
        &self.roots
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

#[derive(Debug)]
struct OriginValue {
    record: OriginRecord,
    children: SmallVec<[OriginRef; 3]>,
    source_registration: Option<SourceRegistrationRef>,
}

/// Strong ownership of one structural token position.
///
/// Direct source positions and graceful fallbacks allocate no atom. Arena
/// positions keep their immutable record and structural child roots alive.
#[derive(Debug)]
pub struct OriginRef {
    id: OriginId,
    value: Option<Arc<OriginValue>>,
    source_registration: Option<SourceRegistrationRef>,
}

impl Clone for OriginRef {
    fn clone(&self) -> Self {
        #[cfg(feature = "profiling")]
        if let Some(value) = &self.value {
            crate::measurement::record_provenance_root_retain(matches!(
                value.record,
                OriginRecord::MacroInvocation(_)
            ));
        }
        Self {
            id: self.id,
            value: self.value.clone(),
            source_registration: self.source_registration.clone(),
        }
    }
}

impl Drop for OriginRef {
    fn drop(&mut self) {
        #[cfg(feature = "profiling")]
        if let Some(value) = &self.value {
            crate::measurement::record_provenance_root_release(matches!(
                value.record,
                OriginRecord::MacroInvocation(_)
            ));
        }
    }
}

impl OriginRef {
    #[must_use]
    pub const fn direct(id: OriginId) -> Self {
        Self {
            id,
            value: None,
            source_registration: None,
        }
    }

    pub(crate) fn direct_registered(
        id: OriginId,
        source_registration: SourceRegistrationRef,
    ) -> Self {
        Self {
            id,
            value: None,
            source_registration: Some(source_registration),
        }
    }

    #[must_use]
    pub const fn unknown() -> Self {
        Self::direct(OriginId::UNKNOWN)
    }

    #[must_use]
    pub const fn id(&self) -> OriginId {
        self.id
    }

    #[must_use]
    pub fn record(&self) -> Option<OriginRecord> {
        self.value.as_ref().map(|value| value.record)
    }

    #[must_use]
    pub fn children(&self) -> &[OriginRef] {
        self.value
            .as_ref()
            .map_or(&[], |value| value.children.as_slice())
    }

    #[must_use]
    pub fn source_registration(&self) -> Option<&SourceRegistrationRef> {
        self.value
            .as_ref()
            .and_then(|value| value.source_registration.as_ref())
            .or(self.source_registration.as_ref())
    }
}

impl PartialEq for OriginRef {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for OriginRef {}

impl Default for OriginRef {
    fn default() -> Self {
        Self::unknown()
    }
}

impl Hash for OriginRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Strong structural root for one macro-expansion frame.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ExpansionFrameRef(pub(crate) OriginRef);

impl ExpansionFrameRef {
    #[must_use]
    pub fn unknown() -> Self {
        Self(OriginRef::unknown())
    }

    #[doc(hidden)]
    #[must_use]
    pub fn from_origin(origin: OriginRef) -> Self {
        Self(origin)
    }

    #[must_use]
    pub fn id(&self) -> OriginId {
        self.0.id()
    }

    #[must_use]
    pub fn as_origin(&self) -> &OriginRef {
        &self.0
    }

    #[must_use]
    pub fn into_origin(self) -> OriginRef {
        self.0
    }
}

/// Copy-only identity of one immutable exact token-position sequence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OriginListRef {
    id: OriginListId,
}

impl OriginListRef {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            id: OriginListId::EMPTY,
        }
    }

    pub(crate) const fn new(id: OriginListId) -> Self {
        Self { id }
    }

    #[must_use]
    pub const fn id(&self) -> OriginListId {
        self.id
    }
}

/// Borrowed exact origin sequence admitted through one live aggregate store.
pub struct OriginListView<'a> {
    runtime: RuntimeOriginListView<'a>,
    provenance: &'a ProvenanceStore,
    source_map: &'a SourceMap,
}

impl<'a> OriginListView<'a> {
    pub(crate) const fn new(
        runtime: RuntimeOriginListView<'a>,
        provenance: &'a ProvenanceStore,
        source_map: &'a SourceMap,
    ) -> Self {
        Self {
            runtime,
            provenance,
            source_map,
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.runtime.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.runtime.is_empty()
    }

    #[must_use]
    pub fn origin(&self, index: usize) -> Option<OriginId> {
        self.runtime.origin(index)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = OriginId> + '_ {
        self.runtime.iter()
    }

    /// Materializes the structural root aligned with one exact list entry.
    #[must_use]
    pub fn root(&self, index: usize) -> Option<OriginRef> {
        let id = self.origin(index)?;
        Some(
            self.provenance
                .materialize_origin_ref(id, self.source_map)
                .unwrap_or_else(|| OriginRef::direct(id)),
        )
    }

    /// Materializes structural roots in exact list order.
    pub fn roots(&self) -> impl ExactSizeIterator<Item = OriginRef> + '_ {
        self.iter().map(|id| {
            self.provenance
                .materialize_origin_ref(id, self.source_map)
                .unwrap_or_else(|| OriginRef::direct(id))
        })
    }
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
        let insertion = self.runs.partition_point(|run| run.first_key < key);
        if insertion > 0 {
            let previous = &mut self.runs[insertion - 1];
            assert!(
                key >= previous.end_key(),
                "process-global provenance key must be unique"
            );
            if key == previous.end_key() && slot == previous.end_slot() {
                previous.len = previous
                    .len
                    .checked_add(1)
                    .expect("origin key run overflow");
                return;
            }
        }
        if let Some(next) = self.runs.get(insertion) {
            assert!(
                key < next.first_key,
                "process-global provenance key must be unique"
            );
        }
        if self.runs.is_empty() {
            assert_eq!(slot, 0, "first provenance record slot must be zero");
        }
        self.runs.insert(
            insertion,
            OriginKeyRun {
                first_key: key,
                first_slot: slot,
                len: 1,
            },
        );
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
        self.runs.retain_mut(|run| {
            let Some(remaining) = records.checked_sub(run.first_slot) else {
                return false;
            };
            run.len = run.len.min(remaining);
            run.len > 0
        });
        if records == 0 {
            debug_assert!(self.runs.is_empty());
        }
    }
}

/// Conservative logical charge for one record plus its packed key/index
/// metadata. Straight-line macro workloads currently retain less than this;
/// using the charge for admission keeps branch-heavy key runs within the same
/// documented aggregate budget.
const ORIGIN_RECORD_BUDGET_CHARGE: usize = 64;
const ORIGIN_RECORD_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_ORIGIN_RECORD_LIMIT: usize = ORIGIN_RECORD_BUDGET_BYTES / ORIGIN_RECORD_BUDGET_CHARGE;
const DEFAULT_ORIGIN_LIST_SPAN_LIMIT: usize = 262_144;
const DEFAULT_ORIGIN_LIST_ENTRY_LIMIT: usize = 2_097_152;
const RECORD_CANDIDATE_KEY_BUDGET: usize = 4_096;

/// Compatibility origin-record archive plus reachability-owned provenance.
#[derive(Debug)]
pub(crate) struct ProvenanceStore {
    records: OriginRecordArchive,
    /// Exact structural candidates. Buckets are non-authoritative: packed
    /// keys still resolve through `record_keys`, and every candidate compares
    /// the complete record before reuse.
    record_candidates: HashMap<OriginRecord, Vec<u32>>,
    record_keys: OriginKeyRuns,
    next_record_key: u32,
    record_key_lease_end: u32,
    unique_candidates: SmallVec<[(OriginRecord, OriginId); 4]>,
    record_limit: usize,
}

impl Clone for ProvenanceStore {
    fn clone(&self) -> Self {
        Self {
            records: self.records.clone(),
            record_candidates: self.record_candidates.clone(),
            record_keys: self.record_keys.clone(),
            next_record_key: 0,
            record_key_lease_end: 0,
            unique_candidates: self.unique_candidates.clone(),
            record_limit: self.record_limit,
        }
    }
}

impl ProvenanceStore {
    /// Creates a provenance store with reserved unknown and empty records.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            records: OriginRecordArchive::default(),
            record_candidates: HashMap::new(),
            record_keys: OriginKeyRuns::default(),
            next_record_key: 0,
            record_key_lease_end: 0,
            unique_candidates: SmallVec::new(),
            record_limit: DEFAULT_ORIGIN_RECORD_LIMIT,
        }
    }

    pub(crate) fn configure_budgets(&mut self, budgets: ProvenanceBudgets) {
        self.record_limit = budgets.live_atoms;
    }

    /// Returns the reserved unknown/bootstrap origin id.
    #[must_use]
    pub(crate) const fn unknown_id() -> OriginId {
        OriginId::UNKNOWN
    }

    /// Allocates a new origin record, saturating capacity overflow to unknown.
    pub(crate) fn allocate(&mut self, record: OriginRecord) -> OriginId {
        if let Some(existing) = self.exact_record_candidate(record) {
            return existing;
        }
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
        let Some(key) = self.next_packed_arena_origin() else {
            return OriginId::UNKNOWN;
        };
        let slot = u32::try_from(self.records.len())
            .expect("global origin key capacity bounds provenance record slots");
        self.records.append(key, record);
        self.record_keys.append(key, slot);
        self.index_record_candidate(record, key);
        OriginId::arena(key).expect("global packed provenance key is representable")
    }

    /// Appends a record whose occurrence identity is semantically unique.
    ///
    /// Macro invocation frames use this path: their call-site and parent
    /// coordinates already distinguish occurrences, so a weak exact-candidate
    /// entry would add hot allocation and reference-count traffic without a
    /// possible reuse benefit.
    pub(crate) fn allocate_unique(&mut self, record: OriginRecord) -> OriginId {
        if let Some((_, id)) = self.unique_candidates.iter().rev().find(|(candidate, id)| {
            *candidate == record
                && matches!(
                    id.decode(),
                    OriginEncoding::Arena(key)
                        if self.record_keys.slot(key).is_some_and(|slot| {
                            self.records.get_slot(slot as usize) == Some(record)
                        })
                )
        }) {
            return *id;
        }
        if self.records.len() >= self.record_limit {
            return OriginId::UNKNOWN;
        }
        let Some(key) = self.next_packed_arena_origin() else {
            return OriginId::UNKNOWN;
        };
        let slot = u32::try_from(self.records.len())
            .expect("global origin key capacity bounds provenance record slots");
        self.records.append(key, record);
        self.record_keys.append(key, slot);
        let id = OriginId::arena(key).expect("global packed provenance key is representable");
        if self.unique_candidates.len() == 4 {
            self.unique_candidates.remove(0);
        }
        self.unique_candidates.push((record, id));
        id
    }

    /// Archives one immutable origin and returns an optional cold structural
    /// projection. The archive coordinate remains authoritative after the
    /// projection is dropped.
    pub(crate) fn allocate_rooted(
        &mut self,
        record: OriginRecord,
        children: impl IntoIterator<Item = OriginRef>,
    ) -> OriginRef {
        self.allocate_rooted_with_registration(record, children, None)
    }

    pub(crate) fn allocate_rooted_with_registration(
        &mut self,
        record: OriginRecord,
        children: impl IntoIterator<Item = OriginRef>,
        source_registration: Option<SourceRegistrationRef>,
    ) -> OriginRef {
        let id = self.allocate(record);
        if !matches!(id.decode(), OriginEncoding::Arena(_)) {
            return OriginRef::direct(id);
        }
        let value = Arc::new(OriginValue {
            record,
            children: children.into_iter().collect(),
            source_registration,
        });
        OriginRef {
            id,
            value: Some(value),
            source_registration: None,
        }
    }

    pub(crate) fn origin_ref(&self, id: OriginId) -> Option<OriginRef> {
        #[cfg(feature = "profiling")]
        crate::measurement::record_provenance_origin_resolution();
        #[cfg(feature = "profiling")]
        let _allocation_scope = crate::measurement::hot_core_allocation_scope(
            crate::measurement::HotCoreAllocationOwner::ProvenanceMaterialization,
        );
        let resolved = match id.decode() {
            crate::token::OriginEncoding::Unknown
            | crate::token::OriginEncoding::NoExpandFallback
            | crate::token::OriginEncoding::DirectSource(_) => Some(OriginRef::direct(id)),
            crate::token::OriginEncoding::Arena(key) => {
                self.record_keys.slot(key).map(|_| OriginRef::direct(id))
            }
        };
        #[cfg(feature = "profiling")]
        crate::measurement::record_hot_core_provenance_materialization(resolved.is_some());
        resolved
    }

    /// Materializes an archived coordinate as a strong structural root.
    ///
    /// Ordinary macro replay carries only the archived coordinate. Cold node,
    /// diagnostic, and continuation publication call this boundary when the
    /// record must outlive the command arena.
    pub(crate) fn materialize_origin_ref(
        &self,
        id: OriginId,
        source_map: &crate::source_map::SourceMap,
    ) -> Option<OriginRef> {
        #[cfg(feature = "profiling")]
        crate::measurement::record_provenance_origin_resolution();
        let OriginEncoding::Arena(_) = id.decode() else {
            if let OriginEncoding::DirectSource(position) = id.decode() {
                return Some(source_map.registration_for_position(position).map_or_else(
                    || OriginRef::direct(id),
                    |registration| OriginRef::direct_registered(id, registration),
                ));
            }
            return Some(OriginRef::direct(id));
        };
        if !self.contains_origin(id) {
            #[cfg(feature = "profiling")]
            crate::measurement::record_hot_core_provenance_materialization(false);
            return None;
        }
        let record = self.get(id);
        let child_ids: SmallVec<[OriginId; 3]> = match record {
            OriginRecord::MacroInvocation(origin) => smallvec::smallvec![
                origin.invocation(),
                origin.definition_origin(),
                origin.parent_invocation(),
            ],
            OriginRecord::Inserted(origin) => smallvec::smallvec![origin.parent()],
            OriginRecord::Synthesized(origin) => smallvec::smallvec![origin.parent()],
            _ => SmallVec::new(),
        };
        let children = child_ids
            .into_iter()
            .map(|child| self.materialize_origin_ref(child, source_map))
            .collect::<Option<SmallVec<[OriginRef; 3]>>>()?;
        let source_registration = match record {
            OriginRecord::SourceSpan(span) => source_map.registration_for_span(span),
            _ => None,
        };
        let value = Arc::new(OriginValue {
            record,
            children,
            source_registration,
        });
        #[cfg(feature = "profiling")]
        crate::measurement::record_hot_core_provenance_materialization(true);
        Some(OriginRef {
            id,
            value: Some(value),
            source_registration: None,
        })
    }

    /// Cold atom projections are deliberately not indexed by the store.
    #[cfg(test)]
    const fn rooted_record_shape(&self) -> (usize, usize) {
        (0, 0)
    }

    fn index_record_candidate(&mut self, record: OriginRecord, key: u32) {
        if self.record_candidates.len() >= RECORD_CANDIDATE_KEY_BUDGET
            && !self.record_candidates.contains_key(&record)
        {
            self.record_candidates.clear();
        }
        self.record_candidates.entry(record).or_default().push(key);
    }

    fn exact_record_candidate(&self, record: OriginRecord) -> Option<OriginId> {
        self.record_candidates.get(&record).and_then(|candidates| {
            candidates.iter().copied().find_map(|key| {
                let slot = self.record_keys.slot(key)?;
                (self.records.get_slot(slot as usize) == Some(record))
                    .then(|| OriginId::arena(key))
                    .flatten()
            })
        })
    }

    fn next_packed_arena_origin(&mut self) -> Option<u32> {
        if self.next_record_key == self.record_key_lease_end {
            let lease = reserve_packed_arena_origins()?;
            self.next_record_key = lease.start;
            self.record_key_lease_end = lease.end;
        }
        let key = self.next_record_key;
        self.next_record_key += 1;
        Some(key)
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

    /// Returns compatibility origin-record counters.
    ///
    /// The aggregate store adds runtime-region origin-list accounting because
    /// this archive is deliberately no longer an origin-list authority.
    #[must_use]
    pub(crate) fn stats(&self) -> ProvenanceStats {
        ProvenanceStats::with_capacities(
            self.records.len(),
            0,
            0,
            OriginRecordStorageStats {
                capacity: self.records.capacity(),
                archive_metadata_retained_bytes: self.records.retained_metadata_bytes(),
                key_runs: self.record_keys.runs.len(),
                key_run_capacity: self.record_keys.runs.capacity(),
            },
            0,
            0,
            0,
        )
    }

    /// Scans live fixed-width records to report the macro-invocation share of
    /// retained arena and packed-key storage.
    #[must_use]
    pub(crate) fn macro_invocation_stats(&self) -> MacroInvocationProvenanceStats {
        let invocations = self.records.macro_invocation_records();
        let records = self.records.len();
        let record_bytes = self.stats().origin_record_retained_bytes();
        let retained_bytes = if records == 0 {
            0
        } else {
            record_bytes.saturating_mul(invocations).div_ceil(records)
        };
        MacroInvocationProvenanceStats {
            invocations,
            retained_bytes,
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn macro_invocation_origins(&self) -> Vec<OriginId> {
        self.records.macro_invocation_origins()
    }

    /// Takes a rollback watermark for aggregate snapshots.
    #[must_use]
    pub(crate) fn watermark(&self) -> ProvenanceStoreMark {
        ProvenanceStoreMark {
            records: u32_len(self.records.len())
                .expect("provenance record arena exceeded representable mark"),
        }
    }

    /// Truncates to a previously-taken aggregate snapshot watermark.
    pub(crate) fn truncate_to(&mut self, mark: ProvenanceStoreMark) {
        self.truncate_to_inner(mark);
    }

    fn truncate_to_inner(&mut self, mark: ProvenanceStoreMark) {
        let records = mark.records as usize;
        assert!(
            records <= self.records.len(),
            "provenance mark has too many records"
        );

        for slot in records..self.records.len() {
            let (key, record) = self
                .records
                .get_entry(slot)
                .expect("discarded provenance slot is live");
            if let Some(candidates) = self.record_candidates.get_mut(&record) {
                candidates.retain(|candidate| *candidate != key);
                if candidates.is_empty() {
                    self.record_candidates.remove(&record);
                }
            }
        }
        self.record_keys.truncate(mark.records);
        self.records.truncate(records);
    }
}

fn u32_len(value: usize) -> Option<u32> {
    u32::try_from(value).ok()
}

fn reserve_packed_arena_origins() -> Option<std::ops::Range<u32>> {
    let start = NEXT_PACKED_ARENA_ORIGIN
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            (next < PACKED_ARENA_ORIGIN_END).then(|| {
                next.saturating_add(ORIGIN_KEY_LEASE_LEN)
                    .min(PACKED_ARENA_ORIGIN_END)
            })
        })
        .ok()?;
    let end = start
        .saturating_add(ORIGIN_KEY_LEASE_LEN)
        .min(PACKED_ARENA_ORIGIN_END);
    Some(start..end)
}

#[cfg(test)]
fn packed_origin_successor(next: u32) -> Option<u32> {
    (next <= 0x7fff_ffff).then_some(next + 1)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
fn arena_index(value: usize) -> Option<u32> {
    let value = u32::try_from(value).ok()?;
    (value <= 0x7fff_ffff).then_some(value)
}

#[cfg(test)]
mod tests;
