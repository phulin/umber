use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use tex_state::interner::InternerBudget;
use tex_state::measurement::{
    HotCoreAllocationMeasurement, HotCoreAllocationOwner, HotCoreAllocator, hot_core_census,
};
use tex_state::{DetachedFormatImage, World, with_materialized_format};

const PLAIN_FORMAT: &[u8] = include_bytes!("../../../crates/umber-wasm/assets/plain.fmt");

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

fn budget() -> InternerBudget {
    InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024).expect("benchmark interner budget")
}

fn materialize(image: &DetachedFormatImage) {
    with_materialized_format(budget(), World::memory(), image, |universe| {
        black_box(universe.interaction_mode());
        black_box(
            universe
                .state_journal_bytes()
                .expect("materialized format has a live state journal"),
        );
    })
    .expect("pinned Plain format must materialize");
}

fn allocation_delta(
    after: HotCoreAllocationMeasurement,
    before: HotCoreAllocationMeasurement,
) -> HotCoreAllocationMeasurement {
    HotCoreAllocationMeasurement {
        calls: after.calls.saturating_sub(before.calls),
        requested_bytes: after.requested_bytes.saturating_sub(before.requested_bytes),
    }
}

fn decode(c: &mut Criterion) {
    // Validation is an explicit detached-input boundary and happens once. The
    // measured operation is destination-local materialization of that already
    // validated image.
    let image = DetachedFormatImage::try_from_bytes(PLAIN_FORMAT.to_vec())
        .expect("pinned Plain format must validate");
    assert_eq!(image.as_bytes(), PLAIN_FORMAT);
    materialize(&image);

    let before = hot_core_census();
    materialize(&image);
    let after = hot_core_census();
    let cold = allocation_delta(
        after.allocations[HotCoreAllocationOwner::ColdMaterialization as usize],
        before.allocations[HotCoreAllocationOwner::ColdMaterialization as usize],
    );
    let generation = allocation_delta(
        after.allocations[HotCoreAllocationOwner::GenerationBoundary as usize],
        before.allocations[HotCoreAllocationOwner::GenerationBoundary as usize],
    );
    eprintln!(
        "FORMAT_MATERIALIZATION_BENCH image_bytes={} cold_calls={} cold_bytes={} generation_calls={} generation_bytes={}",
        image.as_bytes().len(),
        cold.calls,
        cold.requested_bytes,
        generation.calls,
        generation.requested_bytes,
    );

    c.bench_function("loaded_format_materialization/plain", |b| {
        b.iter(|| materialize(black_box(&image)));
    });
}

criterion_group!(benches, decode);
criterion_main!(benches);
