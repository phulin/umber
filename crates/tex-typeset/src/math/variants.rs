use tex_fonts::{
    MathMetricsSource, MathVariantDirection, OpenTypeMathAssembly, OpenTypeMathAssemblyPart,
    OpenTypeMathGlyph,
};
use tex_state::ids::FontId;
use tex_state::node::KernKind;
use tex_state::scaled::Scaled;
use tex_state::token::OriginId;

use super::{Context, FetchedChar, MathBox, MathNode, MathTypesetState, add, char_box, sub};

#[cfg(test)]
mod tests;

const MAX_EXPANDED_ASSEMBLY_PARTS: usize = 65_536;

/// Selects the first adequate OpenType MATH variant, or builds its assembly.
/// If neither reaches the target, the largest valid variant is returned.
pub(crate) fn variant_box(
    ctx: &mut Context<'_, impl MathTypesetState>,
    font: FontId,
    base: FetchedChar,
    target: Scaled,
    direction: MathVariantDirection,
    origin: OriginId,
) -> Option<(MathBox, Scaled)> {
    let MathMetricsSource::OpenType(math) = ctx.state.math_metrics_source(font) else {
        return None;
    };
    let construction = math.construction(base.glyph_id?, direction)?;
    if let Some(variant) = construction
        .variants
        .iter()
        .find(|variant| variant.advance >= target)
    {
        return Some((
            glyph_box(ctx, font, base.ch, variant.glyph, origin),
            variant.glyph.italic_correction,
        ));
    }
    if let Some(assembly) = construction.assembly
        && let Some(boxed) = assembly_box(ctx, font, base.ch, target, direction, origin, &assembly)
    {
        return Some((boxed, assembly.italic_correction));
    }
    construction.variants.last().map(|variant| {
        (
            glyph_box(ctx, font, base.ch, variant.glyph, origin),
            variant.glyph.italic_correction,
        )
    })
}

fn glyph_box(
    ctx: &mut Context<'_, impl MathTypesetState>,
    font: FontId,
    ch: char,
    glyph: OpenTypeMathGlyph,
    origin: OriginId,
) -> MathBox {
    char_box(
        ctx,
        FetchedChar {
            font,
            ch,
            metrics: glyph.metrics,
            glyph_id: Some(glyph.glyph_id),
            top_accent_attachment: glyph.top_accent_attachment,
        },
        origin,
    )
}

fn assembly_box(
    ctx: &mut Context<'_, impl MathTypesetState>,
    font: FontId,
    ch: char,
    target: Scaled,
    direction: MathVariantDirection,
    origin: OriginId,
    assembly: &OpenTypeMathAssembly,
) -> Option<MathBox> {
    let (parts, overlaps) = plan_assembly(assembly, target)?;
    match direction {
        MathVariantDirection::Horizontal => {
            horizontal_assembly(ctx, font, ch, origin, &parts, &overlaps)
        }
        MathVariantDirection::Vertical => vertical_assembly(
            ctx,
            font,
            ch,
            origin,
            &parts,
            &overlaps,
            assembly.italic_correction,
        ),
    }
}

fn plan_assembly(
    assembly: &OpenTypeMathAssembly,
    target: Scaled,
) -> Option<(Vec<OpenTypeMathAssemblyPart>, Vec<Scaled>)> {
    let repeats = required_repeats(assembly, target)?;
    let mut parts = Vec::new();
    for part in &assembly.parts {
        let count = if part.extender { repeats } else { 1 };
        if parts.len().checked_add(count)? > MAX_EXPANDED_ASSEMBLY_PARTS {
            return None;
        }
        parts.extend(std::iter::repeat_n(*part, count));
    }
    if parts.is_empty() {
        return None;
    }
    let overlaps = connector_overlaps(&parts, assembly.min_connector_overlap, target)?;
    Some((parts, overlaps))
}

