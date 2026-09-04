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
struct HotCoreProfilingReport {
    enabled: bool,
    hot_core_before: tex_state::measurement::HotCoreCensus,
    retained_generations_before: tex_state::measurement::RetainedGenerationCensus,
    node_graph_before: tex_state::measurement::NodeGraphCensus,
}

#[cfg(feature = "profiling")]
impl HotCoreProfilingReport {
    fn new(enabled: bool) -> Self {
        if enabled {
            tex_state::measurement::enable_node_pool_owner_census();
        }
        Self {
            enabled,
            hot_core_before: tex_state::measurement::hot_core_census(),
            retained_generations_before: tex_state::measurement::retained_generation_census(),
            node_graph_before: tex_state::measurement::node_graph_census(),
        }
    }
}

#[cfg(feature = "profiling")]
impl Drop for HotCoreProfilingReport {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        let hot_core =
            tex_state::measurement::hot_core_census().saturating_sub(self.hot_core_before);
        eprintln!("HOT_CORE_CENSUS {}", hot_core_census_json(&hot_core));
        let generations = tex_state::measurement::retained_generation_census()
            .saturating_sub(self.retained_generations_before);
        eprintln!(
            "RETAINED_GENERATION_CENSUS created={} dropped={} live={} peak_live={} retired_explicitly={}",
            generations.created,
            generations.dropped,
            generations.live,
            generations.peak_live,
            generations.retired_explicitly,
        );
        let journal = tex_state::measurement::save_journal_census();
        eprintln!(
            "SAVE_JOURNAL_CENSUS entries={} capacity={} peak_entries={} entry_size={} mutation_size={} group_frame_size={} semantic_live_bytes={} spare_capacity_bytes={} group_entries={} group_capacity={} group_entry_size={} checkpoint_entries={} checkpoint_capacity={} checkpoint_entry_size={} operation_entries={} operation_capacity={} operation_entry_size={} stamp_entries={} stamp_capacity={} mutations={} mutation_words={:?} group_enters={} group_exits={} append_calls={} growths={} bytes_moved_by_growth={} maximum_group_depth={} entries_at_maximum_group_depth={}",
            journal.entries,
            journal.capacity,
            journal.peak_entries,
            journal.entry_size,
            journal.mutation_size,
            journal.group_frame_size,
            journal
                .group_entries
                .saturating_mul(journal.group_entry_size)
                .saturating_add(
                    journal
                        .checkpoint_entries
                        .saturating_mul(journal.checkpoint_entry_size)
                )
                .saturating_add(
                    journal
                        .operation_entries
                        .saturating_mul(journal.operation_entry_size)
                ),
            journal
                .group_capacity
                .saturating_sub(journal.group_entries)
                .saturating_mul(journal.group_entry_size)
                .saturating_add(
                    journal
                        .checkpoint_capacity
                        .saturating_sub(journal.checkpoint_entries)
                        .saturating_mul(journal.checkpoint_entry_size)
                )
                .saturating_add(
                    journal
                        .operation_capacity
                        .saturating_sub(journal.operation_entries)
                        .saturating_mul(journal.operation_entry_size)
                ),
            journal.group_entries,
            journal.group_capacity,
            journal.group_entry_size,
            journal.checkpoint_entries,
            journal.checkpoint_capacity,
            journal.checkpoint_entry_size,
            journal.operation_entries,
            journal.operation_capacity,
            journal.operation_entry_size,
            journal.stamp_entries,
            journal.stamp_capacity,
            journal.mutations,
            journal.mutation_words,
            journal.group_enters,
            journal.group_exits,
            journal.append_calls,
            journal.growths,
            journal.bytes_moved_by_growth,
            journal.maximum_group_depth,
            journal.entries_at_maximum_group_depth,
        );
        let nodes =
            tex_state::measurement::node_graph_census().saturating_sub(self.node_graph_before);
        eprintln!(
            "NODE_GRAPH_CENSUS rows_published={} nodes_published={} coordinate_transfers={} logical_aliases={} physical_copy_rows={} physical_copy_nodes={} external_materialization_rows={} external_materialization_nodes={} diagnostic_projection_rows={} diagnostic_projection_nodes={} checkpoint_sidecar_rows={} checkpoint_sidecar_nodes={} checkpoint_shared_rows={}",
            nodes.rows_published,
            nodes.nodes_published,
            nodes.coordinate_transfers,
            nodes.logical_aliases,
            nodes.physical_copy_rows,
            nodes.physical_copy_nodes,
            nodes.external_materialization_rows,
            nodes.external_materialization_nodes,
            nodes.diagnostic_projection_rows,
            nodes.diagnostic_projection_nodes,
            nodes.checkpoint_sidecar_rows,
            nodes.checkpoint_sidecar_nodes,
            nodes.checkpoint_shared_rows,
        );
        let storage = tex_state::measurement::node_pool_storage_census();
        eprintln!(
            "NODE_POOL_STORAGE_CENSUS node_fresh_allocations={} node_reuse_allocations={} node_releases={} node_live_blocks={} node_peak_live_blocks={} node_vacant_slots={} node_peak_vacant_slots={} node_live_payload_bytes={} node_peak_live_payload_bytes={} node_vacant_payload_bytes={} node_peak_vacant_payload_bytes={} annex_fresh_allocations={} annex_reuse_allocations={} annex_releases={} annex_live_blocks={} annex_peak_live_blocks={} annex_vacant_slots={} annex_peak_vacant_slots={} annex_live_payload_bytes={} annex_peak_live_payload_bytes={} annex_vacant_payload_bytes={} annex_peak_vacant_payload_bytes={}",
            storage.nodes.fresh_allocations,
            storage.nodes.reuse_allocations,
            storage.nodes.releases,
            storage.nodes.live_blocks,
            storage.nodes.peak_live_blocks,
            storage.nodes.vacant_slots,
            storage.nodes.peak_vacant_slots,
            storage.nodes.live_payload_bytes,
            storage.nodes.peak_live_payload_bytes,
            storage.nodes.vacant_payload_bytes,
            storage.nodes.peak_vacant_payload_bytes,
            storage.annexes.fresh_allocations,
            storage.annexes.reuse_allocations,
            storage.annexes.releases,
            storage.annexes.live_blocks,
            storage.annexes.peak_live_blocks,
            storage.annexes.vacant_slots,
            storage.annexes.peak_vacant_slots,
            storage.annexes.live_payload_bytes,
            storage.annexes.peak_live_payload_bytes,
            storage.annexes.vacant_payload_bytes,
            storage.annexes.peak_vacant_payload_bytes,
        );
        let owners = tex_state::measurement::node_pool_owner_census();
        eprintln!(
            "NODE_POOL_OWNER_CENSUS samples={} page_regions={} retained_page_regions={} checkpoint_rows={} node_live_blocks={} node_current_generation_blocks={} node_prior_generation_blocks={} node_checkpoint_history_blocks={} node_current_prior_shared_blocks={} node_current_checkpoint_shared_blocks={} node_prior_checkpoint_shared_blocks={} node_page_owner_union_blocks={} node_durable_or_other_blocks={} node_output_blocks=0 node_accepted_artifact_blocks=0 annex_live_blocks={} annex_current_generation_blocks={} annex_prior_generation_blocks={} annex_checkpoint_history_blocks={} annex_current_prior_shared_blocks={} annex_current_checkpoint_shared_blocks={} annex_prior_checkpoint_shared_blocks={} annex_page_owner_union_blocks={} annex_durable_or_other_blocks={} annex_output_blocks=0 annex_accepted_artifact_blocks=0",
            owners.samples,
            owners.page_regions,
            owners.retained_page_regions,
            owners.checkpoint_rows,
            owners.nodes.live_blocks,
            owners.nodes.current_generation_blocks,
            owners.nodes.prior_generation_blocks,
            owners.nodes.checkpoint_history_blocks,
            owners.nodes.current_prior_shared_blocks,
            owners.nodes.current_checkpoint_shared_blocks,
            owners.nodes.prior_checkpoint_shared_blocks,
            owners.nodes.page_owner_union_blocks,
            owners.nodes.durable_or_other_blocks,
            owners.annexes.live_blocks,
            owners.annexes.current_generation_blocks,
            owners.annexes.prior_generation_blocks,
            owners.annexes.checkpoint_history_blocks,
            owners.annexes.current_prior_shared_blocks,
            owners.annexes.current_checkpoint_shared_blocks,
            owners.annexes.prior_checkpoint_shared_blocks,
            owners.annexes.page_owner_union_blocks,
            owners.annexes.durable_or_other_blocks,
        );
        let output = tex_state::measurement::page_output_pool_census();
        eprintln!(
            "PAGE_OUTPUT_POOL_CENSUS installations={} zero_copy_takes={} on_demand_promotions={} node_live_blocks={} node_used_records={} node_stranded_records={} node_partial_blocks={} node_physically_shared_blocks={} node_output_region_blocks={} node_durable_or_other_blocks={} annex_live_blocks={} annex_used_records={} annex_stranded_records={} annex_partial_blocks={} annex_physically_shared_blocks={} annex_output_region_blocks={} annex_durable_or_other_blocks={}",
            output.installations,
            output.zero_copy_takes,
            output.on_demand_promotions,
            output.nodes.live_blocks,
            output.nodes.used_records,
            output.nodes.stranded_records,
            output.nodes.partial_blocks,
            output.nodes.physically_shared_blocks,
            output.nodes.output_region_blocks,
            output.nodes.durable_or_other_blocks,
            output.annexes.live_blocks,
            output.annexes.used_records,
            output.annexes.stranded_records,
            output.annexes.partial_blocks,
            output.annexes.physically_shared_blocks,
            output.annexes.output_region_blocks,
            output.annexes.durable_or_other_blocks,
        );
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

