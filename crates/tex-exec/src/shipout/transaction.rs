use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::node::{Node, NodeKind};
use tex_state::node_arena::NodeView;
use tex_state::{
    ContentHash, DetachedArtifact, MemoTimingPhase, MemoValueLimits, PrintSink, PureMemoKey,
    PureMemoLayer, PureShipoutEntry, Universe,
};

use crate::ExecError;
use crate::dispatch::{CommittedPagePublication, PreparedDviPage};

use super::direct;
use super::{ShipoutGeometry, ShipoutGeometrySink, ShipoutOrigin, TextReplayHost, WriteReplayHost};

#[cfg(test)]
mod tests;

const SHIPOUT_EPISODE_DOMAIN: u32 = 4;
const SHIPOUT_EPISODE_SCHEMA: u32 = 1;
const SHIPOUT_ENV_HASH_DOMAIN: u64 = 0x7368_6970_656e_7601;

/// Resumes TeX82 §§530 and 1373--1375 after an authoritative output-open
/// failure retained the failed effect and its following suffix.
pub fn retry_unavailable_stream_open<G>(
    stores: &mut Universe<G>,
    failed: &tex_state::StreamOpenFailure,
) -> Result<std::path::PathBuf, ExecError> {
    let interaction = stores.interaction_mode();
    let failed_name = failed.path().to_string_lossy();
    if matches!(
        interaction,
        tex_state::InteractionMode::Batch | tex_state::InteractionMode::Nonstop
    ) {
        stores
            .print_err("I can't write on file `")
            .print(&failed_name)
            .print("'.");
        return Err(ExecError::Fatal(tex_command::FatalError::emergency_stop(
            "job aborted, file error in nonstop mode",
        )));
    }
    let mut report = stores.print_err("I can't write on file `");
    report
        .print(&failed_name)
        .print("'.")
        .print_rendered(failed.context())
        .print_rendered("\n")
        .print("Please type another output file name");
    drop(report);
    let replacement = stores
        .command_context()
        .expect("output retry runs inside an admitted command episode")
        .input_ln(tex_state::CommandLineSource::Terminal { prompt: ": " })
        .ok_or(ExecError::Fatal(tex_command::FatalError::emergency_stop(
            "End of file on the terminal!",
        )))?;
    let replacement = direct::terminal_output_name(&replacement);
    Ok(replacement.into())
}

// TeX82 map: `ship_out` consumes a box whose child list is visited by
// `hlist_out`/`vlist_out`. Fresh pages use the direct two-phase emitter in
// `direct`: mutation and rare-node normalization finish first, then one live
// compact-list traversal writes canonical artifact bytes. The same detached
// artifact-to-DVI compiler serves fresh and memo-hit publication.

