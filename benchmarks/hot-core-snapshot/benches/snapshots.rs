use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_state::hot_core_benchmark::TestingHotCore;

const LIVE_WORDS: [usize; 3] = [0, 1_024, 65_536];

fn fixed_size_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("hot_core_fixed_snapshot");
    group.throughput(Throughput::Elements(1));
    for live_words in LIVE_WORDS {
        let mut core = TestingHotCore::with_live_words(live_words);
        group.bench_with_input(
            BenchmarkId::from_parameter(live_words),
            &live_words,
            |b, _| b.iter(|| black_box(core.snapshot_commit())),
        );
    }
    group.finish();
}

fn bounded_rollback(c: &mut Criterion) {
    let mut core = TestingHotCore::with_live_words(0);
    core.warm_bounded_cycle();
    c.bench_function("hot_core_bounded_rollback/all_families", |b| {
        let mut seed = 0_u64;
        b.iter(|| {
            seed = seed.wrapping_add(1);
            black_box(core.rollback_cycle(seed))
        });
    });
}

criterion_group!(benches, fixed_size_snapshot, bounded_rollback);
criterion_main!(benches);
