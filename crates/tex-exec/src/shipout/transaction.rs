use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::node::{Node, NodeKind};
use tex_state::node_arena::{NodeRef, PageListId};
use tex_state::{
    ContentHash, DetachedArtifact, GeometryObservation, MemoTimingPhase, MemoValueLimits,
    PrintSink, PureMemoKey, PureMemoLayer, PureShipoutEntry, Universe,
};

use crate::ExecError;
use crate::dispatch::{CommittedPagePublication, PreparedDviPage};

use super::direct;
use super::{ShipoutOrigin, TextReplayHost, WriteReplayHost};

#[cfg(test)]
mod tests;

const SHIPOUT_EPISODE_DOMAIN: u32 = 4;
const SHIPOUT_EPISODE_SCHEMA: u32 = 1;
const SHIPOUT_ENV_HASH_DOMAIN: u64 = 0x7368_6970_656e_7601;

/// Resumes TeX82 §§530 and 1373--1375 after an authoritative output-open
/// failure retained the failed effect and its following suffix.
pub fn retry_unavailable_stream_open(
    stores: &mut Universe,
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

/// What the surrounding job already was when a `\shipout` began.
///
/// Both fields are boundaries between the job and the page: the §82 context
/// an `\openout` retry reports against, and the point in the live effect log
/// past which nothing belongs to the page. They travel together because a
/// caller that knows one always knows the other.
/// Ships a completed box using an already-owned publication summary.
///
/// Command replay has no independent source stack: it publishes the
/// most recently committed input summary while retaining command input in its
/// own state.  The direct artifact kernel needs only this detached summary,
/// never a source-consumption capability.
pub(crate) fn shipout_node_with_input_summary(
    node: Node,
    input_summary: tex_state::InputSummary,
    origin: ShipoutOrigin,
    stores: &mut Universe,
    emit_dvi: bool,
    write_expander: &mut direct::WriteExpander<'_>,
    replay_expander: &mut direct::ReplayTextExpander<'_>,
) -> Result<Option<CommittedPagePublication>, ExecError> {
    let pending_end = origin.pending_end;
    prepare_pdf_output_policy(stores)?;
    let page_before_shipout = stores.page_node_cursor();
    let page_root = stores.publish_page_nodes(std::slice::from_ref(&node));
    let shipout_scratch = stores.page_node_cursor();
    let geometry = shipout_geometry(&node, stores);
    if huge_shipout_box(&node, stores) {
        // TeX.web §641 drops the page rather than emitting it, so the report
        // is the whole of the engine's response. Shipout also runs from
        // command replay, which owns no live source stack. Its caller
        // captured §82's display from the command-owned stack before
        // releasing that borrow; the Universe summary is only republished
        // later by successful artifact staging and can still name an older
        // input position here.
        let context = shipout_error_context(stores, &input_summary, &origin);
        let reported = crate::error_report::report_error(
            stores,
            "Huge page cannot be shipped out",
            &[
                "The page just created is more than 18 feet tall or",
                "more than 18 feet wide, so I suspect something went wrong.",
            ],
            context,
        );
        if let Err(error) = reported {
            stores
                .truncate_page_nodes(page_before_shipout)
                .expect("aborted huge-page report restores its speculative root");
            return Err(error);
        }
        report_huge_page_deleted_box(
            stores,
            page_root,
            stores.int_param(IntParam::TRACING_OUTPUT),
        );
        stores
            .release_completed_page(page_root)
            .expect("discarded page root is exclusively owned");
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
        && effect_free_shipout_graph(stores, &node)
        && stores.world().effect_records()[..pending_end].is_empty()
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
                entry.render_origin_ends,
                entry.render_provenance,
                None,
            );
            let (hash, artifact, publication) = match replayed {
                Ok(replayed) => replayed,
                Err(error) => {
                    stores
                        .truncate_page_nodes(page_before_shipout)
                        .expect("failed memo replay restores its speculative page root");
                    return Err(error);
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
                stores.record_geometry_observation(geometry);
            }
            stores
                .release_completed_page(page_root)
                .expect("memo-replayed page root is exclusively owned");
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
        input_summary,
        origin,
        &mut transaction,
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
    let memo_payload =
        (key.is_some() && !staged.artifact.has_deferred_render_origins()).then(|| {
            let artifact_bytes = staged.artifact.bytes().to_vec();
            let render_origin_ends = staged.artifact.render_origin_ends_for_memo().to_vec();
            let render_origins = staged
                .artifact
                .render_origins_for_memo()
                .iter()
                .flat_map(|origins| origins.iter().copied())
                .collect::<Vec<_>>();
            (artifact_bytes, render_origin_ends, render_origins)
        });
    let reservation = transaction
        .world_mut()
        .reserve_active_artifact_publication_at(effect_start, None);
    let effect_end = transaction.world().effect_records().len();
    let committed = transaction.commit(staged.artifact, staged.effect_pos, reservation);
    let (hash, publication) = match committed {
        Ok(committed) => committed,
        Err(error) => {
            stores
                .truncate_page_nodes(page_before_shipout)
                .expect("rejected shipout restores its entire speculative suffix");
            return Err(error);
        }
    };
    stores
        .truncate_page_nodes(shipout_scratch)
        .expect("shipout normalization restores its scratch suffix");
    stores
        .release_completed_page(page_root)
        .expect("committed page root is exclusively owned");
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
        stores.record_geometry_observation(geometry);
    }
    for (sink, text) in retained_diagnostics {
        stores.world_mut().write_text(sink, &text);
    }
    if let (Some(key), Some((artifact_bytes, render_origin_ends, render_origins))) =
        (key, memo_payload)
        && stores.world().effect_pos() == effect_pos_start
        && let Ok(artifact) = tex_state::DetachedMemoValue::from_artifact(&DetachedArtifact {
            artifact_schema: 10,
            payload: artifact_bytes,
        })
        && let Some(render_provenance) =
            crate::output_provenance::provenance_recipe_for_origins(stores, render_origins)
    {
        stores.with_pure_memo(|memo| {
            memo.insert_shipout(
                key,
                PureShipoutEntry {
                    artifact,
                    render_origin_ends,
                    render_provenance,
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

/// TeX82 §641's huge-page recovery tail.
///
/// Positive `\tracingoutput` has already displayed the page at §638. At
/// zero or below, `ship_out` must identify and display the rejected box here
/// before its caller prints the closing page marker.
fn report_huge_page_deleted_box(stores: &mut Universe, page_root: PageListId, tracing_output: i32) {
    if tracing_output > 0 {
        return;
    }
    let dump = crate::node_dump::dump_page_list(
        stores,
        page_root,
        crate::node_dump::DumpConfig::read(stores),
    );
    let mut diagnostic = stores.begin_diagnostic();
    diagnostic
        .print_nl("The following box has been deleted:")
        .print_ln()
        .print_rendered(&dump);
    diagnostic.end(true);
}

fn shipout_error_context(
    stores: &Universe,
    input_summary: &tex_state::InputSummary,
    origin: &ShipoutOrigin,
) -> String {
    origin
        .output_open_context
        .clone()
        .unwrap_or_else(|| crate::diagnostics::show_context(stores, input_summary))
}

pub(crate) fn stage_page(
    node: Node,
    input_summary: tex_state::InputSummary,
    origin: ShipoutOrigin,
    stores: &mut Universe,
    emit_dvi: bool,
    write_expander: &mut WriteReplayHost<'_>,
    replay_expander: &mut TextReplayHost<'_>,
) -> Result<Option<CommittedPagePublication>, ExecError> {
    shipout_node_with_input_summary(
        node,
        input_summary,
        origin,
        stores,
        emit_dvi,
        write_expander,
        replay_expander,
    )
}

pub(crate) fn stage_form(
    form: tex_state::PdfFormRecord,
    stores: &mut Universe,
    write_expander: &mut WriteReplayHost<'_>,
    replay_expander: &mut TextReplayHost<'_>,
) -> Result<tex_state::PdfFormArtifact, ExecError> {
    direct::stage_form(form, stores, write_expander, replay_expander)
}

fn shipout_geometry(node: &Node, stores: &Universe) -> Option<GeometryObservation> {
    let (Node::HList(node) | Node::VList(node)) = node else {
        return None;
    };
    Some(GeometryObservation::Shipout {
        page_width_sp: i64::from(node.width.raw()),
        page_height_sp: i64::from(node.height.raw()) + i64::from(node.depth.raw()),
        counts: direct::page_counts(stores),
        line: stores.current_input_line().max(0) as u32,
        source: stores.current_input_source(),
    })
}

fn prepare_pdf_output_policy(stores: &mut Universe) -> Result<(), ExecError> {
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
        )?;
        stores.set_int_param(IntParam::PDF_MAJOR_VERSION, 1);
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
        )?;
        stores.set_int_param(IntParam::PDF_MINOR_VERSION, 4);
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
/// was scanned, so §82's context is whatever input the job last published.
fn report_invalid_pdf_version(
    stores: &mut Universe,
    message: &str,
    help: &[&str],
    value: i32,
) -> Result<(), ExecError> {
    let context = crate::diagnostics::show_context(stores, stores.input_summary());
    let mut report = stores.print_err(message);
    // pdftex.web breaks the line before the value; `print_nl` on an open line
    // is that `print_ln`.
    report.print_nl("").help(help).context(context);
    report.int_error(value).jump_out()?;
    Ok(())
}

fn shipout_key(stores: &mut Universe, root: PageListId) -> PureMemoKey {
    let environment = stores.engine_boundary_hash(SHIPOUT_ENV_HASH_DOMAIN, |hash| {
        hash.page_node_list(stores, root);
        hash.i32(stores.int_param(IntParam::MAG));
        hash.i32(stores.dimen_param(DimenParam::H_OFFSET).raw());
        hash.i32(stores.dimen_param(DimenParam::V_OFFSET).raw());
        for index in 0..10 {
            hash.i32(stores.count(index));
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

fn effect_free_shipout_graph(stores: &Universe, root: &Node) -> bool {
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

fn huge_shipout_box(node: &Node, stores: &Universe) -> bool {
    let Some(box_node) = NodeRef::from(node).box_node() else {
        return false;
    };
    let (width, height, depth) = (box_node.width, box_node.height, box_node.depth);
    height > tex_state::scaled::Scaled::MAX_DIMEN
        || depth > tex_state::scaled::Scaled::MAX_DIMEN
        || height
            .checked_add(depth)
            .and_then(|value| value.checked_add(stores.dimen_param(DimenParam::V_OFFSET)))
            .is_none_or(|value| value > tex_state::scaled::Scaled::MAX_DIMEN)
        || width
            .checked_add(stores.dimen_param(DimenParam::H_OFFSET))
            .is_none_or(|value| value > tex_state::scaled::Scaled::MAX_DIMEN)
}
