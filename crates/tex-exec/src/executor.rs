use std::collections::{BTreeMap, VecDeque};
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tex_expand::{
    EngineStateSnapshot, InputResolver, ReadRecorder, ResourceLookup, ResourceResult,
    get_x_token_with_context,
};
use tex_lex::{InputStack, InputStackSnapshot};
use tex_out::dvi::DviPagePlan;
use tex_state::ids::TokenListId;
use tex_state::node::Node;
use tex_state::token::TracedTokenWord;
use tex_state::{
    FileContent, InputReadState, InputSummary, ParagraphBarrierReason, TokenListReplayKind,
    Universe,
};

use crate::checkpoint::{CheckpointSink, EngineBoundary, EngineSession, NoopCheckpointSink};
use crate::dispatch::{dispatch_delivered_token_with_context, unimplemented_typesetting};
use crate::mode::ignored_depth;
use crate::output;
use crate::timing::TelemetryTimer;
use crate::vertical::is_outer_vertical;
use crate::{DispatchAction, ExecError, ExecutionStats, ModeNest, assignments};

fn report_recoverable_expansion_diagnostics(
    execution: &mut crate::ExecutionContext<'_>,
    stores: &mut Universe,
) {
    for diagnostic in execution.take_recoverable_diagnostics() {
        match diagnostic {
            tex_expand::RecoverableExpansionDiagnostic::MacroDoesNotMatchDefinition {
                macro_name,
                ..
            } => stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                &format!("\n! Use of {macro_name} doesn't match its definition.\n"),
            ),
        }
    }
}

