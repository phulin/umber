use tex_state::font::NULL_FONT;
use tex_state::ids::FontId;
use tex_state::math::MathFontSize;
use tex_state::scaled::Scaled;

use super::params::MathParams;
use super::style::Style;
use super::{
    BoxAxis, Context, FetchedChar, FrozenHList, MathBox, MathTypesetState, add, boxed_node,
    char_box, sub,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DelimiterCode {
    small_family: u8,
    small_char: u8,
    large_family: u8,
    large_char: u8,
}

#[derive(Clone, Copy, Debug)]
struct DelimiterCandidate {
    font: FontId,
    code: u8,
    height_plus_depth: Scaled,
}

#[must_use]
pub fn left_right_delimiter_target(
    params: &MathParams,
    style: Style,
    max_height: Scaled,
    max_depth: Scaled,
) -> Scaled {
    // AppG rule 19
    let axis = params.for_size(style.size()).symbols.axis_height;
    let delta2 = add(max_depth, axis);
    let mut delta1 = sub(add(max_height, max_depth), delta2);
    if delta2 > delta1 {
        delta1 = delta2;
    }
    let factored = Scaled::from_raw(
        ((i64::from(delta1.raw() / 500)) * i64::from(params.delimiter_factor))
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    );
    let shortfall_target = sub(add(delta1, delta1), params.delimiter_shortfall);
    factored.max(shortfall_target)
}

#[must_use]
pub fn var_delimiter(
    state: &impl MathTypesetState,
    params: &MathParams,
    delimiter: u32,
    size: MathFontSize,
    target: Scaled,
) -> MathBox {
    // AppG rule 15, rule 19
    let code = decode_delimiter(delimiter);
    let mut best = None;
    let candidate = search_variant_chain(
        state,
        code.small_family,
        code.small_char,
        size,
        target,
        &mut best,
    )
    .or_else(|| {
        search_variant_chain(
            state,
            code.large_family,
            code.large_char,
            size,
            target,
            &mut best,
        )
    })
    .or(best)
    .unwrap_or(DelimiterCandidate {
        font: NULL_FONT,
        code: 0,
        height_plus_depth: Scaled::from_raw(0),
    });

    let mut boxed = if candidate.font == NULL_FONT {
        MathBox {
            width: params.null_delimiter_space,
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            list: FrozenHList::default(),
            axis: BoxAxis::Horizontal,
        }
    } else if state
        .font_extensible_recipe(candidate.font, candidate.code)
        .is_some()
    {
        extensible_box(state, candidate.font, candidate.code, target)
    } else {
        char_box_for(state, candidate.font, candidate.code)
            .expect("delimiter candidate came from a present character")
    };

    let axis = params.for_size(size).symbols.axis_height;
    boxed.shift = sub(
        Scaled::from_raw(tex_arith::half(sub(boxed.height, boxed.depth).raw())),
        axis,
    );
    boxed
}

pub(super) fn make_delimiter(
    ctx: &Context<'_, impl MathTypesetState>,
    delimiter: u32,
    target: Scaled,
) -> MathBox {
    var_delimiter(ctx.state, ctx.params, delimiter, ctx.style.size(), target)
}

fn decode_delimiter(delimiter: u32) -> DelimiterCode {
    DelimiterCode {
        small_family: ((delimiter >> 20) & 0xf) as u8,
        small_char: ((delimiter >> 12) & 0xff) as u8,
        large_family: ((delimiter >> 8) & 0xf) as u8,
        large_char: (delimiter & 0xff) as u8,
    }
}

fn search_variant_chain(
    state: &impl MathTypesetState,
    family: u8,
    code: u8,
    size: MathFontSize,
    target: Scaled,
    best: &mut Option<DelimiterCandidate>,
) -> Option<DelimiterCandidate> {
    // AppG rule 15, rule 19
    if family == 0 && code == 0 {
        return None;
    }
    for size in delimiter_font_sizes(size) {
        let font = state.math_family_font(size, family);
        let mut current = code;
        while let Some(metrics) = state.font_char_metrics(font, current) {
            if state.font_extensible_recipe(font, current).is_some() {
                return Some(DelimiterCandidate {
                    font,
                    code: current,
                    height_plus_depth: add(metrics.height, metrics.depth),
                });
            }
            let height_plus_depth = add(metrics.height, metrics.depth);
            if best
                .as_ref()
                .is_none_or(|old| height_plus_depth > old.height_plus_depth)
            {
                let candidate = DelimiterCandidate {
                    font,
                    code: current,
                    height_plus_depth,
                };
                if height_plus_depth >= target {
                    return Some(candidate);
                }
                *best = Some(candidate);
            }
            let Some(next) = state.font_next_larger(font, current) else {
                break;
            };
            current = next;
        }
    }
    None
}

fn delimiter_font_sizes(size: MathFontSize) -> impl Iterator<Item = MathFontSize> {
    const TEXT: &[MathFontSize] = &[MathFontSize::Text];
    const SCRIPT: &[MathFontSize] = &[MathFontSize::Script, MathFontSize::Text];
    const SCRIPT_SCRIPT: &[MathFontSize] = &[
        MathFontSize::ScriptScript,
        MathFontSize::Script,
        MathFontSize::Text,
    ];
    match size {
        MathFontSize::Text => TEXT,
        MathFontSize::Script => SCRIPT,
        MathFontSize::ScriptScript => SCRIPT_SCRIPT,
    }
    .iter()
    .copied()
}

fn extensible_box(
    state: &impl MathTypesetState,
    font: FontId,
    code: u8,
    target: Scaled,
) -> MathBox {
    // AppG rule 15, rule 19
    let recipe = state
        .font_extensible_recipe(font, code)
        .expect("caller checked for an extensible recipe");
    let repeated = recipe.repeated;
    let repeat_size = height_plus_depth(state, font, repeated);
    let repeated_metrics = state
        .font_char_metrics(font, repeated)
        .expect("TFM parser validates extensible repeated pieces");
    let mut total = Scaled::from_raw(0);
    for code in [recipe.bottom, recipe.middle, recipe.top]
        .into_iter()
        .flatten()
    {
        total = add(total, height_plus_depth(state, font, code));
    }
    let mut repeats = 0;
    if repeat_size.raw() > 0 {
        while total < target {
            total = add(total, repeat_size);
            repeats += 1;
            if recipe.middle.is_some() {
                total = add(total, repeat_size);
            }
        }
    }

    let mut boxed = MathBox {
        width: add(repeated_metrics.width, repeated_metrics.italic_correction),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        list: FrozenHList::default(),
        axis: BoxAxis::Vertical,
    };

    if let Some(code) = recipe.bottom {
        stack_into_box(&mut boxed, state, font, code);
    }
    for _ in 0..repeats {
        stack_into_box(&mut boxed, state, font, repeated);
    }
    if let Some(code) = recipe.middle {
        stack_into_box(&mut boxed, state, font, code);
        for _ in 0..repeats {
            stack_into_box(&mut boxed, state, font, repeated);
        }
    }
    if let Some(code) = recipe.top {
        stack_into_box(&mut boxed, state, font, code);
    }
    boxed.depth = sub(total, boxed.height);
    boxed
}

fn stack_into_box(boxed: &mut MathBox, state: &impl MathTypesetState, font: FontId, code: u8) {
    // AppG rule 15, rule 19
    let component = char_box_for(state, font, code)
        .expect("TFM parser validates extensible recipe component characters");
    boxed.height = component.height;
    boxed.list.nodes.insert(0, boxed_node(component));
}

fn height_plus_depth(state: &impl MathTypesetState, font: FontId, code: u8) -> Scaled {
    // AppG rule 15, rule 19
    state
        .font_char_metrics(font, code)
        .map_or(Scaled::from_raw(0), |metrics| {
            add(metrics.height, metrics.depth)
        })
}

fn char_box_for(state: &impl MathTypesetState, font: FontId, code: u8) -> Option<MathBox> {
    let metrics = state.font_char_metrics(font, code)?;
    Some(char_box(FetchedChar {
        font,
        ch: char::from(code),
        metrics,
    }))
}
