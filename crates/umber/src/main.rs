use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tex_lex::{Lexer, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::token::Token;
use tex_state::{FormatError, Universe, World, WorldError};
use umber::EngineMode as RunEngine;
use umber::{DriverFile, PlannedFinalization};

mod bib;
mod classic_bib;
mod expand_dump;
mod format_cache_cli;
mod watch;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("umber: {err}");
            ExitCode::from(err.exit_status())
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
        Some("format-cache") => format_cache_cli::run(args).map_err(CliError::FormatCache),
        Some("run") => {
            let opts = RunCliOptions::parse(args)?;
            run_tex(&opts)
        }
        Some("bib") => bib::run(args).map_err(CliError::Bib),
        Some("bibtex") => classic_bib::run(args).map_err(CliError::Bib),
        Some("watch") => watch::run(args).map_err(CliError::Watch),
        None => {
            println!("umber {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(_) => Err(CliError::Usage(
            "expected: umber <lex-dump|expand-dump|format-cache|bib|bibtex|run|watch> <input>",
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

#[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
fn run_tex(opts: &RunCliOptions) -> Result<(), CliError> {
    let run_started = std::time::Instant::now();
    let accepted =
        umber::cli_resource::run_for_finalization(&umber::cli_resource::NativeRunOptions {
            input: opts.input.clone(),
            format: opts.format.clone(),
            initial_prefetch_keys: opts.initial_prefetch_keys.clone(),
            engine: opts.engine,
            html: false,
            distribution: opts.distribution.clone(),
            distribution_sha256: opts.distribution_sha256.clone(),
            offline: opts.offline,
            expansion_fuel: opts.expansion_fuel,
        })?;
    if env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1") {
        eprintln!(
            "RESOURCE_ENGINE_ACCEPTED accepted_wall_ns={}",
            run_started.elapsed().as_nanos()
        );
    }
    finalize_run(opts, accepted, run_started)
}

#[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
fn finalize_run(
    opts: &RunCliOptions,
    mut accepted: umber::cli_resource::NativeAcceptedRun,
    run_started: std::time::Instant,
) -> Result<(), CliError> {
    let font_resources_started = std::time::Instant::now();
    if opts.pdf.is_some() && !accepted.pdf_draft_mode() {
        accepted.provide_pdf_font_programs()?;
    }
    let font_resources_ns = font_resources_started.elapsed().as_nanos();
    let (output, finalization, input_path_map, resolved_inputs, main_input, telemetry) =
        accepted.into_parts();
    if env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1") {
        eprintln!(
            "RESOURCE_TELEMETRY cold_starts={} suspensions={} local_step_retries={} replayed_delivered_tokens={} replayed_dispatches={} cumulative_fuel={} resource_wait_ns={} engine_ns={}",
            telemetry.execution.cold_starts,
            telemetry.execution.suspensions,
            telemetry.execution.local_step_retries,
            telemetry.execution.replayed_delivered_tokens,
            telemetry.execution.replayed_dispatches,
            telemetry.execution.cumulative_fuel,
            telemetry.resource_wait_time.as_nanos(),
            telemetry.execution.engine_time.as_nanos(),
        );
    }
    let virtual_font_resources = finalization.virtual_font_resources;
    let mut stores = finalization.stores;
    let dumped_format = finalization.dumped_format;
    #[cfg_attr(not(feature = "profiling-stats"), allow(unused_variables))]
    let expansion_stats = finalization.expansion_stats;
    let committed_artifacts = stores.world().committed_artifacts().to_vec();
    if opts.format_out.is_some() && !dumped_format {
        return Err(CliError::MissingFormatDump);
    }
    #[cfg(feature = "profiling-stats")]
    if opts.profiling_stats {
        let stats = expansion_stats;
        eprintln!(
            "EXPANSION_STATS token_frame_steps={} provenance_resolutions={} character_tokens={} character_fraction={:.6} meaning_lookups={} meaning_cache_hits={} meaning_cache_misses={} literal_spans={} literal_tokens={} mean_literal_run={:.6} segmentation_cache_hits={} segmentation_cache_misses={} builder_appends={} source_text_span_attempts={} source_text_spans={} source_text_tokens={} mean_source_text_run={:.6}",
            stats.token_frame_steps,
            stats.provenance_resolutions,
            stats.character_tokens,
            stats.character_fraction(),
            stats.meaning_lookups,
            stats.meaning_cache_hits,
            stats.meaning_cache_misses,
            stats.literal_spans,
            stats.literal_tokens,
            stats.mean_literal_run(),
            stats.segmentation_cache_hits,
            stats.segmentation_cache_misses,
            stats.builder_appends,
            stats.source_text_span_attempts,
            stats.source_text_spans,
            stats.source_text_tokens,
            stats.mean_source_text_run(),
        );
        eprintln!(
            "EXPANSION_TIMERS_NS frame_step={} frame_step_samples={} provenance={} provenance_samples={} classification_meaning={} classification_meaning_samples={} builder_append={} builder_append_samples={} attributed_total={}",
            stats.frame_step_nanos,
            stats.frame_step_timer_samples,
            stats.provenance_nanos,
            stats.provenance_timer_samples,
            stats.classification_meaning_nanos,
            stats.classification_meaning_timer_samples,
            stats.builder_append_nanos,
            stats.builder_append_timer_samples,
            stats.attributed_nanos(),
        );
    }
    #[cfg(feature = "profiling-stats")]
    if opts.profiling_stats {
        for (kind, count) in tex_state::node::node_append_histogram() {
            eprintln!("NODE_HISTOGRAM {kind} {count}");
        }
    }
    #[cfg(feature = "profiling-stats")]
    if opts.profiling_stats {
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
            "NODE_SURVIVOR fresh_calls={} fresh_nanos={} recycled_calls={} recycled_nanos={} release_calls={} release_nanos={} shared_payload_drops={} shared_payload_drop_nanos={} peak_scratch_logical_bytes={} peak_scratch_retained_bytes={} source_words={} child_bearing_nodes={} remap_entries={} pending_entries={} peak_remap_entries={} peak_pending_entries={}",
            survivor.fresh_promotions,
            survivor.fresh_promotion_nanos,
            survivor.recycled_promotions,
            survivor.recycled_promotion_nanos,
            survivor.releases_to_recycling,
            survivor.release_nanos,
            survivor.shared_payload_drops,
            survivor.shared_payload_drop_nanos,
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
    if let Some(path) = &opts.dvi {
        driver_files.push(DriverFile::new(path.clone(), output.dvi.clone()));
    }
    if let Some(output) = &opts.pdf {
        if stores
            .fixed_pdf_output_parameters()
            .is_some_and(|parameters| parameters.draft_mode > 0)
        {
            eprintln!("pdfTeX warning: \\pdfdraftmode enabled, not changing output pdf");
        } else {
            let pdf_started = std::time::Instant::now();
            let pdf = umber::pdf_from_committed_artifacts_with_virtual_fonts(
                &mut stores,
                &committed_artifacts,
                &virtual_font_resources,
            )?;
            if env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1") {
                eprintln!(
                    "PDF_DRIVER_BUILD pdf_build_ns={}",
                    pdf_started.elapsed().as_nanos()
                );
            }
            driver_files.push(DriverFile::new(output.clone(), pdf));
        }
    }
    if let Some(output) = &opts.html {
        let font_dir = opts
            .html_font_dir
            .as_ref()
            .ok_or(CliError::Usage("--html requires --html-font-dir <path>"))?;
        let mut resolver = umber::DirectoryHtmlFontResolver::new(font_dir, stores.world_mut());
        let mut html_options = tex_out::html::HtmlOptions::default();
        if let Some(asset_dir) = &opts.html_assets {
            let relative_directory = asset_dir
                .to_str()
                .ok_or(CliError::Usage("--html-assets must be valid UTF-8"))?
                .to_owned();
            html_options.asset_mode = tex_out::html::AssetMode::Manifest { relative_directory };
        }
        let html = umber::html_from_committed_artifacts(
            &committed_artifacts,
            &mut resolver,
            &html_options,
        )?;
        if let Some(asset_dir) = &opts.html_assets {
            let base = output.parent().unwrap_or_else(|| std::path::Path::new("."));
            for asset in html.assets {
                driver_files.push(DriverFile::new(
                    base.join(asset_dir).join(asset.path),
                    asset.bytes,
                ));
            }
        }
        driver_files.push(DriverFile::new(output.clone(), html.html));
    }
    if dumped_format {
        let output = opts
            .format_out
            .as_ref()
            .ok_or(CliError::Usage("\\dump requires --format-out <path>"))?;
        let format = stores.dump_format()?;
        driver_files.push(DriverFile::new(output.clone(), format));
    }
    if let Some(output) = &opts.input_records_out {
        driver_files.push(DriverFile::new(
            output.clone(),
            input_record_receipt(
                stores.world(),
                &input_path_map,
                &resolved_inputs,
                Some(main_input),
            )?,
        ));
    }
    let effect_pos = stores.world().effect_pos();
    let materialize_started = std::time::Instant::now();
    let finalization = PlannedFinalization::new(effect_pos, driver_files)?;
    if opts.show_fixtures {
        print!("{}", String::from_utf8_lossy(&output.terminal));
        finalization.discard_uncommitted();
        return Ok(());
    }
    finalization
        .commit_effects(&mut stores)?
        .materialize(&mut stores)?;
    if env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1") {
        eprintln!(
            "PDF_DRIVER_TELEMETRY font_resources_ns={} materialize_ns={} run_wall_ns={}",
            font_resources_ns,
            materialize_started.elapsed().as_nanos(),
            run_started.elapsed().as_nanos()
        );
    }
    Ok(())
}