/// Object-safe host boundary used only by the `\font` assignment.
pub trait FontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
        request_index: u64,
    ) -> ResourceResult<FontSource>;
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PdfImagePageBox {
    #[default]
    Crop,
    Media,
    Bleed,
    Trim,
    Art,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfImageRequest {
    pub name: String,
    pub page: u32,
    pub page_box: PdfImagePageBox,
    pub resolution: u32,
}

/// Host boundary for loading and validating `\pdfximage` resources.
pub trait PdfImageResolver {
    fn open_image(
        &mut self,
        input: &mut dyn InputReadState,
        request: &PdfImageRequest,
        request_index: u64,
    ) -> ResourceResult<tex_state::PdfExternalImageSource>;
}

/// Font inputs selected atomically by the host.
pub enum FontSource {
    /// Classic TFM metrics, optionally paired with an OpenType program for
    /// Unicode character queries and shaping.
    Tfm {
        metrics: FileContent,
        opentype: Option<tex_fonts::OpenTypeProgramSelection>,
    },
    /// A validated OpenType program selected without any TFM dependency.
    OpenType(tex_fonts::OpenTypeProgramSelection),
}

/// Concrete execution-session context shared by stomach operations.
///
/// Expansion scanners see this only through its concrete dereference to
/// [`tex_expand::ExpansionContext`]; font resolution remains an execution-only
/// operation and is invoked solely by `\font` assignment.
#[derive(Clone)]
pub(crate) struct PendingParagraphMemo {
    pub(crate) break_dependency_ordinals: Vec<u32>,
    pub(crate) prev_graf: Option<i32>,
    pub(crate) continuation: ParagraphContinuation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParagraphContinuation {
    End,
    Display,
}

#[derive(Clone)]
pub(crate) struct ColdParagraphRecording {
    pub(crate) effect_start: usize,
    pub(crate) starting_span: Option<tex_state::RootSpanId>,
    pub(crate) starting_root_span: Option<tex_state::RootSpanId>,
    pub(crate) starting_input: Option<tex_state::InputSummary>,
    pub(crate) starting_input_identity: Option<u64>,
    pub(crate) starting_group_depth: u32,
    pub(crate) delivered_tokens: usize,
    pub(crate) inline_math: Option<InlineMathReads>,
    /// Box registers first supplied by this paragraph below its entry group.
    /// Their contents remain source-proven only while that group is active.
    pub(crate) local_boxes: Vec<(u16, u32)>,
    /// Stomach-side reads which do not pass through expansion scanners.
    pub(crate) dependencies: Vec<tex_state::DependencyKey>,
    pub(crate) barriers: std::collections::BTreeSet<ParagraphBarrierReason>,
}

#[derive(Clone, Default)]
pub(crate) struct InlineMathReads {
    pub(crate) mathcodes: Vec<char>,
    pub(crate) delcodes: Vec<char>,
    /// One bit per `(size, family)` binding, in `size * 16 + family` order.
    pub(crate) family_mask: u64,
}

#[derive(Clone)]
pub(crate) struct CachedParagraphDependency {
    pub(crate) observation: tex_state::ObservedDependency,
    /// Ordinal in the speculative history currently being recorded. A cache
    /// entry populated by validation receives an ordinal only if execution
    /// later falls back to cold publication.
    pub(crate) recorded_ordinal: Option<u32>,
}

/// Expansion and paragraph state that survives between bounded executor calls.
pub struct ExecutionState {
    expansion: tex_expand::ExpansionSessionState,
    pending_paragraph_memo: Option<PendingParagraphMemo>,
    paragraph_memo_barrier: bool,
    paragraph_memo_disabled_for_run: bool,
    cold_paragraph_recording: Option<ColdParagraphRecording>,
    paragraph_dependency_cache:
        ahash::AHashMap<tex_state::DependencyKey, CachedParagraphDependency>,
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self {
            expansion: tex_expand::ExpansionSessionState::default(),
            pending_paragraph_memo: None,
            paragraph_memo_barrier: false,
            paragraph_memo_disabled_for_run: false,
            cold_paragraph_recording: None,
            paragraph_dependency_cache: ahash::AHashMap::new(),
        }
    }
}

impl ExecutionState {
    #[must_use]
    pub fn with_expansion_fuel(mut self, fuel: u64) -> Self {
        self.expansion = self.expansion.with_fuel(fuel);
        self
    }

    fn snapshot(&self) -> ExecutionStateSnapshot {
        ExecutionStateSnapshot {
            expansion: self.expansion.snapshot(),
            pending_paragraph_memo: self.pending_paragraph_memo.clone(),
            paragraph_memo_barrier: self.paragraph_memo_barrier,
            paragraph_memo_disabled_for_run: self.paragraph_memo_disabled_for_run,
            cold_paragraph_recording: self.cold_paragraph_recording.clone(),
            paragraph_dependency_cache: self.paragraph_dependency_cache.clone(),
        }
    }

    fn rollback(&mut self, snapshot: ExecutionStateSnapshot) {
        self.expansion.rollback(snapshot.expansion);
        self.pending_paragraph_memo = snapshot.pending_paragraph_memo;
        self.paragraph_memo_barrier = snapshot.paragraph_memo_barrier;
        self.paragraph_memo_disabled_for_run = snapshot.paragraph_memo_disabled_for_run;
        self.cold_paragraph_recording = snapshot.cold_paragraph_recording;
        self.paragraph_dependency_cache = snapshot.paragraph_dependency_cache;
    }
}

struct ExecutionStateSnapshot {
    expansion: tex_expand::ExpansionSessionSnapshot,
    pending_paragraph_memo: Option<PendingParagraphMemo>,
    paragraph_memo_barrier: bool,
    paragraph_memo_disabled_for_run: bool,
    cold_paragraph_recording: Option<ColdParagraphRecording>,
    paragraph_dependency_cache:
        ahash::AHashMap<tex_state::DependencyKey, CachedParagraphDependency>,
}

/// Host capabilities borrowed for exactly one [`ExecutionRun::step`] call.
pub struct ExecutionServices<'call, 'host> {
    pub input: &'call mut InputStack,
    pub stores: &'call mut Universe,
    input_resolver: Option<&'host mut dyn InputResolver>,
    font_resolver: Option<&'host mut dyn FontResolver>,
    image_resolver: Option<&'host mut dyn PdfImageResolver>,
    recorder: Option<&'host mut dyn ReadRecorder>,
    checkpoints: Option<&'call mut dyn CheckpointSink>,
}

impl<'a> ExecutionServices<'a, 'a> {
    pub fn new(input: &'a mut InputStack, stores: &'a mut Universe) -> Self {
        Self {
            input,
            stores,
            input_resolver: None,
            font_resolver: None,
            image_resolver: None,
            recorder: None,
            checkpoints: None,
        }
    }

    #[must_use]
    pub fn with_input_resolver(mut self, resolver: &'a mut dyn InputResolver) -> Self {
        self.input_resolver = Some(resolver);
        self
    }

    #[must_use]
    pub fn with_font_resolver(mut self, resolver: &'a mut dyn FontResolver) -> Self {
        self.font_resolver = Some(resolver);
        self
    }

    #[must_use]
    pub fn with_image_resolver(mut self, resolver: &'a mut dyn PdfImageResolver) -> Self {
        self.image_resolver = Some(resolver);
        self
    }

    #[must_use]
    pub fn recording(mut self, recorder: &'a mut dyn ReadRecorder) -> Self {
        self.recorder = Some(recorder);
        self
    }

    #[must_use]
    pub fn with_checkpoints(mut self, checkpoints: &'a mut dyn CheckpointSink) -> Self {
        self.checkpoints = Some(checkpoints);
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionStep {
    JobStart,
    MainControl,
    FinishEnd,
    Finalize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionLifecycle {
    Created,
    Ready,
    Awaiting,
    Finishing,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceSite {
    Expansion,
    MainControl,
    ParagraphFinish,
    LineBuild,
    PageBuild,
    Shipout,
    FontLoad,
    ExternalImageParse,
    EndFinalization,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceSuspension {
    pub requests: Vec<tex_expand::ResourceNeed>,
    pub site: ResourceSite,
    pub serial: u64,
    pub blocked_step: ExecutionStep,
}

#[derive(Clone, Debug)]
pub struct ExecutionProgress {
    pub next_step: ExecutionStep,
    pub checkpoints: Vec<crate::EngineCheckpoint>,
    pub stop_requested: bool,
}

/// Monotonic work and retry telemetry for one owned executor run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutionTelemetry {
    pub cold_starts: u64,
    pub advance_calls: u64,
    pub suspensions: u64,
    pub local_step_retries: u64,
    pub replayed_delivered_tokens: u64,
    pub replayed_dispatches: u64,
    pub cumulative_fuel: u64,
    pub engine_time: Duration,
}

#[derive(Debug)]
pub enum ExecutionStepResult {
    Progress(ExecutionProgress),
    AwaitingResources(ResourceSuspension),
    Complete(ExecutionStats),
    Failed(ExecError),
    Cancelled,
}

/// Shareable monotonic cancellation latch polled at executor step boundaries.
#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub struct ExecutionContext<'a> {
    expansion: tex_expand::ExpansionContext<'a>,
    font_resolver: Option<&'a mut dyn FontResolver>,
    image_resolver: Option<&'a mut dyn PdfImageResolver>,
    pub(crate) pending_paragraph_memo: Option<PendingParagraphMemo>,
    pub(crate) paragraph_memo_barrier: bool,
    /// A changed line-breaking dependency invalidates finished-line artifacts
    /// for the revision. Once observed, run cold instead of rediscovering the
    /// same miss and rebuilding memo history for every following paragraph.
    pub(crate) paragraph_memo_disabled_for_run: bool,
    pub(crate) cold_paragraph_recording: Option<ColdParagraphRecording>,
    /// Detached paragraph observations reusable only while their authoritative
    /// changed-at stamps remain equal during this execution run.
    pub(crate) paragraph_dependency_cache:
        ahash::AHashMap<tex_state::DependencyKey, CachedParagraphDependency>,
}

type ExecutionContextParts<'a> = (
    ExecutionState,
    Option<&'a mut dyn InputResolver>,
    Option<&'a mut dyn FontResolver>,
    Option<&'a mut dyn PdfImageResolver>,
    Option<&'a mut dyn ReadRecorder>,
);

impl<'a> ExecutionContext<'a> {
    #[must_use]
    pub fn new(job_name: &str) -> Self {
        Self {
            expansion: tex_expand::ExpansionContext::new(job_name),
            font_resolver: None,
            image_resolver: None,
            pending_paragraph_memo: None,
            paragraph_memo_barrier: false,
            paragraph_memo_disabled_for_run: false,
            cold_paragraph_recording: None,
            paragraph_dependency_cache: ahash::AHashMap::new(),
        }
    }

    fn from_owned_state(
        job_name: &str,
        state: ExecutionState,
        input_resolver: Option<&'a mut dyn InputResolver>,
        font_resolver: Option<&'a mut dyn FontResolver>,
        image_resolver: Option<&'a mut dyn PdfImageResolver>,
        recorder: Option<&'a mut dyn ReadRecorder>,
    ) -> Self {
        Self {
            expansion: tex_expand::ExpansionContext::from_state(
                job_name,
                state.expansion,
                input_resolver,
                recorder,
            ),
            font_resolver,
            image_resolver,
            pending_paragraph_memo: state.pending_paragraph_memo,
            paragraph_memo_barrier: state.paragraph_memo_barrier,
            paragraph_memo_disabled_for_run: state.paragraph_memo_disabled_for_run,
            cold_paragraph_recording: state.cold_paragraph_recording,
            paragraph_dependency_cache: state.paragraph_dependency_cache,
        }
    }

    fn into_owned_parts(self) -> ExecutionContextParts<'a> {
        let (expansion, input_resolver, recorder) = self.expansion.into_parts();
        (
            ExecutionState {
                expansion,
                pending_paragraph_memo: self.pending_paragraph_memo,
                paragraph_memo_barrier: self.paragraph_memo_barrier,
                paragraph_memo_disabled_for_run: self.paragraph_memo_disabled_for_run,
                cold_paragraph_recording: self.cold_paragraph_recording,
                paragraph_dependency_cache: self.paragraph_dependency_cache,
            },
            input_resolver,
            self.font_resolver,
            self.image_resolver,
            recorder,
        )
    }

    #[must_use]
    pub fn with_resolvers(
        job_name: &str,
        input_resolver: &'a mut dyn InputResolver,
        font_resolver: &'a mut dyn FontResolver,
    ) -> Self {
        Self {
            expansion: tex_expand::ExpansionContext::with_input_resolver(job_name, input_resolver),
            font_resolver: Some(font_resolver),
            image_resolver: None,
            pending_paragraph_memo: None,
            paragraph_memo_barrier: false,
            paragraph_memo_disabled_for_run: false,
            cold_paragraph_recording: None,
            paragraph_dependency_cache: ahash::AHashMap::new(),
        }
    }

    /// Replaces the recursive expansion-work budget for each delivered token.
    #[must_use]
    pub fn with_expansion_fuel(mut self, fuel: u64) -> Self {
        self.expansion = self.expansion.with_fuel(fuel);
        self
    }

    #[must_use]
    pub fn with_resource_resolvers(
        job_name: &str,
        input_resolver: &'a mut dyn InputResolver,
        font_resolver: &'a mut dyn FontResolver,
        image_resolver: &'a mut dyn PdfImageResolver,
    ) -> Self {
        Self {
            expansion: tex_expand::ExpansionContext::with_input_resolver(job_name, input_resolver),
            font_resolver: Some(font_resolver),
            image_resolver: Some(image_resolver),
            pending_paragraph_memo: None,
            paragraph_memo_barrier: false,
            paragraph_memo_disabled_for_run: false,
            cold_paragraph_recording: None,
            paragraph_dependency_cache: ahash::AHashMap::new(),
        }
    }

    /// Installs an erased expansion read recorder for this execution session.
    #[must_use]
    pub fn recording(mut self, recorder: &'a mut dyn ReadRecorder) -> Self {
        self.expansion = self.expansion.recording(recorder);
        self
    }

    pub(crate) fn open_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
    ) -> ResourceResult<FontSource> {
        let request_index = self.expansion.next_resolution_index();
        match self.font_resolver.as_deref_mut() {
            Some(resolver) => resolver.open_font(input, path, request_index),
            None => Ok(match input.read_input_file(path) {
                Ok(metrics) => ResourceLookup::Available(FontSource::Tfm {
                    metrics,
                    opentype: None,
                }),
                Err(_) => ResourceLookup::Unavailable,
            }),
        }
    }

    pub(crate) fn open_pdf_image(
        &mut self,
        input: &mut dyn InputReadState,
        request: &PdfImageRequest,
    ) -> ResourceResult<tex_state::PdfExternalImageSource> {
        let request_index = self.expansion.next_resolution_index();
        self.image_resolver
            .as_deref_mut()
            .map_or(Ok(ResourceLookup::Unavailable), |resolver| {
                resolver.open_image(input, request, request_index)
            })
    }

    pub(crate) fn begin_cold_paragraph_recording(
        &mut self,
        effect_start: usize,
        starting_span: Option<tex_state::RootSpanId>,
        starting_root_span: Option<tex_state::RootSpanId>,
        starting_input: Option<tex_state::InputSummary>,
        starting_input_identity: Option<u64>,
        starting_group_depth: u32,
    ) -> bool {
        if self.cold_paragraph_recording.is_some() {
            return false;
        }
        self.expansion.begin_paragraph_recording();
        self.cold_paragraph_recording = Some(ColdParagraphRecording {
            effect_start,
            starting_span,
            starting_root_span,
            starting_input,
            starting_input_identity,
            starting_group_depth,
            delivered_tokens: 0,
            inline_math: None,
            local_boxes: Vec::new(),
            dependencies: Vec::new(),
            barriers: std::collections::BTreeSet::new(),
        });
        true
    }

    pub(crate) fn count_paragraph_token(&mut self) {
        if let Some(recording) = &mut self.cold_paragraph_recording
            && recording.barriers.is_empty()
        {
            recording.delivered_tokens = recording.delivered_tokens.saturating_add(1);
        }
    }

    pub(crate) fn update_cold_paragraph_start(
        &mut self,
        starting_span: Option<tex_state::RootSpanId>,
        starting_group_depth: u32,
    ) {
        if let Some(recording) = &mut self.cold_paragraph_recording
            && starting_span.is_some()
        {
            recording.starting_span = starting_span;
            recording.starting_group_depth = starting_group_depth;
        }
    }

    pub(crate) fn abandon_cold_paragraph_recording(&mut self) {
        self.cold_paragraph_recording = None;
        let _ = self.expansion.finish_paragraph_recording();
    }

    pub(crate) fn mark_paragraph_barrier(&mut self, reason: ParagraphBarrierReason) {
        if let Some(recording) = &mut self.cold_paragraph_recording {
            recording.barriers.insert(reason);
            self.expansion.stop_paragraph_read_tracking();
        }
    }

    pub(crate) fn mark_paragraph_group_scoped_assignment(
        &mut self,
        stores: &Universe,
        global: bool,
    ) {
        let Some(recording) = &self.cold_paragraph_recording else {
            return;
        };
        let current_depth = tex_state::ExpansionState::execution_group_depth(stores);
        if global || current_depth <= recording.starting_group_depth {
            self.mark_paragraph_barrier(ParagraphBarrierReason::UnsupportedEscapingWrite);
        }
    }

    pub(crate) fn mark_paragraph_local_meaning(
        &mut self,
        stores: &Universe,
        symbol: tex_state::interner::Symbol,
        global: bool,
    ) {
        let Some(recording) = &self.cold_paragraph_recording else {
            return;
        };
        let current_depth = tex_state::ExpansionState::execution_group_depth(stores);
        if !global && current_depth > recording.starting_group_depth {
            self.expansion
                .mark_paragraph_local_meaning(symbol, current_depth);
        }
    }

    pub(crate) fn mark_paragraph_local_box(&mut self, stores: &Universe, index: u16, global: bool) {
        let Some(recording) = &mut self.cold_paragraph_recording else {
            return;
        };
        let current_depth = tex_state::ExpansionState::execution_group_depth(stores);
        if !global && current_depth > recording.starting_group_depth {
            recording.local_boxes.push((index, current_depth));
        }
    }

    pub(crate) fn paragraph_box_is_source_proven(&self, index: u16) -> bool {
        self.cold_paragraph_recording
            .as_ref()
            .is_some_and(|recording| {
                recording
                    .local_boxes
                    .iter()
                    .rev()
                    .any(|&(local_index, _)| local_index == index)
            })
    }

    pub(crate) fn record_paragraph_box_read(&mut self, index: u16) {
        if let Some(recording) = &mut self.cold_paragraph_recording {
            recording.dependencies.push(tex_state::DependencyKey::Cell {
                bank: tex_state::DependencyBank::Box,
                index: u32::from(index),
            });
        }
    }

    pub(crate) fn paragraph_group_exited(&mut self, stores: &Universe) {
        let remaining_depth = tex_state::ExpansionState::execution_group_depth(stores);
        self.expansion.paragraph_group_exited(remaining_depth);
        if let Some(recording) = &mut self.cold_paragraph_recording {
            recording
                .local_boxes
                .retain(|&(_, definition_depth)| definition_depth <= remaining_depth);
        }
    }

    pub(crate) fn mark_paragraph_inline_math(&mut self) {
        if let Some(recording) = &mut self.cold_paragraph_recording {
            recording.inline_math.get_or_insert_default();
        }
    }

    pub(crate) fn record_paragraph_mathcode(&mut self, ch: char) {
        if let Some(reads) = self
            .cold_paragraph_recording
            .as_mut()
            .and_then(|recording| recording.inline_math.as_mut())
        {
            reads.mathcodes.push(ch);
        }
    }

    pub(crate) fn record_paragraph_delcode(&mut self, ch: char) {
        if let Some(reads) = self
            .cold_paragraph_recording
            .as_mut()
            .and_then(|recording| recording.inline_math.as_mut())
        {
            reads.delcodes.push(ch);
        }
    }

    pub(crate) fn record_paragraph_math_families(&mut self, family_mask: u64) {
        if let Some(reads) = self
            .cold_paragraph_recording
            .as_mut()
            .and_then(|recording| recording.inline_math.as_mut())
        {
            reads.family_mask |= family_mask;
        }
    }

    pub(crate) fn finish_paragraph_expansion_recording(
        &mut self,
    ) -> (
        Vec<tex_state::DependencyKey>,
        Vec<tex_expand::ParagraphExpansionBarrier>,
    ) {
        self.expansion.finish_paragraph_recording()
    }
}