/// Ships a completed box using already-rendered diagnostic context.
///
/// Command replay has no independent source stack. The surrounding command
/// boundary therefore renders §82's context before staging and passes only
/// its owned string here. The live pending-effect offset remains a separate
/// transaction argument rather than entering that detached value.
#[allow(clippy::too_many_arguments)] // Shipout is the explicit join of transaction capabilities and page policy.
pub(crate) fn shipout_node<G>(
    source: direct::ShipoutRoot,
    region: Option<tex_state::node_region::PageClosureBuildMark>,
    origin: ShipoutOrigin,
    pending_effect_end: usize,
    stores: &mut Universe<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source_resolver: &dyn crate::output_provenance::ArtifactSourceResolver,
    provenance_demand: tex_state::ProvenanceDemand,
    provenance_budget_bytes: usize,
    geometry_sink: &mut dyn ShipoutGeometrySink,
    emit_dvi: bool,
    write_expander: &mut direct::WriteExpander<'_, G>,
    replay_expander: &mut direct::ReplayTextExpander<'_, G>,
) -> Result<Option<CommittedPagePublication>, ExecError> {
    if let Err(error) =
        prepare_pdf_output_policy(stores, diagnostic_effects, &origin.output_open_context)
    {
        retain_failed_page(stores, region);
        return Err(error);
    }
    let geometry = shipout_geometry(&source, stores);
    if huge_shipout_box(&source, stores) {
        // TeX.web §641 drops the page rather than emitting it, so the report
        // is the whole of the engine's response. Shipout also runs from
        // command replay, which owns no live source stack. Its caller
        // captured §82's display from the command-owned stack before
        // releasing that borrow.
        let context = origin.output_open_context.clone();
        let reported = {
            let mut command = stores.command_context().expect("live generation");
            crate::error_report::report_error(
                &mut command,
                diagnostic_effects,
                "Huge page cannot be shipped out",
                &[
                    "The page just created is more than 18 feet tall or",
                    "more than 18 feet wide, so I suspect something went wrong.",
                ],
                context,
            )
        };
        if let Err(error) = reported {
            retain_failed_page(stores, region);
            return Err(error);
        }
        report_huge_page_deleted_box(
            stores,
            diagnostic_effects,
            &source,
            stores.int_param(IntParam::TRACING_OUTPUT),
        );
        release_published_page(stores, region);
        return Ok(None);
    }
    let memo_enabled = stores
        .with_pure_memo(|memo| memo.is_enabled())
        .unwrap_or(false);
    let shipout_memo_enabled = stores
        .with_pure_memo(|memo| memo.shipout_episodes_enabled())
        .unwrap_or(false);
    if memo_enabled && !shipout_memo_enabled {
        stores.with_pure_memo(|memo| memo.record_not_attempted(PureMemoLayer::Shipout));
    }
    let cacheable = shipout_memo_enabled
        && !provenance_demand.rendered_source()
        && matches!(&source, direct::ShipoutRoot::Page(node) if effect_free_shipout_graph(stores, node))
        && stores.world().effect_records()[..pending_effect_end].is_empty()
        && (1..=32_768).contains(&stores.int_param(IntParam::MAG));
    let validation_started = crate::timing::TelemetryTimer::start();
    let key = cacheable.then(|| match &source {
        direct::ShipoutRoot::Page(node) => shipout_key(stores, node),
    });
    if cacheable {
        stores.with_pure_memo(|memo| {
            memo.record_timing(
                PureMemoLayer::Shipout,
                MemoTimingPhase::Validation,
                validation_started.elapsed(),
            );
        });
    }
    if !cacheable {
        stores.with_pure_memo(tex_state::PureMemoRuntime::record_shipout_barrier);
    }
    if let Some(key) = key
        && let Some(entry) = stores
            .with_pure_memo(|memo| memo.lookup_shipout(key))
            .flatten()
    {
        let import_started = crate::timing::TelemetryTimer::start();
        let detached = entry.artifact.artifact(MemoValueLimits::default());
        if let Ok(detached) = detached {
            let imported_bytes = entry.artifact.retained_bytes();
            let replayed = stores.commit_replayed_artifact(
                detached.payload,
                Vec::new(),
                Default::default(),
                None,
            );
            let (hash, artifact, publication) = match replayed {
                Ok(replayed) => replayed,
                Err(error) => {
                    retain_failed_page(stores, region);
                    return Err(error.into());
                }
            };
            stores.with_pure_memo(|memo| {
                memo.record_timing(
                    PureMemoLayer::Shipout,
                    MemoTimingPhase::Import,
                    import_started.elapsed(),
                );
                memo.record_shipout_hit(imported_bytes);
            });
            // The memo retains the detached artifact, not an execution-owned
            // plan. Rebuild its equivalent pure receipt exactly once at this
            // publication boundary so callers never need to lower
            // an already-committed page during finalization.
            if let Some(geometry) = geometry {
                geometry_sink.committed_shipout_geometry(geometry);
            }
            release_published_page(stores, region);
            let plan = direct::compile_dvi_plan(
                stores
                    .world()
                    .committed_artifacts()
                    .last()
                    .expect("replayed artifact commit must publish a receipt")
                    .bytes(),
                emit_dvi,
            )?;
            return Ok(Some(CommittedPagePublication {
                artifact,
                dvi: plan.map(|plan| PreparedDviPage {
                    hash,
                    plan,
                    committed_effects: Box::new([]),
                    publication,
                    receipt: publication.receipt(),
                }),
                effects: 0..0,
                effect_output_attempt: None,
            }));
        }
        stores.with_pure_memo(|memo| {
            memo.record_timing(
                PureMemoLayer::Shipout,
                MemoTimingPhase::Import,
                import_started.elapsed(),
            );
            memo.reject(key);
        });
    }
    let effect_start = stores.world().effect_records().len();
    // Absolute, unlike the index above: committing the transaction *drains*
    // the live effect log, so only a position that survives the drain can
    // answer "did staging this page produce an effect of its own?" below.
    let effect_pos_start = stores.world().effect_pos();
    let mut transaction = stores.begin_shipout();
    let staged = direct::stage_shipout(
        source,
        origin,
        pending_effect_end,
        &mut transaction,
        diagnostic_effects,
        source_resolver,
        provenance_demand,
        provenance_budget_bytes,
        emit_dvi,
        write_expander,
        replay_expander,
    );
    let mut staged = match staged {
        Ok(staged) => staged,
        Err(error) => {
            drop(transaction);
            retain_failed_page(stores, region);
            return Err(error);
        }
    };
    // Deferred command expansion happens while the aggregate shipout
    // transaction is live. Its detached diagnostics are part of the same
    // ordered TeX print program as the surrounding `[` marker and the page
    // payload, so admit them to the speculative World before that World
    // commits its effect prefix. A later commit failure still rolls them back
    // with the transaction; carrying them out to MainControl would publish
    // them only after the already-materialized page output.
    transaction
        .world_mut()
        .publish_diagnostic_effects(std::mem::take(diagnostic_effects));
    let committed_effects = transaction.world().effect_records()[effect_start..]
        .to_vec()
        .into_boxed_slice();
    let retained_diagnostics = std::mem::take(&mut staged.retained_diagnostics);
    let memo_payload = key.is_some().then(|| staged.artifact.bytes().to_vec());
    let reservation = transaction
        .world_mut()
        .reserve_active_artifact_publication_at(effect_start, None);
    let effect_end = transaction.world().effect_records().len();
    let staged_effect_pos = transaction.world().effect_pos();
    let committed = transaction.commit(staged.artifact, staged_effect_pos, reservation);
    let (hash, publication) = match committed {
        Ok(committed) => committed,
        Err(error) => {
            retain_failed_page(stores, region);
            return Err(error.into());
        }
    };
    release_published_page(stores, region);
    let effect_publication = stores.world_mut().reserve_effect_publication();
    stores
        .world_mut()
        .claim_effect_publication(effect_start..effect_end, effect_publication);
    stores
        .world_mut()
        .link_artifact_effect_publication(publication.publication(), effect_publication);
    let publication = publication.with_effect_publication(effect_publication);
    let artifact =
        tex_state::PageOutputPublicationReceipt::committed(effect_publication, publication);
    if let Some(geometry) = geometry {
        geometry_sink.committed_shipout_geometry(geometry);
    }
    for (sink, text) in retained_diagnostics {
        stores.world_mut().write_text(sink, &text);
    }
    if let (Some(key), Some(artifact_bytes)) = (key, memo_payload)
        && stores.world().effect_pos() == effect_pos_start
        && let Ok(artifact) = tex_state::DetachedMemoValue::from_artifact(&DetachedArtifact {
            artifact_schema: 10,
            payload: artifact_bytes,
        })
    {
        stores.with_pure_memo(|memo| {
            memo.insert_shipout(
                key,
                PureShipoutEntry {
                    artifact,
                    render_origin_ends: Vec::new(),
                    render_provenance: Default::default(),
                },
            );
        });
    }
    Ok(Some(CommittedPagePublication {
        artifact,
        dvi: staged.dvi_plan.map(|plan| PreparedDviPage {
            hash,
            plan,
            committed_effects,
            publication,
            receipt: publication.receipt(),
        }),
        effects: 0..0,
        effect_output_attempt: None,
    }))
}

