//! Named-boundary incremental editor sessions.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};

use tex_exec::{
    CheckpointSink, EditorRestoreError, EngineBoundary, EngineCheckpoint, ExecutionContext,
    ExecutionStats, Executor,
};
use tex_expand::InputResolver;
use tex_lex::{InputSource, InputStack, MemoryInput, WorldInput};
use tex_out::dvi::{DviError, DviPagePlan, DviStreamWriter};
use tex_state::{
    CommittedArtifact, ContentHash, EffectRecord, FileContent, GenerationForkError,
    GenerationSubstrate, InputReadState, Universe, WorldError,
};

/// Monotonic identity of an immutable editor buffer.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RevisionId(u64);

impl RevisionId {
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// One replacement against the currently accepted revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edit {
    pub base_revision: RevisionId,
    pub expected_hash: ContentHash,
    pub range: std::ops::Range<usize>,
    pub replacement: String,
}

/// Executor-owned occurrence key for one named boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BoundaryKey {
    pub position: usize,
    pub boundary: EngineBoundary,
    pub ordinal: u32,
}

/// One directly restartable accepted-revision record.
#[derive(Clone, Debug)]
pub struct BoundaryRecord {
    revision: RevisionId,
    key: BoundaryKey,
    effect_prefix: usize,
    artifact_prefix: usize,
    checkpoint: EngineCheckpoint,
}

impl BoundaryRecord {
    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn key(&self) -> BoundaryKey {
        self.key
    }

    #[must_use]
    pub const fn artifact_prefix(&self) -> usize {
        self.artifact_prefix
    }

    #[must_use]
    pub const fn effect_prefix(&self) -> usize {
        self.effect_prefix
    }

    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.checkpoint.state_hash()
    }

    #[must_use]
    pub const fn checkpoint(&self) -> &EngineCheckpoint {
        &self.checkpoint
    }
}

/// Honest split between restart roots and detached accepted output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetentionMetrics {
    pub checkpoint_root_bytes: usize,
    pub output_bytes: usize,
    pub protected_overage_bytes: usize,
}

/// Work and reuse observed while accepting a revision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReuseMetrics {
    pub restart_boundary: Option<BoundaryKey>,
    pub convergence_boundary: Option<BoundaryKey>,
    pub pages_reused: usize,
    pub pages_retyped: usize,
    pub restart_fork_latency: Duration,
    pub reexecution_latency: Duration,
    pub splice_latency: Duration,
}

/// Detached result of one accepted editor revision.
#[derive(Clone, Debug)]
pub struct AcceptedOutput {
    pub revision: RevisionId,
    pub content_hash: ContentHash,
    pub effects: Vec<EffectRecord>,
    pub artifacts: Vec<CommittedArtifact>,
    pub dvi_pages: Vec<DviPagePlan>,
    pub history: Vec<BoundaryRecord>,
    pub reuse: ReuseMetrics,
    pub retention: RetentionMetrics,
}

impl AcceptedOutput {
    pub fn dvi_bytes(&self) -> Result<Vec<u8>, DviError> {
        let mut writer = DviStreamWriter::new(Vec::new());
        for plan in &self.dvi_pages {
            writer.write_page_plan(plan)?;
        }
        writer.finish()
    }
}

/// Long-lived incremental session. Live executor state is deliberately private.
pub struct Session {
    template: Universe,
    job_name: String,
    revision: RevisionId,
    source: String,
    content_hash: ContentHash,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    dvi_pages: Vec<DviPagePlan>,
    history: Vec<BoundaryRecord>,
    substrate: Option<GenerationSubstrate>,
    checkpoint_budget: usize,
}