impl<'a> Deref for ExecutionContext<'a> {
    type Target = tex_expand::ExpansionContext<'a>;

    fn deref(&self) -> &Self::Target {
        &self.expansion
    }
}

impl DerefMut for ExecutionContext<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.expansion
    }
}

/// Stomach interpreter state.
#[derive(Clone, Debug, PartialEq)]
pub struct Executor {
    pub(crate) nest: ModeNest,
}

/// Owned stomach state and lifecycle for a cooperatively driven execution.
pub struct ExecutionRun {
    job_name: String,
    nest: ModeNest,
    execution: Option<ExecutionState>,
    stats: ExecutionStats,
    artifact_start: Option<usize>,
    next_step: ExecutionStep,
    lifecycle: ExecutionLifecycle,
    publish_job_start: bool,
    suspension_serial: u64,
    checkpoint_mode_projection: Option<(crate::ModeNestSummary, u64)>,
    cumulative_fuel_limit: u64,
    telemetry: ExecutionTelemetry,
}

struct StepSavepoint {
    universe: tex_state::Snapshot,
    input: InputStackSnapshot,
    nest: ModeNest,
    execution: ExecutionStateSnapshot,
    stats: ExecutionStats,
    artifact_start: Option<usize>,
    next_step: ExecutionStep,
    lifecycle: ExecutionLifecycle,
    publish_job_start: bool,
    checkpoint_mode_projection: Option<(crate::ModeNestSummary, u64)>,
}

impl ExecutionRun {
    #[must_use]
    pub fn new(job_name: impl Into<String>) -> Self {
        Self::from_parts(job_name, ModeNest::new(), ExecutionState::default(), true)
    }

