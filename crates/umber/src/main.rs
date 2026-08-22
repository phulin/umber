use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest, Sha256};
use tex_command::{
    CatcodeQueries, CharacterCode, CommandDialect, CommandProfile, CommandState,
    SourceControlSequenceKind, SourceRegistration, SourceToken, SourceTokenizationStep,
};
use tex_state::env::banks::IntParam;
use tex_state::token::Token;
use tex_state::{FileContent, FormatError, Universe, World, WorldError};
use umber::EngineMode as RunEngine;
use umber::{DriverFile, PlannedFinalization};

#[cfg(feature = "profiling")]
#[global_allocator]
static HOT_CORE_ALLOCATOR: tex_state::measurement::HotCoreAllocator =
    tex_state::measurement::HotCoreAllocator;

mod bib;
mod classic_bib;
mod expand_dump;
mod format_cache_cli;
mod watch;

fn main() -> ExitCode {
    if let Some(worker) = umber::dispatch_format_worker() {
        return match worker {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("umber format worker: {error}");
                ExitCode::from(70)
            }
        };
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if env::var_os("UMBER_CAUSAL_DIAGNOSTIC").is_some_and(|value| value == "1")
                && let Some(diagnostic) = err.causal_diagnostic()
            {
                eprintln!("{}", causal_diagnostic_line(diagnostic));
            }
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
    umber::with_engine_world(World::real(), |stores| {
        let content = stores.world_mut().read_file(path)?;
        lex_dump_generation(stores, content)
    })
    .map_err(|error| CliError::Lex(format!("{error:?}")))?
}

fn lex_dump_generation<G>(stores: &mut Universe<G>, content: FileContent) -> Result<(), CliError> {
    // `lex-dump` reports what a format-loaded engine would tokenize, matching
    // `umber run`; the run-store preparation supplies the plain category codes
    // that INITEX itself deliberately leaves as `other`.
    umber::prepare_run_stores(stores);
    let mut command =
        CommandState::<G>::new(CommandProfile::unicode_extended(CommandDialect::Tex82));
    let source = command
        .register_source(SourceRegistration::world(content))
        .map_err(|error| CliError::Lex(error.to_string()))?;
    command
        .open_registered_source(source)
        .map_err(|error| CliError::Lex(error.to_string()))?;
    loop {
        let step = command.next_unicode_source_step(
            stores.int_param(IntParam::END_LINE_CHAR),
            &mut CatcodeQueries(|code: CharacterCode| {
                stores.catcode(code.to_char().expect("Unicode command profile"))
            }),
        );
        match step {
            SourceTokenizationStep::Token(token) => {
                println!("{}", format_source_token(&token));
            }
            SourceTokenizationStep::InvalidCharacter(invalid) => {
                return Err(CliError::Lex(format!(
                    "invalid character {}",
                    invalid.code().to_char().expect("Unicode command profile") as u32
                )));
            }
            SourceTokenizationStep::End => break,
        }
    }
    Ok(())
}

#[cfg(feature = "profiling")]
struct MainMemoryProjectionReport {
    enabled: bool,
    before: tex_state::measurement::MainMemoryProjectionMeasurement,
    provenance_before: tex_state::measurement::ProvenanceLifecycleMeasurement,
    format_restore_before: tex_state::measurement::FormatRestoreMeasurement,
    hot_core_before: tex_state::measurement::HotCoreCensus,
}

#[cfg(feature = "profiling")]
impl MainMemoryProjectionReport {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            before: tex_state::measurement::main_memory_projection_measurement(),
            provenance_before: tex_state::measurement::provenance_lifecycle_measurement(),
            format_restore_before: tex_state::measurement::format_restore_measurement(),
            hot_core_before: tex_state::measurement::hot_core_census(),
        }
    }
}

