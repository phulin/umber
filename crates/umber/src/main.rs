use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use tex_lex::{InputStack, Lexer, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::token::Token;
use tex_state::{FormatError, Universe, World, WorldError};
use umber::{DriverFile, EngineSession, FileSessionHooks, PlannedFinalization};

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
        if opts.etex {
            umber::prepare_etex_run_stores(&mut stores);
        } else {
            umber::prepare_run_stores(&mut stores);
        }
        stores
    };
    if opts.etex && opts.format.is_some() {
        tex_exec::install_etex_unexpandable_primitives(&mut stores);
    }
    let content = stores.world_mut().read_file(path)?;

    let mut input = InputStack::new(WorldInput::from_content(content));
    let mut hooks = FileSessionHooks::from_environment(path);
    let run = match EngineSession::new(&mut input, &mut stores, &mut hooks).execute() {
        Ok(run) => run,
        Err(err) => {
            return Err(CliError::RenderedExec(
                err.format_with_provenance(&stores).trim_end().to_owned(),
            ));
        }
    };
    #[cfg(feature = "expansion-stats")]
    {
        let stats = input.expansion_stats();
        eprintln!(
            "EXPANSION_STATS token_frame_steps={} provenance_resolutions={} character_tokens={} character_fraction={:.6} meaning_lookups={} literal_spans={} literal_tokens={} mean_literal_run={:.6} segmentation_cache_hits={} segmentation_cache_misses={} builder_appends={}",
            stats.token_frame_steps,
            stats.provenance_resolutions,
            stats.character_tokens,
            stats.character_fraction(),
            stats.meaning_lookups,
            stats.literal_spans,
            stats.literal_tokens,
            stats.mean_literal_run(),
            stats.segmentation_cache_hits,
            stats.segmentation_cache_misses,
            stats.builder_appends,
        );
    }
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
        for (component, measurement) in hash.named_components() {
            eprintln!(
                "STATE_HASH_COMPONENT {component} calls={} visits={} nanos={}",
                measurement.calls, measurement.visits, measurement.nanos
            );
        }
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
    let mut driver_files = Vec::new();
    if let Some(output) = &opts.dvi {
        let dvi = umber::dvi_from_page_plans(&run.dvi_pages)?;
        driver_files.push(DriverFile::new(output.clone(), dvi));
    }
    if run.dumped_format {
        let output = opts
            .format_out
            .as_ref()
            .ok_or(CliError::Usage("\\dump requires --format-out <path>"))?;
        let format = stores.dump_format()?;
        driver_files.push(DriverFile::new(output.clone(), format));
    }
    let effect_pos = stores.world().effect_pos();
    let finalization = PlannedFinalization::new(effect_pos, driver_files)?;
    if opts.show_fixtures {
        print!("{}", run.terminal_text);
        finalization.discard_uncommitted();
        return Ok(());
    }
    finalization
        .commit_effects(&mut stores)?
        .materialize(&mut stores)?;
    Ok(())
}

struct RunCliOptions {
    input: PathBuf,
    show_fixtures: bool,
    dvi: Option<PathBuf>,
    format: Option<PathBuf>,
    format_out: Option<PathBuf>,
    etex: bool,
}

impl RunCliOptions {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, CliError> {
        let mut input = None;
        let mut show_fixtures = false;
        let mut dvi = None;
        let mut format = None;
        let mut format_out = None;
        let mut etex = false;
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--show-fixtures" => {
                    show_fixtures = true;
                }
                "--etex" => {
                    etex = true;
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
        if dvi
            .as_ref()
            .zip(format_out.as_ref())
            .is_some_and(|(dvi_path, format_path)| dvi_path == format_path)
        {
            return Err(CliError::Usage(
                "--dvi and --format-out must use different output paths",
            ));
        }
        Ok(Self {
            input,
            show_fixtures,
            dvi,
            format,
            format_out,
            etex,
        })
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
    Finalization(umber::FinalizationError),
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
            Self::Finalization(err) => write!(f, "{err}"),
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

impl From<umber::FinalizationError> for CliError {
    fn from(value: umber::FinalizationError) -> Self {
        Self::Finalization(value)
    }
}
