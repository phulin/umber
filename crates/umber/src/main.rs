use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tex_expand::ExpansionHooks;
use tex_lex::{InputStack, Lexer, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::token::Token;
use tex_state::{FormatError, Universe, World, WorldError};
use umber::{TexFontSearchPath, TexInputSearchPath};

mod expand_dump;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("umber: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), CliError> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("lex-dump") => {
            let Some(path) = args.next() else {
                return Err(CliError::Usage("missing input path for lex-dump"));
            };
            if args.next().is_some() {
                return Err(CliError::Usage("lex-dump accepts exactly one input path"));
            }
            lex_dump(&path)
        }
        Some("expand-dump") => {
            let Some(path) = args.next() else {
                return Err(CliError::Usage("missing input path for expand-dump"));
            };
            if args.next().is_some() {
                return Err(CliError::Usage(
                    "expand-dump accepts exactly one input path",
                ));
            }
            expand_dump::expand_dump(&path).map_err(CliError::ExpandDump)
        }
        Some("run") => {
            let opts = RunCliOptions::parse(args)?;
            run_tex(&opts)
        }
        None => {
            println!("umber {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(_) => Err(CliError::Usage(
            "expected: umber <lex-dump|expand-dump|run> <file.tex>",
        )),
    }
}

fn lex_dump(path: &str) -> Result<(), CliError> {
    let mut stores = Universe::with_world(World::real());
    let content = stores.world_mut().read_file(path)?;
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(WorldInput::from_content(content));

    while let Some(token) = lexer.next_token(&mut stores)? {
        println!("{}", format_token(token, &stores));
    }

    Ok(())
}