#[cfg(feature = "profiling")]
impl Drop for MainMemoryProjectionReport {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        let after = tex_state::measurement::main_memory_projection_measurement();
        let before = self.before;
        let delta = tex_state::measurement::MainMemoryProjectionMeasurement {
            dynamic_observations: after
                .dynamic_observations
                .saturating_sub(before.dynamic_observations),
            base_requests: after.base_requests.saturating_sub(before.base_requests),
            base_reuses: after.base_reuses.saturating_sub(before.base_reuses),
            full_rebuilds: after.full_rebuilds.saturating_sub(before.full_rebuilds),
            operation_boundaries: after
                .operation_boundaries
                .saturating_sub(before.operation_boundaries),
            operation_boundaries_retained: after
                .operation_boundaries_retained
                .saturating_sub(before.operation_boundaries_retained),
            cell_root_updates: after
                .cell_root_updates
                .saturating_sub(before.cell_root_updates),
            cell_root_updates_retained: after
                .cell_root_updates_retained
                .saturating_sub(before.cell_root_updates_retained),
            box_root_updates: after
                .box_root_updates
                .saturating_sub(before.box_root_updates),
            box_root_updates_retained: after
                .box_root_updates_retained
                .saturating_sub(before.box_root_updates_retained),
            cache_losses: core::array::from_fn(|index| {
                after.cache_losses[index].saturating_sub(before.cache_losses[index])
            }),
        };
        eprintln!(
            "MAIN_MEMORY_PROJECTION dynamic_observations={} base_requests={} base_reuses={} full_rebuilds={} operation_boundaries={} operation_boundaries_retained={} cell_root_updates={} cell_root_updates_retained={} box_root_updates={} box_root_updates_retained={}",
            delta.dynamic_observations,
            delta.base_requests,
            delta.base_reuses,
            delta.full_rebuilds,
            delta.operation_boundaries,
            delta.operation_boundaries_retained,
            delta.cell_root_updates,
            delta.cell_root_updates_retained,
            delta.box_root_updates,
            delta.box_root_updates_retained,
        );
        for (owner, count) in delta.named_cache_losses() {
            eprintln!("MAIN_MEMORY_PROJECTION_CACHE_LOSS owner={owner} count={count}");
        }
        let provenance = tex_state::measurement::provenance_lifecycle_measurement()
            .saturating_sub(self.provenance_before);
        eprintln!(
            "PROVENANCE_LIFECYCLE atom_intern_calls={} atom_hits={} atom_misses={} atom_allocations={} frame_intern_calls={} frame_hits={} frame_misses={} frame_allocations={} list_intern_calls={} list_hits={} list_misses={} list_allocations={} atom_retains={} atom_releases={} frame_retains={} frame_releases={} origin_resolutions={} list_resolutions={} list_resolution_comparisons={}",
            provenance.atom_intern_calls,
            provenance.atom_intern_hits,
            provenance.atom_intern_misses,
            provenance.atom_allocations,
            provenance.frame_intern_calls,
            provenance.frame_intern_hits,
            provenance.frame_intern_misses,
            provenance.frame_allocations,
            provenance.list_intern_calls,
            provenance.list_intern_hits,
            provenance.list_intern_misses,
            provenance.list_allocations,
            provenance.atom_retains,
            provenance.atom_releases,
            provenance.frame_retains,
            provenance.frame_releases,
            provenance.origin_resolutions,
            provenance.list_resolutions,
            provenance.list_resolution_comparisons,
        );
        let format_restore = tex_state::measurement::format_restore_measurement()
            .saturating_sub(self.format_restore_before);
        eprintln!(
            "FORMAT_RESTORE calls={} bytes_decoded={} token_entries={} macro_entries={} glue_entries={} node_entries={} validation_passes={} copies={} allocations={}",
            format_restore.calls,
            format_restore.bytes_decoded,
            format_restore.token_entries_restored,
            format_restore.macro_entries_restored,
            format_restore.glue_entries_restored,
            format_restore.node_entries_restored,
            format_restore.validation_passes,
            format_restore.copies,
            format_restore.allocations,
        );
        let hot_core =
            tex_state::measurement::hot_core_census().saturating_sub(self.hot_core_before);
        eprintln!("HOT_CORE_CENSUS {}", hot_core_census_json(&hot_core));
    }
}

