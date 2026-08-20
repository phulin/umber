use tex_command::DimensionDiagnostic;
use tex_out::dvi::{DviPagePlan, DviPagePlanCoEmitter};
use tex_out::{
    ArtifactEmitter, ArtifactNodeListEmitter, BoxNode as PageBoxNode,
    ContentHash as PageContentHash, DEFAULT_BANNER, DiscKind as PageDiscKind, EffectSink,
    FontResource, FontResourceConstruction, GlueKind as PageGlueKind, GlueOrder as PageGlueOrder,
    GlueSign, GlueSpec as PageGlueSpec, JobInfo, KernKind as PageKernKind,
    MarginKernSide as PageMarginKernSide, PageEffect, TokenCatcode,
};
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::glue::Order;
use tex_state::ids::{FontId, NodeListId, TokenListId};
use tex_state::node::{
    BoxNode as StateBoxNode, Direction, DiscKind as StateDiscKind, GlueKind as StateGlueKind,
    KernKind as StateKernKind, LeaderPayload as StateLeaderPayload,
    MarginKernSide as StateMarginKernSide, Node, Sign, Whatsit,
};
use tex_state::node_arena::{NodeList, NodeListRef, NodeRef};
use tex_state::provenance::OriginRef;
use tex_state::token::{Catcode, Token};
use tex_state::{EffectRecord, PrintSink, Universe, VerifiedArtifact};

use crate::ExecError;
use crate::diagnostics;

const MAX_SHIPOUT_DEPTH: usize = 4096;

pub(crate) type WriteExpander<'a> = dyn FnMut(&mut Universe, PrintSink, TokenListId) -> Result<crate::shipout::ExpandedWrite, ExecError>
    + 'a;

pub(crate) use crate::shipout::ReplayTextKind;

pub(crate) type ReplayTextExpander<'a> = dyn FnMut(
        &mut Universe,
        ReplayTextKind,
        TokenListId,
    ) -> Result<crate::shipout::ExpandedReplayText, ExecError>
    + 'a;

