use tex_lex::InputStack;
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::node::Node;
use tex_state::token::TracedTokenWord;
use tex_state::{
    ContentHash, DetachedArtifact, MemoTimingPhase, MemoValueLimits, PrintSink, PureMemoKey,
    PureMemoLayer, PureShipoutEntry, Universe,
};

use super::scan_box_value_node;
use crate::ExecError;
use crate::dispatch::PreparedDviPage;

mod direct;

const SHIPOUT_EPISODE_DOMAIN: u32 = 4;
const SHIPOUT_EPISODE_SCHEMA: u32 = 1;
const SHIPOUT_ENV_HASH_DOMAIN: u64 = 0x7368_6970_656e_7601;

// TeX82 map: `ship_out` consumes a box whose child list is visited by
// `hlist_out`/`vlist_out`. Fresh pages use the direct two-phase emitter in
// `direct`: mutation and rare-node normalization finish first, then one live
// compact-list traversal writes canonical artifact bytes and DVI plan bytes.
// No detached node tree or per-list snapshot crosses that traversal.

pub(super) fn execute_shipout(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<PreparedDviPage>, ExecError> {
    let Some(node) = scan_box_value_node(input, stores, execution, context)? else {
        return Ok(None);
    };
    shipout_node(node, input, stores, execution)
}

pub(crate) fn shipout_node(
    node: Node,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<PreparedDviPage>, ExecError> {
    prepare_pdf_output_policy(stores)?;
    if huge_shipout_box(&node, stores) {
        stores.world_mut().write_text(
            PrintSink::TerminalAndLog,
            "\n! Huge page cannot be shipped out.\nThe page just created is more than 18 feet tall or\nmore than 18 feet wide, so I suspect something went wrong.\n",
        );
        return Ok(None);
    }
    if stores.pure_memo_enabled() && !stores.shipout_memo_enabled() {
        stores.record_pure_memo_not_attempted(PureMemoLayer::Shipout);
    }
    let cacheable = stores.shipout_memo_enabled()
        && effect_free_shipout_graph(stores, &node)
        && stores.world().effect_records().is_empty()
        && (1..=32_768).contains(&stores.int_param(IntParam::MAG));
    let validation_started = crate::timing::TelemetryTimer::start();
    let key = cacheable.then(|| shipout_key(stores, &node));
    if cacheable {
        stores.record_pure_memo_timing(
            PureMemoLayer::Shipout,
            MemoTimingPhase::Validation,
            validation_started.elapsed(),
        );
    }
    if !cacheable {
        stores.record_pure_shipout_barrier();
    }
    if let Some(key) = key
        && let Some(entry) = stores.lookup_pure_shipout(key)
    {
        let import_started = crate::timing::TelemetryTimer::start();
        let detached = entry.artifact.artifact(MemoValueLimits::default());
        if let Ok(detached) = detached {
            let imported_bytes = entry.artifact.retained_bytes();
            stores.commit_replayed_artifact(
                detached.payload,
                entry.render_origin_ends,
                entry.render_provenance,
            )?;
            stores.record_pure_memo_timing(
                PureMemoLayer::Shipout,
                MemoTimingPhase::Import,
                import_started.elapsed(),
            );
            stores.record_pure_shipout_hit(imported_bytes);
            return Ok(None);
        }
        stores.record_pure_memo_timing(
            PureMemoLayer::Shipout,
            MemoTimingPhase::Import,
            import_started.elapsed(),
        );
        stores.reject_pure_memo(key);
    }
    let effect_start = stores.world().effect_records().len();
    let mut transaction = stores.begin_shipout();
    let staged = direct::stage_shipout(node, input, &mut transaction, execution)?;
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
    let hash = transaction.commit(staged.artifact, staged.effect_pos)?;
    for (sink, text) in retained_diagnostics {
        stores.world_mut().write_text(sink, &text);
    }
    if let (Some(key), Some((artifact_bytes, render_origin_ends, render_origins))) =
        (key, memo_payload)
        && stores.world().effect_records().len() == effect_start
        && let Ok(artifact) = tex_state::DetachedMemoValue::from_artifact(&DetachedArtifact {
            artifact_schema: 10,
            payload: artifact_bytes,
        })
    {
        let render_provenance =
            crate::paragraph_memo::provenance_recipe_for_origins(stores, render_origins);
        stores.insert_pure_shipout(
            key,
            PureShipoutEntry {
                artifact,
                render_origin_ends,
                render_provenance,
            },
        );
    }
    Ok(Some(PreparedDviPage {
        hash,
        plan: staged.dvi_plan,
    }))
}

pub(super) fn stage_pdf_form(
    form: tex_state::PdfFormRecord,
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
) -> Result<tex_state::PdfFormArtifact, ExecError> {
    direct::stage_form(form, stores, expansion)
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
        stores.world_mut().write_text(
            PrintSink::TerminalAndLog,
            "\n! pdfTeX error (invalid pdfmajorversion).\nThe pdfmajorversion must be 1 or greater.\nI changed this to 1.\n",
        );
        stores.set_int_param(IntParam::PDF_MAJOR_VERSION, 1);
    }
    let minor = stores.int_param(IntParam::PDF_MINOR_VERSION);
    if !(0..=9).contains(&minor) {
        stores.world_mut().write_text(
            PrintSink::TerminalAndLog,
            "\n! pdfTeX error (invalid pdfminorversion).\nThe pdfminorversion must be between 0 and 9.\nI changed this to 4.\n",
        );
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

fn shipout_key(stores: &mut Universe, node: &Node) -> PureMemoKey {
    let root = stores.freeze_node_list(std::slice::from_ref(node));
    let environment = stores.engine_boundary_hash(SHIPOUT_ENV_HASH_DOMAIN, |hash| {
        hash.node_list(root);
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
        let children = match node {
            Node::HList(box_node) | Node::VList(box_node) => Some(box_node.children),
            Node::Glue {
                leader:
                    Some(
                        tex_state::node::LeaderPayload::HList(box_node)
                        | tex_state::node::LeaderPayload::VList(box_node),
                    ),
                ..
            } => Some(box_node.children),
            Node::Disc {
                pre, post, replace, ..
            } => {
                nodes.extend(stores.nodes(pre).into_iter().map(|node| node.to_owned()));
                nodes.extend(stores.nodes(post).into_iter().map(|node| node.to_owned()));
                Some(replace)
            }
            Node::Whatsit(_)
            | Node::Unset(_)
            | Node::Ins { .. }
            | Node::Direction(_)
            | Node::MathNoad(_)
            | Node::FractionNoad(_)
            | Node::MathStyle(_)
            | Node::MathChoice(_)
            | Node::MathList(_)
            | Node::Nonscript
            | Node::Adjust(_) => return false,
            _ => None,
        };
        if let Some(children) = children {
            nodes.extend(
                stores
                    .nodes(children)
                    .into_iter()
                    .map(|node| node.to_owned()),
            );
        }
    }
    true
}

fn huge_shipout_box(node: &Node, stores: &Universe) -> bool {
    let (width, height, depth) = match node {
        Node::HList(box_node) | Node::VList(box_node) => {
            (box_node.width, box_node.height, box_node.depth)
        }
        _ => return false,
    };
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
