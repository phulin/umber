use tex_expand::{
    EngineMode, ExpansionHooks, NoopRecorder, ReadRecorder, get_x_token_with_recorder_and_hooks,
};
use tex_lex::{InputSource, InputStack};
use tex_state::{ExpansionCtx, Universe};

use crate::dispatch::unimplemented_typesetting;
use crate::{DispatchAction, ExecError, ExecutionStats, ModeNest, dispatch_delivered_token};

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
        let mut stats = ExecutionStats::default();
        loop {
            let token = {
                let mut expansion = ExpansionCtx::new(stores);
                get_x_token_with_recorder_and_hooks(input, &mut expansion, recorder, hooks)?
            };
            let Some(token) = token else {
                return Ok(stats);
            };
            stats.delivered_tokens += 1;
            match dispatch_delivered_token(&mut self.nest, token, input, stores, hooks)? {
                DispatchAction::Continue => {}
                DispatchAction::End => return Ok(stats),
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
}