    #[must_use]
    pub fn from_parts(
        job_name: impl Into<String>,
        nest: ModeNest,
        execution: ExecutionState,
        publish_job_start: bool,
    ) -> Self {
        Self {
            job_name: job_name.into(),
            nest,
            execution: Some(execution),
            stats: ExecutionStats::default(),
            artifact_start: None,
            next_step: if publish_job_start {
                ExecutionStep::JobStart
            } else {
                ExecutionStep::MainControl
            },
            lifecycle: if publish_job_start {
                ExecutionLifecycle::Created
            } else {
                ExecutionLifecycle::Ready
            },
            publish_job_start,
            suspension_serial: 0,
            checkpoint_mode_projection: None,
            cumulative_fuel_limit: u64::MAX,
            telemetry: ExecutionTelemetry {
                cold_starts: 1,
                ..ExecutionTelemetry::default()
            },
        }
    }

    #[must_use]
    pub fn with_cumulative_fuel_limit(mut self, limit: u64) -> Self {
        self.cumulative_fuel_limit = limit;
        self
    }

    pub fn set_cumulative_fuel_limit(&mut self, limit: u64) {
        self.cumulative_fuel_limit = limit;
    }

    #[must_use]
    pub const fn telemetry(&self) -> ExecutionTelemetry {
        self.telemetry
    }

    #[must_use]
    pub const fn next_step(&self) -> ExecutionStep {
        self.next_step
    }

    #[must_use]
    pub const fn lifecycle(&self) -> ExecutionLifecycle {
        self.lifecycle
    }

    /// Finishes a cooperatively driven run after a checkpoint sink requested
    /// an early stop. The checkpoint and its producing step must already have
    /// committed before this transition is requested.
    pub fn finish_after_checkpoint(&mut self) {
        if !matches!(
            self.lifecycle,
            ExecutionLifecycle::Complete
                | ExecutionLifecycle::Failed
                | ExecutionLifecycle::Cancelled
        ) {
            self.next_step = ExecutionStep::Finalize;
            self.lifecycle = ExecutionLifecycle::Finishing;
        }
    }

    #[must_use]
    pub fn nest(&self) -> &ModeNest {
        &self.nest
    }

    pub fn step(
        &mut self,
        services: &mut ExecutionServices<'_, '_>,
        cancellation: &Cancellation,
    ) -> ExecutionStepResult {
        if cancellation.is_cancelled() {
            self.lifecycle = ExecutionLifecycle::Cancelled;
            return ExecutionStepResult::Cancelled;
        }
        if matches!(
            self.lifecycle,
            ExecutionLifecycle::Complete
                | ExecutionLifecycle::Failed
                | ExecutionLifecycle::Cancelled
        ) {
            return ExecutionStepResult::Failed(ExecError::ExecutionAlreadyTerminated);
        }
        if self.lifecycle == ExecutionLifecycle::Awaiting {
            self.telemetry.local_step_retries = self.telemetry.local_step_retries.saturating_add(1);
            self.lifecycle = match self.next_step {
                ExecutionStep::FinishEnd | ExecutionStep::Finalize => ExecutionLifecycle::Finishing,
                ExecutionStep::JobStart => ExecutionLifecycle::Created,
                ExecutionStep::MainControl => ExecutionLifecycle::Ready,
            };
        }
        self.telemetry.advance_calls = self.telemetry.advance_calls.saturating_add(1);
        let timer = TelemetryTimer::start();
        if self.artifact_start.is_none() {
            services
                .input
                .ensure_source_ids_at_least(services.stores.input_summary().next_source_id());
            self.execution_mut()
                .expansion
                .set_job_clock(services.stores.world().job_clock());
            self.artifact_start = Some(services.stores.world().artifact_commits().len());
        }

        let blocked_step = self.next_step;
        let savepoint = self.capture_step_savepoint(services);
        let checkpoint_destination = services.checkpoints.take();
        let mut staged = StagedCheckpointSink::new(checkpoint_destination);
        let mut staged_reads = Vec::new();
        let stats_before = self.stats.clone();
        let result = match self.next_step {
            ExecutionStep::JobStart => {
                if self.publish_job_start {
                    let every_job = services.stores.take_pending_every_job();
                    if every_job != TokenListId::EMPTY {
                        services
                            .input
                            .push_token_list(every_job, TokenListReplayKind::EveryJob);
                    }
                    let mut session = EngineSession::with_mode_projection(
                        &mut staged,
                        self.checkpoint_mode_projection.take(),
                    );
                    session.publish(
                        EngineBoundary::JobStart,
                        &self.nest,
                        services.input,
                        services.stores,
                    );
                    self.checkpoint_mode_projection = session.into_mode_projection();
                }
                self.next_step = ExecutionStep::MainControl;
                self.lifecycle = ExecutionLifecycle::Ready;
                Ok(())
            }
            ExecutionStep::MainControl => {
                self.step_main_control(services, &mut staged, &mut staged_reads)
            }
            ExecutionStep::FinishEnd => self.step_finish_end(services, &mut staged_reads),
            ExecutionStep::Finalize => self.step_finalize(services),
        };

        self.telemetry.engine_time = self.telemetry.engine_time.saturating_add(timer.elapsed());
        self.telemetry.cumulative_fuel = self
            .execution
            .as_ref()
            .expect("execution state is installed after a step")
            .expansion
            .cumulative_fuel_burned();
        let result = if self.telemetry.cumulative_fuel > self.cumulative_fuel_limit {
            Err(ExecError::CumulativeFuelExceeded {
                limit: self.cumulative_fuel_limit,
                attempted: self.telemetry.cumulative_fuel,
            })
        } else {
            result
        };
        match result {
            Ok(()) => {
                let (checkpoints, stop_requested, checkpoint_destination) = staged.commit();
                services.checkpoints = checkpoint_destination;
                if let Some(recorder) = services.recorder.as_deref_mut() {
                    for reads in staged_reads {
                        reads.deliver(recorder);
                    }
                }
                if self.lifecycle == ExecutionLifecycle::Complete {
                    ExecutionStepResult::Complete(self.stats.clone())
                } else {
                    ExecutionStepResult::Progress(ExecutionProgress {
                        next_step: self.next_step,
                        checkpoints,
                        stop_requested,
                    })
                }
            }
            Err(error) => {
                let suspends = resource_need(&error).is_some();
                if suspends {
                    self.telemetry.suspensions = self.telemetry.suspensions.saturating_add(1);
                    self.telemetry.replayed_delivered_tokens =
                        self.telemetry.replayed_delivered_tokens.saturating_add(
                            self.stats
                                .delivered_tokens
                                .saturating_sub(stats_before.delivered_tokens)
                                as u64,
                        );
                    self.telemetry.replayed_dispatches =
                        self.telemetry.replayed_dispatches.saturating_add(
                            self.stats
                                .main_control_dispatches
                                .saturating_sub(stats_before.main_control_dispatches)
                                as u64,
                        );
                    services.checkpoints = staged.discard();
                    self.rollback_step_savepoint(services, savepoint);
                } else {
                    // A terminal TeX error historically leaves the live engine
                    // at its failure point.  Only a typed host-resource need
                    // is replayable; rolling semantic failures back would
                    // erase preceding commands in this bounded chunk and can
                    // invalidate a group-local Universe snapshot.
                    services.checkpoints = staged.discard();
                }
                self.handle_step_error(error, blocked_step)
            }
        }
    }

    fn capture_step_savepoint(
        &mut self,
        services: &mut ExecutionServices<'_, '_>,
    ) -> StepSavepoint {
        StepSavepoint {
            universe: services.stores.snapshot(),
            input: services.input.snapshot(),
            nest: self.nest.clone(),
            execution: self
                .execution
                .as_ref()
                .expect("execution state is installed between steps")
                .snapshot(),
            stats: self.stats.clone(),
            artifact_start: self.artifact_start,
            next_step: self.next_step,
            lifecycle: self.lifecycle,
            publish_job_start: self.publish_job_start,
            checkpoint_mode_projection: self.checkpoint_mode_projection.clone(),
        }
    }

