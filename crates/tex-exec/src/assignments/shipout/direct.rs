use tex_expand::{
    ReadRecorder, get_x_token_with_recorder_and_context, scan_dimen::DimensionDiagnostic,
};
use tex_lex::{InputSource, InputStack, MemoryInput, TokenListReplayKind};
use tex_out::dvi::{DviPagePlan, DviPagePlanBuilder};
use tex_out::{
    BoxNode as PageBoxNode, ContentHash as PageContentHash, DEFAULT_BANNER,
    DiscKind as PageDiscKind, EffectSink, FontResource, GlueKind as PageGlueKind,
    GlueOrder as PageGlueOrder, GlueSign, GlueSpec as PageGlueSpec, JobInfo,
    KernKind as PageKernKind, LeaderPayload as PageLeaderPayload, PageEffect, PageNode, PageToken,
    TokenCatcode, V10ArtifactBuilder, V10NodeListWriter,
};
use tex_state::env::banks::DimenParam;
use tex_state::glue::Order;
use tex_state::ids::{FontId, NodeListId, TokenListId};
use tex_state::node::{
    BoxNode as StateBoxNode, Direction, DiscKind as StateDiscKind, GlueKind as StateGlueKind,
    KernKind as StateKernKind, LeaderPayload as StateLeaderPayload, Node, Sign, Whatsit,
};
use tex_state::node_arena::NodeRef;
use tex_state::token::{Catcode, Token};
use tex_state::{EffectRecord, PrintSink, Universe, VerifiedArtifact};

use crate::ExecError;
use crate::diagnostics;

const MAX_SHIPOUT_DEPTH: usize = 4096;

pub(super) struct StagedShipout {
    pub(super) artifact: VerifiedArtifact,
    pub(super) dvi_plan: DviPagePlan,
    pub(super) effect_pos: tex_state::EffectPos,
}

pub(super) fn stage_shipout<S, R>(
    node: Node,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
) -> Result<StagedShipout, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
{
    let pending_effects = pending_page_effects(stores.world().effect_records());
    let counts = page_counts(stores);
    let (mag, diagnostic) = stores.prepare_mag();
    if let Some(diagnostic) = diagnostic {
        diagnostics::report_dimension_diagnostic(stores, DimensionDiagnostic::from(diagnostic));
    }
    let job = JobInfo {
        mag,
        banner: DEFAULT_BANNER.to_owned(),
        h_offset: stores.dimen_param(DimenParam::H_OFFSET),
        v_offset: stores.dimen_param(DimenParam::V_OFFSET),
    };
    let (root, children, vertical) = match node {
        Node::HList(box_node) => (lower_box_header(&box_node), box_node.children, false),
        Node::VList(box_node) => (lower_box_header(&box_node), box_node.children, true),
        Node::Unset(_) => {
            return Err(ExecError::UnsupportedShipoutNode {
                node: "unset alignment",
            });
        }
        _ => {
            return Err(ExecError::UnsupportedShipoutNode {
                node: "non-box shipout root",
            });
        }
    };

    // Phase A is the only mutable pass. It executes deferred effects, freezes
    // math substitutions, and records the rare direction permutations.
    let overlay = normalize_page(children, pending_effects, stores, recorder)?;

    // Phase B holds only an immutable state view. One compact-list walk feeds
    // the canonical writer and DVI state machine together.
    let mut encoder = V10ArtifactBuilder::new(job.clone(), counts, &root, vertical);
    let mut dvi =
        DviPagePlanBuilder::new(job, counts, &root, vertical).map_err(invalid_artifact)?;
    let mut emission = EmissionState {
        fonts: Vec::new(),
        font_slots: Vec::new(),
        anchor: u32::try_from(overlay.pending_effect_count)
            .map_err(|_| ExecError::ArithmeticOverflow)?,
    };
    encoder.stream_root_nodes(|output| {
        emit_node_list(
            stores,
            &overlay,
            children,
            output,
            Some(&mut dvi),
            &mut emission,
            false,
            1,
        )
    })?;
    debug_assert_eq!(
        usize::try_from(emission.anchor).ok(),
        Some(overlay.effects.len()),
        "normalization and emission must anchor identical effects"
    );
    let artifact_bytes = encoder
        .finish(&emission.fonts, &overlay.effects)
        .map_err(invalid_artifact)?;
    let dvi_plan = dvi.finish(&emission.fonts).map_err(invalid_artifact)?;

    let input_summary = input.publication_summary(stores);
    stores.set_input_summary(input_summary);
    let effect_pos = stores.world().effect_pos();
    Ok(StagedShipout {
        artifact: VerifiedArtifact::new(artifact_bytes),
        dvi_plan,
        effect_pos,
    })
}

