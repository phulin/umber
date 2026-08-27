//! External-effect capability boundary for the engine.
//!
//! This is the only engine module that may name host I/O and clock APIs.
//! Higher layers receive content-addressed inputs, buffered effect records,
//! deterministic RNG values, and job-start clock parameters through this API.

#![allow(clippy::disallowed_methods)]

use crate::env::banks::IntParam;
use crate::identity::{HandleIdentity, IdentityAllocator, IdentityMark};
use crate::memo::DetachedMemoValue;
use crate::state_hash::StateHashFragment;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "profiling")]
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
pub use tex_content::{ContentDomain, ContentHash, ContentIdentity};

/// TeX's 16 read/write stream slots.
pub const STREAM_SLOT_COUNT: usize = 16;

/// Hard ceiling for distinct semantic input paths retained by one World.
pub const MAX_INPUT_DEPENDENCIES: usize = 8_192;

static NEXT_TERMINAL_INPUT_OWNER: AtomicU64 = AtomicU64::new(1);

fn fresh_terminal_input_owner() -> u64 {
    NEXT_TERMINAL_INPUT_OWNER
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .expect("terminal-input owner identity space exhausted")
}

/// An output-open answer that is safe to use before effect commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedOutputOpenOutcome {
    Available,
    Unavailable,
    DeferredToCommit,
}

/// A process-local elapsed-time sample obtained through the host-effect boundary.
///
/// Profiling data is deliberately separate from the snapshot-owned pdfTeX
/// clock: it is neither semantic state nor replayable engine input.
#[cfg(feature = "profiling")]
pub struct ProfilingTimer(Instant);

#[cfg(feature = "profiling")]
impl ProfilingTimer {
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

/// Host-materialization policy for one engine timeline.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WorldCommitMode {
    /// Shipout immediately exposes effects to the configured host backend.
    #[default]
    Eager,
    /// Shipout advances TeX-visible virtual state while host effects remain retained.
    Retained,
    /// A retained session has exported its effects and cannot be rolled back again.
    Exported,
}

/// Handle-free source recipe retained by a committed artifact.
///
/// The content identity is durable authority; path and byte range are owned
/// presentation data. No source id, origin coordinate, or arena owner crosses
/// the artifact boundary.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ArtifactSourceRecipe {
    pub content: ContentHash,
    pub logical_path: String,
    pub start: u64,
    pub end: u64,
}

/// DTO-local ordinal of an effect referenced by a committed artifact.
///
/// This is relative to the artifact's detached effect journal. It is not a
/// live [`EffectPos`] cursor and carries no World timeline identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ArtifactEffectOrdinal(u32);

impl ArtifactEffectOrdinal {
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Exact bytes published by one successful page-artifact commit.
///
/// Construction stays inside the aggregate shipout boundary, so downstream
/// code can consume these bytes without rereading and reverifying the
/// content-addressed store.  The content id remains the authoritative durable
/// reference for replay and out-of-process drivers.
#[derive(Clone, Debug)]
pub struct CommittedArtifact {
    hash: ContentHash,
    bytes: Vec<u8>,
    render_provenance: ArtifactRenderProvenance,
    open_out_occurrences: Vec<(usize, ArtifactEffectOrdinal)>,
}

#[derive(Clone, Debug)]
pub(crate) struct ArtifactRenderProvenance {
    ends: Vec<u32>,
    sources: Vec<Option<ArtifactSourceRecipe>>,
}

impl ArtifactRenderProvenance {
    fn live(ends: Vec<u32>, origins: Vec<ArtifactSourceRecipe>) -> Self {
        assert_valid_render_origins(&ends, origins.len());
        Self {
            ends,
            sources: origins.into_iter().map(Some).collect(),
        }
    }

    fn built(ends: Vec<u32>, builder: RenderProvenanceBuilder) -> Self {
        let flat_len = ends.last().copied().unwrap_or(0) as usize;
        assert_eq!(flat_len, builder.sources.len());
        Self {
            ends,
            sources: builder.sources,
        }
    }

    fn is_deferred(&self) -> bool {
        self.sources.iter().any(Option::is_none)
    }
}

/// Cold artifact-provenance construction selected by rendered-source demand.
#[doc(hidden)]
#[derive(Debug)]
pub struct RenderProvenanceBuilder {
    sources: Vec<Option<ArtifactSourceRecipe>>,
}

impl RenderProvenanceBuilder {
    /// Opens artifact presentation staging only for a rendered-source job.
    #[must_use]
    pub fn for_demand(demand: crate::ProvenanceDemand) -> Option<Self> {
        demand.rendered_source().then(|| Self {
            sources: Vec::new(),
        })
    }

    #[doc(hidden)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub fn push_source(&mut self, source: ArtifactSourceRecipe) {
        self.sources.push(Some(source));
    }

    pub fn push_unknown(&mut self) {
        self.sources.push(None);
    }
}

/// One artifact source reference, kept stable until diagnostic consumption.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactOrigin {
    /// Already-detached source presentation owned directly by the artifact.
    Detached(ArtifactSourceRecipe),
    Unknown,
}

/// Borrowed diagnostic provenance spans aligned with artifact nodes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderOrigins<'a> {
    ends: &'a [u32],
    origins: &'a [Option<ArtifactSourceRecipe>],
}

impl<'a> RenderOrigins<'a> {
    #[must_use]
    pub const fn len(self) -> usize {
        self.ends.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.ends.is_empty()
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<&'a [Option<ArtifactSourceRecipe>]> {
        let &end = self.ends.get(index)?;
        let start = index
            .checked_sub(1)
            .and_then(|previous| self.ends.get(previous).copied())
            .unwrap_or(0);
        self.origins.get(start as usize..end as usize)
    }

    pub fn iter(self) -> RenderOriginIter<'a> {
        RenderOriginIter {
            origins: self,
            index: 0,
        }
    }
}

impl<'a> IntoIterator for RenderOrigins<'a> {
    type Item = &'a [Option<ArtifactSourceRecipe>];
    type IntoIter = RenderOriginIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Clone, Debug)]
pub struct RenderOriginIter<'a> {
    origins: RenderOrigins<'a>,
    index: usize,
}

impl<'a> Iterator for RenderOriginIter<'a> {
    type Item = &'a [Option<ArtifactSourceRecipe>];

    fn next(&mut self) -> Option<Self::Item> {
        let origins = self.origins.get(self.index)?;
        self.index += 1;
        Some(origins)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.origins.len().saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for RenderOriginIter<'_> {}

/// Artifact bytes paired with their already-computed content identity.
///
/// Construction hashes the bytes exactly once. Private fields keep identity
/// and payload inseparable across the shipout commit boundary.
#[derive(Clone, Debug)]
pub struct VerifiedArtifact {
    hash: ContentHash,
    bytes: Vec<u8>,
    render_provenance: ArtifactRenderProvenance,
    open_out_occurrences: Vec<(usize, ArtifactEffectOrdinal)>,
}

impl VerifiedArtifact {
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        let hash = ContentHash::for_domain(ContentDomain::Artifact, &bytes);
        Self {
            hash,
            bytes,
            render_provenance: ArtifactRenderProvenance::live(Vec::new(), Vec::new()),
            open_out_occurrences: Vec::new(),
        }
    }

    /// Attaches exact ordered World occurrences to OpenOut effects.
    #[doc(hidden)]
    #[must_use]
    pub fn with_open_out_occurrences(
        mut self,
        occurrences: Vec<(usize, ArtifactEffectOrdinal)>,
    ) -> Self {
        self.open_out_occurrences = occurrences;
        self
    }

    /// Attaches diagnostic-only origins in artifact-node preorder.
    #[must_use]
    pub fn with_render_origins(mut self, render_origins: Vec<Vec<ArtifactSourceRecipe>>) -> Self {
        let mut ends = Vec::with_capacity(render_origins.len());
        let mut origins = Vec::with_capacity(render_origins.iter().map(Vec::len).sum());
        for node_origins in render_origins {
            origins.extend(node_origins);
            ends.push(
                u32::try_from(origins.len())
                    .expect("artifact render provenance exceeds u32 entries"),
            );
        }
        self.render_provenance = ArtifactRenderProvenance::live(ends, origins);
        self
    }

    /// Attaches already-flattened diagnostic origins in artifact-node order.
    #[doc(hidden)]
    #[must_use]
    pub fn with_flat_render_origins(
        mut self,
        render_origin_ends: Vec<u32>,
        render_origins: Vec<ArtifactSourceRecipe>,
    ) -> Self {
        self.render_provenance = ArtifactRenderProvenance::live(render_origin_ends, render_origins);
        self
    }

    #[doc(hidden)]
    #[must_use]
    pub fn with_built_render_origins(
        mut self,
        render_origin_ends: Vec<u32>,
        provenance: RenderProvenanceBuilder,
    ) -> Self {
        self.render_provenance = ArtifactRenderProvenance::built(render_origin_ends, provenance);
        self
    }

    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Diagnostic provenance captured before a memoized shipout commit.
    #[doc(hidden)]
    #[must_use]
    pub fn render_origins_for_memo(&self) -> RenderOrigins<'_> {
        RenderOrigins {
            ends: &self.render_provenance.ends,
            origins: &self.render_provenance.sources,
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn render_origin_ends_for_memo(&self) -> &[u32] {
        &self.render_provenance.ends
    }

    #[doc(hidden)]
    #[must_use]
    pub fn has_deferred_render_origins(&self) -> bool {
        self.render_provenance.is_deferred()
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<u8>,
        ArtifactRenderProvenance,
        Vec<(usize, ArtifactEffectOrdinal)>,
    ) {
        (
            self.bytes,
            self.render_provenance,
            self.open_out_occurrences,
        )
    }
}

impl PartialEq for VerifiedArtifact {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.bytes == other.bytes
            && self.open_out_occurrences == other.open_out_occurrences
    }
}

impl Eq for VerifiedArtifact {}

impl CommittedArtifact {
    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn open_out_occurrences(&self) -> &[(usize, ArtifactEffectOrdinal)] {
        &self.open_out_occurrences
    }

    /// Rebases occurrences owned by an adopted effect suffix.
    #[doc(hidden)]
    pub fn rebase_open_out_suffix(
        &mut self,
        old_prefix: usize,
        new_prefix: usize,
    ) -> Result<(), WorldError> {
        let old_prefix = u32::try_from(old_prefix).map_err(|_| {
            WorldError::new(
                "rebase artifact effects",
                None,
                "old effect prefix overflow",
            )
        })?;
        let new_prefix = u32::try_from(new_prefix).map_err(|_| {
            WorldError::new(
                "rebase artifact effects",
                None,
                "new effect prefix overflow",
            )
        })?;
        for (_, position) in &mut self.open_out_occurrences {
            if position.index() <= old_prefix {
                continue;
            }
            let suffix_offset = position.index() - old_prefix;
            *position =
                ArtifactEffectOrdinal::new(new_prefix.checked_add(suffix_offset).ok_or_else(
                    || WorldError::new("rebase artifact effects", None, "effect position overflow"),
                )?);
        }
        Ok(())
    }

    /// Replaces prepared bytes while retaining the diagnostic provenance sidecar.
    ///
    /// This is used only before publication when TeX82's openout retry changes
    /// the page effect payload and therefore its final content identity.
    pub fn with_prepared_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.hash = ContentHash::for_domain(ContentDomain::Artifact, &bytes);
        self.bytes = bytes;
        self
    }

    /// Replaces bytes without changing their asserted identity for corruption tests.
    ///
    /// This deliberately constructs an invalid committed-artifact boundary and
    /// must only be used to exercise downstream rejection paths.
    #[doc(hidden)]
    pub fn with_testing_bytes_preserving_identity(mut self, bytes: Vec<u8>) -> Self {
        self.bytes = bytes;
        self
    }

    /// Detached diagnostic sources aligned with artifact nodes in preorder.
    #[must_use]
    pub fn render_origins(&self) -> Option<RenderOrigins<'_>> {
        Some(RenderOrigins {
            ends: &self.render_provenance.ends,
            origins: &self.render_provenance.sources,
        })
    }

    /// Number of artifact nodes addressable through [`Self::render_origin`].
    #[must_use]
    pub fn render_node_count(&self) -> usize {
        self.render_provenance.ends.len()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn has_deferred_render_origins(&self) -> bool {
        self.render_provenance.is_deferred()
    }

    /// Returns one diagnostic source without materializing deferred origins.
    #[must_use]
    pub fn render_origin(&self, node: usize, source: usize) -> ArtifactOrigin {
        let Some(&end) = self.render_provenance.ends.get(node) else {
            return ArtifactOrigin::Unknown;
        };
        let start = node
            .checked_sub(1)
            .and_then(|previous| self.render_provenance.ends.get(previous).copied())
            .unwrap_or(0);
        let Some(flat) = (start as usize).checked_add(source) else {
            return ArtifactOrigin::Unknown;
        };
        if flat >= end as usize {
            return ArtifactOrigin::Unknown;
        }
        self.render_provenance
            .sources
            .get(flat)
            .and_then(Clone::clone)
            .map_or(ArtifactOrigin::Unknown, ArtifactOrigin::Detached)
    }

    /// Retained bytes used by the diagnostic-only provenance sidecar.
    #[must_use]
    pub fn render_provenance_bytes(&self) -> usize {
        self.render_provenance
            .ends
            .len()
            .saturating_mul(std::mem::size_of::<u32>())
            .saturating_add(
                self.render_provenance
                    .sources
                    .len()
                    .saturating_mul(std::mem::size_of::<Option<ArtifactSourceRecipe>>()),
            )
            .saturating_add(
                self.render_provenance
                    .sources
                    .iter()
                    .flatten()
                    .map(|source| source.logical_path.len())
                    .sum::<usize>(),
            )
    }

    pub(crate) fn new(
        hash: ContentHash,
        bytes: Vec<u8>,
        render_provenance: ArtifactRenderProvenance,
        open_out_occurrences: Vec<(usize, ArtifactEffectOrdinal)>,
    ) -> Self {
        Self {
            hash,
            bytes,
            render_provenance,
            open_out_occurrences,
        }
    }
}

fn assert_valid_render_origins(ends: &[u32], origin_len: usize) {
    assert!(
        ends.windows(2).all(|ends| ends[0] <= ends[1]),
        "artifact render-origin ends must be monotonic"
    );
    assert_eq!(
        ends.last().copied().unwrap_or(0) as usize,
        origin_len,
        "artifact render-origin ends must cover the flat origin buffer"
    );
}

impl PartialEq for CommittedArtifact {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.bytes == other.bytes
            && self.open_out_occurrences == other.open_out_occurrences
    }
}

impl Eq for CommittedArtifact {}

/// Bytes returned from a content-addressed `World` read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileContent {
    record: InputRecordId,
    path: PathBuf,
    bytes: Arc<[u8]>,
    hash: ContentHash,
    modification_date: Option<FileModificationDate>,
    origin: InputOrigin,
}

impl FileContent {
    #[must_use]
    pub(crate) fn new(record: InputRecordId, path: PathBuf, bytes: Vec<u8>) -> Self {
        Self::from_shared(record, path, bytes.into(), None, InputOrigin::External)
    }

    #[must_use]
    fn from_shared(
        record: InputRecordId,
        path: PathBuf,
        bytes: Arc<[u8]>,
        modification_date: Option<FileModificationDate>,
        origin: InputOrigin,
    ) -> Self {
        let hash = ContentHash::from_bytes(&bytes);
        Self {
            record,
            path,
            bytes,
            hash,
            modification_date,
            origin,
        }
    }

    /// Returns the stable record for this successful `World` read.
    #[must_use]
    pub const fn record(&self) -> InputRecordId {
        self.record
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn shared_bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }

    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    /// Returns immutable modification metadata captured with this read.
    #[must_use]
    pub const fn modification_date(&self) -> Option<FileModificationDate> {
        self.modification_date
    }

    /// Returns whether these bytes came from an immutable external input or
    /// from an output generated transactionally by the current TeX run.
    #[must_use]
    pub const fn origin(&self) -> InputOrigin {
        self.origin
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes.to_vec()
    }
}

/// Provenance of bytes returned by a successful input read.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InputOrigin {
    /// Immutable bytes selected from the host, VFS, or resource distribution.
    External,
    /// Rollback-safe bytes written and reopened during the current TeX run.
    SameRunGenerated,
}

/// Host-neutral civil modification time attached to immutable file content.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FileModificationDate {
    pub clock: JobClock,
    pub utc_offset_minutes: i16,
}

impl FileModificationDate {
    #[must_use]
    pub const fn utc(clock: JobClock) -> Self {
        Self {
            clock,
            utc_offset_minutes: 0,
        }
    }

    #[must_use]
    pub const fn with_offset(clock: JobClock, utc_offset_minutes: i16) -> Self {
        Self {
            clock,
            utc_offset_minutes,
        }
    }
}

/// Rollback-safe identity of one successful read in the `World` input log.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputRecordId(HandleIdentity);

impl InputRecordId {
    #[must_use]
    pub(crate) const fn raw(self) -> u32 {
        self.0.slot()
    }
}

impl std::hash::Hash for InputRecordId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.raw(), state);
    }
}

/// One recorded file read.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InputRecord {
    path: PathBuf,
    hash: ContentHash,
    len: usize,
    modification_date: Option<FileModificationDate>,
    origin: InputOrigin,
}

/// Semantic class of one external input observation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InputDependencyAccess {
    /// The engine required the file's bytes to continue execution.
    RequiredRead,
    /// The engine authoritatively tested whether the file was available.
    AuthoritativeProbe,
}

/// Immutable outcome observed for one canonical external input path.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InputDependencyOutcome {
    Present(ContentHash),
    Missing,
}

/// One reduced semantic external-input dependency.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InputDependency {
    path: Arc<Path>,
    outcome: InputDependencyOutcome,
    access: InputDependencyAccess,
}

impl InputDependency {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn outcome(&self) -> InputDependencyOutcome {
        self.outcome
    }

    #[must_use]
    pub const fn access(&self) -> InputDependencyAccess {
        self.access
    }
}

impl InputRecord {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub const fn modification_date(&self) -> Option<FileModificationDate> {
        self.modification_date
    }

    /// Returns the provenance of this successful read.
    #[must_use]
    pub const fn origin(&self) -> InputOrigin {
        self.origin
    }

    /// Returns whether this read belongs to the immutable external dependency
    /// closure used by retained validation and format-cache receipts.
    #[must_use]
    pub const fn is_external_dependency(&self) -> bool {
        matches!(self.origin, InputOrigin::External)
    }
}

/// A TeX stream slot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct StreamSlot(u8);

impl StreamSlot {
    #[must_use]
    pub const fn new(raw: u8) -> Self {
        assert!(
            raw < STREAM_SLOT_COUNT as u8,
            "TeX stream slot must be in 0..16"
        );
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The kind of sink a write is routed to.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum PrintSink {
    Terminal,
    Log,
    TerminalAndLog,
    Stream(StreamSlot),
}

/// Buffered write-stream target.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WriteTarget {
    path: PathBuf,
}

impl WriteTarget {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// One materialized output borrowed from a memory-backed [`World`].
///
/// This deliberately exposes only the immutable path and bytes. Backend
/// storage and effect-timeline control remain private to `World`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryOutput<'a> {
    path: &'a Path,
    bytes: &'a [u8],
}

impl<'a> MemoryOutput<'a> {
    #[must_use]
    pub const fn path(self) -> &'a Path {
        self.path
    }

    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }
}

/// Buffered read-stream target pinned to content read through `World`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ReadTarget {
    path: PathBuf,
    hash: ContentHash,
    next_byte: usize,
}

impl ReadTarget {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    #[must_use]
    pub const fn next_byte(&self) -> usize {
        self.next_byte
    }
}

/// Snapshot-ready state for all partial stream/log buffers.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct StreamBufState {
    read_streams: [Option<ReadTarget>; STREAM_SLOT_COUNT],
    write_streams: [Option<WriteTarget>; STREAM_SLOT_COUNT],
    log_partial_line: String,
    terminal_partial_line: String,
    terminal_input_next: usize,
}

/// Opaque cursor for restoring a borrowed terminal-input position without
/// exposing or replacing the World's retained terminal line storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalInputPosition {
    owner: u64,
    next: usize,
}

impl StreamBufState {
    #[must_use]
    pub fn read_stream_path(&self, slot: StreamSlot) -> Option<&Path> {
        self.read_streams[slot.index()]
            .as_ref()
            .map(ReadTarget::path)
    }

    #[must_use]
    pub fn read_stream_target(&self, slot: StreamSlot) -> Option<&ReadTarget> {
        self.read_streams[slot.index()].as_ref()
    }

    #[must_use]
    pub fn write_stream_target(&self, slot: StreamSlot) -> Option<&WriteTarget> {
        self.write_streams[slot.index()].as_ref()
    }

    #[must_use]
    pub fn log_partial_line(&self) -> &str {
        &self.log_partial_line
    }

    #[must_use]
    pub fn terminal_partial_line(&self) -> &str {
        &self.terminal_partial_line
    }

    #[must_use]
    pub const fn terminal_input_next(&self) -> usize {
        self.terminal_input_next
    }
}

/// Absolute position in the append-only effect log.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EffectPos(u64);

impl EffectPos {
    #[must_use]
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn advanced_by(self, count: u64) -> Self {
        Self(self.0.saturating_add(count))
    }

    #[must_use]
    pub const fn retreated_by(self, count: u64) -> Self {
        Self(self.0.saturating_sub(count))
    }
}

/// One append-only effect record.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum EffectRecord {
    StreamOpen {
        slot: StreamSlot,
        target: WriteTarget,
    },
    StreamClose {
        slot: StreamSlot,
    },
    StreamWrite {
        sink: PrintSink,
        text: String,
    },
    /// Externally encoded output whose byte identity must survive retention.
    StreamWriteBytes {
        sink: PrintSink,
        bytes: Vec<u8>,
    },
    /// Deferred `\write` seam: the token list is intentionally unexpanded.
    DeferredWrite {
        stream: StreamSlot,
        tokens: DetachedMemoValue,
    },
    Special {
        class: String,
        payload: Vec<u8>,
    },
    PdfObjectPlaceholder {
        label: String,
    },
    ShellEscape(ShellEscapeRecord),
}

impl EffectRecord {
    /// Retargets one exact detached `StreamOpen` without exposing the
    /// `WriteTarget` constructor outside the World boundary.
    #[doc(hidden)]
    pub fn retarget_detached_stream_open(
        &mut self,
        slot: StreamSlot,
        failed: &Path,
        replacement: PathBuf,
    ) -> bool {
        let Self::StreamOpen {
            slot: candidate,
            target,
        } = self
        else {
            return false;
        };
        if *candidate != slot || target.path != failed {
            return false;
        }
        target.path = replacement;
        true
    }
}

/// Value identity of one immutable effect prefix.
///
/// The stamp neither owns nor upgrades a runtime root.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EffectRootIdentity {
    len: usize,
    hash: u64,
}

impl EffectRootIdentity {
    #[must_use]
    pub fn is_mounted_in(&self, world: &World) -> bool {
        if *self == effect_root_identity_for(&world.effects) {
            return true;
        }
        let mut block = world.accepted_effects.as_deref();
        while let Some(current) = block {
            if *self == effect_root_identity_for(&current.effects[..current.len]) {
                return true;
            }
            block = current.parent.as_deref();
        }
        false
    }
}

fn effect_root_identity_for(records: &[EffectRecord]) -> EffectRootIdentity {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    records.hash(&mut hasher);
    EffectRootIdentity {
        len: records.len(),
        hash: hasher.finish(),
    }
}

fn stable_hash(value: &impl std::hash::Hash) -> u64 {
    use std::hash::Hasher as _;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn world_scalar_identities(world: &World) -> [(u64, u64); 7] {
    [
        (0, stable_hash(world.stream_bufs.as_ref())),
        (1, stable_hash(&world.rng)),
        (2, stable_hash(&world.pdf_rng)),
        (3, stable_hash(&world.pdf_time_micros)),
        (4, stable_hash(&world.pdf_timer_origin_micros)),
        (5, stable_hash(&world.job_clock)),
        (6, stable_hash(&world.shell_escape_policy)),
    ]
}

impl EffectRecord {
    /// Opaque retained-memory charge for detached session accounting.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        effect_retained_bytes(self)
    }
}

/// Deterministic xoshiro256** RNG state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RngState {
    state: [u64; 4],
}

impl RngState {
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        let mut value = seed;
        let mut state = [0; 4];
        for slot in &mut state {
            value = splitmix64(value);
            *slot = value;
        }
        if state == [0; 4] {
            state[0] = 1;
        }
        Self { state }
    }

    #[must_use]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.state[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.state[1] << 17;

        self.state[2] ^= self.state[0];
        self.state[3] ^= self.state[1];
        self.state[1] ^= self.state[2];
        self.state[0] ^= self.state[3];
        self.state[2] ^= t;
        self.state[3] = self.state[3].rotate_left(45);

        result
    }
}