    fn rollback_step_savepoint(
        &mut self,
        services: &mut ExecutionServices<'_, '_>,
        savepoint: StepSavepoint,
    ) {
        services.stores.rollback(&savepoint.universe);
        services.input.rollback(savepoint.input);
        self.nest = savepoint.nest;
        self.execution
            .as_mut()
            .expect("execution state is reinstalled before rollback")
            .rollback(savepoint.execution);
        self.stats = savepoint.stats;
        self.artifact_start = savepoint.artifact_start;
        self.next_step = savepoint.next_step;
        self.lifecycle = savepoint.lifecycle;
        self.publish_job_start = savepoint.publish_job_start;
        self.checkpoint_mode_projection = savepoint.checkpoint_mode_projection;
    }

    fn execution_mut(&mut self) -> &mut ExecutionState {
        self.execution
            .as_mut()
            .expect("execution state is installed outside call-local dispatch")
    }

    fn with_context<T>(
        &mut self,
        services: &mut ExecutionServices<'_, '_>,
        staged_reads: &mut Vec<tex_expand::ReadRecorderBatch>,
        operation: impl FnOnce(
            &mut ModeNest,
            &mut InputStack,
            &mut Universe,
            &mut ExecutionContext<'_>,
            &mut ExecutionStats,
        ) -> Result<T, ExecError>,
    ) -> Result<T, ExecError> {
        let state = self.execution.take().expect("execution state is owned");
        let input_resolver = services.input_resolver.take();
        let font_resolver = services.font_resolver.take();
        let image_resolver = services.image_resolver.take();
        let recorder = services.recorder.take();
        let mut context = ExecutionContext::from_owned_state(
            &self.job_name,
            state,
            input_resolver,
            font_resolver,
            image_resolver,
            recorder,
        );
        context.begin_transactional_recording();
        let output = operation(
            &mut self.nest,
            services.input,
            services.stores,
            &mut context,
            &mut self.stats,
        );
        if output.is_ok() {
            staged_reads.push(context.finish_transactional_recording());
        } else {
            context.discard_transactional_recording();
        }
        let (state, input_resolver, font_resolver, image_resolver, recorder) =
            context.into_owned_parts();
        self.execution = Some(state);
        services.input_resolver = input_resolver;
        services.font_resolver = font_resolver;
        services.image_resolver = image_resolver;
        services.recorder = recorder;
        output
    }

    fn step_main_control(
        &mut self,
        services: &mut ExecutionServices<'_, '_>,
        staged: &mut StagedCheckpointSink<'_>,
        staged_reads: &mut Vec<tex_expand::ReadRecorderBatch>,
    ) -> Result<(), ExecError> {
        let mut checkpoint_mode_projection = self.checkpoint_mode_projection.take();
        let exit = self.with_context(
            services,
            staged_reads,
            |nest, input, stores, execution, stats| {
                run_outer_main_control_step(
                    nest,
                    input,
                    stores,
                    execution,
                    stats,
                    staged,
                    &mut checkpoint_mode_projection,
                )
            },
        );
        self.checkpoint_mode_projection = checkpoint_mode_projection;
        let exit = exit?;
        match exit {
            MainControlExit::EndOfInput => {
                self.next_step = ExecutionStep::Finalize;
                self.lifecycle = ExecutionLifecycle::Finishing;
            }
            MainControlExit::End { .. } => {
                self.next_step = ExecutionStep::FinishEnd;
                self.lifecycle = ExecutionLifecycle::Finishing;
            }
            MainControlExit::Stopped => {}
            MainControlExit::NotConsumed { token } => {
                return Err(unimplemented_typesetting(
                    self.nest.current_mode(),
                    tex_expand::semantic_token(token),
                    token.origin(),
                    "non-assignment command",
                )
                .expect_err("unimplemented_typesetting always returns Err")
                .capture(services.input));
            }
        }
        Ok(())
    }

    fn step_finish_end(
        &mut self,
        services: &mut ExecutionServices<'_, '_>,
        staged_reads: &mut Vec<tex_expand::ReadRecorderBatch>,
    ) -> Result<(), ExecError> {
        self.with_context(
            services,
            staged_reads,
            |nest, input, stores, execution, stats| {
                output::finish_end(nest, input, stores, execution, stats)
                    .map_err(|error| error.capture(input))
            },
        )?;
        self.next_step = ExecutionStep::Finalize;
        self.lifecycle = ExecutionLifecycle::Finishing;
        Ok(())
    }

    fn step_finalize(&mut self, services: &mut ExecutionServices<'_, '_>) -> Result<(), ExecError> {
        let dumped_format = self.stats.dumped_format;
        let summary = if dumped_format {
            InputSummary::default()
        } else {
            services.input.publication_summary(services.stores)
        };
        if dumped_format {
            services.stores.start_new_page();
        }
        services.stores.set_input_summary(summary);
        let artifact_start = self
            .artifact_start
            .unwrap_or_else(|| services.stores.world().artifact_commits().len());
        self.stats.shipped_artifacts =
            services.stores.world().artifact_commits()[artifact_start..].to_vec();
        let mut prepared = BTreeMap::<_, VecDeque<_>>::new();
        for page in std::mem::take(&mut self.stats.prepared_dvi_pages) {
            prepared.entry(page.hash).or_default().push_back(page.plan);
        }
        let plans = services.stores.world().committed_artifacts()[artifact_start..]
            .iter()
            .map(|committed| {
                if let Some(plan) = prepared
                    .get_mut(&committed.hash())
                    .and_then(VecDeque::pop_front)
                {
                    return Ok(plan);
                }
                DviPagePlan::compile_v10(committed.bytes())
                    .map_err(|error| ExecError::InvalidShipoutArtifact(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>();
        self.stats.dvi_pages = plans?;
        self.lifecycle = ExecutionLifecycle::Complete;
        Ok(())
    }

    fn handle_step_error(
        &mut self,
        error: ExecError,
        blocked_step: ExecutionStep,
    ) -> ExecutionStepResult {
        if let Some(need) = resource_need(&error) {
            self.suspension_serial = self.suspension_serial.saturating_add(1);
            self.lifecycle = ExecutionLifecycle::Awaiting;
            return ExecutionStepResult::AwaitingResources(ResourceSuspension {
                requests: vec![need],
                site: match blocked_step {
                    ExecutionStep::JobStart | ExecutionStep::MainControl => {
                        ResourceSite::MainControl
                    }
                    ExecutionStep::FinishEnd | ExecutionStep::Finalize => {
                        ResourceSite::EndFinalization
                    }
                },
                serial: self.suspension_serial,
                blocked_step,
            });
        }
        self.lifecycle = ExecutionLifecycle::Failed;
        ExecutionStepResult::Failed(error)
    }

    fn into_parts(self) -> (ModeNest, ExecutionState, ExecutionStats) {
        (
            self.nest,
            self.execution
                .expect("terminal run retains execution state"),
            self.stats,
        )
    }
}

fn resource_need(error: &ExecError) -> Option<tex_expand::ResourceNeed> {
    match error {
        ExecError::NeedResource(need) => Some(*need),
        ExecError::Captured { error, .. } => resource_need(error),
        ExecError::Expand(error) => error.resource_need(),
        ExecError::ScanToks(error) => error.resource_need(),
        ExecError::ScanGlue(error) => error.resource_need(),
        _ => None,
    }
}

struct StagedCheckpointSink<'a> {
    destination: Option<&'a mut dyn CheckpointSink>,
    checkpoints: Vec<crate::EngineCheckpoint>,
}

impl<'a> StagedCheckpointSink<'a> {
    fn new(destination: Option<&'a mut dyn CheckpointSink>) -> Self {
        Self {
            destination,
            checkpoints: Vec::new(),
        }
    }

    fn commit(
        mut self,
    ) -> (
        Vec<crate::EngineCheckpoint>,
        bool,
        Option<&'a mut dyn CheckpointSink>,
    ) {
        let checkpoints = std::mem::take(&mut self.checkpoints);
        if let Some(destination) = self.destination.as_deref_mut() {
            for checkpoint in &checkpoints {
                destination.checkpoint(checkpoint.clone());
            }
        }
        let stop_requested = self
            .destination
            .as_deref()
            .is_some_and(CheckpointSink::stop_requested);
        (checkpoints, stop_requested, self.destination.take())
    }

    fn discard(mut self) -> Option<&'a mut dyn CheckpointSink> {
        self.destination.take()
    }
}

impl CheckpointSink for StagedCheckpointSink<'_> {
    fn wants_checkpoint(&self, boundary: EngineBoundary) -> bool {
        self.destination
            .as_deref()
            .is_some_and(|sink| sink.wants_checkpoint(boundary))
    }

    fn stop_requested(&self) -> bool {
        self.destination
            .as_deref()
            .is_some_and(CheckpointSink::stop_requested)
    }

    fn wants_exact_state_identity(&self, boundary: EngineBoundary, root_anchor: usize) -> bool {
        self.destination
            .as_deref()
            .is_some_and(|sink| sink.wants_exact_state_identity(boundary, root_anchor))
    }

    fn checkpoint(&mut self, checkpoint: crate::EngineCheckpoint) {
        self.checkpoints.push(checkpoint);
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nest: ModeNest::new(),
        }
    }