/// Drops the complete execution-scoped page suffix at its terminal boundary.
fn release_published_page<G>(
    stores: &mut Universe<G>,
    region: Option<tex_state::node_region::PageClosureBuildMark>,
) {
    if let Some(region) = region {
        stores
            .release_page_node_region(region)
            .expect("terminal shipout owns its complete nested page suffix");
    }
}

/// Returns a failed operand suffix to the enclosing direct operation.
///
/// Aggregate shipout rollback has restored its own owners before this call,
/// but the enclosing command rollback may still reopen the operand's box mode.
/// Retaining the rows is therefore required until that command commits or its
/// complete page arena is disposed.
fn retain_failed_page<G>(
    _stores: &mut Universe<G>,
    region: Option<tex_state::node_region::PageClosureBuildMark>,
) {
    if let Some(region) = region {
        // The enclosing command operation already owns the rollback cursor
        // for these rows. Consuming the narrower construction capability
        // leaves that aggregate owner authoritative until commit or rollback.
        let _region = region;
    }
}

/// TeX82 §641's huge-page recovery tail.
///
/// Positive `\tracingoutput` has already displayed the page at §638. At
/// zero or below, `ship_out` must identify and display the rejected box here
/// before its caller prints the closing page marker.
fn report_huge_page_deleted_box<G>(
    stores: &mut Universe<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source: &direct::ShipoutRoot,
    tracing_output: i32,
) {
    if tracing_output > 0 {
        return;
    }
    let command = stores.command_context().expect("live generation");
    let config = crate::node_dump::DumpConfig::read(&command);
    let dump = match source {
        direct::ShipoutRoot::Page(node) => {
            crate::node_dump::dump_node_slice(&command, std::slice::from_ref(node), config)
        }
    };
    let mut diagnostic = command.begin_diagnostic(diagnostic_effects);
    diagnostic
        .print_nl("The following box has been deleted:")
        .print_ln()
        .print_rendered(&dump);
    diagnostic.end(true);
}

