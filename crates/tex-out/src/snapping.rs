use tex_arith::Scaled;

use crate::{GlueOrder, GlueSpec, PageEffect};

pub(crate) fn initial_reference(effects: &[PageEffect]) -> (Scaled, Scaled) {
    effects
        .iter()
        .find_map(|effect| match effect {
            PageEffect::PdfSnapState { x, y } => Some((*x, *y)),
            _ => None,
        })
        .unwrap_or((Scaled::from_raw(0), Scaled::from_raw(0)))
}

pub(crate) fn correction(current: Scaled, reference: Scaled, spec: GlueSpec) -> Option<Scaled> {
    let width = i64::from(spec.width.raw());
    if width <= 0 {
        return None;
    }
    let relative = i64::from(current.raw()) - i64::from(reference.raw());
    let lower = i64::from(reference.raw()) + relative.div_euclid(width) * width;
    let upper = lower + width;
    let backward = lower - i64::from(current.raw());
    let forward = upper - i64::from(current.raw());
    let backward_allowed =
        spec.shrink_order != GlueOrder::Normal || -backward < i64::from(spec.shrink.raw());
    let forward_allowed =
        spec.stretch_order != GlueOrder::Normal || forward < i64::from(spec.stretch.raw());
    let selected = match (backward_allowed, forward_allowed) {
        (false, false) => return None,
        (true, false) => backward,
        (false, true) => forward,
        (true, true) if -backward < forward => backward,
        (true, true) => forward,
    };
    i32::try_from(selected).ok().map(Scaled::from_raw)
}

pub(crate) fn compensate(correction: Scaled, ratio: u16) -> Scaled {
    let raw = i64::from(correction.raw()) * i64::from(ratio) / 1000;
    Scaled::from_raw(i32::try_from(raw).expect("clamped snapping compensation fits i32"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(
        stretch: i32,
        stretch_order: GlueOrder,
        shrink: i32,
        shrink_order: GlueOrder,
    ) -> GlueSpec {
        GlueSpec {
            width: Scaled::from_raw(10),
            stretch: Scaled::from_raw(stretch),
            stretch_order,
            shrink: Scaled::from_raw(shrink),
            shrink_order,
        }
    }

    #[test]
    fn finite_limits_ties_and_negative_coordinates_choose_the_pinned_neighbor() {
        let flexible = spec(10, GlueOrder::Normal, 10, GlueOrder::Normal);
        assert_eq!(
            correction(Scaled::from_raw(5), Scaled::from_raw(0), flexible),
            Some(Scaled::from_raw(5))
        );
        assert_eq!(
            correction(Scaled::from_raw(6), Scaled::from_raw(0), flexible),
            Some(Scaled::from_raw(4))
        );
        assert_eq!(
            correction(Scaled::from_raw(-6), Scaled::from_raw(0), flexible),
            Some(Scaled::from_raw(-4))
        );
        let bounded = spec(2, GlueOrder::Normal, 2, GlueOrder::Normal);
        assert_eq!(
            correction(Scaled::from_raw(5), Scaled::from_raw(0), bounded),
            None
        );
        let exact_limit = spec(5, GlueOrder::Normal, 5, GlueOrder::Normal);
        assert_eq!(
            correction(Scaled::from_raw(5), Scaled::from_raw(0), exact_limit),
            None,
            "finite flex is a strict bound"
        );
    }

    #[test]
    fn infinite_flex_and_compensation_clamping_are_exact() {
        let forward = spec(0, GlueOrder::Fil, 0, GlueOrder::Normal);
        assert_eq!(
            correction(Scaled::from_raw(6), Scaled::from_raw(0), forward),
            Some(Scaled::from_raw(4))
        );
        assert_eq!(compensate(Scaled::from_raw(-9), 500), Scaled::from_raw(-4));
        assert_eq!(compensate(Scaled::from_raw(9), 1000), Scaled::from_raw(9));
    }
}
