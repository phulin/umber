//! External-effect capability boundary for the engine.
//!
//! This is the only engine module that may name host I/O and clock APIs.
//! Higher layers receive content-addressed inputs, buffered effect records,
//! deterministic RNG values, and job-start clock parameters through this API.

#![allow(clippy::disallowed_methods)]

use crate::env::banks::IntParam;
use crate::ids::TokenListId;
use std::collections::BTreeMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// TeX's 16 read/write stream slots.
pub const STREAM_SLOT_COUNT: usize = 16;

/// Stable content hash for bytes consumed through `World`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hashes bytes with a small deterministic content-addressing hash.
    ///
    /// f26.2 needs stable addressing but not a cryptographic dependency.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        const OFFSETS: [u64; 4] = [
            0xcbf2_9ce4_8422_2325,
            0x8422_2325_cbf2_9ce4,
            0x9e37_79b9_7f4a_7c15,
            0x94d0_49bb_1331_11eb,
        ];
        const PRIMES: [u64; 4] = [
            0x0000_0100_0000_01b3,
            0x0000_0100_0000_01d3,
            0x0000_0100_0000_01f3,
            0x0000_0100_0000_0213,
        ];

        let mut words = OFFSETS;
        for (index, &byte) in bytes.iter().enumerate() {
            for lane in 0..4 {
                words[lane] ^=
                    u64::from(byte).wrapping_add(((index as u64) << (lane * 7)) | lane as u64);
                words[lane] = words[lane].wrapping_mul(PRIMES[lane]);
                words[lane] ^= words[lane].rotate_right(17 + lane as u32);
            }
        }
        for word in &mut words {
            *word ^= bytes.len() as u64;
            *word = splitmix64(*word);
        }

        let mut out = [0; 32];
        for (chunk, word) in out.chunks_exact_mut(8).zip(words) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        Self(out)
    }

    /// Returns the raw 32-byte hash.
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }

    /// Returns a lowercase hexadecimal encoding.
    #[must_use]
    pub fn hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            use fmt::Write as _;
            write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
        }
        out
    }
}

/// Bytes returned from a content-addressed `World` read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileContent {
    path: PathBuf,
    bytes: Vec<u8>,
    hash: ContentHash,
}

