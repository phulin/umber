use super::*;

pub(super) struct PendingPageEffects {
    pub(super) effects: Vec<PageEffect>,
    pub(super) open_out_occurrences: Vec<(usize, tex_state::ArtifactEffectOrdinal)>,
}

/// Lowers the effects a page must carry forward: the whatsit output produced
/// *before* this `\shipout` began.
///
/// `pending_end` is that boundary, an index into the live effect suffix. It
/// is not a refinement of an already-correct sweep: without it the sweep is
/// unscoped, so anything the shipout itself prints before staging -- §638's
/// own `[<counts>` progress marker above all -- is embedded in the page's
/// serialized content as a spurious whatsit (`umber2-v4dx`). Job framing and
/// page content are distinguished by *when* they happen, which is exactly
/// what this index records.
pub(super) fn pending_page_effects(
    world: &tex_state::World,
    pending_end: usize,
) -> PendingPageEffects {
    let mut effects = Vec::new();
    let mut open_out_occurrences = Vec::new();
    for (world_index, record) in world
        .page_effect_prefix()
        .iter()
        .chain(world.effect_records()[..pending_end].iter())
        .enumerate()
    {
        let Some(effect) = lower_effect_record(record) else {
            continue;
        };
        let page_index = effects.len();
        if matches!(effect, PageEffect::OpenOut { .. }) {
            let ordinal = u32::try_from(world_index + 1)
                .expect("page-visible effect ordinal exceeds detached artifact schema");
            open_out_occurrences.push((page_index, tex_state::ArtifactEffectOrdinal::new(ordinal)));
        }
        effects.push(effect);
    }
    PendingPageEffects {
        effects,
        open_out_occurrences,
    }
}

pub(super) fn lower_effect_record(record: &EffectRecord) -> Option<PageEffect> {
    match record {
        EffectRecord::StreamOpen { slot, target } => Some(PageEffect::OpenOut {
            stream: slot.raw(),
            path: target.path().to_string_lossy().into_owned(),
        }),
        EffectRecord::StreamClose { slot } => Some(PageEffect::CloseOut { stream: slot.raw() }),
        EffectRecord::StreamWrite { sink, text } => Some(PageEffect::Write {
            sink: lower_sink(*sink),
            text: text.clone(),
        }),
        EffectRecord::Special { class, payload } => Some(PageEffect::Special {
            class: class.clone(),
            payload: payload.clone(),
        }),
        EffectRecord::DeferredWrite { .. }
        | EffectRecord::StreamWriteBytes { .. }
        | EffectRecord::PdfObjectPlaceholder { .. }
        | EffectRecord::ShellEscape(_) => None,
    }
}

pub(crate) fn page_counts(stores: &Universe) -> [i32; 10] {
    let mut counts = [0; 10];
    for (index, value) in counts.iter_mut().enumerate() {
        *value = stores.count(index as u16);
    }
    counts
}

pub(super) fn lower_box_header<List>(box_node: &StateBoxNode<List>) -> PageBoxNode {
    PageBoxNode {
        width: box_node.width,
        height: box_node.height,
        depth: box_node.depth,
        shift: box_node.shift,
        glue_set: box_node.glue_set,
        glue_sign: lower_glue_sign(box_node.glue_sign),
        glue_order: lower_order(box_node.glue_order),
        children: Vec::new(),
    }
}

pub(super) fn lower_glue(spec: tex_state::glue::GlueSpec) -> PageGlueSpec {
    PageGlueSpec {
        width: spec.width,
        stretch: spec.stretch,
        stretch_order: lower_order(spec.stretch_order),
        shrink: spec.shrink,
        shrink_order: lower_order(spec.shrink_order),
    }
}

pub(super) fn lower_order(order: Order) -> PageGlueOrder {
    match order {
        Order::Normal => PageGlueOrder::Normal,
        Order::Fil => PageGlueOrder::Fil,
        Order::Fill => PageGlueOrder::Fill,
        Order::Filll => PageGlueOrder::Filll,
    }
}

pub(super) fn lower_glue_sign(sign: Sign) -> GlueSign {
    match sign {
        Sign::Normal => GlueSign::Normal,
        Sign::Stretching => GlueSign::Stretching,
        Sign::Shrinking => GlueSign::Shrinking,
    }
}

