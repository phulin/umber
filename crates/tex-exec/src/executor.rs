use tex_expand::{
    EngineMode, EngineStateSnapshot, ExpansionHooks, NoopRecorder, ReadRecorder,
    get_x_token_with_recorder_and_hooks,
};
use tex_lex::{InputSource, InputStack};
use tex_state::glue::GlueSpec;
use tex_state::node::Node;
use tex_state::scaled::Scaled;
use tex_state::token::TracedTokenWord;
use tex_state::{ExpansionContext, Universe};

use crate::dispatch::{dispatch_delivered_token_with_recorder, unimplemented_typesetting};
use crate::mode::IGNORE_DEPTH;
use crate::output;
use crate::vertical::is_outer_vertical;
use crate::{DispatchAction, ExecError, ExecutionStats, ModeNest, assignments};

/// Stomach interpreter state.
#[derive(Clone, Debug, PartialEq)]
pub struct Executor {
    nest: ModeNest,
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
        let mut exec_hooks = ExecExpansionHooks::new(hooks);
        let mut stats = ExecutionStats::default();
        let exit = match run_main_control_until(
            &mut self.nest,
            input,
            stores,
            recorder,
            &mut exec_hooks,
            &mut stats,
            |_, _| false,
        ) {
            Ok(exit) => exit,
            Err(err) => {
                stores.set_input_summary(input.summary());
                return Err(err);
            }
        };
        match exit {
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
                    stores.set_input_summary(input.summary());
                    return Err(err);
                }
                Ok(stats)
            }
            MainControlExit::NotConsumed { token } => Err(unimplemented_typesetting(
                self.nest.current_mode(),
                tex_expand::semantic_token(token),
                token.origin(),
                "non-assignment command",
            )
            .expect_err("unimplemented_typesetting always returns Err")),
        }
    }
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
    mut should_stop: F,
) -> Result<MainControlExit, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    F: FnMut(&mut InputStack<S>, &Universe) -> bool,
{
    loop {
        if should_stop(input, stores) {
            return Ok(MainControlExit::Stopped);
        }

        sync_engine_state::<S, _>(hooks, nest, stores);
        let token = {
            let mut expansion = ExpansionContext::new(stores);
            match get_x_token_with_recorder_and_hooks(input, &mut expansion, recorder, hooks) {
                Ok(token) => token,
                Err(err) => {
                    stores.set_input_summary(input.summary());
                    return Err(err.into());
                }
            }
        };
        let Some(token) = token else {
            assignments::flush_pending_hchars(nest, stores)?;
            return Ok(MainControlExit::EndOfInput);
        };
        stats.delivered_tokens += 1;
        let action = match dispatch_delivered_token_with_recorder(
            nest, token, input, stores, recorder, hooks,
        ) {
            Ok(action) => action,
            Err(err) => {
                stores.set_input_summary(input.summary());
                return Err(err);
            }
        };
        match action {
            DispatchAction::Continue => {
                output::drain_pending_output(nest, input, stores, recorder, hooks, stats)?;
            }
            DispatchAction::Shipout(artifact) => {
                stats.shipped_artifacts.push(artifact);
                output::drain_pending_output(nest, input, stores, recorder, hooks, stats)?;
            }
            DispatchAction::End => {
                assignments::flush_pending_hchars(nest, stores)?;
                return Ok(MainControlExit::End { token });
            }
            DispatchAction::NotConsumed => {
                return Ok(MainControlExit::NotConsumed { token });
            }
        }
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
}