    pub fn from_nest(nest: ModeNest) -> Self {
        Self { nest }
    }

    #[must_use]
    pub fn nest(&self) -> &ModeNest {
        &self.nest
    }

    pub fn nest_mut(&mut self) -> &mut ModeNest {
        &mut self.nest
    }

    /// Runs main control until the gullet has no more delivered tokens.
    pub fn run(
        &mut self,
        input: &mut InputStack,
        stores: &mut Universe,
    ) -> Result<ExecutionStats, ExecError>
where {
        let mut context = ExecutionContext::new("texput");
        self.run_with_context(input, stores, &mut context)
    }

    /// Runs main control using driver-provided execution context.
    pub fn run_with_context(
        &mut self,
        input: &mut InputStack,
        stores: &mut Universe,
        execution: &mut crate::ExecutionContext<'_>,
    ) -> Result<ExecutionStats, ExecError>
where {
        let mut checkpoints = NoopCheckpointSink;
        self.run_with_context_and_checkpoints(input, stores, execution, &mut checkpoints)
    }

    /// Runs main control and publishes restartable state at named safe boundaries.
    pub fn run_with_context_and_checkpoints<C>(
        &mut self,
        input: &mut InputStack,
        stores: &mut Universe,
        execution: &mut crate::ExecutionContext<'_>,
        checkpoints: &mut C,
    ) -> Result<ExecutionStats, ExecError>
    where
        C: CheckpointSink,
    {
        self.run_session(input, stores, execution, checkpoints, true)
    }

    /// Continues a previously restored named checkpoint without publishing a
    /// second `JobStart` boundary.
    pub fn resume_with_context_and_checkpoints<C>(
        &mut self,
        input: &mut InputStack,
        stores: &mut Universe,
        execution: &mut crate::ExecutionContext<'_>,
        checkpoints: &mut C,
    ) -> Result<ExecutionStats, ExecError>
    where
        C: CheckpointSink,
    {
        self.run_session(input, stores, execution, checkpoints, false)
    }

    fn run_session<C>(
        &mut self,
        input: &mut InputStack,
        stores: &mut Universe,
        execution: &mut crate::ExecutionContext<'_>,
        checkpoints: &mut C,
        publish_job_start: bool,
    ) -> Result<ExecutionStats, ExecError>
    where
        C: CheckpointSink,
    {
        let job_name = execution.job_name.clone();
        let detached = std::mem::replace(execution, ExecutionContext::new(&job_name));
        let (state, input_resolver, font_resolver, image_resolver, recorder) =
            detached.into_owned_parts();
        let nest = std::mem::take(&mut self.nest);
        let mut run = ExecutionRun::from_parts(&job_name, nest, state, publish_job_start);
        let cancellation = Cancellation::new();
        let mut services = ExecutionServices {
            input,
            stores,
            input_resolver,
            font_resolver,
            image_resolver,
            recorder,
            checkpoints: Some(checkpoints),
        };
        let result = loop {
            let attempted_step = run.next_step();
            match run.step(&mut services, &cancellation) {
                ExecutionStepResult::Progress(progress) => {
                    if attempted_step == ExecutionStep::MainControl && progress.stop_requested {
                        run.next_step = ExecutionStep::Finalize;
                        run.lifecycle = ExecutionLifecycle::Finishing;
                    }
                }
                ExecutionStepResult::Complete(stats) => break Ok(stats),
                ExecutionStepResult::AwaitingResources(suspension) => {
                    break Err(ExecError::NeedResource(suspension.requests[0]));
                }
                ExecutionStepResult::Failed(error) => break Err(error),
                ExecutionStepResult::Cancelled => break Err(ExecError::ExecutionCancelled),
            }
        };
        let (nest, state, _) = run.into_parts();
        self.nest = nest;
        let input_resolver = services.input_resolver.take();
        let font_resolver = services.font_resolver.take();
        let image_resolver = services.image_resolver.take();
        let recorder = services.recorder.take();
        let summary = services.input.publication_summary(services.stores);
        if result.is_err() {
            services.stores.set_input_summary(summary);
        }
        *execution = ExecutionContext::from_owned_state(
            &job_name,
            state,
            input_resolver,
            font_resolver,
            image_resolver,
            recorder,
        );
        result
    }
}

const EXECUTION_TEXT_SPAN_CHUNK: usize = 256;
const EXECUTION_STEP_OPERATION_CHUNK: usize = 256;

fn run_outer_main_control_step<C>(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
    session_sink: &mut C,
    mode_projection: &mut Option<(crate::ModeNestSummary, u64)>,
) -> Result<MainControlExit, ExecError>
where
    C: CheckpointSink,
{
    let mut session = EngineSession::with_mode_projection(session_sink, mode_projection.take());
    let mut operations = 0usize;
    let entry_group_depth = tex_state::ExpansionState::execution_group_depth(stores);
    let result = run_main_control_until_observing(
        nest,
        input,
        stores,
        execution,
        stats,
        true,
        |_, _| false,
        |nest, input, stores, event| {
            if event.shipout_complete {
                session.publish(EngineBoundary::ShipoutComplete, nest, input, stores);
            }
            if event.outer_paragraph_end {
                session.publish(EngineBoundary::OuterParagraphEnd, nest, input, stores);
            }
            operations += 1;
            event.shipout_complete
                || event.outer_paragraph_end
                || session.stop_requested()
                || tex_state::ExpansionState::execution_group_depth(stores) != entry_group_depth
                || operations >= EXECUTION_STEP_OPERATION_CHUNK
        },
    );
    *mode_projection = session.into_mode_projection();
    result.map_err(|error| error.capture(input))
}