fn invalid_artifact(error: impl ToString) -> ExecError {
    ExecError::InvalidShipoutArtifact(error.to_string())
}

mod lower;
mod materialize;
mod normalize;

use lower::*;
use materialize::{emitted_list_is_empty, materialize_node_list};
use normalize::{PageOverlay, normalize_page};

struct EmissionState {
    fonts: Vec<FontResource>,
    font_slots: Vec<Option<u32>>,
    anchor: u32,
}

#[allow(clippy::too_many_arguments)]
fn emit_node_list(
    stores: &Universe,
    overlay: &PageOverlay,
    list: NodeListId,
    output: &mut V10NodeListWriter<'_>,
    mut dvi: Option<&mut DviPagePlanBuilder>,
    emission: &mut EmissionState,
    suppress_deferred_streams: bool,
    depth: usize,
) -> Result<(), ExecError> {
    check_depth(depth)?;
    if let Some(order) = permutation_for(overlay, list) {
        for &index in order {
            emit_index(
                stores,
                overlay,
                list,
                index,
                output,
                dvi.as_deref_mut(),
                emission,
                suppress_deferred_streams,
                depth,
            )?;
        }
        return Ok(());
    }

    let nodes = stores.nodes(list);
    let mut index = 0;
    while index < nodes.len() {
        if let Some(run) = nodes.char_run(index) {
            emit_char_run(stores, run, output, dvi.as_deref_mut(), emission)?;
            index += run.len();
        } else {
            emit_index(
                stores,
                overlay,
                list,
                index,
                output,
                dvi.as_deref_mut(),
                emission,
                suppress_deferred_streams,
                depth,
            )?;
            index += 1;
        }
    }
    Ok(())
}