fn required_repeats(assembly: &OpenTypeMathAssembly, target: Scaled) -> Option<usize> {
    let extenders = assembly.parts.iter().filter(|part| part.extender).count();
    let extent = |repeats: usize| -> Option<i64> {
        let count = assembly
            .parts
            .len()
            .checked_add(extenders.checked_mul(repeats.checked_sub(1)?)?)?;
        let advances = assembly.parts.iter().try_fold(0_i64, |sum, part| {
            let copies = if part.extender { repeats } else { 1 };
            sum.checked_add(i64::from(part.full_advance.raw()).checked_mul(copies as i64)?)
        })?;
        advances.checked_sub(
            i64::from(assembly.min_connector_overlap.raw())
                .checked_mul(count.saturating_sub(1) as i64)?,
        )
    };
    let first = extent(1)?;
    if first >= i64::from(target.raw()) {
        return Some(1);
    }
    if extenders == 0 {
        return None;
    }
    let second = extent(2)?;
    let growth = second.checked_sub(first)?;
    if growth <= 0 {
        return None;
    }
    let missing = i64::from(target.raw()).checked_sub(first)?;
    let additional = missing.checked_add(growth - 1)?.checked_div(growth)?;
    usize::try_from(additional.checked_add(1)?).ok()
}

fn connector_overlaps(
    parts: &[OpenTypeMathAssemblyPart],
    minimum: Scaled,
    target: Scaled,
) -> Option<Vec<Scaled>> {
    let mut overlaps = Vec::with_capacity(parts.len().saturating_sub(1));
    let mut extent = parts.iter().try_fold(Scaled::from_raw(0), |sum, part| {
        sum.raw()
            .checked_add(part.full_advance.raw())
            .map(Scaled::from_raw)
    })?;
    for pair in parts.windows(2) {
        let maximum = pair[0].end_connector.min(pair[1].start_connector);
        if maximum < minimum {
            return None;
        }
        overlaps.push(minimum);
        extent = sub(extent, minimum);
    }
    let mut excess = sub(extent, target).max(Scaled::from_raw(0));
    for (index, pair) in parts.windows(2).enumerate() {
        let maximum = pair[0].end_connector.min(pair[1].start_connector);
        let extra = sub(maximum, minimum).min(excess);
        overlaps[index] = add(overlaps[index], extra);
        excess = sub(excess, extra);
    }
    Some(overlaps)
}

fn horizontal_assembly(
    ctx: &mut Context<'_, impl MathTypesetState>,
    font: FontId,
    ch: char,
    origin: OriginId,
    parts: &[OpenTypeMathAssemblyPart],
    overlaps: &[Scaled],
) -> Option<MathBox> {
    let mut nodes = Vec::with_capacity(parts.len() * 2);
    for (index, part) in parts.iter().enumerate() {
        nodes.push(MathNode::Char {
            font,
            ch,
            glyph_id: Some(part.glyph.glyph_id),
            metrics: part.glyph.metrics,
            origin,
        });
        let overlap = overlaps.get(index).copied().unwrap_or(Scaled::from_raw(0));
        nodes.push(MathNode::Kern {
            amount: sub(sub(part.full_advance, part.glyph.metrics.width), overlap),
            kind: KernKind::Explicit,
        });
    }
    let list = ctx.layout.hlist(nodes);
    Some(ctx.layout.hpack(list))
}

fn vertical_assembly(
    ctx: &mut Context<'_, impl MathTypesetState>,
    font: FontId,
    ch: char,
    origin: OriginId,
    parts: &[OpenTypeMathAssemblyPart],
    overlaps: &[Scaled],
    italic_correction: Scaled,
) -> Option<MathBox> {
    let mut components = Vec::with_capacity(parts.len() * 2);
    for index in (0..parts.len()).rev() {
        let part = parts[index];
        let mut boxed = glyph_box(ctx, font, ch, part.glyph, origin);
        boxed.width = part.glyph.metrics.width;
        boxed.depth = sub(part.full_advance, boxed.height);
        components.push(MathNode::HList(boxed));
        if index > 0 {
            components.push(MathNode::Kern {
                amount: Scaled::from_raw(-overlaps[index - 1].raw()),
                kind: KernKind::Explicit,
            });
        }
    }
    let list = ctx.layout.hlist(components);
    let mut boxed = ctx.layout.vpack(list);
    boxed.width = add(boxed.width, italic_correction);
    Some(boxed)
}