    let mut output = String::from("{\"schema\":4,\"allocations\":{");
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
    output.push_str("},\"command_families\":{");
    first = true;
    for (name, count) in tex_state::measurement::HotCoreCommandFamily::NAMES
        .into_iter()
        .zip(census.command_families)
    {
        separator(&mut output, &mut first);
        write!(output, "\"{name}\":{count}").expect("writing to a String cannot fail");
    }
    output.push_str("},\"main_control_meanings\":{");
    first = true;
    for (name, count) in tex_state::measurement::HotCoreMeaningFamily::NAMES
        .into_iter()
        .zip(census.main_control_meanings)
    {
        separator(&mut output, &mut first);
        write!(output, "\"{name}\":{count}").expect("writing to a String cannot fail");
    }
    write!(
        output,
        "}},\"expansion_opcodes\":{{\"macro\":{},\"undefined\":{},\"primitives\":{{",
        census.macro_expansions, census.undefined_expansions,
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
    output.push_str("},\"page_builder_transitions\":{");
    first = true;
    for (name, count) in tex_state::measurement::HotCorePageBuilderTransition::NAMES
        .into_iter()
        .zip(census.page_builder_transitions)
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
    let _hot_core_profiling_report = HotCoreProfilingReport::new(opts.profiling_stats);
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
            pdf_output_mode: opts.pdf_output_mode(),
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
            distribution_ahash64: opts.distribution_ahash64.clone(),
            offline: opts.offline,
            expansion_fuel: opts.expansion_fuel,
            execution_steps: opts.execution_steps,
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
            "RESOURCE_HOST_TELEMETRY startup_ns={} engine_core_ns={} savepoint_capture_ns={} savepoint_restore_ns={} candidate_restore_ns={} resolver_index_ns={} vfs_stage_ns={} request_extraction_ns={} engine_entry_exit_ns={} resolver_ns={} local_lookup_ns={} manifest_lookup_ns={} object_load_ns={} content_hash_ns={} response_build_ns={} resolver_overhead_ns={} preload_ns={} provision_ns={} accepted_handoff_ns={} cli_overhead_ns={} accepted_phase_sum_ns={} local_lookups={} local_hits={} manifest_lookups={} manifest_cache_hits={} verified_manifest_hits={} manifest_reads={} manifest_read_bytes={} manifest_parses={} manifest_validations={} shard_loads={} packed_selection_calls={} packed_selection_keys={} packed_selection_bytes={} packed_validation_calls={} packed_validation_bytes={} manifest_parse_peak_bytes={} retained_manifest_shards={} retained_manifest_bytes={} object_requests={} object_cache_hits={} object_hashes={}",
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
            host.resolver.verified_manifest_hits,
            host.resolver.manifest_reads,
            host.resolver.manifest_read_bytes,
            host.resolver.manifest_parses,
            host.resolver.manifest_validations,
            host.resolver.shard_loads,
            host.resolver.packed_selection_calls,
            host.resolver.packed_selection_keys,
            host.resolver.packed_selection_bytes,
            host.resolver.packed_validation_calls,
            host.resolver.packed_validation_bytes,
            host.resolver.manifest_parse_peak_bytes,
            host.resolver.retained_manifest_shards,
            host.resolver.retained_manifest_bytes,
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
    distribution_ahash64: Option<String>,
    offline: bool,
    expansion_fuel: Option<u64>,
    execution_steps: Option<u64>,
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
        let mut distribution_ahash64 = None;
        let mut offline = env::var_os("UMBER_OFFLINE").is_some_and(|value| value == "1");
        let mut expansion_fuel = None;
        let mut execution_steps = None;
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
                "--execution-steps" => {
                    if execution_steps.is_some() {
                        return Err(CliError::Usage("run accepts at most one --execution-steps"));
                    }
                    let value = args.next().ok_or(CliError::Usage(
                        "missing positive integer for --execution-steps",
                    ))?;
                    execution_steps =
                        Some(value.parse::<u64>().ok().filter(|value| *value > 0).ok_or(
                            CliError::Usage("--execution-steps must be a positive integer"),
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
                "--distribution-ahash64" => {
                    if distribution_ahash64.is_some() {
                        return Err(CliError::Usage(
                            "run accepts at most one --distribution-ahash64",
                        ));
                    }
                    distribution_ahash64 = Some(
                        args.next()
                            .ok_or(CliError::Usage("missing digest for --distribution-ahash64"))?,
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
                        "--html-font-dir was removed; configure the authenticated HTML root with --distribution and --distribution-ahash64, or provide application/private fonts through the typed resource resolver API",
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
        if distribution_ahash64.is_none() {
            distribution_ahash64 = env::var("UMBER_DISTRIBUTION_AHASH64").ok();
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
            distribution_ahash64,
            offline,
            expansion_fuel,
            execution_steps,
            #[cfg(feature = "profiling")]
            profiling_stats,
        })
    }

    /// Maps explicit native publication flags to pdftex.web §1515's
    /// process-selected semantic mode. A PDF request wins when the host also
    /// asks for a downstream DVI copy; absent flags preserve format/default
    /// behavior.
    fn pdf_output_mode(&self) -> Option<umber::PdfOutputMode> {
        if !self.engine.supports_pdf_output() {
            None
        } else if self.pdf.is_some() {
            Some(umber::PdfOutputMode::Pdf)
        } else if self.dvi.is_some() {
            Some(umber::PdfOutputMode::Dvi)
        } else {
            None
        }
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
    if let Some(first) = &diagnostic.first_recoverable {
        let first_message_sha256 = Sha256::digest(first.message.as_bytes());
        line.push_str(&format!(
            " first_kind={} first_message_sha256={first_message_sha256:x} first_mode={:?} first_scanner_status={} first_interaction={:?}",
            first.kind, first.mode, first.scanner_status, first.interaction
        ));
        if let Some(command) = &first.command {
            let command_sha256 = Sha256::digest(command.as_bytes());
            line.push_str(&format!(" first_command_sha256={command_sha256:x}"));
        } else {
            line.push_str(" first_command=unknown");
        }
        if let Some(token) = &first.observed_token {
            let token_sha256 = Sha256::digest(format!("{token:?}").as_bytes());
            line.push_str(&format!(" first_token_sha256={token_sha256:x}"));
        } else {
            line.push_str(" first_token=unknown");
        }
        if let Some(context) = &first.context {
            line.push_str(&format!(
                " first_cause_kind={} first_input_frames={} first_input_tail={:?} first_group_depth={} first_group_tail={:?}",
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
            line.push_str(" first_input_frames=unknown first_input_tail=\"\" first_group_depth=unknown first_group_tail=\"\"");
        }
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
    fn run_parser_keeps_expansion_fuel_and_execution_steps_independent() {
        let options = RunCliOptions::parse(
            [
                "--expansion-fuel",
                "50000000",
                "--execution-steps",
                "100000000",
                "main.tex",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .expect("independent run guards");

        assert_eq!(options.expansion_fuel, Some(50_000_000));
        assert_eq!(options.execution_steps, Some(100_000_000));
    }

    #[test]
    fn run_parser_rejects_invalid_or_duplicate_execution_step_caps() {
        for (arguments, expected) in [
            (
                vec!["--execution-steps", "0", "main.tex"],
                "--execution-steps must be a positive integer",
            ),
            (
                vec!["--execution-steps", "word", "main.tex"],
                "--execution-steps must be a positive integer",
            ),
            (
                vec![
                    "--execution-steps",
                    "1",
                    "--execution-steps",
                    "2",
                    "main.tex",
                ],
                "run accepts at most one --execution-steps",
            ),
        ] {
            let error = match RunCliOptions::parse(arguments.into_iter().map(str::to_owned)) {
                Ok(_) => panic!("invalid execution-step cap was accepted"),
                Err(error) => error,
            };
            assert!(matches!(error, CliError::Usage(message) if message == expected));
        }
    }

    #[test]
    fn explicit_pdftex_output_paths_select_the_semantic_output_mode() {
        for (arguments, expected) in [
            (
                vec!["--pdflatex", "--dvi", "main.dvi", "main.tex"],
                Some(umber::PdfOutputMode::Dvi),
            ),
            (
                vec!["--pdflatex", "--pdf", "main.pdf", "main.tex"],
                Some(umber::PdfOutputMode::Pdf),
            ),
            (
                vec![
                    "--pdflatex",
                    "--dvi",
                    "main.dvi",
                    "--pdf",
                    "main.pdf",
                    "main.tex",
                ],
                Some(umber::PdfOutputMode::Pdf),
            ),
            (vec!["--pdflatex", "main.tex"], None),
            (vec!["--latex", "--dvi", "main.dvi", "main.tex"], None),
        ] {
            let options = RunCliOptions::parse(arguments.into_iter().map(str::to_owned))
                .expect("valid output selection");
            assert_eq!(options.pdf_output_mode(), expected);
        }
    }

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
            first_recoverable: None,
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
    fn causal_diagnostic_renders_first_recoverable_metadata_without_content() {
        let diagnostic = umber::CompileDiagnostic {
            message: "fatal aggregate".into(),
            location: None,
            context: None,
            first_recoverable: Some(Box::new(tex_exec::FirstRecoverableDiagnostic {
                kind: "undefined-control-sequence",
                message: "secret source token".into(),
                arguments: Vec::new(),
                command: Some("\\secret".into()),
                command_operand: None,
                observed_token: Some(tex_command::ObservedToken::ControlSequence("secret".into())),
                origin: None,
                context: None,
                mode: tex_exec::Mode::Vertical,
                scanner_status: "normal",
                interaction: tex_state::InteractionMode::Nonstop,
            })),
        };

        let line = causal_diagnostic_line(&diagnostic);
        assert!(line.contains("first_kind=undefined-control-sequence"));
        assert!(line.contains("first_message_sha256="));
        assert!(line.contains("first_command_sha256="));
        assert!(line.contains("first_token_sha256="));
        assert!(!line.contains("secret source token"));
        assert!(!line.contains("\\secret"));
        assert!(!line.contains("first_command=unknown"));
        assert!(!line.contains("first_token=unknown"));
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
