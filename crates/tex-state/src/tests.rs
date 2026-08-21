#[cfg(feature = "testing")]
mod replay;

mod node_semantics;

#[test]
fn etex_math_boundary_stack_counts_missing_and_extra_by_identity() {
    // Merged e-TeX WEB §53a counts unmatched ends as extra and open begins
    // remaining at list end as missing, while properly nested M/L/R pairs cancel.
    use crate::node::{LrAnomalies, MathBoundary as B, MathBoundaryStack};

    let mut matched = MathBoundaryStack::default();
    for boundary in [B::BeginL, B::BeginR, B::EndR, B::EndL] {
        matched.observe(boundary);
    }
    assert_eq!(matched.finish(), LrAnomalies::default());

    let mut anomalous = MathBoundaryStack::default();
    for boundary in [B::BeginM, B::BeginR, B::EndL, B::EndM] {
        anomalous.observe(boundary);
    }
    assert_eq!(
        anomalous.finish(),
        LrAnomalies {
            missing: 2,
            extra: 2,
        }
    );
}

#[test]
fn smoke() {
    assert!(!env!("CARGO_PKG_NAME").is_empty());
}