#[cfg(feature = "profiling")]
fn hot_core_census_json(census: &tex_state::measurement::HotCoreCensus) -> String {
    use std::fmt::Write as _;

    fn separator(output: &mut String, first: &mut bool) {
        if !std::mem::replace(first, false) {
            output.push(',');
        }
    }

    let mut output = String::from("{\"schema\":2,\"allocations\":{");
    let mut first = true;
    for (name, measurement) in tex_state::measurement::HotCoreAllocationOwner::NAMES
        .into_iter()
        .zip(census.allocations)
    {
        separator(&mut output, &mut first);
        write!(
            output,
            "\"{name}\":{{\"calls\":{},\"requested_bytes\":{}}}",
            measurement.calls, measurement.requested_bytes
        )
        .expect("writing to a String cannot fail");
    }
    output.push_str("},\"episode_lengths\":{");
    first = true;
    for (length, count) in census.episode_lengths.iter().copied().enumerate() {
        if count == 0 {
            continue;
        }
        separator(&mut output, &mut first);
        write!(output, "\"{length}\":{count}").expect("writing to a String cannot fail");
    }
    output.push_str("},\"stop_reasons\":{");
    first = true;
    for (name, count) in tex_state::measurement::HotCoreStopReason::NAMES
        .into_iter()
        .zip(census.stop_reasons)
    {
        separator(&mut output, &mut first);
        write!(output, "\"{name}\":{count}").expect("writing to a String cannot fail");
    }
    write!(
        output,
        "}},\"clones\":{{\"command_state\":{{\"calls\":{},\"nanos\":{},\"logical_bytes\":{}}},\"step_snapshot\":{{\"calls\":{},\"nanos\":{},\"logical_bytes\":{}}}}},",
        census.command_state_clones.calls,
        census.command_state_clones.nanos,
        census.command_state_clones.logical_bytes,
        census.step_snapshot_clones.calls,
        census.step_snapshot_clones.nanos,
        census.step_snapshot_clones.logical_bytes,
    )
    .expect("writing to a String cannot fail");
    write!(
        output,
        "\"weak_graph\":{{\"arc_retains\":{},\"weak_retains\":{},\"upgrade_calls\":{},\"upgrade_hits\":{}}},\"weak_index\":{{\"calls\":{},\"candidate_entries\":{},\"exact_comparisons\":{},\"content_hash_calls\":{}}},\"provenance_materialization\":{{\"calls\":{},\"hits\":{}}},",
        census.weak_graph.arc_retains,
        census.weak_graph.weak_retains,
        census.weak_graph.weak_upgrade_calls,
        census.weak_graph.weak_upgrade_hits,
        census.weak_index.calls,
        census.weak_index.candidate_entries,
        census.weak_index.exact_comparisons,
        census.weak_index.content_hash_calls,
        census.provenance_materialization_calls,
        census.provenance_materialization_hits,
    )
    .expect("writing to a String cannot fail");
    output.push_str("\"command_families\":{");
    first = true;
    for (name, count) in tex_state::measurement::HotCoreCommandFamily::NAMES
        .into_iter()
        .zip(census.command_families)
    {
        separator(&mut output, &mut first);
        write!(output, "\"{name}\":{count}").expect("writing to a String cannot fail");
    }
    write!(
        output,
        "}},\"expansion_opcodes\":{{\"macro\":{},\"primitives\":{{",
        census.macro_expansions,
    )
    .expect("writing to a String cannot fail");
    first = true;
    for (operand, count) in census.expandable_opcodes.into_iter().enumerate() {
        if count == 0 {
            continue;
        }
        let primitive = tex_state::meaning::ExpandablePrimitive::from_operand(operand as u64)
            .expect("census operand names an expandable primitive");
        separator(&mut output, &mut first);
        write!(output, "\"{operand}:{primitive:?}\":{count}")
            .expect("writing to a String cannot fail");
    }
    output.push_str("}},\"dispatch_opcodes\":{\"unexpandable_primitives\":{");
    first = true;
    for (operand, count) in census.unexpandable_opcodes.into_iter().enumerate() {
        if count == 0 {
            continue;
        }
        let primitive = tex_state::meaning::UnexpandablePrimitive::from_operand(operand as u64)
            .expect("census operand names an unexpandable primitive");
        separator(&mut output, &mut first);
        write!(output, "\"{operand}:{primitive:?}\":{count}")
            .expect("writing to a String cannot fail");
    }
    output.push_str("}},\"materializations\":{");
    first = true;
    for (name, count) in tex_state::measurement::HotCoreMaterialization::NAMES
        .into_iter()
        .zip(census.materializations)
    {
        separator(&mut output, &mut first);
        write!(output, "\"{name}\":{count}").expect("writing to a String cannot fail");
    }
    write!(
        output,
        "}},\"interpreter\":{{\"constructions\":{},\"operation_entries\":{}}},\"phase_boundaries\":{{",
        census.interpreter_constructions,
        census.interpreter_operation_entries,
    )
    .expect("writing to a String cannot fail");
    first = true;
    for (name, count) in tex_state::measurement::HotCorePhase::NAMES
        .into_iter()
        .zip(census.phase_boundaries)
    {
        separator(&mut output, &mut first);
        write!(output, "\"{name}\":{count}").expect("writing to a String cannot fail");
    }
    output.push_str("}}");
    output
}