impl Default for RngState {
    fn default() -> Self {
        Self::from_seed(0x9e37_79b9_7f4a_7c15)
    }
}

/// pdfTeX's MetaPost-derived subtractive random-number generator.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PdfRandomState {
    values: [i32; 55],
    next: usize,
    seed: i32,
}

impl PdfRandomState {
    fn from_seed(seed: i32) -> Self {
        let seed = seed.saturating_abs();
        let mut state = Self {
            values: [0; 55],
            next: 0,
            seed,
        };
        state.initialize(seed);
        state
    }

    fn initialize(&mut self, seed: i32) {
        const FRACTION_ONE: i32 = 1 << 28;
        let mut j = seed;
        while j >= FRACTION_ONE {
            j /= 2;
        }
        let mut k = 1;
        for i in 0..55 {
            let jj = k;
            k = j - k;
            j = jj;
            if k < 0 {
                k += FRACTION_ONE;
            }
            self.values[(i * 21) % 55] = j;
        }
        self.refresh();
        self.refresh();
        self.refresh();
    }

    fn refresh(&mut self) {
        const FRACTION_ONE: i32 = 1 << 28;
        for k in 0..24 {
            let mut value = self.values[k] - self.values[k + 31];
            if value < 0 {
                value += FRACTION_ONE;
            }
            self.values[k] = value;
        }
        for k in 24..55 {
            let mut value = self.values[k] - self.values[k - 24];
            if value < 0 {
                value += FRACTION_ONE;
            }
            self.values[k] = value;
        }
        self.next = 54;
    }

    fn next_fraction(&mut self) -> i32 {
        if self.next == 0 {
            self.refresh();
        } else {
            self.next -= 1;
        }
        self.values[self.next]
    }

    fn uniform(&mut self, bound: i32) -> i32 {
        let magnitude = i64::from(bound).abs();
        let trial = take_fraction(magnitude, i64::from(self.next_fraction()));
        let trial = if trial == magnitude { 0 } else { trial };
        if bound < 0 {
            -(trial as i32)
        } else {
            trial as i32
        }
    }

    fn normal(&mut self) -> i32 {
        const FRACTION_HALF: i64 = 1 << 27;
        loop {
            let (x, u) = loop {
                let x = take_fraction(112_429, i64::from(self.next_fraction()) - FRACTION_HALF);
                let u = i64::from(self.next_fraction());
                if x.abs() < u {
                    break (x, u);
                }
            };
            let x = make_fraction(x, u);
            let l = 139_548_960 - metapost_log(u);
            if 1024_i64 * l >= x * x {
                return x as i32;
            }
        }
    }
}

impl Default for PdfRandomState {
    fn default() -> Self {
        Self::from_seed(0)
    }
}

fn take_fraction(value: i64, fraction: i64) -> i64 {
    let negative = (value < 0) != (fraction < 0);
    let rounded = (value.abs() * fraction.abs() + (1 << 27)) / (1 << 28);
    if negative { -rounded } else { rounded }
}

fn make_fraction(numerator: i64, denominator: i64) -> i64 {
    let negative = (numerator < 0) != (denominator < 0);
    let rounded = (numerator.abs() * (1 << 28) + denominator.abs() / 2) / denominator.abs();
    if negative { -rounded } else { rounded }
}

fn metapost_log(mut value: i64) -> i64 {
    const FRACTION_FOUR: i64 = 1 << 30;
    const SPEC_LOG: [i64; 29] = [
        0, 93_032_640, 38_612_034, 17_922_280, 8_662_214, 4_261_238, 2_113_709, 1_052_693, 525_315,
        262_400, 131_136, 65_552, 32_772, 16_385, 8_192, 4_096, 2_048, 1_024, 512, 256, 128, 64,
        32, 16, 8, 4, 2, 1, 1,
    ];
    let mut y = 1_302_456_860_i64;
    let mut z = 6_581_195_i64;
    while value < FRACTION_FOUR {
        value *= 2;
        y -= 93_032_639;
        z -= 48_782;
    }
    y += z / 65_536;
    let mut k = 2_usize;
    while value > FRACTION_FOUR + 4 {
        let mut step = ((value - 1) / (1_i64 << k)) + 1;
        while value < FRACTION_FOUR + step {
            step = (step + 1) / 2;
            k += 1;
        }
        y += SPEC_LOG[k];
        value -= step;
    }
    y / 8
}

/// TeX's job-start clock values.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct JobClock {
    pub time: i32,
    pub second: i32,
    pub day: i32,
    pub month: i32,
    pub year: i32,
}

impl JobClock {
    /// A deterministic clock used by hermetic in-memory worlds.
    pub const DEFAULT: Self = Self {
        time: 0,
        second: 0,
        day: 1,
        month: 1,
        year: 1970,
    };
}

impl Default for JobClock {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Shell-escape execution policy.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ShellEscapePolicy {
    #[default]
    Disabled,
    Enabled,
    Restricted,
}

/// A recorded shell-escape request.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ShellEscapeRecord {
    command: String,
    allowed: bool,
}

impl ShellEscapeRecord {
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    #[must_use]
    pub const fn allowed(&self) -> bool {
        self.allowed
    }
}

/// `World` error with host details erased at the capability boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldError {
    operation: &'static str,
    path: Option<PathBuf>,
    message: String,
    committed_effects_through: Option<EffectPos>,
    retry_safety: EffectRetrySafety,
    stream_open_unavailable: Option<Box<StreamOpenFailure>>,
}

/// Exact append-only occurrence of an unavailable `StreamOpen`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamOpenFailure {
    position: EffectPos,
    slot: StreamSlot,
    path: PathBuf,
    context: String,
}

/// Handle-free failure returned by detached terminal-effect publication.
///
/// The runtime `EffectPos` used by the destination World is translated to a
/// one-based ordinal in the supplied detached suffix before this value crosses
/// the publication boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedEffectPublicationError {
    committed: usize,
    failed_ordinal: Option<u32>,
    slot: Option<StreamSlot>,
    path: Option<PathBuf>,
    error: Box<WorldError>,
}

impl DetachedEffectPublicationError {
    #[must_use]
    pub const fn committed(&self) -> usize {
        self.committed
    }

    #[must_use]
    pub const fn failed_ordinal(&self) -> Option<u32> {
        self.failed_ordinal
    }

    #[must_use]
    pub const fn slot(&self) -> Option<StreamSlot> {
        self.slot
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[must_use]
    pub fn retry_safety(&self) -> EffectRetrySafety {
        self.error.retry_safety()
    }

    #[must_use]
    pub fn world_error(&self) -> &WorldError {
        &self.error
    }
}

impl StreamOpenFailure {
    #[must_use]
    pub const fn position(&self) -> EffectPos {
        self.position
    }

    #[must_use]
    pub const fn slot(&self) -> StreamSlot {
        self.slot
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn context(&self) -> &str {
        &self.context
    }
}

/// Whether an effect commit can be retried after a reported failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EffectRetrySafety {
    NotAnEffectCommit,
    Safe,
    Poisoned,
}

/// Non-semantic execution trace event captured through the host boundary.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ExecutionTraceEvent {
    subsystem: &'static str,
    message: String,
}

impl ExecutionTraceEvent {
    #[must_use]
    pub const fn subsystem(&self) -> &'static str {
        self.subsystem
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl WorldError {
    pub(crate) fn new(
        operation: &'static str,
        path: Option<PathBuf>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            path,
            message: message.into(),
            committed_effects_through: None,
            retry_safety: EffectRetrySafety::NotAnEffectCommit,
            stream_open_unavailable: None,
        }
    }

    pub(crate) fn pdf_object_ids_exhausted() -> Self {
        Self::new(
            "allocate PDF object",
            None,
            "pdfTeX object-number space is exhausted",
        )
    }

    fn effect_commit(mut self, through: EffectPos, retry_safety: EffectRetrySafety) -> Self {
        self.committed_effects_through = Some(through);
        self.retry_safety = retry_safety;
        self
    }

    fn effect_retry(mut self, retry_safety: EffectRetrySafety) -> Self {
        self.retry_safety = retry_safety;
        self
    }

    /// Returns the exact output-open occurrence that failed before mutation.
    #[must_use]
    pub fn stream_open_unavailable(&self) -> Option<&StreamOpenFailure> {
        self.stream_open_unavailable.as_deref()
    }

    #[must_use]
    pub const fn committed_effects_through(&self) -> Option<EffectPos> {
        self.committed_effects_through
    }

    #[must_use]
    pub const fn retry_safety(&self) -> EffectRetrySafety {
        self.retry_safety
    }
}

impl fmt::Display for WorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "{} {}: {}", self.operation, path.display(), self.message)?,
            None => write!(f, "{}: {}", self.operation, self.message)?,
        }
        if let Some(failure) = &self.stream_open_unavailable
            && !failure.context.is_empty()
        {
            f.write_str(&failure.context)?;
        }
        Ok(())
    }
}

impl std::error::Error for WorldError {}

/// Bounded World cursors plus genuinely small scalar/root metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldSnapshot {
    /// Absolute base plus aligned live-column cursors. The World lineage owns
    /// the values; this mark does not retain another effect payload root.
    effect_base: EffectPos,
    page_effect_artifact_cursor: usize,
    effect_len: usize,
    effect_publication_disposition_len: usize,
    next_effect_sequence: u64,
    next_publication_sequence: u64,
    next_effect_publication_identity: u64,
    effect_counter_journal_len: usize,
    next_effect_domain: u64,
    next_effect_output_attempt_identity: u64,
    next_effect_placement_intra_order: u64,
    next_terminal_publication_identity: u64,
    effect_pos: EffectPos,
    stream_bufs: Arc<StreamBufState>,
    rng: RngState,
    pdf_rng: PdfRandomState,
    pdf_time_micros: u64,
    pdf_timer_origin_micros: u64,
    job_clock: JobClock,
    shell_escape_policy: ShellEscapePolicy,
    input_len: usize,
    input_identities: IdentityMark,
    input_dependency_journal_len: usize,
    input_dependency_len: usize,
    shell_escape_len: usize,
    artifact_base: usize,
    artifact_commit_len: usize,
    next_artifact_publication_identity: u64,
    active_artifact_publication_group: Option<ArtifactPublicationGroup>,
    active_terminal_publication: Option<TerminalPublication>,
    commit_mode: WorldCommitMode,
    /// tex.web §54's `open_parens`: a step that printed `(name` and is then
    /// abandoned must take the count back with the print, or §1335 would
    /// close a paren nobody opened.
    file_framing: crate::file_framing::FileFraming,
    /// tex.web §76's `history` and §82's `error_count`.
    ///
    /// These roll back with the effects that carried the reports they count.
    /// A rolled-back step's diagnostic is truncated out of the effect log, so
    /// leaving the tallies behind would count errors no channel ever showed
    /// -- and, since Umber re-runs a step suspended for a missing resource,
    /// would count the same report once per attempt. §82's hundredth-error
    /// transition reads this count, so the two have to move together.
    error_channel: crate::print::ErrorChannel,
    reachable_state_identity: Option<WorldReachableStateIdentity>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WorldReachableStateIdentity {
    effects: crate::state_hash::SemanticSequenceIdentity,
    inputs: crate::state_hash::SemanticSequenceIdentity,
    artifacts: crate::state_hash::SemanticSequenceIdentity,
    scalars: crate::state_hash::SemanticMapIdentity,
}

impl WorldReachableStateIdentity {
    fn new(world: &World) -> Self {
        let mut scalars = crate::state_hash::SemanticMapIdentity::empty(0x776f_726c_645f_7363);
        for (key, value) in world_scalar_identities(world) {
            scalars.replace(key, None, Some(value));
        }
        Self {
            effects: crate::state_hash::SemanticSequenceIdentity::empty(0x776f_726c_645f_6566),
            inputs: crate::state_hash::SemanticSequenceIdentity::empty(0x776f_726c_645f_696e),
            artifacts: crate::state_hash::SemanticSequenceIdentity::empty(0x776f_726c_645f_6172),
            scalars,
        }
    }

    fn root(self) -> u64 {
        crate::state_hash::semantic_scalar_root(0x776f_726c_645f_7274, |hasher| {
            hasher.u64(self.effects.root());
            hasher.u64(self.inputs.root());
            hasher.u64(self.artifacts.root());
            hasher.u64(self.scalars.root());
        })
    }
}

struct StreamBufIdentityGuard<'a> {
    bufs: &'a mut Arc<StreamBufState>,
    identity: Option<&'a mut WorldReachableStateIdentity>,
    old: Option<u64>,
}

impl std::ops::Deref for StreamBufIdentityGuard<'_> {
    type Target = StreamBufState;

    fn deref(&self) -> &Self::Target {
        self.bufs.as_ref()
    }
}

impl std::ops::DerefMut for StreamBufIdentityGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(self.bufs)
    }
}

impl Drop for StreamBufIdentityGuard<'_> {
    fn drop(&mut self) {
        if let (Some(identity), Some(old)) = (&mut self.identity, self.old) {
            let new = stable_hash(self.bufs.as_ref());
            identity.scalars.replace(0, Some(old), Some(new));
        }
    }
}

/// Immutable accepted effect blocks shared by successive revision lineages.
///
/// Each block owns one aligned set of publication columns and a logical
/// prefix length. A fork adds at most one block for the selected source
/// suffix and opens empty destination-local columns; it never concatenates or
/// copies an older accepted prefix. Blocks are output history, not mutable
/// revision generations, and their payload crosses into a detached artifact
/// only through the existing shipout boundary.
#[derive(Debug, Eq, PartialEq)]
struct AcceptedEffectBlock {
    parent: Option<Arc<Self>>,
    effects: Arc<Vec<EffectRecord>>,
    sequences: Arc<Vec<EffectSequence>>,
    publications: Arc<Vec<Option<EffectPublicationId>>>,
    publication_record_ordinals: Arc<Vec<Option<EffectPublicationRecordOrdinal>>>,
    domains: Arc<Vec<EffectDomain>>,
    semantic_record_ordinals: Arc<Vec<EffectSemanticRecordOrdinal>>,
    placement_intra_orders: Arc<Vec<EffectPlacementIntraOrder>>,
    stream_open_contexts: Arc<BTreeMap<EffectPos, String>>,
    len: usize,
    total_len: usize,
}

impl AcceptedEffectBlock {
    fn extend(
        parent: Option<Arc<Self>>,
        source: &World,
        snapshot: &WorldSnapshot,
    ) -> Option<Arc<Self>> {
        let len = snapshot.effect_len;
        if len == 0 {
            return parent;
        }
        let parent_len = parent.as_ref().map_or(0, |block| block.total_len);
        Some(Arc::new(Self {
            parent,
            effects: Arc::clone(&source.effects),
            sequences: Arc::clone(&source.effect_sequences),
            publications: Arc::clone(&source.effect_publications),
            publication_record_ordinals: Arc::clone(&source.effect_publication_record_ordinals),
            domains: Arc::clone(&source.effect_domains),
            semantic_record_ordinals: Arc::clone(&source.effect_semantic_record_ordinals),
            placement_intra_orders: Arc::clone(&source.effect_placement_intra_orders),
            stream_open_contexts: Arc::clone(&source.stream_open_contexts),
            len,
            total_len: parent_len.saturating_add(len),
        }))
    }

    fn append_detached_records(
        &self,
        records: &mut Vec<EffectRecord>,
        contexts: &mut Vec<Option<String>>,
    ) {
        if let Some(parent) = &self.parent {
            parent.append_detached_records(records, contexts);
        }
        let journal = crate::EffectJournal::from_parts(
            self.effects[..self.len].to_vec(),
            self.sequences[..self.len].to_vec(),
            self.publications[..self.len].to_vec(),
            self.publication_record_ordinals[..self.len].to_vec(),
            self.domains[..self.len].to_vec(),
            self.semantic_record_ordinals[..self.len].to_vec(),
            self.placement_intra_orders[..self.len].to_vec(),
        )
        .expect("accepted effect columns remain aligned");
        let base = self.total_len.saturating_sub(self.len);
        for index in journal.materialized_record_indices() {
            let record = self.effects[index].clone();
            let context = matches!(record, EffectRecord::StreamOpen { .. })
                .then(|| {
                    let raw = u64::try_from(base.saturating_add(index).saturating_add(1)).ok()?;
                    self.stream_open_contexts.get(&EffectPos(raw)).cloned()
                })
                .flatten();
            records.push(record);
            contexts.push(context);
        }
    }

    fn publication_counter(&self, key: EffectPublicationId) -> Option<u64> {
        self.publications[..self.len]
            .iter()
            .zip(&self.publication_record_ordinals[..self.len])
            .rev()
            .find_map(|(publication, ordinal)| {
                (*publication == Some(key)).then(|| ordinal.as_ref().map(|ordinal| ordinal.0))?
            })
            .or_else(|| self.parent.as_ref()?.publication_counter(key))
    }

    fn semantic_counter(&self, key: EffectDomain) -> Option<u64> {
        self.domains[..self.len]
            .iter()
            .zip(&self.semantic_record_ordinals[..self.len])
            .rev()
            .find_map(|(domain, ordinal)| {
                let domain = match domain {
                    EffectDomain::World(_) => EffectDomain::World(0),
                    domain => *domain,
                };
                (domain == key).then_some(ordinal.0)
            })
            .or_else(|| self.parent.as_ref()?.semantic_counter(key))
    }

    #[cfg(feature = "profiling")]
    fn retained_payload_bytes(&self) -> usize {
        self.parent
            .as_ref()
            .map_or(0, |parent| parent.retained_payload_bytes())
            .saturating_add(
                self.effects[..self.len]
                    .iter()
                    .map(effect_retained_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(self.len.saturating_mul(
                std::mem::size_of::<EffectSequence>()
                    + std::mem::size_of::<Option<EffectPublicationId>>()
                    + std::mem::size_of::<Option<EffectPublicationRecordOrdinal>>()
                    + std::mem::size_of::<EffectDomain>()
                    + std::mem::size_of::<EffectSemanticRecordOrdinal>()
                    + std::mem::size_of::<EffectPlacementIntraOrder>(),
            ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EffectCounterUndo {
    Publication {
        key: EffectPublicationId,
        previous: Option<u64>,
    },
    Semantic {
        key: EffectDomain,
        previous: Option<u64>,
    },
}

#[derive(Debug, Eq, PartialEq)]
struct AcceptedInputBlock {
    parent: Option<Arc<Self>>,
    records: Arc<Vec<InputRecord>>,
    contents: Arc<BTreeMap<ContentHash, Arc<[u8]>>>,
    len: usize,
    total_len: usize,
}

#[derive(Debug, Eq, PartialEq)]
struct AcceptedInputDependencyBlock {
    parent: Option<Arc<Self>>,
    values: Arc<BTreeMap<Arc<Path>, InputDependency>>,
    journal: Arc<Vec<(Arc<Path>, Option<InputDependency>)>>,
    journal_len: usize,
}

impl AcceptedInputDependencyBlock {
    fn get(&self, path: &Path) -> Option<&InputDependency> {
        let mut value = self.values.get(path);
        for (changed, previous) in self.journal[self.journal_len..].iter().rev() {
            if changed.as_ref() == path {
                value = previous.as_ref();
            }
        }
        value.or_else(|| self.parent.as_ref()?.get(path))
    }

    fn merge_into(&self, merged: &mut BTreeMap<Arc<Path>, InputDependency>) {
        if let Some(parent) = &self.parent {
            parent.merge_into(merged);
        }
        for path in self.values.keys() {
            match self.get(path) {
                Some(value) => {
                    merged.insert(Arc::clone(path), value.clone());
                }
                None => {
                    merged.remove(path.as_ref());
                }
            }
        }
    }
}

impl AcceptedInputBlock {
    fn extend(parent: Option<Arc<Self>>, source: &World, len: usize) -> Option<Arc<Self>> {
        if len == 0 {
            return parent;
        }
        let parent_len = parent.as_ref().map_or(0, |block| block.total_len);
        Some(Arc::new(Self {
            parent,
            records: Arc::clone(&source.inputs),
            contents: Arc::clone(&source.input_contents),
            len,
            total_len: parent_len.saturating_add(len),
        }))
    }

    fn record(&self, index: usize) -> Option<&InputRecord> {
        let parent_len = self.parent.as_ref().map_or(0, |block| block.total_len);
        if index < parent_len {
            return self.parent.as_ref()?.record(index);
        }
        self.records
            .get(index - parent_len)
            .filter(|_| index < self.total_len)
    }

    fn content(&self, hash: ContentHash) -> Option<&[u8]> {
        self.contents
            .get(&hash)
            .map(AsRef::as_ref)
            .or_else(|| self.parent.as_ref()?.content(hash))
    }

    fn content_root(&self, hash: ContentHash) -> Option<Arc<[u8]>> {
        self.contents
            .get(&hash)
            .cloned()
            .or_else(|| self.parent.as_ref()?.content_root(hash))
    }

    #[cfg(feature = "profiling")]
    fn retained_payload_bytes(&self) -> usize {
        self.parent
            .as_ref()
            .map_or(0, |parent| parent.retained_payload_bytes())
            .saturating_add(
                self.records[..self.len]
                    .iter()
                    .map(|record| {
                        std::mem::size_of::<InputRecord>()
                            .saturating_add(record.path.as_os_str().len())
                            .saturating_add(
                                self.contents
                                    .get(&record.hash)
                                    .map_or(0, |bytes| bytes.len()),
                            )
                    })
                    .sum::<usize>(),
            )
    }
}

/// Borrowed logical view over accepted input blocks plus the current suffix.
#[derive(Clone, Copy)]
pub struct InputRecords<'a> {
    world: &'a World,
}

impl<'a> InputRecords<'a> {
    #[must_use]
    pub fn len(self) -> usize {
        self.world.accepted_input_len() + self.world.inputs.len()
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<&'a InputRecord> {
        let accepted = self.world.accepted_input_len();
        if index < accepted {
            return self.world.accepted_inputs.as_ref()?.record(index);
        }
        self.world.inputs.get(index - accepted)
    }

    #[must_use]
    pub fn first(self) -> Option<&'a InputRecord> {
        self.get(0)
    }

    pub fn iter(self) -> InputRecordIter<'a> {
        InputRecordIter {
            records: self,
            index: 0,
        }
    }
}

impl std::ops::Index<usize> for InputRecords<'_> {
    type Output = InputRecord;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index)
            .expect("World input-record index is in range")
    }
}

pub struct InputRecordIter<'a> {
    records: InputRecords<'a>,
    index: usize,
}

impl<'a> Iterator for InputRecordIter<'a> {
    type Item = &'a InputRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let record = self.records.get(self.index)?;
        self.index += 1;
        Some(record)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.records.len().saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for InputRecordIter<'_> {}

/// Engine capability object for all external effects.
#[derive(Debug)]
pub struct World {
    backend: WorldBackend,
    /// Accepted effects detached from publication but still preceding every
    /// page produced by a retained generation fork.
    accepted_effects: Option<Arc<AcceptedEffectBlock>>,
    /// Number of prefix-or-live effects already embedded in a committed page.
    /// This is an in-session page-staging cursor, not detached identity.
    page_effect_artifact_cursor: usize,
    effect_base: EffectPos,
    effects: Arc<Vec<EffectRecord>>,
    effect_sequences: Arc<Vec<EffectSequence>>,
    effect_publications: Arc<Vec<Option<EffectPublicationId>>>,
    effect_publication_record_ordinals: Arc<Vec<Option<EffectPublicationRecordOrdinal>>>,
    effect_domains: Arc<Vec<EffectDomain>>,
    effect_semantic_record_ordinals: Arc<Vec<EffectSemanticRecordOrdinal>>,
    effect_placement_intra_orders: Arc<Vec<EffectPlacementIntraOrder>>,
    effect_publication_dispositions: Arc<Vec<EffectPublicationDisposition>>,
    next_effect_sequence: u64,
    next_publication_sequence: u64,
    next_effect_publication_identity: u64,
    next_effect_publication_record_ordinals: Arc<BTreeMap<EffectPublicationId, u64>>,
    next_effect_domain: u64,
    next_effect_output_attempt_identity: u64,
    next_effect_semantic_record_ordinals: Arc<BTreeMap<EffectDomain, u64>>,
    effect_counter_journal: Arc<Vec<EffectCounterUndo>>,
    next_effect_placement_intra_order: u64,
    active_effect_publication: Option<EffectPublicationId>,
    active_effect_output_attempt: Option<EffectOutputAttemptId>,
    active_effect_domain: Option<EffectDomain>,
    active_terminal_publication: Option<TerminalPublication>,
    next_terminal_publication_identity: u64,
    stream_bufs: Arc<StreamBufState>,
    committed_write_streams: [Option<WriteTarget>; STREAM_SLOT_COUNT],
    committed_output_paths: BTreeSet<PathBuf>,
    rng: RngState,
    pdf_rng: PdfRandomState,
    pdf_time_micros: u64,
    pdf_timer_origin_micros: u64,
    job_clock: JobClock,
    shell_escape_policy: ShellEscapePolicy,
    accepted_inputs: Option<Arc<AcceptedInputBlock>>,
    inputs: Arc<Vec<InputRecord>>,
    input_identities: IdentityAllocator,
    input_contents: Arc<BTreeMap<ContentHash, Arc<[u8]>>>,
    accepted_input_dependencies: Option<Arc<AcceptedInputDependencyBlock>>,
    input_dependencies: Arc<BTreeMap<Arc<Path>, InputDependency>>,
    input_dependency_journal: Arc<Vec<(Arc<Path>, Option<InputDependency>)>>,
    input_dependency_len: usize,
    terminal_inputs: Vec<String>,
    terminal_input_owner: u64,
    shell_escapes: Vec<ShellEscapeRecord>,
    artifact_base: usize,
    artifact_commits: Arc<Vec<ContentHash>>,
    committed_artifacts: Arc<Vec<CommittedArtifact>>,
    artifact_publications: Arc<Vec<ArtifactPublicationRecord>>,
    provisional_page_output_receipts:
        Arc<BTreeMap<PageOutputPublicationReceiptId, Arc<[ArtifactPublicationRecord]>>>,
    next_artifact_publication_identity: u64,
    active_artifact_publication_group: Option<ArtifactPublicationGroup>,
    verified_artifacts: BTreeSet<ContentHash>,
    effect_commit_poison: Option<WorldError>,
    commit_mode: WorldCommitMode,
    error_channel: crate::print::ErrorChannel,
    file_framing: crate::file_framing::FileFraming,
    execution_tracing: bool,
    execution_trace: Vec<ExecutionTraceEvent>,
    unavailable_memory_outputs: BTreeSet<PathBuf>,
    stream_open_contexts: Arc<BTreeMap<EffectPos, String>>,
    reachable_state_identity: Option<WorldReachableStateIdentity>,
}

/// Memory-backed materialization prefix retained across one host retry.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct MemoryMaterializationCheckpoint(Arc<MemoryBackend>);

/// Final semantic disposition of one effect publication commit.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectPublicationDisposition {
    rejected: Option<EffectPublicationId>,
    winner: EffectPublicationId,
    output_attempt: EffectOutputAttemptId,
    recursive_receipt: Option<PageOutputPublicationReceiptId>,
}