#[derive(Clone, Copy, Debug, Default)]
struct BoundaryEvent {
    outer_paragraph_end: bool,
    shipout_complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MainControlExit {
    EndOfInput,
    Stopped,
    End { token: TracedTokenWord },
    NotConsumed { token: TracedTokenWord },
}

pub(crate) fn run_main_control_until<F>(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
    should_stop: F,
) -> Result<MainControlExit, ExecError>
where
    F: FnMut(&mut InputStack, &Universe) -> bool,
{
    let result = run_main_control_until_observing(
        nest,
        input,
        stores,
        execution,
        stats,
        false,
        should_stop,
        |_, _, _, _| false,
    );
    result.map_err(|error| error.capture(input))
}

#[allow(clippy::too_many_arguments)]
fn run_main_control_until_observing<F, O>(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
    allow_text_spans: bool,
    mut should_stop: F,
    mut observe: O,
) -> Result<MainControlExit, ExecError>
where
    F: FnMut(&mut InputStack, &Universe) -> bool,
    O: FnMut(&ModeNest, &mut InputStack, &mut Universe, BoundaryEvent) -> bool,
{
    let mut macro_text = Vec::new();
    loop {
        #[cfg(feature = "profiling-stats")]
        let mut paragraph_probe_anchor = None;
        abandon_stale_vertical_paragraph_probe(nest, stores, execution);
        report_recoverable_expansion_diagnostics(execution, stores);
        if should_stop(input, stores) {
            return Ok(MainControlExit::Stopped);
        }

        if allow_text_spans
            && nest.current_mode() == crate::Mode::Vertical
            && execution.pending_paragraph_memo.is_none()
            && !execution.paragraph_memo_barrier
            && !execution.paragraph_memo_disabled_for_run
            && stores.paragraph_memo_enabled()
            // PDF microtype consults mutable per-font expansion and character
            // code tables which are not part of the finished-line dependency
            // vocabulary. Keep those paragraphs on the ordinary cold path.
            && stores.int_param(tex_state::env::banks::IntParam::PDF_ADJUST_SPACING) == 0
            && stores.int_param(tex_state::env::banks::IntParam::PDF_PROTRUDE_CHARS) == 0
        {
            let starting_span = input.current_root_delivery_anchor(stores)?;
            let starting_input = starting_span.is_none().then(|| input.summary());
            let starting_input_identity = starting_input
                .as_ref()
                .map(|summary| summary.paragraph_boundary_identity(stores));
            let starting_root_span = match starting_span {
                Some(span) => Some(span),
                None => input.root_source_cursor_anchor(stores),
            };
            #[cfg(feature = "profiling-stats")]
            {
                paragraph_probe_anchor = Some(starting_span.is_some());
            }
            if starting_root_span.is_some() {
                if execution.cold_paragraph_recording.is_none()
                    && execution.begin_cold_paragraph_recording(
                        stores.world().effect_records().len(),
                        starting_span,
                        starting_root_span,
                        starting_input.clone(),
                        starting_input_identity,
                        tex_state::ExpansionState::execution_group_depth(stores),
                    )
                {
                    stores.begin_pure_paragraph_recording();
                }
                execution.update_cold_paragraph_start(
                    starting_span,
                    tex_state::ExpansionState::execution_group_depth(stores),
                );
                if execution.cold_paragraph_recording.is_some() {
                    let before_artifacts = stores.world().artifact_commits().len();
                    if crate::paragraph_memo::try_reuse_aligned_paragraph(
                        starting_span,
                        starting_root_span,
                        starting_input_identity,
                        nest,
                        input,
                        stores,
                        execution,
                    )? {
                        output::drain_pending_output(nest, input, stores, execution, stats)?;
                        execution.paragraph_memo_barrier = false;
                        let outer_paragraph_end =
                            nest.current_mode() == crate::Mode::Vertical && nest.depth() == 1;
                        if observe(
                            nest,
                            input,
                            stores,
                            BoundaryEvent {
                                outer_paragraph_end,
                                shipout_complete: stores.world().artifact_commits().len()
                                    != before_artifacts,
                            },
                        ) {
                            return Ok(MainControlExit::Stopped);
                        }
                        continue;
                    }
                }
            }
        }

        if allow_text_spans
            && matches!(
                nest.current_mode(),
                crate::Mode::Horizontal | crate::Mode::RestrictedHorizontal
            )
            && !input.has_active_alignment()
            && !stores.world().execution_tracing_enabled()
        {
            macro_text.clear();
            if input.append_macro_text_span_bounded(
                stores,
                &mut macro_text,
                EXECUTION_TEXT_SPAN_CHUNK,
            ) > 0
            {
                stats.delivered_tokens += macro_text.len();
                stats.macro_text_span_tokens += macro_text.len();
                for token in macro_text.drain(..) {
                    execution.count_paragraph_token();
                    let appended = assignments::try_append_character(nest, token, stores)?;
                    debug_assert!(appended);
                }
                if observe(nest, input, stores, BoundaryEvent::default()) {
                    return Ok(MainControlExit::Stopped);
                }
                continue;
            }
            if input.append_source_text_span_bounded(
                stores,
                &mut macro_text,
                EXECUTION_TEXT_SPAN_CHUNK,
            ) > 0
            {
                stats.delivered_tokens += macro_text.len();
                stats.source_text_span_tokens += macro_text.len();
                for token in macro_text.drain(..) {
                    execution.count_paragraph_token();
                    let appended = assignments::try_append_character(nest, token, stores)?;
                    debug_assert!(appended);
                }
                if observe(nest, input, stores, BoundaryEvent::default()) {
                    return Ok(MainControlExit::Stopped);
                }
                continue;
            }
        }

        let before_mode = nest.current_mode();
        let before_depth = nest.depth();
        let before_artifacts = stores.world().artifact_commits().len();
        sync_engine_state(execution, nest, stores);
        let token = {
            let mut expansion = tex_state::ExpansionContext::new(stores);
            match get_x_token_with_context(input, &mut expansion, execution) {
                Ok(token) => token,
                Err(tex_expand::ExpandError::Captured { error, site }) => match *error {
                    tex_expand::ExpandError::UndefinedControlSequence { name, .. } => {
                        stores.world_mut().write_text(
                            tex_state::PrintSink::TerminalAndLog,
                            &format!("\n! Undefined control sequence \\{name}.\n"),
                        );
                        continue;
                    }
                    tex_expand::ExpandError::MacroCall(
                        tex_expand::args::MacroCallError::DoesNotMatchDefinition {
                            macro_name, ..
                        },
                    ) => {
                        stores.world_mut().write_text(
                            tex_state::PrintSink::TerminalAndLog,
                            &format!("\n! Use of {macro_name} doesn't match its definition.\n"),
                        );
                        continue;
                    }
                    tex_expand::ExpandError::MacroCall(
                        tex_expand::args::MacroCallError::ParagraphEndedBeforeComplete {
                            macro_name,
                            context,
                        }
                        | tex_expand::args::MacroCallError::ForbiddenOuterToken {
                            macro_name,
                            context,
                        },
                    ) => {
                        crate::push_traced_tokens(input, stores, [context]);
                        stores.world_mut().write_text(
                            tex_state::PrintSink::TerminalAndLog,
                            &format!(
                                "\n! Runaway argument while scanning use of {macro_name}.\nThe terminating token will be read again.\n"
                            ),
                        );
                        continue;
                    }
                    tex_expand::ExpandError::ExtraConditionalControl { name, .. } => {
                        crate::diagnostics::report_extra_conditional(stores, name);
                        continue;
                    }
                    error => {
                        let summary = input.publication_summary(stores);
                        stores.set_input_summary(summary);
                        return Err(tex_expand::ExpandError::Captured {
                            error: Box::new(error),
                            site,
                        }
                        .into());
                    }
                },
                Err(tex_expand::ExpandError::UndefinedControlSequence { name, .. }) => {
                    // In TeX.web main_control, undefined control sequences
                    // report an error and otherwise behave like a consumed
                    // relax token. Scanner-owned expansion errors still
                    // propagate from their scanner call sites.
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        &format!("\n! Undefined control sequence \\{name}.\n"),
                    );
                    continue;
                }
                Err(tex_expand::ExpandError::ExtraConditionalControl { name, .. }) => {
                    crate::diagnostics::report_extra_conditional(stores, name);
                    continue;
                }
                Err(tex_expand::ExpandError::Lex(tex_lex::LexError::InvalidCharacter {
                    ch,
                    ..
                })) => {
                    // TeX.web's `get_next` reports an invalid-category input
                    // character and restarts tokenization after consuming it.
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        &format!("\n! Text line contains an invalid character ({ch}).\n"),
                    );
                    continue;
                }
                Err(tex_expand::ExpandError::MacroCall(
                    tex_expand::args::MacroCallError::DoesNotMatchDefinition { macro_name, .. },
                )) => {
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        &format!("\n! Use of {macro_name} doesn't match its definition.\n"),
                    );
                    continue;
                }
                Err(tex_expand::ExpandError::MacroCall(
                    tex_expand::args::MacroCallError::ParagraphEndedBeforeComplete {
                        macro_name,
                        context,
                    }
                    | tex_expand::args::MacroCallError::ForbiddenOuterToken {
                        macro_name,
                        context,
                    },
                )) => {
                    // With scanner_status=matching, TeX.web §336 aborts the
                    // partial macro call and inserts/replays the token that
                    // terminated it (normally \par or an outer control
                    // sequence).
                    crate::push_traced_tokens(input, stores, [context]);
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        &format!(
                            "\n! Runaway argument while scanning use of {macro_name}.\nThe terminating token will be read again.\n"
                        ),
                    );
                    continue;
                }
                Err(err) => {
                    let summary = input.publication_summary(stores);
                    stores.set_input_summary(summary);
                    return Err(err.into());
                }
            }
        };
        let Some(token) = token else {
            abandon_stale_vertical_paragraph_probe(nest, stores, execution);
            assignments::flush_pending_hchars(nest, stores)?;
            return Ok(MainControlExit::EndOfInput);
        };
        if stores.world().execution_tracing_enabled() {
            let message = format!(
                "deliver {:?} in {:?}",
                tex_expand::semantic_token(token),
                nest.current_mode()
            );
            stores.world_mut().trace_execution("executor", message);
        }
        stats.delivered_tokens += 1;
        stats.main_control_dispatches += 1;
        execution.count_paragraph_token();
        let action =
            match dispatch_delivered_token_with_context(nest, token, input, stores, execution) {
                Ok(action) => action,
                Err(ExecError::Expand(tex_expand::ExpandError::UndefinedControlSequence {
                    name,
                    ..
                })) => {
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        &format!("\n! Undefined control sequence \\{name}.\n"),
                    );
                    continue;
                }
                Err(ExecError::Expand(tex_expand::ExpandError::Captured { error, .. }))
                    if matches!(
                        error.as_ref(),
                        tex_expand::ExpandError::UndefinedControlSequence { .. }
                    ) =>
                {
                    let tex_expand::ExpandError::UndefinedControlSequence { name, .. } = *error
                    else {
                        unreachable!("guard restricts captured expansion error")
                    };
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        &format!("\n! Undefined control sequence \\{name}.\n"),
                    );
                    continue;
                }
                Err(ExecError::UnsupportedAssignmentTarget) => {
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        "\n! Improper assignment target; this assignment is ignored.\n",
                    );
                    continue;
                }
                Err(
                    ExecError::UnexpectedMacroDelivery { .. }
                    | ExecError::UnexpectedExpandableDelivery { .. },
                ) => continue,
                Err(ExecError::ExtraConditionalControl { primitive, .. }) => {
                    let name = match primitive {
                        tex_state::meaning::ExpandablePrimitive::Else => "else",
                        tex_state::meaning::ExpandablePrimitive::Fi => "fi",
                        tex_state::meaning::ExpandablePrimitive::Or => "or",
                        _ => unreachable!("error variant is restricted to conditional controls"),
                    };
                    crate::diagnostics::report_extra_conditional(stores, name);
                    continue;
                }
                Err(
                    ExecError::ExtraRightBraceOrForgottenEndgroup { .. }
                    | ExecError::ExtraRightBraceOrForgottenDollar { .. }
                    | ExecError::TooManyRightBraces { .. }
                    | ExecError::ExtraEndGroup { .. }
                    | ExecError::EndGroupMismatch { .. }
                    | ExecError::MathShiftGroupMismatch { .. },
                ) => continue,
                Err(err) => {
                    let summary = input.publication_summary(stores);
                    stores.set_input_summary(summary);
                    return Err(err);
                }
            };
        match action {
            DispatchAction::Continue => {
                output::drain_pending_output(nest, input, stores, execution, stats)?;
            }
            DispatchAction::Shipout(page) => {
                stats.prepared_dvi_pages.push(page);
                output::drain_pending_output(nest, input, stores, execution, stats)?;
            }
            DispatchAction::End => {
                stats.dumped_format = match tex_expand::semantic_token(token) {
                    tex_state::token::Token::Cs(symbol) => matches!(
                        stores.meaning(symbol),
                        tex_state::meaning::Meaning::UnexpandablePrimitive(
                            tex_state::meaning::UnexpandablePrimitive::Dump
                        )
                    ),
                    _ => false,
                };
                assignments::flush_pending_hchars(nest, stores)?;
                abandon_stale_vertical_paragraph_probe(nest, stores, execution);
                return Ok(MainControlExit::End { token });
            }
            DispatchAction::NotConsumed => {
                return Ok(MainControlExit::NotConsumed { token });
            }
        }
        #[cfg(feature = "profiling-stats")]
        if before_mode == crate::Mode::Vertical
            && before_depth == 1
            && nest.current_mode() == crate::Mode::Horizontal
            && nest.depth() == 2
            && stores.paragraph_memo_enabled()
        {
            let anchored = execution
                .cold_paragraph_recording
                .as_ref()
                .map(|recording| recording.starting_span.is_some())
                .or(paragraph_probe_anchor);
            stores.record_pure_paragraph_cold_start(anchored);
        }
        // Paragraph alignment is probed while the engine is still in outer
        // vertical mode so expansion reads made by the paragraph-starting
        // token are part of the recording. That probe is provisional: a
        // command which leaves us in vertical mode belongs to the vertical
        // prelude, not to the following paragraph. Discard it here so glue,
        // penalties, assignments, and macro-expanded vertical commands can
        // never be swallowed by a retained paragraph hlist.
        if before_mode == crate::Mode::Vertical
            && nest.current_mode() == crate::Mode::Vertical
            && execution.cold_paragraph_recording.is_some()
        {
            abandon_stale_vertical_paragraph_probe(nest, stores, execution);
        }
        if before_mode == crate::Mode::Horizontal
            && before_depth == 2
            && nest.current_mode() == crate::Mode::Vertical
            && nest.depth() == 1
        {
            execution.paragraph_memo_barrier = false;
        }
        if observe(
            nest,
            input,
            stores,
            BoundaryEvent {
                outer_paragraph_end: before_mode == crate::Mode::Horizontal
                    && before_depth == 2
                    && nest.current_mode() == crate::Mode::Vertical
                    && nest.depth() == 1,
                shipout_complete: stores.world().artifact_commits().len() != before_artifacts,
            },
        ) {
            return Ok(MainControlExit::Stopped);
        }
    }
}