#[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
fn run_tex(opts: &RunCliOptions) -> Result<(), CliError> {
    #[cfg(feature = "profiling")]
    let _main_memory_projection_report = MainMemoryProjectionReport::new(opts.profiling_stats);
    let run_started = std::time::Instant::now();
    let mut outputs = if opts.dvi.is_some() {
        umber::OutputCapabilitySet::DVI
    } else if opts.pdf.is_some() {
        umber::OutputCapabilitySet::PDF
    } else {
        // Legacy CLI runs without a publication path retain the classic DVI
        // driver as their compatibility output.
        umber::OutputCapabilitySet::DVI
    };
    if opts.pdf.is_some() {
        outputs = outputs.with(umber::OutputCapability::Pdf);
    }
    if opts.html.is_some() {
        outputs = outputs.with(umber::OutputCapability::Html);
    }
    let accepted =
        umber::cli_resource::run_for_finalization(&umber::cli_resource::NativeRunOptions {
            input: opts.input.clone(),
            format: opts.format.clone(),
            initial_prefetch_keys: opts.initial_prefetch_keys.clone(),
            engine: opts.engine,
            outputs,
            html_asset_directory: opts
                .html_assets
                .as_ref()
                .map(|path| {
                    path.to_str()
                        .map(str::to_owned)
                        .ok_or(CliError::Usage("--html-assets must be valid UTF-8"))
                })
                .transpose()?,
            distribution: opts.distribution.clone(),
            distribution_sha256: opts.distribution_sha256.clone(),
            offline: opts.offline,
            expansion_fuel: opts.expansion_fuel,
        })?;
    if let Some(path) = &opts.pdf_font_closure_out {
        accepted.write_pdf_font_closure_receipt(path)?;
    }
    if env::var_os("UMBER_RESOURCE_TELEMETRY").is_some_and(|value| value == "1") {
        eprintln!(
            "RESOURCE_ENGINE_ACCEPTED accepted_wall_ns={}",
            run_started.elapsed().as_nanos()
        );
    }
    let accepted_wall = run_started.elapsed();
    finalize_run(opts, accepted, run_started, accepted_wall)
}

