//! Named-boundary incremental editor sessions.

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use tex_exec::{
    CheckpointSink, EditorRestoreError, EngineBoundary, EngineCheckpoint, ExecutionContext,
    ExecutionStats, Executor,
};
use tex_expand::InputResolver;
use tex_lex::{InputSource, InputStack, LayoutCursor, LayoutCursorError, MemoryInput, WorldInput};
use tex_out::dvi::{DviError, DviPagePlan, DviStreamWriter};
pub use tex_out::html::RenderedOutputId;
use tex_state::token::OriginId;
use tex_state::{
    CommittedArtifact, ContentHash, EditorLayout, EditorLayoutError, EffectRecord, FragmentStore,
    GenerationForkError, GenerationSubstrate, InputReadState, LayoutGeneration,
    LayoutResolvedOrigin, Piece, Universe, WorldError,
};

mod delivery;
mod episode;

pub use delivery::{DeliveryIdentity, SyntheticDeliveryKind};
pub use episode::TransientTokenEpisode;

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
    exact_checkpoint: Arc<OnceLock<EngineCheckpoint>>,
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

    fn exact_checkpoint<'a>(
        &'a self,
        substrate: &GenerationSubstrate,
    ) -> Result<&'a EngineCheckpoint, GenerationForkError> {
        if self.exact_checkpoint.get().is_none() {
            let checkpoint = self.checkpoint.with_exact_state_identity(substrate)?;
            let _ = self.exact_checkpoint.set(checkpoint);
        }
        Ok(self
            .exact_checkpoint
            .get()
            .expect("exact checkpoint was initialized or concurrently supplied"))
    }
}

/// Honest split between restart roots and detached accepted output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetentionMetrics {
    pub checkpoint_root_bytes: usize,
    pub memo_result_bytes: usize,
    pub diagnostic_bytes: usize,
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
    pub reexecuted_bytes: usize,
    /// Tokens accounted during reexecution, including text spans and memo-hit traces.
    pub reexecuted_tokens: usize,
    /// Tokens that required scalar main-control dispatch.
    pub reexecuted_commands: usize,
    /// Ordinary macro-body character tokens handled by the batched text path.
    pub reexecuted_macro_text_span_tokens: usize,
    /// Ordinary physical-source character tokens handled by the batched text path.
    pub reexecuted_source_text_span_tokens: usize,
    pub reexecuted_paragraphs: usize,
    pub same_history_attempts: usize,
    pub same_history_hash_mismatches: usize,
    pub trace_nodes_walked: usize,
    pub suffixes_adopted: usize,
    pub same_history_stop: SameHistoryStop,
    pub restart_fork_latency: Duration,
    pub reexecution_latency: Duration,
    pub splice_latency: Duration,
}

/// Why identical-history suffix adoption did or did not stop re-execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SameHistoryStop {
    /// A mapped schedule entry passed exact future-state verification.
    Matched,
    /// The mapped named-boundary schedule differed from the accepted revision.
    ScheduleDiverged,
    /// Every comparable boundary failed exact future-state verification.
    HashesDiverged,
    /// No old boundary after the restart anchor could be mapped and compared.
    NoComparableBoundary,
    /// This was a cold execution, so identical-history adoption was not attempted.
    #[default]
    NotAttempted,
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

/// One fully executed editor revision that has not replaced accepted session
/// state yet.
///
/// Hosts may materialize and validate its detached output before calling
/// [`Session::accept_pending`]. Dropping this value rolls the candidate back
/// without changing the accepted revision.
pub struct PendingRevision {
    session_output_id: RenderedOutputId,
    base_revision: RevisionId,
    base_content_hash: ContentHash,
    revision: RevisionId,
    source: String,
    fragments: FragmentStore,
    layout: EditorLayout,
    content_hash: ContentHash,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    dvi_pages: Vec<DviPagePlan>,
    history: Vec<BoundaryRecord>,
    substrate: PendingSubstrate,
    reuse: ReuseMetrics,
    dumped_format: bool,
    expansion_stats: tex_lex::ExpansionStats,
}

enum PendingSubstrate {
    Retained {
        scratch: Universe,
        adopted_origins: Vec<OriginId>,
    },
    Replaced(GenerationSubstrate),
}

impl PendingRevision {
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
    pub fn artifacts(&self) -> &[CommittedArtifact] {
        &self.artifacts
    }

    #[must_use]
    pub const fn reuse(&self) -> ReuseMetrics {
        self.reuse
    }

    pub fn dvi_bytes(&self) -> Result<Vec<u8>, DviError> {
        dvi_bytes(&self.dvi_pages)
    }
}

/// Typed result of resolving an accepted rendered event against a DOM revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderedSourceResult {
    Current(tex_state::ResolvedSourceLocation),
    Deleted { minted_revision: u64 },
    StaleRevision { accepted: RevisionId },
    OutputMismatch { accepted: RenderedOutputId },
}

#[derive(Debug)]
struct PageRenderMap {
    event_units: Vec<u32>,
    origins: Vec<OriginId>,
}

impl PageRenderMap {
    fn retained_bytes(&self) -> usize {
        self.event_units
            .capacity()
            .saturating_mul(size_of::<u32>())
            .saturating_add(
                self.origins
                    .capacity()
                    .saturating_mul(size_of::<OriginId>()),
            )
    }

    fn origin(&self, event: u32, unit: Option<u32>) -> Option<OriginId> {
        let event = usize::try_from(event).ok()?;
        let start = *self.event_units.get(event)? as usize;
        let end = *self.event_units.get(event.checked_add(1)?)? as usize;
        let origins = self.origins.get(start..end)?;
        let origin = match unit {
            Some(unit) => *origins.get(usize::try_from(unit).ok()?)?,
            None => origins
                .iter()
                .copied()
                .find(|origin| *origin != OriginId::UNKNOWN)?,
        };
        (origin != OriginId::UNKNOWN).then_some(origin)
    }
}

#[derive(Debug, Default)]
struct RenderMapCache {
    pages: Vec<Option<PageRenderMap>>,
    #[cfg(test)]
    page_lowerings: Vec<usize>,
}

impl RenderMapCache {
    fn retained_bytes(&self) -> usize {
        self.pages
            .capacity()
            .saturating_mul(size_of::<Option<PageRenderMap>>())
            .saturating_add(
                self.pages
                    .iter()
                    .flatten()
                    .map(PageRenderMap::retained_bytes)
                    .sum::<usize>(),
            )
    }
}

