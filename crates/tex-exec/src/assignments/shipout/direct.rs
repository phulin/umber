use tex_expand::{get_x_or_protected_with_context, scan_dimen::DimensionDiagnostic};
use tex_lex::{InputStack, TokenListReplayKind};
use tex_out::dvi::{DviPagePlan, DviPagePlanBuilder};
use tex_out::{
    BoxNode as PageBoxNode, ContentHash as PageContentHash, DEFAULT_BANNER,
    DiscKind as PageDiscKind, EffectSink, FontResource, FontResourceConstruction,
    GlueKind as PageGlueKind, GlueOrder as PageGlueOrder, GlueSign, GlueSpec as PageGlueSpec,
    JobInfo, KernKind as PageKernKind, LeaderPayload as PageLeaderPayload, PageEffect, PageNode,
    PageToken, TokenCatcode, V10ArtifactBuilder, V10NodeListWriter,
};
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::glue::Order;
use tex_state::ids::{FontId, NodeListId, TokenListId};
use tex_state::node::{
    BoxNode as StateBoxNode, Direction, DiscKind as StateDiscKind, GlueKind as StateGlueKind,
    KernKind as StateKernKind, LeaderPayload as StateLeaderPayload, Node, Sign, Whatsit,
};
use tex_state::node_arena::NodeRef;
use tex_state::token::{Catcode, OriginId, Token};
use tex_state::{EffectRecord, PrintSink, Universe, VerifiedArtifact};

use crate::ExecError;
use crate::diagnostics;

const MAX_SHIPOUT_DEPTH: usize = 4096;

pub(super) struct StagedShipout {
    pub(super) artifact: VerifiedArtifact,
    pub(super) dvi_plan: DviPagePlan,
    pub(super) effect_pos: tex_state::EffectPos,
}