/// Semantic publication replaced by one recursive shipout attempt.
///
/// This authority is created by the page-output lifecycle before artifact
/// staging; it is deliberately independent of artifact records.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectPublicationCandidate {
    retained: EffectPublicationId,
}

impl EffectPublicationCandidate {
    #[must_use]
    pub const fn replacing(retained: EffectPublicationId) -> Self {
        Self { retained }
    }

    #[must_use]
    pub const fn retained(self) -> EffectPublicationId {
        self.retained
    }
}

impl EffectPublicationDisposition {
    #[doc(hidden)]
    #[must_use]
    pub const fn new(
        rejected: Option<EffectPublicationId>,
        winner: EffectPublicationId,
        output_attempt: EffectOutputAttemptId,
        recursive_receipt: Option<PageOutputPublicationReceiptId>,
    ) -> Self {
        Self {
            rejected,
            winner,
            output_attempt,
            recursive_receipt,
        }
    }

    #[must_use]
    pub const fn rejected(&self) -> Option<EffectPublicationId> {
        self.rejected
    }

    #[must_use]
    pub const fn winner(&self) -> EffectPublicationId {
        self.winner
    }

    #[must_use]
    pub const fn output_attempt(&self) -> EffectOutputAttemptId {
        self.output_attempt
    }

    /// Receipt inherited by recursive page-output transactions. Unlike an
    /// encounter-order position, this identity remains rooted at the retained
    /// episode while descendant episodes publish under it.
    #[must_use]
    pub const fn recursive_receipt(&self) -> Option<PageOutputPublicationReceiptId> {
        self.recursive_receipt
    }
}

/// One committed page-output publication.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageOutputPublicationReceipt {
    _committed: (),
    effect: EffectPublicationId,
    artifacts: Arc<[ArtifactPublicationRecord]>,
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EffectPublicationId(u64);

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EffectOutputAttemptId(u64);

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactPublicationId(u64);

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PageOutputPublicationReceiptId(u64);

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TerminalPublicationId(u64);

/// Raw semantic ordering sidecar aligned one-for-one with committed artifacts.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactPublicationRecord {
    pub publication: ArtifactPublicationId,
    pub receipt: PageOutputPublicationReceiptId,
    pub effect_publication: Option<EffectPublicationId>,
    pub sequence: EffectSequence,
    pub domain: EffectDomain,
    pub intra_order: u32,
}