impl AcceptedOutput {
    pub fn dvi_bytes(&self) -> Result<Vec<u8>, DviError> {
        dvi_bytes(&self.dvi_pages)
    }
}

fn dvi_bytes(pages: &[DviPagePlan]) -> Result<Vec<u8>, DviError> {
    let mut writer = DviStreamWriter::new(Vec::new());
    for plan in pages {
        writer.write_page_plan(plan)?;
    }
    writer.finish()
}

/// Long-lived incremental session. Live executor state is deliberately private.
pub struct Session {
    template: Universe,
    pure_memo: tex_state::PureMemoRuntime,
    job_name: String,
    source_path: String,
    revision: RevisionId,
    output_id: RenderedOutputId,
    source: String,
    fragments: FragmentStore,
    layout: EditorLayout,
    content_hash: ContentHash,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    dvi_pages: Vec<DviPagePlan>,
    history: Vec<BoundaryRecord>,
    substrate: Option<GenerationSubstrate>,
    checkpoint_budget: usize,
    registered_inputs: BTreeMap<PathBuf, Vec<u8>>,
    accepted_retention: Option<RetentionMetrics>,
    dumped_format: bool,
    expansion_stats: tex_lex::ExpansionStats,
    render_maps: RefCell<RenderMapCache>,
}

impl Session {
    pub fn start(
        template: Universe,
        job_name: impl Into<String>,
        revision: RevisionId,
        source: impl Into<String>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        Self::start_with_source_path(
            template,
            job_name,
            "<editor>",
            revision,
            source,
            checkpoint_budget,
        )
    }