pub(crate) struct StagedShipout {
    pub(crate) artifact: VerifiedArtifact,
    pub(crate) dvi_plan: Option<DviPagePlan>,
    pub(crate) effect_pos: tex_state::EffectPos,
    pub(crate) retained_diagnostics: Vec<(PrintSink, String)>,
    #[cfg(test)]
    pub(crate) base_whatsit_visits: Vec<BaseWhatsitVisit>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BaseWhatsitVisitKind {
    OpenOut,
    DeferredWrite,
    NumberedCloseOut,
    FallbackCloseOut,
    Special,
    Language,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BaseWhatsitVisit {
    pub(crate) in_hlist: bool,
    pub(crate) position: usize,
    pub(crate) kind: BaseWhatsitVisitKind,
}

pub(crate) fn stage_form(
    form: tex_state::PdfFormRecord,
    stores: &mut Universe,
    write_expander: &mut WriteExpander<'_>,
    replay_expander: &mut ReplayTextExpander<'_>,
) -> Result<tex_state::PdfFormArtifact, ExecError> {
    let color_rollback = stores.pdf_form_color_rollback();
    let result = stage_form_inner(form, stores, write_expander, replay_expander);
    if result.is_err() {
        stores.rollback_pdf_form_colors(color_rollback);
    }
    result
}

fn stage_form_inner(
    form: tex_state::PdfFormRecord,
    stores: &mut Universe,
    write_expander: &mut WriteExpander<'_>,
    replay_expander: &mut ReplayTextExpander<'_>,
) -> Result<tex_state::PdfFormArtifact, ExecError> {
    let root_node = form
        .box_list_ref()
        .get(0)
        .ok_or(ExecError::PdfXFormVoidBox)?;
    let (root, children, vertical, box_lr) = match root_node {
        Node::HList(node) => (lower_box_header(&node), node.children, false, node.box_lr),
        Node::VList(node) => (lower_box_header(&node), node.children, true, node.box_lr),
        _ => return Err(ExecError::PdfXFormVoidBox),
    };
    let overlay = normalize_page(
        children.clone(),
        (vertical, box_lr),
        (
            PendingPageEffects {
                effects: Vec::new(),
                open_out_occurrences: Vec::new(),
            },
            String::new(),
            true,
        ),
        stores,
        write_expander,
        replay_expander,
        tex_state::PdfColorStackTarget::Form,
    )?;
    let job = JobInfo {
        mag: stores.prepared_mag().unwrap_or_else(|| stores.mag()),
        banner: DEFAULT_BANNER.to_owned(),
        h_offset: tex_state::scaled::Scaled::from_raw(0),
        v_offset: tex_state::scaled::Scaled::from_raw(0),
        page_origin_x: tex_state::scaled::Scaled::from_raw(0),
        page_origin_y: tex_state::scaled::Scaled::from_raw(0),
        page_width: tex_state::scaled::Scaled::from_raw(0),
        page_height: tex_state::scaled::Scaled::from_raw(0),
    };
    let mut encoder = ArtifactEmitter::new(job, [0; 10], &root, vertical);
    let mut emission = EmissionState {
        fonts: Vec::new(),
        live_fonts: Vec::new(),
        font_slots: Vec::new(),
        direct_font: None,
        direct_glyph: None,
        dvi_font_count: 0,
        render_origin_ends: None,
        render_origins: tex_state::RenderProvenanceBuilder::default(),
        anchor: 0,
    };
    let mut dvi_emitter = DviPagePlanCoEmitter::disabled();
    encoder.stream_root_nodes(|output| {
        emit_node_list(
            stores,
            &overlay,
            &children,
            output,
            &mut dvi_emitter,
            &mut emission,
            false,
            1,
        )
    })?;
    ensure_pdf_font_resources(stores, &emission.live_fonts)?;
    let bytes = encoder
        .finish(&emission.fonts, &overlay.effects)
        .map_err(invalid_artifact)?;
    let artifact = tex_out::PageArtifact::from_bytes(&bytes).map_err(invalid_artifact)?;
    let positioned =
        tex_out::positioned::lower_page(&artifact, 0).map_err(|error| match error {
            tex_out::positioned::PositionedError::UnmatchedPdfSaves { count } => {
                ExecError::InvalidShipoutArtifact(format!(
                    "pdfTeX error: {count} unmatched \\pdfsave after form shipout"
                ))
            }
            error => invalid_artifact(error),
        })?;
    let total = root
        .height
        .checked_add(root.depth)
        .ok_or(ExecError::ArithmeticOverflow)?;
    let convert = |(x, y): (tex_state::scaled::Scaled, tex_state::scaled::Scaled)| {
        total
            .checked_sub(y)
            .map(|y| (x, y))
            .ok_or(ExecError::ArithmeticOverflow)
    };
    let last_position = positioned.last_saved_position.map(convert).transpose()?;
    let snap_reference = convert(positioned.snap_reference)?;
    Ok(tex_state::PdfFormArtifact::new(
        bytes,
        last_position,
        snap_reference,
    ))
}

#[allow(clippy::too_many_arguments)] // Shipout staging capabilities remain explicit at this seam.
pub(crate) fn stage_shipout(
    node: Node,
    input_summary: tex_state::InputSummary,
    origin: super::ShipoutOrigin,
    stores: &mut Universe,
    emit_dvi: bool,
    write_expander: &mut WriteExpander<'_>,
    replay_expander: &mut ReplayTextExpander<'_>,
) -> Result<StagedShipout, ExecError> {
    let super::ShipoutOrigin {
        output_open_context,
        pending_end,
        announce_openout,
    } = origin;
    let pending_effects = pending_page_effects(stores.world(), pending_end);
    let counts = page_counts(stores);
    let (mag, diagnostic) = stores.prepare_mag();
    if let Some(diagnostic) = diagnostic {
        diagnostics::report_dimension_diagnostic(stores, DimensionDiagnostic::from(diagnostic));
    }
    const DVI_ONE_INCH: i32 = 4_736_286;
    let has_configured_page = stores.dimen_param(DimenParam::PDF_PAGE_WIDTH).raw() > 0
        || stores.dimen_param(DimenParam::PDF_PAGE_HEIGHT).raw() > 0;
    let (page_origin_x, page_origin_y) =
        if stores.int_param(IntParam::PDF_OUTPUT) > 0 || has_configured_page {
            (
                stores.dimen_param(DimenParam::PDF_H_ORIGIN),
                stores.dimen_param(DimenParam::PDF_V_ORIGIN),
            )
        } else {
            let inch = tex_state::scaled::Scaled::from_raw(DVI_ONE_INCH);
            (inch, inch)
        };
    let job = JobInfo {
        mag,
        banner: DEFAULT_BANNER.to_owned(),
        h_offset: stores.dimen_param(DimenParam::H_OFFSET),
        v_offset: stores.dimen_param(DimenParam::V_OFFSET),
        page_origin_x,
        page_origin_y,
        page_width: stores.dimen_param(DimenParam::PDF_PAGE_WIDTH),
        page_height: stores.dimen_param(DimenParam::PDF_PAGE_HEIGHT),
    };
    let (root, children, vertical, root_box_lr) = match node {
        Node::HList(box_node) => (
            lower_box_header(&box_node),
            box_node.children,
            false,
            box_node.box_lr,
        ),
        Node::VList(box_node) => (
            lower_box_header(&box_node),
            box_node.children,
            true,
            box_node.box_lr,
        ),
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
    let output_open_context = output_open_context
        .unwrap_or_else(|| crate::diagnostics::show_context(stores, &input_summary));
    let overlay = normalize_page(
        children.clone(),
        (vertical, root_box_lr),
        (pending_effects, output_open_context, announce_openout),
        stores,
        write_expander,
        replay_expander,
        tex_state::PdfColorStackTarget::Page,
    )?;
    if emit_dvi {
        reject_pdf_nodes_in_dvi(&overlay.effects)?;
    }

    // Phase B holds only an immutable state view. One compact-list walk emits
    // the canonical artifact; every downstream driver consumes those bytes.
    let mut dvi_emitter = DviPagePlanCoEmitter::new(
        job.clone(),
        counts,
        &root,
        vertical,
        &overlay.effects,
        emit_dvi,
    )
    .map_err(invalid_artifact)?;
    let mut encoder = ArtifactEmitter::new(job, counts, &root, vertical);
    let mut emission = EmissionState::page(
        stores.provenance_demand().rendered_source(),
        u32::try_from(overlay.pending_effect_count).map_err(|_| ExecError::ArithmeticOverflow)?,
    );
    encoder.stream_root_nodes(|output| {
        emit_node_list(
            stores,
            &overlay,
            &children,
            output,
            &mut dvi_emitter,
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
        ensure_pdf_font_resources(stores, &emission.live_fonts)?;
    }
    let artifact_bytes = encoder
        .finish(&emission.fonts, &overlay.effects)
        .map_err(invalid_artifact)?;
    let dvi_plan = dvi_emitter
        .finish(&emission.fonts, &artifact_bytes)
        .map_err(invalid_artifact)?;
    if needs_positioned_shipout(&overlay.effects) {
        let artifact =
            tex_out::PageArtifact::from_bytes(&artifact_bytes).map_err(invalid_artifact)?;
        let positioned =
            tex_out::positioned::lower_page_for_shipout(&artifact, 0).map_err(invalid_artifact)?;
        let last_position = positioned
            .last_saved_position
            .map(|position| saved_position(stores, &root, position))
            .transpose()?;
        stores.publish_pdf_traversal_positions(last_position, positioned.snap_reference);
    }

    stores.set_input_summary(input_summary);
    let effect_pos = stores.world().effect_pos();
    let retained_diagnostics = overlay.diagnostics.clone();
    let artifact = if let Some(render_origin_ends) = emission.render_origin_ends {
        VerifiedArtifact::new(artifact_bytes)
            .with_built_render_origins(render_origin_ends, emission.render_origins)
    } else {
        VerifiedArtifact::new(artifact_bytes)
    };
    Ok(StagedShipout {
        artifact: artifact.with_open_out_occurrences(overlay.open_out_occurrences),
        dvi_plan,
        effect_pos,
        retained_diagnostics,
        #[cfg(test)]
        base_whatsit_visits: overlay.base_whatsit_visits,
    })
}

fn reject_pdf_nodes_in_dvi(effects: &[PageEffect]) -> Result<(), ExecError> {
    let rejected = effects.iter().find_map(|effect| match effect {
        PageEffect::OpenOut { .. }
        | PageEffect::CloseOut { .. }
        | PageEffect::Write { .. }
        | PageEffect::Special { .. }
        | PageEffect::PdfSavePosition => None,
        PageEffect::PdfAccessibility(_) => Some("pdfextension"),
        PageEffect::PdfAnnotation(_) => Some("pdfannot"),
        PageEffect::PdfLiteral { .. } => Some("pdfliteral"),
        PageEffect::PdfSetMatrix { .. } => Some("pdfsetmatrix"),
        PageEffect::PdfSave => Some("pdfsave"),
        PageEffect::PdfRestore => Some("pdfrestore"),
        PageEffect::PdfColorStack { .. } => Some("pdfcolorstack"),
        PageEffect::PdfSnapState { .. } => Some("pdfsnaprefpoint"),
        PageEffect::PdfSnapRefPoint => Some("pdfsnaprefpoint"),
        PageEffect::PdfSnapY { .. } => Some("pdfsnapy"),
        PageEffect::PdfSnapYComp { .. } => Some("pdfsnapycomp"),
        PageEffect::PdfRefXForm { .. } => Some("pdfrefxform"),
        PageEffect::PdfRefXImage { .. } => Some("pdfrefximage"),
        PageEffect::PdfDestination(_) => Some("pdfdest"),
        PageEffect::PdfThread(_) => Some("pdfthread"),
        PageEffect::PdfStartThread(_) => Some("pdfstartthread"),
        PageEffect::PdfEndThread => Some("pdfendthread"),
    });
    match rejected {
        Some(name) => Err(ExecError::PdfDeferredNodeInDviMode(name)),
        None => Ok(()),
    }
}

fn needs_positioned_shipout(effects: &[PageEffect]) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            PageEffect::PdfSavePosition
                | PageEffect::PdfSnapRefPoint
                | PageEffect::PdfSnapY { .. }
                | PageEffect::PdfSnapYComp { .. }
        )
    })
}

fn ensure_pdf_font_resources(stores: &mut Universe, fonts: &[FontId]) -> Result<(), ExecError> {
    for &font in fonts {
        let first_use = stores.pdf_font_resource(font).is_none();
        stores
            .ensure_pdf_font_resource(font)
            .map_err(|_| ExecError::ArithmeticOverflow)?;
        if first_use && stores.int_param(IntParam::PDF_MOVE_CHARS) > 0 {
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\npdfTeX warning: Primitive \\pdfmovechars is obsolete.\n",
            );
            // pdfTeX performs a direct parameter write here. A local positive
            // assignment therefore restores its saved outer value at group
            // exit, while an ordinary/global value remains zero thereafter.
            stores.set_int_param(IntParam::PDF_MOVE_CHARS, 0);
        }
    }
    Ok(())
}

