use tex_arith::{x_over_n, xn_over_d};
use tex_state::glue::{GlueSpec, Order};
use tex_state::math::NoadClass;
use tex_state::scaled::Scaled;

use super::params::MathParams;
use super::style::Style;

const SPACING: &[u8; 64] = b"0234000122*4000133**3**344*0400400*000000234000111*1111112341011";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpacingKind {
    None,
    Thin,
    Med,
    Thick,
}

#[must_use]
pub fn inter_noad_spacing(left: NoadClass, right: NoadClass, style: Style) -> SpacingKind {
    // AppG rule 18
    match SPACING[class_index(left) * 8 + class_index(right)] {
        b'0' | b'*' => SpacingKind::None,
        b'1' if !style.is_script_or_smaller() => SpacingKind::Thin,
        b'1' => SpacingKind::None,
        b'2' => SpacingKind::Thin,
        b'3' if !style.is_script_or_smaller() => SpacingKind::Med,
        b'3' => SpacingKind::None,
        b'4' if !style.is_script_or_smaller() => SpacingKind::Thick,
        b'4' => SpacingKind::None,
        _ => unreachable!("math spacing table contains only TeX spacing digits"),
    }
}

#[must_use]
pub fn spacing_glue(kind: SpacingKind, params: &MathParams, mu: Scaled) -> Option<GlueSpec> {
    // AppG rule 18
    let spec = match kind {
        SpacingKind::None => return None,
        SpacingKind::Thin => params.thin_mu_skip,
        SpacingKind::Med => params.med_mu_skip,
        SpacingKind::Thick => params.thick_mu_skip,
    };
    Some(math_glue(spec, mu))
}

#[must_use]
pub fn math_glue(spec: GlueSpec, mu: Scaled) -> GlueSpec {
    // AppG rule 18
    GlueSpec {
        width: mu_mult(spec.width, mu),
        stretch: if spec.stretch_order == Order::Normal {
            mu_mult(spec.stretch, mu)
        } else {
            spec.stretch
        },
        stretch_order: spec.stretch_order,
        shrink: if spec.shrink_order == Order::Normal {
            mu_mult(spec.shrink, mu)
        } else {
            spec.shrink
        },
        shrink_order: spec.shrink_order,
    }
}

#[must_use]
pub fn math_kern(amount: Scaled, mu: Scaled) -> Scaled {
    // AppG rule 18
    mu_mult(amount, mu)
}

fn mu_mult(value: Scaled, mu: Scaled) -> Scaled {
    let divided = x_over_n(mu, 0o200000).expect("math unit denominator is nonzero");
    let mut n = divided.quotient.raw();
    let mut f = divided.remainder.raw();
    if f < 0 {
        n -= 1;
        f += 0o200000;
    }
    let base = value.raw().saturating_mul(n);
    let frac = xn_over_d(value, f, 0o200000)
        .expect("mu glue conversion stays inside TeX dimension range")
        .quotient
        .raw();
    Scaled::from_raw(base.saturating_add(frac))
}

const fn class_index(class: NoadClass) -> usize {
    match class {
        NoadClass::Ord => 0,
        NoadClass::Op => 1,
        NoadClass::Bin => 2,
        NoadClass::Rel => 3,
        NoadClass::Open => 4,
        NoadClass::Close => 5,
        NoadClass::Punct => 6,
        NoadClass::Inner => 7,
    }
}