fn run_tex(opts: &RunCliOptions) -> Result<(), CliError> {
    let path = opts.input.as_path();
    let mut world = World::real();
    let mut stores = if let Some(format) = &opts.format {
        let content = world.read_file(format)?;
        Universe::from_format(world, content.bytes())?
    } else {
        let mut stores = Universe::with_world(world);
        umber::prepare_run_stores(&mut stores);
        stores
    };
    let content = stores.world_mut().read_file(path)?;

    let mut input = InputStack::new(WorldInput::from_content(content));
    let tex_input_areas = env::var_os("TEXINPUTS")
        .map(|value| {
            env::split_paths(&value)
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    let tex_font_areas = env::var_os("TEXFONTS")
        .map(|value| {
            env::split_paths(&value)
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    let mut hooks = RunHooks::new(path, tex_input_areas, tex_font_areas);
    let run = match umber::run_input_collecting_artifacts(&mut input, &mut stores, &mut hooks) {
        Ok(run) => run,
        Err(err) => {
            return Err(CliError::RenderedExec(
                err.format_with_provenance(&stores).trim_end().to_owned(),
            ));
        }
    };
    #[cfg(feature = "node-stats")]
    for (kind, count) in tex_state::node::node_append_histogram() {
        eprintln!("NODE_HISTOGRAM {kind} {count}");
    }
    #[cfg(feature = "node-stats")]
    {
        let columns = stores.node_memory_columns();
        for column in &columns {
            eprintln!(
                "NODE_MEMORY {} len={} capacity={} element_bytes={} logical_bytes={} retained_payload_bytes={}",
                column.name,
                column.len,
                column.capacity,
                column.element_bytes,
                column.logical_bytes,
                column.retained_payload_bytes
            );
        }
        let logical: usize = columns.iter().map(|column| column.logical_bytes).sum();
        let retained: usize = columns
            .iter()
            .map(|column| column.retained_payload_bytes)
            .sum();
        eprintln!("NODE_MEMORY_TOTAL logical_bytes={logical} retained_payload_bytes={retained}");
        if let Some(peak) = tex_state::node_arena::peak_node_storage_measurement() {
            eprintln!(
                "NODE_STORAGE_PEAK logical_bytes={} retained_payload_bytes={}",
                peak.logical_bytes, peak.retained_payload_bytes
            );
            for column in peak.columns {
                eprintln!(
                    "NODE_STORAGE_PEAK_COLUMN {} len={} capacity={} element_bytes={} logical_bytes={} retained_payload_bytes={}",
                    column.name,
                    column.len,
                    column.capacity,
                    column.element_bytes,
                    column.logical_bytes,
                    column.retained_payload_bytes
                );
            }
        }
        let survivor = tex_state::survivor::survivor_measurement();
        eprintln!(
            "NODE_SURVIVOR fresh_calls={} fresh_nanos={} recycled_calls={} recycled_nanos={} release_calls={} release_nanos={} peak_scratch_logical_bytes={} peak_scratch_retained_bytes={} source_words={} child_bearing_nodes={} remap_entries={} pending_entries={} peak_remap_entries={} peak_pending_entries={}",
            survivor.fresh_promotions,
            survivor.fresh_promotion_nanos,
            survivor.recycled_promotions,
            survivor.recycled_promotion_nanos,
            survivor.releases_to_recycling,
            survivor.release_nanos,
            survivor.peak_promotion_scratch_logical_bytes,
            survivor.peak_promotion_scratch_retained_bytes,
            survivor.source_words,
            survivor.child_bearing_nodes,
            survivor.remap_entries,
            survivor.pending_entries,
            survivor.peak_remap_entries,
            survivor.peak_pending_entries
        );
        let append = tex_state::measurement::node_append_measurement();
        eprintln!(
            "ALLOC_NODE_APPEND calls={} words={} sidecar_rows={:?} growth_events={} grown_bytes={}",
            append.calls,
            append.words,
            append.sidecar_rows,
            append.capacity_growth_events,
            append.retained_payload_bytes_grown,
        );
        let clone = tex_state::measurement::epoch_clone_measurement();
        eprintln!(
            "ALLOC_EPOCH_CLONE calls={} source_words={} transient_owned_node_bytes={}",
            clone.list_calls, clone.source_words, clone.transient_owned_node_bytes,
        );
        let hash = tex_state::measurement::state_hash_measurement();
        eprintln!(
            "ALLOC_STATE_HASH calls={} journal_entries={} changed_cells={} node_frames={} owned_node_bytes={} owned_font_keys={} peak_changed_scratch_bytes={} peak_node_scratch_bytes={}",
            hash.calls,
            hash.journal_entries,
            hash.changed_cells,
            hash.node_frames,
            hash.owned_node_bytes,
            hash.owned_font_keys,
            hash.peak_changed_cell_scratch_bytes,
            hash.peak_node_scratch_bytes,
        );
        let traced = tex_state::measurement::traced_list_measurement();
        eprintln!(
            "ALLOC_TRACED_LIST finishes={} tokens={} token_builder_bytes={} origin_builder_bytes={}",
            traced.finishes,
            traced.tokens,
            traced.token_builder_retained_bytes,
            traced.origin_builder_retained_bytes,
        );
        let token_store = tex_state::measurement::token_store_measurement();
        eprintln!(
            "ALLOC_TOKEN_STORE calls={} hits={} misses={} requested_tokens={} arena_grown_bytes={}",
            token_store.intern_calls,
            token_store.hits,
            token_store.misses,
            token_store.requested_tokens,
            token_store.arena_capacity_bytes_grown,
        );
    }
    if let Some(output) = &opts.dvi {
        let dvi = umber::dvi_from_artifacts(&stores, &run.artifacts)?;
        stores.world_mut().write_file(output, dvi)?;
    }
    if run.dumped_format {
        let output = opts
            .format_out
            .as_ref()
            .ok_or(CliError::Usage("\\dump requires --format-out <path>"))?;
        let format = stores.dump_format()?;
        stores.world_mut().write_file(output, format)?;
    }
    if opts.show_fixtures {
        print!("{}", run.terminal_text);
        return Ok(());
    }
    let effect_pos = stores.world().effect_pos();
    stores.commit_effects(effect_pos)?;
    Ok(())
}

struct RunCliOptions {
    input: PathBuf,
    show_fixtures: bool,
    dvi: Option<PathBuf>,
    format: Option<PathBuf>,
    format_out: Option<PathBuf>,
}

impl RunCliOptions {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, CliError> {
        let mut input = None;
        let mut show_fixtures = false;
        let mut dvi = None;
        let mut format = None;
        let mut format_out = None;
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--show-fixtures" => {
                    show_fixtures = true;
                }
                "--dvi" => {
                    if dvi.is_some() {
                        return Err(CliError::Usage("run accepts at most one --dvi output path"));
                    }
                    let Some(path) = args.next() else {
                        return Err(CliError::Usage("missing output path for --dvi"));
                    };
                    dvi = Some(PathBuf::from(path));
                }
                "--format" => {
                    if format.is_some() {
                        return Err(CliError::Usage("run accepts at most one --format input"));
                    }
                    let Some(path) = args.next() else {
                        return Err(CliError::Usage("missing input path for --format"));
                    };
                    format = Some(PathBuf::from(path));
                }
                "--format-out" => {
                    if format_out.is_some() {
                        return Err(CliError::Usage("run accepts at most one --format-out path"));
                    }
                    let Some(path) = args.next() else {
                        return Err(CliError::Usage("missing output path for --format-out"));
                    };
                    format_out = Some(PathBuf::from(path));
                }
                flag if flag.starts_with('-') => {
                    return Err(CliError::Usage(
                        "run accepts one input path with optional --show-fixtures and --dvi <path>",
                    ));
                }
                path => {
                    if input.is_some() {
                        return Err(CliError::Usage(
                            "run accepts one input path with optional --show-fixtures and --dvi <path>",
                        ));
                    }
                    input = Some(PathBuf::from(path));
                }
            }
        }
        let input = input.ok_or(CliError::Usage("missing input path for run"))?;
        Ok(Self {
            input,
            show_fixtures,
            dvi,
            format,
            format_out,
        })
    }
}

struct RunHooks {
    input_search: TexInputSearchPath,
    font_search: TexFontSearchPath,
    job_name: String,
}

impl RunHooks {
    fn new(path: &Path, tex_input_areas: Vec<PathBuf>, tex_font_areas: Vec<PathBuf>) -> Self {
        let base_dir = path.parent().unwrap_or_else(|| Path::new(".")).to_owned();
        let job_name = path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("texput")
            .to_owned();
        Self {
            input_search: TexInputSearchPath::new(&base_dir, tex_input_areas),
            font_search: TexFontSearchPath::new(base_dir, tex_font_areas),
            job_name,
        }
    }
}

impl ExpansionHooks<WorldInput> for RunHooks {
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        input: &mut C,
        name: &str,
    ) -> Result<WorldInput, String> {
        self.input_search
            .read(input, name)
            .map(WorldInput::from_content)
    }

    fn open_font<C: tex_state::InputReadState>(
        &mut self,
        input: &mut C,
        path: &Path,
    ) -> Result<tex_state::FileContent, String> {
        self.font_search.read(input, path)
    }

    fn job_name(&self) -> &str {
        &self.job_name
    }
}

