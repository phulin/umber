//! Exact overflow policy shared by the Appendix G implementation.
//!
//! TeX's legal scaled dimensions preserve their integer operation order.
//! Values outside the representable `Scaled` domain saturate consistently
//! instead of depending on debug/release overflow behavior.

use tex_state::scaled::Scaled;

pub(crate) fn add(left: Scaled, right: Scaled) -> Scaled {
    tex_arith::saturating_add(left, right)
}

pub(crate) fn sub(left: Scaled, right: Scaled) -> Scaled {
    tex_arith::saturating_sub(left, right)
}

pub(crate) fn neg(value: Scaled) -> Scaled {
    Scaled::from_raw(value.raw().saturating_neg())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_saturate_at_scaled_boundaries() {
        let max = Scaled::from_raw(i32::MAX);
        let min = Scaled::from_raw(i32::MIN);
        let one = Scaled::from_raw(1);
        assert_eq!(add(max, one), max);
        assert_eq!(sub(min, one), min);
        assert_eq!(neg(min), max);
        assert_eq!(neg(max), Scaled::from_raw(-i32::MAX));
    }
}
