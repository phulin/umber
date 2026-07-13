//! External-effect capability boundary for the engine.
//!
//! This is the only engine module that may name host I/O and clock APIs.
//! Higher layers receive content-addressed inputs, buffered effect records,
//! deterministic RNG values, and job-start clock parameters through this API.

#![allow(clippy::disallowed_methods)]

use crate::env::banks::IntParam;
use crate::identity::{HandleIdentity, IdentityAllocator, IdentityMark};
use crate::ids::TokenListId;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
pub use tex_content::{ContentDomain, ContentHash, ContentIdentity};

/// TeX's 16 read/write stream slots.
pub const STREAM_SLOT_COUNT: usize = 16;

/// Exact bytes published by one successful page-artifact commit.
///
/// Construction stays inside the aggregate shipout boundary, so downstream
/// code can consume these bytes without rereading and reverifying the
/// content-addressed store.  The content id remains the authoritative durable
/// reference for replay and out-of-process drivers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommittedArtifact {
    hash: ContentHash,
    bytes: Arc<[u8]>,
}

/// Artifact bytes paired with their already-computed content identity.
///
/// Construction hashes the bytes exactly once. Private fields keep identity
/// and payload inseparable across the shipout commit boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedArtifact {
    hash: ContentHash,
    bytes: Vec<u8>,
}

impl VerifiedArtifact {
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        let hash = ContentHash::for_domain(ContentDomain::Artifact, &bytes);
        Self { hash, bytes }
    }

    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl CommittedArtifact {
    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn new(hash: ContentHash, bytes: Vec<u8>) -> Self {
        Self {
            hash,
            bytes: bytes.into(),
        }
    }
}

/// Bytes returned from a content-addressed `World` read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileContent {
    record: InputRecordId,
    path: PathBuf,
    bytes: Arc<[u8]>,
    hash: ContentHash,
}

impl FileContent {
    #[must_use]
    pub(crate) fn new(record: InputRecordId, path: PathBuf, bytes: Vec<u8>) -> Self {
        Self::from_shared(record, path, bytes.into())
    }

    #[must_use]
    fn from_shared(record: InputRecordId, path: PathBuf, bytes: Arc<[u8]>) -> Self {
        let hash = ContentHash::from_bytes(&bytes);
        Self {
            record,
            path,
            bytes,
            hash,
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

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes.to_vec()
    }
}

/// Rollback-safe identity of one successful read in the `World` input log.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputRecordId(HandleIdentity);

impl InputRecordId {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn new(raw: u32) -> Self {
        Self(HandleIdentity::builtin(raw))
    }

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
    next_line: usize,
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
    pub const fn next_line(&self) -> usize {
        self.next_line
    }
}

/// Snapshot-ready state for all partial stream/log buffers.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct StreamBufState {
    read_streams: [Option<ReadTarget>; STREAM_SLOT_COUNT],
    write_streams: [Option<WriteTarget>; STREAM_SLOT_COUNT],
    partial_lines: [String; STREAM_SLOT_COUNT],
    log_partial_line: String,
    terminal_partial_line: String,
    terminal_input_next: usize,
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
    pub fn partial_line(&self, slot: StreamSlot) -> &str {
        &self.partial_lines[slot.index()]
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
    pub const fn raw(self) -> u64 {
        self.0
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
    /// Deferred `\write` seam: the token list is intentionally unexpanded.
    DeferredWrite {
        stream: StreamSlot,
        tokens: TokenListId,
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

/// TeX's job-start clock values.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct JobClock {
    pub time: i32,
    pub day: i32,
    pub month: i32,
    pub year: i32,
}

impl JobClock {
    /// A deterministic clock used by hermetic in-memory worlds.
    pub const DEFAULT: Self = Self {
        time: 0,
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
    fn new(operation: &'static str, path: Option<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            operation,
            path,
            message: message.into(),
            committed_effects_through: None,
            retry_safety: EffectRetrySafety::NotAnEffectCommit,
        }
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
            Some(path) => write!(f, "{} {}: {}", self.operation, path.display(), self.message),
            None => write!(f, "{}: {}", self.operation, self.message),
        }
    }
}

impl std::error::Error for WorldError {}

/// Snapshot-owned `World` state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldSnapshot {
    effect_pos: EffectPos,
    stream_bufs: Arc<StreamBufState>,
    rng: RngState,
    job_clock: JobClock,
    shell_escape_policy: ShellEscapePolicy,
    input_len: usize,
    input_identities: IdentityMark,
    shell_escape_len: usize,
}

/// Cursor into World-owned state for semantic convergence hashing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorldStateHashCursor {
    effect_pos: EffectPos,
    stream_bufs: Arc<StreamBufState>,
    rng: RngState,
    job_clock: JobClock,
    shell_escape_policy: ShellEscapePolicy,
    shell_escape_len: usize,
}