fn abandon_stale_vertical_paragraph_probe(
    nest: &ModeNest,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) {
    if nest.current_mode() != crate::Mode::Vertical || execution.cold_paragraph_recording.is_none()
    {
        return;
    }
    let abandoned_state_recording = stores.abandon_pure_paragraph_recording();
    debug_assert!(
        abandoned_state_recording,
        "provisional paragraph recording must have a state checkpoint"
    );
    execution.abandon_cold_paragraph_recording();
}

pub(crate) fn sync_engine_state(
    execution: &mut crate::ExecutionContext<'_>,
    nest: &ModeNest,
    stores: &Universe,
) {
    execution.engine = engine_state_snapshot(nest, stores);
}

fn engine_state_snapshot(nest: &ModeNest, stores: &Universe) -> EngineStateSnapshot {
    let list = nest.current_list();
    let mut state = EngineStateSnapshot {
        mode: nest.current_mode().engine_mode(),
        is_inner_mode: nest.current_mode().is_inner(),
        space_factor: list.space_factor(),
        prev_depth: list.prev_depth().unwrap_or_else(|| ignored_depth(stores)),
        prev_graf: nest.enclosing_vertical_prev_graf(),
        par_shape_len: stores.paragraph_shape_len().min(i32::MAX as usize) as i32,
        ..EngineStateSnapshot::default()
    };
    if is_outer_vertical(nest) {
        match stores.page_contribution_tail() {
            Some(Node::Penalty(value)) => state.last_penalty = *value,
            Some(Node::Kern { amount, .. }) => state.last_kern = *amount,
            Some(Node::Glue { spec, .. }) => state.last_skip = stores.glue(*spec),
            Some(_) => {}
            None => {
                state.last_penalty = stores.page_last_penalty();
                state.last_kern = stores.page_last_kern();
                state.last_skip = stores.page_last_skip();
            }
        }
    } else {
        match list.nodes().last() {
            Some(Node::Penalty(value)) => state.last_penalty = *value,
            Some(Node::Kern { amount, .. }) => state.last_kern = *amount,
            Some(Node::Glue { spec, .. }) => state.last_skip = stores.glue(*spec),
            _ => {}
        }
    }
    let effective_tail = if is_outer_vertical(nest) {
        stores
            .page_contribution_tail()
            .or_else(|| stores.current_page_tail())
    } else {
        list.nodes().last()
    };
    state.last_node_type = effective_tail.map_or_else(
        || {
            if is_outer_vertical(nest) {
                stores.page_last_node_type()
            } else {
                -1
            }
        },
        Node::etex_type,
    );
    state
}
