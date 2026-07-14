use std::collections::{BTreeMap, VecDeque};
use std::ops::{Deref, DerefMut};
use std::path::Path;

use tex_expand::{EngineStateSnapshot, InputResolver, ReadRecorder, get_x_token_with_context};
use tex_lex::InputStack;
use tex_out::dvi::DviPagePlan;
use tex_state::node::Node;
use tex_state::token::TracedTokenWord;
use tex_state::{FileContent, InputReadState, InputSummary, Universe};

use crate::checkpoint::{CheckpointSink, EngineBoundary, EngineSession, NoopCheckpointSink};
use crate::dispatch::{dispatch_delivered_token_with_context, unimplemented_typesetting};
use crate::mode::IGNORE_DEPTH;
use crate::output;
use crate::vertical::is_outer_vertical;
use crate::{DispatchAction, ExecError, ExecutionStats, ModeNest, assignments};

/// Object-safe host boundary used only by the `\font` assignment.
pub trait FontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
        request_index: u64,
    ) -> Result<FileContent, String>;
}

/// Concrete execution-session context shared by stomach operations.
///
/// Expansion scanners see this only through its concrete dereference to
/// [`tex_expand::ExpansionContext`]; font resolution remains an execution-only
/// operation and is invoked solely by `\font` assignment.
pub struct ExecutionContext<'a> {
    expansion: tex_expand::ExpansionContext<'a>,
    font_resolver: Option<&'a mut dyn FontResolver>,
}

impl<'a> ExecutionContext<'a> {
    #[must_use]
    pub fn new(job_name: &'a str) -> Self {
        Self {
            expansion: tex_expand::ExpansionContext::new(job_name),
            font_resolver: None,
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
    ) -> Result<FileContent, String> {
        let request_index = self.expansion.next_resolution_index();
        match self.font_resolver.as_deref_mut() {
            Some(resolver) => resolver.open_font(input, path, request_index),
            None => input
                .read_input_file(path)
                .map_err(|error| error.to_string()),
        }
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
        input.ensure_source_ids_at_least(stores.input_summary().next_source_id());
        let mut session = EngineSession::new(checkpoints);
        session.publish(EngineBoundary::JobStart, &self.nest, input, stores);
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
            MainControlExit::Stopped => {
                unreachable!("top-level main control has no stop condition")
            }
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
            }
            if event.outer_paragraph_end {
                session.publish(EngineBoundary::OuterParagraphEnd, nest, input, stores);
            }
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
        |_, _, _, _| {},
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
    O: FnMut(&ModeNest, &mut InputStack, &mut Universe, BoundaryEvent),
{
    let mut macro_text = Vec::new();
    loop {
        if should_stop(input, stores) {
            return Ok(MainControlExit::Stopped);
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
                    let appended = assignments::try_append_character(nest, token, stores)?;
                    debug_assert!(appended);
                }
                continue;
            }
            if input.append_source_text_span(stores, &mut macro_text) > 0 {
                stats.delivered_tokens += macro_text.len();
                stats.source_text_span_tokens += macro_text.len();
                for token in macro_text.drain(..) {
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
                            macro_name,
                            context,
                        },
                    ) => {
                        crate::push_traced_tokens(input, stores, [context]);
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
                    tex_expand::args::MacroCallError::DoesNotMatchDefinition {
                        macro_name,
                        context,
                    },
                )) => {
                    crate::push_traced_tokens(input, stores, [context]);
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
                return Ok(MainControlExit::End { token });
            }
            DispatchAction::NotConsumed => {
                return Ok(MainControlExit::NotConsumed { token });
            }
        }
        observe(
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
        );
    }
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
        prev_depth: list.prev_depth().unwrap_or(IGNORE_DEPTH),
        prev_graf: nest.enclosing_vertical_prev_graf(),
        par_shape_len: stores.paragraph_shape_len().min(i32::MAX as usize) as i32,
        ..EngineStateSnapshot::default()
    };
    if is_outer_vertical(nest) {
        match stores.page_contribution_tail() {
            Some(Node::Penalty(value)) => state.last_penalty = *value,
            Some(Node::Kern { amount, .. }) => state.last_kern = *amount,
            Some(Node::Glue { spec, .. }) => state.last_skip = stores.glue(*spec),
            Some(_) | None => {
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
