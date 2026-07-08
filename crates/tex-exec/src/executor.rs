use tex_expand::{
    EngineMode, ExpansionHooks, NoopRecorder, ReadRecorder, get_x_token_with_recorder_and_hooks,
};
use tex_lex::{InputSource, InputStack};
use tex_state::stores::Stores;

use crate::dispatch::unimplemented_typesetting;
use crate::{
    DispatchAction, ExecError, ExecutionStats, LogSink, ModeNest, NoopLogSink,
    dispatch_delivered_token_with_log_sink,
};

/// Stomach interpreter state.
#[derive(Clone, Debug, Eq, PartialEq)]
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
        stores: &mut Stores,
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
        stores: &mut Stores,
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
        stores: &mut Stores,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<ExecutionStats, ExecError>
    where
        S: InputSource,
        R: ReadRecorder,
        H: ExpansionHooks<S>,
    {
        self.run_with_recorder_and_hooks_and_log_sink(
            input,
            stores,
            recorder,
            hooks,
            &mut NoopLogSink,
        )
    }

    /// Runs main control with expansion hooks and a diagnostic log sink.
    pub fn run_with_recorder_and_hooks_and_log_sink<S, R, H, L>(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut Stores,
        recorder: &mut R,
        hooks: &mut H,
        log: &mut L,
    ) -> Result<ExecutionStats, ExecError>
    where
        S: InputSource,
        R: ReadRecorder,
        H: ExpansionHooks<S>,
        L: LogSink,
    {
        let mut stats = ExecutionStats::default();
        loop {
            let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
            else {
                return Ok(stats);
            };
            stats.delivered_tokens += 1;
            match dispatch_delivered_token_with_log_sink(
                self.nest.current_mode(),
                token,
                input,
                stores,
                hooks,
                log,
            )? {
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
    fn open_input(&mut self, _name: &str) -> Result<S, String> {
        Err("execution input hook is not installed".to_owned())
    }
}

impl<S> ExpansionHooks<S> for Executor
where
    S: InputSource,
{
    fn open_input(&mut self, _name: &str) -> Result<S, String> {
        Err("execution input hook is not installed".to_owned())
    }

    fn mode(&self) -> EngineMode {
        self.nest.current_mode().engine_mode()
    }

    fn is_inner_mode(&self) -> bool {
        self.nest.current_mode().is_inner()
    }
}