/// Opaque single-use authority to publish one artifact record.
#[doc(hidden)]
#[derive(Debug)]
pub struct ArtifactPublicationReservation {
    record: ArtifactPublicationRecord,
    provisional_receipt: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ArtifactPublicationGroup {
    sequence: EffectSequence,
    domain: EffectDomain,
    next_intra_order: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalPublication {
    identity: TerminalPublicationId,
    sequence: EffectSequence,
    phase: TerminalPublicationPhase,
    start: usize,
    next_intra_order: u32,
}

impl ArtifactPublicationReservation {
    #[must_use]
    pub const fn record(&self) -> ArtifactPublicationRecord {
        self.record
    }
}

impl ArtifactPublicationRecord {
    #[must_use]
    pub const fn new(
        publication: ArtifactPublicationId,
        receipt: PageOutputPublicationReceiptId,
        effect_publication: Option<EffectPublicationId>,
        sequence: EffectSequence,
        domain: EffectDomain,
        intra_order: u32,
    ) -> Self {
        Self {
            publication,
            receipt,
            effect_publication,
            sequence,
            domain,
            intra_order,
        }
    }
    #[must_use]
    pub const fn publication(self) -> ArtifactPublicationId {
        self.publication
    }

    #[must_use]
    pub const fn receipt(self) -> PageOutputPublicationReceiptId {
        self.receipt
    }

    #[must_use]
    pub const fn effect_publication(self) -> Option<EffectPublicationId> {
        self.effect_publication
    }

    #[must_use]
    pub const fn with_effect_publication(mut self, publication: EffectPublicationId) -> Self {
        self.effect_publication = Some(publication);
        self
    }

    #[must_use]
    pub const fn sequence(self) -> EffectSequence {
        self.sequence
    }

    #[must_use]
    pub const fn domain(self) -> EffectDomain {
        self.domain
    }

    #[must_use]
    pub const fn intra_order(self) -> u32 {
        self.intra_order
    }
}

/// Stable semantic position of an effect across retained-generation forks.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EffectSequence(u64);

/// Stable record position allocated independently within one restart domain.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EffectSemanticRecordOrdinal(u64);

/// Stable record position allocated independently within one effect publication.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EffectPublicationRecordOrdinal(u64);

/// Stable tie-break position within a mapped semantic correspondence.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EffectPlacementIntraOrder(u64);

/// Stable semantic producer domain for revision reconciliation.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EffectDomain {
    TerminalPublication {
        identity: TerminalPublicationId,
        phase: TerminalPublicationPhase,
        intra_order: u32,
        committed: bool,
    },
    PublicationBoundary {
        left: Option<EffectPublicationId>,
        right: Option<EffectPublicationId>,
        /// The output attempt that claimed this gap. Endpoints can map to the
        /// same retained publications for several recursive attempts, while
        /// only one attempt owns the canonical publication.
        output_attempt: EffectOutputAttemptId,
    },
    World(u64),
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TerminalPublicationPhase {
    CloseOpenParens,
    Notices,
    PdfFinalizationNotices,
    PdfFatal,
}

impl EffectSequence {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

impl EffectSemanticRecordOrdinal {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

impl EffectPublicationRecordOrdinal {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

impl EffectPlacementIntraOrder {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

impl EffectPublicationId {
    #[must_use]
    pub const fn new(identity: u64) -> Self {
        Self(identity)
    }
}

impl EffectOutputAttemptId {
    #[must_use]
    pub const fn new(identity: u64) -> Self {
        Self(identity)
    }
}

impl ArtifactPublicationId {
    #[must_use]
    pub const fn new(identity: u64) -> Self {
        Self(identity)
    }
}

impl PageOutputPublicationReceiptId {
    #[must_use]
    pub const fn new(identity: u64) -> Self {
        Self(identity)
    }

    #[must_use]
    pub const fn identity(self) -> u64 {
        self.0
    }
}

impl TerminalPublicationId {
    #[must_use]
    pub const fn new(identity: u64) -> Self {
        Self(identity)
    }
}

impl PageOutputPublicationReceipt {
    #[doc(hidden)]
    #[must_use]
    pub fn committed(effect: EffectPublicationId, artifact: ArtifactPublicationRecord) -> Self {
        Self {
            _committed: (),
            effect,
            artifacts: Arc::from([artifact]),
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn committed_group(
        effect: EffectPublicationId,
        artifacts: Arc<[ArtifactPublicationRecord]>,
    ) -> Option<Self> {
        (!artifacts.is_empty()).then_some(Self {
            _committed: (),
            effect,
            artifacts,
        })
    }

    #[must_use]
    pub const fn effect(&self) -> EffectPublicationId {
        self.effect
    }

    #[must_use]
    pub fn artifacts(&self) -> &[ArtifactPublicationRecord] {
        &self.artifacts
    }

    #[must_use]
    pub fn receipt(&self) -> PageOutputPublicationReceiptId {
        self.artifacts
            .first()
            .expect("committed publication receipt is nonempty")
            .receipt()
    }

    #[doc(hidden)]
    pub fn extend(&mut self, other: &Self) {
        debug_assert_eq!(self.receipt(), other.receipt());
        let mut artifacts = self.artifacts.to_vec();
        artifacts.extend_from_slice(&other.artifacts);
        artifacts.sort_by_key(|record| (record.intra_order(), record.publication()));
        artifacts.dedup_by_key(|record| record.publication());
        self.artifacts = Arc::from(artifacts);
    }
}

impl Clone for World {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            accepted_effects: self.accepted_effects.clone(),
            page_effect_artifact_cursor: self.page_effect_artifact_cursor,
            effect_base: self.effect_base,
            effects: self.effects.clone(),
            effect_sequences: self.effect_sequences.clone(),
            effect_publications: self.effect_publications.clone(),
            effect_publication_record_ordinals: self.effect_publication_record_ordinals.clone(),
            effect_domains: self.effect_domains.clone(),
            effect_semantic_record_ordinals: self.effect_semantic_record_ordinals.clone(),
            effect_placement_intra_orders: self.effect_placement_intra_orders.clone(),
            effect_publication_dispositions: self.effect_publication_dispositions.clone(),
            next_effect_sequence: self.next_effect_sequence,
            next_publication_sequence: self.next_publication_sequence,
            next_effect_publication_identity: self.next_effect_publication_identity,
            next_effect_publication_record_ordinals: self
                .next_effect_publication_record_ordinals
                .clone(),
            next_effect_domain: self.next_effect_domain,
            next_effect_output_attempt_identity: self.next_effect_output_attempt_identity,
            next_effect_semantic_record_ordinals: self.next_effect_semantic_record_ordinals.clone(),
            effect_counter_journal: self.effect_counter_journal.clone(),
            next_effect_placement_intra_order: self.next_effect_placement_intra_order,
            active_effect_publication: self.active_effect_publication,
            active_effect_output_attempt: self.active_effect_output_attempt,
            active_effect_domain: self.active_effect_domain,
            next_terminal_publication_identity: self.next_terminal_publication_identity,
            stream_bufs: self.stream_bufs.clone(),
            committed_write_streams: self.committed_write_streams.clone(),
            committed_output_paths: self.committed_output_paths.clone(),
            rng: self.rng,
            pdf_rng: self.pdf_rng.clone(),
            pdf_time_micros: self.pdf_time_micros,
            pdf_timer_origin_micros: self.pdf_timer_origin_micros,
            job_clock: self.job_clock,
            shell_escape_policy: self.shell_escape_policy,
            accepted_inputs: self.accepted_inputs.clone(),
            inputs: self.inputs.clone(),
            input_identities: self.input_identities.fork(),
            input_contents: self.input_contents.clone(),
            accepted_input_dependencies: self.accepted_input_dependencies.clone(),
            input_dependencies: self.input_dependencies.clone(),
            input_dependency_journal: self.input_dependency_journal.clone(),
            input_dependency_len: self.input_dependency_len,
            terminal_inputs: self.terminal_inputs.clone(),
            terminal_input_owner: fresh_terminal_input_owner(),
            shell_escapes: self.shell_escapes.clone(),
            artifact_base: self.artifact_base,
            artifact_commits: self.artifact_commits.clone(),
            committed_artifacts: self.committed_artifacts.clone(),
            artifact_publications: self.artifact_publications.clone(),
            provisional_page_output_receipts: self.provisional_page_output_receipts.clone(),
            next_artifact_publication_identity: self.next_artifact_publication_identity,
            active_artifact_publication_group: self.active_artifact_publication_group,
            active_terminal_publication: self.active_terminal_publication,
            verified_artifacts: self.verified_artifacts.clone(),
            effect_commit_poison: self.effect_commit_poison.clone(),
            commit_mode: self.commit_mode,
            error_channel: self.error_channel.clone(),
            file_framing: self.file_framing,
            execution_tracing: self.execution_tracing,
            execution_trace: self.execution_trace.clone(),
            unavailable_memory_outputs: self.unavailable_memory_outputs.clone(),
            stream_open_contexts: self.stream_open_contexts.clone(),
            reachable_state_identity: self.reachable_state_identity,
        }
    }
}

impl PartialEq for World {
    fn eq(&self, other: &Self) -> bool {
        self.backend == other.backend
            && self.accepted_effects == other.accepted_effects
            && self.page_effect_artifact_cursor == other.page_effect_artifact_cursor
            && self.effect_base == other.effect_base
            && self.effects == other.effects
            && self.stream_bufs == other.stream_bufs
            && self.committed_write_streams == other.committed_write_streams
            && self.committed_output_paths == other.committed_output_paths
            && self.rng == other.rng
            && self.pdf_rng == other.pdf_rng
            && self.pdf_time_micros == other.pdf_time_micros
            && self.pdf_timer_origin_micros == other.pdf_timer_origin_micros
            && self.job_clock == other.job_clock
            && self.shell_escape_policy == other.shell_escape_policy
            && self.accepted_inputs == other.accepted_inputs
            && self.inputs == other.inputs
            && self.input_contents == other.input_contents
            && self.input_dependency_values() == other.input_dependency_values()
            && self.terminal_inputs == other.terminal_inputs
            && self.unavailable_memory_outputs == other.unavailable_memory_outputs
            && self.stream_open_contexts == other.stream_open_contexts
            && self.shell_escapes == other.shell_escapes
            && self.artifact_base == other.artifact_base
            && self.artifact_commits == other.artifact_commits
            && self.committed_artifacts == other.committed_artifacts
            && self.artifact_publications == other.artifact_publications
            && self.provisional_page_output_receipts == other.provisional_page_output_receipts
            && self.effect_commit_poison == other.effect_commit_poison
            && self.commit_mode == other.commit_mode
    }
}

impl Eq for World {}

impl World {
    /// Starts a process-local profiling timer through the `World` clock boundary.
    #[cfg(feature = "profiling")]
    #[must_use]
    pub fn start_profiling_timer() -> ProfilingTimer {
        ProfilingTimer(Instant::now())
    }

    /// Publishes one deterministic artifact for the aggregate-checkpoint
    /// profiling fixture without exposing artifact-ledger internals.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_publish_artifact(&mut self, bytes: Vec<u8>) {
        let artifact = VerifiedArtifact::new(bytes);
        self.store_verified_artifact(&artifact)
            .expect("profiling artifact stores in the memory backend");
        let reservation = self.reserve_artifact_publication_at(0);
        let hash = artifact.hash();
        let (bytes, provenance, occurrences) = artifact.into_parts();
        self.record_artifact_commit(hash, bytes, provenance, occurrences, reservation);
    }

    /// Opaque World mark capture used by the standalone checkpoint gate.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    #[must_use]
    pub fn profile_checkpoint_capture(&self) -> WorldSnapshot {
        self.snapshot()
    }

    /// Same-lineage World restore used by the standalone checkpoint gate.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_checkpoint_restore(&mut self, snapshot: &WorldSnapshot) {
        self.rollback(snapshot);
    }

    /// Candidate-lineage World fork used by the standalone checkpoint gate.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    #[must_use]
    pub fn profile_checkpoint_fork(&self, snapshot: &WorldSnapshot) -> Self {
        self.fork_checkpoint(snapshot)
    }

    /// Logical payload charged once to this World's accepted/current lineage
    /// and committed-artifact owner by the standalone checkpoint gate.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_retained_checkpoint_bytes(&self) -> usize {
        let effects = self
            .accepted_effects
            .as_ref()
            .map_or(0, |block| block.retained_payload_bytes())
            .saturating_add(
                self.effects
                    .iter()
                    .map(effect_retained_bytes)
                    .sum::<usize>(),
            );
        let inputs = self
            .accepted_inputs
            .as_ref()
            .map_or(0, |block| block.retained_payload_bytes())
            .saturating_add(
                self.inputs
                    .iter()
                    .map(|record| {
                        std::mem::size_of::<InputRecord>()
                            .saturating_add(record.path.as_os_str().len())
                            .saturating_add(
                                self.input_contents
                                    .get(&record.hash)
                                    .map_or(0, |bytes| bytes.len()),
                            )
                    })
                    .sum::<usize>(),
            );
        effects.saturating_add(inputs).saturating_add(
            self.committed_artifacts
                .iter()
                .map(|artifact| artifact.bytes().len())
                .sum::<usize>(),
        )
    }

    /// Constant-time carrier charge for the coarse World lineage retained by
    /// an aggregate checkpoint. Variable effect/artifact bytes remain owned
    /// by their separately accounted immutable output blocks; this charge
    /// covers every live World row and index carrier without walking payload.
    pub(crate) fn checkpoint_retained_bytes(&self) -> usize {
        let accepted_effects = self
            .accepted_effects
            .as_ref()
            .map_or(0, |block| block.total_len);
        let accepted_inputs = self
            .accepted_inputs
            .as_ref()
            .map_or(0, |block| block.total_len);
        let effect_rows = accepted_effects.saturating_add(self.effects.len());
        let input_rows = accepted_inputs.saturating_add(self.inputs.len());
        std::mem::size_of::<Self>()
            .saturating_add(effect_rows.saturating_mul(
                std::mem::size_of::<EffectRecord>()
                    + std::mem::size_of::<EffectSequence>()
                    + std::mem::size_of::<Option<EffectPublicationId>>()
                    + std::mem::size_of::<EffectDomain>()
                    + std::mem::size_of::<EffectSemanticRecordOrdinal>()
                    + std::mem::size_of::<EffectPlacementIntraOrder>(),
            ))
            .saturating_add(input_rows.saturating_mul(std::mem::size_of::<InputRecord>()))
            .saturating_add(
                self.committed_artifacts
                    .len()
                    .saturating_mul(std::mem::size_of::<CommittedArtifact>()),
            )
            .saturating_add(
                self.input_dependencies
                    .len()
                    .saturating_mul(std::mem::size_of::<(Arc<Path>, InputDependency)>()),
            )
    }

    /// Enables rollback-capable ownership for the standalone checkpoint gate.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_begin_retained_session(&mut self) {
        self.begin_retained_session()
            .expect("profiling World is rollback-capable");
    }
    /// Creates a deterministic in-memory world for tests and hermetic runs.
    #[must_use]
    pub fn memory() -> Self {
        Self::memory_with_clock(JobClock::DEFAULT)
    }

    /// Creates a deterministic in-memory world with an explicit job clock.
    #[must_use]
    pub fn memory_with_clock(job_clock: JobClock) -> Self {
        Self::memory_with_pdftex_inputs(job_clock, 0, 0, ShellEscapePolicy::Disabled)
    }

    /// Creates a hermetic world with all pdfTeX session inputs supplied explicitly.
    #[must_use]
    pub fn memory_with_pdftex_inputs(
        job_clock: JobClock,
        random_seed: i32,
        monotonic_micros: u64,
        shell_escape_policy: ShellEscapePolicy,
    ) -> Self {
        Self::new(
            WorldBackend::Memory(Arc::new(MemoryBackend::default())),
            job_clock,
            random_seed,
            monotonic_micros,
            shell_escape_policy,
        )
    }

    /// Creates a real host-backed world and reads the job clock once.
    #[must_use]
    pub fn real() -> Self {
        Self::real_with_artifact_dir(".umber/artifacts")
    }

    /// Creates a real host-backed world with an explicit page artifact store.
    #[must_use]
    pub fn real_with_artifact_dir(artifact_dir: impl Into<PathBuf>) -> Self {
        let job_clock = real_job_clock();
        let monotonic_micros = system_time_micros();
        let random_seed = ((monotonic_micros % 1_000_000) * 1_000
            + (monotonic_micros / 1_000_000) % 1_000_000) as i32;
        Self::new(
            WorldBackend::Real {
                artifact_dir: artifact_dir.into(),
            },
            job_clock,
            random_seed,
            monotonic_micros,
            ShellEscapePolicy::Disabled,
        )
    }

    fn new(
        backend: WorldBackend,
        job_clock: JobClock,
        random_seed: i32,
        monotonic_micros: u64,
        shell_escape_policy: ShellEscapePolicy,
    ) -> Self {
        Self {
            backend,
            accepted_effects: None,
            page_effect_artifact_cursor: 0,
            effect_base: EffectPos::default(),
            effects: Arc::new(Vec::new()),
            effect_sequences: Arc::new(Vec::new()),
            effect_publications: Arc::new(Vec::new()),
            effect_publication_record_ordinals: Arc::new(Vec::new()),
            effect_domains: Arc::new(Vec::new()),
            effect_semantic_record_ordinals: Arc::new(Vec::new()),
            effect_placement_intra_orders: Arc::new(Vec::new()),
            effect_publication_dispositions: Arc::new(Vec::new()),
            next_effect_sequence: 0,
            next_publication_sequence: 0,
            next_effect_publication_identity: 0,
            next_effect_publication_record_ordinals: Arc::new(BTreeMap::new()),
            next_effect_domain: 0,
            next_effect_output_attempt_identity: 0,
            next_effect_semantic_record_ordinals: Arc::new(BTreeMap::new()),
            effect_counter_journal: Arc::new(Vec::new()),
            next_effect_placement_intra_order: 0,
            active_effect_publication: None,
            active_effect_output_attempt: None,
            active_effect_domain: None,
            next_terminal_publication_identity: 0,
            stream_bufs: Arc::new(StreamBufState::default()),
            committed_write_streams: Default::default(),
            committed_output_paths: BTreeSet::new(),
            rng: RngState::default(),
            pdf_rng: PdfRandomState::from_seed(random_seed),
            pdf_time_micros: monotonic_micros,
            pdf_timer_origin_micros: monotonic_micros,
            job_clock,
            shell_escape_policy,
            accepted_inputs: None,
            inputs: Arc::new(Vec::new()),
            input_identities: IdentityAllocator::new(0),
            input_contents: Arc::new(BTreeMap::new()),
            accepted_input_dependencies: None,
            input_dependencies: Arc::new(BTreeMap::new()),
            input_dependency_journal: Arc::new(Vec::new()),
            input_dependency_len: 0,
            terminal_inputs: Vec::new(),
            terminal_input_owner: fresh_terminal_input_owner(),
            shell_escapes: Vec::new(),
            artifact_base: 0,
            artifact_commits: Arc::new(Vec::new()),
            committed_artifacts: Arc::new(Vec::new()),
            artifact_publications: Arc::new(Vec::new()),
            provisional_page_output_receipts: Arc::new(BTreeMap::new()),
            next_artifact_publication_identity: 0,
            active_artifact_publication_group: None,
            active_terminal_publication: None,
            verified_artifacts: BTreeSet::new(),
            effect_commit_poison: None,
            commit_mode: WorldCommitMode::Eager,
            error_channel: crate::print::ErrorChannel::default(),
            file_framing: crate::file_framing::FileFraming::default(),
            execution_tracing: false,
            execution_trace: Vec::new(),
            unavailable_memory_outputs: BTreeSet::new(),
            stream_open_contexts: Arc::new(BTreeMap::new()),
            reachable_state_identity: None,
        }
    }

    /// tex.web §76's error-channel state: `error_count` and §1281's
    /// `long_help_seen`, which persist across recoverable errors.
    pub const fn error_channel_mut(&mut self) -> &mut crate::print::ErrorChannel {
        &mut self.error_channel
    }

    /// Read access to the same state.
    #[must_use]
    pub const fn error_channel(&self) -> &crate::print::ErrorChannel {
        &self.error_channel
    }

    /// tex.web §54's `open_parens`, which §537, §362, and §1335 maintain
    /// between them; see [`crate::file_framing`] for why it is print-adjacent
    /// state here rather than driver state.
    pub const fn file_framing_mut(&mut self) -> &mut crate::file_framing::FileFraming {
        &mut self.file_framing
    }

    /// Read access to the same state.
    #[must_use]
    pub const fn file_framing(&self) -> &crate::file_framing::FileFraming {
        &self.file_framing
    }

    /// Enables or disables non-semantic execution tracing.
    pub fn set_execution_tracing(&mut self, enabled: bool) {
        self.execution_tracing = enabled;
    }

    #[must_use]
    pub const fn execution_tracing_enabled(&self) -> bool {
        self.execution_tracing
    }

    pub fn trace_execution(&mut self, subsystem: &'static str, message: impl Into<String>) {
        if self.execution_tracing {
            self.execution_trace.push(ExecutionTraceEvent {
                subsystem,
                message: message.into(),
            });
        }
    }

    #[must_use]
    pub fn execution_trace(&self) -> &[ExecutionTraceEvent] {
        &self.execution_trace
    }

    /// Adds or replaces one file in an in-memory world.
    pub fn set_memory_file(
        &mut self,
        path: impl Into<PathBuf>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<(), WorldError> {
        self.set_shared_memory_file(path, Arc::from(bytes.into()))
    }

    /// Adds or replaces one already-shared immutable file in an in-memory world.
    pub fn set_shared_memory_file(
        &mut self,
        path: impl Into<PathBuf>,
        bytes: Arc<[u8]>,
    ) -> Result<(), WorldError> {
        let path = path.into();
        let WorldBackend::Memory(memory) = &mut self.backend else {
            return Err(WorldError::new(
                "set memory file",
                None,
                "world is not memory-backed",
            ));
        };
        Arc::make_mut(memory).files.insert(path.clone(), bytes);
        Ok(())
    }

    /// Attaches deterministic modification metadata to a seeded memory file.
    pub fn set_memory_file_modification_date(
        &mut self,
        path: impl Into<PathBuf>,
        date: FileModificationDate,
    ) -> Result<(), WorldError> {
        let WorldBackend::Memory(memory) = &mut self.backend else {
            return Err(WorldError::new(
                "set memory file modification date",
                None,
                "world is not memory-backed",
            ));
        };
        Arc::make_mut(memory)
            .modification_dates
            .insert(path.into(), date);
        Ok(())
    }

    /// Adds one terminal input line to an in-memory world.
    ///
    /// The line should not include its trailing newline; real terminal reads
    /// return the same normalized physical-line shape.
    pub fn push_memory_terminal_line(&mut self, line: impl Into<String>) -> Result<(), WorldError> {
        if !matches!(self.backend, WorldBackend::Memory(_)) {
            return Err(WorldError::new(
                "set terminal input",
                None,
                "world is not memory-backed",
            ));
        };
        self.terminal_inputs.push(line.into());
        Ok(())
    }

    /// Reads a file as bytes, records the hash, and returns both together.
    pub fn read_file(&mut self, path: impl AsRef<Path>) -> Result<FileContent, WorldError> {
        let path = path.as_ref();
        let (bytes, modification_date, origin): (Arc<[u8]>, _, _) =
            match self.pending_output_bytes(path)? {
                Some(bytes) => (
                    Arc::from(bytes),
                    Some(FileModificationDate::utc(self.job_clock)),
                    InputOrigin::SameRunGenerated,
                ),
                None => {
                    let origin = if self.committed_output_paths.contains(path) {
                        InputOrigin::SameRunGenerated
                    } else {
                        InputOrigin::External
                    };
                    (
                        self.materialized_file_bytes(path)?,
                        self.materialized_file_modification_date(path),
                        origin,
                    )
                }
            };
        Ok(self.register_input_content(path, bytes, modification_date, origin))
    }

    /// Reads bytes generated but not yet committed by this TeX run.
    ///
    /// This narrow lookup lets a driver resolver preserve pending-output
    /// precedence without exposing unrelated materialized host or VFS files.
    pub(crate) fn read_pending_output_file(
        &mut self,
        path: &Path,
    ) -> Result<Option<FileContent>, WorldError> {
        let Some(bytes) = self.pending_output_bytes(path)? else {
            return Ok(None);
        };
        Ok(Some(self.register_input_content(
            path,
            Arc::from(bytes),
            Some(FileModificationDate::utc(self.job_clock)),
            InputOrigin::SameRunGenerated,
        )))
    }

    /// Reads an exact path only when this run generated it.
    ///
    /// Unlike [`Self::read_file`], a miss neither consults ordinary host
    /// inputs nor allocates an input record. This lets retained sessions give
    /// their own committed outputs canonical precedence without bypassing
    /// driver-owned search policy for external files.
    pub fn read_same_run_output_file(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Option<FileContent>, WorldError> {
        let path = path.as_ref();
        if let Some(content) = self.read_pending_output_file(path)? {
            return Ok(Some(content));
        }
        if !self.committed_output_paths.contains(path) {
            return Ok(None);
        }
        let bytes = self.materialized_file_bytes(path)?;
        Ok(Some(self.register_input_content(
            path,
            bytes,
            Some(FileModificationDate::utc(self.job_clock)),
            InputOrigin::SameRunGenerated,
        )))
    }

    /// Registers immutable bytes supplied by a driver-owned resolver as one
    /// successful input read.
    ///
    /// A pending TeX output at the same path still takes precedence. This
    /// preserves TeX's ability to close and reopen a file within one run while
    /// keeping host search and storage policy outside [`World`].
    pub(crate) fn read_supplied_file(
        &mut self,
        path: &Path,
        supplied: Arc<[u8]>,
    ) -> Result<FileContent, WorldError> {
        let pending = self.pending_output_bytes(path)?;
        if let WorldBackend::Memory(memory) = &mut self.backend {
            Arc::make_mut(memory)
                .files
                .insert(path.to_owned(), Arc::clone(&supplied));
        }
        let (bytes, modification_date, origin) = match pending {
            Some(bytes) => (
                Arc::from(bytes),
                Some(FileModificationDate::utc(self.job_clock)),
                InputOrigin::SameRunGenerated,
            ),
            None => (
                supplied,
                self.materialized_file_modification_date(path),
                InputOrigin::External,
            ),
        };
        Ok(self.register_input_content(path, bytes, modification_date, origin))
    }

    fn register_input_content(
        &mut self,
        path: &Path,
        bytes: Arc<[u8]>,
        modification_date: Option<FileModificationDate>,
        origin: InputOrigin,
    ) -> FileContent {
        let record = self.allocate_input_record();
        let content =
            FileContent::from_shared(record, path.to_owned(), bytes, modification_date, origin);
        Arc::make_mut(&mut self.input_contents)
            .entry(content.hash)
            .or_insert_with(|| content.bytes.clone());
        Arc::make_mut(&mut self.inputs).push(InputRecord {
            path: content.path.clone(),
            hash: content.hash,
            len: content.bytes.len(),
            modification_date: content.modification_date,
            origin: content.origin,
        });
        self.record_input_identity();
        content
    }

    /// Replays the uncommitted stream suffix for one path without publishing
    /// it to the host. TeX may close an immediate output and read it again in
    /// the same job (LaTeX does this with its main aux file), while retained
    /// sessions must still keep speculative writes rollback-safe.
    fn pending_output_bytes(&self, path: &Path) -> Result<Option<Vec<u8>>, WorldError> {
        let mut active = self.committed_write_streams.clone();
        let mut bytes = None;

        for effect in self.effects.iter() {
            match effect {
                EffectRecord::StreamOpen { slot, target } => {
                    active[slot.index()] = Some(target.clone());
                    if target.path() == path {
                        bytes = Some(Vec::new());
                    }
                }
                EffectRecord::StreamClose { slot } => active[slot.index()] = None,
                EffectRecord::StreamWrite {
                    sink: PrintSink::Stream(slot),
                    text,
                } if active[slot.index()]
                    .as_ref()
                    .is_some_and(|target| target.path() == path) =>
                {
                    if bytes.is_none() {
                        bytes = Some(self.materialized_file_bytes(path)?.to_vec());
                    }
                    bytes
                        .as_mut()
                        .expect("pending output bytes were initialized")
                        .extend_from_slice(text.as_bytes());
                }
                EffectRecord::StreamWriteBytes {
                    sink: PrintSink::Stream(slot),
                    bytes: encoded,
                } if active[slot.index()]
                    .as_ref()
                    .is_some_and(|target| target.path() == path) =>
                {
                    if bytes.is_none() {
                        bytes = Some(self.materialized_file_bytes(path)?.to_vec());
                    }
                    bytes
                        .as_mut()
                        .expect("pending output bytes were initialized")
                        .extend_from_slice(encoded);
                }
                EffectRecord::StreamWrite { .. }
                | EffectRecord::StreamWriteBytes { .. }
                | EffectRecord::DeferredWrite { .. }
                | EffectRecord::Special { .. }
                | EffectRecord::PdfObjectPlaceholder { .. }
                | EffectRecord::ShellEscape(_) => {}
            }
        }
        Ok(bytes)
    }

    fn materialized_file_bytes(&self, path: &Path) -> Result<Arc<[u8]>, WorldError> {
        match &self.backend {
            WorldBackend::Real { .. } => Ok(Arc::from(std::fs::read(path).map_err(|err| {
                WorldError::new("read file", Some(path.to_owned()), err.to_string())
            })?)),
            WorldBackend::Memory(memory) => memory
                .outputs
                .get(path)
                .map(|bytes| Arc::from(bytes.as_slice()))
                .or_else(|| memory.files.get(path).cloned())
                .ok_or_else(|| {
                    WorldError::new(
                        "read file",
                        Some(path.to_owned()),
                        "not found in memory world",
                    )
                }),
        }
    }

    fn materialized_file_modification_date(&self, path: &Path) -> Option<FileModificationDate> {
        match &self.backend {
            WorldBackend::Real { .. } => {
                use chrono::{Datelike as _, Offset as _, Timelike as _};

                let modified = std::fs::metadata(path).ok()?.modified().ok()?;
                let local: chrono::DateTime<chrono::Local> = modified.into();
                Some(FileModificationDate::with_offset(
                    JobClock {
                        time: i32::try_from(local.hour() * 60 + local.minute()).ok()?,
                        second: i32::try_from(local.second()).ok()?,
                        day: i32::try_from(local.day()).ok()?,
                        month: i32::try_from(local.month()).ok()?,
                        year: local.year(),
                    },
                    i16::try_from(local.offset().fix().local_minus_utc() / 60).ok()?,
                ))
            }
            WorldBackend::Memory(memory) => memory.modification_dates.get(path).copied(),
        }
    }

    /// Writes a complete host file through the world I/O boundary.
    pub fn write_file(
        &mut self,
        path: impl AsRef<Path>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<(), WorldError> {
        let path = path.as_ref();
        match &mut self.backend {
            WorldBackend::Real { .. } => std::fs::write(path, bytes).map_err(|err| {
                WorldError::new("write file", Some(path.to_owned()), err.to_string())
            }),
            WorldBackend::Memory(memory) => {
                Arc::make_mut(memory)
                    .files
                    .insert(path.to_owned(), Arc::from(bytes.as_ref()));
                Ok(())
            }
        }
    }

    /// Stages a set of complete downstream files before publishing any of them.
    ///
    /// Real files are written to unique siblings before any destination is
    /// changed. Existing destinations are moved to rollback siblings, and a
    /// failed publish restores the entire prior set. Readers never observe
    /// truncated contents. Memory worlds publish the complete set in one
    /// mutation pass.
    pub fn publish_files(&mut self, files: Vec<(PathBuf, Vec<u8>)>) -> Result<(), WorldError> {
        static NEXT_TEMP_OUTPUT: AtomicU64 = AtomicU64::new(0);
        match &mut self.backend {
            WorldBackend::Real { .. } => {
                let mut staged: Vec<StagedPublication> = Vec::with_capacity(files.len());
                for (path, bytes) in files {
                    let parent = path
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty());
                    if let Some(parent) = parent {
                        std::fs::create_dir_all(parent).map_err(|error| {
                            WorldError::new(
                                "create output directory",
                                Some(parent.to_owned()),
                                error.to_string(),
                            )
                        })?;
                    }
                    let file_name = path.file_name().ok_or_else(|| {
                        WorldError::new(
                            "stage file",
                            Some(path.clone()),
                            "output path has no file name",
                        )
                    })?;
                    let nonce = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
                    let temporary = path.with_file_name(format!(
                        ".{}.{}.{}.tmp",
                        file_name.to_string_lossy(),
                        std::process::id(),
                        nonce
                    ));
                    let result = (|| {
                        let mut file = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&temporary)?;
                        file.write_all(&bytes)
                    })();
                    if let Err(error) = result {
                        let _ = std::fs::remove_file(&temporary);
                        cleanup_staged_publication(&staged);
                        return Err(WorldError::new("stage file", Some(path), error.to_string()));
                    }
                    staged.push((path, temporary, None));
                }

                for index in 0..staged.len() {
                    let path = staged[index].0.clone();
                    match std::fs::symlink_metadata(&path) {
                        Ok(metadata)
                            if metadata.file_type().is_file()
                                || metadata.file_type().is_symlink() =>
                        {
                            let file_name = path.file_name().expect("staged path has a file name");
                            let nonce = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
                            let rollback = path.with_file_name(format!(
                                ".{}.{}.{}.rollback",
                                file_name.to_string_lossy(),
                                std::process::id(),
                                nonce
                            ));
                            if let Err(error) = std::fs::rename(&path, &rollback) {
                                rollback_staged_publication(&staged, 0);
                                return Err(WorldError::new(
                                    "prepare file publication",
                                    Some(path),
                                    error.to_string(),
                                ));
                            }
                            staged[index].2 = Some(rollback);
                        }
                        Ok(_) => {
                            rollback_staged_publication(&staged, 0);
                            return Err(WorldError::new(
                                "publish file",
                                Some(path),
                                "output path exists but is not a regular file",
                            ));
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => {
                            rollback_staged_publication(&staged, 0);
                            return Err(WorldError::new(
                                "prepare file publication",
                                Some(path),
                                error.to_string(),
                            ));
                        }
                    }
                }

                for (published, (path, temporary, _)) in staged.iter().enumerate() {
                    let result = std::fs::rename(temporary, path);
                    if let Err(error) = result {
                        rollback_staged_publication(&staged, published);
                        return Err(WorldError::new(
                            "publish file",
                            Some(path.clone()),
                            error.to_string(),
                        ));
                    }
                }
                for (_, _, backup) in &staged {
                    if let Some(backup) = backup {
                        let _ = std::fs::remove_file(backup);
                    }
                }
                Ok(())
            }
            WorldBackend::Memory(memory) => {
                let memory = Arc::make_mut(memory);
                for (path, bytes) in files {
                    memory.files.insert(path, Arc::from(bytes));
                }
                Ok(())
            }
        }
    }

    /// Opens an input stream slot by reading and pinning its content now.
    pub fn open_in(
        &mut self,
        slot: StreamSlot,
        path: impl AsRef<Path>,
    ) -> Result<FileContent, WorldError> {
        let content = self.read_file(path)?;
        self.open_in_content(slot, &content)?;
        Ok(content)
    }

    /// Opens an input stream from content already resolved and recorded by
    /// this World.
    pub fn open_in_content(
        &mut self,
        slot: StreamSlot,
        content: &FileContent,
    ) -> Result<(), WorldError> {
        let Some(record) = self.input_record(content.record) else {
            return Err(WorldError::new(
                "open input stream",
                Some(content.path.clone()),
                "resolved input record is not live in this World",
            ));
        };
        if record.path != content.path
            || record.hash != content.hash
            || record.len != content.bytes.len()
        {
            return Err(WorldError::new(
                "open input stream",
                Some(content.path.clone()),
                "resolved input content does not match its World record",
            ));
        }
        self.stream_bufs_mut().read_streams[slot.index()] = Some(ReadTarget {
            path: content.path.clone(),
            hash: content.hash,
            next_byte: 0,
        });
        Ok(())
    }

    pub fn close_in(&mut self, slot: StreamSlot) {
        self.stream_bufs_mut().read_streams[slot.index()] = None;
    }

    #[must_use]
    pub fn input_stream_eof(&self, slot: StreamSlot) -> bool {
        let Some(target) = self.stream_bufs.read_streams[slot.index()].as_ref() else {
            return true;
        };
        self.input_content(target.hash).is_none()
    }

    pub fn read_stream_line(&mut self, slot: StreamSlot) -> Result<Option<String>, WorldError> {
        let Some(target) = self.stream_bufs.read_streams[slot.index()].as_ref() else {
            return Ok(None);
        };
        let (hash, path, next_byte) = (target.hash, target.path.clone(), target.next_byte);
        let Some(bytes) = self.input_content_root(hash) else {
            return Err(WorldError::new(
                "read input stream",
                Some(path),
                "pinned input content is missing",
            ));
        };
        let Some((line, next_byte)) = next_physical_line(&bytes, next_byte) else {
            self.stream_bufs_mut().read_streams[slot.index()] = None;
            return Ok(Some(String::new()));
        };
        self.stream_bufs_mut().read_streams[slot.index()]
            .as_mut()
            .expect("read stream remained open")
            .next_byte = next_byte;
        Ok(Some(line))
    }

    /// Reads one normalized physical line from the terminal input source.
    pub fn read_terminal_line(&mut self) -> Result<Option<String>, WorldError> {
        let line = if let Some(line) = self
            .terminal_inputs
            .get(self.stream_bufs.terminal_input_next)
            .cloned()
        {
            line
        } else {
            match &mut self.backend {
                WorldBackend::Real { .. } => {
                    let mut line = String::new();
                    let read = io::stdin()
                        .read_line(&mut line)
                        .map_err(|err| WorldError::new("read terminal", None, err.to_string()))?;
                    if read == 0 {
                        return Ok(None);
                    }
                    let line = normalize_terminal_line(line);
                    self.terminal_inputs.push(line.clone());
                    line
                }
                WorldBackend::Memory(_) => {
                    return Ok(None);
                }
            }
        };
        self.stream_bufs_mut().terminal_input_next += 1;
        let bytes = line.as_bytes().to_vec();
        let record = self.allocate_input_record();
        let content = FileContent::new(record, PathBuf::from("<terminal>"), bytes);
        Arc::make_mut(&mut self.input_contents)
            .entry(content.hash)
            .or_insert_with(|| content.bytes.clone());
        Arc::make_mut(&mut self.inputs).push(InputRecord {
            path: content.path,
            hash: content.hash,
            len: content.bytes.len(),
            modification_date: content.modification_date,
            origin: content.origin,
        });
        self.record_input_identity();
        Ok(Some(line))
    }

    pub(crate) fn terminal_input_position(&self) -> TerminalInputPosition {
        TerminalInputPosition {
            owner: self.terminal_input_owner,
            next: self.stream_bufs.terminal_input_next,
        }
    }

    pub(crate) fn restore_terminal_input_position(
        &mut self,
        position: TerminalInputPosition,
    ) -> Result<(), WorldError> {
        if position.owner != self.terminal_input_owner {
            return Err(WorldError::new(
                "restore terminal input position",
                None,
                "terminal input position belongs to a different World",
            ));
        }
        if position.next > self.terminal_inputs.len() {
            return Err(WorldError::new(
                "restore terminal input position",
                None,
                "terminal input position is no longer retained",
            ));
        }
        self.stream_bufs_mut().terminal_input_next = position.next;
        Ok(())
    }

    pub fn recorded_input_content(&self, id: InputRecordId) -> Option<FileContent> {
        let record = self.input_record(id)?;
        let bytes = self.input_content_root(record.hash)?;
        Some(FileContent {
            record: id,
            path: record.path.clone(),
            bytes,
            hash: record.hash,
            modification_date: record.modification_date,
            origin: record.origin,
        })
    }

    /// Stores committed page artifact bytes by content hash.
    ///
    /// This method is intended for the shipout commit barrier: callers prepare
    /// deterministic artifact bytes first, then ask `World` to materialize the
    /// content-addressed object in the configured artifact store. Real-world
    /// publication is atomic for concurrent readers, but is not promised to
    /// survive a process or machine crash: bytes are written to a unique
    /// temporary file and renamed into place without forcing them to stable
    /// storage.
    #[allow(dead_code)]
    pub(crate) fn store_artifact(&mut self, bytes: &[u8]) -> Result<ContentHash, WorldError> {
        self.store_verified_artifact(&VerifiedArtifact::new(bytes.to_vec()))
    }

    pub(crate) fn store_verified_artifact(
        &mut self,
        artifact: &VerifiedArtifact,
    ) -> Result<ContentHash, WorldError> {
        static NEXT_TEMP_ARTIFACT: AtomicU64 = AtomicU64::new(0);
        let hash = artifact.hash();
        let bytes = artifact.bytes();
        match &mut self.backend {
            WorldBackend::Real { artifact_dir } => {
                std::fs::create_dir_all(&artifact_dir).map_err(|err| {
                    WorldError::new(
                        "create artifact directory",
                        Some(artifact_dir.clone()),
                        err.to_string(),
                    )
                })?;
                let path = artifact_dir.join(hash.hex());
                if path.exists() && !path.is_file() {
                    return Err(WorldError::new(
                        "write artifact",
                        Some(path),
                        "artifact path exists but is not a regular file",
                    ));
                }
                if path.is_file() {
                    if !self.verified_artifacts.contains(&hash) {
                        verify_stored_artifact(hash, &path, "verify stored artifact")?;
                    }
                } else {
                    let nonce = NEXT_TEMP_ARTIFACT.fetch_add(1, Ordering::Relaxed);
                    let temporary = artifact_dir.join(format!(
                        ".{}.{}.{}.tmp",
                        hash.hex(),
                        std::process::id(),
                        nonce
                    ));
                    let write_result = (|| {
                        let mut file = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&temporary)?;
                        file.write_all(bytes)?;
                        std::fs::rename(&temporary, &path)
                    })();
                    if let Err(err) = write_result {
                        let _ = std::fs::remove_file(&temporary);
                        if path.is_file() {
                            verify_stored_artifact(
                                hash,
                                &path,
                                "verify concurrently stored artifact",
                            )?;
                        } else {
                            return Err(WorldError::new(
                                "write artifact",
                                Some(path),
                                err.to_string(),
                            ));
                        }
                    }
                }
                self.verified_artifacts.insert(hash);
            }
            WorldBackend::Memory(memory) => {
                Arc::make_mut(memory)
                    .artifacts
                    .entry(hash)
                    .or_insert_with(|| bytes.to_vec());
            }
        }
        Ok(hash)
    }

    /// Reads committed page artifact bytes from the content-addressed store.
    pub fn read_artifact(&self, hash: ContentHash) -> Result<Option<Vec<u8>>, WorldError> {
        match &self.backend {
            WorldBackend::Real { artifact_dir } => {
                let path = artifact_dir.join(hash.hex());
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        verify_artifact_identity(hash, &bytes, Some(path))?;
                        Ok(Some(bytes))
                    }
                    Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
                    Err(err) => Err(WorldError::new(
                        "read artifact",
                        Some(path),
                        err.to_string(),
                    )),
                }
            }
            WorldBackend::Memory(memory) => {
                let Some(bytes) = memory.artifacts.get(&hash).cloned() else {
                    return Ok(None);
                };
                verify_artifact_identity(hash, &bytes, None)?;
                Ok(Some(bytes))
            }
        }
    }

    /// Returns committed page artifact ids in shipout order.
    ///
    /// This is downstream notification state: shipout is the commit barrier,
    /// so these entries are never rolled back or included in semantic hashes.
    #[must_use]
    pub fn artifact_commits(&self) -> &[ContentHash] {
        self.artifact_commits.as_slice()
    }

    /// Absolute artifact prefix position including the detached inherited prefix.
    #[must_use]
    pub fn artifact_pos(&self) -> usize {
        self.artifact_base + self.artifact_commits.len()
    }

    /// Returns the in-process commit receipts aligned with
    /// [`Self::artifact_commits`].
    ///
    /// These are downstream notification state, not rollback or semantic
    /// state. Durable consumers should retain the content id and use
    /// [`Self::read_artifact`] in a later process.
    #[must_use]
    pub fn committed_artifacts(&self) -> &[CommittedArtifact] {
        self.committed_artifacts.as_slice()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn artifact_publications(&self) -> &[ArtifactPublicationRecord] {
        self.artifact_publications.as_slice()
    }

    /// Returns the ordered records committed under a still-provisional page
    /// output receipt. The registry is part of the World timeline, so a
    /// checkpoint observes exactly the records that existed at capture time.
    #[doc(hidden)]
    #[must_use]
    pub fn provisional_page_output_receipt(
        &self,
        receipt: PageOutputPublicationReceiptId,
    ) -> Option<Arc<[ArtifactPublicationRecord]>> {
        self.provisional_page_output_receipts.get(&receipt).cloned()
    }

    #[doc(hidden)]
    pub fn discard_provisional_page_output_receipt(
        &mut self,
        receipt: PageOutputPublicationReceiptId,
    ) {
        Arc::make_mut(&mut self.provisional_page_output_receipts).remove(&receipt);
    }

    #[doc(hidden)]
    #[must_use]
    pub fn artifact_publication_at(&self, absolute: usize) -> Option<ArtifactPublicationRecord> {
        absolute
            .checked_sub(self.artifact_base)
            .and_then(|index| self.artifact_publications.get(index))
            .copied()
    }

    #[doc(hidden)]
    pub fn link_artifact_effect_publication(
        &mut self,
        artifact: ArtifactPublicationId,
        effect: EffectPublicationId,
    ) {
        let record = Arc::make_mut(&mut self.artifact_publications)
            .iter_mut()
            .rev()
            .find(|record| record.publication == artifact)
            .expect("committed artifact publication is present");
        record.effect_publication = Some(effect);
        if let Some(records) =
            Arc::make_mut(&mut self.provisional_page_output_receipts).get_mut(&record.receipt())
        {
            let record = Arc::make_mut(records)
                .iter_mut()
                .find(|record| record.publication() == artifact)
                .expect("provisional receipt contains committed artifact publication");
            record.effect_publication = Some(effect);
        }
    }

    #[doc(hidden)]
    pub fn reserve_artifact_publication(
        &mut self,
        sequence: EffectSequence,
        domain: EffectDomain,
        receipt: Option<PageOutputPublicationReceiptId>,
    ) -> ArtifactPublicationReservation {
        self.next_publication_sequence = self.next_publication_sequence.max(sequence.0);
        self.next_artifact_publication_identity = self
            .next_artifact_publication_identity
            .checked_add(1)
            .expect("artifact publication identity exhausted");
        ArtifactPublicationReservation {
            record: ArtifactPublicationRecord {
                publication: ArtifactPublicationId::new(self.next_artifact_publication_identity),
                receipt: receipt.unwrap_or_else(|| {
                    PageOutputPublicationReceiptId::new(
                        u64::MAX - self.next_artifact_publication_identity,
                    )
                }),
                effect_publication: None,
                sequence,
                domain,
                intra_order: 0,
            },
            provisional_receipt: receipt.is_some(),
        }
    }

    #[doc(hidden)]
    pub fn reserve_artifact_publication_at(
        &mut self,
        effect_index: usize,
    ) -> ArtifactPublicationReservation {
        let sequence = self.allocate_publication_sequence();
        let domain = self
            .effect_domains
            .get(effect_index)
            .copied()
            .unwrap_or_else(|| self.allocate_effect_domain());
        self.reserve_artifact_publication(sequence, domain, None)
    }

    #[doc(hidden)]
    pub fn set_active_artifact_publication_group(
        &mut self,
        group: Option<(EffectSequence, EffectDomain)>,
    ) {
        if let Some((sequence, _)) = group {
            self.next_publication_sequence = self.next_publication_sequence.max(sequence.0);
        }
        self.active_artifact_publication_group =
            group.map(|(sequence, domain)| ArtifactPublicationGroup {
                sequence,
                domain,
                next_intra_order: 0,
            });
    }

    #[doc(hidden)]
    pub fn reserve_active_artifact_publication_at(
        &mut self,
        effect_index: usize,
        receipt: Option<PageOutputPublicationReceiptId>,
    ) -> ArtifactPublicationReservation {
        let provisional_group = receipt.and_then(|receipt| {
            self.provisional_page_output_receipts
                .get(&receipt)
                .and_then(|records| {
                    let first = records.first()?;
                    Some(ArtifactPublicationGroup {
                        sequence: first.sequence(),
                        domain: first.domain(),
                        next_intra_order: records
                            .iter()
                            .map(|record| record.intra_order())
                            .max()?
                            .checked_add(1)
                            .expect("artifact publication intra-order exhausted"),
                    })
                })
        });
        let group = if let Some(group) = provisional_group {
            group
        } else if let Some(group) = self.active_artifact_publication_group.as_mut() {
            let reserved = *group;
            group.next_intra_order = group
                .next_intra_order
                .checked_add(1)
                .expect("artifact publication intra-order exhausted");
            reserved
        } else {
            let sequence = self.allocate_publication_sequence();
            let domain = self
                .effect_domains
                .get(effect_index)
                .copied()
                .unwrap_or_else(|| self.allocate_effect_domain());
            ArtifactPublicationGroup {
                sequence,
                domain,
                next_intra_order: 0,
            }
        };
        let mut reservation =
            self.reserve_artifact_publication(group.sequence, group.domain, receipt);
        reservation.record.intra_order = group.next_intra_order;
        reservation
    }

    #[doc(hidden)]
    pub fn begin_terminal_publication(&mut self, phase: TerminalPublicationPhase) {
        assert!(
            self.active_terminal_publication.is_none(),
            "terminal publication transaction is already active"
        );
        self.next_terminal_publication_identity = self
            .next_terminal_publication_identity
            .checked_add(1)
            .expect("terminal publication identity exhausted");
        let identity = TerminalPublicationId::new(self.next_terminal_publication_identity);
        let sequence = self.allocate_publication_sequence();
        self.active_terminal_publication = Some(TerminalPublication {
            identity,
            sequence,
            phase,
            start: self.effects.len(),
            next_intra_order: 0,
        });
    }

    #[doc(hidden)]
    pub fn commit_terminal_publication(&mut self) {
        let publication = self
            .active_terminal_publication
            .take()
            .expect("terminal publication transaction is active");
        for domain in &mut Arc::make_mut(&mut self.effect_domains)[publication.start..] {
            if let EffectDomain::TerminalPublication {
                identity,
                committed,
                ..
            } = domain
                && *identity == publication.identity
            {
                *committed = true;
            }
        }
    }

    pub(crate) fn record_artifact_commit(
        &mut self,
        hash: ContentHash,
        bytes: Vec<u8>,
        render_provenance: ArtifactRenderProvenance,
        open_out_occurrences: Vec<(usize, ArtifactEffectOrdinal)>,
        reservation: ArtifactPublicationReservation,
    ) {
        Arc::make_mut(&mut self.artifact_commits).push(hash);
        self.record_artifact_identity(hash);
        Arc::make_mut(&mut self.committed_artifacts).push(CommittedArtifact::new(
            hash,
            bytes,
            render_provenance,
            open_out_occurrences,
        ));
        Arc::make_mut(&mut self.artifact_publications).push(reservation.record);
        if reservation.provisional_receipt {
            let receipts = Arc::make_mut(&mut self.provisional_page_output_receipts);
            let records = receipts.entry(reservation.record.receipt()).or_default();
            let mut ordered = records.to_vec();
            ordered.push(reservation.record);
            ordered.sort_by_key(|record| (record.intra_order(), record.publication()));
            ordered.dedup_by_key(|record| record.publication());
            *records = Arc::from(ordered);
        }
    }

    pub(crate) fn store_prepared_artifact(
        &mut self,
        artifact: &CommittedArtifact,
    ) -> Result<ContentHash, WorldError> {
        let verified = VerifiedArtifact {
            hash: artifact.hash,
            bytes: artifact.bytes.clone(),
            render_provenance: artifact.render_provenance.clone(),
            open_out_occurrences: artifact.open_out_occurrences.to_vec(),
        };
        self.store_verified_artifact(&verified)
    }

    pub(crate) fn record_prepared_artifact(
        &mut self,
        artifact: CommittedArtifact,
        publication: ArtifactPublicationRecord,
    ) {
        let hash = artifact.hash;
        Arc::make_mut(&mut self.artifact_commits).push(hash);
        self.record_artifact_identity(hash);
        Arc::make_mut(&mut self.committed_artifacts).push(artifact);
        Arc::make_mut(&mut self.artifact_publications).push(publication);
    }

    /// Mutation-free validation for a detached terminal publication.
    #[doc(hidden)]
    pub fn preflight_detached_publication(&self) -> Result<(), WorldError> {
        if self.effect_pos() != EffectPos::default()
            || !self.effect_records().is_empty()
            || !self.committed_artifacts().is_empty()
        {
            return Err(WorldError::new(
                "publish detached completion",
                None,
                "destination already contains an unpublished effect or page artifact",
            ));
        }
        if let Some(error) = &self.effect_commit_poison {
            return Err(error.clone());
        }
        if self.commit_mode == WorldCommitMode::Exported {
            return Err(WorldError::new(
                "publish detached completion",
                None,
                "destination retained session was already exported",
            ));
        }
        Ok(())
    }

    /// Validates that a retry destination still contains exactly the prefix
    /// committed by the preceding detached-publication attempt.
    #[doc(hidden)]
    pub fn preflight_detached_retry(&self, committed_prefix: usize) -> Result<(), WorldError> {
        let expected = u64::try_from(committed_prefix).unwrap_or(u64::MAX);
        if self.commit_mode == WorldCommitMode::Retained
            || !self.effect_records().is_empty()
            || !self.committed_artifacts().is_empty()
            || self.effect_pos().raw() != expected
        {
            return Err(WorldError::new(
                "retry detached completion",
                None,
                "destination no longer contains the exact committed effect prefix",
            ));
        }
        if let Some(error) = &self.effect_commit_poison {
            return Err(error.clone());
        }
        Ok(())
    }

    /// Publishes one detached suffix without exposing destination positions.
    /// A safe failure removes the uncommitted tail; the successful prefix stays
    /// committed and is reported as a count relative to this call.
    #[doc(hidden)]
    pub fn publish_detached_effect_records(
        &mut self,
        records: &[EffectRecord],
    ) -> Result<(), DetachedEffectPublicationError> {
        let contexts = vec![None; records.len()];
        self.publish_detached_effect_records_with_contexts(records, &contexts)
    }

    /// Publishes one detached suffix with ordinal-aligned stream-open context.
    #[doc(hidden)]
    pub fn publish_detached_effect_records_with_contexts(
        &mut self,
        records: &[EffectRecord],
        stream_open_contexts: &[Option<String>],
    ) -> Result<(), DetachedEffectPublicationError> {
        assert_eq!(
            records.len(),
            stream_open_contexts.len(),
            "detached effect contexts must stay ordinal-aligned"
        );
        let start = self.effect_pos();
        for (record, context) in records.iter().zip(stream_open_contexts) {
            self.append_effect(record.clone());
            if let Some(context) = context {
                assert!(
                    matches!(record, EffectRecord::StreamOpen { .. }),
                    "only a detached stream open may carry rendered context"
                );
                self.set_last_stream_open_context(context.clone());
            }
        }
        if self.commit_mode == WorldCommitMode::Retained {
            return Ok(());
        }
        let end = self.effect_pos();
        if let Err(error) = self.commit_effects(end) {
            let committed = error
                .committed_effects_through()
                .and_then(|through| through.raw().checked_sub(start.raw()))
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or(0);
            let (failed_ordinal, slot, path) = error
                .stream_open_unavailable()
                .and_then(|failure| {
                    failure
                        .position()
                        .raw()
                        .checked_sub(start.raw())
                        .and_then(|ordinal| u32::try_from(ordinal).ok())
                        .map(|ordinal| {
                            (
                                Some(ordinal),
                                Some(failure.slot()),
                                Some(failure.path().to_owned()),
                            )
                        })
                })
                .unwrap_or((None, None, None));
            let pending = self.effects.len();
            self.effects_mut().truncate(0);
            Arc::make_mut(&mut self.effect_sequences).truncate(0);
            Arc::make_mut(&mut self.effect_publications).truncate(0);
            Arc::make_mut(&mut self.effect_publication_record_ordinals).truncate(0);
            Arc::make_mut(&mut self.effect_domains).truncate(0);
            Arc::make_mut(&mut self.effect_semantic_record_ordinals).truncate(0);
            Arc::make_mut(&mut self.effect_placement_intra_orders).truncate(0);
            Arc::make_mut(&mut self.stream_open_contexts)
                .retain(|position, _| *position <= self.effect_base);
            debug_assert_eq!(pending, records.len().saturating_sub(committed));
            return Err(DetachedEffectPublicationError {
                committed,
                failed_ordinal,
                slot,
                path,
                error: Box::new(error),
            });
        }
        Ok(())
    }

    /// Stores and publishes a completely validated detached page set. Runtime
    /// publication identities are allocated only inside the destination.
    #[doc(hidden)]
    pub fn publish_detached_artifacts(
        &mut self,
        artifacts: Vec<CommittedArtifact>,
    ) -> Result<(), WorldError> {
        for artifact in &artifacts {
            verify_artifact_identity(artifact.hash(), artifact.bytes(), None)?;
        }
        for artifact in &artifacts {
            self.store_prepared_artifact(artifact)?;
        }
        for artifact in artifacts {
            let reservation = self.reserve_artifact_publication_at(0);
            self.record_prepared_artifact(artifact, reservation.record);
        }
        Ok(())
    }

    pub fn open_out(&mut self, slot: StreamSlot, path: impl Into<PathBuf>) {
        let target = WriteTarget { path: path.into() };
        self.append_effect(EffectRecord::StreamOpen {
            slot,
            target: target.clone(),
        });
        self.stream_bufs_mut().write_streams[slot.index()] = Some(target);
    }

    /// Whether TeX's numbered output stream is currently open.
    ///
    /// TeX82 §1370 uses this live bit when an immediate write executes; a
    /// closed numbered stream writes through the current print selector.
    #[must_use]
    pub fn write_stream_is_open(&self, slot: StreamSlot) -> bool {
        self.stream_bufs().write_streams[slot.index()].is_some()
    }

    /// Attaches the canonical input display captured for the just-recorded open.
    pub fn set_last_stream_open_context(&mut self, context: impl Into<String>) {
        let position = self.effect_pos();
        assert!(
            matches!(self.effects.last(), Some(EffectRecord::StreamOpen { .. })),
            "stream-open context must follow its exact effect"
        );
        Arc::make_mut(&mut self.stream_open_contexts).insert(position, context.into());
    }

    /// Returns a retained host outcome for an output target when one exists.
    ///
    /// The memory backend has an authoritative, immutable answer. A real
    /// filesystem does not: probing it here would either create the file
    /// before the effect commit or introduce a probe/commit TOCTOU. Real
    /// opens therefore remain deferred to the atomic `StreamOpen` effect.
    #[must_use]
    pub fn retained_output_open_outcome(
        &self,
        path: impl AsRef<Path>,
    ) -> RetainedOutputOpenOutcome {
        match &self.backend {
            WorldBackend::Real { .. } => RetainedOutputOpenOutcome::DeferredToCommit,
            WorldBackend::Memory(_) if self.unavailable_memory_outputs.contains(path.as_ref()) => {
                RetainedOutputOpenOutcome::Unavailable
            }
            WorldBackend::Memory(_) => RetainedOutputOpenOutcome::Available,
        }
    }

    /// Makes one memory output name unavailable to focused engine tests.
    pub fn deny_memory_output(&mut self, path: impl Into<PathBuf>) {
        self.unavailable_memory_outputs.insert(path.into());
    }

    /// Closes an open numbered output stream.
    ///
    /// TeX82 §1374 tests `write_open[j]` before closing the file. A close of
    /// a never-opened stream therefore has no host effect and records no
    /// [`EffectRecord::StreamClose`].
    pub fn close_out(&mut self, slot: StreamSlot) -> bool {
        if self.stream_bufs().write_streams[slot.index()].is_none() {
            return false;
        }
        self.append_effect(EffectRecord::StreamClose { slot });
        self.stream_bufs_mut().write_streams[slot.index()] = None;
        true
    }

    /// Buffers routed output as a deferred effect record.
    /// tex.web §58's `print_char`, over a whole string.
    ///
    /// §58 wraps the terminal and the transcript independently: each keeps
    /// its own offset (`term_offset`, `file_offset`), and each emits a line
    /// break of its own the moment that offset reaches `max_print_line`.
    /// A `\write` stream has no offset in §58 at all -- its case is a bare
    /// `write(write_file[selector],xchr[s])` -- so stream text is never
    /// wrapped.
    ///
    /// The two printable offsets diverge routinely (§71's log-only echo of a
    /// typed line, §245's `begin_diagnostic` redirect, §90's help lines), so
    /// one [`PrintSink::TerminalAndLog`] call can legitimately place its
    /// breaks at different points in the two sinks. The record describes what
    /// each sink actually received, so a call whose two wrappings differ
    /// records one write per sink rather than one shared write.
    pub fn write_text(&mut self, sink: PrintSink, text: &str) {
        self.write_text_with_line_limit(sink, text, crate::print::MAX_PRINT_LINE);
    }

    /// Publishes text which already crossed an admitted TeX printer while
    /// retaining that printer's process-selected §3 line width.
    ///
    /// The outer owner calls this immediately after releasing the command
    /// context, so wrapping is evaluated against the live per-sink partial
    /// lines without exposing those lines through the admitted facade.
    pub fn publish_print_text(&mut self, sink: PrintSink, text: &str, max_print_line: usize) {
        self.write_text_with_line_limit(sink, text, max_print_line);
    }

    /// Publishes tex.web §62's `print_nl` followed by already-rendered text.
    ///
    /// The caller supplies no captured offset. This outer boundary evaluates
    /// the current per-sink partial lines after every earlier detached effect
    /// has committed, then records the optional break and text as one logical
    /// write. For `term_and_log`, tex.web's shared predicate means either open
    /// selected line inserts the newline in both sinks.
    pub fn publish_print_nl_text(&mut self, sink: PrintSink, text: &str, max_print_line: usize) {
        let (terminal_open, log_open) = {
            let bufs = self.stream_bufs();
            (
                !bufs.terminal_partial_line.is_empty(),
                !bufs.log_partial_line.is_empty(),
            )
        };
        let line_is_open = match sink {
            PrintSink::Terminal => terminal_open,
            PrintSink::Log => log_open,
            PrintSink::TerminalAndLog => terminal_open || log_open,
            PrintSink::Stream(_) => false,
        };
        if line_is_open {
            let mut framed = String::with_capacity(text.len().saturating_add(1));
            framed.push('\n');
            framed.push_str(text);
            self.write_text_with_line_limit(sink, &framed, max_print_line);
        } else {
            self.write_text_with_line_limit(sink, text, max_print_line);
        }
    }

    /// Detaches tex.web's terminal and transcript partial-line predicates for
    /// an outer publication barrier.
    #[must_use]
    pub fn printable_lines_are_open(&self) -> (bool, bool) {
        let bufs = self.stream_bufs();
        (
            !bufs.terminal_partial_line.is_empty(),
            !bufs.log_partial_line.is_empty(),
        )
    }

    pub(crate) fn write_text_with_line_limit(
        &mut self,
        sink: PrintSink,
        text: &str,
        max_print_line: usize,
    ) {
        let (terminal_offset, log_offset) = {
            let bufs = self.stream_bufs();
            (
                bufs.terminal_partial_line.chars().count(),
                bufs.log_partial_line.chars().count(),
            )
        };
        match sink {
            PrintSink::Terminal => {
                let wrapped = wrap_print_lines_at(text, terminal_offset, max_print_line);
                self.record_printable_write(PrintSink::Terminal, wrapped);
            }
            PrintSink::Log => {
                let wrapped = wrap_print_lines_at(text, log_offset, max_print_line);
                self.record_printable_write(PrintSink::Log, wrapped);
            }
            PrintSink::TerminalAndLog => {
                let terminal = wrap_print_lines_at(text, terminal_offset, max_print_line);
                let log = wrap_print_lines_at(text, log_offset, max_print_line);
                if terminal == log {
                    self.record_printable_write(PrintSink::TerminalAndLog, terminal);
                } else {
                    self.record_printable_write(PrintSink::Terminal, terminal);
                    self.record_printable_write(PrintSink::Log, log);
                }
            }
            PrintSink::Stream(_) => {
                self.append_effect(EffectRecord::StreamWrite {
                    sink,
                    text: text.to_owned(),
                });
            }
        }
    }

    /// Publishes the ordered diagnostics produced by one committed command
    /// operation.
    ///
    /// Each detached effect is evaluated against the then-current terminal
    /// and transcript partial lines. A logical diagnostic appends no records
    /// when it has no routed bytes, one record when both sinks receive the
    /// same payload, or two physical records sharing one effect sequence when
    /// their independently wrapped payloads differ. No intermediate print
    /// primitive is observable in the World journal.
    pub fn publish_diagnostic_effects(
        &mut self,
        mut effects: crate::diagnostic::DiagnosticEffects,
    ) {
        for effect in effects.drain() {
            self.publish_diagnostic_effect(effect);
        }
    }

    fn publish_diagnostic_effect(&mut self, effect: crate::diagnostic::DetachedDiagnosticEffect) {
        if effect.records_warning_history() {
            self.error_channel_mut().record_warning_history();
        }
        let Some(sink) = effect.selector().sink() else {
            return;
        };
        let max_print_line = effect.max_print_line();
        let (terminal_line, log_line) = {
            let bufs = self.stream_bufs();
            (
                bufs.terminal_partial_line.clone(),
                bufs.log_partial_line.clone(),
            )
        };
        let render =
            |line: &str| render_detached_diagnostic(effect.operations(), line, max_print_line);
        let mut records = Vec::with_capacity(2);
        match sink {
            PrintSink::Terminal => {
                let (text, _) = render(&terminal_line);
                if !text.is_empty() {
                    records.push(EffectRecord::StreamWrite {
                        sink: PrintSink::Terminal,
                        text,
                    });
                }
            }
            PrintSink::Log => {
                let (text, _) = render(&log_line);
                if !text.is_empty() {
                    records.push(EffectRecord::StreamWrite {
                        sink: PrintSink::Log,
                        text,
                    });
                }
            }
            PrintSink::TerminalAndLog => {
                // tex.web §62 tests both selected offsets together before
                // calling §57 `print_ln`. If either line is open, that one
                // call writes a newline to both terminal and transcript; the
                // sinks are independent again for ordinary text wrapping.
                // Replaying the program twice in isolation loses the blank
                // transcript line when only the terminal is open (e-TeX
                // change 17.516's forced-online missing-character report).
                let (terminal, log) = render_detached_diagnostic_pair(
                    effect.operations(),
                    &terminal_line,
                    &log_line,
                    max_print_line,
                );
                if terminal == log {
                    if !terminal.is_empty() {
                        records.push(EffectRecord::StreamWrite {
                            sink: PrintSink::TerminalAndLog,
                            text: terminal,
                        });
                    }
                } else {
                    if !terminal.is_empty() {
                        records.push(EffectRecord::StreamWrite {
                            sink: PrintSink::Terminal,
                            text: terminal,
                        });
                    }
                    if !log.is_empty() {
                        records.push(EffectRecord::StreamWrite {
                            sink: PrintSink::Log,
                            text: log,
                        });
                    }
                }
            }
            PrintSink::Stream(_) => {
                unreachable!("§245 diagnostics never select a numbered stream")
            }
        }
        self.append_printable_batch(records);
    }

    fn append_printable_batch(&mut self, records: Vec<EffectRecord>) {
        let start = self.effects.len();
        for record in &records {
            let EffectRecord::StreamWrite { sink, text } = record else {
                unreachable!("a diagnostic batch contains printable writes only")
            };
            let mut bufs = self.stream_bufs_mut();
            match sink {
                PrintSink::Terminal => append_partial_line(&mut bufs.terminal_partial_line, text),
                PrintSink::Log => append_partial_line(&mut bufs.log_partial_line, text),
                PrintSink::TerminalAndLog => {
                    append_partial_line(&mut bufs.terminal_partial_line, text);
                    append_partial_line(&mut bufs.log_partial_line, text);
                }
                PrintSink::Stream(_) => {
                    unreachable!("a diagnostic batch contains printable writes only")
                }
            }
        }
        for record in records {
            self.append_effect(record);
        }
        let end = self.effects.len();
        if end > start + 1 {
            let sequence = self.effect_sequences[start];
            Arc::make_mut(&mut self.effect_sequences)[start..end].fill(sequence);
        }
    }

    /// Buffers bytes that have already crossed the active character-profile
    /// encoding boundary.
    ///
    /// Unlike [`Self::write_text`], this API never projects through UTF-8.
    /// It is the final output seam for TeX82 byte-domain characters and is
    /// also suitable for any future profile with an explicit external
    /// encoding. Encoding policy belongs to the caller; `World` retains and
    /// commits the resulting bytes exactly. Printable sinks still pass
    /// through §58's independent terminal and transcript line meters; a
    /// numbered `\write` stream remains unmetered.
    pub fn write_encoded_bytes(&mut self, sink: PrintSink, bytes: &[u8]) {
        self.write_encoded_bytes_with_line_limit(sink, bytes, crate::print::MAX_PRINT_LINE);
    }

    pub(crate) fn write_encoded_bytes_with_line_limit(
        &mut self,
        sink: PrintSink,
        bytes: &[u8],
        max_print_line: usize,
    ) {
        let (terminal_offset, log_offset) = {
            let bufs = self.stream_bufs();
            (
                bufs.terminal_partial_line.chars().count(),
                bufs.log_partial_line.chars().count(),
            )
        };
        match sink {
            PrintSink::Terminal => {
                let wrapped = wrap_print_bytes_at(bytes, terminal_offset, max_print_line);
                self.record_printable_bytes(PrintSink::Terminal, wrapped);
            }
            PrintSink::Log => {
                let wrapped = wrap_print_bytes_at(bytes, log_offset, max_print_line);
                self.record_printable_bytes(PrintSink::Log, wrapped);
            }
            PrintSink::TerminalAndLog => {
                let terminal = wrap_print_bytes_at(bytes, terminal_offset, max_print_line);
                let log = wrap_print_bytes_at(bytes, log_offset, max_print_line);
                if terminal == log {
                    self.record_printable_bytes(PrintSink::TerminalAndLog, terminal);
                } else {
                    self.record_printable_bytes(PrintSink::Terminal, terminal);
                    self.record_printable_bytes(PrintSink::Log, log);
                }
            }
            PrintSink::Stream(_) => {
                self.append_effect(EffectRecord::StreamWriteBytes {
                    sink,
                    bytes: bytes.to_vec(),
                });
            }
        }
    }

    /// tex.web §71's `term_input` after `input_ln` has succeeded.
    ///
    /// §71 does two things the read itself does not. `term_offset:=0` records
    /// that "the user's line ended with <return>": the prompt is still on the
    /// screen, but the cursor is at the left margin, so the next `print_nl`
    /// must not break. Then `decr(selector)` echoes the line the user typed to
    /// the *transcript alone* -- the terminal already showed it as it was
    /// typed -- and ends that transcript line.
    ///
    /// This is why a prompt and the message after it share one terminal line
    /// while the transcript shows the prompt, the answer, and the message on
    /// three.
    pub fn echo_terminal_input(&mut self, line: &str) {
        self.stream_bufs_mut().terminal_partial_line.clear();
        self.write_text(PrintSink::Log, line);
        self.write_text(PrintSink::Log, "\n");
    }

    /// tex.web §54's `wterm`/`wlog`: a direct write to the terminal or the
    /// transcript that bypasses §58's print primitives entirely.
    ///
    /// The Pascal `write(term_out,...)` and `write(log_file,...)` macros
    /// touch neither `term_offset` nor `file_offset`, so text sent this way
    /// neither wraps at `max_print_line` nor counts toward the column a
    /// later `print_nl` consults. §61's and §536's start-up banners are the
    /// sites that depend on it: the banner is longer than `max_print_line`
    /// and is nevertheless one unbroken line in every reference transcript.
    pub fn write_text_unmetered(&mut self, sink: PrintSink, text: &str) {
        debug_assert!(
            !matches!(sink, PrintSink::Stream(_)),
            "§54's wterm/wlog address the terminal and the transcript only"
        );
        self.append_effect(EffectRecord::StreamWrite {
            sink,
            text: text.to_owned(),
        });
    }

    /// Records one already-wrapped write to a printable sink and advances
    /// that sink's §58 offset.
    fn record_printable_write(&mut self, sink: PrintSink, text: String) {
        self.append_effect(EffectRecord::StreamWrite {
            sink,
            text: text.clone(),
        });
        let mut bufs = self.stream_bufs_mut();
        match sink {
            PrintSink::Terminal => append_partial_line(&mut bufs.terminal_partial_line, &text),
            PrintSink::Log => append_partial_line(&mut bufs.log_partial_line, &text),
            PrintSink::TerminalAndLog => {
                append_partial_line(&mut bufs.terminal_partial_line, &text);
                append_partial_line(&mut bufs.log_partial_line, &text);
            }
            PrintSink::Stream(_) => unreachable!("stream writes are not printable-sink writes"),
        }
    }

    fn record_printable_bytes(&mut self, sink: PrintSink, bytes: Vec<u8>) {
        self.append_effect(EffectRecord::StreamWriteBytes {
            sink,
            bytes: bytes.clone(),
        });
        let projection = bytes_to_partial_line_projection(&bytes);
        let mut bufs = self.stream_bufs_mut();
        match sink {
            PrintSink::Terminal => {
                append_partial_line(&mut bufs.terminal_partial_line, &projection)
            }
            PrintSink::Log => append_partial_line(&mut bufs.log_partial_line, &projection),
            PrintSink::TerminalAndLog => {
                append_partial_line(&mut bufs.terminal_partial_line, &projection);
                append_partial_line(&mut bufs.log_partial_line, &projection);
            }
            PrintSink::Stream(_) => unreachable!("stream writes are not printable-sink writes"),
        }
    }
    pub fn record_special(&mut self, class: impl Into<String>, payload: impl Into<Vec<u8>>) {
        self.append_effect(EffectRecord::Special {
            class: class.into(),
            payload: payload.into(),
        });
    }

    pub fn record_pdf_object_placeholder(&mut self, label: impl Into<String>) {
        self.append_effect(EffectRecord::PdfObjectPlaceholder {
            label: label.into(),
        });
    }

    /// Records a shell escape request without executing it by default.
    pub fn record_shell_escape(&mut self, command: impl Into<String>) -> bool {
        let allowed = self.shell_escape_policy == ShellEscapePolicy::Enabled;
        let record = ShellEscapeRecord {
            command: command.into(),
            allowed,
        };
        self.append_effect(EffectRecord::ShellEscape(record.clone()));
        self.shell_escapes.push(record);
        allowed
    }

    /// Flushes all effect records up to `effect_pos`, in order, exactly once.
    pub(crate) fn commit_effects(&mut self, effect_pos: EffectPos) -> Result<(), WorldError> {
        if let Some(error) = &self.effect_commit_poison {
            return Err(error.clone());
        }
        if effect_pos <= self.effect_base {
            return Ok(());
        }
        if effect_pos > self.effect_pos() {
            return Err(WorldError::new(
                "commit effects",
                None,
                format!(
                    "effect position {} is beyond current end {}",
                    effect_pos.raw(),
                    self.effect_pos().raw()
                ),
            )
            .effect_commit(self.effect_base, EffectRetrySafety::Safe));
        }

        let mut applied = 0usize;
        let count = (effect_pos.raw() - self.effect_base.raw()) as usize;
        for index in 0..count {
            if let Err(err) = self.apply_effect(index) {
                if applied > 0 {
                    self.drain_page_effect_interval_prefix(applied);
                    self.effects_mut().drain(0..applied);
                    Arc::make_mut(&mut self.effect_sequences).drain(0..applied);
                    Arc::make_mut(&mut self.effect_publications).drain(0..applied);
                    Arc::make_mut(&mut self.effect_publication_record_ordinals).drain(0..applied);
                    Arc::make_mut(&mut self.effect_domains).drain(0..applied);
                    Arc::make_mut(&mut self.effect_semantic_record_ordinals).drain(0..applied);
                    Arc::make_mut(&mut self.effect_placement_intra_orders).drain(0..applied);
                    self.effect_base.0 += applied as u64;
                    Arc::make_mut(&mut self.stream_open_contexts)
                        .retain(|position, _| *position > self.effect_base);
                }
                let retry_safety = match err.retry_safety() {
                    EffectRetrySafety::Safe => EffectRetrySafety::Safe,
                    EffectRetrySafety::NotAnEffectCommit | EffectRetrySafety::Poisoned => {
                        EffectRetrySafety::Poisoned
                    }
                };
                let err = err.effect_commit(self.effect_base, retry_safety);
                if retry_safety == EffectRetrySafety::Poisoned {
                    self.effect_commit_poison = Some(err.clone());
                }
                return Err(err);
            }
            applied += 1;
        }

        self.drain_page_effect_interval_prefix(applied);
        self.effects_mut().drain(0..applied);
        Arc::make_mut(&mut self.effect_sequences).drain(0..applied);
        Arc::make_mut(&mut self.effect_publications).drain(0..applied);
        Arc::make_mut(&mut self.effect_publication_record_ordinals).drain(0..applied);
        Arc::make_mut(&mut self.effect_domains).drain(0..applied);
        Arc::make_mut(&mut self.effect_semantic_record_ordinals).drain(0..applied);
        Arc::make_mut(&mut self.effect_placement_intra_orders).drain(0..applied);
        self.effect_base = effect_pos;
        Arc::make_mut(&mut self.stream_open_contexts)
            .retain(|position, _| *position > self.effect_base);
        Ok(())
    }

    #[must_use]
    pub const fn shell_escape_policy(&self) -> ShellEscapePolicy {
        self.shell_escape_policy
    }

    pub fn set_shell_escape_policy(&mut self, policy: ShellEscapePolicy) {
        let old = self
            .reachable_state_identity
            .as_ref()
            .map(|_| stable_hash(&self.shell_escape_policy));
        self.shell_escape_policy = policy;
        if let Some(old) = old {
            self.replace_identity_scalar(6, old, stable_hash(&self.shell_escape_policy));
        }
    }

    #[must_use]
    pub fn next_random_u64(&mut self) -> u64 {
        let old = self
            .reachable_state_identity
            .as_ref()
            .map(|_| stable_hash(&self.rng));
        let value = self.rng.next_u64();
        if let Some(old) = old {
            self.replace_identity_scalar(1, old, stable_hash(&self.rng));
        }
        value
    }

    /// Re-seeds pdfTeX's independent deterministic random stream.
    pub fn set_pdf_random_seed(&mut self, seed: i32) {
        let old = self
            .reachable_state_identity
            .as_ref()
            .map(|_| stable_hash(&self.pdf_rng));
        self.pdf_rng = PdfRandomState::from_seed(seed);
        if let Some(old) = old {
            self.replace_identity_scalar(2, old, stable_hash(&self.pdf_rng));
        }
    }

    #[must_use]
    pub fn pdf_random_seed(&self) -> i32 {
        self.pdf_rng.seed
    }

    #[must_use]
    pub fn pdf_uniform_deviate(&mut self, bound: i32) -> i32 {
        let old = self
            .reachable_state_identity
            .as_ref()
            .map(|_| stable_hash(&self.pdf_rng));
        let value = self.pdf_rng.uniform(bound);
        if let Some(old) = old {
            self.replace_identity_scalar(2, old, stable_hash(&self.pdf_rng));
        }
        value
    }

    #[must_use]
    pub fn pdf_normal_deviate(&mut self) -> i32 {
        let old = self
            .reachable_state_identity
            .as_ref()
            .map(|_| stable_hash(&self.pdf_rng));
        let value = self.pdf_rng.normal();
        if let Some(old) = old {
            self.replace_identity_scalar(2, old, stable_hash(&self.pdf_rng));
        }
        value
    }

    /// Supplies the current monotonic time without consulting the host during expansion.
    pub fn set_pdf_time_micros(&mut self, micros: u64) {
        let old = self
            .reachable_state_identity
            .as_ref()
            .map(|_| stable_hash(&self.pdf_time_micros));
        self.pdf_time_micros = micros;
        if let Some(old) = old {
            self.replace_identity_scalar(3, old, stable_hash(&self.pdf_time_micros));
        }
    }

    pub fn reset_pdf_timer(&mut self) {
        let old = self
            .reachable_state_identity
            .as_ref()
            .map(|_| stable_hash(&self.pdf_timer_origin_micros));
        self.pdf_timer_origin_micros = self.pdf_time_micros;
        if let Some(old) = old {
            self.replace_identity_scalar(4, old, stable_hash(&self.pdf_timer_origin_micros));
        }
    }

    #[must_use]
    pub fn pdf_elapsed_time(&self) -> i32 {
        let elapsed = self
            .pdf_time_micros
            .saturating_sub(self.pdf_timer_origin_micros);
        if elapsed / 1_000_000 > 32_767 {
            i32::MAX
        } else {
            i32::try_from((elapsed / 100) * 65_536 / 10_000).unwrap_or(i32::MAX)
        }
    }

    #[must_use]
    pub const fn job_clock(&self) -> JobClock {
        self.job_clock
    }

    #[must_use]
    pub fn input_records(&self) -> InputRecords<'_> {
        InputRecords { world: self }
    }

    fn accepted_input_len(&self) -> usize {
        self.accepted_inputs
            .as_ref()
            .map_or(0, |block| block.total_len)
    }

    /// Records one authoritative semantic observation of a canonical path.
    ///
    /// Repeated observations are reduced by path. Required reads dominate
    /// probes, and a later authoritative outcome replaces an earlier one.
    pub fn record_input_dependency(
        &mut self,
        path: impl Into<PathBuf>,
        outcome: InputDependencyOutcome,
        access: InputDependencyAccess,
    ) -> Result<(), WorldError> {
        let path = path.into();
        if let Some(existing) = self.input_dependency(path.as_path()).cloned() {
            let mut updated = existing;
            updated.outcome = outcome;
            if access == InputDependencyAccess::RequiredRead {
                updated.access = access;
            }
            self.journal_input_dependency(path.as_path());
            Arc::make_mut(&mut self.input_dependencies).insert(updated.path.clone(), updated);
            return Ok(());
        }
        if self.input_dependency_len == MAX_INPUT_DEPENDENCIES {
            return Err(WorldError::new(
                "record input dependency",
                Some(path),
                format!("distinct input dependency limit {MAX_INPUT_DEPENDENCIES} exceeded"),
            ));
        }
        let path: Arc<Path> = Arc::from(path.into_boxed_path());
        self.journal_input_dependency(path.as_ref());
        Arc::make_mut(&mut self.input_dependencies).insert(
            Arc::clone(&path),
            InputDependency {
                path,
                outcome,
                access,
            },
        );
        self.input_dependency_len += 1;
        Ok(())
    }

    /// Enumerates reduced dependencies in canonical path order.
    pub fn input_dependencies(&self) -> impl Iterator<Item = InputDependency> {
        self.input_dependency_values().into_iter()
    }

    fn input_dependency(&self, path: &Path) -> Option<&InputDependency> {
        self.input_dependencies
            .get(path)
            .or_else(|| self.accepted_input_dependencies.as_ref()?.get(path))
    }

    fn input_dependency_values(&self) -> Vec<InputDependency> {
        let mut merged = BTreeMap::new();
        if let Some(accepted) = &self.accepted_input_dependencies {
            accepted.merge_into(&mut merged);
        }
        merged.extend(
            self.input_dependencies
                .iter()
                .map(|(path, value)| (Arc::clone(path), value.clone())),
        );
        merged.into_values().collect()
    }

    fn journal_input_dependency(&mut self, path: &Path) {
        let previous = self.input_dependencies.get(path).cloned();
        let path = self
            .input_dependencies
            .get_key_value(path)
            .map_or_else(|| Arc::from(path), |(path, _)| Arc::clone(path));
        Arc::make_mut(&mut self.input_dependency_journal).push((path, previous));
    }

    fn rollback_input_dependencies(&mut self, mark: usize) {
        if self.input_dependency_journal.len() == mark {
            return;
        }
        let journal = Arc::make_mut(&mut self.input_dependency_journal);
        for (path, previous) in journal[mark..].iter().rev() {
            match previous {
                Some(value) => {
                    Arc::make_mut(&mut self.input_dependencies)
                        .insert(Arc::clone(path), value.clone());
                }
                None => {
                    Arc::make_mut(&mut self.input_dependencies).remove(path.as_ref());
                }
            }
        }
        journal.truncate(mark);
    }

    /// Enumerates only immutable external dependencies, excluding files
    /// generated and reopened transactionally by this TeX run.
    pub fn external_input_records(&self) -> impl Iterator<Item = &InputRecord> {
        self.input_records()
            .iter()
            .filter(|record| record.is_external_dependency())
    }

    /// Verifies that every pinned included/font input still names the same
    /// host bytes before a retained checkpoint is reused.
    pub fn validate_recorded_inputs(&self) -> Result<(), WorldError> {
        for record in self.external_input_records() {
            let current = match &self.backend {
                WorldBackend::Real { .. } => std::fs::read(record.path()).map_err(|error| {
                    WorldError::new(
                        "validate retained input",
                        Some(record.path().to_owned()),
                        error.to_string(),
                    )
                })?,
                WorldBackend::Memory(memory) => memory
                    .files
                    .get(record.path())
                    .map(|bytes| bytes.to_vec())
                    .ok_or_else(|| {
                        WorldError::new(
                            "validate retained input",
                            Some(record.path().to_owned()),
                            "input is no longer available",
                        )
                    })?,
            };
            if ContentHash::from_bytes(&current) != record.hash() {
                return Err(WorldError::new(
                    "validate retained input",
                    Some(record.path().to_owned()),
                    "input content changed since the accepted checkpoint",
                ));
            }
        }
        Ok(())
    }

    /// Returns a recorded input only when `id` is live in this World timeline.
    #[must_use]
    pub fn input_record(&self, id: InputRecordId) -> Option<&InputRecord> {
        if !self.input_identities.contains(id.0) {
            return None;
        }
        self.input_records().get(id.raw() as usize)
    }

    /// Returns the content-addressed bytes for a previously-read input.
    #[must_use]
    pub fn input_content(&self, hash: ContentHash) -> Option<&[u8]> {
        self.input_contents
            .get(&hash)
            .map(AsRef::as_ref)
            .or_else(|| self.accepted_inputs.as_ref()?.content(hash))
    }

    fn input_content_root(&self, hash: ContentHash) -> Option<Arc<[u8]>> {
        self.input_contents
            .get(&hash)
            .cloned()
            .or_else(|| self.accepted_inputs.as_ref()?.content_root(hash))
    }

    #[must_use]
    pub fn shell_escape_records(&self) -> &[ShellEscapeRecord] {
        &self.shell_escapes
    }

    #[must_use]
    pub fn effect_pos(&self) -> EffectPos {
        EffectPos(self.effect_base.raw() + self.effects.len() as u64)
    }

    #[must_use]
    pub fn effect_records(&self) -> &[EffectRecord] {
        self.effects.as_slice()
    }

    /// Closes the live aligned effect columns into one validated in-session
    /// revision journal. Positional publication sidecars remain runtime-local;
    /// cold consumers detach the materialized records instead.
    #[must_use]
    pub fn effect_journal(&self) -> crate::EffectJournal {
        crate::EffectJournal::from_parts(
            self.effects.as_ref().clone(),
            self.effect_sequences.as_ref().clone(),
            self.effect_publications.as_ref().clone(),
            self.effect_publication_record_ordinals.as_ref().clone(),
            self.effect_domains.as_ref().clone(),
            self.effect_semantic_record_ordinals.as_ref().clone(),
            self.effect_placement_intra_orders.as_ref().clone(),
        )
        .expect("World effect columns are aligned")
    }

    /// Detaches canonical effect values and their optional rendered
    /// stream-open contexts in one ordinal-aligned projection.
    ///
    /// The contexts are already-owned diagnostic text. Runtime positions and
    /// publication sidecars remain inside this World.
    #[doc(hidden)]
    #[must_use]
    pub fn detached_effect_records(&self) -> (Vec<EffectRecord>, Vec<Option<String>>) {
        let journal = self.effect_journal();
        let indices = journal.materialized_record_indices();
        let mut records = Vec::with_capacity(indices.len());
        let mut contexts = Vec::with_capacity(indices.len());
        for index in indices {
            let record = self.effects[index].clone();
            let context = if matches!(record, EffectRecord::StreamOpen { .. }) {
                self.effect_position(index)
                    .and_then(|position| self.stream_open_contexts.get(&position).cloned())
            } else {
                None
            };
            records.push(record);
            contexts.push(context);
        }
        (records, contexts)
    }

    /// Detaches the complete accepted prefix followed by the live private
    /// suffix. Historical blocks are materialized only at this terminal cold
    /// boundary; checkpoint capture, fork, restore, and mutation keep their
    /// immutable owner roots.
    #[doc(hidden)]
    #[must_use]
    pub fn detached_complete_effect_records(&self) -> (Vec<EffectRecord>, Vec<Option<String>>) {
        let mut records = Vec::with_capacity(
            self.page_effect_prefix_len()
                .saturating_add(self.effects.len()),
        );
        let mut contexts = Vec::with_capacity(records.capacity());
        if let Some(accepted) = &self.accepted_effects {
            accepted.append_detached_records(&mut records, &mut contexts);
        }
        let (mut suffix, mut suffix_contexts) = self.detached_effect_records();
        records.append(&mut suffix);
        contexts.append(&mut suffix_contexts);
        (records, contexts)
    }

    /// Reinstalls one validated in-session journal's aligned runtime sidecars.
    pub fn install_effect_journal(&mut self, journal: &crate::EffectJournal) {
        self.install_effect_sequences(journal.sequences());
        self.install_effect_publications(journal.publications());
        self.install_effect_publication_record_ordinals(journal.publication_record_ordinals());
        self.install_effect_domains(journal.domains());
        self.install_effect_semantic_record_ordinals(journal.semantic_record_ordinals());
        self.install_effect_placement_intra_orders(journal.placement_intra_orders());
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_sequences(&self) -> Arc<Vec<EffectSequence>> {
        Arc::clone(&self.effect_sequences)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_publications(&self) -> Arc<Vec<Option<EffectPublicationId>>> {
        Arc::clone(&self.effect_publications)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_publication_record_ordinals(
        &self,
    ) -> Arc<Vec<Option<EffectPublicationRecordOrdinal>>> {
        Arc::clone(&self.effect_publication_record_ordinals)
    }

    /// Returns winner decisions made at completed semantic-effect commits.
    #[doc(hidden)]
    #[must_use]
    pub fn effect_publication_dispositions(&self) -> Arc<Vec<EffectPublicationDisposition>> {
        Arc::clone(&self.effect_publication_dispositions)
    }

    /// Commits the live publication as the semantic winner over the retained
    /// publication. This ledger deliberately does not describe artifact
    /// selection: artifact and effect transactions can choose differently.
    #[doc(hidden)]
    pub fn commit_effect_publication_winner(
        &mut self,
        rejected: Option<EffectPublicationId>,
        winner: EffectPublicationId,
        output_attempt: EffectOutputAttemptId,
        recursive_receipt: Option<PageOutputPublicationReceiptId>,
    ) {
        Arc::make_mut(&mut self.effect_publication_dispositions).push(
            EffectPublicationDisposition::new(rejected, winner, output_attempt, recursive_receipt),
        );
    }

    #[doc(hidden)]
    pub fn claim_effect_publication(
        &mut self,
        range: std::ops::Range<usize>,
        publication: EffectPublicationId,
    ) {
        let start = range.start.min(self.effect_publications.len());
        let end = range.end.min(self.effect_publications.len());
        let mut next = self.publication_counter(publication);
        self.journal_publication_counter(publication);
        let publications = Arc::make_mut(&mut self.effect_publications);
        let ordinals = Arc::make_mut(&mut self.effect_publication_record_ordinals);
        for index in start..end {
            if publications[index] == Some(publication) && ordinals[index].is_some() {
                continue;
            }
            publications[index] = Some(publication);
            next = next
                .checked_add(1)
                .expect("effect publication record ordinal exhausted");
            ordinals[index] = Some(EffectPublicationRecordOrdinal::new(next));
        }
        Arc::make_mut(&mut self.next_effect_publication_record_ordinals).insert(publication, next);
    }

    /// Reserves a stable identity in the effect-publication ledger.
    #[doc(hidden)]
    pub fn reserve_effect_publication(&mut self) -> EffectPublicationId {
        if let Some(publication) = self.active_effect_publication {
            return publication;
        }
        self.next_effect_publication_identity = self
            .next_effect_publication_identity
            .checked_add(1)
            .expect("effect publication identity exhausted");
        EffectPublicationId::new(self.next_effect_publication_identity)
    }

    #[doc(hidden)]
    pub fn extend_previous_effect_publication(&mut self, range: std::ops::Range<usize>) {
        let previous = self.effect_publications[..range.start.min(self.effect_publications.len())]
            .iter()
            .rev()
            .copied()
            .flatten()
            .next();
        if let Some(previous) = previous {
            self.claim_effect_publication(range, previous);
        }
    }

    #[doc(hidden)]
    pub fn claim_effect_publication_boundary(
        &mut self,
        range: std::ops::Range<usize>,
        source: usize,
        right: EffectPublicationId,
        output_attempt: EffectOutputAttemptId,
    ) {
        let Some(sequence) = self.effect_sequences.get(source).copied() else {
            return;
        };
        let start = range.start.min(self.effect_sequences.len());
        let end = range.end.min(self.effect_sequences.len());
        let left = self.effect_publications[..start]
            .iter()
            .rev()
            .copied()
            .flatten()
            .next();
        Arc::make_mut(&mut self.effect_sequences)[start..end].fill(sequence);
        let domain = EffectDomain::PublicationBoundary {
            left,
            right: Some(right),
            output_attempt,
        };
        Arc::make_mut(&mut self.effect_domains)[start..end].fill(domain);
        // This operation defines the complete typed record set for one
        // publication gap. A checkpoint may already contain an earlier
        // execution of the same claim, but that retained counter is not part
        // of the claim's semantic identity. Restart its local namespace so
        // replay reproduces the same per-record identities.
        self.journal_semantic_counter(domain);
        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals).insert(domain, 0);
        let ordinals = (start..end)
            .map(|_| self.allocate_effect_semantic_record_ordinal(domain))
            .collect::<Vec<_>>();
        Arc::make_mut(&mut self.effect_semantic_record_ordinals)[start..end]
            .copy_from_slice(&ordinals);
    }

    #[doc(hidden)]
    pub fn install_effect_publications(&mut self, publications: &[Option<EffectPublicationId>]) {
        let mut installed = publications[..publications.len().min(self.effects.len())].to_vec();
        installed.resize(self.effects.len(), None);
        self.next_effect_publication_identity = self.next_effect_publication_identity.max(
            installed
                .iter()
                .flatten()
                .map(|publication| publication.0)
                .max()
                .unwrap_or(0),
        );
        self.effect_publications = Arc::new(installed);
    }

    #[doc(hidden)]
    pub fn install_effect_publication_record_ordinals(
        &mut self,
        ordinals: &[Option<EffectPublicationRecordOrdinal>],
    ) {
        let mut installed = ordinals[..ordinals.len().min(self.effects.len())].to_vec();
        installed.resize(self.effects.len(), None);
        let existing = self
            .next_effect_publication_record_ordinals
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for key in existing {
            self.journal_publication_counter(key);
        }
        Arc::make_mut(&mut self.next_effect_publication_record_ordinals).clear();
        for (publication, ordinal) in self
            .effect_publications
            .iter()
            .copied()
            .zip(installed.iter().copied())
        {
            if let (Some(publication), Some(EffectPublicationRecordOrdinal(ordinal))) =
                (publication, ordinal)
            {
                Arc::make_mut(&mut self.next_effect_publication_record_ordinals)
                    .entry(publication)
                    .and_modify(|next| *next = (*next).max(ordinal))
                    .or_insert(ordinal);
            }
        }
        self.effect_publication_record_ordinals = Arc::new(installed);
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_domains(&self) -> Arc<Vec<EffectDomain>> {
        Arc::clone(&self.effect_domains)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_semantic_record_ordinals(&self) -> Arc<Vec<EffectSemanticRecordOrdinal>> {
        Arc::clone(&self.effect_semantic_record_ordinals)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_placement_intra_orders(&self) -> Arc<Vec<EffectPlacementIntraOrder>> {
        Arc::clone(&self.effect_placement_intra_orders)
    }

    #[doc(hidden)]
    pub fn install_effect_placement_intra_orders(&mut self, orders: &[EffectPlacementIntraOrder]) {
        let mut installed = orders[..orders.len().min(self.effects.len())].to_vec();
        while installed.len() < self.effects.len() {
            installed.push(self.allocate_effect_placement_intra_order());
        }
        self.next_effect_placement_intra_order =
            installed.iter().map(|order| order.0).max().unwrap_or(0);
        self.effect_placement_intra_orders = Arc::new(installed);
    }

    #[doc(hidden)]
    pub fn install_effect_semantic_record_ordinals(
        &mut self,
        ordinals: &[EffectSemanticRecordOrdinal],
    ) {
        let mut installed = ordinals[..ordinals.len().min(self.effects.len())].to_vec();
        for index in installed.len()..self.effects.len() {
            let domain = self.effect_domains[index];
            installed.push(self.allocate_effect_semantic_record_ordinal(domain));
        }
        let existing = self
            .next_effect_semantic_record_ordinals
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for key in existing {
            self.journal_semantic_counter(key);
        }
        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals).clear();
        for (&domain, &ordinal) in self.effect_domains.iter().zip(&installed) {
            let domain = match domain {
                EffectDomain::World(_) => EffectDomain::World(0),
                // A publication boundary is a typed claim over the records
                // between two publication identities.  Replaying that same
                // claim must reproduce its claim-local ordinals rather than
                // continue after the accepted copy installed above.  A
                // genuinely different boundary has a different `{left,
                // right}` domain, while distinct records in this claim are
                // still numbered independently by the claiming operation.
                EffectDomain::PublicationBoundary { .. } => continue,
                domain => domain,
            };
            Arc::make_mut(&mut self.next_effect_semantic_record_ordinals)
                .entry(domain)
                .and_modify(|next| *next = (*next).max(ordinal.0))
                .or_insert(ordinal.0);
        }
        self.effect_semantic_record_ordinals = Arc::new(installed);
    }

    #[doc(hidden)]
    pub fn install_effect_domains(&mut self, domains: &[EffectDomain]) {
        let mut installed = domains[..domains.len().min(self.effects.len())].to_vec();
        while installed.len() < self.effects.len() {
            installed.push(self.allocate_effect_domain());
        }
        self.next_publication_sequence = self.next_publication_sequence.max(
            self.effect_sequences
                .iter()
                .zip(&installed)
                .filter_map(|(sequence, domain)| {
                    matches!(domain, EffectDomain::TerminalPublication { .. }).then_some(sequence.0)
                })
                .max()
                .unwrap_or(0),
        );
        self.effect_domains = Arc::new(installed);
    }

    #[doc(hidden)]
    pub fn install_effect_sequences(&mut self, sequences: &[EffectSequence]) {
        let mut installed = sequences[..sequences.len().min(self.effects.len())].to_vec();
        for _ in installed.len()..self.effects.len() {
            installed.push(self.allocate_effect_sequence());
        }
        self.next_effect_sequence = self.next_effect_sequence.max(
            installed
                .iter()
                .map(|sequence| sequence.0)
                .max()
                .unwrap_or(0),
        );
        self.next_publication_sequence = self.next_publication_sequence.max(
            installed
                .iter()
                .map(|sequence| sequence.0)
                .max()
                .unwrap_or(0),
        );
        self.effect_sequences = Arc::new(installed);
    }

    #[doc(hidden)]
    #[must_use]
    pub fn effect_root_identity(&self) -> EffectRootIdentity {
        effect_root_identity_for(&self.effects)
    }

    /// Number of effects accepted before this revision's private suffix.
    #[doc(hidden)]
    #[must_use]
    pub fn page_effect_prefix_len(&self) -> usize {
        self.accepted_effects
            .as_ref()
            .map_or(0, |block| block.total_len)
    }

    /// Visits the page-visible effect interval in canonical prefix order.
    ///
    /// Building the short block spine is confined to shipout, where effects
    /// cross into an artifact-owned value. Named checkpoint capture, clone,
    /// restore, and fork never materialize or concatenate accepted blocks.
    #[doc(hidden)]
    pub fn visit_pending_page_effects(
        &self,
        pending_live_end: usize,
        mut visit: impl FnMut(usize, &EffectRecord),
    ) {
        let pending = self.pending_page_effect_range(pending_live_end);
        let mut blocks = Vec::new();
        let mut block = self.accepted_effects.as_deref();
        while let Some(current) = block {
            blocks.push(current);
            block = current.parent.as_deref();
        }
        let mut index = 0;
        for block in blocks.into_iter().rev() {
            for record in block.effects[..block.len].iter() {
                if pending.contains(&index) {
                    visit(index, record);
                }
                index += 1;
            }
        }
        for record in self.effects[..pending_live_end.min(self.effects.len())].iter() {
            if pending.contains(&index) {
                visit(index, record);
            }
            index += 1;
        }
    }

    /// Prefix-or-live indices not yet embedded in a committed page, bounded
    /// by the caller's pre-shipout live-effect end.
    #[doc(hidden)]
    #[must_use]
    pub fn pending_page_effect_range(&self, pending_live_end: usize) -> std::ops::Range<usize> {
        let end = self
            .page_effect_prefix_len()
            .saturating_add(pending_live_end.min(self.effects.len()));
        self.page_effect_artifact_cursor.min(end)..end
    }

    /// Closes the page-visible effect interval after an artifact commit.
    #[doc(hidden)]
    pub fn finish_page_effect_interval(&mut self) {
        self.page_effect_artifact_cursor = self
            .page_effect_prefix_len()
            .saturating_add(self.effects.len());
    }

    fn drain_page_effect_interval_prefix(&mut self, count: usize) {
        let prefix = self.page_effect_prefix_len();
        if self.page_effect_artifact_cursor > prefix {
            self.page_effect_artifact_cursor = prefix
                + self
                    .page_effect_artifact_cursor
                    .saturating_sub(prefix)
                    .saturating_sub(count);
        }
    }

    /// Absolute position of a page-visible prefix-or-live effect.
    #[doc(hidden)]
    #[must_use]
    pub fn page_effect_position(&self, index: usize) -> Option<EffectPos> {
        let len = self
            .page_effect_prefix_len()
            .checked_add(self.effects.len())?;
        (index < len).then(|| {
            EffectPos::from_raw(u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1))
        })
    }

    /// Absolute append-only identity of one currently retained effect.
    #[must_use]
    pub fn effect_position(&self, index: usize) -> Option<EffectPos> {
        (index < self.effects.len()).then(|| {
            EffectPos(self.effect_base.raw() + u64::try_from(index).unwrap_or(u64::MAX) + 1)
        })
    }

    /// Retargets the first pending stream-open after an authoritative,
    /// retry-safe failure.
    ///
    /// Earlier effects have already been drained by [`Self::commit_effects`];
    /// the failed open and its following suffix remain ordered and untouched.
    /// TeX82 §1374 changes only the failed open's filename before retrying.
    pub fn retarget_pending_stream_open(
        &mut self,
        failed: &StreamOpenFailure,
        replacement: impl Into<PathBuf>,
    ) -> Result<(), WorldError> {
        let replacement = replacement.into();
        let next_effect_position = self.effect_base.0 + 1;
        let slot = {
            let Some(EffectRecord::StreamOpen { slot, target }) = self.effects_mut().first_mut()
            else {
                return Err(WorldError::new(
                    "retarget stream open",
                    Some(failed.path.clone()),
                    "the pending effect prefix does not begin with a stream open",
                ));
            };
            if next_effect_position != failed.position.0
                || *slot != failed.slot
                || target.path != failed.path
            {
                return Err(WorldError::new(
                    "retarget stream open",
                    Some(failed.path.clone()),
                    "the pending stream open identity, slot, or target is stale",
                ));
            }
            target.path = replacement.clone();
            *slot
        };
        if let Some(live) = self.stream_bufs_mut().write_streams[slot.index()].as_mut() {
            live.path = replacement;
        }
        Ok(())
    }

    /// Opens a rollback-capable editor branch before any host-visible effect commits.
    pub(crate) fn begin_retained_session(&mut self) -> Result<(), WorldError> {
        if self.shell_escape_policy == ShellEscapePolicy::Enabled {
            return Err(WorldError::new(
                "begin retained session",
                None,
                "shell escape must be disabled for rollback-capable editor sessions",
            ));
        }
        if self.effect_base != EffectPos::default() {
            return Err(WorldError::new(
                "begin retained session",
                None,
                "host effects were already materialized on this timeline",
            ));
        }
        self.commit_mode = WorldCommitMode::Retained;
        Ok(())
    }

    #[must_use]
    pub const fn commit_mode(&self) -> WorldCommitMode {
        self.commit_mode
    }

    /// Selects the destination backend for a retained session's eventual
    /// effects without exposing that backend during engine execution.
    pub fn retarget_output_backend(&mut self, destination: &World) -> Result<(), WorldError> {
        if self.commit_mode != WorldCommitMode::Retained {
            return Err(WorldError::new(
                "retarget output backend",
                None,
                "world is not an unexported retained session",
            ));
        }
        if destination.effect_pos() != EffectPos::default() {
            return Err(WorldError::new(
                "retarget output backend",
                None,
                "destination world already contains effects",
            ));
        }
        self.backend = destination.backend.clone();
        Ok(())
    }

    /// Materializes a retained branch once, in order, and seals it against rollback.
    pub(crate) fn export_retained_effects(&mut self) -> Result<(), WorldError> {
        if self.commit_mode != WorldCommitMode::Retained {
            return Err(WorldError::new(
                "export retained session",
                None,
                "world is not an unexported retained session",
            ));
        }
        let end = self.effect_pos();
        self.commit_mode = WorldCommitMode::Eager;
        if let Err(error) = self.commit_effects(end) {
            self.commit_mode = WorldCommitMode::Retained;
            return Err(error);
        }
        self.commit_mode = WorldCommitMode::Exported;
        Ok(())
    }

    #[must_use]
    pub fn memory_output(&self, path: impl AsRef<Path>) -> Option<&[u8]> {
        let WorldBackend::Memory(memory) = &self.backend else {
            return None;
        };
        memory.outputs.get(path.as_ref()).map(Vec::as_slice)
    }

    /// Enumerates every materialized memory output in deterministic path order.
    ///
    /// Seeded input files are not outputs and are therefore absent. The
    /// iterator borrows immutable entries and offers no access to the backing
    /// map or to effect commit/rollback operations.
    pub fn memory_outputs(&self) -> Option<impl ExactSizeIterator<Item = MemoryOutput<'_>> + '_> {
        let WorldBackend::Memory(memory) = &self.backend else {
            return None;
        };
        Some(memory.outputs.iter().map(|(path, bytes)| MemoryOutput {
            path,
            bytes: bytes.as_slice(),
        }))
    }

    #[must_use]
    pub fn memory_terminal_output(&self) -> Option<&[u8]> {
        let WorldBackend::Memory(memory) = &self.backend else {
            return None;
        };
        Some(&memory.terminal_output)
    }

    #[must_use]
    pub fn memory_log_output(&self) -> Option<&[u8]> {
        let WorldBackend::Memory(memory) = &self.backend else {
            return None;
        };
        Some(&memory.log_output)
    }

    /// Captures the already materialized memory prefix before a host retry.
    #[doc(hidden)]
    #[must_use]
    pub fn memory_materialization_checkpoint(&self) -> Option<MemoryMaterializationCheckpoint> {
        let WorldBackend::Memory(memory) = &self.backend else {
            return None;
        };
        Some(MemoryMaterializationCheckpoint(Arc::clone(memory)))
    }

    /// Removes a replayed suffix that was already materialized by the
    /// suspended attempt, while retaining non-replayed output from that
    /// attempt and all new output following the replay.
    #[doc(hidden)]
    pub fn reconcile_memory_retry_materialization(
        &mut self,
        checkpoint: &MemoryMaterializationCheckpoint,
    ) -> bool {
        let WorldBackend::Memory(current) = &mut self.backend else {
            return false;
        };
        let current = Arc::make_mut(current);
        let mut reconciled =
            deduplicate_retry_suffix(&checkpoint.0.terminal_output, &mut current.terminal_output);
        reconciled |= deduplicate_retry_suffix(&checkpoint.0.log_output, &mut current.log_output);
        for (path, before) in &checkpoint.0.outputs {
            if let Some(after) = current.outputs.get_mut(path) {
                reconciled |= deduplicate_retry_suffix(before, after);
            }
        }
        reconciled
    }

    #[must_use]
    pub fn stream_bufs(&self) -> &StreamBufState {
        &self.stream_bufs
    }

    /// Stable request identity used for tracked input resources.
    #[must_use]
    pub fn input_resource_dependency_identity(path: impl AsRef<Path>) -> u64 {
        StateHashFragment::from_exact_builder(0x776f_726c_645f_7271, |hash| {
            hash.bytes(path.as_ref().as_os_str().as_encoded_bytes());
        })
        .fingerprint()
    }

    fn stream_bufs_mut(&mut self) -> StreamBufIdentityGuard<'_> {
        let old = self
            .reachable_state_identity
            .as_ref()
            .map(|_| stable_hash(self.stream_bufs.as_ref()));
        StreamBufIdentityGuard {
            bufs: &mut self.stream_bufs,
            identity: self.reachable_state_identity.as_mut(),
            old,
        }
    }

    #[must_use]
    pub const fn rng_state(&self) -> RngState {
        self.rng
    }

    #[must_use]
    pub(crate) fn snapshot(&self) -> WorldSnapshot {
        assert!(
            self.provisional_page_output_receipts.is_empty(),
            "a provisional page-output receipt crossed a World checkpoint"
        );
        WorldSnapshot {
            effect_base: self.effect_base,
            page_effect_artifact_cursor: self.page_effect_artifact_cursor,
            effect_len: self.effects.len(),
            effect_publication_disposition_len: self.effect_publication_dispositions.len(),
            next_effect_sequence: self.next_effect_sequence,
            next_publication_sequence: self.next_publication_sequence,
            next_effect_publication_identity: self.next_effect_publication_identity,
            effect_counter_journal_len: self.effect_counter_journal.len(),
            next_effect_domain: self.next_effect_domain,
            next_effect_output_attempt_identity: self.next_effect_output_attempt_identity,
            next_effect_placement_intra_order: self.next_effect_placement_intra_order,
            next_terminal_publication_identity: self.next_terminal_publication_identity,
            effect_pos: self.effect_pos(),
            stream_bufs: self.stream_bufs.clone(),
            rng: self.rng,
            pdf_rng: self.pdf_rng.clone(),
            pdf_time_micros: self.pdf_time_micros,
            pdf_timer_origin_micros: self.pdf_timer_origin_micros,
            job_clock: self.job_clock,
            shell_escape_policy: self.shell_escape_policy,
            input_len: self.inputs.len(),
            input_identities: self.input_identities.watermark(),
            input_dependency_journal_len: self.input_dependency_journal.len(),
            input_dependency_len: self.input_dependency_len,
            shell_escape_len: self.shell_escapes.len(),
            artifact_base: self.artifact_base,
            artifact_commit_len: self.artifact_pos(),
            next_artifact_publication_identity: self.next_artifact_publication_identity,
            active_artifact_publication_group: self.active_artifact_publication_group,
            active_terminal_publication: self.active_terminal_publication,
            commit_mode: self.commit_mode,
            file_framing: self.file_framing,
            error_channel: self.error_channel.clone(),
            reachable_state_identity: self.reachable_state_identity,
        }
    }

    pub(crate) fn enable_reachable_state_identity(&mut self) -> bool {
        if self.reachable_state_identity.is_some() {
            return true;
        }
        if self.effect_pos() != EffectPos::default()
            || !self.inputs.is_empty()
            || self.artifact_pos() != 0
            || !self.shell_escapes.is_empty()
        {
            return false;
        }
        self.reachable_state_identity = Some(WorldReachableStateIdentity::new(self));
        true
    }

    pub(crate) fn reachable_state_identity_root(&self) -> Option<u64> {
        self.reachable_state_identity.map(|root| {
            crate::state_hash::semantic_scalar_root(0x776f_726c_645f_6669, |hasher| {
                hasher.u64(root.root());
                hasher.u32(self.file_framing.open_parens());
                hasher.u64(self.error_channel.reachable_state_identity());
            })
        })
    }

    fn replace_identity_scalar(&mut self, key: u64, old: u64, new: u64) {
        if let Some(identity) = &mut self.reachable_state_identity {
            identity.scalars.replace(key, Some(old), Some(new));
        }
    }

    fn record_input_identity(&mut self) {
        let record = self.inputs.last().expect("input record was just published");
        if let Some(identity) = &mut self.reachable_state_identity {
            identity.inputs.push(stable_hash(record));
        }
    }

    fn record_artifact_identity(&mut self, hash: ContentHash) {
        if let Some(identity) = &mut self.reachable_state_identity {
            identity.artifacts.push(stable_hash(&hash));
        }
    }

    pub(crate) fn assert_snapshot_retained(&self, snapshot: &WorldSnapshot) {
        assert!(
            self.snapshot_effects_are_retained(snapshot)
                && (self.artifact_base..=self.artifact_pos())
                    .contains(&snapshot.artifact_commit_len),
            "World snapshot output position has already been committed and dropped"
        );
    }

    #[must_use]
    pub(crate) fn snapshot_is_retained(&self, snapshot: &WorldSnapshot) -> bool {
        self.snapshot_effects_are_retained(snapshot)
            && (self.artifact_base..=self.artifact_pos()).contains(&snapshot.artifact_commit_len)
    }

    /// Whether a strongly owned checkpoint root can seed a new retained
    /// generation after the source timeline has published its prefix.
    #[must_use]
    pub(crate) fn snapshot_is_forkable(&self, snapshot: &WorldSnapshot) -> bool {
        snapshot.effect_pos == EffectPos(snapshot.effect_base.raw() + snapshot.effect_len as u64)
    }

    fn snapshot_effects_are_retained(&self, snapshot: &WorldSnapshot) -> bool {
        snapshot.effect_pos >= self.effect_base
            && snapshot.effect_pos
                == EffectPos(snapshot.effect_base.raw() + snapshot.effect_len as u64)
            && snapshot.effect_len <= self.effects.len()
    }

    pub(crate) fn rollback(&mut self, snapshot: &WorldSnapshot) {
        self.assert_snapshot_retained(snapshot);
        self.input_identities
            .rollback(snapshot.input_identities)
            .expect("World input identity mark must name a retained ancestor");
        self.effect_base = snapshot.effect_base;
        self.page_effect_artifact_cursor = snapshot.page_effect_artifact_cursor;
        if self.effects.len() != snapshot.effect_len {
            Arc::make_mut(&mut self.effects).truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_sequences).truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_publications).truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_publication_record_ordinals)
                .truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_domains).truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_semantic_record_ordinals).truncate(snapshot.effect_len);
            Arc::make_mut(&mut self.effect_placement_intra_orders).truncate(snapshot.effect_len);
        }
        if self.effect_publication_dispositions.len() != snapshot.effect_publication_disposition_len
        {
            Arc::make_mut(&mut self.effect_publication_dispositions)
                .truncate(snapshot.effect_publication_disposition_len);
        }
        self.next_effect_sequence = snapshot.next_effect_sequence;
        self.next_publication_sequence = snapshot.next_publication_sequence;
        self.next_effect_publication_identity = snapshot.next_effect_publication_identity;
        self.rollback_effect_counters(snapshot.effect_counter_journal_len);
        self.next_effect_domain = snapshot.next_effect_domain;
        self.next_effect_output_attempt_identity = snapshot.next_effect_output_attempt_identity;
        self.next_effect_placement_intra_order = snapshot.next_effect_placement_intra_order;
        self.next_terminal_publication_identity = snapshot.next_terminal_publication_identity;
        self.next_artifact_publication_identity = snapshot.next_artifact_publication_identity;
        if !self.provisional_page_output_receipts.is_empty() {
            Arc::make_mut(&mut self.provisional_page_output_receipts).clear();
        }
        self.active_artifact_publication_group = snapshot.active_artifact_publication_group;
        self.active_terminal_publication = snapshot.active_terminal_publication;
        Arc::make_mut(&mut self.stream_open_contexts)
            .retain(|position, _| *position <= snapshot.effect_pos);
        self.stream_bufs = snapshot.stream_bufs.clone();
        self.rng = snapshot.rng;
        self.pdf_rng = snapshot.pdf_rng.clone();
        self.pdf_time_micros = snapshot.pdf_time_micros;
        self.pdf_timer_origin_micros = snapshot.pdf_timer_origin_micros;
        self.shell_escape_policy = snapshot.shell_escape_policy;
        if self.inputs.len() != snapshot.input_len {
            Arc::make_mut(&mut self.inputs).truncate(snapshot.input_len);
        }
        self.rollback_input_dependencies(snapshot.input_dependency_journal_len);
        self.input_dependency_len = snapshot.input_dependency_len;
        self.shell_escapes.truncate(snapshot.shell_escape_len);
        if snapshot.commit_mode == WorldCommitMode::Retained {
            let retained = snapshot
                .artifact_commit_len
                .checked_sub(self.artifact_base)
                .expect("World artifact snapshot precedes retained base");
            if self.artifact_commits.len() != retained {
                Arc::make_mut(&mut self.artifact_commits).truncate(retained);
                Arc::make_mut(&mut self.committed_artifacts).truncate(retained);
                Arc::make_mut(&mut self.artifact_publications).truncate(retained);
            }
        }
        self.commit_mode = snapshot.commit_mode;
        self.file_framing = snapshot.file_framing;
        self.error_channel = snapshot.error_channel.clone();
        self.reachable_state_identity = snapshot.reachable_state_identity;
    }

    /// Installs a retained checkpoint into a new generation. Accepted effects
    /// become an immutable page-visible prefix, while the destination starts
    /// a fresh publishable suffix at the same absolute semantic position.
    fn install_checkpoint_fork(&mut self, source: &Self, snapshot: &WorldSnapshot) {
        assert!(source.snapshot_is_forkable(snapshot));
        self.input_identities = source
            .input_identities
            .fork_at(snapshot.input_identities)
            .expect("World input identity mark must name a retained ancestor");

        let accepted_len = self.page_effect_prefix_len();
        assert_eq!(
            accepted_len as u64,
            snapshot.effect_base.raw(),
            "accepted effect blocks align with the source live suffix"
        );
        self.accepted_effects =
            AcceptedEffectBlock::extend(source.accepted_effects.clone(), source, snapshot);
        assert_eq!(
            self.page_effect_prefix_len() as u64,
            snapshot.effect_pos.raw()
        );
        self.page_effect_artifact_cursor = snapshot.page_effect_artifact_cursor;

        self.effect_base = snapshot.effect_pos;
        self.effects = Arc::new(Vec::new());
        self.effect_sequences = Arc::new(Vec::new());
        self.effect_publications = Arc::new(Vec::new());
        self.effect_publication_record_ordinals = Arc::new(Vec::new());
        self.effect_domains = Arc::new(Vec::new());
        self.effect_semantic_record_ordinals = Arc::new(Vec::new());
        self.effect_placement_intra_orders = Arc::new(Vec::new());
        self.active_effect_publication = None;
        self.active_effect_output_attempt = None;
        self.active_effect_domain = None;
        self.provisional_page_output_receipts = Arc::new(BTreeMap::new());
        self.next_terminal_publication_identity = self
            .next_terminal_publication_identity
            .max(snapshot.next_terminal_publication_identity);
        self.next_artifact_publication_identity = self
            .next_artifact_publication_identity
            .max(snapshot.next_artifact_publication_identity);
        self.active_artifact_publication_group = None;
        self.active_terminal_publication = None;
        self.stream_open_contexts = Arc::new(BTreeMap::new());
        self.next_effect_sequence = snapshot.next_effect_sequence;
        self.next_effect_publication_record_ordinals = Arc::new(BTreeMap::new());
        self.next_effect_semantic_record_ordinals = Arc::new(BTreeMap::new());
        self.effect_counter_journal = Arc::new(Vec::new());
        self.next_publication_sequence = self
            .next_publication_sequence
            .max(snapshot.next_publication_sequence);
        self.next_effect_publication_identity = self
            .next_effect_publication_identity
            .max(snapshot.next_effect_publication_identity);
        self.next_effect_domain = snapshot.next_effect_domain;
        self.next_effect_placement_intra_order = snapshot.next_effect_placement_intra_order;
        self.stream_bufs = snapshot.stream_bufs.clone();
        self.rng = snapshot.rng;
        self.pdf_rng = snapshot.pdf_rng.clone();
        self.pdf_time_micros = snapshot.pdf_time_micros;
        self.pdf_timer_origin_micros = snapshot.pdf_timer_origin_micros;
        self.shell_escape_policy = snapshot.shell_escape_policy;
        self.accepted_inputs =
            AcceptedInputBlock::extend(source.accepted_inputs.clone(), source, snapshot.input_len);
        self.inputs = Arc::new(Vec::new());
        self.input_contents = Arc::new(BTreeMap::new());
        self.accepted_input_dependencies = Some(Arc::new(AcceptedInputDependencyBlock {
            parent: source.accepted_input_dependencies.clone(),
            values: Arc::clone(&source.input_dependencies),
            journal: Arc::clone(&source.input_dependency_journal),
            journal_len: snapshot.input_dependency_journal_len,
        }));
        self.input_dependencies = Arc::new(BTreeMap::new());
        self.input_dependency_journal = Arc::new(Vec::new());
        self.input_dependency_len = snapshot.input_dependency_len;
        self.shell_escapes.truncate(snapshot.shell_escape_len);
        if snapshot.commit_mode == WorldCommitMode::Retained {
            self.artifact_base = snapshot.artifact_commit_len;
            self.artifact_commits = Arc::new(Vec::new());
            self.committed_artifacts = Arc::new(Vec::new());
            self.artifact_publications = Arc::new(Vec::new());
        }
        self.commit_mode = snapshot.commit_mode;
        self.file_framing = snapshot.file_framing;
        self.error_channel = snapshot.error_channel.clone();
        self.reachable_state_identity = snapshot.reachable_state_identity;
    }

    /// Builds one isolated revision suffix from a retained mark without
    /// copying the accepted effect ledger.
    pub(crate) fn fork_checkpoint(&self, snapshot: &WorldSnapshot) -> Self {
        assert!(self.snapshot_is_forkable(snapshot));
        let mut fork = self.clone();
        fork.install_checkpoint_fork(self, snapshot);
        fork
    }

    fn allocate_input_record(&mut self) -> InputRecordId {
        let identity = self
            .input_identities
            .allocate()
            .expect("World input record identity capacity exhausted");
        assert_eq!(
            identity.slot() as usize,
            self.accepted_input_len().saturating_add(self.inputs.len()),
            "World input identities and records diverged"
        );
        InputRecordId(identity)
    }

    fn append_effect(&mut self, record: EffectRecord) {
        if let Some(identity) = &mut self.reachable_state_identity {
            identity.effects.push(stable_hash(&record));
        }
        self.effects_mut().push(record);
        let terminal = self.active_terminal_publication;
        let sequence = if let Some(publication) = terminal {
            publication.sequence
        } else {
            self.allocate_effect_sequence()
        };
        Arc::make_mut(&mut self.effect_sequences).push(sequence);
        Arc::make_mut(&mut self.effect_publications).push(self.active_effect_publication);
        Arc::make_mut(&mut self.effect_publication_record_ordinals).push(None);
        let domain = if let Some(publication) = terminal {
            self.active_terminal_publication
                .as_mut()
                .expect("terminal publication transaction is active")
                .next_intra_order = publication
                .next_intra_order
                .checked_add(1)
                .expect("terminal publication intra-order exhausted");
            EffectDomain::TerminalPublication {
                identity: publication.identity,
                phase: publication.phase,
                intra_order: publication.next_intra_order,
                committed: false,
            }
        } else {
            self.active_effect_domain
                .unwrap_or_else(|| self.allocate_effect_domain())
        };
        Arc::make_mut(&mut self.effect_domains).push(domain);
        let ordinal = self.allocate_effect_semantic_record_ordinal(domain);
        Arc::make_mut(&mut self.effect_semantic_record_ordinals).push(ordinal);
        let placement = self.allocate_effect_placement_intra_order();
        Arc::make_mut(&mut self.effect_placement_intra_orders).push(placement);
    }

    #[doc(hidden)]
    pub fn set_active_effect_publication(&mut self, publication: Option<EffectPublicationId>) {
        self.active_effect_publication = publication;
    }

    #[doc(hidden)]
    #[must_use]
    pub const fn active_effect_output_attempt(&self) -> Option<EffectOutputAttemptId> {
        self.active_effect_output_attempt
    }

    #[doc(hidden)]
    pub fn set_active_effect_output_attempt(&mut self, attempt: Option<EffectOutputAttemptId>) {
        self.active_effect_output_attempt = attempt;
    }

    #[doc(hidden)]
    pub fn allocate_effect_output_attempt(&mut self) -> EffectOutputAttemptId {
        self.next_effect_output_attempt_identity = self
            .next_effect_output_attempt_identity
            .checked_add(1)
            .expect("effect output-attempt identity exhausted");
        EffectOutputAttemptId::new(self.next_effect_output_attempt_identity)
    }

    #[doc(hidden)]
    pub fn set_active_effect_domain(&mut self, domain: Option<EffectDomain>) {
        self.active_effect_domain = domain;
    }

    fn allocate_effect_sequence(&mut self) -> EffectSequence {
        self.next_effect_sequence = self
            .next_effect_sequence
            .checked_add(1)
            .expect("effect sequence exhausted");
        EffectSequence(self.next_effect_sequence)
    }

    fn allocate_publication_sequence(&mut self) -> EffectSequence {
        self.next_publication_sequence = self
            .next_publication_sequence
            .checked_add(1)
            .expect("publication sequence exhausted");
        EffectSequence(self.next_publication_sequence)
    }

    fn allocate_effect_domain(&mut self) -> EffectDomain {
        self.next_effect_domain = self
            .next_effect_domain
            .checked_add(1)
            .expect("effect domain exhausted");
        EffectDomain::World(self.next_effect_domain)
    }

    fn allocate_effect_semantic_record_ordinal(
        &mut self,
        domain: EffectDomain,
    ) -> EffectSemanticRecordOrdinal {
        let domain = match domain {
            EffectDomain::World(_) => EffectDomain::World(0),
            domain => domain,
        };
        let mut next = self.semantic_counter(domain);
        self.journal_semantic_counter(domain);
        next = next
            .checked_add(1)
            .expect("effect semantic record ordinal exhausted");
        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals).insert(domain, next);
        EffectSemanticRecordOrdinal(next)
    }

    fn publication_counter(&self, key: EffectPublicationId) -> u64 {
        self.next_effect_publication_record_ordinals
            .get(&key)
            .copied()
            .or_else(|| self.accepted_effects.as_ref()?.publication_counter(key))
            .unwrap_or(0)
    }

    fn semantic_counter(&self, key: EffectDomain) -> u64 {
        self.next_effect_semantic_record_ordinals
            .get(&key)
            .copied()
            .or_else(|| self.accepted_effects.as_ref()?.semantic_counter(key))
            .unwrap_or(0)
    }

    fn journal_publication_counter(&mut self, key: EffectPublicationId) {
        let previous = self
            .next_effect_publication_record_ordinals
            .get(&key)
            .copied();
        Arc::make_mut(&mut self.effect_counter_journal)
            .push(EffectCounterUndo::Publication { key, previous });
    }

    fn journal_semantic_counter(&mut self, key: EffectDomain) {
        let previous = self.next_effect_semantic_record_ordinals.get(&key).copied();
        Arc::make_mut(&mut self.effect_counter_journal)
            .push(EffectCounterUndo::Semantic { key, previous });
    }

    fn rollback_effect_counters(&mut self, mark: usize) {
        let undo = Arc::make_mut(&mut self.effect_counter_journal);
        for entry in undo[mark..].iter().rev() {
            match *entry {
                EffectCounterUndo::Publication { key, previous } => match previous {
                    Some(value) => {
                        Arc::make_mut(&mut self.next_effect_publication_record_ordinals)
                            .insert(key, value);
                    }
                    None => {
                        Arc::make_mut(&mut self.next_effect_publication_record_ordinals)
                            .remove(&key);
                    }
                },
                EffectCounterUndo::Semantic { key, previous } => match previous {
                    Some(value) => {
                        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals)
                            .insert(key, value);
                    }
                    None => {
                        Arc::make_mut(&mut self.next_effect_semantic_record_ordinals).remove(&key);
                    }
                },
            }
        }
        undo.truncate(mark);
    }

    fn allocate_effect_placement_intra_order(&mut self) -> EffectPlacementIntraOrder {
        self.next_effect_placement_intra_order = self
            .next_effect_placement_intra_order
            .checked_add(1)
            .expect("effect placement intra-order exhausted");
        EffectPlacementIntraOrder(self.next_effect_placement_intra_order)
    }

    fn effects_mut(&mut self) -> &mut Vec<EffectRecord> {
        Arc::make_mut(&mut self.effects)
    }

    fn apply_effect(&mut self, index: usize) -> Result<(), WorldError> {
        match &self.effects[index] {
            EffectRecord::StreamOpen { slot, target } => {
                let position = EffectPos(self.effect_base.0 + index as u64 + 1);
                Self::truncate_output(&mut self.backend, target.path()).map_err(|mut error| {
                    error.stream_open_unavailable = Some(Box::new(StreamOpenFailure {
                        position,
                        slot: *slot,
                        path: target.path().to_owned(),
                        context: self
                            .stream_open_contexts
                            .get(&position)
                            .cloned()
                            .unwrap_or_default(),
                    }));
                    error.effect_retry(EffectRetrySafety::Safe)
                })?;
                self.committed_output_paths.insert(target.path().to_owned());
                self.committed_write_streams[slot.index()] = Some(target.clone());
            }
            EffectRecord::StreamClose { slot } => {
                self.committed_write_streams[slot.index()] = None;
            }
            EffectRecord::StreamWrite { sink, text } => Self::commit_write(
                &mut self.backend,
                &self.committed_write_streams,
                *sink,
                text.as_bytes(),
            )?,
            EffectRecord::StreamWriteBytes { sink, bytes } => Self::commit_write(
                &mut self.backend,
                &self.committed_write_streams,
                *sink,
                bytes,
            )?,
            EffectRecord::DeferredWrite { .. }
            | EffectRecord::Special { .. }
            | EffectRecord::PdfObjectPlaceholder { .. }
            | EffectRecord::ShellEscape(_) => {}
        }
        Ok(())
    }

    fn commit_write(
        backend: &mut WorldBackend,
        committed_write_streams: &[Option<WriteTarget>; STREAM_SLOT_COUNT],
        sink: PrintSink,
        bytes: &[u8],
    ) -> Result<(), WorldError> {
        match sink {
            PrintSink::Terminal => Self::write_terminal(backend, bytes),
            PrintSink::Log => {
                Self::write_log(backend, bytes);
                Ok(())
            }
            PrintSink::TerminalAndLog => {
                Self::write_terminal(backend, bytes)?;
                Self::write_log(backend, bytes);
                Ok(())
            }
            PrintSink::Stream(slot) => {
                let Some(target) = &committed_write_streams[slot.index()] else {
                    return Ok(());
                };
                Self::append_output(backend, target.path(), bytes)
            }
        }
    }

    fn truncate_output(backend: &mut WorldBackend, path: &Path) -> Result<(), WorldError> {
        match backend {
            WorldBackend::Real { .. } => std::fs::write(path, []).map_err(|err| {
                WorldError::new("open output", Some(path.to_owned()), err.to_string())
            }),
            WorldBackend::Memory(memory) => {
                Arc::make_mut(memory)
                    .outputs
                    .insert(path.to_owned(), Vec::new());
                Ok(())
            }
        }
    }

    fn append_output(
        backend: &mut WorldBackend,
        path: &Path,
        bytes: &[u8],
    ) -> Result<(), WorldError> {
        match backend {
            WorldBackend::Real { .. } => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|err| {
                        WorldError::new("open output", Some(path.to_owned()), err.to_string())
                            .effect_retry(EffectRetrySafety::Safe)
                    })?;
                file.write_all(bytes).map_err(|err| {
                    WorldError::new("write output", Some(path.to_owned()), err.to_string())
                        .effect_retry(EffectRetrySafety::Poisoned)
                })
            }
            WorldBackend::Memory(memory) => {
                Arc::make_mut(memory)
                    .outputs
                    .entry(path.to_owned())
                    .or_default()
                    .extend_from_slice(bytes);
                Ok(())
            }
        }
    }

    fn write_terminal(backend: &mut WorldBackend, bytes: &[u8]) -> Result<(), WorldError> {
        match backend {
            WorldBackend::Real { .. } => io::stdout().write_all(bytes).map_err(|err| {
                WorldError::new("write terminal", None, err.to_string())
                    .effect_retry(EffectRetrySafety::Poisoned)
            }),
            WorldBackend::Memory(memory) => {
                Arc::make_mut(memory)
                    .terminal_output
                    .extend_from_slice(bytes);
                Ok(())
            }
        }
    }

    fn write_log(backend: &mut WorldBackend, bytes: &[u8]) {
        if let WorldBackend::Memory(memory) = backend {
            Arc::make_mut(memory).log_output.extend_from_slice(bytes);
        }
    }
}

