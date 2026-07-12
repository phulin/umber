use tex_arith::Scaled;

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
    // tex.web section 109 stores glue_set in a one-word `glue_ratio`, normally
    // a short_real, after computing the quotient as a `real`. The canonical
    // TeX82 implementation therefore rounds the ratio to binary32 before the
    // hlist_out/vlist_out `real` multiplication and final `round` (sections
    // 625 and 635). Emulate both floating-point roundings with integers so DVI
    // output remains deterministic on every host.
    let numerator = glue_set.numerator() as u128;
    let denominator = glue_set.denominator() as u128;
    let (ratio_significand, ratio_exponent) = binary32_ratio(numerator, denominator);
    let negative = cur_glue.raw() < 0;
    let magnitude = u128::from(cur_glue.raw().unsigned_abs());
    let (significand, exponent) =
        binary64_multiply_integer(ratio_significand, ratio_exponent, magnitude);
    let rounded = round_binary_product(significand, exponent).min(BILLION);
    Scaled::from_raw(if negative { -rounded } else { rounded } as i32)
}

/// Returns `significand * 2^exponent`, the nearest binary32 value to `n / d`.
fn binary32_ratio(n: u128, d: u128) -> (u128, i32) {
    if n == 0 {
        return (0, 0);
    }
    let mut exponent = floor_log2_ratio(n, d);
    let shift = 23 - exponent;
    let mut significand = if shift >= 0 {
        round_quotient_to_even(n << shift, d)
    } else {
        round_quotient_to_even(n, d << (-shift))
    };
    if significand == 1_u128 << 24 {
        significand >>= 1;
        exponent += 1;
    }
    (significand, exponent - 23)
}

fn floor_log2_ratio(n: u128, d: u128) -> i32 {
    let mut exponent = i32::try_from(n.ilog2()).expect("u128 log fits i32")
        - i32::try_from(d.ilog2()).expect("u128 log fits i32");
    let below = if exponent >= 0 {
        n < d << exponent
    } else {
        n << (-exponent) < d
    };
    if below {
        exponent -= 1;
    }
    exponent
}

fn binary64_multiply_integer(
    significand: u128,
    mut exponent: i32,
    multiplier: u128,
) -> (u128, i32) {
    let product = significand * multiplier;
    if product == 0 {
        return (0, 0);
    }
    let bits = product.ilog2() + 1;
    if bits <= 53 {
        return (product, exponent);
    }
    let shift = bits - 53;
    let mut rounded = round_power_of_two_to_even(product, shift);
    exponent += i32::try_from(shift).expect("binary64 shift fits i32");
    if rounded == 1_u128 << 53 {
        rounded >>= 1;
        exponent += 1;
    }
    (rounded, exponent)
}

fn round_quotient_to_even(n: u128, d: u128) -> u128 {
    let quotient = n / d;
    let remainder = n % d;
    match (remainder * 2).cmp(&d) {
        std::cmp::Ordering::Greater => quotient + 1,
        std::cmp::Ordering::Equal if !quotient.is_multiple_of(2) => quotient + 1,
        _ => quotient,
    }
}

fn round_power_of_two_to_even(value: u128, shift: u32) -> u128 {
    let quotient = value >> shift;
    let remainder = value & ((1_u128 << shift) - 1);
    let halfway = 1_u128 << (shift - 1);
    if remainder > halfway || (remainder == halfway && !quotient.is_multiple_of(2)) {
        quotient + 1
    } else {
        quotient
    }
}

fn round_binary_product(significand: u128, exponent: i32) -> i64 {
    if significand == 0 {
        return 0;
    }
    let billion = BILLION as u128;
    if exponent >= 0 {
        return i64::try_from((significand << exponent).min(billion))
            .expect("clamped glue fits i64");
    }
    let shift = u32::try_from(-exponent).expect("binary exponent magnitude fits u32");
    if significand >= billion << shift {
        return BILLION;
    }
    let quotient = significand >> shift;
    let remainder = significand & ((1_u128 << shift) - 1);
    let rounded = quotient + u128::from(remainder >= 1_u128 << (shift - 1));
    i64::try_from(rounded).expect("vetted glue fits i64")
}

pub(super) fn add_scaled(left: Scaled, right: Scaled) -> Result<Scaled, DviError> {
    left.checked_add(right).ok_or(DviError::PositionOverflow)
}

pub(super) fn sub_scaled(left: Scaled, right: Scaled) -> Result<Scaled, DviError> {
    left.checked_sub(right).ok_or(DviError::PositionOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glue_ratio_is_rounded_to_tex82_short_real_before_output() {
        assert_eq!(
            rounded_glue_set(
                GlueSetRatio::from_ratio_parts(638_162_529, 65_536),
                Scaled::from_raw(65_536),
            ),
            Scaled::from_raw(638_162_560)
        );
        assert_eq!(
            rounded_glue_set(
                GlueSetRatio::from_ratio_parts(50_816_599, 16_384),
                Scaled::from_raw(65_536),
            ),
            Scaled::from_raw(203_266_400)
        );
    }

    #[test]
    fn binary32_ratio_handles_large_one_word_glue_ratios() {
        assert_eq!(
            rounded_glue_set(
                GlueSetRatio::from_ratio_parts(i32::MAX, 1),
                Scaled::from_raw(1),
            ),
            Scaled::from_raw(1_000_000_000)
        );
    }
}
