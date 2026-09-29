//! Opt-in release-resolution timing of explicit node-region copies.

use std::hint::black_box;
use std::time::Instant;

use tex_state::node_region::{ExplicitCopyHarness, ExplicitCopyShape};

fn main() {
    let samples = std::env::args()
        .nth(1)
        .map(|value| value.parse::<usize>().expect("sample count"))
        .unwrap_or(9);
    let selected = std::env::args().nth(2);
    for (name, shape, nodes) in [
        ("inline", ExplicitCopyShape::Inline, 4_096),
        ("inline", ExplicitCopyShape::Inline, 16_384),
        ("fixed_annex", ExplicitCopyShape::FixedAnnex, 4_096),
        ("fixed_annex", ExplicitCopyShape::FixedAnnex, 16_384),
        ("nested", ExplicitCopyShape::Nested, 4_096),
        ("variable_span", ExplicitCopyShape::VariableSpan, 4_096),
        ("paragraph", ExplicitCopyShape::Paragraph, 40),
        ("listing", ExplicitCopyShape::Listing, 40),
    ] {
        if selected.as_deref().is_some_and(|selected| selected != name) {
            continue;
        }
        let mut harness = ExplicitCopyHarness::new(shape, nodes);
        let copied_nodes = black_box(harness.copy_once());
        harness.restore();
        let mut timings = Vec::with_capacity(samples);
        for _ in 0..samples {
            #[allow(clippy::disallowed_methods)]
            // The opt-in timing binary measures wall time outside engine state.
            let started = Instant::now();
            black_box(harness.copy_once());
            timings.push(started.elapsed().as_nanos());
            harness.restore();
        }
        timings.sort_unstable();
        let median_ns = timings[timings.len() / 2];
        println!(
            "EXPLICIT_COPY shape={name} root_nodes={nodes} copied_nodes={copied_nodes} samples={samples} median_ns={median_ns} median_ns_per_node={:.2}",
            median_ns as f64 / copied_nodes as f64
        );
    }
}
