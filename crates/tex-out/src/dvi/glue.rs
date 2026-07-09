use tex_arith::{GLUE_SET_RATIO_SCALE, Scaled};

use crate::{GlueOrder, GlueSetRatio, GlueSign, GlueSpec};

use super::DviError;

const BILLION: i64 = 1_000_000_000;

pub(super) fn adjusted_glue_width(
    spec: GlueSpec,
    g_sign: GlueSign,
    g_order: GlueOrder,
    glue_set: GlueSetRatio,
    cur_glue: &mut Scaled,
    cur_g: &mut Scaled,
) -> Result<Scaled, DviError> {
    // tex.web hlist_out/vlist_out: rule_wd/rule_ht := width(g) - cur_g,
    // then cur_g becomes round(glue_set(this_box) * cur_glue).
    let base = sub_scaled(spec.width, *cur_g)?;
    if g_sign != GlueSign::Normal {
        match g_sign {
            GlueSign::Stretching if spec.stretch_order == g_order => {
                *cur_glue = add_scaled(*cur_glue, spec.stretch)?;
                *cur_g = rounded_glue_set(glue_set, *cur_glue);
            }
            GlueSign::Shrinking if spec.shrink_order == g_order => {
                *cur_glue = sub_scaled(*cur_glue, spec.shrink)?;
                *cur_g = rounded_glue_set(glue_set, *cur_glue);
            }
            _ => {}
        }
    }
    add_scaled(base, *cur_g)
}

fn rounded_glue_set(glue_set: GlueSetRatio, cur_glue: Scaled) -> Scaled {
    let product = i128::from(glue_set.raw()) * i128::from(cur_glue.raw());
    let rounded = rounded_div(product, i128::from(GLUE_SET_RATIO_SCALE));
    let vetted = rounded.clamp(-i128::from(BILLION), i128::from(BILLION));
    Scaled::from_raw(i32::try_from(vetted).expect("vetted glue is in i32 range"))
}

fn rounded_div(value: i128, divisor: i128) -> i128 {
    debug_assert!(divisor > 0);
    if value >= 0 {
        (value + divisor / 2) / divisor
    } else {
        -((-value + divisor / 2) / divisor)
    }
}

pub(super) fn add_scaled(left: Scaled, right: Scaled) -> Result<Scaled, DviError> {
    left.checked_add(right).ok_or(DviError::PositionOverflow)
}

pub(super) fn sub_scaled(left: Scaled, right: Scaled) -> Result<Scaled, DviError> {
    left.checked_sub(right).ok_or(DviError::PositionOverflow)
}
