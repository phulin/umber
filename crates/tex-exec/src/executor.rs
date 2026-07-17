use std::collections::{BTreeMap, VecDeque};
use std::ops::{Deref, DerefMut};
use std::path::Path;

use tex_expand::{EngineStateSnapshot, InputResolver, ReadRecorder, get_x_token_with_context};
use tex_lex::InputStack;
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
    ) -> Result<FontSource, String>;
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
    ) -> Result<tex_state::PdfExternalImageSource, String>;
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
pub(crate) struct PendingParagraphMemo;

pub(crate) struct ColdParagraphRecording {
    pub(crate) effect_start: usize,
    pub(crate) starting_span: Option<tex_state::RootSpanId>,
    pub(crate) starting_group_depth: u32,
    pub(crate) starting_group_changed_at: tex_state::ChangedAt,
    pub(crate) delivered_tokens: usize,
    pub(crate) barriers: std::collections::BTreeSet<ParagraphBarrierReason>,
}

pub struct ExecutionContext<'a> {
    expansion: tex_expand::ExpansionContext<'a>,
    font_resolver: Option<&'a mut dyn FontResolver>,
    image_resolver: Option<&'a mut dyn PdfImageResolver>,
    pub(crate) pending_paragraph_memo: Option<PendingParagraphMemo>,
    pub(crate) paragraph_memo_barrier: bool,
    pub(crate) cold_paragraph_recording: Option<ColdParagraphRecording>,
    /// Detached paragraph observations reusable only while their authoritative
    /// changed-at stamps remain equal during this execution run.
    pub(crate) paragraph_dependency_cache:
        BTreeMap<tex_state::DependencyKey, tex_state::ObservedDependency>,
}