struct RunCliOptions {
    input: PathBuf,
    show_fixtures: bool,
    dvi: Option<PathBuf>,
    pdf: Option<PathBuf>,
    html: Option<PathBuf>,
    html_font_dir: Option<PathBuf>,
    html_assets: Option<PathBuf>,
    format: Option<PathBuf>,
    format_out: Option<PathBuf>,
    input_records_out: Option<PathBuf>,
    initial_prefetch_keys: Vec<String>,
    engine: RunEngine,
    distribution: Option<String>,
    distribution_sha256: Option<String>,
    offline: bool,
    expansion_fuel: Option<u64>,
    #[cfg(feature = "profiling-stats")]
    profiling_stats: bool,
}

impl RunCliOptions {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, CliError> {
        let mut input = None;
        let mut show_fixtures = false;
        let mut dvi = None;
        let mut pdf = None;
        let mut html = None;
        let mut html_font_dir = None;
        let mut html_assets = None;
        let mut format = None;
        let mut format_out = None;
        let mut input_records_out = None;
        let mut initial_prefetch_keys = Vec::new();
        let mut engine = RunEngine::Tex82;
        let mut distribution = None;
        let mut distribution_sha256 = None;
        let mut offline = env::var_os("UMBER_OFFLINE").is_some_and(|value| value == "1");
        let mut expansion_fuel = None;
        #[cfg(feature = "profiling-stats")]
        let mut profiling_stats = false;
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--show-fixtures" => {
                    show_fixtures = true;
                }
                "--offline" => offline = true,
                "--expansion-fuel" => {
                    if expansion_fuel.is_some() {
                        return Err(CliError::Usage("run accepts at most one --expansion-fuel"));
                    }
                    let value = args.next().ok_or(CliError::Usage(
                        "missing positive integer for --expansion-fuel",
                    ))?;
                    expansion_fuel =
                        Some(value.parse::<u64>().ok().filter(|value| *value > 0).ok_or(
                            CliError::Usage("--expansion-fuel must be a positive integer"),
                        )?);
                }
                "--distribution" => {
                    if distribution.is_some() {
                        return Err(CliError::Usage("run accepts at most one --distribution"));
                    }
                    distribution = Some(
                        args.next()
                            .ok_or(CliError::Usage("missing URL or path for --distribution"))?,
                    );
                }
                "--distribution-sha256" => {
                    if distribution_sha256.is_some() {
                        return Err(CliError::Usage(
                            "run accepts at most one --distribution-sha256",
                        ));
                    }
                    distribution_sha256 = Some(
                        args.next()
                            .ok_or(CliError::Usage("missing digest for --distribution-sha256"))?,
                    );
                }
                "--etex" => {
                    if engine != RunEngine::Tex82 {
                        return Err(CliError::Usage("run accepts only one engine mode flag"));
                    }
                    engine = RunEngine::ETex;
                }
                "--pdftex" => {
                    if engine != RunEngine::Tex82 {
                        return Err(CliError::Usage("run accepts only one engine mode flag"));
                    }
                    engine = RunEngine::PdfTex;
                }
                "--latex" => {
                    if engine != RunEngine::Tex82 {
                        return Err(CliError::Usage("run accepts only one engine mode flag"));
                    }
                    engine = RunEngine::Latex;
                }
                "--pdflatex" => {
                    if engine != RunEngine::Tex82 {
                        return Err(CliError::Usage("run accepts only one engine mode flag"));
                    }
                    engine = RunEngine::PdfLatex;
                }
                #[cfg(feature = "profiling-stats")]
                "--profiling-stats" => {
                    profiling_stats = true;
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
                "--pdf" => {
                    if pdf.is_some() {
                        return Err(CliError::Usage("run accepts at most one --pdf output path"));
                    }
                    let Some(path) = args.next() else {
                        return Err(CliError::Usage("missing output path for --pdf"));
                    };
                    pdf = Some(PathBuf::from(path));
                }
                "--html" => {
                    if html.is_some() {
                        return Err(CliError::Usage(
                            "run accepts at most one --html output path",
                        ));
                    }
                    let Some(path) = args.next() else {
                        return Err(CliError::Usage("missing output path for --html"));
                    };
                    html = Some(PathBuf::from(path));
                }
                "--html-font-dir" => {
                    if html_font_dir.is_some() {
                        return Err(CliError::Usage("run accepts at most one --html-font-dir"));
                    }
                    let Some(path) = args.next() else {
                        return Err(CliError::Usage("missing path for --html-font-dir"));
                    };
                    html_font_dir = Some(PathBuf::from(path));
                }
                "--html-assets" => {
                    if html_assets.is_some() {
                        return Err(CliError::Usage(
                            "run accepts at most one --html-assets directory",
                        ));
                    }
                    let Some(path) = args.next() else {
                        return Err(CliError::Usage("missing directory for --html-assets"));
                    };
                    html_assets = Some(PathBuf::from(path));
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
                "--input-records-out" => {
                    if input_records_out.is_some() {
                        return Err(CliError::Usage(
                            "run accepts at most one --input-records-out path",
                        ));
                    }
                    let Some(path) = args.next() else {
                        return Err(CliError::Usage(
                            "missing output path for --input-records-out",
                        ));
                    };
                    input_records_out = Some(PathBuf::from(path));
                }
                "--prefetch-input" => initial_prefetch_keys.push(args.next().ok_or(
                    CliError::Usage("missing distribution request key for --prefetch-input"),
                )?),
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
        if distribution_sha256.is_none() {
            distribution_sha256 = env::var("UMBER_DISTRIBUTION_SHA256").ok();
        }
        if pdf.is_some() && !engine.supports_pdf_output() {
            return Err(CliError::Usage("--pdf requires --pdftex or --pdflatex"));
        }
        if dvi
            .as_ref()
            .zip(format_out.as_ref())
            .is_some_and(|(dvi_path, format_path)| dvi_path == format_path)
        {
            return Err(CliError::Usage(
                "--dvi and --format-out must use different output paths",
            ));
        }
        if html_assets.is_some() && html.is_none() {
            return Err(CliError::Usage("--html-assets requires --html"));
        }
        if html_font_dir.is_some() && html.is_none() {
            return Err(CliError::Usage("--html-font-dir requires --html"));
        }
        if dvi
            .as_ref()
            .zip(html.as_ref())
            .is_some_and(|(dvi, html)| dvi == html)
        {
            return Err(CliError::Usage(
                "--dvi and --html must use different output paths",
            ));
        }
        if [&dvi, &html, &format_out]
            .into_iter()
            .flatten()
            .any(|path| Some(path) == pdf.as_ref())
        {
            return Err(CliError::Usage(
                "--pdf must use a distinct downstream output path",
            ));
        }
        Ok(Self {
            input,
            show_fixtures,
            dvi,
            pdf,
            html,
            html_font_dir,
            html_assets,
            format,
            format_out,
            input_records_out,
            initial_prefetch_keys,
            engine,
            distribution,
            distribution_sha256,
            offline,
            expansion_fuel,
            #[cfg(feature = "profiling-stats")]
            profiling_stats,
        })
    }
}

fn input_record_receipt(
    world: &World,
    path_map: &BTreeMap<PathBuf, PathBuf>,
    resolved_inputs: &[(PathBuf, usize)],
    main_input: Option<(PathBuf, usize)>,
) -> Result<Vec<u8>, CliError> {
    let mut records = BTreeMap::<PathBuf, usize>::new();
    for (path, len) in resolved_inputs {
        insert_input_record(&mut records, path.clone(), *len)?;
    }
    for record in world.external_input_records() {
        let path = path_map
            .get(record.path())
            .cloned()
            .unwrap_or_else(|| record.path().to_owned());
        insert_input_record(&mut records, path, record.len())?;
    }
    if let Some((path, len)) = main_input {
        insert_input_record(&mut records, path, len)?;
    }

    let mut receipt = Vec::new();
    for (path, len) in records {
        let Some(path) = path.to_str() else {
            return Err(CliError::InputReceipt(
                "an input path is not valid UTF-8".to_owned(),
            ));
        };
        if path.contains(['\n', '\r', '\t']) {
            return Err(CliError::InputReceipt(format!(
                "an input path contains a receipt delimiter: {}",
                Path::new(path).display()
            )));
        }
        receipt.extend_from_slice(len.to_string().as_bytes());
        receipt.push(b'\t');
        receipt.extend_from_slice(path.as_bytes());
        receipt.push(b'\n');
    }
    Ok(receipt)
}

fn insert_input_record(
    records: &mut BTreeMap<PathBuf, usize>,
    path: PathBuf,
    len: usize,
) -> Result<(), CliError> {
    match records.entry(path) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(len);
        }
        std::collections::btree_map::Entry::Occupied(entry) => {
            if *entry.get() != len {
                return Err(CliError::InputReceipt(format!(
                    "input changed length while the job was running: {}",
                    entry.key().display()
                )));
            }
        }
    }
    Ok(())
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
    FormatCache(format_cache_cli::FormatCacheCliError),
    Exec(tex_exec::ExecError),
    Dvi(umber::DviBuildError),
    Html(umber::HtmlBuildError),
    Pdf(umber::PdfBuildError),
    Format(FormatError),
    MissingFormatDump,
    Finalization(umber::FinalizationError),
    InputReceipt(String),
    Bib(bib::BibCliError),
    Watch(watch::WatchError),
    NativeRun(umber::cli_resource::NativeRunError),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::World(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::ExpandDump(err) => write!(f, "{err}"),
            Self::FormatCache(err) => write!(f, "{err}"),
            Self::Exec(err) => write!(f, "{err}"),
            Self::Dvi(err) => write!(f, "{err}"),
            Self::Html(err) => write!(f, "{err}"),
            Self::Pdf(err) => write!(f, "{err}"),
            Self::Format(err) => write!(f, "{err}"),
            Self::MissingFormatDump => {
                f.write_str("--format-out requires the input to execute \\dump")
            }
            Self::Finalization(err) => write!(f, "{err}"),
            Self::InputReceipt(message) => f.write_str(message),
            Self::Bib(err) => write!(f, "{err}"),
            Self::Watch(err) => write!(f, "{err}"),
            Self::NativeRun(err) => write!(f, "{err}"),
        }
    }
}

