//! Exact checked arithmetic shared by the Appendix G implementation.

use tex_state::scaled::Scaled;

pub(crate) fn add(left: Scaled, right: Scaled) -> Scaled {
    left.checked_add(right)
        .expect("Appendix G scaled addition overflow")
}

pub(crate) fn sub(left: Scaled, right: Scaled) -> Scaled {
    left.checked_sub(right)
        .expect("Appendix G scaled subtraction overflow")
}

pub(crate) fn neg(value: Scaled) -> Scaled {
    value
        .checked_neg()
        .expect("Appendix G scaled negation overflow")
}

pub(crate) fn mul(factor: i32, value: Scaled) -> Scaled {
    let product = i64::from(factor)
        .checked_mul(i64::from(value.raw()))
        .expect("Appendix G multiplication fits the wide domain");
    Scaled::from_raw(i32::try_from(product).expect("Appendix G scaled multiplication overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "Appendix G scaled addition overflow")]
    fn operations_fail_loudly_at_scaled_boundaries() {
        let max = Scaled::from_raw(i32::MAX);
        let one = Scaled::from_raw(1);
        let _ = add(max, one);
    }
}
