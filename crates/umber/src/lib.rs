use tex_exec::Executor;
use tex_expand::{ExpansionHooks, NoopRecorder};
use tex_lex::{InputSource, InputStack, MemoryInput};
use tex_out::PageArtifact;
use tex_out::dvi::{DviError, write_dvi};
use tex_state::env::banks::IntParam;
use tex_state::{ContentHash, EffectRecord, PrintSink, Universe, WorldError};

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

/// Reads committed page artifacts from `World` and writes a complete DVI file.
pub fn dvi_from_artifacts(
    stores: &Universe,
    artifacts: &[ContentHash],
) -> Result<Vec<u8>, DviBuildError> {
    let mut pages = Vec::with_capacity(artifacts.len());
    for &hash in artifacts {
        let bytes = stores
            .world()
            .read_artifact(hash)?
            .ok_or(DviBuildError::MissingArtifact(hash))?;
        pages.push(PageArtifact::from_bytes(&bytes)?);
    }
    Ok(write_dvi(&pages)?)
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

#[derive(Debug)]
pub enum DviBuildError {
    MissingArtifact(ContentHash),
    World(WorldError),
    Parse(tex_out::ParseError),
    Dvi(DviError),
}

impl std::fmt::Display for DviBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingArtifact(hash) => {
                write!(f, "shipped page artifact {} is missing", hash.hex())
            }
            Self::World(err) => write!(f, "{err}"),
            Self::Parse(err) => write!(f, "{err}"),
            Self::Dvi(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for DviBuildError {}

impl From<WorldError> for DviBuildError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<tex_out::ParseError> for DviBuildError {
    fn from(value: tex_out::ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<DviError> for DviBuildError {
    fn from(value: DviError) -> Self {
        Self::Dvi(value)
    }
}