impl FileContent {
    #[must_use]
    pub(crate) fn new(path: PathBuf, bytes: Vec<u8>) -> Self {
        let hash = ContentHash::from_bytes(&bytes);
        Self { path, bytes, hash }
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
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
}

impl WorldError {
    fn new(operation: &'static str, path: Option<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            operation,
            path,
            message: message.into(),
        }
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
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WorldSnapshot {
    effect_pos: EffectPos,
    stream_bufs: StreamBufState,
    rng: RngState,
    job_clock: JobClock,
    shell_escape_policy: ShellEscapePolicy,
    input_len: usize,
    shell_escape_len: usize,
}

/// Cursor into World-owned state for semantic convergence hashing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorldStateHashCursor {
    effect_pos: EffectPos,
    stream_bufs: StreamBufState,
    rng: RngState,
    job_clock: JobClock,
    shell_escape_policy: ShellEscapePolicy,
    input_len: usize,
    shell_escape_len: usize,
}

/// Engine capability object for all external effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct World {
    backend: WorldBackend,
    effect_base: EffectPos,
    effects: Vec<EffectRecord>,
    stream_bufs: StreamBufState,
    committed_write_streams: [Option<WriteTarget>; STREAM_SLOT_COUNT],
    rng: RngState,
    job_clock: JobClock,
    shell_escape_policy: ShellEscapePolicy,
    inputs: Vec<InputRecord>,
    input_contents: BTreeMap<ContentHash, Vec<u8>>,
    terminal_inputs: Vec<String>,
    shell_escapes: Vec<ShellEscapeRecord>,
}

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
            stream_bufs: StreamBufState::default(),
            committed_write_streams: Default::default(),
            rng: RngState::default(),
            job_clock,
            shell_escape_policy: ShellEscapePolicy::default(),
            inputs: Vec::new(),
            input_contents: BTreeMap::new(),
            terminal_inputs: Vec::new(),
            shell_escapes: Vec::new(),
        }
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
        memory.files.insert(path.into(), bytes.into());
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
        let bytes = match &self.backend {
            WorldBackend::Real { .. } => std::fs::read(path).map_err(|err| {
                WorldError::new("read file", Some(path.to_owned()), err.to_string())
            })?,
            WorldBackend::Memory(memory) => memory.files.get(path).cloned().ok_or_else(|| {
                WorldError::new(
                    "read file",
                    Some(path.to_owned()),
                    "not found in memory world",
                )
            })?,
        };
        let content = FileContent::new(path.to_owned(), bytes);
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

    /// Opens an input stream slot by reading and pinning its content now.
    pub fn open_in(
        &mut self,
        slot: StreamSlot,
        path: impl AsRef<Path>,
    ) -> Result<FileContent, WorldError> {
        let content = self.read_file(path)?;
        self.stream_bufs.read_streams[slot.index()] = Some(ReadTarget {
            path: content.path.clone(),
            hash: content.hash,
            next_line: 0,
        });
        Ok(content)
    }

    pub fn close_in(&mut self, slot: StreamSlot) {
        self.stream_bufs.read_streams[slot.index()] = None;
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
        let Some(target) = self.stream_bufs.read_streams[slot.index()].as_mut() else {
            return Ok(None);
        };
        let Some(bytes) = self.input_contents.get(&target.hash) else {
            return Err(WorldError::new(
                "read input stream",
                Some(target.path.clone()),
                "pinned input content is missing",
            ));
        };
        let lines = split_physical_lines(&String::from_utf8_lossy(bytes));
        let Some(line) = lines.get(target.next_line).cloned() else {
            self.stream_bufs.read_streams[slot.index()] = None;
            return Ok(Some(String::new()));
        };
        target.next_line += 1;
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
        self.stream_bufs.terminal_input_next += 1;
        let bytes = line.as_bytes().to_vec();
        let content = FileContent::new(PathBuf::from("<terminal>"), bytes);
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

    pub fn recorded_input_content(&self, index: usize) -> Option<FileContent> {
        let record = self.inputs.get(index)?;
        let bytes = self.input_contents.get(&record.hash)?.clone();
        Some(FileContent {
            path: record.path.clone(),
            bytes,
            hash: record.hash,
        })
    }

    /// Stores committed page artifact bytes by content hash.
    ///
    /// This method is intended for the shipout commit barrier: callers prepare
    /// deterministic artifact bytes first, then ask `World` to materialize the
    /// content-addressed object in the configured artifact store.
    pub fn store_artifact(&mut self, bytes: &[u8]) -> Result<ContentHash, WorldError> {
        let hash = ContentHash::from_bytes(bytes);
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
                if !path.exists() {
                    std::fs::write(&path, bytes).map_err(|err| {
                        WorldError::new("write artifact", Some(path), err.to_string())
                    })?;
                }
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
                    Ok(bytes) => Ok(Some(bytes)),
                    Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
                    Err(err) => Err(WorldError::new(
                        "read artifact",
                        Some(path),
                        err.to_string(),
                    )),
                }
            }
            WorldBackend::Memory(memory) => Ok(memory.artifacts.get(&hash).cloned()),
        }
    }

    pub fn open_out(&mut self, slot: StreamSlot, path: impl Into<PathBuf>) {
        let target = WriteTarget { path: path.into() };
        self.append_effect(EffectRecord::StreamOpen {
            slot,
            target: target.clone(),
        });
        self.stream_bufs.write_streams[slot.index()] = Some(target);
        self.stream_bufs.partial_lines[slot.index()].clear();
    }

    pub fn close_out(&mut self, slot: StreamSlot) {
        self.append_effect(EffectRecord::StreamClose { slot });
        self.stream_bufs.write_streams[slot.index()] = None;
        self.stream_bufs.partial_lines[slot.index()].clear();
    }

    /// Buffers routed output as a deferred effect record.
    pub fn write_text(&mut self, sink: PrintSink, text: &str) {
        self.append_effect(EffectRecord::StreamWrite {
            sink,
            text: text.to_owned(),
        });
        match sink {
            PrintSink::Terminal => {
                append_partial_line(&mut self.stream_bufs.terminal_partial_line, text)
            }
            PrintSink::Log => append_partial_line(&mut self.stream_bufs.log_partial_line, text),
            PrintSink::TerminalAndLog => {
                append_partial_line(&mut self.stream_bufs.terminal_partial_line, text);
                append_partial_line(&mut self.stream_bufs.log_partial_line, text);
            }
            PrintSink::Stream(slot) => {
                append_partial_line(&mut self.stream_bufs.partial_lines[slot.index()], text)
            }
        }
    }

    /// Records a deferred `\write` token list without expanding it.
    ///
    /// TODO(umber2-n11): expand these token lists at shipout before emitting
    /// stream-write bytes. f26.3 deliberately has no `\immediate\write`.
    pub fn record_deferred_write(&mut self, stream: StreamSlot, tokens: TokenListId) {
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
    pub fn commit_effects(&mut self, effect_pos: EffectPos) -> Result<(), WorldError> {
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
            ));
        }

        let mut applied = 0usize;
        let count = (effect_pos.raw() - self.effect_base.raw()) as usize;
        for index in 0..count {
            if let Err(err) = self.apply_effect(index) {
                if applied > 0 {
                    self.effects.drain(0..applied);
                    self.effect_base.0 += applied as u64;
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
            input_len: self.inputs.len(),
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
            input_len: snapshot.input_len,
            shell_escape_len: snapshot.shell_escape_len,
        }
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
    pub(crate) fn input_records_since(&self, cursor: &WorldStateHashCursor) -> &[InputRecord] {
        assert!(
            cursor.input_len <= self.inputs.len(),
            "World hash cursor is past input end"
        );
        &self.inputs[cursor.input_len..]
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
    pub const fn stream_bufs(&self) -> &StreamBufState {
        &self.stream_bufs
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
            shell_escape_len: self.shell_escapes.len(),
        }
    }

    pub(crate) fn rollback(&mut self, snapshot: &WorldSnapshot) {
        assert!(
            snapshot.effect_pos >= self.effect_base,
            "World snapshot effect position has already been committed and dropped"
        );
        self.effects
            .truncate((snapshot.effect_pos.raw() - self.effect_base.raw()) as usize);
        self.stream_bufs = snapshot.stream_bufs.clone();
        self.rng = snapshot.rng;
        self.shell_escape_policy = snapshot.shell_escape_policy;
        self.inputs.truncate(snapshot.input_len);
        self.shell_escapes.truncate(snapshot.shell_escape_len);
    }

    fn append_effect(&mut self, record: EffectRecord) {
        self.effects.push(record);
    }

    fn apply_effect(&mut self, index: usize) -> Result<(), WorldError> {
        let record = self.effects[index].clone();
        match record {
            EffectRecord::StreamOpen { slot, target } => {
                self.truncate_output(target.path())?;
                self.committed_write_streams[slot.index()] = Some(target);
            }
            EffectRecord::StreamClose { slot } => {
                self.committed_write_streams[slot.index()] = None;
            }
            EffectRecord::StreamWrite { sink, text } => self.commit_write(sink, text.as_bytes())?,
            EffectRecord::DeferredWrite { .. }
            | EffectRecord::Special { .. }
            | EffectRecord::PdfObjectPlaceholder { .. }
            | EffectRecord::ShellEscape(_) => {}
        }
        Ok(())
    }

    fn commit_write(&mut self, sink: PrintSink, bytes: &[u8]) -> Result<(), WorldError> {
        match sink {
            PrintSink::Terminal => self.write_terminal(bytes),
            PrintSink::Log => {
                self.write_log(bytes);
                Ok(())
            }
            PrintSink::TerminalAndLog => {
                self.write_terminal(bytes)?;
                self.write_log(bytes);
                Ok(())
            }
            PrintSink::Stream(slot) => {
                let Some(target) = self.committed_write_streams[slot.index()].clone() else {
                    return Ok(());
                };
                self.append_output(target.path(), bytes)
            }
        }
    }

    fn truncate_output(&mut self, path: &Path) -> Result<(), WorldError> {
        match &mut self.backend {
            WorldBackend::Real { .. } => std::fs::write(path, []).map_err(|err| {
                WorldError::new("open output", Some(path.to_owned()), err.to_string())
            }),
            WorldBackend::Memory(memory) => {
                memory.outputs.insert(path.to_owned(), Vec::new());
                Ok(())
            }
        }
    }

    fn append_output(&mut self, path: &Path, bytes: &[u8]) -> Result<(), WorldError> {
        match &mut self.backend {
            WorldBackend::Real { .. } => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|err| {
                        WorldError::new("write output", Some(path.to_owned()), err.to_string())
                    })?;
                file.write_all(bytes).map_err(|err| {
                    WorldError::new("write output", Some(path.to_owned()), err.to_string())
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

    fn write_terminal(&mut self, bytes: &[u8]) -> Result<(), WorldError> {
        match &mut self.backend {
            WorldBackend::Real { .. } => io::stdout()
                .write_all(bytes)
                .map_err(|err| WorldError::new("write terminal", None, err.to_string())),
            WorldBackend::Memory(memory) => {
                memory.terminal_output.extend_from_slice(bytes);
                Ok(())
            }
        }
    }

    fn write_log(&mut self, bytes: &[u8]) {
        if let WorldBackend::Memory(memory) = &mut self.backend {
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct MemoryBackend {
    files: BTreeMap<PathBuf, Vec<u8>>,
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