#[allow(clippy::disallowed_methods)] // Process telemetry; TeX state never observes it.
fn finalize_run(
    opts: &RunCliOptions,
    accepted: umber::cli_resource::NativeAcceptedRun,
    run_started: std::time::Instant,
    accepted_wall: std::time::Duration,
) -> Result<(), CliError> {
    let font_resources_ns = 0;
    let (output, finalization, _input_path_map, resolved_inputs, main_input, telemetry, host) =
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
        let engine_core = telemetry
            .execution
            .engine_time
            .saturating_sub(telemetry.execution.savepoint_capture_time)
            .saturating_sub(telemetry.execution.savepoint_restore_time);
        let engine_entry_exit = host
            .compile_attempt_time
            .saturating_sub(telemetry.execution.engine_time)
            .saturating_sub(telemetry.request_extraction_time)
            .saturating_sub(telemetry.candidate_restore_time)
            .saturating_sub(telemetry.resolver_index_time)
            .saturating_sub(telemetry.vfs_stage_time);
        let cli_overhead = accepted_wall
            .saturating_sub(host.startup_time)
            .saturating_sub(host.compile_attempt_time)
            .saturating_sub(host.resolver_time)
            .saturating_sub(host.preload_time)
            .saturating_sub(host.provision_time)
            .saturating_sub(host.accepted_handoff_time);
        let resolver_accounted = host
            .resolver
            .local_lookup_time
            .saturating_add(host.resolver.manifest_lookup_time)
            .saturating_add(host.resolver.object_load_time)
            .saturating_add(host.resolver.content_hash_time)
            .saturating_add(host.resolver.response_build_time);
        eprintln!(
            "RESOURCE_HOST_TELEMETRY startup_ns={} engine_core_ns={} savepoint_capture_ns={} savepoint_restore_ns={} candidate_restore_ns={} resolver_index_ns={} vfs_stage_ns={} request_extraction_ns={} engine_entry_exit_ns={} resolver_ns={} local_lookup_ns={} manifest_lookup_ns={} object_load_ns={} content_hash_ns={} response_build_ns={} resolver_overhead_ns={} preload_ns={} provision_ns={} accepted_handoff_ns={} cli_overhead_ns={} accepted_phase_sum_ns={} local_lookups={} local_hits={} manifest_lookups={} manifest_cache_hits={} authenticated_manifest_hits={} manifest_reads={} manifest_parses={} manifest_authentications={} shard_loads={} object_requests={} object_cache_hits={} object_hashes={}",
            host.startup_time.as_nanos(),
            engine_core.as_nanos(),
            telemetry.execution.savepoint_capture_time.as_nanos(),
            telemetry.execution.savepoint_restore_time.as_nanos(),
            telemetry.candidate_restore_time.as_nanos(),
            telemetry.resolver_index_time.as_nanos(),
            telemetry.vfs_stage_time.as_nanos(),
            telemetry.request_extraction_time.as_nanos(),
            engine_entry_exit.as_nanos(),
            host.resolver_time.as_nanos(),
            host.resolver.local_lookup_time.as_nanos(),
            host.resolver.manifest_lookup_time.as_nanos(),
            host.resolver.object_load_time.as_nanos(),
            host.resolver.content_hash_time.as_nanos(),
            host.resolver.response_build_time.as_nanos(),
            host.resolver_time
                .saturating_sub(resolver_accounted)
                .as_nanos(),
            host.preload_time.as_nanos(),
            host.provision_time.as_nanos(),
            host.accepted_handoff_time.as_nanos(),
            cli_overhead.as_nanos(),
            accepted_wall.as_nanos(),
            host.resolver.local_lookups,
            host.resolver.local_hits,
            host.resolver.manifest_lookups,
            host.resolver.manifest_cache_hits,
            host.resolver.authenticated_manifest_hits,
            host.resolver.manifest_reads,
            host.resolver.manifest_parses,
            host.resolver.manifest_authentications,
            host.resolver.shard_loads,
            host.resolver.object_requests,
            host.resolver.object_cache_hits,
            host.resolver.object_hashes,
        );
    }
    let virtual_font_resources = finalization.virtual_font_resources;
    let pdf_raw_object_file_receipt = finalization.pdf_raw_object_file_receipt;
    let completion = finalization.completion;
    let format_dump = finalization.format_dump;
    #[cfg_attr(not(feature = "profiling"), allow(unused_variables))]
    let expansion_stats = finalization.expansion_stats;
    if opts.format_out.is_some() && format_dump.is_none() {
        return Err(CliError::MissingFormatDump);
    }
    #[cfg(feature = "profiling")]
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
    #[cfg(feature = "profiling")]
    if opts.profiling_stats {
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
        if completion
            .pdf()
            .and_then(tex_state::DetachedPdfCompletion::output_parameters)
            .is_some_and(|parameters| parameters.draft_mode > 0)
        {
            eprintln!("pdfTeX warning: \\pdfdraftmode enabled, not changing output pdf");
        } else {
            let pdf_started = std::time::Instant::now();
            let pdf_completion = completion.pdf().ok_or(CliError::Usage(
                "the accepted session returned no PDF completion",
            ))?;
            let pdf = umber::pdf_from_accepted_artifacts_with_virtual_fonts(
                pdf_completion,
                &virtual_font_resources,
                &pdf_raw_object_file_receipt,
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
    if let Some(html_path) = &opts.html {
        let html = output
            .html
            .as_ref()
            .ok_or(CliError::Usage("the accepted session returned no HTML"))?;
        if let Some(asset_dir) = &opts.html_assets {
            let base = html_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            for asset in &output.html_assets {
                driver_files.push(DriverFile::new(
                    base.join(asset_dir).join(&asset.path),
                    asset.bytes.clone(),
                ));
            }
        }
        driver_files.push(DriverFile::new(html_path.clone(), html.clone()));
    }
    let format_output = format_dump.as_ref().map(|_| {
        opts.format_out.clone().unwrap_or_else(|| {
            PathBuf::from(
                opts.input
                    .file_stem()
                    .unwrap_or_else(|| std::ffi::OsStr::new("texput")),
            )
            .with_extension("fmt")
        })
    });
    if let Some(output) = &format_output {
        let format = format_dump
            .as_ref()
            .expect("a dumped format has its detached image")
            .image
            .as_bytes()
            .to_vec();
        driver_files.push(DriverFile::new(output.clone(), format));
    }
    if let Some(receipt_output) = &opts.input_records_out {
        driver_files.push(DriverFile::new(
            receipt_output.clone(),
            input_record_receipt(&resolved_inputs, Some(main_input))?,
        ));
    }
    let materialize_started = std::time::Instant::now();
    let publication = completion
        .into_publication()
        .map_err(umber::FinalizationError::from)?;
    let finalization = PlannedFinalization::new(publication, driver_files)?;
    if opts.show_fixtures {
        print!("{}", String::from_utf8_lossy(&output.terminal));
        finalization.discard_uncommitted();
        return Ok(());
    }
    let mut destination = World::real();
    let committed = finalization.commit_effects(&mut destination)?;
    committed.materialize(&mut destination)?;
    if let (Some(dump), Some(path)) = (&format_dump, &format_output) {
        confirm_detached_format_publication(&dump.receipt, &path.to_string_lossy());
    }
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

fn confirm_detached_format_publication(
    receipt: &tex_exec::FormatDumpReceipt,
    displayed_file_name: &str,
) {
    let ident = &receipt.format_ident;
    let announcement = format!(
        "\nBeginning to dump on file {displayed_file_name}\n (preloaded format={} {}.{}.{})\n",
        ident.format_name, ident.year, ident.month, ident.day
    );
    print!("{announcement}");
}

struct RunCliOptions {
    input: PathBuf,
    show_fixtures: bool,
    dvi: Option<PathBuf>,
    pdf: Option<PathBuf>,
    html: Option<PathBuf>,
    html_assets: Option<PathBuf>,
    format: Option<PathBuf>,
    format_out: Option<PathBuf>,
    input_records_out: Option<PathBuf>,
    pdf_font_closure_out: Option<PathBuf>,
    initial_prefetch_keys: Vec<String>,
    engine: RunEngine,
    distribution: Option<String>,
    distribution_sha256: Option<String>,
    offline: bool,
    expansion_fuel: Option<u64>,
    #[cfg(feature = "profiling")]
    profiling_stats: bool,
}

impl RunCliOptions {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, CliError> {
        let mut input = None;
        let mut show_fixtures = false;
        let mut dvi = None;
        let mut pdf = None;
        let mut html = None;
        let mut html_assets = None;
        let mut format = None;
        let mut format_out = None;
        let mut input_records_out = None;
        let mut pdf_font_closure_out = None;
        let mut initial_prefetch_keys = Vec::new();
        let mut engine = RunEngine::Tex82;
        let mut distribution = None;
        let mut distribution_sha256 = None;
        let mut offline = env::var_os("UMBER_OFFLINE").is_some_and(|value| value == "1");
        let mut expansion_fuel = None;
        #[cfg(feature = "profiling")]
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
                #[cfg(feature = "profiling")]
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
                    return Err(CliError::Usage(
                        "--html-font-dir was removed; configure the authenticated HTML root with --distribution and --distribution-sha256, or provide application/private fonts through the typed resource resolver API",
                    ));
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
                "--pdf-font-closure-out" => {
                    if pdf_font_closure_out.is_some() {
                        return Err(CliError::Usage(
                            "run accepts at most one --pdf-font-closure-out path",
                        ));
                    }
                    let Some(path) = args.next() else {
                        return Err(CliError::Usage(
                            "missing output path for --pdf-font-closure-out",
                        ));
                    };
                    pdf_font_closure_out = Some(PathBuf::from(path));
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
        if pdf_font_closure_out.is_some() && pdf.is_none() {
            return Err(CliError::Usage("--pdf-font-closure-out requires --pdf"));
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
            html_assets,
            format,
            format_out,
            input_records_out,
            pdf_font_closure_out,
            initial_prefetch_keys,
            engine,
            distribution,
            distribution_sha256,
            offline,
            expansion_fuel,
            #[cfg(feature = "profiling")]
            profiling_stats,
        })
    }
}

fn input_record_receipt(
    resolved_inputs: &[(PathBuf, usize)],
    main_input: Option<(PathBuf, usize)>,
) -> Result<Vec<u8>, CliError> {
    let mut records = BTreeMap::<PathBuf, usize>::new();
    for (path, len) in resolved_inputs {
        insert_input_record(&mut records, path.clone(), *len)?;
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

fn format_token<G>(token: Token, stores: &Universe<G>) -> String {
    match token {
        Token::Char { ch, cat } => format!("char:{}:{}", ch as u32, cat as u8),
        Token::Cs(symbol) => format!(
            "cs:{}",
            stores
                .resolve(symbol)
                .expect("command token symbol belongs to the admitted engine")
        ),
        Token::Param(slot) => format!("param:{slot}"),
        token if token.is_frozen_end_template() => "frozen:endtemplate".to_owned(),
        token if token.is_frozen_endv() => "frozen:endv".to_owned(),
        Token::Frozen(_) => unreachable!("invalid frozen token payload"),
    }
}

fn format_source_token(token: &SourceToken) -> String {
    match token {
        SourceToken::Character { code, catcode, .. } => format!(
            "char:{}:{}",
            code.to_char().expect("Unicode command profile") as u32,
            *catcode as u8
        ),
        SourceToken::ControlSequence { name, kind, .. } => {
            let name = match kind {
                SourceControlSequenceKind::Paragraph => "par".to_owned(),
                SourceControlSequenceKind::Null => String::new(),
                SourceControlSequenceKind::Word
                | SourceControlSequenceKind::Symbol
                | SourceControlSequenceKind::Active => name
                    .iter()
                    .map(|code| code.to_char().expect("Unicode command profile"))
                    .collect(),
            };
            format!("cs:{name}")
        }
    }
}

#[derive(Debug)]
enum CliError {
    Usage(&'static str),
    World(WorldError),
    Lex(String),
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

    fn causal_diagnostic(&self) -> Option<&umber::CompileDiagnostic> {
        match self {
            Self::NativeRun(error) => error.diagnostic(),
            _ => None,
        }
    }
}

fn causal_diagnostic_line(diagnostic: &umber::CompileDiagnostic) -> String {
    let cause_sha256 = Sha256::digest(diagnostic.message.as_bytes());
    let mut line = format!("CAUSAL_DIAGNOSTIC schema=1 cause_sha256={cause_sha256:x}");
    match &diagnostic.location {
        Some(location) => {
            let source_sha256 = Sha256::digest(location.file.as_bytes());
            line.push_str(&format!(
                " source_sha256={source_sha256:x} bytes={}..{} line={} column={}",
                location.byte_start, location.byte_end, location.line, location.column
            ));
        }
        None => line.push_str(" source=unknown"),
    }
    if let Some(context) = &diagnostic.context {
        line.push_str(&format!(
            " cause_kind={} input_frames={} input_tail={:?} group_depth={} group_tail={:?}",
            context.cause_kind,
            context.input_frame_count,
            context.input_frame_tail.join(","),
            context.group_depth,
            context
                .group_tail
                .iter()
                .map(|group| format!("{}@{}", group.kind, group.entered_line))
                .collect::<Vec<_>>()
                .join(",")
        ));
    } else {
        line.push_str(" input_frames=unknown input_tail=\"\" group_depth=unknown group_tail=\"\"");
    }
    line
}

impl std::error::Error for CliError {}

impl From<WorldError> for CliError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
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

    #[test]
    fn causal_diagnostic_is_bounded_and_content_free() {
        let diagnostic = umber::CompileDiagnostic {
            message: "invalid parameter token #4".to_owned(),
            location: Some(umber::CompileSourceLocation {
                file: format!("/private/secret/{}/package/main.tex", "x".repeat(200)),
                byte_start: 41,
                byte_end: 43,
                line: 7,
                column: 11,
                excerpt: "private source text".to_owned(),
            }),
            context: Some(Box::new(tex_exec::FrozenDiagnosticContext {
                cause_kind: "command-recoverable",
                input_frame_count: 29,
                input_frame_tail: vec!["macro-body", "macro-argument", "source"],
                group_depth: 3,
                group_tail: vec![tex_exec::FrozenDiagnosticGroup {
                    kind: "simple",
                    entered_line: 6,
                }],
            })),
        };

        let line = causal_diagnostic_line(&diagnostic);
        assert!(line.starts_with("CAUSAL_DIAGNOSTIC schema=1 cause_sha256="));
        assert!(line.contains("bytes=41..43 line=7 column=11"));
        assert!(line.contains("input_tail=\"macro-body,macro-argument,source\""));
        assert!(line.contains("group_tail=\"simple@6\""));
        assert!(!line.contains("invalid parameter token"));
        assert!(line.contains("source_sha256="));
        assert!(!line.contains("private"));
        assert!(!line.contains("secret"));
        assert!(line.len() < 1024);
    }

    #[test]
    fn input_receipt_deduplicates_accepted_resolved_reads() {
        let path = PathBuf::from("external.cfg");
        let receipt = input_record_receipt(&[(path.clone(), 8), (path, 8)], None)
            .expect("build input receipt");
        assert_eq!(receipt, b"8\texternal.cfg\n");
    }

    #[test]
    fn input_receipt_uses_resolved_paths() {
        let resolved = PathBuf::from("/locked/texmf/logical.ltx");
        let receipt =
            input_record_receipt(&[(resolved, 8)], None).expect("build authoritative receipt");

        assert_eq!(receipt, b"8\t/locked/texmf/logical.ltx\n");
    }

    #[test]
    fn input_receipt_rejects_unescaped_tsv_delimiters() {
        for path in ["tab\tname.tex", "line\nname.tex", "return\rname.tex"] {
            let error = input_record_receipt(&[(PathBuf::from(path), 1)], None)
                .expect_err("receipt paths containing delimiters must be rejected");
            assert!(matches!(error, CliError::InputReceipt(_)));
        }
    }
}