#[allow(clippy::too_many_arguments)] // Staging retains the same explicit capabilities at the replay boundary.
pub(crate) fn stage_page<G>(
    source: direct::ShipoutRoot,
    region: Option<tex_state::node_region::PageClosureBuildMark>,
    origin: ShipoutOrigin,
    pending_effect_end: usize,
    stores: &mut Universe<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source_resolver: &dyn crate::output_provenance::ArtifactSourceResolver,
    provenance_demand: tex_state::ProvenanceDemand,
    provenance_budget_bytes: usize,
    geometry_sink: &mut dyn ShipoutGeometrySink,
    emit_dvi: bool,
    write_expander: &mut WriteReplayHost<'_, G>,
    replay_expander: &mut TextReplayHost<'_, G>,
) -> Result<Option<CommittedPagePublication>, ExecError> {
    shipout_node(
        source,
        region,
        origin,
        pending_effect_end,
        stores,
        diagnostic_effects,
        source_resolver,
        provenance_demand,
        provenance_budget_bytes,
        geometry_sink,
        emit_dvi,
        write_expander,
        replay_expander,
    )
}

pub(crate) fn stage_form<G>(
    form: tex_state::PdfFormRecord<G>,
    stores: &mut Universe<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    write_expander: &mut WriteReplayHost<'_, G>,
    replay_expander: &mut TextReplayHost<'_, G>,
) -> Result<tex_state::PdfFormArtifact, ExecError> {
    direct::stage_form(
        form,
        stores,
        diagnostic_effects,
        write_expander,
        replay_expander,
    )
}