pub(super) fn lower_kern_kind(kind: StateKernKind) -> PageKernKind {
    match kind {
        StateKernKind::Explicit => PageKernKind::Explicit,
        StateKernKind::Font => PageKernKind::Font,
        StateKernKind::Auto => PageKernKind::Auto,
        StateKernKind::Accent => PageKernKind::Accent,
        StateKernKind::Mu => PageKernKind::Explicit,
        StateKernKind::LeftMargin => PageKernKind::LeftMargin,
        StateKernKind::RightMargin => PageKernKind::RightMargin,
    }
}

pub(super) fn lower_margin_kern_side(side: StateMarginKernSide) -> PageMarginKernSide {
    match side {
        StateMarginKernSide::Left => PageMarginKernSide::Left,
        StateMarginKernSide::Right => PageMarginKernSide::Right,
    }
}

pub(super) fn lower_disc_kind(kind: StateDiscKind) -> PageDiscKind {
    match kind {
        StateDiscKind::Discretionary => PageDiscKind::Discretionary,
        StateDiscKind::ExplicitHyphen => PageDiscKind::ExplicitHyphen,
        StateDiscKind::AutomaticHyphen => PageDiscKind::AutomaticHyphen,
    }
}

pub(super) fn lower_glue_kind(kind: StateGlueKind) -> PageGlueKind {
    match kind {
        StateGlueKind::Normal
        | StateGlueKind::SpaceSkip
        | StateGlueKind::XSpaceSkip
        | StateGlueKind::TabSkip
        | StateGlueKind::ParSkip => PageGlueKind::Normal,
        StateGlueKind::BaselineSkip => PageGlueKind::BaselineSkip,
        StateGlueKind::LineSkip => PageGlueKind::LineSkip,
        StateGlueKind::TopSkip
        | StateGlueKind::SplitTopSkip
        | StateGlueKind::AboveDisplaySkip
        | StateGlueKind::BelowDisplaySkip
        | StateGlueKind::AboveDisplayShortSkip
        | StateGlueKind::BelowDisplayShortSkip => PageGlueKind::Normal,
        StateGlueKind::LeftSkip => PageGlueKind::LeftSkip,
        StateGlueKind::RightSkip => PageGlueKind::RightSkip,
        StateGlueKind::ParFillSkip => PageGlueKind::ParFillSkip,
        StateGlueKind::Leaders => PageGlueKind::Leaders,
        StateGlueKind::Cleaders => PageGlueKind::Cleaders,
        StateGlueKind::Xleaders => PageGlueKind::Xleaders,
        StateGlueKind::MuSkip
        | StateGlueKind::ThinMuSkip
        | StateGlueKind::MedMuSkip
        | StateGlueKind::ThickMuSkip
        | StateGlueKind::NonScript => PageGlueKind::Normal,
    }
}

pub(super) fn lower_token_catcode(cat: Catcode) -> TokenCatcode {
    match cat {
        Catcode::Escape => TokenCatcode::Escape,
        Catcode::BeginGroup => TokenCatcode::BeginGroup,
        Catcode::EndGroup => TokenCatcode::EndGroup,
        Catcode::MathShift => TokenCatcode::MathShift,
        Catcode::AlignmentTab => TokenCatcode::AlignmentTab,
        Catcode::EndLine => TokenCatcode::EndLine,
        Catcode::Parameter => TokenCatcode::Parameter,
        Catcode::Superscript => TokenCatcode::Superscript,
        Catcode::Subscript => TokenCatcode::Subscript,
        Catcode::Ignored => TokenCatcode::Ignored,
        Catcode::Space => TokenCatcode::Space,
        Catcode::Letter => TokenCatcode::Letter,
        Catcode::Other => TokenCatcode::Other,
        Catcode::Active => TokenCatcode::Active,
        Catcode::Comment => TokenCatcode::Comment,
        Catcode::Invalid => TokenCatcode::Invalid,
    }
}

pub(super) fn lower_sink(sink: PrintSink) -> EffectSink {
    match sink {
        PrintSink::Terminal => EffectSink::Terminal,
        PrintSink::Log => EffectSink::Log,
        PrintSink::TerminalAndLog => EffectSink::TerminalAndLog,
        PrintSink::Stream(slot) => EffectSink::Stream(slot.raw()),
    }
}