impl Session {
    pub fn start(
        template: Universe,
        job_name: impl Into<String>,
        revision: RevisionId,
        source: impl Into<String>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        let source = source.into();
        Ok(Self {
            template,
            job_name: job_name.into(),
            revision,
            content_hash: ContentHash::from_bytes(source.as_bytes()),
            source,
            effects: Vec::new(),
            artifacts: Vec::new(),
            dvi_pages: Vec::new(),
            history: Vec::new(),
            substrate: None,
            checkpoint_budget,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn history(&self) -> &[BoundaryRecord] {
        &self.history
    }

    pub fn cold(&mut self) -> Result<AcceptedOutput, SessionError> {
        let run = execute_revision(&self.template, &self.job_name, &self.source)?;
        self.accept_cold(run)
    }

    /// Consumes the rollback-capable session and materializes its accepted
    /// effect history once. Further edits require constructing a new Session.
    pub fn finalize(mut self) -> Result<tex_state::World, SessionError> {
        let substrate = self
            .substrate
            .take()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        Ok(substrate.export_detached_effects(self.effects)?)
    }

    #[allow(clippy::disallowed_methods)] // Session telemetry; no TeX state observes it.
    pub fn advance(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
    ) -> Result<AcceptedOutput, SessionError> {
        self.validate_edit(next_revision, &edit)?;
        let old_source = self.source.clone();
        let old_history = self.history.clone();
        let old_effects = self.effects.clone();
        let old_artifacts = self.artifacts.clone();
        let old_pages = self.dvi_pages.clone();
        let mut next = old_source.clone();
        next.replace_range(edit.range.clone(), &edit.replacement);
        let restart_index = select_restart(&old_history, &old_source, &next, &edit);
        let map = EditMap::new(edit.range.clone(), edit.replacement.len());
        let substrate = self
            .substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        substrate.world().validate_recorded_inputs()?;
        let advance = execute_advance(
            &self.template,
            substrate,
            &self.job_name,
            &old_source,
            &next,
            &old_history,
            &old_pages,
            restart_index,
            &map,
        )?;

        let restart_fork_latency = advance.restart_fork_latency;
        let reexecution_latency = advance.reexecution_latency;
        let splice_started = Instant::now();
        let (effects, artifacts, pages, mut history, accepted_substrate, mut reuse) =
            if let Some(old_index) = advance.convergence_old_index {
                let old_effect_prefix = old_history[old_index].effect_prefix;
                let new_effect_prefix = advance
                    .new_records
                    .last()
                    .expect("convergence requires a new matching record")
                    .effect_prefix;
                let restart_effect_prefix = old_history[restart_index].effect_prefix;
                let scratch_effect_count = new_effect_prefix.saturating_sub(restart_effect_prefix);
                let mut effects = old_effects[..restart_effect_prefix].to_vec();
                effects.extend_from_slice(&advance.effects[..scratch_effect_count]);
                effects.extend_from_slice(&old_effects[old_effect_prefix..]);
                let old_prefix = old_history[old_index].artifact_prefix;
                let new_prefix = advance
                    .new_records
                    .last()
                    .expect("convergence requires a new matching record")
                    .artifact_prefix;
                let restart_artifact_prefix = old_history[restart_index].artifact_prefix;
                let scratch_artifact_count = new_prefix.saturating_sub(restart_artifact_prefix);
                let mut artifacts = old_artifacts[..restart_artifact_prefix].to_vec();
                artifacts.extend_from_slice(&advance.artifacts[..scratch_artifact_count]);
                artifacts.extend_from_slice(&old_artifacts[old_prefix..]);
                let mut pages = advance.pages_through_stop;
                pages.extend_from_slice(&old_pages[old_prefix..]);
                let mut history = Vec::with_capacity(
                    restart_index + 1 + old_history.len().saturating_sub(old_index),
                );
                for mut record in old_history[..=restart_index].iter().cloned() {
                    record.checkpoint =
                        record
                            .checkpoint
                            .rehome_unchanged_prefix(substrate, &old_source, &next)?;
                    history.push(record);
                }
                for mut record in old_history[old_index..].iter().cloned() {
                    let mapped_position = map
                        .map(record.key.position)
                        .expect("adopted suffix anchors were validated as mappable");
                    record.key.position = mapped_position;
                    record.checkpoint = record.checkpoint.rehome_converged_root(
                        substrate,
                        &old_source,
                        &next,
                        mapped_position,
                    )?;
                    record.revision = next_revision;
                    history.push(record);
                }
                let convergence_boundary = history.get(restart_index + 1).map(BoundaryRecord::key);
                (
                    effects,
                    artifacts,
                    pages,
                    history,
                    None,
                    ReuseMetrics {
                        restart_boundary: old_history.get(restart_index).map(BoundaryRecord::key),
                        convergence_boundary,
                        pages_reused: old_artifacts.len().saturating_sub(old_prefix),
                        pages_retyped: scratch_artifact_count,
                        restart_fork_latency,
                        reexecution_latency,
                        ..ReuseMetrics::default()
                    },
                )
            } else {
                let target = advance.scratch.freeze_generation();
                let mut history = Vec::with_capacity(restart_index + 1 + advance.new_records.len());
                for record in &old_history[..=restart_index] {
                    let mut record = record.clone();
                    record.checkpoint = record.checkpoint.retarget_prefix(
                        &target,
                        substrate,
                        &old_source,
                        &next,
                    )?;
                    record.revision = next_revision;
                    history.push(record);
                }
                history.extend(advance.new_records);
                let pages_retyped = advance.artifacts.len();
                let mut artifacts =
                    old_artifacts[..old_history[restart_index].artifact_prefix].to_vec();
                artifacts.extend(advance.artifacts);
                (
                    {
                        let mut effects =
                            old_effects[..old_history[restart_index].effect_prefix].to_vec();
                        effects.extend(advance.effects);
                        effects
                    },
                    artifacts,
                    advance.pages_through_stop,
                    history,
                    Some(target),
                    ReuseMetrics {
                        restart_boundary: old_history.get(restart_index).map(BoundaryRecord::key),
                        convergence_boundary: None,
                        pages_reused: 0,
                        pages_retyped,
                        restart_fork_latency,
                        reexecution_latency,
                        ..ReuseMetrics::default()
                    },
                )
            };
        reuse.splice_latency = splice_started.elapsed();
        for record in &mut history {
            record.revision = next_revision;
        }
        self.revision = next_revision;
        self.source = next;
        self.content_hash = ContentHash::from_bytes(self.source.as_bytes());
        self.effects = effects;
        self.artifacts = artifacts;
        self.dvi_pages = pages;
        if let Some(substrate) = accepted_substrate {
            self.substrate = Some(substrate);
        }
        let substrate_bytes = self
            .substrate
            .as_ref()
            .expect("accepted substrate is retained")
            .charged_bytes();
        let output_bytes = output_bytes(&self.effects, &self.artifacts);
        let (history, retention) = prune_history(
            history,
            self.checkpoint_budget,
            substrate_bytes,
            output_bytes,
        );
        self.history = history;
        Ok(self.output(reuse, retention))
    }

    fn validate_edit(&self, next_revision: RevisionId, edit: &Edit) -> Result<(), SessionError> {
        if edit.base_revision != self.revision {
            return Err(SessionError::StaleRevision {
                expected: self.revision,
                actual: edit.base_revision,
            });
        }
        if edit.expected_hash != self.content_hash {
            return Err(SessionError::ContentHashMismatch);
        }
        if next_revision <= self.revision {
            return Err(SessionError::NonMonotonicRevision);
        }
        if edit.range.start > edit.range.end
            || edit.range.end > self.source.len()
            || !self.source.is_char_boundary(edit.range.start)
            || !self.source.is_char_boundary(edit.range.end)
        {
            return Err(SessionError::InvalidEditRange);
        }
        Ok(())
    }

    fn accept_cold(&mut self, mut run: RevisionRun) -> Result<AcceptedOutput, SessionError> {
        for record in &mut run.history {
            record.revision = self.revision;
        }
        let substrate_bytes = run.substrate.charged_bytes();
        let (history, retention) = prune_history(
            run.history,
            self.checkpoint_budget,
            substrate_bytes,
            run.output_bytes,
        );
        self.history = history;
        self.effects = run.effects;
        self.artifacts = run.artifacts;
        self.dvi_pages = run.dvi_pages;
        self.substrate = Some(run.substrate);
        Ok(self.output(
            ReuseMetrics {
                pages_retyped: self.artifacts.len(),
                ..ReuseMetrics::default()
            },
            retention,
        ))
    }

    fn output(&self, reuse: ReuseMetrics, retention: RetentionMetrics) -> AcceptedOutput {
        AcceptedOutput {
            revision: self.revision,
            content_hash: self.content_hash,
            effects: self.effects.clone(),
            artifacts: self.artifacts.clone(),
            dvi_pages: self.dvi_pages.clone(),
            history: self.history.clone(),
            reuse,
            retention,
        }
    }
}

struct RevisionRun {
    history: Vec<BoundaryRecord>,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    dvi_pages: Vec<DviPagePlan>,
    output_bytes: usize,
    substrate: GenerationSubstrate,
}

#[derive(Default)]
struct HistorySink {
    records: Vec<BoundaryRecord>,
    occurrences: HashMap<(usize, EngineBoundary), u32>,
}

impl CheckpointSink for HistorySink {
    fn checkpoint(&mut self, checkpoint: EngineCheckpoint) {
        push_checkpoint(&mut self.records, &mut self.occurrences, checkpoint);
    }
}

fn execute_revision(
    template: &Universe,
    job_name: &str,
    source: &str,
) -> Result<RevisionRun, SessionError> {
    let mut universe = template.clone();
    universe.begin_retained_session()?;
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut executor = Executor::new();
    let mut sink = HistorySink::default();
    let mut input_resolver = DirectInputResolver;
    let mut font_resolver = DirectFontResolver;
    let mut context =
        ExecutionContext::with_resolvers(job_name, &mut input_resolver, &mut font_resolver);
    let ExecutionStats { dvi_pages, .. } = executor.run_with_context_and_checkpoints(
        &mut input,
        &mut universe,
        &mut context,
        &mut sink,
    )?;
    let effects = universe.world().effect_records().to_vec();
    let artifacts = universe.world().committed_artifacts().to_vec();
    let output_bytes = universe.retained_output_bytes();
    let substrate = universe.freeze_generation();
    Ok(RevisionRun {
        history: sink.records,
        effects,
        artifacts,
        dvi_pages,
        output_bytes,
        substrate,
    })
}

struct AdvanceRun {
    scratch: Universe,
    new_records: Vec<BoundaryRecord>,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    pages_through_stop: Vec<DviPagePlan>,
    convergence_old_index: Option<usize>,
    restart_fork_latency: Duration,
    reexecution_latency: Duration,
}

struct ResumeSink {
    records: Vec<BoundaryRecord>,
    occurrences: HashMap<(usize, EngineBoundary), u32>,
    expected: Vec<(usize, BoundaryKey, u64)>,
    next_expected: usize,
    convergence_old_index: Option<usize>,
    schedule_diverged: bool,
    changed_new_range: std::ops::Range<usize>,
}

impl ResumeSink {
    fn new(old: &[BoundaryRecord], restart: usize, map: &EditMap) -> Self {
        let mut occurrences = HashMap::new();
        for record in &old[..=restart] {
            occurrences
                .entry((record.key.position, record.key.boundary))
                .and_modify(|next: &mut u32| *next = (*next).max(record.key.ordinal + 1))
                .or_insert(record.key.ordinal + 1);
        }
        let expected = old[restart + 1..]
            .iter()
            .enumerate()
            .filter_map(|(offset, record)| {
                map.map(record.key.position).map(|position| {
                    (
                        restart + 1 + offset,
                        BoundaryKey {
                            position,
                            ..record.key
                        },
                        record.state_hash(),
                    )
                })
            })
            .collect();
        Self {
            records: Vec::new(),
            occurrences,
            expected,
            next_expected: 0,
            convergence_old_index: None,
            schedule_diverged: false,
            changed_new_range: map.old.start..map.old.start + map.replacement_len,
        }
    }
}

impl CheckpointSink for ResumeSink {
    fn stop_requested(&self) -> bool {
        self.convergence_old_index.is_some()
    }

    fn checkpoint(&mut self, checkpoint: EngineCheckpoint) {
        push_checkpoint(&mut self.records, &mut self.occurrences, checkpoint);
        if self.schedule_diverged {
            return;
        }
        let Some((old_index, expected_key, expected_hash)) =
            self.expected.get(self.next_expected).copied()
        else {
            self.schedule_diverged = true;
            return;
        };
        let actual = self.records.last().expect("checkpoint was just recorded");
        if self.changed_new_range.contains(&actual.key.position) {
            return;
        }
        if actual.key != expected_key {
            self.schedule_diverged = true;
            return;
        }
        self.next_expected += 1;
        if actual.state_hash() == expected_hash {
            self.convergence_old_index = Some(old_index);
        }
    }
}

fn push_checkpoint(
    records: &mut Vec<BoundaryRecord>,
    occurrences: &mut HashMap<(usize, EngineBoundary), u32>,
    checkpoint: EngineCheckpoint,
) {
    let position = checkpoint.root_anchor();
    let boundary = checkpoint.boundary();
    let ordinal = occurrences.entry((position, boundary)).or_default();
    let key = BoundaryKey {
        position,
        boundary,
        ordinal: *ordinal,
    };
    *ordinal = ordinal.saturating_add(1);
    records.push(BoundaryRecord {
        revision: RevisionId::new(0),
        key,
        effect_prefix: checkpoint.effect_prefix_len(),
        artifact_prefix: checkpoint.artifact_prefix_len(),
        checkpoint,
    });
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::disallowed_methods)] // Session telemetry; no TeX state observes it.
fn execute_advance(
    template: &Universe,
    substrate: &GenerationSubstrate,
    job_name: &str,
    old_source: &str,
    source: &str,
    old_history: &[BoundaryRecord],
    old_pages: &[DviPagePlan],
    restart: usize,
    map: &EditMap,
) -> Result<AdvanceRun, SessionError> {
    let anchor = &old_history[restart];
    let mut scratch = template.clone();
    let mut input = InputStack::new(MemoryInput::new(String::new()));
    let mut executor = Executor::new();
    let restart_fork_latency = executor.restore_editor_checkpoint(
        &mut input,
        &mut scratch,
        substrate,
        anchor.checkpoint(),
        old_source,
        source,
    )?;
    let mut sink = ResumeSink::new(old_history, restart, map);
    let mut input_resolver = DirectInputResolver;
    let mut font_resolver = DirectFontResolver;
    let mut context =
        ExecutionContext::with_resolvers(job_name, &mut input_resolver, &mut font_resolver);
    let reexecution_started = Instant::now();
    let ExecutionStats { dvi_pages, .. } = executor.resume_with_context_and_checkpoints(
        &mut input,
        &mut scratch,
        &mut context,
        &mut sink,
    )?;
    let reexecution_latency = reexecution_started.elapsed();
    let effects = scratch.world().effect_records().to_vec();
    let artifacts = scratch.world().committed_artifacts().to_vec();
    let mut pages_through_stop = old_pages[..anchor.artifact_prefix].to_vec();
    pages_through_stop.extend(dvi_pages);
    Ok(AdvanceRun {
        scratch,
        new_records: sink.records,
        effects,
        artifacts,
        pages_through_stop,
        convergence_old_index: sink.convergence_old_index,
        restart_fork_latency,
        reexecution_latency,
    })
}

fn select_restart(history: &[BoundaryRecord], old: &str, new: &str, edit: &Edit) -> usize {
    history
        .iter()
        .enumerate()
        .rev()
        .find(|(_, record)| {
            record.key.position <= edit.range.start
                && old.as_bytes().get(..record.key.position)
                    == new.as_bytes().get(..record.key.position)
        })
        .map_or(0, |(index, _)| index)
}

struct DirectInputResolver;

impl InputResolver for DirectInputResolver {
    fn open_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        _request_index: u64,
    ) -> Result<Box<dyn InputSource>, String> {
        input
            .read_input_file(Path::new(name))
            .map(WorldInput::from_content)
            .map(|source| Box::new(source) as Box<dyn InputSource>)
            .map_err(|error| error.to_string())
    }
}

struct DirectFontResolver;

impl tex_exec::FontResolver for DirectFontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
        _request_index: u64,
    ) -> Result<FileContent, String> {
        input
            .read_input_file(path)
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Debug)]
struct EditMap {
    old: std::ops::Range<usize>,
    replacement_len: usize,
}

