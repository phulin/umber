use tex_expand::{
    EngineMode, EngineStateSnapshot, ExpansionHooks, NoopRecorder, ReadRecorder,
    get_x_token_with_recorder_and_hooks,
};
use tex_lex::{InputSource, InputStack};
use tex_state::glue::GlueSpec;
use tex_state::node::Node;
use tex_state::scaled::Scaled;
use tex_state::{ExpansionContext, Universe};

use crate::dispatch::{dispatch_delivered_token_with_recorder, unimplemented_typesetting};
use crate::mode::IGNORE_DEPTH;
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
        loop {
            sync_engine_state::<S, _>(&mut exec_hooks, &self.nest, stores);
            let token = {
                let mut expansion = ExpansionContext::new(stores);
                get_x_token_with_recorder_and_hooks(
                    input,
                    &mut expansion,
                    recorder,
                    &mut exec_hooks,
                )?
            };
            let Some(token) = token else {
                assignments::flush_pending_hchars(&mut self.nest, stores)?;
                return Ok(stats);
            };
            stats.delivered_tokens += 1;
            match dispatch_delivered_token_with_recorder(
                &mut self.nest,
                token,
                input,
                stores,
                recorder,
                &mut exec_hooks,
            )? {
                DispatchAction::Continue => {}
                DispatchAction::Shipout(artifact) => {
                    stats.shipped_artifacts.push(artifact);
                }
                DispatchAction::End => {
                    assignments::flush_pending_hchars(&mut self.nest, stores)?;
                    return Ok(stats);
                }
                DispatchAction::NotConsumed => {
                    return Err(unimplemented_typesetting(
                        self.nest.current_mode(),
                        token,
                        "non-assignment command",
                    )
                    .expect_err("unimplemented_typesetting always returns Err"));
                }
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
    match list.nodes().last() {
        Some(Node::Penalty(value)) => state.last_penalty = *value,
        Some(Node::Kern { amount, .. }) => state.last_kern = *amount,
        Some(Node::Glue { spec, .. }) => state.last_skip = stores.glue(*spec),
        _ => {}
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