impl Default for World {
    fn default() -> Self {
        Self::memory()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WorldBackend {
    Real { artifact_dir: PathBuf },
    Memory(Arc<MemoryBackend>),
}

fn deduplicate_retry_suffix(before: &[u8], current: &mut Vec<u8>) -> bool {
    if current.len() <= before.len() || !current.starts_with(before) {
        return false;
    }
    let replay = &current[before.len()..];
    let overlap_limit = before.len().min(replay.len());
    let overlap = (1..=overlap_limit)
        .rev()
        .find(|&len| before.ends_with(&replay[..len]))
        .unwrap_or(0);
    if overlap != 0 {
        current.drain(before.len()..before.len() + overlap);
        return true;
    }
    false
}

type StagedPublication = (PathBuf, PathBuf, Option<PathBuf>);

fn cleanup_staged_publication(staged: &[StagedPublication]) {
    for (_, temporary, _) in staged {
        let _ = std::fs::remove_file(temporary);
    }
}

fn rollback_staged_publication(staged: &[StagedPublication], published: usize) {
    for (path, _, _) in staged.iter().take(published) {
        let _ = std::fs::remove_file(path);
    }
    for (path, _, backup) in staged.iter().rev() {
        if let Some(backup) = backup {
            let _ = std::fs::rename(backup, path);
        }
    }
    cleanup_staged_publication(staged);
}

fn verify_stored_artifact(
    expected: ContentHash,
    path: &Path,
    operation: &'static str,
) -> Result<(), WorldError> {
    let bytes = std::fs::read(path)
        .map_err(|err| WorldError::new(operation, Some(path.to_owned()), err.to_string()))?;
    verify_artifact_identity(expected, &bytes, Some(path.to_owned()))
}

fn verify_artifact_identity(
    expected: ContentHash,
    bytes: &[u8],
    path: Option<PathBuf>,
) -> Result<(), WorldError> {
    if expected.matches_current_or_legacy(ContentDomain::Artifact, bytes) {
        return Ok(());
    }
    let actual = ContentHash::for_domain(ContentDomain::Artifact, bytes);
    Err(WorldError::new(
        "verify artifact identity",
        path,
        format!(
            "content identity mismatch: requested {}, actual {}",
            expected.hex(),
            actual.hex()
        ),
    ))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct MemoryBackend {
    files: BTreeMap<PathBuf, Arc<[u8]>>,
    modification_dates: BTreeMap<PathBuf, FileModificationDate>,
    outputs: BTreeMap<PathBuf, Vec<u8>>,
    artifacts: BTreeMap<ContentHash, Vec<u8>>,
    terminal_output: Vec<u8>,
    log_output: Vec<u8>,
}

/// tex.web §58's `if term_offset=max_print_line then wterm_cr`, applied to a
/// whole string starting from `offset`.
///
/// Returns `text` with a line break inserted wherever the sink's own column
/// reaches `limit`. A break the text already carries resets the column,
/// exactly as §57's `print_ln` does.
fn wrap_print_lines_at(text: &str, offset: usize, limit: usize) -> String {
    if offset + text.chars().count() < limit && !text.contains('\n') {
        return text.to_owned();
    }
    let mut wrapped = String::with_capacity(text.len() + text.len() / limit + 1);
    let mut column = offset;
    for character in text.chars() {
        if character == '\n' {
            wrapped.push('\n');
            column = 0;
            continue;
        }
        wrapped.push(character);
        column += 1;
        if column == limit {
            wrapped.push('\n');
            column = 0;
        }
    }
    wrapped
}

/// Replays one detached diagnostic for a single printable sink. The caller
/// runs this independently for terminal and transcript, because §§57--62
/// maintain distinct offsets for those sinks.
fn render_detached_diagnostic(
    operations: &[crate::diagnostic::DiagnosticPrintOperation],
    initial_partial_line: &str,
    max_print_line: usize,
) -> (String, String) {
    let mut output = String::new();
    let mut partial_line = initial_partial_line.to_owned();
    for operation in operations {
        match operation {
            crate::diagnostic::DiagnosticPrintOperation::Rendered(text) => {
                let wrapped =
                    wrap_print_lines_at(text, partial_line.chars().count(), max_print_line);
                append_partial_line(&mut partial_line, &wrapped);
                output.push_str(&wrapped);
            }
            crate::diagnostic::DiagnosticPrintOperation::EnsureLineStart => {
                if !partial_line.is_empty() {
                    partial_line.clear();
                    output.push('\n');
                }
            }
        }
    }
    (output, partial_line)
}

/// Replays a §245 diagnostic selected for both terminal and transcript.
///
/// Character wrapping retains independent per-sink offsets, while §62's
/// `print_nl` predicate is selector-wide: one open selected line makes the
/// resulting §57 newline visible in both sinks.
fn render_detached_diagnostic_pair(
    operations: &[crate::diagnostic::DiagnosticPrintOperation],
    initial_terminal_line: &str,
    initial_log_line: &str,
    max_print_line: usize,
) -> (String, String) {
    let mut terminal_output = String::new();
    let mut log_output = String::new();
    let mut terminal_line = initial_terminal_line.to_owned();
    let mut log_line = initial_log_line.to_owned();
    for operation in operations {
        match operation {
            crate::diagnostic::DiagnosticPrintOperation::Rendered(text) => {
                let terminal =
                    wrap_print_lines_at(text, terminal_line.chars().count(), max_print_line);
                let log = wrap_print_lines_at(text, log_line.chars().count(), max_print_line);
                append_partial_line(&mut terminal_line, &terminal);
                append_partial_line(&mut log_line, &log);
                terminal_output.push_str(&terminal);
                log_output.push_str(&log);
            }
            crate::diagnostic::DiagnosticPrintOperation::EnsureLineStart => {
                if !terminal_line.is_empty() || !log_line.is_empty() {
                    terminal_line.clear();
                    log_line.clear();
                    terminal_output.push('\n');
                    log_output.push('\n');
                }
            }
        }
    }
    (terminal_output, log_output)
}

/// The byte-domain counterpart of [`wrap_print_lines_at`]. Every TeX82 output
/// byte is one printed character, including bytes outside UTF-8.
fn wrap_print_bytes_at(bytes: &[u8], offset: usize, limit: usize) -> Vec<u8> {
    if offset + bytes.len() < limit && !bytes.contains(&b'\n') {
        return bytes.to_vec();
    }
    let mut wrapped = Vec::with_capacity(bytes.len() + bytes.len() / limit + 1);
    let mut column = offset;
    for &byte in bytes {
        if byte == b'\n' {
            wrapped.push(byte);
            column = 0;
            continue;
        }
        wrapped.push(byte);
        column += 1;
        if column == limit {
            wrapped.push(b'\n');
            column = 0;
        }
    }
    wrapped
}

fn bytes_to_partial_line_projection(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

fn append_partial_line(buffer: &mut String, text: &str) {
    for chunk in text.split_inclusive('\n') {
        if chunk.ends_with('\n') {
            buffer.clear();
        } else {
            buffer.push_str(chunk);
        }
    }
}

fn next_physical_line(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    let tail = bytes.get(start..)?;
    if tail.is_empty() {
        return None;
    }
    let newline = tail.iter().position(|&byte| byte == b'\n');
    let (mut end, next) = match newline {
        Some(offset) => (start + offset, start + offset + 1),
        None => (bytes.len(), bytes.len()),
    };
    if end > start && bytes[end - 1] == b'\r' {
        end -= 1;
    }
    Some((
        String::from_utf8_lossy(&bytes[start..end]).into_owned(),
        next,
    ))
}

fn normalize_terminal_line(mut line: String) -> String {
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    line
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn effect_retained_bytes(effect: &EffectRecord) -> usize {
    std::mem::size_of::<EffectRecord>()
        + match effect {
            EffectRecord::StreamOpen { target, .. } => target.path.as_os_str().len(),
            EffectRecord::StreamClose { .. } | EffectRecord::DeferredWrite { .. } => 0,
            EffectRecord::StreamWrite { text, .. } => text.len(),
            EffectRecord::StreamWriteBytes { bytes, .. } => bytes.len(),
            EffectRecord::Special { class, payload } => class.len().saturating_add(payload.len()),
            EffectRecord::PdfObjectPlaceholder { label } => label.len(),
            EffectRecord::ShellEscape(record) => record.command.len(),
        }
}

fn real_job_clock() -> JobClock {
    source_date_epoch().map_or_else(system_job_clock, unix_seconds_to_job_clock)
}

fn source_date_epoch() -> Option<u64> {
    parse_source_date_epoch(std::env::var_os("SOURCE_DATE_EPOCH"))
}

fn parse_source_date_epoch(value: Option<OsString>) -> Option<u64> {
    let value = value?;
    value.to_str()?.parse().ok()
}

fn system_job_clock() -> JobClock {
    let now: chrono::DateTime<chrono::Local> = SystemTime::now().into();
    datetime_to_job_clock(&now)
}

fn system_time_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn unix_seconds_to_job_clock(seconds: u64) -> JobClock {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    JobClock {
        time: (seconds_of_day / 60) as i32,
        second: (seconds_of_day % 60) as i32,
        day,
        month,
        year,
    }
}

fn datetime_to_job_clock<Tz: chrono::TimeZone>(date: &chrono::DateTime<Tz>) -> JobClock {
    use chrono::{Datelike as _, Timelike as _};

    JobClock {
        time: (date.hour() * 60 + date.minute()) as i32,
        second: date.second() as i32,
        day: date.day() as i32,
        month: date.month() as i32,
        year: date.year(),
    }
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, i32, i32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(m <= 2);
    (year as i32, m as i32, d as i32)
}

pub(crate) fn install_job_clock_params(
    set_int_param: &mut impl FnMut(IntParam, i32),
    clock: JobClock,
) {
    set_int_param(IntParam::TIME, clock.time);
    set_int_param(IntParam::DAY, clock.day);
    set_int_param(IntParam::MONTH, clock.month);
    set_int_param(IntParam::YEAR, clock.year);
}

#[cfg(test)]
mod tests;