pub(super) fn stage_shipout(
    node: Node,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<StagedShipout, ExecError> {
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
    let overlay = execution
        .with_nested(|expansion| normalize_page(children, pending_effects, stores, expansion))?;

    // Phase B holds only an immutable state view. One compact-list walk feeds
    // the canonical writer and DVI state machine together.
    let mut encoder = V10ArtifactBuilder::new(job.clone(), counts, &root, vertical);
    let mut dvi =
        DviPagePlanBuilder::new(job, counts, &root, vertical).map_err(invalid_artifact)?;
    let mut emission = EmissionState {
        fonts: Vec::new(),
        live_fonts: Vec::new(),
        font_slots: Vec::new(),
        // The artifact root is a synthetic box header preceding its children.
        render_origins: vec![Vec::new()],
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
    if stores.int_param(IntParam::PDF_OUTPUT) > 0 {
        for &font in &emission.live_fonts {
            stores
                .ensure_pdf_font_resource(font)
                .map_err(|_| ExecError::ArithmeticOverflow)?;
        }
    }
    let artifact_bytes = encoder
        .finish(&emission.fonts, &overlay.effects)
        .map_err(invalid_artifact)?;
    let dvi_plan = dvi.finish(&emission.fonts).map_err(invalid_artifact)?;

    let input_summary = input.publication_summary(stores);
    stores.set_input_summary(input_summary);
    let effect_pos = stores.world().effect_pos();
    Ok(StagedShipout {
        artifact: VerifiedArtifact::new(artifact_bytes)
            .with_render_origins(emission.render_origins),
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
    live_fonts: Vec<FontId>,
    font_slots: Vec<Option<u32>>,
    anchor: u32,
    render_origins: Vec<Vec<OriginId>>,
}

impl EmissionState {
    fn node(&mut self, origins: impl IntoIterator<Item = OriginId>) {
        self.render_origins.push(origins.into_iter().collect());
    }
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
    let widths = stores.font_widths(font);
    let characters = stores.font_characters(font);
    for (code, origin) in run.codes().zip(run.origins()) {
        if characters[usize::from(code)].is_none() {
            return Err(ExecError::UnsupportedShipoutNode {
                node: "missing character metrics",
            });
        }
        let width = widths[usize::from(code)];
        emit_glyph(
            stores,
            font,
            code,
            width,
            [origin],
            output,
            dvi.as_deref_mut(),
            emission,
        )?;
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
        NodeRef::Char { font, ch, origin } => {
            let (code, width) = glyph(stores, font, ch)?;
            emit_glyph(stores, font, code, width, [origin], output, dvi, emission)?;
        }
        NodeRef::Lig {
            font,
            ch,
            orig,
            origins,
        } => {
            let (code, width) = glyph(stores, font, ch)?;
            emit_ligature(
                stores,
                font,
                code,
                orig,
                width,
                origins.iter().copied(),
                output,
                dvi,
                emission,
            )?;
        }
        NodeRef::Kern { amount, kind } => {
            emission.node([]);
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
        NodeRef::Penalty(value) => {
            emission.node([]);
            output.penalty(value)?;
        }
        NodeRef::Rule {
            width,
            height,
            depth,
        } => {
            emission.node([]);
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
        } => {
            emission.node([]);
            output.disc(lower_disc_kind(kind), |disc| {
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
            })?;
        }
        NodeRef::Mark { class, tokens } => {
            emission.node([]);
            output.mark_stream(class, |tokens_out| {
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
            })?;
        }
        NodeRef::Ins { class, content, .. } => {
            emission.node([]);
            output.insert(class, |nodes| {
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
            })?;
        }
        NodeRef::Whatsit(whatsit) => {
            if let Some(effect_index) =
                anchor_for_whatsit(whatsit, suppress_deferred_streams, &mut emission.anchor)?
            {
                emission.node([]);
                output.whatsit_anchor(effect_index)?;
                if let Some(dvi) = dvi.as_deref_mut() {
                    dvi.whatsit(effect_index, &overlay.effects)
                        .map_err(invalid_artifact)?;
                }
            }
        }
        NodeRef::MathOn(width) => {
            emission.node([]);
            output.math_on(width)?;
            if let Some(dvi) = dvi.as_deref_mut() {
                dvi.math(width).map_err(invalid_artifact)?;
            }
        }
        NodeRef::MathOff(width) => {
            emission.node([]);
            output.math_off(width)?;
            if let Some(dvi) = dvi {
                dvi.math(width).map_err(invalid_artifact)?;
            }
        }
        NodeRef::Direction(_) => {}
        NodeRef::Adjust(content) => {
            emission.node([]);
            output.adjust(|nodes| {
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
            })?;
        }
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
    emission.node([]);
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
            emission.node([]);
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
            emission.node([]);
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
            emission.node([]);
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
        Whatsit::Language { .. } | Whatsit::PdfReferenceObject { .. } => false,
        Whatsit::OpenOut { .. } | Whatsit::CloseOut { .. } | Whatsit::DeferredWrite { .. } => {
            !suppress_deferred_streams
        }
        Whatsit::Special { .. }
        | Whatsit::PdfAccessibility(_)
        | Whatsit::PdfAnnotation { .. }
        | Whatsit::PdfLinkStart { .. }
        | Whatsit::PdfLinkEnd { .. }
        | Whatsit::PdfRunningLink(_)
        | Whatsit::PdfLiteral { .. }
        | Whatsit::DeferredPdfLiteral { .. }
        | Whatsit::PdfSetMatrix { .. }
        | Whatsit::PdfSave
        | Whatsit::PdfRestore => true,
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
    let logical_id = register_font_resource(stores, font, emission);
    match stores.font(font).construction() {
        tex_fonts::FontConstruction::Loaded => logical_id,
        tex_fonts::FontConstruction::Copied { source }
        | tex_fonts::FontConstruction::Letterspaced { source, .. }
        | tex_fonts::FontConstruction::Expanded { source, .. } => {
            let source = stores
                .font_by_source_identity(*source)
                .expect("validated generated font source is live");
            font_resource_id(stores, source, emission)
        }
    }
}

#[derive(Clone, Copy)]
struct GlyphProjection {
    font_id: u32,
    width: tex_state::scaled::Scaled,
    left: tex_state::scaled::Scaled,
    right: tex_state::scaled::Scaled,
}

fn glyph_projection(
    stores: &Universe,
    font: FontId,
    code: u8,
    logical_width: tex_state::scaled::Scaled,
    emission: &mut EmissionState,
) -> Result<GlyphProjection, ExecError> {
    let font_id = font_resource_id(stores, font, emission);
    let loaded = stores.font(font);
    let tex_fonts::FontConstruction::Letterspaced { source, amount, .. } = loaded.construction()
    else {
        return Ok(GlyphProjection {
            font_id,
            width: logical_width,
            left: tex_state::scaled::Scaled::from_raw(0),
            right: tex_state::scaled::Scaled::from_raw(0),
        });
    };
    let source_font = stores
        .font_by_source_identity(*source)
        .expect("validated letterspaced font source is live");
    let source_width = stores
        .font_char_metrics(source_font, code)
        .map(|metrics| metrics.width)
        .ok_or(ExecError::UnsupportedShipoutNode {
            node: "missing letterspace source character metrics",
        })?;
    let quad = stores.font(source_font).parameters()[5];
    let left = round_scaled_ratio(quad, i32::from(*amount), 2000)?;
    let right = logical_width
        .checked_sub(source_width)
        .and_then(|difference| difference.checked_sub(left))
        .ok_or(ExecError::ArithmeticOverflow)?;
    Ok(GlyphProjection {
        font_id,
        width: source_width,
        left,
        right,
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_glyph(
    stores: &Universe,
    font: FontId,
    code: u8,
    logical_width: tex_state::scaled::Scaled,
    origins: impl IntoIterator<Item = OriginId>,
    output: &mut V10NodeListWriter<'_>,
    mut dvi: Option<&mut DviPagePlanBuilder>,
    emission: &mut EmissionState,
) -> Result<(), ExecError> {
    let projection = glyph_projection(stores, font, code, logical_width, emission)?;
    emit_projection_kern(projection.left, output, dvi.as_deref_mut(), emission)?;
    emission.node(origins);
    output.char(projection.font_id, u32::from(code), projection.width)?;
    if let Some(dvi) = dvi.as_deref_mut() {
        dvi.add_fonts(&emission.fonts).map_err(invalid_artifact)?;
        dvi.char(projection.font_id, u32::from(code), projection.width)
            .map_err(invalid_artifact)?;
    }
    emit_projection_kern(projection.right, output, dvi, emission)
}

#[allow(clippy::too_many_arguments)]
fn emit_ligature(
    stores: &Universe,
    font: FontId,
    code: u8,
    source: &[char],
    logical_width: tex_state::scaled::Scaled,
    origins: impl IntoIterator<Item = OriginId>,
    output: &mut V10NodeListWriter<'_>,
    mut dvi: Option<&mut DviPagePlanBuilder>,
    emission: &mut EmissionState,
) -> Result<(), ExecError> {
    let projection = glyph_projection(stores, font, code, logical_width, emission)?;
    emit_projection_kern(projection.left, output, dvi.as_deref_mut(), emission)?;
    emission.node(origins);
    output.lig(
        projection.font_id,
        u32::from(code),
        source.iter().map(|source| *source as u32),
        projection.width,
    )?;
    if let Some(dvi) = dvi.as_deref_mut() {
        dvi.add_fonts(&emission.fonts).map_err(invalid_artifact)?;
        dvi.char(projection.font_id, u32::from(code), projection.width)
            .map_err(invalid_artifact)?;
    }
    emit_projection_kern(projection.right, output, dvi, emission)
}

fn emit_projection_kern(
    amount: tex_state::scaled::Scaled,
    output: &mut V10NodeListWriter<'_>,
    dvi: Option<&mut DviPagePlanBuilder>,
    emission: &mut EmissionState,
) -> Result<(), ExecError> {
    if amount.raw() == 0 {
        return Ok(());
    }
    emission.node([]);
    output.kern(amount, PageKernKind::Explicit)?;
    if let Some(dvi) = dvi {
        dvi.kern(amount).map_err(invalid_artifact)?;
    }
    Ok(())
}

fn round_scaled_ratio(
    value: tex_state::scaled::Scaled,
    numerator: i32,
    denominator: i32,
) -> Result<tex_state::scaled::Scaled, ExecError> {
    let product = i64::from(value.raw()) * i64::from(numerator);
    let denominator = i64::from(denominator);
    let rounded = if product >= 0 {
        (product + denominator / 2) / denominator
    } else {
        -((-product + denominator / 2) / denominator)
    };
    Ok(tex_state::scaled::Scaled::from_raw(
        i32::try_from(rounded).map_err(|_| ExecError::ArithmeticOverflow)?,
    ))
}

fn register_font_resource(stores: &Universe, font: FontId, emission: &mut EmissionState) -> u32 {
    let slot = font.raw() as usize;
    if emission.font_slots.len() <= slot {
        emission.font_slots.resize(slot + 1, None);
    }
    if let Some(id) = emission.font_slots[slot] {
        return id;
    }
    let id = font.raw().saturating_sub(1);
    let loaded = stores.font(font);
    let construction = match loaded.construction() {
        tex_fonts::FontConstruction::Loaded => FontResourceConstruction::Loaded,
        tex_fonts::FontConstruction::Copied { source } => {
            let source_font = stores
                .font_by_source_identity(*source)
                .expect("validated copied font source is live");
            FontResourceConstruction::Copied {
                source_font_id: register_font_resource(stores, source_font, emission),
                source_identity: *source,
            }
        }
        tex_fonts::FontConstruction::Letterspaced {
            source,
            amount,
            no_ligatures,
        } => {
            let source_font = stores
                .font_by_source_identity(*source)
                .expect("validated letterspaced font source is live");
            FontResourceConstruction::Letterspaced {
                source_font_id: register_font_resource(stores, source_font, emission),
                source_identity: *source,
                amount: *amount,
                no_ligatures: *no_ligatures,
            }
        }
        tex_fonts::FontConstruction::Expanded { source, ratio } => {
            let source_font = stores
                .font_by_source_identity(*source)
                .expect("validated expanded font source is live");
            FontResourceConstruction::Expanded {
                source_font_id: register_font_resource(stores, source_font, emission),
                source_identity: *source,
                ratio: *ratio,
            }
        }
    };
    emission.fonts.push(FontResource {
        font_id: id,
        name: loaded.name().to_owned(),
        tfm_content_hash: PageContentHash::new(loaded.content_hash()),
        tfm_checksum: loaded.checksum(),
        design_size: loaded.design_size(),
        at_size: loaded.size(),
        opentype: loaded.opentype().map(|font| tex_out::OpenTypeFontResource {
            program_identity: font.program_identity,
            object_identity: font.object_identity,
            instance_identity: font.instance_identity,
            container: font.container,
        }),
        semantic_identity: loaded.source_identity(),
        construction,
    });
    emission.live_fonts.push(font);
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