    pub fn start_with_source_path(
        template: Universe,
        job_name: impl Into<String>,
        source_path: impl Into<String>,
        revision: RevisionId,
        source: impl Into<String>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        let source = source.into();
        let source_path = source_path.into();
        let mut fragments = FragmentStore::new();
        let (fragment, _) = fragments.append(Arc::from(source.as_bytes()), revision.raw())?;
        let fragment_len = u32::try_from(source.len())
            .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
        let layout = EditorLayout::new(
            source_path.clone(),
            LayoutGeneration::new(revision.raw()),
            vec![Piece::new(fragment, 0, fragment_len)],
            &fragments,
        )?;
        let mut output_id = [0; 16];
        getrandom::fill(&mut output_id).map_err(SessionError::OutputIdentity)?;
        let mut template = template;
        let pure_memo = template.take_pure_memo_runtime();
        Ok(Self {
            template,
            pure_memo,
            job_name: job_name.into(),
            source_path,
            revision,
            output_id: RenderedOutputId::from_bytes(output_id),
            content_hash: ContentHash::from_bytes(source.as_bytes()),
            source,
            fragments,
            layout,
            effects: Vec::new(),
            artifacts: Vec::new(),
            dvi_pages: Vec::new(),
            history: Vec::new(),
            substrate: None,
            checkpoint_budget,
            registered_inputs: BTreeMap::new(),
            accepted_retention: None,
            dumped_format: false,
            expansion_stats: tex_lex::ExpansionStats::default(),
            render_maps: RefCell::default(),
        })
    }

    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn output_id(&self) -> RenderedOutputId {
        self.output_id
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

    /// Returns telemetry for the session-owned pure-query cache.
    #[must_use]
    pub fn pure_memo_stats(&self) -> tex_state::PureMemoStats {
        self.pure_memo.stats()
    }

    /// Returns live retention telemetry for the accepted session state.
    ///
    /// The accepted output keeps its point-in-time metrics, while this view
    /// also charges caches constructed by later rendered-source queries.
    #[must_use]
    pub fn retention_metrics(&self) -> Option<RetentionMetrics> {
        self.accepted_retention.map(|mut retention| {
            retention.memo_result_bytes = self.pure_memo.stats().retained_bytes;
            retention.diagnostic_bytes = self.diagnostic_retained_bytes();
            retention.output_bytes = retention
                .output_bytes
                .saturating_add(self.render_maps.borrow().retained_bytes());
            retention.protected_overage_bytes = retention
                .checkpoint_root_bytes
                .saturating_add(retention.diagnostic_bytes)
                .saturating_sub(self.checkpoint_budget);
            retention
        })
    }

    pub fn cold(&mut self) -> Result<AcceptedOutput, SessionError> {
        let mut input_resolver = DirectInputResolver;
        let mut font_resolver = DirectFontResolver;
        self.cold_with_resolvers(&mut input_resolver, &mut font_resolver)
    }

    pub fn cold_with_resolvers(
        &mut self,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
    ) -> Result<AcceptedOutput, SessionError> {
        self.cold_with_optional_image_resolver(input_resolver, font_resolver, None)
    }

    pub fn cold_with_resource_resolvers(
        &mut self,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
        image_resolver: &mut dyn tex_exec::PdfImageResolver,
    ) -> Result<AcceptedOutput, SessionError> {
        self.cold_with_optional_image_resolver(input_resolver, font_resolver, Some(image_resolver))
    }

    fn cold_with_optional_image_resolver(
        &mut self,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
        image_resolver: Option<&mut dyn tex_exec::PdfImageResolver>,
    ) -> Result<AcceptedOutput, SessionError> {
        let run = execute_revision(
            &self.template,
            &mut self.pure_memo,
            &self.job_name,
            &self.source,
            &self.fragments,
            &self.layout,
            input_resolver,
            font_resolver,
            image_resolver,
        )?;
        self.accept_cold(run)
    }

    /// Adds immutable host input to the template used by a not-yet-accepted
    /// initial revision or a retry that discovered a new resource.
    pub fn register_input_file(&mut self, path: &Path, bytes: Vec<u8>) -> Result<(), SessionError> {
        self.template
            .world_mut()
            .set_memory_file(path, bytes.clone())?;
        self.registered_inputs.insert(path.to_owned(), bytes);
        Ok(())
    }

    /// Materializes the currently accepted detached effects without consuming
    /// the checkpoints required by later edits.
    pub fn materialize_accepted_world(&self) -> Result<tex_state::World, SessionError> {
        let substrate = self
            .substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        Ok(substrate.materialize_detached_outputs(self.effects.clone(), self.artifacts.clone())?)
    }

    /// Consumes the accepted session into the reached engine state with its
    /// detached effects still uncommitted. This is the client finalization
    /// boundary for one-shot drivers.
    pub fn into_accepted_universe(mut self) -> Result<Universe, SessionError> {
        let substrate = self
            .substrate
            .take()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        Ok(substrate.into_detached_universe(self.effects, self.artifacts)?)
    }

    #[must_use]
    pub const fn accepted_dumped_format(&self) -> bool {
        self.dumped_format
    }

    #[must_use]
    pub const fn accepted_expansion_stats(&self) -> tex_lex::ExpansionStats {
        self.expansion_stats
    }

    /// Resolves one rendered HTML event/unit against the accepted revision.
    pub fn rendered_source_location(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
        output_id: RenderedOutputId,
        revision: RevisionId,
    ) -> Result<Option<RenderedSourceResult>, SessionError> {
        if output_id != self.output_id {
            return Ok(Some(RenderedSourceResult::OutputMismatch {
                accepted: self.output_id,
            }));
        }
        if revision != self.revision {
            return Ok(Some(RenderedSourceResult::StaleRevision {
                accepted: self.revision,
            }));
        }
        match self.rendered_source_origin(page, event, unit)? {
            Some(LayoutResolvedOrigin::Current {
                path,
                doc_offset_lo,
                doc_offset_hi,
                line,
                column,
            }) => Ok(Some(RenderedSourceResult::Current(
                tex_state::ResolvedSourceLocation {
                    path,
                    start: doc_offset_lo,
                    end: doc_offset_hi,
                    line,
                    column,
                },
            ))),
            Some(LayoutResolvedOrigin::Foreign) => {
                let Some(origin) = self.rendered_origin(page, event, unit)? else {
                    return Ok(None);
                };
                let substrate = self
                    .substrate
                    .as_ref()
                    .ok_or(SessionError::MissingAcceptedSubstrate)?;
                Ok(substrate
                    .resolve_origin_with_generated_path(origin, &self.source_path)
                    .map(RenderedSourceResult::Current))
            }
            Some(LayoutResolvedOrigin::Deleted { minted_revision }) => {
                Ok(Some(RenderedSourceResult::Deleted { minted_revision }))
            }
            Some(LayoutResolvedOrigin::Unknown) | None => Ok(None),
        }
    }

    /// Resolves one rendered unit with typed current/deleted editor semantics.
    pub fn rendered_source_origin(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
    ) -> Result<Option<LayoutResolvedOrigin>, SessionError> {
        let Some(origin) = self.rendered_origin(page, event, unit)? else {
            return Ok(None);
        };
        let substrate = self
            .substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        Ok(Some(substrate.resolve_layout_origin(
            origin,
            &self.fragments,
            &self.layout,
        )))
    }

    fn rendered_origin(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
    ) -> Result<Option<OriginId>, SessionError> {
        let Some(page_index) = page.checked_sub(1).map(|page| page as usize) else {
            return Ok(None);
        };
        let Some(artifact) = self.artifacts.get(page_index) else {
            return Ok(None);
        };
        let mut maps = self.render_maps.borrow_mut();
        if maps.pages.len() <= page_index {
            maps.pages.resize_with(page_index + 1, || None);
            #[cfg(test)]
            maps.page_lowerings.resize(page_index + 1, 0);
        }
        if maps.pages[page_index].is_none() {
            maps.pages[page_index] = Some(build_page_render_map(artifact, page)?);
            #[cfg(test)]
            {
                maps.page_lowerings[page_index] += 1;
            }
        }
        Ok(maps.pages[page_index]
            .as_ref()
            .and_then(|map| map.origin(event, unit)))
    }

    fn clear_render_maps(&self) {
        *self.render_maps.borrow_mut() = RenderMapCache::default();
    }

    #[cfg(test)]
    fn page_lowerings(&self, page: u32) -> usize {
        let Some(index) = page.checked_sub(1).map(|page| page as usize) else {
            return 0;
        };
        self.render_maps
            .borrow()
            .page_lowerings
            .get(index)
            .copied()
            .unwrap_or(0)
    }

    /// Consumes the rollback-capable session and materializes its accepted
    /// effect history once. Further edits require constructing a new Session.
    pub fn finalize(mut self) -> Result<tex_state::World, SessionError> {
        let substrate = self
            .substrate
            .take()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        Ok(substrate.export_detached_outputs(self.effects, self.artifacts)?)
    }

    #[allow(clippy::disallowed_methods)] // Session telemetry; no TeX state observes it.
    pub fn advance(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
    ) -> Result<AcceptedOutput, SessionError> {
        let mut input_resolver = DirectInputResolver;
        let mut font_resolver = DirectFontResolver;
        self.advance_with_resolvers(next_revision, edit, &mut input_resolver, &mut font_resolver)
    }

    pub fn advance_with_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
    ) -> Result<AcceptedOutput, SessionError> {
        let pending = self.prepare_advance_with_resolvers(
            next_revision,
            edit,
            input_resolver,
            font_resolver,
        )?;
        self.accept_pending(pending)
    }