fn shipout_geometry<G>(
    source: &direct::ShipoutRoot,
    stores: &mut Universe<G>,
) -> Option<ShipoutGeometry> {
    let (width, height, depth) = shipout_box_dimensions(source)?;
    let command = stores
        .command_context()
        .expect("shipout geometry runs inside an admitted command episode");
    Some(ShipoutGeometry {
        page_width_sp: i64::from(width.raw()),
        page_height_sp: i64::from(height.raw()) + i64::from(depth.raw()),
        counts: direct::page_counts(&command),
    })
}

fn prepare_pdf_output_policy<G>(
    stores: &mut Universe<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    error_context: &str,
) -> Result<(), ExecError> {
    let current_output = stores.int_param(IntParam::PDF_OUTPUT);
    if let Some(fixed) = stores.fixed_pdf_output_parameters() {
        if current_output != fixed.output {
            return Err(ExecError::PdfOutputModeChanged);
        }
        let current_major = stores.int_param(IntParam::PDF_MAJOR_VERSION);
        let current_minor = stores.int_param(IntParam::PDF_MINOR_VERSION);
        if fixed.output > 0
            && (current_major != fixed.major_version || current_minor != fixed.minor_version)
        {
            return Err(ExecError::PdfVersionChanged);
        }
        if stores.int_param(IntParam::PDF_DRAFT_MODE) != fixed.draft_mode {
            return Err(ExecError::PdfDraftModeChanged);
        }
        return Ok(());
    }
    if current_output <= 0 {
        return Ok(());
    }

    let major = stores.int_param(IntParam::PDF_MAJOR_VERSION);
    if major < 1 {
        report_invalid_pdf_version(
            stores,
            diagnostic_effects,
            "pdfTeX error (invalid pdfmajorversion)",
            &[
                "The pdfmajorversion must be 1 or greater.",
                "I changed this to 1.",
            ],
            major,
            error_context,
        )?;
        stores
            .assign_int_param(
                IntParam::PDF_MAJOR_VERSION,
                1,
                tex_state::AssignmentScope::Local,
            )
            .expect("pdf major version assignment targets admitted state");
    }
    let minor = stores.int_param(IntParam::PDF_MINOR_VERSION);
    if !(0..=9).contains(&minor) {
        report_invalid_pdf_version(
            stores,
            diagnostic_effects,
            "pdfTeX error (invalid pdfminorversion)",
            &[
                "The pdfminorversion must be between 0 and 9.",
                "I changed this to 4.",
            ],
            minor,
            error_context,
        )?;
        stores
            .assign_int_param(
                IntParam::PDF_MINOR_VERSION,
                4,
                tex_state::AssignmentScope::Local,
            )
            .expect("pdf minor version assignment targets admitted state");
    }

    let major = stores.int_param(IntParam::PDF_MAJOR_VERSION);
    let minor = stores.int_param(IntParam::PDF_MINOR_VERSION);
    if stores
        .int_param(IntParam::PDF_OBJ_COMPRESS_LEVEL)
        .clamp(0, 3)
        > 0
        && major == 1
        && minor < 5
    {
        stores.world_mut().write_text(
            PrintSink::TerminalAndLog,
            "\npdfTeX warning (Object streams): \\pdfobjcompresslevel > 0 requires PDF-1.5 or greater. Object streams disabled now.\n",
        );
    }
    Ok(())
}

/// pdftex.web's `check_pdfversion`: `print_err`, `print_ln`, `help2`, then
/// tex.web §91's `int_error` naming the rejected value.
///
/// The version is fixed at the first page, long after the command that set it
/// was scanned, so the caller supplies the already-rendered §82 context owned
/// by the shipout request.
fn report_invalid_pdf_version<G>(
    stores: &mut Universe<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    message: &str,
    help: &[&str],
    value: i32,
    error_context: &str,
) -> Result<(), ExecError> {
    let mut report = stores.print_err(message);
    // pdftex.web breaks the line before the value; `print_nl` on an open line
    // is that `print_ln`.
    report
        .print_nl("")
        .help(help)
        .context(error_context.to_owned());
    report.int_error(value).defer_recovery(diagnostic_effects)?;
    Ok(())
}

