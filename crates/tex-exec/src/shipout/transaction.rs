use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::node::{Node, NodeKind};
use tex_state::node_arena::{NodeRef, PageListId};
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
pub(crate) fn shipout_node<G>(
    node: Node,
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
    prepare_pdf_output_policy(stores, &origin.output_open_context)?;
    let page_before_shipout = stores.page_node_cursor();
    let page_root = stores.publish_page_nodes(std::slice::from_ref(&node));
    let shipout_scratch = stores.page_node_cursor();
    let geometry = shipout_geometry(&node, stores);
    if huge_shipout_box(&node, stores) {
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
                "Huge page cannot be shipped out",
                &[
                    "The page just created is more than 18 feet tall or",
                    "more than 18 feet wide, so I suspect something went wrong.",
                ],
                context,
            )
        };
        if let Err(error) = reported {
            stores
                .truncate_page_nodes(page_before_shipout)
                .expect("aborted huge-page report restores its speculative root");
            return Err(error);
        }
        report_huge_page_deleted_box(
            stores,
            diagnostic_effects,
            page_root,
            stores.int_param(IntParam::TRACING_OUTPUT),
        );
        release_published_page(stores, shipout_scratch, page_root);
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
        && effect_free_shipout_graph(stores, &node)
        && stores.world().effect_records()[..pending_effect_end].is_empty()
        && (1..=32_768).contains(&stores.int_param(IntParam::MAG));
    let validation_started = crate::timing::TelemetryTimer::start();
    let key = cacheable.then(|| shipout_key(stores, page_root));
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
                    stores
                        .truncate_page_nodes(page_before_shipout)
                        .expect("failed memo replay restores its speculative page root");
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
            release_published_page(stores, shipout_scratch, page_root);
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
        node,
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
    let staged = match staged {
        Ok(staged) => staged,
        Err(error) => {
            drop(transaction);
            stores
                .truncate_page_nodes(page_before_shipout)
                .expect("failed shipout restores its entire speculative suffix");
            return Err(error);
        }
    };
    let committed_effects = transaction.world().effect_records()[effect_start..]
        .to_vec()
        .into_boxed_slice();
    let retained_diagnostics = staged.retained_diagnostics.clone();
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
            stores
                .truncate_page_nodes(page_before_shipout)
                .expect("rejected shipout restores its entire speculative suffix");
            return Err(error.into());
        }
    };
    release_published_page(stores, shipout_scratch, page_root);
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

/// Drops exactly the completed page closure after publication has succeeded.
///
/// Normalization scratch is always newer than the published root, so it must
/// be truncated first. The root then owns the complete remaining closure and
/// can be released without disturbing older, unrelated page rows.
fn release_published_page<G>(
    stores: &mut Universe<G>,
    shipout_scratch: tex_state::node_arena::NodeArenaCursor<tex_state::node_arena::PageLifetime>,
    page_root: PageListId,
) {
    stores
        .truncate_page_nodes(shipout_scratch)
        .expect("shipout restores only its normalization scratch suffix");
    stores
        .release_completed_page(page_root)
        .expect("completed page root is exclusively owned");
}

/// TeX82 §641's huge-page recovery tail.
///
/// Positive `\tracingoutput` has already displayed the page at §638. At
/// zero or below, `ship_out` must identify and display the rejected box here
/// before its caller prints the closing page marker.
fn report_huge_page_deleted_box<G>(
    stores: &mut Universe<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    page_root: PageListId,
    tracing_output: i32,
) {
    if tracing_output > 0 {
        return;
    }
    let command = stores.command_context().expect("live generation");
    let dump = crate::node_dump::dump_page_list(
        &command,
        page_root,
        crate::node_dump::DumpConfig::read(&command),
    );
    let mut diagnostic = command.begin_diagnostic(diagnostic_effects);
    diagnostic
        .print_nl("The following box has been deleted:")
        .print_ln()
        .print_rendered(&dump);
    diagnostic.end(true);
}

pub(crate) fn stage_page<G>(
    node: Node,
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
        node,
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

fn shipout_geometry<G>(node: &Node, stores: &mut Universe<G>) -> Option<ShipoutGeometry> {
    let (Node::HList(node) | Node::VList(node)) = node else {
        return None;
    };
    let command = stores
        .command_context()
        .expect("shipout geometry runs inside an admitted command episode");
    Some(ShipoutGeometry {
        page_width_sp: i64::from(node.width.raw()),
        page_height_sp: i64::from(node.height.raw()) + i64::from(node.depth.raw()),
        counts: direct::page_counts(&command),
    })
}

fn prepare_pdf_output_policy<G>(
    stores: &mut Universe<G>,
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
    report.int_error(value).jump_out()?;
    Ok(())
}

fn shipout_key<G>(stores: &mut Universe<G>, root: PageListId) -> PureMemoKey {
    let environment = stores.engine_boundary_hash(SHIPOUT_ENV_HASH_DOMAIN, |hash| {
        hash.page_node_list(stores, root);
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
    let mut nodes = vec![root.clone()];
    while let Some(node) = nodes.pop() {
        let view = NodeRef::from(&node);
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
        node.visit_node_lists(|children| {
            nodes.extend(
                stores
                    .page_node_list(*children)
                    .expect("shipout child belongs to the live page arena")
                    .nodes()
                    .iter()
                    .cloned(),
            );
        });
    }
    true
}

fn huge_shipout_box<G>(node: &Node, stores: &Universe<G>) -> bool {
    let Some(box_node) = NodeRef::from(node).box_node() else {
        return false;
    };
    let (width, height, depth) = (box_node.width, box_node.height, box_node.depth);
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
