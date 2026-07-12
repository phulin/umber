use criterion::{BatchSize, Criterion, Throughput, black_box, criterion_group, criterion_main};
use semantic_hash_model::{CurrentSystem, PromotedSystem, Workload};

fn construction(c: &mut Criterion) {
    let workload = Workload::realistic();
    let mut group = c.benchmark_group("semantic_hash_model/construction");
    group.bench_function("current", |b| {
        b.iter(|| black_box(CurrentSystem::build(&workload, false)));
    });
    group.bench_function("promoted_atoms", |b| {
        b.iter(|| black_box(PromotedSystem::build(&workload, false)));
    });
    group.finish();
}

fn complete_session(c: &mut Criterion) {
    let workload = Workload::realistic();
    let mut group = c.benchmark_group("semantic_hash_model/complete_session");
    group.throughput(Throughput::Elements(workload.boundary_count() as u64));
    group.bench_function("current", |b| {
        b.iter_batched(
            || CurrentSystem::build(&workload, false),
            |system| black_box(system.run_session(&workload)),
            BatchSize::SmallInput,
        );
    });
    group.bench_function("lazy_promotion", |b| {
        b.iter_batched(
            || PromotedSystem::build(&workload, false),
            |mut system| black_box(system.run_session(&workload)),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn warm_checkpoint(c: &mut Criterion) {
    let workload = Workload::realistic();
    let current = CurrentSystem::build(&workload, false);
    let mut promoted = PromotedSystem::build(&workload, false);
    promoted.promote_all_session_roots(&workload);
    let boundary = workload.boundary_count() / 2;

    let mut group = c.benchmark_group("semantic_hash_model/warm_checkpoint");
    group.bench_function("current", |b| {
        b.iter(|| black_box(current.hash_boundary_at(&workload, boundary)));
    });
    group.bench_function("promoted", |b| {
        b.iter(|| black_box(promoted.hash_boundary_at(&workload, boundary)));
    });
    group.finish();
}

criterion_group!(benches, construction, complete_session, warm_checkpoint);
criterion_main!(benches);