fn format_token(token: Token, stores: &Universe) -> String {
    match token {
        Token::Char { ch, cat } => format!("char:{}:{}", ch as u32, cat as u8),
        Token::Cs(symbol) => format!("cs:{}", stores.resolve(symbol)),
        Token::Param(slot) => format!("param:{slot}"),
        token if token.is_frozen_end_template() => "frozen:endtemplate".to_owned(),
        token if token.is_frozen_endv() => "frozen:endv".to_owned(),
        Token::Frozen(_) => unreachable!("invalid frozen token payload"),
    }
}

#[derive(Debug)]
enum CliError {
    Usage(&'static str),
    World(WorldError),
    Lex(tex_lex::LexError),
    ExpandDump(expand_dump::ExpandDumpError),
    Exec(tex_exec::ExecError),
    RenderedExec(String),
    Dvi(umber::DviBuildError),
    Format(FormatError),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::World(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::ExpandDump(err) => write!(f, "{err}"),
            Self::Exec(err) => write!(f, "{err}"),
            Self::RenderedExec(text) => f.write_str(text),
            Self::Dvi(err) => write!(f, "{err}"),
            Self::Format(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<WorldError> for CliError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<tex_lex::LexError> for CliError {
    fn from(value: tex_lex::LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<tex_exec::ExecError> for CliError {
    fn from(value: tex_exec::ExecError) -> Self {
        Self::Exec(value)
    }
}

impl From<umber::DviBuildError> for CliError {
    fn from(value: umber::DviBuildError) -> Self {
        Self::Dvi(value)
    }
}

impl From<FormatError> for CliError {
    fn from(value: FormatError) -> Self {
        Self::Format(value)
    }
}