/// Engine capability object for all external effects.
#[derive(Debug)]
pub struct World {
    backend: WorldBackend,
    effect_base: EffectPos,
    effects: Vec<EffectRecord>,
    stream_bufs: Arc<StreamBufState>,
    committed_write_streams: [Option<WriteTarget>; STREAM_SLOT_COUNT],
    rng: RngState,
    job_clock: JobClock,
    shell_escape_policy: ShellEscapePolicy,
    inputs: Vec<InputRecord>,
    input_identities: IdentityAllocator,
    input_contents: BTreeMap<ContentHash, Arc<[u8]>>,
    terminal_inputs: Vec<String>,
    shell_escapes: Vec<ShellEscapeRecord>,
    artifact_commits: Vec<ContentHash>,
    committed_artifacts: Vec<CommittedArtifact>,
    verified_artifacts: BTreeSet<ContentHash>,
    effect_commit_poison: Option<WorldError>,
    execution_tracing: bool,
    execution_trace: Vec<ExecutionTraceEvent>,
    #[cfg(test)]
    effect_commit_fault: Option<EffectCommitFault>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EffectCommitFault {
    Before(EffectPos),
    AfterPartial(EffectPos),
}

impl Clone for World {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            effect_base: self.effect_base,
            effects: self.effects.clone(),
            stream_bufs: self.stream_bufs.clone(),
            committed_write_streams: self.committed_write_streams.clone(),
            rng: self.rng,
            job_clock: self.job_clock,
            shell_escape_policy: self.shell_escape_policy,
            inputs: self.inputs.clone(),
            input_identities: self.input_identities.fork(),
            input_contents: self.input_contents.clone(),
            terminal_inputs: self.terminal_inputs.clone(),
            shell_escapes: self.shell_escapes.clone(),
            artifact_commits: self.artifact_commits.clone(),
            committed_artifacts: self.committed_artifacts.clone(),
            verified_artifacts: self.verified_artifacts.clone(),
            effect_commit_poison: self.effect_commit_poison.clone(),
            execution_tracing: self.execution_tracing,
            execution_trace: self.execution_trace.clone(),
            #[cfg(test)]
            effect_commit_fault: self.effect_commit_fault,
        }
    }
}

impl PartialEq for World {
    fn eq(&self, other: &Self) -> bool {
        self.backend == other.backend
            && self.effect_base == other.effect_base
            && self.effects == other.effects
            && self.stream_bufs == other.stream_bufs
            && self.committed_write_streams == other.committed_write_streams
            && self.rng == other.rng
            && self.job_clock == other.job_clock
            && self.shell_escape_policy == other.shell_escape_policy
            && self.inputs == other.inputs
            && self.input_contents == other.input_contents
            && self.terminal_inputs == other.terminal_inputs
            && self.shell_escapes == other.shell_escapes
            && self.artifact_commits == other.artifact_commits
            && self.committed_artifacts == other.committed_artifacts
            && self.effect_commit_poison == other.effect_commit_poison
    }
}

impl Eq for World {}

impl World {
    /// Creates a deterministic in-memory world for tests and hermetic runs.
    #[must_use]
    pub fn memory() -> Self {
        Self::memory_with_clock(JobClock::DEFAULT)
    }

    /// Creates a deterministic in-memory world with an explicit job clock.
    #[must_use]
    pub fn memory_with_clock(job_clock: JobClock) -> Self {
        Self::new(WorldBackend::Memory(MemoryBackend::default()), job_clock)
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
        Self::new(
            WorldBackend::Real {
                artifact_dir: artifact_dir.into(),
            },
            job_clock,
        )
    }