impl<'a> ExecutionContext<'a> {
    #[must_use]
    pub fn new(job_name: &'a str) -> Self {
        Self {
            expansion: tex_expand::ExpansionContext::new(job_name),
            font_resolver: None,
            image_resolver: None,
            pending_paragraph_memo: None,
            paragraph_memo_barrier: false,
            cold_paragraph_recording: None,
            paragraph_dependency_cache: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_resolvers(
        job_name: &'a str,
        input_resolver: &'a mut dyn InputResolver,
        font_resolver: &'a mut dyn FontResolver,
    ) -> Self {
        Self {
            expansion: tex_expand::ExpansionContext::with_input_resolver(job_name, input_resolver),
            font_resolver: Some(font_resolver),
            image_resolver: None,
            pending_paragraph_memo: None,
            paragraph_memo_barrier: false,
            cold_paragraph_recording: None,
            paragraph_dependency_cache: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_resource_resolvers(
        job_name: &'a str,
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
            cold_paragraph_recording: None,
            paragraph_dependency_cache: BTreeMap::new(),
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
    ) -> Result<FontSource, String> {
        let request_index = self.expansion.next_resolution_index();
        match self.font_resolver.as_deref_mut() {
            Some(resolver) => resolver.open_font(input, path, request_index),
            None => input
                .read_input_file(path)
                .map(|metrics| FontSource::Tfm {
                    metrics,
                    opentype: None,
                })
                .map_err(|error| error.to_string()),
        }
    }

    pub(crate) fn open_pdf_image(
        &mut self,
        input: &mut dyn InputReadState,
        request: &PdfImageRequest,
    ) -> Result<tex_state::PdfExternalImageSource, String> {
        let request_index = self.expansion.next_resolution_index();
        self.image_resolver
            .as_deref_mut()
            .ok_or_else(|| format!("PDF image {} has no host resolver", request.name))?
            .open_image(input, request, request_index)
    }

    pub(crate) fn begin_cold_paragraph_recording(
        &mut self,
        effect_start: usize,
        starting_span: Option<tex_state::RootSpanId>,
        starting_group_depth: u32,
        starting_group_changed_at: tex_state::ChangedAt,
    ) -> bool {
        if self.cold_paragraph_recording.is_some() {
            return false;
        }
        self.expansion.begin_paragraph_recording();
        self.cold_paragraph_recording = Some(ColdParagraphRecording {
            effect_start,
            starting_span,
            starting_group_depth,
            starting_group_changed_at,
            delivered_tokens: 0,
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
        starting_group_changed_at: tex_state::ChangedAt,
    ) {
        if let Some(recording) = &mut self.cold_paragraph_recording
            && starting_span.is_some()
        {
            recording.starting_span = starting_span;
            recording.starting_group_depth = starting_group_depth;
            recording.starting_group_changed_at = starting_group_changed_at;
        }
    }

    pub(crate) fn abandon_cold_paragraph_recording(&mut self) {
        self.cold_paragraph_recording = None;
        let _ = self.expansion.finish_paragraph_recording();
    }

    pub(crate) fn mark_paragraph_barrier(&mut self, reason: ParagraphBarrierReason) {
        if let Some(recording) = &mut self.cold_paragraph_recording {
            recording.barriers.insert(reason);
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
        input.ensure_source_ids_at_least(stores.input_summary().next_source_id());
        execution.job_clock = stores.world().job_clock();
        let mut session = EngineSession::new(checkpoints);
        if publish_job_start {
            let every_job = stores.take_pending_every_job();
            if every_job != TokenListId::EMPTY {
                input.push_token_list(every_job, TokenListReplayKind::EveryJob);
            }
            session.publish(EngineBoundary::JobStart, &self.nest, input, stores);
        }
        let artifact_start = stores.world().artifact_commits().len();
        let mut stats = ExecutionStats::default();
        let exit = match run_outer_main_control_until(
            &mut self.nest,
            input,
            stores,
            execution,
            &mut stats,
            &mut session,
        ) {
            Ok(exit) => exit,
            Err(err) => {
                let summary = input.publication_summary(stores);
                stores.set_input_summary(summary);
                return Err(err);
            }
        };
        let result = match exit {
            MainControlExit::EndOfInput => Ok(stats),
            MainControlExit::Stopped => Ok(stats),
            MainControlExit::End { .. } => {
                if let Err(err) =
                    output::finish_end(&mut self.nest, input, stores, execution, &mut stats)
                {
                    let summary = input.publication_summary(stores);
                    stores.set_input_summary(summary);
                    return Err(err.capture(input));
                }
                Ok(stats)
            }
            MainControlExit::NotConsumed { token } => Err(unimplemented_typesetting(
                self.nest.current_mode(),
                tex_expand::semantic_token(token),
                token.origin(),
                "non-assignment command",
            )
            .expect_err("unimplemented_typesetting always returns Err")
            .capture(input)),
        };
        let dumped_format = result.as_ref().is_ok_and(|stats| stats.dumped_format);
        let summary = if dumped_format {
            // TeX's `\dump` ends INITEX immediately. The remaining source
            // frames belong to the terminated job, while format images are a
            // quiescent semantic-state boundary and intentionally exclude
            // input cursors.
            InputSummary::default()
        } else {
            input.publication_summary(stores)
        };
        if dumped_format {
            // Page-builder bookkeeping is likewise job-local and is not part
            // of a TeX format image.
            stores.start_new_page();
        }
        stores.set_input_summary(summary);
        result.and_then(|mut stats| {
            stats.shipped_artifacts = stores.world().artifact_commits()[artifact_start..].to_vec();
            let mut prepared = BTreeMap::<_, VecDeque<_>>::new();
            for page in std::mem::take(&mut stats.prepared_dvi_pages) {
                prepared.entry(page.hash).or_default().push_back(page.plan);
            }
            stats.dvi_pages = stores.world().committed_artifacts()[artifact_start..]
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
                .collect::<Result<Vec<_>, _>>()?;
            Ok(stats)
        })
    }
}

fn run_outer_main_control_until<C>(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
    session: &mut EngineSession<'_, C>,
) -> Result<MainControlExit, ExecError>
where
    C: CheckpointSink,
{
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
                if session.stop_requested() {
                    return true;
                }
            }
            if event.outer_paragraph_end {
                session.publish(EngineBoundary::OuterParagraphEnd, nest, input, stores);
            }
            session.stop_requested()
        },
    );
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
        abandon_stale_vertical_paragraph_probe(nest, stores, execution);
        report_recoverable_expansion_diagnostics(execution, stores);
        if should_stop(input, stores) {
            return Ok(MainControlExit::Stopped);
        }

        if allow_text_spans
            && nest.current_mode() == crate::Mode::Vertical
            && execution.pending_paragraph_memo.is_none()
            && !execution.paragraph_memo_barrier
            && stores.paragraph_memo_enabled()
        {
            let starting_span = input.current_root_delivery_anchor(stores)?;
            if let Some(starting_span) = starting_span {
                let group_key =
                    tex_state::DependencyKey::Engine(tex_state::DependencyEngineField::GroupLevel);
                let starting_group_changed_at = stores.track_dependency(group_key);
                if execution.cold_paragraph_recording.is_none()
                    && execution.begin_cold_paragraph_recording(
                        stores.world().effect_records().len(),
                        Some(starting_span),
                        tex_state::ExpansionState::execution_group_depth(stores),
                        starting_group_changed_at,
                    )
                {
                    stores.begin_pure_paragraph_recording();
                }
                execution.update_cold_paragraph_start(
                    Some(starting_span),
                    tex_state::ExpansionState::execution_group_depth(stores),
                    starting_group_changed_at,
                );
                if execution.cold_paragraph_recording.is_some() {
                    let before_artifacts = stores.world().artifact_commits().len();
                    if crate::paragraph_memo::try_reuse_aligned_paragraph(
                        Some(starting_span),
                        nest,
                        input,
                        stores,
                        execution,
                        stats,
                    )? {
                        output::drain_pending_output(nest, input, stores, execution, stats)?;
                        execution.paragraph_memo_barrier = false;
                        if observe(
                            nest,
                            input,
                            stores,
                            BoundaryEvent {
                                outer_paragraph_end: true,
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
            if input.append_macro_text_span(stores, &mut macro_text) > 0 {
                stats.delivered_tokens += macro_text.len();
                stats.macro_text_span_tokens += macro_text.len();
                for token in macro_text.drain(..) {
                    execution.count_paragraph_token();
                    let appended = assignments::try_append_character(nest, token, stores)?;
                    debug_assert!(appended);
                }
                continue;
            }
            if input.append_source_text_span(stores, &mut macro_text) > 0 {
                stats.delivered_tokens += macro_text.len();
                stats.source_text_span_tokens += macro_text.len();
                for token in macro_text.drain(..) {
                    execution.count_paragraph_token();
                    let appended = assignments::try_append_character(nest, token, stores)?;
                    debug_assert!(appended);
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