fn saved_position(
    stores: &Universe,
    root: &PageBoxNode,
    position: (tex_state::scaled::Scaled, tex_state::scaled::Scaled),
) -> Result<(tex_state::scaled::Scaled, tex_state::scaled::Scaled), ExecError> {
    const DVI_ONE_INCH: i32 = 4_736_286;
    if stores.int_param(IntParam::PDF_OUTPUT) > 0 {
        let h_origin = stores.dimen_param(DimenParam::PDF_H_ORIGIN);
        let v_origin = stores.dimen_param(DimenParam::PDF_V_ORIGIN);
        let configured_height = stores.dimen_param(DimenParam::PDF_PAGE_HEIGHT);
        let page_height = if configured_height.raw() == 0 {
            root.height
                .checked_add(root.depth)
                .and_then(|height| height.checked_add(v_origin))
                .and_then(|height| height.checked_add(v_origin))
                .ok_or(ExecError::ArithmeticOverflow)?
        } else {
            configured_height
        };
        Ok((
            position
                .0
                .checked_add(h_origin)
                .ok_or(ExecError::ArithmeticOverflow)?,
            page_height
                .checked_sub(position.1)
                .and_then(|value| value.checked_sub(v_origin))
                .ok_or(ExecError::ArithmeticOverflow)?,
        ))
    } else {
        let inch = tex_state::scaled::Scaled::from_raw(DVI_ONE_INCH);
        let page_height = root
            .height
            .checked_add(root.depth)
            .and_then(|height| height.checked_add(stores.dimen_param(DimenParam::V_OFFSET)))
            .ok_or(ExecError::ArithmeticOverflow)?;
        Ok((
            position
                .0
                .checked_add(inch)
                .ok_or(ExecError::ArithmeticOverflow)?,
            page_height
                .checked_sub(position.1)
                .and_then(|value| value.checked_sub(inch))
                .ok_or(ExecError::ArithmeticOverflow)?,
        ))
    }
}