    fn new(backend: WorldBackend, job_clock: JobClock) -> Self {
        Self {
            backend,
            effect_base: EffectPos::default(),
            effects: Vec::new(),
            stream_bufs: Arc::new(StreamBufState::default()),
            committed_write_streams: Default::default(),
            rng: RngState::default(),
            job_clock,
            shell_escape_policy: ShellEscapePolicy::default(),
            inputs: Vec::new(),
            input_identities: IdentityAllocator::new(0),
            input_contents: BTreeMap::new(),
            terminal_inputs: Vec::new(),
            shell_escapes: Vec::new(),
            artifact_commits: Vec::new(),
            committed_artifacts: Vec::new(),
            verified_artifacts: BTreeSet::new(),
            effect_commit_poison: None,
            execution_tracing: false,
            execution_trace: Vec::new(),
            #[cfg(test)]
            effect_commit_fault: None,
        }
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

    #[cfg(test)]
    pub(crate) fn fail_effect_commit_before(&mut self, position: EffectPos) {
        self.effect_commit_fault = Some(EffectCommitFault::Before(position));
    }

    #[cfg(test)]
    pub(crate) fn fail_effect_commit_after_partial(&mut self, position: EffectPos) {
        self.effect_commit_fault = Some(EffectCommitFault::AfterPartial(position));
    }

    /// Adds or replaces one file in an in-memory world.
    pub fn set_memory_file(
        &mut self,
        path: impl Into<PathBuf>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<(), WorldError> {
        let WorldBackend::Memory(memory) = &mut self.backend else {
            return Err(WorldError::new(
                "set memory file",
                None,
                "world is not memory-backed",
            ));
        };
        memory.files.insert(path.into(), Arc::from(bytes.into()));
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
        let bytes: Arc<[u8]> = match &self.backend {
            WorldBackend::Real { .. } => Arc::from(std::fs::read(path).map_err(|err| {
                WorldError::new("read file", Some(path.to_owned()), err.to_string())
            })?),
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
                })?,
        };
        let record = self.allocate_input_record();
        let content = FileContent::from_shared(record, path.to_owned(), bytes);
        self.input_contents
            .entry(content.hash)
            .or_insert_with(|| content.bytes.clone());
        self.inputs.push(InputRecord {
            path: content.path.clone(),
            hash: content.hash,
            len: content.bytes.len(),
        });
        Ok(content)
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
                memory
                    .files
                    .insert(path.to_owned(), Arc::from(bytes.as_ref()));
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
        self.stream_bufs_mut().read_streams[slot.index()] = Some(ReadTarget {
            path: content.path.clone(),
            hash: content.hash,
            next_line: 0,
        });
        Ok(content)
    }

    pub fn close_in(&mut self, slot: StreamSlot) {
        self.stream_bufs_mut().read_streams[slot.index()] = None;
    }

    #[must_use]
    pub fn input_stream_eof(&self, slot: StreamSlot) -> bool {
        let Some(target) = self.stream_bufs.read_streams[slot.index()].as_ref() else {
            return true;
        };
        let Some(bytes) = self.input_contents.get(&target.hash) else {
            return true;
        };
        target.next_line >= split_physical_lines(&String::from_utf8_lossy(bytes)).len()
    }

    pub fn read_stream_line(&mut self, slot: StreamSlot) -> Result<Option<String>, WorldError> {
        let Some(target) = self.stream_bufs.read_streams[slot.index()].as_ref() else {
            return Ok(None);
        };
        let (hash, path, next_line) = (target.hash, target.path.clone(), target.next_line);
        let Some(bytes) = self.input_contents.get(&hash) else {
            return Err(WorldError::new(
                "read input stream",
                Some(path),
                "pinned input content is missing",
            ));
        };
        let lines = split_physical_lines(&String::from_utf8_lossy(bytes));
        let Some(line) = lines.get(next_line).cloned() else {
            self.stream_bufs_mut().read_streams[slot.index()] = None;
            return Ok(Some(String::new()));
        };
        self.stream_bufs_mut().read_streams[slot.index()]
            .as_mut()
            .expect("read stream remained open")
            .next_line += 1;
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
        self.input_contents
            .entry(content.hash)
            .or_insert_with(|| content.bytes.clone());
        self.inputs.push(InputRecord {
            path: content.path,
            hash: content.hash,
            len: content.bytes.len(),
        });
        Ok(Some(line))
    }