impl EditMap {
    fn new(old: std::ops::Range<usize>, replacement_len: usize) -> Self {
        Self {
            old,
            replacement_len,
        }
    }

    fn map(&self, position: usize) -> Option<usize> {
        if position < self.old.start {
            Some(position)
        } else if position >= self.old.end {
            position
                .checked_sub(self.old.end - self.old.start)
                .and_then(|position| position.checked_add(self.replacement_len))
        } else {
            None
        }
    }
}

fn prune_history(
    mut history: Vec<BoundaryRecord>,
    budget: usize,
    substrate_bytes: usize,
    output_bytes: usize,
) -> (Vec<BoundaryRecord>, RetentionMetrics) {
    loop {
        let charged = charged_bytes(&history, substrate_bytes);
        if charged <= budget || history.len() <= 2 {
            let overage = charged.saturating_sub(budget);
            return (
                history,
                RetentionMetrics {
                    checkpoint_root_bytes: charged,
                    output_bytes,
                    protected_overage_bytes: overage,
                },
            );
        }
        let newest = history.len() - 1;
        let victim = history
            .iter()
            .enumerate()
            .find(|(index, record)| {
                *index != 0
                    && *index != newest
                    && record.key.boundary == EngineBoundary::OuterParagraphEnd
            })
            .or_else(|| {
                history.iter().enumerate().find(|(index, record)| {
                    *index != 0
                        && *index != newest
                        && record.key.boundary == EngineBoundary::ShipoutComplete
                })
            })
            .map(|(index, _)| index);
        let Some(victim) = victim else {
            let charged = charged_bytes(&history, substrate_bytes);
            return (
                history,
                RetentionMetrics {
                    checkpoint_root_bytes: charged,
                    output_bytes,
                    protected_overage_bytes: charged.saturating_sub(budget),
                },
            );
        };
        history.remove(victim);
    }
}