fn invalid_artifact(error: impl ToString) -> ExecError {
    ExecError::InvalidShipoutArtifact(error.to_string())
}

pub(super) fn compile_dvi_plan(
    artifact_bytes: &[u8],
    emit_dvi: bool,
) -> Result<Option<DviPagePlan>, ExecError> {
    emit_dvi
        .then(|| DviPagePlan::compile_v10(artifact_bytes).map_err(invalid_artifact))
        .transpose()
}

mod lower;
pub(crate) use lower::page_counts;
mod normalize;
#[cfg(test)]
mod tests;

use lower::*;
use normalize::{PageOverlay, normalize_page};

pub(crate) fn terminal_output_name(line: &str) -> String {
    normalize::scan_terminal_output_name(line)
}

struct EmissionState {
    fonts: Vec<FontResource>,
    live_fonts: Vec<FontId>,
    font_slots: Vec<Option<u32>>,
    direct_font: Option<(FontId, u32, bool)>,
    direct_glyph: Option<(FontId, u8, tex_state::scaled::Scaled)>,
    dvi_font_count: usize,
    anchor: u32,
    render_origin_ends: Option<Vec<u32>>,
    render_origins: tex_state::RenderProvenanceBuilder,
}

impl EmissionState {
    fn page(rendered_source: bool, anchor: u32) -> Self {
        Self {
            fonts: Vec::new(),
            live_fonts: Vec::new(),
            font_slots: Vec::new(),
            direct_font: None,
            direct_glyph: None,
            dvi_font_count: 0,
            // The artifact root is a synthetic box header preceding its children.
            // Batch jobs retain no column at all; editor sessions select it once.
            render_origin_ends: rendered_source.then(|| vec![0]),
            render_origins: tex_state::RenderProvenanceBuilder::default(),
            anchor,
        }
    }

    fn node(&mut self, stores: &Universe, origins: impl IntoIterator<Item = OriginRef>) {
        let Some(ends) = &mut self.render_origin_ends else {
            return;
        };
        let mut len = 0_u32;
        for origin in origins {
            if let Some(span) = stores.root_span_for_origin(origin.id()) {
                self.render_origins.push_stable_span(span);
            } else {
                self.render_origins.push_root(origin);
            }
            len = len
                .checked_add(1)
                .expect("artifact render provenance exceeds u32 entries");
        }
        ends.push(
            ends.last()
                .copied()
                .unwrap_or(0)
                .checked_add(len)
                .expect("artifact render provenance exceeds u32 entries"),
        );
    }