impl CliError {
    const fn exit_status(&self) -> u8 {
        match self {
            Self::Bib(error) => error.exit_status(),
            _ => 1,
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

impl From<umber::HtmlBuildError> for CliError {
    fn from(value: umber::HtmlBuildError) -> Self {
        Self::Html(value)
    }
}

impl From<umber::PdfBuildError> for CliError {
    fn from(value: umber::PdfBuildError) -> Self {
        Self::Pdf(value)
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

impl From<umber::cli_resource::NativeRunError> for CliError {
    fn from(value: umber::cli_resource::NativeRunError) -> Self {
        Self::NativeRun(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::{InputOrigin, PrintSink, StreamSlot};

    #[test]
    fn input_receipt_deduplicates_external_reads_and_excludes_same_run_outputs() {
        let mut world = World::memory();
        world
            .set_memory_file("external.cfg", b"external".to_vec())
            .expect("seed external input");
        let first = world
            .read_file("external.cfg")
            .expect("first external read");
        let second = world
            .read_file("external.cfg")
            .expect("repeated external read");
        assert_eq!(first.origin(), InputOrigin::External);
        assert_eq!(second.origin(), InputOrigin::External);

        let slot = StreamSlot::new(1);
        world.open_out(slot, "generated.tmp");
        world.write_text(PrintSink::Stream(slot), "generated");
        world.close_out(slot);
        let generated = world
            .read_file("generated.tmp")
            .expect("same-run generated read");
        assert_eq!(generated.origin(), InputOrigin::SameRunGenerated);

        let receipt =
            input_record_receipt(&world, &BTreeMap::new(), &[], None).expect("build input receipt");
        assert_eq!(receipt, b"8\texternal.cfg\n");
    }

    #[test]
    fn input_receipt_rejects_unescaped_tsv_delimiters() {
        for path in ["tab\tname.tex", "line\nname.tex", "return\rname.tex"] {
            let error = input_record_receipt(
                &World::memory(),
                &BTreeMap::new(),
                &[(PathBuf::from(path), 1)],
                None,
            )
            .expect_err("receipt paths containing delimiters must be rejected");
            assert!(matches!(error, CliError::InputReceipt(_)));
        }
    }
}
