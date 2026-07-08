use tex_exec::Executor;
use tex_expand::{ExpansionHooks, NoopRecorder};
use tex_lex::{InputSource, InputStack, MemoryInput};
use tex_state::env::banks::IntParam;
use tex_state::{EffectRecord, PrintSink, Universe};

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
    Executor::new().run_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?;
    Ok(uncommitted_terminal_text(stores))
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
    fn open_input<C: tex_state::ExpansionState>(
        &mut self,
        _stores: &mut C,
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