    pub fn advance_with_resource_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
        image_resolver: &mut dyn tex_exec::PdfImageResolver,
    ) -> Result<AcceptedOutput, SessionError> {
        let pending = self.prepare_advance_with_resource_resolvers(
            next_revision,
            edit,
            input_resolver,
            font_resolver,
            image_resolver,
        )?;
        self.accept_pending(pending)
    }

    /// Executes an edit into private candidate state without changing the
    /// accepted revision. The caller may validate all downstream output and
    /// either atomically accept the candidate or drop it.
    pub fn prepare_advance_with_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
    ) -> Result<PendingRevision, SessionError> {
        self.prepare_advance_with_optional_image_resolver(
            next_revision,
            edit,
            input_resolver,
            font_resolver,
            None,
        )
    }

    pub fn prepare_advance_with_resource_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
        image_resolver: &mut dyn tex_exec::PdfImageResolver,
    ) -> Result<PendingRevision, SessionError> {
        self.prepare_advance_with_optional_image_resolver(
            next_revision,
            edit,
            input_resolver,
            font_resolver,
            Some(image_resolver),
        )
    }

    fn prepare_advance_with_optional_image_resolver(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
        image_resolver: Option<&mut dyn tex_exec::PdfImageResolver>,
    ) -> Result<PendingRevision, SessionError> {
        self.validate_edit(next_revision, &edit)?;
        self.clear_render_maps();
        let old_source = self.source.clone();
        let old_history = self.history.clone();
        let old_effects = self.effects.clone();
        let old_artifacts = self.artifacts.clone();
        let old_pages = self.dvi_pages.clone();
        let mut next = old_source.clone();
        next.replace_range(edit.range.clone(), &edit.replacement);
        let (expanded_range, expanded_replacement) = line_expanded_replacement(&old_source, &edit);
        let mut fragments = self.fragments.clone();
        let (fragment, _) = fragments.append(
            Arc::from(expanded_replacement.as_bytes()),
            next_revision.raw(),
        )?;
        let next_layout = replace_layout_range(
            &self.layout,
            &fragments,
            expanded_range,
            fragment,
            expanded_replacement.len(),
            LayoutGeneration::new(next_revision.raw()),
        )?;
        let restart_index = select_restart(&old_history, &old_source, &next, &edit);
        let map = EditMap::new(edit.range.clone(), edit.replacement.len());
        self.substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?
            .world()
            .validate_recorded_inputs()?;
        let Some(restart_index) = restart_index else {
            let mut run = execute_revision(
                &self.template,
                &mut self.pure_memo,
                &self.job_name,
                &next,
                &fragments,
                &next_layout,
                input_resolver,
                font_resolver,
                image_resolver,
            )?;
            for record in &mut run.history {
                record.revision = next_revision;
            }
            let history = retain_restorable_history(run.history, &run.substrate)?;
            let reuse = ReuseMetrics {
                pages_retyped: run.artifacts.len(),
                reexecuted_bytes: run.executed_bytes,
                reexecuted_tokens: run.executed_tokens,
                reexecuted_commands: run.executed_commands,
                reexecuted_macro_text_span_tokens: run.executed_macro_text_span_tokens,
                reexecuted_source_text_span_tokens: run.executed_source_text_span_tokens,
                reexecuted_paragraphs: run.executed_paragraphs,
                ..ReuseMetrics::default()
            };
            return Ok(PendingRevision {
                session_output_id: self.output_id,
                base_revision: self.revision,
                base_content_hash: self.content_hash,
                revision: next_revision,
                content_hash: ContentHash::from_bytes(next.as_bytes()),
                source: next,
                fragments,
                layout: next_layout,
                effects: run.effects,
                artifacts: run.artifacts,
                dvi_pages: run.dvi_pages,
                history,
                substrate: PendingSubstrate::Replaced(run.substrate),
                reuse,
            });
        };
        let substrate = self
            .substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        let mut advance = execute_advance(
            &self.template,
            &mut self.pure_memo,
            substrate,
            &self.job_name,
            &old_source,
            &next,
            &old_history,
            &old_pages,
            &fragments,
            &next_layout,
            restart_index,
            &map,
            input_resolver,
            font_resolver,
            image_resolver,
            &self.registered_inputs,
        )?;

        let restart_fork_latency = advance.restart_fork_latency;
        let reexecution_latency = advance.reexecution_latency;
        let reexecuted_bytes = advance.reexecuted_bytes;
        let reexecuted_tokens = advance.reexecuted_tokens;
        let reexecuted_commands = advance.reexecuted_commands;
        let reexecuted_macro_text_span_tokens = advance.reexecuted_macro_text_span_tokens;
        let reexecuted_source_text_span_tokens = advance.reexecuted_source_text_span_tokens;
        let reexecuted_paragraphs = advance.reexecuted_paragraphs;
        let same_history_attempts = advance.same_history_attempts;
        let same_history_hash_mismatches = advance.same_history_hash_mismatches;
        let same_history_stop = advance.same_history_stop;
        if advance.convergence_old_index.is_some() {
            self.pure_memo.discard_paragraph_generation();
        } else {
            advance
                .scratch
                .accept_paragraph_result_generation(advance.paragraph_generation_mark);
            self.pure_memo.accept_paragraph_generation();
        }
        let splice_started = Timer::start();
        let (effects, artifacts, pages, mut history, pending_substrate, mut reuse) =
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
                let adopted_origins = advance.artifacts[..scratch_artifact_count]
                    .iter()
                    .flat_map(|artifact| artifact.render_origins())
                    .flat_map(|origins| origins.iter())
                    .copied()
                    .collect::<Vec<_>>();
                let convergence_boundary = history.get(restart_index + 1).map(BoundaryRecord::key);
                (
                    effects,
                    artifacts,
                    pages,
                    history,
                    PendingSubstrate::Retained {
                        scratch: advance.scratch,
                        adopted_origins,
                    },
                    ReuseMetrics {
                        restart_boundary: old_history.get(restart_index).map(BoundaryRecord::key),
                        convergence_boundary,
                        pages_reused: old_artifacts.len().saturating_sub(old_prefix),
                        pages_retyped: scratch_artifact_count,
                        reexecuted_bytes,
                        reexecuted_tokens,
                        reexecuted_commands,
                        reexecuted_macro_text_span_tokens,
                        reexecuted_source_text_span_tokens,
                        reexecuted_paragraphs,
                        same_history_attempts,
                        same_history_hash_mismatches,
                        trace_nodes_walked: same_history_attempts,
                        suffixes_adopted: 1,
                        same_history_stop,
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
                    PendingSubstrate::Replaced(target),
                    ReuseMetrics {
                        restart_boundary: old_history.get(restart_index).map(BoundaryRecord::key),
                        convergence_boundary: None,
                        pages_reused: 0,
                        pages_retyped,
                        reexecuted_bytes,
                        reexecuted_tokens,
                        reexecuted_commands,
                        reexecuted_macro_text_span_tokens,
                        reexecuted_source_text_span_tokens,
                        reexecuted_paragraphs,
                        same_history_attempts,
                        same_history_hash_mismatches,
                        trace_nodes_walked: same_history_attempts,
                        suffixes_adopted: 0,
                        same_history_stop,
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
        let retained_substrate = match &pending_substrate {
            PendingSubstrate::Retained { .. } => self
                .substrate
                .as_ref()
                .ok_or(SessionError::MissingAcceptedSubstrate)?,
            PendingSubstrate::Replaced(substrate) => substrate,
        };
        let history = retain_restorable_history(history, retained_substrate)?;
        let content_hash = ContentHash::from_bytes(next.as_bytes());
        Ok(PendingRevision {
            session_output_id: self.output_id,
            base_revision: self.revision,
            base_content_hash: self.content_hash,
            revision: next_revision,
            source: next,
            fragments,
            layout: next_layout,
            content_hash,
            effects,
            artifacts,
            dvi_pages: pages,
            history,
            substrate: pending_substrate,
            reuse,
            dumped_format: advance.dumped_format,
            expansion_stats: advance.expansion_stats,
        })
    }

    /// Materializes detached effects for a prepared revision without
    /// publishing that revision into the session.
    pub fn materialize_pending_world(
        &self,
        pending: &PendingRevision,
    ) -> Result<tex_state::World, SessionError> {
        self.validate_pending(pending)?;
        let substrate = match &pending.substrate {
            PendingSubstrate::Retained { .. } => self
                .substrate
                .as_ref()
                .ok_or(SessionError::MissingAcceptedSubstrate)?,
            PendingSubstrate::Replaced(substrate) => substrate,
        };
        Ok(substrate
            .materialize_detached_outputs(pending.effects.clone(), pending.artifacts.clone())?)
    }

    /// Atomically replaces accepted editor state with one prepared revision.
    pub fn accept_pending(
        &mut self,
        pending: PendingRevision,
    ) -> Result<AcceptedOutput, SessionError> {
        self.validate_pending(&pending)?;
        let PendingRevision {
            revision,
            source,
            mut fragments,
            layout,
            content_hash,
            effects,
            artifacts,
            dvi_pages,
            history,
            substrate,
            reuse,
            dumped_format,
            expansion_stats,
            ..
        } = pending;

        match substrate {
            PendingSubstrate::Retained {
                scratch,
                adopted_origins,
            } => self
                .substrate
                .as_mut()
                .ok_or(SessionError::MissingAcceptedSubstrate)?
                .retain_artifact_origins_from_fork(&scratch, &adopted_origins, &self.source_path)?,
            PendingSubstrate::Replaced(substrate) => self.substrate = Some(substrate),
        }

        let substrate_bytes = self
            .substrate
            .as_ref()
            .expect("prepared revisions retain an accepted substrate")
            .charged_bytes();
        let output_bytes = output_bytes(&effects, &artifacts);
        let oldest_revision = oldest_retained_revision(&history, revision);
        fragments.prune_for_layout(&layout, revision.raw(), oldest_revision.raw());
        let diagnostic_bytes = fragments
            .retained_bytes()
            .saturating_add(layout.retained_bytes());
        let (history, mut retention) = prune_history(
            history,
            self.checkpoint_budget,
            substrate_bytes,
            diagnostic_bytes,
            output_bytes,
        );
        retention.memo_result_bytes = self.pure_memo.stats().retained_bytes;
        let pruned_oldest_revision = oldest_retained_revision(&history, revision);
        if pruned_oldest_revision > oldest_revision
            && fragments.prune_for_layout(&layout, revision.raw(), pruned_oldest_revision.raw()) > 0
        {
            retention.diagnostic_bytes = fragments
                .retained_bytes()
                .saturating_add(layout.retained_bytes());
            retention.protected_overage_bytes = retention
                .checkpoint_root_bytes
                .saturating_add(retention.diagnostic_bytes)
                .saturating_sub(self.checkpoint_budget);
        }

        self.clear_render_maps();
        self.revision = revision;
        self.source = source;
        self.fragments = fragments;
        self.layout = layout;
        self.content_hash = content_hash;
        self.effects = effects;
        self.artifacts = artifacts;
        self.dvi_pages = dvi_pages;
        self.history = history;
        self.dumped_format = dumped_format;
        self.expansion_stats = expansion_stats;
        self.accepted_retention = Some(retention);
        Ok(self.output(reuse, retention))
    }

    fn validate_pending(&self, pending: &PendingRevision) -> Result<(), SessionError> {
        if pending.session_output_id != self.output_id
            || pending.base_revision != self.revision
            || pending.base_content_hash != self.content_hash
        {
            return Err(SessionError::StaleRevision {
                expected: self.revision,
                actual: pending.base_revision,
            });
        }
        Ok(())
    }

    pub fn validate_edit(
        &self,
        next_revision: RevisionId,
        edit: &Edit,
    ) -> Result<(), SessionError> {
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
        self.clear_render_maps();
        for record in &mut run.history {
            record.revision = self.revision;
        }
        run.history = retain_restorable_history(run.history, &run.substrate)?;
        let substrate_bytes = run.substrate.charged_bytes();
        let diagnostic_bytes = self.diagnostic_retained_bytes();
        let (history, mut retention) = prune_history(
            run.history,
            self.checkpoint_budget,
            substrate_bytes,
            diagnostic_bytes,
            run.output_bytes,
        );
        retention.memo_result_bytes = self.pure_memo.stats().retained_bytes;
        self.history = history;
        self.effects = run.effects;
        self.artifacts = run.artifacts;
        self.dvi_pages = run.dvi_pages;
        self.dumped_format = run.dumped_format;
        self.expansion_stats = run.expansion_stats;
        self.substrate = Some(run.substrate);
        self.accepted_retention = Some(retention);
        Ok(self.output(
            ReuseMetrics {
                pages_retyped: self.artifacts.len(),
                reexecuted_bytes: run.executed_bytes,
                reexecuted_tokens: run.executed_tokens,
                reexecuted_commands: run.executed_commands,
                reexecuted_macro_text_span_tokens: run.executed_macro_text_span_tokens,
                reexecuted_source_text_span_tokens: run.executed_source_text_span_tokens,
                reexecuted_paragraphs: run.executed_paragraphs,
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

    fn diagnostic_retained_bytes(&self) -> usize {
        self.fragments
            .retained_bytes()
            .saturating_add(self.layout.retained_bytes())
    }
}

fn retain_restorable_history(
    history: Vec<BoundaryRecord>,
    substrate: &GenerationSubstrate,
) -> Result<Vec<BoundaryRecord>, SessionError> {
    let mut retained = Vec::with_capacity(history.len());
    for record in history {
        match record.checkpoint.validate_retained_by(substrate) {
            Ok(()) => retained.push(record),
            Err(GenerationForkError::InvalidatedSnapshot) => {}
            Err(error) => return Err(SessionError::Fork(error)),
        }
    }
    if retained.is_empty() {
        return Err(SessionError::MissingAcceptedSubstrate);
    }
    Ok(retained)
}

fn build_page_render_map(
    artifact: &CommittedArtifact,
    page: u32,
) -> Result<PageRenderMap, SessionError> {
    let page_artifact = tex_out::PageArtifact::from_bytes(artifact.bytes())
        .map_err(|error| SessionError::RenderSource(error.to_string()))?;
    let positioned = tex_out::positioned::lower_page(&page_artifact, page)
        .map_err(|error| SessionError::RenderSource(error.to_string()))?;
    let mut event_units = Vec::with_capacity(positioned.events.len().saturating_add(1));
    let mut origins = Vec::new();
    event_units.push(0);
    for event in positioned.events {
        if let tex_out::positioned::PositionedEvent::TextRun(run) = event {
            for source in run.sources {
                origins.push(
                    source
                        .and_then(|source| {
                            artifact
                                .render_origins()
                                .get(source.node_ordinal as usize)
                                .and_then(|origins| origins.get(source.source_index as usize))
                                .copied()
                        })
                        .unwrap_or(OriginId::UNKNOWN),
                );
            }
        }
        event_units.push(u32::try_from(origins.len()).map_err(|_| {
            SessionError::RenderSource("rendered source map exceeds u32 capacity".to_owned())
        })?);
    }
    Ok(PageRenderMap {
        event_units,
        origins,
    })
}

struct RevisionRun {
    history: Vec<BoundaryRecord>,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    dvi_pages: Vec<DviPagePlan>,
    output_bytes: usize,
    substrate: GenerationSubstrate,
    dumped_format: bool,
    expansion_stats: tex_lex::ExpansionStats,
    executed_bytes: usize,
    executed_tokens: usize,
    executed_commands: usize,
    executed_macro_text_span_tokens: usize,
    executed_source_text_span_tokens: usize,
    executed_paragraphs: usize,
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

#[allow(clippy::too_many_arguments)]
fn execute_revision(
    template: &Universe,
    pure_memo: &mut tex_state::PureMemoRuntime,
    job_name: &str,
    source: &str,
    fragments: &FragmentStore,
    layout: &EditorLayout,
    input_resolver: &mut dyn InputResolver,
    font_resolver: &mut dyn tex_exec::FontResolver,
    image_resolver: Option<&mut dyn tex_exec::PdfImageResolver>,
) -> Result<RevisionRun, SessionError> {
    let mut universe = template.clone();
    universe.begin_retained_session()?;
    let mut input = InputStack::new(MemoryInput::new(source));
    universe.install_editor_fragments(fragments, layout)?;
    universe.set_root_editor_content_hash(ContentHash::from_bytes(source.as_bytes()));
    input
        .install_root_layout_cursor(LayoutCursor::new(layout, fragments)?)
        .expect("new editor input has a root source");
    let mut executor = Executor::new();
    let mut sink = HistorySink::default();
    let mut context = match image_resolver {
        Some(image_resolver) => ExecutionContext::with_resource_resolvers(
            job_name,
            input_resolver,
            font_resolver,
            image_resolver,
        ),
        None => ExecutionContext::with_resolvers(job_name, input_resolver, font_resolver),
    };
    let paragraph_generation_mark = universe.paragraph_result_generation_mark();
    pure_memo.begin_paragraph_generation(false);
    universe.install_pure_memo_runtime(std::mem::take(pure_memo));
    let execution_result = executor.run_with_context_and_checkpoints(
        &mut input,
        &mut universe,
        &mut context,
        &mut sink,
    );
    *pure_memo = universe.take_pure_memo_runtime();
    if execution_result.is_err() {
        pure_memo.discard_paragraph_generation();
    }
    let ExecutionStats {
        dvi_pages,
        dumped_format,
        delivered_tokens,
        main_control_dispatches,
        macro_text_span_tokens,
        source_text_span_tokens,
        ..
    } = execution_result?;
    let expansion_stats = input.expansion_stats();
    universe.accept_paragraph_result_generation(paragraph_generation_mark);
    pure_memo.accept_paragraph_generation();
    let effects = universe.world().effect_records().to_vec();
    let artifacts = universe.world().committed_artifacts().to_vec();
    let output_bytes = universe.retained_output_bytes();
    let substrate = universe.freeze_generation();
    let executed_paragraphs = sink
        .records
        .iter()
        .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
        .count();
    Ok(RevisionRun {
        history: sink.records,
        effects,
        artifacts,
        dvi_pages,
        output_bytes,
        substrate,
        dumped_format,
        expansion_stats,
        executed_bytes: source.len(),
        executed_tokens: delivered_tokens,
        executed_commands: main_control_dispatches,
        executed_macro_text_span_tokens: macro_text_span_tokens,
        executed_source_text_span_tokens: source_text_span_tokens,
        executed_paragraphs,
    })
}

struct AdvanceRun {
    scratch: Universe,
    paragraph_generation_mark: usize,
    new_records: Vec<BoundaryRecord>,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    pages_through_stop: Vec<DviPagePlan>,
    convergence_old_index: Option<usize>,
    reexecuted_bytes: usize,
    reexecuted_tokens: usize,
    reexecuted_commands: usize,
    reexecuted_macro_text_span_tokens: usize,
    reexecuted_source_text_span_tokens: usize,
    reexecuted_paragraphs: usize,
    same_history_attempts: usize,
    same_history_hash_mismatches: usize,
    same_history_stop: SameHistoryStop,
    restart_fork_latency: Duration,
    reexecution_latency: Duration,
    dumped_format: bool,
    expansion_stats: tex_lex::ExpansionStats,
}

struct ResumeSink<'a> {
    records: Vec<BoundaryRecord>,
    occurrences: HashMap<(usize, EngineBoundary), u32>,
    expected: Vec<(usize, BoundaryKey, BoundaryRecord)>,
    next_expected: usize,
    convergence_old_index: Option<usize>,
    schedule_diverged: bool,
    changed_new_range: std::ops::Range<usize>,
    same_history_attempts: usize,
    same_history_hash_mismatches: usize,
    substrate: &'a GenerationSubstrate,
}

impl<'a> ResumeSink<'a> {
    fn new(
        old: &[BoundaryRecord],
        restart: usize,
        map: &EditMap,
        substrate: &'a GenerationSubstrate,
    ) -> Self {
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
                        record.clone(),
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
            same_history_attempts: 0,
            same_history_hash_mismatches: 0,
            substrate,
        }
    }

    fn prospective_key(&self, boundary: EngineBoundary, position: usize) -> BoundaryKey {
        BoundaryKey {
            position,
            boundary,
            ordinal: self
                .occurrences
                .get(&(position, boundary))
                .copied()
                .unwrap_or_default(),
        }
    }
}

impl CheckpointSink for ResumeSink<'_> {
    fn wants_exact_state_identity(&self, boundary: EngineBoundary, root_anchor: usize) -> bool {
        if self.schedule_diverged || self.changed_new_range.contains(&root_anchor) {
            return false;
        }
        self.expected
            .get(self.next_expected)
            .is_some_and(|(_, expected, _)| {
                self.prospective_key(boundary, root_anchor) == *expected
            })
    }

    fn stop_requested(&self) -> bool {
        self.convergence_old_index.is_some()
    }

    fn checkpoint(&mut self, checkpoint: EngineCheckpoint) {
        push_checkpoint(&mut self.records, &mut self.occurrences, checkpoint);
        if self.schedule_diverged {
            return;
        }
        let Some((old_index, expected_key, expected_record)) =
            self.expected.get(self.next_expected)
        else {
            self.schedule_diverged = true;
            return;
        };
        let actual = self.records.last().expect("checkpoint was just recorded");
        if self.changed_new_range.contains(&actual.key.position) {
            return;
        }
        if actual.key != *expected_key {
            self.schedule_diverged = true;
            return;
        }
        self.next_expected += 1;
        self.same_history_attempts += 1;
        let exact_match = actual.checkpoint().has_exact_state_identity()
            && expected_record
                .exact_checkpoint(self.substrate)
                .is_ok_and(|expected| actual.checkpoint().exact_future_state_matches(expected));
        if exact_match {
            self.convergence_old_index = Some(*old_index);
        } else {
            self.same_history_hash_mismatches += 1;
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
    let exact_checkpoint = Arc::new(OnceLock::new());
    if checkpoint.has_exact_state_identity() {
        let _ = exact_checkpoint.set(checkpoint.clone());
    }
    records.push(BoundaryRecord {
        revision: RevisionId::new(0),
        key,
        effect_prefix: checkpoint.effect_prefix_len(),
        artifact_prefix: checkpoint.artifact_prefix_len(),
        checkpoint,
        exact_checkpoint,
    });
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::disallowed_methods)] // Session telemetry; no TeX state observes it.
fn execute_advance(
    template: &Universe,
    pure_memo: &mut tex_state::PureMemoRuntime,
    substrate: &GenerationSubstrate,
    job_name: &str,
    old_source: &str,
    source: &str,
    old_history: &[BoundaryRecord],
    old_pages: &[DviPagePlan],
    fragments: &FragmentStore,
    layout: &EditorLayout,
    restart: usize,
    map: &EditMap,
    input_resolver: &mut dyn InputResolver,
    font_resolver: &mut dyn tex_exec::FontResolver,
    image_resolver: Option<&mut dyn tex_exec::PdfImageResolver>,
    registered_inputs: &BTreeMap<PathBuf, Vec<u8>>,
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
        fragments,
        layout,
        LayoutCursor::new(layout, fragments)?,
    )?;
    for (path, bytes) in registered_inputs {
        scratch.world_mut().set_memory_file(path, bytes.clone())?;
    }
    let mut sink = ResumeSink::new(old_history, restart, map, substrate);
    let mut context = match image_resolver {
        Some(image_resolver) => ExecutionContext::with_resource_resolvers(
            job_name,
            input_resolver,
            font_resolver,
            image_resolver,
        ),
        None => ExecutionContext::with_resolvers(job_name, input_resolver, font_resolver),
    };
    let reexecution_started = Timer::start();
    let paragraph_generation_mark = scratch.paragraph_result_generation_mark();
    pure_memo.begin_paragraph_generation(true);
    scratch.install_pure_memo_runtime(std::mem::take(pure_memo));
    let execution_result = executor.resume_with_context_and_checkpoints(
        &mut input,
        &mut scratch,
        &mut context,
        &mut sink,
    );
    *pure_memo = scratch.take_pure_memo_runtime();
    if execution_result.is_err() {
        pure_memo.discard_paragraph_generation();
    }
    let ExecutionStats {
        dvi_pages,
        dumped_format,
        delivered_tokens,
        main_control_dispatches,
        macro_text_span_tokens,
        source_text_span_tokens,
        ..
    } = execution_result?;
    let reexecution_latency = reexecution_started.elapsed();
    let expansion_stats = input.expansion_stats();
    let reexecuted_paragraphs = sink
        .records
        .iter()
        .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
        .count();
    let reexecuted_through = sink
        .records
        .last()
        .map_or(source.len(), |record| record.key.position);
    let reexecuted_bytes = reexecuted_through.saturating_sub(anchor.key.position);
    let same_history_stop = if sink.convergence_old_index.is_some() {
        SameHistoryStop::Matched
    } else if sink.schedule_diverged {
        SameHistoryStop::ScheduleDiverged
    } else if sink.same_history_attempts > 0 {
        SameHistoryStop::HashesDiverged
    } else {
        SameHistoryStop::NoComparableBoundary
    };
    let expansion_stats = input.expansion_stats();
    let effects = scratch.world().effect_records().to_vec();
    let artifacts = scratch.world().committed_artifacts().to_vec();
    let mut pages_through_stop = old_pages[..anchor.artifact_prefix].to_vec();
    pages_through_stop.extend(dvi_pages);
    Ok(AdvanceRun {
        scratch,
        paragraph_generation_mark,
        new_records: sink.records,
        effects,
        artifacts,
        pages_through_stop,
        convergence_old_index: sink.convergence_old_index,
        reexecuted_bytes,
        reexecuted_tokens: delivered_tokens,
        reexecuted_commands: main_control_dispatches,
        reexecuted_macro_text_span_tokens: macro_text_span_tokens,
        reexecuted_source_text_span_tokens: source_text_span_tokens,
        reexecuted_paragraphs,
        same_history_attempts: sink.same_history_attempts,
        same_history_hash_mismatches: sink.same_history_hash_mismatches,
        same_history_stop,
        restart_fork_latency,
        reexecution_latency,
        dumped_format,
        expansion_stats,
    })
}

fn line_expanded_replacement(old: &str, edit: &Edit) -> (std::ops::Range<usize>, String) {
    let start = old.as_bytes()[..edit.range.start]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    let end = if edit.range.start != edit.range.end
        && old.as_bytes().get(edit.range.end.wrapping_sub(1)) == Some(&b'\n')
    {
        edit.range.end
    } else {
        old.as_bytes()[edit.range.end..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(old.len(), |position| edit.range.end + position + 1)
    };
    let mut replacement = String::with_capacity(
        edit.range.start - start + edit.replacement.len() + end - edit.range.end,
    );
    replacement.push_str(&old[start..edit.range.start]);
    replacement.push_str(&edit.replacement);
    replacement.push_str(&old[edit.range.end..end]);
    (start..end, replacement)
}

fn replace_layout_range(
    old: &EditorLayout,
    fragments: &FragmentStore,
    replaced: std::ops::Range<usize>,
    replacement: tex_state::FragmentId,
    replacement_len: usize,
    generation: LayoutGeneration,
) -> Result<EditorLayout, SessionError> {
    let replaced_start = u64::try_from(replaced.start)
        .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
    let replaced_end = u64::try_from(replaced.end)
        .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
    let replacement_len = u32::try_from(replacement_len)
        .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
    let mut pieces = Vec::with_capacity(old.pieces().len().saturating_add(2));
    let mut inserted = false;
    for (index, piece) in old.pieces().iter().enumerate() {
        if piece.start() == piece.end() {
            continue;
        }
        let doc_start = old.doc_starts()[index];
        let doc_end = doc_start + u64::from(piece.end() - piece.start());
        if doc_end <= replaced_start {
            pieces.push(piece.clone());
            continue;
        }
        if doc_start >= replaced_end {
            if !inserted {
                pieces.push(Piece::new(replacement, 0, replacement_len));
                inserted = true;
            }
            pieces.push(piece.clone());
            continue;
        }
        if doc_start < replaced_start {
            let left_end = piece.start()
                + u32::try_from(replaced_start - doc_start)
                    .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
            pieces.push(Piece::new(piece.fragment(), piece.start(), left_end));
        }
        if !inserted {
            pieces.push(Piece::new(replacement, 0, replacement_len));
            inserted = true;
        }
        if doc_end > replaced_end {
            let right_start = piece.start()
                + u32::try_from(replaced_end - doc_start)
                    .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
            pieces.push(Piece::new(piece.fragment(), right_start, piece.end()));
        }
    }
    if !inserted {
        pieces.push(Piece::new(replacement, 0, replacement_len));
    }
    Ok(EditorLayout::new(
        old.path(),
        generation,
        pieces,
        fragments,
    )?)
}

fn select_restart(history: &[BoundaryRecord], old: &str, new: &str, edit: &Edit) -> Option<usize> {
    history
        .iter()
        .enumerate()
        .rev()
        .find(|(_, record)| {
            record.key.position <= edit.range.start
                && old.as_bytes().get(..record.key.position)
                    == new.as_bytes().get(..record.key.position)
        })
        .map(|(index, _)| index)
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

struct Timer {
    #[cfg(not(target_arch = "wasm32"))]
    started: Instant,
}

impl Timer {
    #[allow(clippy::disallowed_methods)] // Session telemetry; no TeX state observes it.
    fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            started: Instant::now(),
        }
    }

    fn elapsed(&self) -> Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Duration::ZERO
        }
    }
}

impl tex_exec::FontResolver for DirectFontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
        _request_index: u64,
    ) -> Result<tex_exec::FontSource, String> {
        input
            .read_input_file(path)
            .map(|metrics| tex_exec::FontSource::Tfm {
                metrics,
                opentype: None,
            })
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
    diagnostic_bytes: usize,
    output_bytes: usize,
) -> (Vec<BoundaryRecord>, RetentionMetrics) {
    loop {
        let checkpoint_root_bytes = charged_bytes(&history, substrate_bytes);
        let charged = checkpoint_root_bytes.saturating_add(diagnostic_bytes);
        if charged <= budget || history.len() <= 2 {
            let overage = charged.saturating_sub(budget);
            return (
                history,
                RetentionMetrics {
                    checkpoint_root_bytes,
                    memo_result_bytes: 0,
                    diagnostic_bytes,
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
            let checkpoint_root_bytes = charged_bytes(&history, substrate_bytes);
            let charged = checkpoint_root_bytes.saturating_add(diagnostic_bytes);
            return (
                history,
                RetentionMetrics {
                    checkpoint_root_bytes,
                    memo_result_bytes: 0,
                    diagnostic_bytes,
                    output_bytes,
                    protected_overage_bytes: charged.saturating_sub(budget),
                },
            );
        };
        history.remove(victim);
    }
}

fn oldest_retained_revision(history: &[BoundaryRecord], fallback: RevisionId) -> RevisionId {
    history
        .iter()
        .map(BoundaryRecord::revision)
        .min()
        .unwrap_or(fallback)
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
                .map(|artifact| {
                    artifact
                        .bytes()
                        .len()
                        .saturating_add(artifact.render_provenance_bytes())
                })
                .sum::<usize>(),
        )
}

#[derive(Debug)]
pub enum SessionError {
    OutputIdentity(getrandom::Error),
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
    Fragment(tex_state::source_map::SourceMapError),
    Layout(EditorLayoutError),
    LayoutCursor(LayoutCursorError),
    RenderSource(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutputIdentity(error) => {
                write!(f, "could not create rendered-output identity: {error}")
            }
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
            Self::Fragment(error) => write!(f, "editor fragment allocation failed: {error}"),
            Self::Layout(error) => write!(f, "editor layout update failed: {error}"),
            Self::LayoutCursor(error) => write!(f, "editor layout cursor failed: {error}"),
            Self::RenderSource(error) => write!(f, "rendered source query failed: {error}"),
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

impl From<tex_state::source_map::SourceMapError> for SessionError {
    fn from(value: tex_state::source_map::SourceMapError) -> Self {
        Self::Fragment(value)
    }
}

impl From<EditorLayoutError> for SessionError {
    fn from(value: EditorLayoutError) -> Self {
        Self::Layout(value)
    }
}

impl From<LayoutCursorError> for SessionError {
    fn from(value: LayoutCursorError) -> Self {
        Self::LayoutCursor(value)
    }
}

#[cfg(test)]
mod tests;
