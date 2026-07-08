use tex_exec::Executor;
use tex_expand::{ExpansionHooks, NoopRecorder};
use tex_lex::{InputSource, InputStack, MemoryInput};
use tex_state::env::banks::IntParam;
use tex_state::{ContentHash, EffectRecord, PrintSink, Universe};

/// Result of running TeX through the batch executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunResult {
    pub terminal_text: String,
    pub artifacts: Vec<ContentHash>,
}

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
    run_input_collecting_artifacts(input, stores, hooks).map(|result| result.terminal_text)
}

/// Runs input and returns the artifact ids emitted by `\shipout` in order.
pub fn run_input_collecting_artifacts<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<RunResult, tex_exec::ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let stats = Executor::new().run_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?;
    Ok(RunResult {
        terminal_text: uncommitted_terminal_text(stores),
        artifacts: stats.shipped_artifacts,
    })
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
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        _input: &mut C,
        name: &str,
    ) -> Result<MemoryInput, String> {
        Err(format!("memory run cannot open input {name}"))
    }

    fn job_name(&self) -> &str {
        "texput"
    }
}

fn uncommitted_terminal_text(stores: &Universe) -> String {
    let mut text = String::new();
    for record in stores.world().effect_records() {
        let EffectRecord::StreamWrite { sink, text: chunk } = record else {
            continue;
        };
        match sink {
            PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log => {
                text.push_str(chunk);
            }
            PrintSink::Stream(_) => {}
        }
    }
    text
}