fn charged_bytes(history: &[BoundaryRecord], substrate_bytes: usize) -> usize {
    substrate_bytes.saturating_add(std::mem::size_of_val(history))
}

fn output_bytes(effects: &[EffectRecord], artifacts: &[CommittedArtifact]) -> usize {
    effects
        .iter()
        .map(EffectRecord::retained_bytes)
        .sum::<usize>()
        .saturating_add(
            artifacts
                .iter()
                .map(|artifact| artifact.bytes().len())
                .sum::<usize>(),
        )
}

#[derive(Debug)]
pub enum SessionError {
    StaleRevision {
        expected: RevisionId,
        actual: RevisionId,
    },
    ContentHashMismatch,
    NonMonotonicRevision,
    InvalidEditRange,
    MissingAcceptedSubstrate,
    Execute(tex_exec::ExecError),
    World(WorldError),
    Restore(EditorRestoreError),
    Fork(GenerationForkError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                f,
                "edit targets stale revision {} (accepted revision is {})",
                actual.raw(),
                expected.raw()
            ),
            Self::ContentHashMismatch => f.write_str("edit base content hash does not match"),
            Self::NonMonotonicRevision => f.write_str("new revision id must increase"),
            Self::InvalidEditRange => f.write_str("edit range is outside UTF-8 boundaries"),
            Self::MissingAcceptedSubstrate => {
                f.write_str("session has no accepted cold generation")
            }
            Self::Execute(error) => write!(f, "incremental execution failed: {error}"),
            Self::World(error) => write!(f, "incremental world failed: {error}"),
            Self::Restore(error) => write!(f, "incremental restart failed: {error}"),
            Self::Fork(error) => write!(f, "incremental generation retarget failed: {error}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<tex_exec::ExecError> for SessionError {
    fn from(value: tex_exec::ExecError) -> Self {
        Self::Execute(value)
    }
}

impl From<WorldError> for SessionError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<EditorRestoreError> for SessionError {
    fn from(value: EditorRestoreError) -> Self {
        Self::Restore(value)
    }
}

impl From<GenerationForkError> for SessionError {
    fn from(value: GenerationForkError) -> Self {
        Self::Fork(value)
    }
}

#[cfg(test)]
mod tests;