fn emit_char_run(
    stores: &Universe,
    run: tex_state::node_arena::CharRun<'_>,
    output: &mut V10NodeListWriter<'_>,
    mut dvi: Option<&mut DviPagePlanBuilder>,
    emission: &mut EmissionState,
) -> Result<(), ExecError> {
    let font = run.font();
    let font_id = font_resource_id(stores, font, emission);
    let widths = stores.font_widths(font);
    let characters = stores.font_characters(font);
    if let Some(dvi) = dvi.as_deref_mut() {
        dvi.add_fonts(&emission.fonts).map_err(invalid_artifact)?;
    }
    for code in run.codes() {
        if characters[usize::from(code)].is_none() {
            return Err(ExecError::UnsupportedShipoutNode {
                node: "missing character metrics",
            });
        }
        let width = widths[usize::from(code)];
        output.char(font_id, u32::from(code), width)?;
        if let Some(dvi) = dvi.as_deref_mut() {
            dvi.char(font_id, u32::from(code), width)
                .map_err(invalid_artifact)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_index(
    stores: &Universe,
    overlay: &PageOverlay,
    list: NodeListId,
    index: usize,
    output: &mut V10NodeListWriter<'_>,
    mut dvi: Option<&mut DviPagePlanBuilder>,
    emission: &mut EmissionState,
    suppress_deferred_streams: bool,
    depth: usize,
) -> Result<(), ExecError> {
    if let Some(replacement) = math_substitution(overlay, list, index) {
        return emit_node_list(
            stores,
            overlay,
            replacement,
            output,
            dvi,
            emission,
            suppress_deferred_streams,
            depth + 1,
        );
    }
    let node = stores
        .nodes(list)
        .get(index)
        .expect("emission index belongs to the frozen list");
    match node {
        NodeRef::Char { font, ch } => {
            let (code, width) = glyph(stores, font, ch)?;
            let font_id = font_resource_id(stores, font, emission);
            output.char(font_id, u32::from(code), width)?;
            if let Some(dvi) = dvi.as_deref_mut() {
                dvi.add_fonts(&emission.fonts).map_err(invalid_artifact)?;
                dvi.char(font_id, u32::from(code), width)
                    .map_err(invalid_artifact)?;
            }
        }
        NodeRef::Lig { font, ch, orig } => {
            let (code, width) = glyph(stores, font, ch)?;
            let font_id = font_resource_id(stores, font, emission);
            output.lig(
                font_id,
                u32::from(code),
                orig.iter().map(|source| *source as u32),
                width,
            )?;
            if let Some(dvi) = dvi.as_deref_mut() {
                dvi.add_fonts(&emission.fonts).map_err(invalid_artifact)?;
                dvi.char(font_id, u32::from(code), width)
                    .map_err(invalid_artifact)?;
            }
        }
        NodeRef::Kern { amount, kind } => {
            output.kern(amount, lower_kern_kind(kind))?;
            if let Some(dvi) = dvi.as_deref_mut() {
                dvi.kern(amount).map_err(invalid_artifact)?;
            }
        }
        NodeRef::Glue { spec, kind, leader } => {
            let spec = lower_glue(stores.glue(spec));
            let kind = lower_glue_kind(kind);
            let leader = leader.cloned();
            emit_glue(
                stores, overlay, output, dvi, emission, spec, kind, leader, depth,
            )?;
        }
        NodeRef::Penalty(value) => output.penalty(value)?,
        NodeRef::Rule {
            width,
            height,
            depth,
        } => {
            output.rule(width, height, depth)?;
            if let Some(dvi) = dvi.as_deref_mut() {
                dvi.rule(width, height, depth).map_err(invalid_artifact)?;
            }
        }
        NodeRef::HList(box_node) | NodeRef::VList(box_node) => {
            let vertical = matches!(node, NodeRef::VList(_));
            emit_box(
                stores,
                overlay,
                output,
                dvi,
                emission,
                box_node,
                vertical,
                suppress_deferred_streams,
                depth,
            )?;
        }
        NodeRef::Unset(_) => {
            return Err(ExecError::UnsupportedShipoutNode {
                node: "unset alignment",
            });
        }
        NodeRef::Disc {
            kind,
            pre,
            post,
            replace,
        } => output.disc(lower_disc_kind(kind), |disc| {
            disc.pre(|nodes| {
                emit_node_list(
                    stores,
                    overlay,
                    pre,
                    nodes,
                    None,
                    emission,
                    suppress_deferred_streams,
                    depth + 1,
                )
            })?;
            disc.post(|nodes| {
                emit_node_list(
                    stores,
                    overlay,
                    post,
                    nodes,
                    None,
                    emission,
                    suppress_deferred_streams,
                    depth + 1,
                )
            })?;
            disc.replace(|nodes| {
                emit_node_list(
                    stores,
                    overlay,
                    replace,
                    nodes,
                    None,
                    emission,
                    suppress_deferred_streams,
                    depth + 1,
                )
            })
        })?,
        NodeRef::Mark { class, tokens } => output.mark_stream(class, |tokens_out| {
            for token in stores.tokens(tokens) {
                match *token {
                    Token::Char { ch, cat } => {
                        tokens_out.char(ch as u32, lower_token_catcode(cat))?;
                    }
                    Token::Cs(symbol) => {
                        tokens_out.control_sequence(stores.resolve(symbol))?;
                    }
                    Token::Param(slot) => tokens_out.param(slot)?,
                    Token::Frozen(_) => {
                        unreachable!("alignment sentinel escaped into shipout tokens")
                    }
                }
            }
            Ok::<(), ExecError>(())
        })?,
        NodeRef::Ins { class, content, .. } => output.insert(class, |nodes| {
            emit_node_list(
                stores,
                overlay,
                content,
                nodes,
                None,
                emission,
                suppress_deferred_streams,
                depth + 1,
            )
        })?,
        NodeRef::Whatsit(whatsit) => {
            if let Some(effect_index) =
                anchor_for_whatsit(whatsit, suppress_deferred_streams, &mut emission.anchor)?
            {
                output.whatsit_anchor(effect_index)?;
                if let Some(dvi) = dvi.as_deref_mut() {
                    dvi.whatsit(effect_index, &overlay.effects)
                        .map_err(invalid_artifact)?;
                }
            }
        }
        NodeRef::MathOn(width) => {
            output.math_on(width)?;
            if let Some(dvi) = dvi.as_deref_mut() {
                dvi.math(width).map_err(invalid_artifact)?;
            }
        }
        NodeRef::MathOff(width) => {
            output.math_off(width)?;
            if let Some(dvi) = dvi {
                dvi.math(width).map_err(invalid_artifact)?;
            }
        }
        NodeRef::Direction(_) => {}
        NodeRef::Adjust(content) => output.adjust(|nodes| {
            emit_node_list(
                stores,
                overlay,
                content,
                nodes,
                None,
                emission,
                suppress_deferred_streams,
                depth + 1,
            )
        })?,
        NodeRef::MathList(_) => unreachable!("phase A records every math-list substitution"),
        NodeRef::MathNoad(_)
        | NodeRef::FractionNoad(_)
        | NodeRef::MathStyle(_)
        | NodeRef::MathChoice(_)
        | NodeRef::Nonscript => {
            return Err(ExecError::UnsupportedShipoutNode { node: "math" });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_box(
    stores: &Universe,
    overlay: &PageOverlay,
    output: &mut V10NodeListWriter<'_>,
    mut dvi: Option<&mut DviPagePlanBuilder>,
    emission: &mut EmissionState,
    box_node: StateBoxNode,
    vertical: bool,
    suppress_deferred_streams: bool,
    depth: usize,
) -> Result<(), ExecError> {
    let fields = lower_box_header(&box_node);
    let empty = emitted_list_is_empty(
        stores,
        overlay,
        box_node.children,
        suppress_deferred_streams,
        depth + 1,
    )?;
    let entered = if let Some(dvi) = dvi.as_deref_mut() {
        dvi.begin_box(&fields, vertical, empty)
            .map_err(invalid_artifact)?
    } else {
        false
    };
    output.box_node(vertical, &fields, |nodes| {
        emit_node_list(
            stores,
            overlay,
            box_node.children,
            nodes,
            dvi.as_deref_mut().filter(|_| entered),
            emission,
            suppress_deferred_streams,
            depth + 1,
        )
    })?;
    if entered {
        dvi.expect("entered DVI box has a builder")
            .end_box()
            .map_err(invalid_artifact)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_glue(
    stores: &Universe,
    overlay: &PageOverlay,
    output: &mut V10NodeListWriter<'_>,
    mut dvi: Option<&mut DviPagePlanBuilder>,
    emission: &mut EmissionState,
    spec: PageGlueSpec,
    kind: PageGlueKind,
    leader: Option<StateLeaderPayload>,
    depth: usize,
) -> Result<(), ExecError> {
    match leader {
        None => {
            output.glue(spec, kind)?;
            if let Some(dvi) = dvi.as_deref_mut() {
                dvi.glue(spec).map_err(invalid_artifact)?;
            }
        }
        Some(StateLeaderPayload::Rule {
            width,
            height,
            depth,
        }) => {
            output.glue_rule_leader(spec, kind, width, height, depth)?;
            if let Some(dvi) = dvi.as_deref_mut() {
                let node = PageNode::Glue {
                    spec,
                    kind,
                    leader: Some(PageLeaderPayload::Rule {
                        width,
                        height,
                        depth,
                    }),
                };
                dvi.leader(&node, &overlay.effects)
                    .map_err(invalid_artifact)?;
            }
        }
        Some(StateLeaderPayload::HList(box_node)) | Some(StateLeaderPayload::VList(box_node)) => {
            let vertical = matches!(leader, Some(StateLeaderPayload::VList(_)));
            let fields = lower_box_header(&box_node);
            let anchor_before = emission.anchor;
            let materialized = if dvi.is_some() {
                let mut replay_anchor = anchor_before;
                let children = materialize_node_list(
                    stores,
                    overlay,
                    box_node.children,
                    emission,
                    &mut replay_anchor,
                    true,
                    depth + 1,
                )?;
                Some((children, replay_anchor))
            } else {
                None
            };
            output.glue_box_leader(spec, kind, vertical, &fields, |nodes| {
                emit_node_list(
                    stores,
                    overlay,
                    box_node.children,
                    nodes,
                    None,
                    emission,
                    true,
                    depth + 1,
                )
            })?;
            if let (Some(dvi), Some((children, replay_anchor))) = (dvi, materialized) {
                debug_assert_eq!(replay_anchor, emission.anchor);
                dvi.add_fonts(&emission.fonts).map_err(invalid_artifact)?;
                let leader_box = PageBoxNode { children, ..fields };
                let leader = if vertical {
                    PageLeaderPayload::VList(leader_box)
                } else {
                    PageLeaderPayload::HList(leader_box)
                };
                let node = PageNode::Glue {
                    spec,
                    kind,
                    leader: Some(leader),
                };
                dvi.leader(&node, &overlay.effects)
                    .map_err(invalid_artifact)?;
            }
        }
    }
    Ok(())
}

fn anchor_for_whatsit(
    whatsit: &Whatsit,
    suppress_deferred_streams: bool,
    anchor: &mut u32,
) -> Result<Option<u32>, ExecError> {
    let anchored = match whatsit {
        Whatsit::Language { .. } => false,
        Whatsit::OpenOut { .. } | Whatsit::CloseOut { .. } | Whatsit::DeferredWrite { .. } => {
            !suppress_deferred_streams
        }
        Whatsit::Special { .. } => true,
    };
    if !anchored {
        return Ok(None);
    }
    let index = *anchor;
    *anchor = anchor.checked_add(1).ok_or(ExecError::ArithmeticOverflow)?;
    Ok(Some(index))
}

fn permutation_for(overlay: &PageOverlay, list: NodeListId) -> Option<&[usize]> {
    overlay
        .directions
        .iter()
        .find(|entry| entry.list == list)
        .map(|entry| entry.order.as_slice())
}

fn math_substitution(overlay: &PageOverlay, list: NodeListId, index: usize) -> Option<NodeListId> {
    overlay
        .math
        .iter()
        .find(|entry| entry.list == list && entry.index == index)
        .map(|entry| entry.replacement)
}

fn font_resource_id(stores: &Universe, font: FontId, emission: &mut EmissionState) -> u32 {
    let slot = font.raw() as usize;
    if emission.font_slots.len() <= slot {
        emission.font_slots.resize(slot + 1, None);
    }
    if let Some(id) = emission.font_slots[slot] {
        return id;
    }
    let id = font.raw().saturating_sub(1);
    let loaded = stores.font(font);
    emission.fonts.push(FontResource {
        font_id: id,
        name: loaded.name().to_owned(),
        tfm_content_hash: PageContentHash::new(loaded.content_hash()),
        tfm_checksum: loaded.checksum(),
        design_size: loaded.design_size(),
        at_size: loaded.size(),
    });
    emission.font_slots[slot] = Some(id);
    id
}

fn glyph(
    stores: &Universe,
    font: FontId,
    ch: char,
) -> Result<(u8, tex_state::scaled::Scaled), ExecError> {
    let code = u8::try_from(ch as u32).map_err(|_| ExecError::UnsupportedShipoutNode {
        node: "non-TeX82 character",
    })?;
    let width = stores
        .font_char_metrics(font, code)
        .map(|metrics| metrics.width)
        .ok_or(ExecError::UnsupportedShipoutNode {
            node: "missing character metrics",
        })?;
    Ok((code, width))
}

fn check_depth(depth: usize) -> Result<(), ExecError> {
    if depth > MAX_SHIPOUT_DEPTH {
        return Err(ExecError::UnsupportedShipoutNode {
            node: "shipout nesting deeper than 4096",
        });
    }
    Ok(())
}
