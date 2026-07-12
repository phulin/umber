use tex_expand::{
    EngineMode, EngineStateSnapshot, ExpansionHooks, NoopRecorder, ReadRecorder,
    get_x_token_with_recorder_and_hooks,
};
use tex_lex::{InputSource, InputStack};
use tex_state::glue::GlueSpec;
use tex_state::node::Node;
use tex_state::scaled::Scaled;
use tex_state::token::TracedTokenWord;
use tex_state::{ExpansionContext, InputSummary, Universe};

use crate::checkpoint::{CheckpointSink, EngineBoundary, EngineSession, NoopCheckpointSink};
use crate::dispatch::{dispatch_delivered_token_with_recorder, unimplemented_typesetting};
use crate::mode::IGNORE_DEPTH;
use crate::output;
use crate::vertical::is_outer_vertical;
use crate::{DispatchAction, ExecError, ExecutionStats, ModeNest, assignments};

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
    pub fn run<S>(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut Universe,
    ) -> Result<ExecutionStats, ExecError>
    where
        S: InputSource,
    {
        self.run_with_recorder(input, stores, &mut NoopRecorder)
    }

    /// Runs main control while recording expansion meaning reads.
    pub fn run_with_recorder<S, R>(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut Universe,
        recorder: &mut R,
    ) -> Result<ExecutionStats, ExecError>
    where
        S: InputSource,
        R: ReadRecorder,
    {
        let mut hooks = NoopExecHooks;
        self.run_with_recorder_and_hooks(input, stores, recorder, &mut hooks)
    }

    /// Runs main control while recording reads and using driver expansion hooks.
    pub fn run_with_recorder_and_hooks<S, R, H>(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut Universe,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<ExecutionStats, ExecError>
    where
        S: InputSource,
        R: ReadRecorder,
        H: ExpansionHooks<S>,
    {
        let mut checkpoints = NoopCheckpointSink;
        self.run_with_recorder_hooks_and_checkpoints(
            input,
            stores,
            recorder,
            hooks,
            &mut checkpoints,
        )
    }

    /// Runs main control and publishes restartable state at named safe boundaries.
    pub fn run_with_recorder_hooks_and_checkpoints<S, R, H, C>(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut Universe,
        recorder: &mut R,
        hooks: &mut H,
        checkpoints: &mut C,
    ) -> Result<ExecutionStats, ExecError>
    where
        S: InputSource,
        R: ReadRecorder,
        H: ExpansionHooks<S>,
        C: CheckpointSink,
    {
        input.ensure_source_ids_at_least(stores.input_summary().next_source_id());
        let mut session = EngineSession::new(checkpoints);
        session.publish(EngineBoundary::JobStart, &self.nest, input, stores);
        let artifact_start = stores.world().artifact_commits().len();
        let mut exec_hooks = ExecExpansionHooks::new(hooks);
        let mut stats = ExecutionStats::default();
        let exit = match run_outer_main_control_until(
            &mut self.nest,
            input,
            stores,
            recorder,
            &mut exec_hooks,
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
                if let Err(err) = output::finish_end(
                    &mut self.nest,
                    input,
                    stores,
                    recorder,
                    &mut exec_hooks,
                    &mut stats,
                ) {
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
        result.map(|mut stats| {
            stats.shipped_artifacts = stores.world().artifact_commits()[artifact_start..].to_vec();
            stats
        })
    }
}

fn run_outer_main_control_until<S, R, H, C>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
    session: &mut EngineSession<'_, C>,
) -> Result<MainControlExit, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    C: CheckpointSink,
{
    let result = run_main_control_until_observing(
        nest,
        input,
        stores,
        recorder,
        hooks,
        stats,
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

pub(crate) fn run_main_control_until<S, R, H, F>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
    should_stop: F,
) -> Result<MainControlExit, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    F: FnMut(&mut InputStack<S>, &Universe) -> bool,
{
    let result = run_main_control_until_observing(
        nest,
        input,
        stores,
        recorder,
        hooks,
        stats,
        should_stop,
        |_, _, _, _| {},
    );
    result.map_err(|error| error.capture(input))
}

#[allow(clippy::too_many_arguments)]
fn run_main_control_until_observing<S, R, H, F, O>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
    mut should_stop: F,
    mut observe: O,
) -> Result<MainControlExit, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    F: FnMut(&mut InputStack<S>, &Universe) -> bool,
    O: FnMut(&ModeNest, &mut InputStack<S>, &mut Universe, BoundaryEvent),
{
    loop {
        if should_stop(input, stores) {
            return Ok(MainControlExit::Stopped);
        }

        let before_mode = nest.current_mode();
        let before_depth = nest.depth();
        let before_artifacts = stores.world().artifact_commits().len();
        sync_engine_state::<S, _>(hooks, nest, stores);
        let token = {
            let mut expansion = ExpansionContext::new(stores);
            match get_x_token_with_recorder_and_hooks(input, &mut expansion, recorder, hooks) {
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
                        stores.world_mut().write_text(
                            tex_state::PrintSink::TerminalAndLog,
                            &format!("\n! Extra \\{name}.\nI'm ignoring this condition command.\n"),
                        );
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
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        &format!("\n! Extra \\{name}.\nI'm ignoring this condition command.\n"),
                    );
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
        let action = match dispatch_delivered_token_with_recorder(
            nest, token, input, stores, recorder, hooks,
        ) {
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
                let tex_expand::ExpandError::UndefinedControlSequence { name, .. } = *error else {
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
                output::drain_pending_output(nest, input, stores, recorder, hooks, stats)?;
            }
            DispatchAction::Shipout(artifact) => {
                let _ = artifact;
                output::drain_pending_output(nest, input, stores, recorder, hooks, stats)?;
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

pub(crate) fn sync_engine_state<S, H>(hooks: &mut H, nest: &ModeNest, stores: &Universe)
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    hooks.set_engine_state(engine_state_snapshot(nest, stores));
}

fn engine_state_snapshot(nest: &ModeNest, stores: &Universe) -> EngineStateSnapshot {
    let list = nest.current_list();
    let mut state = EngineStateSnapshot {
        mode: nest.current_mode().engine_mode(),
        is_inner_mode: nest.current_mode().is_inner(),
        space_factor: list.space_factor(),
        prev_depth: list.prev_depth().unwrap_or(IGNORE_DEPTH),
        prev_graf: nest.enclosing_vertical_prev_graf(),
        par_shape_len: stores.paragraph_shape().len().min(i32::MAX as usize) as i32,
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
    state
}

struct ExecExpansionHooks<'a, H> {
    inner: &'a mut H,
    state: EngineStateSnapshot,
}

impl<'a, H> ExecExpansionHooks<'a, H> {
    fn new(inner: &'a mut H) -> Self {
        Self {
            inner,
            state: EngineStateSnapshot::default(),
        }
    }
}

impl<S, H> ExpansionHooks<S> for ExecExpansionHooks<'_, H>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        input: &mut C,
        name: &str,
    ) -> Result<S, String> {
        self.inner.open_input(input, name)
    }

    fn open_font<C: tex_state::InputReadState>(
        &mut self,
        input: &mut C,
        path: &std::path::Path,
    ) -> Result<tex_state::FileContent, String> {
        self.inner.open_font(input, path)
    }

    fn job_name(&self) -> &str {
        self.inner.job_name()
    }

    fn mode(&self) -> EngineMode {
        self.state.mode
    }

    fn is_inner_mode(&self) -> bool {
        self.state.is_inner_mode
    }

    fn space_factor(&self) -> i32 {
        self.state.space_factor
    }

    fn prev_depth(&self) -> Scaled {
        self.state.prev_depth
    }

    fn prev_graf(&self) -> i32 {
        self.state.prev_graf
    }

    fn par_shape_len(&self) -> i32 {
        self.state.par_shape_len
    }

    fn last_penalty(&self) -> i32 {
        self.state.last_penalty
    }

    fn last_kern(&self) -> Scaled {
        self.state.last_kern
    }

    fn last_skip(&self) -> GlueSpec {
        self.state.last_skip
    }

    fn input_stream_eof(&self, stores: &impl tex_state::ExpansionState, stream: u8) -> bool {
        self.inner.input_stream_eof(stores, stream)
    }

    fn set_engine_state(&mut self, state: EngineStateSnapshot) {
        self.state = state;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NoopExecHooks;

impl<S> ExpansionHooks<S> for NoopExecHooks
where
    S: InputSource,
{
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        _input: &mut C,
        _name: &str,
    ) -> Result<S, String> {
        Err("execution input hook is not installed".to_owned())
    }
}

impl<S> ExpansionHooks<S> for Executor
where
    S: InputSource,
{
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        _input: &mut C,
        _name: &str,
    ) -> Result<S, String> {
        Err("execution input hook is not installed".to_owned())
    }

    fn mode(&self) -> EngineMode {
        self.nest.current_mode().engine_mode()
    }

    fn is_inner_mode(&self) -> bool {
        self.nest.current_mode().is_inner()
    }

    fn space_factor(&self) -> i32 {
        self.nest.current_list().space_factor()
    }

    fn prev_depth(&self) -> Scaled {
        self.nest
            .current_list()
            .prev_depth()
            .unwrap_or(IGNORE_DEPTH)
    }

    fn prev_graf(&self) -> i32 {
        self.nest.enclosing_vertical_prev_graf()
    }

    fn par_shape_len(&self) -> i32 {
        0
    }
}