    fn character_node(&mut self, stores: &Universe, origin: &OriginRef) {
        if self.render_origin_ends.is_some() {
            self.node(stores, [origin.clone()]);
        }
    }

    fn node_empty(&mut self) {
        let Some(ends) = &mut self.render_origin_ends else {
            return;
        };
        ends.push(ends.last().copied().unwrap_or(0));
    }

    fn sync_dvi_fonts(&mut self, dvi: &mut DviPagePlanCoEmitter) -> Result<(), ExecError> {
        if self.dvi_font_count != self.fonts.len() {
            dvi.add_fonts(&self.fonts).map_err(invalid_artifact)?;
            self.dvi_font_count = self.fonts.len();
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_node_list(
    stores: &Universe,
    overlay: &PageOverlay,
    list: &NodeListRef,
    output: &mut ArtifactNodeListEmitter<'_>,
    dvi: &mut DviPagePlanCoEmitter,
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
                dvi,
                emission,
                suppress_deferred_streams,
                depth,
            )?;
        }
        return Ok(());
    }

    let nodes = list.nodes();
    let unmodified = overlay.math.is_empty()
        && overlay.directions.is_empty()
        && overlay.omitted_whatsits.is_empty();
    let mut index = 0;
    while index < nodes.len() {
        if let Some(run) = nodes.char_run(index) {
            emit_char_run(stores, run, output, dvi, emission)?;
            index += run.len();
        } else if unmodified && let Some(NodeRef::Kern { amount, kind }) = nodes.get(index) {
            emission.node_empty();
            output.kern(amount, lower_kern_kind(kind))?;
            dvi.kern(amount).map_err(invalid_artifact)?;
            index += 1;
        } else {
            emit_index(
                stores,
                overlay,
                list,
                index,
                output,
                dvi,
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
    output: &mut ArtifactNodeListEmitter<'_>,
    dvi: &mut DviPagePlanCoEmitter,
    emission: &mut EmissionState,
) -> Result<(), ExecError> {
    let font = run.font();
    let loaded = stores.font(font);
    let (font_id, letterspaced) = if let Some((cached_font, font_id, letterspaced)) =
        emission.direct_font
        && cached_font == font
    {
        (font_id, letterspaced)
    } else {
        let letterspaced = matches!(
            loaded.construction(),
            tex_fonts::FontConstruction::Letterspaced { .. }
        );
        let font_id = font_resource_id(stores, font, emission);
        emission.direct_font = Some((font, font_id, letterspaced));
        (font_id, letterspaced)
    };
    if !letterspaced {
        for (code, origin) in run.codes().zip(run.origin_roots()) {
            let width = if let Some((cached_font, cached_code, width)) = emission.direct_glyph
                && cached_font == font
                && cached_code == code
            {
                width
            } else {
                let width = loaded
                    .character_metrics(char::from(code))
                    .map(|metrics| metrics.width)
                    .ok_or(ExecError::UnsupportedShipoutNode {
                        node: "missing character metrics",
                    })?;
                emission.direct_glyph = Some((font, code, width));
                width
            };
            emission.character_node(stores, origin);
            output.char(font_id, u32::from(code), width)?;
            emission.sync_dvi_fonts(dvi)?;
            dvi.char(font_id, u32::from(code), width)
                .map_err(invalid_artifact)?;
        }
        return Ok(());
    }

    for (code, origin) in run.codes().zip(run.origin_roots()) {
        let width = stores
            .font_character_metrics(font, char::from(code))
            .map(|metrics| metrics.width)
            .ok_or(ExecError::UnsupportedShipoutNode {
                node: "missing character metrics",
            })?;
        emit_glyph(
            stores,
            font,
            u32::from(code),
            width,
            [origin.clone()],
            output,
            dvi,
            emission,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_index(
    stores: &Universe,
    overlay: &PageOverlay,
    list: &NodeListRef,
    index: usize,
    output: &mut ArtifactNodeListEmitter<'_>,
    dvi: &mut DviPagePlanCoEmitter,
    emission: &mut EmissionState,
    suppress_deferred_streams: bool,
    depth: usize,
) -> Result<(), ExecError> {
    if omitted_whatsit(overlay, list, index) {
        return Ok(());
    }
    if let Some(replacement) = math_substitution(overlay, list, index) {
        return emit_node_list(
            stores,
            overlay,
            &replacement,
            output,
            dvi,
            emission,
            suppress_deferred_streams,
            depth + 1,
        );
    }
    let node = list
        .nodes()
        .get(index)
        .expect("emission index belongs to the frozen list");
    match node {
        NodeRef::Char {
            font,
            ch,
            origin_root,
            ..
        } => {
            let (code, width) = glyph(stores, font, ch)?;
            emit_glyph(
                stores,
                font,
                code,
                width,
                [origin_root.clone()],
                output,
                dvi,
                emission,
            )?;
        }
        NodeRef::Lig {
            font,
            ch,
            orig,
            origin_roots,
            ..
        } => {
            let (code, width) = glyph(stores, font, ch)?;
            emit_ligature(
                stores,
                font,
                code,
                orig,
                width,
                origin_roots.iter().cloned(),
                output,
                dvi,
                emission,
            )?;
        }
        NodeRef::Kern { amount, kind } => {
            emission.node(stores, []);
            output.kern(amount, lower_kern_kind(kind))?;
            dvi.kern(amount).map_err(invalid_artifact)?;
        }
        NodeRef::MarginKern {
            amount,
            side,
            font,
            ch,
        } => {
            let (code, width) = glyph(stores, font, char::from(ch))?;
            let projection = glyph_projection(stores, font, code, width, emission)?;
            emission.node(stores, []);
            output.margin_kern(amount, lower_margin_kern_side(side), projection.font_id, ch)?;
            dvi.kern(amount).map_err(invalid_artifact)?;
        }
        NodeRef::Glue { spec, kind, leader } => {
            let spec = lower_glue(stores.glue_spec(*spec));
            let kind = lower_glue_kind(kind);
            emit_glue(
                stores, overlay, output, dvi, emission, spec, kind, leader, list, depth,
            )?;
        }
        NodeRef::Penalty(value) => {
            emission.node(stores, []);
            output.penalty(value)?;
        }
        NodeRef::Rule {
            width,
            height,
            depth,
        } => {
            emission.node(stores, []);
            output.rule(width, height, depth)?;
            dvi.rule(width, height, depth).map_err(invalid_artifact)?;
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
                list,
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
            ..
        } => {
            emission.node(stores, []);
            let dvi_font_count = emission.dvi_font_count;
            let mut ignored_dvi = DviPagePlanCoEmitter::disabled();
            output.disc(lower_disc_kind(kind), |disc| {
                disc.pre(|nodes| {
                    emit_node_list(
                        stores,
                        overlay,
                        &list
                            .resolve(pre)
                            .expect("discretionary pre-break list belongs to its owner"),
                        nodes,
                        &mut ignored_dvi,
                        emission,
                        suppress_deferred_streams,
                        depth + 1,
                    )
                })?;
                disc.post(|nodes| {
                    emit_node_list(
                        stores,
                        overlay,
                        &list
                            .resolve(post)
                            .expect("discretionary post-break list belongs to its owner"),
                        nodes,
                        &mut ignored_dvi,
                        emission,
                        suppress_deferred_streams,
                        depth + 1,
                    )
                })?;
                disc.replace(|nodes| {
                    emit_node_list(
                        stores,
                        overlay,
                        &list
                            .resolve(replace)
                            .expect("discretionary replacement list belongs to its owner"),
                        nodes,
                        &mut ignored_dvi,
                        emission,
                        suppress_deferred_streams,
                        depth + 1,
                    )
                })
            })?;
            emission.dvi_font_count = dvi_font_count;
        }
        NodeRef::Mark { class, tokens } => {
            emission.node(stores, []);
            output.mark_stream(class, |tokens_out| {
                for &token in stores.tokens(tokens.id()).iter() {
                    match token {
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
            emission.node(stores, []);
            let dvi_font_count = emission.dvi_font_count;
            let mut ignored_dvi = DviPagePlanCoEmitter::disabled();
            output.insert(class, |nodes| {
                emit_node_list(
                    stores,
                    overlay,
                    &list
                        .resolve(content)
                        .expect("insertion content belongs to its enclosing owner"),
                    nodes,
                    &mut ignored_dvi,
                    emission,
                    suppress_deferred_streams,
                    depth + 1,
                )
            })?;
            emission.dvi_font_count = dvi_font_count;
        }
        NodeRef::Whatsit(whatsit) => {
            if let Some(effect_index) =
                anchor_for_whatsit(whatsit, suppress_deferred_streams, &mut emission.anchor)?
            {
                emission.node(stores, []);
                output.whatsit_anchor(effect_index)?;
                dvi.whatsit(effect_index, &overlay.effects)
                    .map_err(invalid_artifact)?;
            }
        }
        NodeRef::MathOn(width) => {
            emission.node(stores, []);
            output.math_on(width)?;
            dvi.math(width).map_err(invalid_artifact)?;
        }
        NodeRef::MathOff(width) => {
            emission.node(stores, []);
            output.math_off(width)?;
            dvi.math(width).map_err(invalid_artifact)?;
        }
        NodeRef::Direction(_) => {}
        NodeRef::Adjust(content) => {
            emission.node(stores, []);
            let dvi_font_count = emission.dvi_font_count;
            let mut ignored_dvi = DviPagePlanCoEmitter::disabled();
            output.adjust(|nodes| {
                emit_node_list(
                    stores,
                    overlay,
                    &list
                        .resolve(content.content)
                        .expect("adjustment content belongs to its enclosing owner"),
                    nodes,
                    &mut ignored_dvi,
                    emission,
                    suppress_deferred_streams,
                    depth + 1,
                )
            })?;
            emission.dvi_font_count = dvi_font_count;
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
    output: &mut ArtifactNodeListEmitter<'_>,
    dvi: &mut DviPagePlanCoEmitter,
    emission: &mut EmissionState,
    box_node: StateBoxNode<NodeListId>,
    owner: &NodeListRef,
    vertical: bool,
    suppress_deferred_streams: bool,
    depth: usize,
) -> Result<(), ExecError> {
    let fields = lower_box_header(&box_node);
    let children = owner
        .resolve(box_node.children)
        .expect("box children belong to their enclosing owner");
    dvi.begin_box(&fields, vertical, children.is_empty())
        .map_err(invalid_artifact)?;
    emission.node_empty();
    output.box_node(vertical, &fields, |nodes| {
        emit_node_list(
            stores,
            overlay,
            &children,
            nodes,
            dvi,
            emission,
            suppress_deferred_streams,
            depth + 1,
        )
    })?;
    if !children.is_empty() {
        dvi.end_box().map_err(invalid_artifact)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_glue(
    stores: &Universe,
    overlay: &PageOverlay,
    output: &mut ArtifactNodeListEmitter<'_>,
    dvi: &mut DviPagePlanCoEmitter,
    emission: &mut EmissionState,
    spec: PageGlueSpec,
    kind: PageGlueKind,
    leader: Option<StateLeaderPayload<NodeListId>>,
    owner: &NodeListRef,
    depth: usize,
) -> Result<(), ExecError> {
    let vertical_leader = matches!(&leader, Some(StateLeaderPayload::VList(_)));
    match leader {
        None => {
            emission.node(stores, []);
            output.glue(spec, kind)?;
            dvi.glue(spec).map_err(invalid_artifact)?;
        }
        Some(StateLeaderPayload::Rule {
            width,
            height,
            depth,
        }) => {
            emission.node(stores, []);
            output.glue_rule_leader(spec, kind, width, height, depth)?;
            dvi.leader_requires_replay();
        }
        Some(StateLeaderPayload::HList(box_node)) | Some(StateLeaderPayload::VList(box_node)) => {
            let vertical = vertical_leader;
            let fields = lower_box_header(&box_node);
            emission.node(stores, []);
            dvi.leader_requires_replay();
            output.glue_box_leader(spec, kind, vertical, &fields, |nodes| {
                emit_node_list(
                    stores,
                    overlay,
                    &owner
                        .resolve(box_node.children)
                        .expect("leader children belong to their enclosing owner"),
                    nodes,
                    dvi,
                    emission,
                    true,
                    depth + 1,
                )
            })?;
        }
    }
    Ok(())
}

fn anchor_for_whatsit(
    whatsit: &Whatsit,
    suppress_deferred_streams: bool,
    anchor: &mut u32,
) -> Result<Option<u32>, ExecError> {
    if !whatsit_is_anchored(whatsit, suppress_deferred_streams) {
        return Ok(None);
    }
    let index = *anchor;
    *anchor = anchor.checked_add(1).ok_or(ExecError::ArithmeticOverflow)?;
    Ok(Some(index))
}

fn whatsit_is_anchored(whatsit: &Whatsit, suppress_deferred_streams: bool) -> bool {
    match whatsit {
        Whatsit::Language { .. } | Whatsit::PdfReferenceObject { .. } => false,
        Whatsit::CloseOut { slot: None } => false,
        Whatsit::OpenOut { .. }
        | Whatsit::CloseOut { slot: Some(_) }
        | Whatsit::DeferredWrite { .. } => !suppress_deferred_streams,
        Whatsit::Special { .. }
        | Whatsit::DeferredSpecial { .. }
        | Whatsit::PdfAccessibility(_)
        | Whatsit::PdfAnnotation { .. }
        | Whatsit::PdfLinkStart { .. }
        | Whatsit::PdfLinkEnd { .. }
        | Whatsit::PdfRunningLink(_)
        | Whatsit::PdfDestination(_)
        | Whatsit::PdfThread(_)
        | Whatsit::PdfEndThread
        | Whatsit::PdfLiteral { .. }
        | Whatsit::DeferredPdfLiteral { .. }
        | Whatsit::PdfSetMatrix { .. }
        | Whatsit::PdfSave
        | Whatsit::PdfRestore => true,
        Whatsit::PdfColorStack { .. } => true,
        Whatsit::PdfSavePos
        | Whatsit::PdfSnapRefPoint
        | Whatsit::PdfSnapY { .. }
        | Whatsit::PdfSnapYComp { .. } => true,
        Whatsit::PdfRefXForm { .. } | Whatsit::PdfRefXImage { .. } => true,
    }
}

fn permutation_for<'a>(overlay: &'a PageOverlay, list: &NodeListRef) -> Option<&'a [usize]> {
    overlay
        .directions
        .iter()
        .find(|entry| &entry.list == list)
        .map(|entry| entry.order.as_slice())
}

fn math_substitution(
    overlay: &PageOverlay,
    list: &NodeListRef,
    index: usize,
) -> Option<NodeListRef> {
    overlay
        .math
        .iter()
        .find(|entry| &entry.list == list && entry.index == index)
        .map(|entry| entry.replacement.clone())
}

fn omitted_whatsit(overlay: &PageOverlay, list: &NodeListRef, index: usize) -> bool {
    overlay
        .omitted_whatsits
        .iter()
        .any(|(candidate, candidate_index)| candidate == list && *candidate_index == index)
}

fn font_resource_id(stores: &Universe, font: FontId, emission: &mut EmissionState) -> u32 {
    let logical_id = register_font_resource(stores, font, emission);
    match stores.font(font).construction() {
        tex_fonts::FontConstruction::Loaded
        | tex_fonts::FontConstruction::Copied { .. }
        | tex_fonts::FontConstruction::Expanded { .. } => logical_id,
        tex_fonts::FontConstruction::Letterspaced { source, .. } => {
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
    ch: u32,
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
    let code = u8::try_from(ch).map_err(|_| ExecError::UnsupportedShipoutNode {
        node: "non-byte generated font character",
    })?;
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
    ch: u32,
    logical_width: tex_state::scaled::Scaled,
    origins: impl IntoIterator<Item = OriginRef>,
    output: &mut ArtifactNodeListEmitter<'_>,
    dvi: &mut DviPagePlanCoEmitter,
    emission: &mut EmissionState,
) -> Result<(), ExecError> {
    let projection = glyph_projection(stores, font, ch, logical_width, emission)?;
    emit_projection_kern(projection.left, output, dvi, emission)?;
    emission.node(stores, origins);
    output.char(projection.font_id, ch, projection.width)?;
    emission.sync_dvi_fonts(dvi)?;
    dvi.char(projection.font_id, ch, projection.width)
        .map_err(invalid_artifact)?;
    emit_projection_kern(projection.right, output, dvi, emission)
}

#[allow(clippy::too_many_arguments)]
fn emit_ligature(
    stores: &Universe,
    font: FontId,
    ch: u32,
    source: &[char],
    logical_width: tex_state::scaled::Scaled,
    origins: impl IntoIterator<Item = OriginRef>,
    output: &mut ArtifactNodeListEmitter<'_>,
    dvi: &mut DviPagePlanCoEmitter,
    emission: &mut EmissionState,
) -> Result<(), ExecError> {
    let projection = glyph_projection(stores, font, ch, logical_width, emission)?;
    emit_projection_kern(projection.left, output, dvi, emission)?;
    emission.node(stores, origins);
    output.lig(
        projection.font_id,
        ch,
        source.iter().map(|source| *source as u32),
        projection.width,
    )?;
    emission.sync_dvi_fonts(dvi)?;
    dvi.char(projection.font_id, ch, projection.width)
        .map_err(invalid_artifact)?;
    emit_projection_kern(projection.right, output, dvi, emission)
}

fn emit_projection_kern(
    amount: tex_state::scaled::Scaled,
    output: &mut ArtifactNodeListEmitter<'_>,
    dvi: &mut DviPagePlanCoEmitter,
    emission: &mut EmissionState,
) -> Result<(), ExecError> {
    if amount.raw() == 0 {
        return Ok(());
    }
    emission.node_empty();
    output.kern(amount, PageKernKind::Explicit)?;
    dvi.kern(amount).map_err(invalid_artifact)?;
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
    let id = font.raw().checked_sub(1).expect("FontId is one-based");
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
        layout_policy: loaded.layout_policy(),
        mapping_fallback: loaded.mapping_fallback(),
        opentype: loaded.opentype().map(|font| tex_out::OpenTypeFontResource {
            program_identity: font.identity,
            object_identity: font.object_identity,
            instance_identity: loaded
                .opentype_instance_identity()
                .expect("OpenType font has an instance identity"),
            container: font.container,
            face_index: font.face_index,
            variation: font.variation.clone(),
            features: font.feature_policy.clone(),
            direction: font.direction,
            script: font.script,
            language: font.language.clone(),
            encoding_map_version: loaded.encoding_map().map(|map| map.version()),
            encoding_map_identity: loaded.encoding_map().map(|map| map.identity()),
            fontdimen_synthesis_version: loaded
                .encoding_map()
                .map(|_| tex_fonts::OPENTYPE_FONTDIMEN_SYNTHESIS_VERSION),
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
) -> Result<(u32, tex_state::scaled::Scaled), ExecError> {
    let width = stores
        .font_character_metrics(font, ch)
        .map(|metrics| metrics.width)
        .ok_or(ExecError::UnsupportedShipoutNode {
            node: "missing character metrics",
        })?;
    Ok((ch as u32, width))
}

fn check_depth(depth: usize) -> Result<(), ExecError> {
    if depth > MAX_SHIPOUT_DEPTH {
        return Err(ExecError::UnsupportedShipoutNode {
            node: "shipout nesting deeper than 4096",
        });
    }
    Ok(())
}
