use tex_exec::{Executor, StringLogSink};
use tex_expand::{ExpansionHooks, NoopRecorder};
use tex_lex::{InputSource, InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::env::banks::IntParam;

/// Installs the primitive/state setup used by `umber run`.
pub fn prepare_run_stores(stores: &mut Universe) {
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    tex_expand::install_expandable_primitives(stores);
    tex_exec::install_unexpandable_primitives(stores);
    stores.intern("par");
}

/// Runs an already-open input stack through the same executor path as `umber run`.
pub fn run_input_with_hooks<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<String, tex_exec::ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let mut log = StringLogSink::new();
    Executor::new().run_with_recorder_and_hooks_and_log_sink(
        input,
        stores,
        &mut recorder,
        hooks,
        &mut log,
    )?;
    Ok(log.as_str().to_owned())
}

/// Runs in-memory TeX through the `umber run` executor setup.
pub fn run_memory_with_stores(
    source: &str,
    stores: &mut Universe,
) -> Result<String, tex_exec::ExecError> {
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut hooks = MemoryRunHooks;
    run_input_with_hooks(&mut input, stores, &mut hooks)
}

#[derive(Clone, Copy, Debug, Default)]
struct MemoryRunHooks;

impl ExpansionHooks<MemoryInput> for MemoryRunHooks {
    fn open_input(&mut self, _stores: &mut Universe, name: &str) -> Result<MemoryInput, String> {
        Err(format!("memory run cannot open input {name}"))
    }

    fn job_name(&self) -> &str {
        "texput"
    }
}