    pub fn recorded_input_content(&self, id: InputRecordId) -> Option<FileContent> {
        let record = self.input_record(id)?;
        let bytes = self.input_contents.get(&record.hash)?.clone();
        Some(FileContent {
            record: id,
            path: record.path.clone(),
            bytes,
            hash: record.hash,
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
                memory
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
        &self.artifact_commits
    }

    /// Returns the in-process commit receipts aligned with
    /// [`Self::artifact_commits`].
    ///
    /// These are downstream notification state, not rollback or semantic
    /// state. Durable consumers should retain the content id and use
    /// [`Self::read_artifact`] in a later process.
    #[must_use]
    pub fn committed_artifacts(&self) -> &[CommittedArtifact] {
        &self.committed_artifacts
    }

    pub(crate) fn record_artifact_commit(&mut self, hash: ContentHash, bytes: Vec<u8>) {
        self.artifact_commits.push(hash);
        self.committed_artifacts
            .push(CommittedArtifact::new(hash, bytes));
    }

    pub fn open_out(&mut self, slot: StreamSlot, path: impl Into<PathBuf>) {
        let target = WriteTarget { path: path.into() };
        self.append_effect(EffectRecord::StreamOpen {
            slot,
            target: target.clone(),
        });
        self.stream_bufs_mut().write_streams[slot.index()] = Some(target);
        self.stream_bufs_mut().partial_lines[slot.index()].clear();
    }

    pub fn close_out(&mut self, slot: StreamSlot) {
        self.append_effect(EffectRecord::StreamClose { slot });
        self.stream_bufs_mut().write_streams[slot.index()] = None;
        self.stream_bufs_mut().partial_lines[slot.index()].clear();
    }

    /// Buffers routed output as a deferred effect record.
    pub fn write_text(&mut self, sink: PrintSink, text: &str) {
        self.append_effect(EffectRecord::StreamWrite {
            sink,
            text: text.to_owned(),
        });
        match sink {
            PrintSink::Terminal => {
                append_partial_line(&mut self.stream_bufs_mut().terminal_partial_line, text)
            }
            PrintSink::Log => {
                append_partial_line(&mut self.stream_bufs_mut().log_partial_line, text)
            }
            PrintSink::TerminalAndLog => {
                append_partial_line(&mut self.stream_bufs_mut().terminal_partial_line, text);
                append_partial_line(&mut self.stream_bufs_mut().log_partial_line, text);
            }
            PrintSink::Stream(slot) => append_partial_line(
                &mut self.stream_bufs_mut().partial_lines[slot.index()],
                text,
            ),
        }
    }

    /// Appends a deferred `\write` after the owning `Universe` validates the
    /// token-list capability against its live store timeline.
    pub(crate) fn record_deferred_write(&mut self, stream: StreamSlot, tokens: TokenListId) {
        self.append_effect(EffectRecord::DeferredWrite { stream, tokens });
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
                    self.effects.drain(0..applied);
                    self.effect_base.0 += applied as u64;
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

        self.effects.drain(0..applied);
        self.effect_base = effect_pos;
        Ok(())
    }

    #[must_use]
    pub const fn shell_escape_policy(&self) -> ShellEscapePolicy {
        self.shell_escape_policy
    }

    pub fn set_shell_escape_policy(&mut self, policy: ShellEscapePolicy) {
        self.shell_escape_policy = policy;
    }

    #[must_use]
    pub fn next_random_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    #[must_use]
    pub const fn job_clock(&self) -> JobClock {
        self.job_clock
    }

    #[must_use]
    pub fn input_records(&self) -> &[InputRecord] {
        &self.inputs
    }

    /// Returns a recorded input only when `id` is live in this World timeline.
    #[must_use]
    pub fn input_record(&self, id: InputRecordId) -> Option<&InputRecord> {
        if !self.input_identities.contains(id.0) {
            return None;
        }
        self.inputs.get(id.raw() as usize)
    }

    /// Returns the content-addressed bytes for a previously-read input.
    #[must_use]
    pub fn input_content(&self, hash: ContentHash) -> Option<&[u8]> {
        self.input_contents.get(&hash).map(AsRef::as_ref)
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
        &self.effects
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub(crate) fn testing_state_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.effect_base.hash(&mut hasher);
        self.effects.hash(&mut hasher);
        self.stream_bufs.hash(&mut hasher);
        self.committed_write_streams.hash(&mut hasher);
        self.rng.hash(&mut hasher);
        self.job_clock.hash(&mut hasher);
        self.shell_escape_policy.hash(&mut hasher);
        self.inputs.hash(&mut hasher);
        self.shell_escapes.hash(&mut hasher);
        hasher.finish()
    }

    #[must_use]
    pub(crate) fn state_hash_cursor(&self) -> WorldStateHashCursor {
        WorldStateHashCursor {
            effect_pos: self.effect_pos(),
            stream_bufs: self.stream_bufs.clone(),
            rng: self.rng,
            job_clock: self.job_clock,
            shell_escape_policy: self.shell_escape_policy,
            shell_escape_len: self.shell_escapes.len(),
        }
    }

    #[must_use]
    pub(crate) fn state_hash_cursor_from_snapshot(
        snapshot: &WorldSnapshot,
    ) -> WorldStateHashCursor {
        WorldStateHashCursor {
            effect_pos: snapshot.effect_pos,
            stream_bufs: snapshot.stream_bufs.clone(),
            rng: snapshot.rng,
            job_clock: snapshot.job_clock,
            shell_escape_policy: snapshot.shell_escape_policy,
            shell_escape_len: snapshot.shell_escape_len,
        }
    }

    #[must_use]
    pub(crate) fn retarget_state_hash_cursor_after_commit(
        &self,
        cursor: &WorldStateHashCursor,
    ) -> WorldStateHashCursor {
        let effect_pos = cursor.effect_pos.max(self.effect_base);
        assert!(
            effect_pos <= self.effect_pos(),
            "World hash cursor effect position is past effect end"
        );
        assert!(
            cursor.shell_escape_len <= self.shell_escapes.len(),
            "World hash cursor shell-escape length is past shell-escape end"
        );
        WorldStateHashCursor {
            effect_pos,
            stream_bufs: cursor.stream_bufs.clone(),
            rng: cursor.rng,
            job_clock: cursor.job_clock,
            shell_escape_policy: cursor.shell_escape_policy,
            shell_escape_len: cursor.shell_escape_len,
        }
    }

    #[must_use]
    pub(crate) fn effect_pos_is_retained(&self, effect_pos: EffectPos) -> bool {
        self.effect_base <= effect_pos && effect_pos <= self.effect_pos()
    }

    #[must_use]
    pub(crate) fn effect_records_since(&self, cursor: &WorldStateHashCursor) -> &[EffectRecord] {
        assert!(
            cursor.effect_pos >= self.effect_base,
            "World hash cursor effect position has already been committed and dropped"
        );
        let start = (cursor.effect_pos.raw() - self.effect_base.raw()) as usize;
        assert!(
            start <= self.effects.len(),
            "World hash cursor is past effect end"
        );
        &self.effects[start..]
    }

    #[must_use]
    pub(crate) fn shell_escape_records_since(
        &self,
        cursor: &WorldStateHashCursor,
    ) -> &[ShellEscapeRecord] {
        assert!(
            cursor.shell_escape_len <= self.shell_escapes.len(),
            "World hash cursor is past shell-escape end"
        );
        &self.shell_escapes[cursor.shell_escape_len..]
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

    #[must_use]
    pub fn stream_bufs(&self) -> &StreamBufState {
        &self.stream_bufs
    }

    pub(crate) fn stream_bufs_root(&self) -> Arc<StreamBufState> {
        Arc::clone(&self.stream_bufs)
    }

    fn stream_bufs_mut(&mut self) -> &mut StreamBufState {
        Arc::make_mut(&mut self.stream_bufs)
    }

    #[must_use]
    pub const fn rng_state(&self) -> RngState {
        self.rng
    }

    #[must_use]
    pub(crate) fn snapshot(&self) -> WorldSnapshot {
        WorldSnapshot {
            effect_pos: self.effect_pos(),
            stream_bufs: self.stream_bufs.clone(),
            rng: self.rng,
            job_clock: self.job_clock,
            shell_escape_policy: self.shell_escape_policy,
            input_len: self.inputs.len(),
            input_identities: self.input_identities.watermark(),
            shell_escape_len: self.shell_escapes.len(),
        }
    }

    pub(crate) fn assert_snapshot_retained(&self, snapshot: &WorldSnapshot) {
        assert!(
            self.effect_pos_is_retained(snapshot.effect_pos),
            "World snapshot effect position has already been committed and dropped"
        );
    }

    pub(crate) fn rollback(&mut self, snapshot: &WorldSnapshot) {
        self.assert_snapshot_retained(snapshot);
        self.input_identities
            .rollback(snapshot.input_identities)
            .expect("World input identity mark must name a retained ancestor");
        self.effects
            .truncate((snapshot.effect_pos.raw() - self.effect_base.raw()) as usize);
        self.stream_bufs = snapshot.stream_bufs.clone();
        self.rng = snapshot.rng;
        self.shell_escape_policy = snapshot.shell_escape_policy;
        self.inputs.truncate(snapshot.input_len);
        self.shell_escapes.truncate(snapshot.shell_escape_len);
    }

    fn allocate_input_record(&mut self) -> InputRecordId {
        let identity = self
            .input_identities
            .allocate()
            .expect("World input record identity capacity exhausted");
        assert_eq!(
            identity.slot() as usize,
            self.inputs.len(),
            "World input identities and records diverged"
        );
        InputRecordId(identity)
    }

    fn append_effect(&mut self, record: EffectRecord) {
        self.effects.push(record);
    }

    fn apply_effect(&mut self, index: usize) -> Result<(), WorldError> {
        #[cfg(test)]
        {
            let position = EffectPos(self.effect_base.0 + index as u64 + 1);
            match self.effect_commit_fault {
                Some(EffectCommitFault::Before(target)) if target == position => {
                    self.effect_commit_fault = None;
                    return Err(
                        WorldError::new("injected effect commit", None, "before apply")
                            .effect_retry(EffectRetrySafety::Safe),
                    );
                }
                Some(EffectCommitFault::AfterPartial(target)) if target == position => {
                    self.effect_commit_fault = None;
                    if let EffectRecord::StreamWrite { sink, text } = &self.effects[index] {
                        let midpoint = text.len().div_ceil(2);
                        Self::commit_write(
                            &mut self.backend,
                            &self.committed_write_streams,
                            *sink,
                            &text.as_bytes()[..midpoint],
                        )?;
                    }
                    return Err(WorldError::new(
                        "injected effect commit",
                        None,
                        "after partial apply",
                    )
                    .effect_retry(EffectRetrySafety::Poisoned));
                }
                _ => {}
            }
        }
        match &self.effects[index] {
            EffectRecord::StreamOpen { slot, target } => {
                Self::truncate_output(&mut self.backend, target.path())
                    .map_err(|error| error.effect_retry(EffectRetrySafety::Poisoned))?;
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
                memory.outputs.insert(path.to_owned(), Vec::new());
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
                memory
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
                memory.terminal_output.extend_from_slice(bytes);
                Ok(())
            }
        }
    }

    fn write_log(backend: &mut WorldBackend, bytes: &[u8]) {
        if let WorldBackend::Memory(memory) = backend {
            memory.log_output.extend_from_slice(bytes);
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
    Memory(MemoryBackend),
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
    outputs: BTreeMap<PathBuf, Vec<u8>>,
    artifacts: BTreeMap<ContentHash, Vec<u8>>,
    terminal_output: Vec<u8>,
    log_output: Vec<u8>,
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

fn split_physical_lines(input: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, ch) in input.char_indices() {
        if ch == '\n' {
            let end = if index > start && input[..index].ends_with('\r') {
                index - 1
            } else {
                index
            };
            lines.push(input[start..end].to_owned());
            start = index + 1;
        }
    }
    if start < input.len() {
        lines.push(input[start..].to_owned());
    }
    lines
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

fn real_job_clock() -> JobClock {
    source_date_epoch().map_or_else(system_clock_seconds, unix_seconds_to_job_clock)
}

fn source_date_epoch() -> Option<u64> {
    parse_source_date_epoch(std::env::var_os("SOURCE_DATE_EPOCH"))
}

fn parse_source_date_epoch(value: Option<OsString>) -> Option<u64> {
    let value = value?;
    value.to_str()?.parse().ok()
}

fn system_clock_seconds() -> JobClock {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    unix_seconds_to_job_clock(seconds)
}

fn unix_seconds_to_job_clock(seconds: u64) -> JobClock {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    JobClock {
        time: (seconds_of_day / 60) as i32,
        day,
        month,
        year,
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