fn shipout_key<G>(stores: &mut Universe<G>, root: &Node) -> PureMemoKey {
    let environment = stores.engine_boundary_hash(SHIPOUT_ENV_HASH_DOMAIN, |hash| {
        hash.nodes(std::slice::from_ref(root));
        hash.i32(stores.int_param(IntParam::MAG));
        hash.i32(
            stores
                .dimen_param(DimenParam::H_OFFSET)
                .expect("shipout memo reads admitted hoffset")
                .raw(),
        );
        hash.i32(
            stores
                .dimen_param(DimenParam::V_OFFSET)
                .expect("shipout memo reads admitted voffset")
                .raw(),
        );
        for index in 0..10 {
            hash.i32(
                stores
                    .count(index)
                    .expect("shipout memo reads admitted count register"),
            );
        }
    });
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&SHIPOUT_EPISODE_SCHEMA.to_le_bytes());
    bytes.extend_from_slice(&environment.to_le_bytes());
    PureMemoKey::new(
        SHIPOUT_EPISODE_DOMAIN,
        environment,
        ContentHash::from_bytes(&bytes),
    )
}

fn effect_free_shipout_graph<G>(stores: &Universe<G>, root: &Node) -> bool {
    fn visit<G>(stores: &Universe<G>, view: NodeView<'_>) -> bool {
        if matches!(
            view.kind(),
            NodeKind::Whatsit
                | NodeKind::Unset
                | NodeKind::Ins
                | NodeKind::Direction
                | NodeKind::MathNoad
                | NodeKind::FractionNoad
                | NodeKind::MathStyle
                | NodeKind::MathChoice
                | NodeKind::MathList
                | NodeKind::Nonscript
                | NodeKind::Adjust
        ) {
            return false;
        }
        let effect_free = std::cell::Cell::new(true);
        view.visit_semantic_node_lists(|children| {
            if effect_free.get() {
                effect_free.set(
                    stores
                        .page_node_list(*children)
                        .expect("shipout child belongs to the live page arena")
                        .nodes()
                        .iter()
                        .all(|node| visit(stores, node)),
                );
            }
        });
        effect_free.get()
    }
    visit(stores, root.into())
}

fn shipout_box_dimensions(
    source: &direct::ShipoutRoot,
) -> Option<(
    tex_state::scaled::Scaled,
    tex_state::scaled::Scaled,
    tex_state::scaled::Scaled,
)> {
    match source {
        direct::ShipoutRoot::Page(Node::HList(node) | Node::VList(node)) => {
            Some((node.width, node.height, node.depth))
        }
        direct::ShipoutRoot::Page(_) => None,
    }
}

fn huge_shipout_box<G>(source: &direct::ShipoutRoot, stores: &Universe<G>) -> bool {
    let Some((width, height, depth)) = shipout_box_dimensions(source) else {
        return false;
    };
    height > tex_state::scaled::Scaled::MAX_DIMEN
        || depth > tex_state::scaled::Scaled::MAX_DIMEN
        || height
            .checked_add(depth)
            .and_then(|value| {
                value.checked_add(
                    stores
                        .dimen_param(DimenParam::V_OFFSET)
                        .expect("shipout reads admitted voffset"),
                )
            })
            .is_none_or(|value| value > tex_state::scaled::Scaled::MAX_DIMEN)
        || width
            .checked_add(
                stores
                    .dimen_param(DimenParam::H_OFFSET)
                    .expect("shipout reads admitted hoffset"),
            )
            .is_none_or(|value| value > tex_state::scaled::Scaled::MAX_DIMEN)
}
