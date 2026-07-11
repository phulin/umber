use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_state::token::Catcode;
use tex_state_benchmarks::{
    DEEP_GROUP_LARGE_DEPTH, DEEP_GROUP_SMALL_DEPTH, WORKLOADS, WorkloadKind, build_workload,
    deep_group_code_table_workload,
};

fn snapshot_capture(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_capture");
    for kind in WORKLOADS {
        for (scale, units) in [("small", kind.small_units()), ("large", kind.large_units())] {
            let mut workload = build_workload(kind, units);
            workload.warm_capture();
            group.throughput(Throughput::Bytes(workload.logical_live_bytes()));
            group.bench_function(BenchmarkId::new(kind.name(), scale), |b| {
                b.iter(|| black_box(workload.capture()));
            });
        }
    }
    group.finish();
}

fn detached_code_table_write(c: &mut Criterion) {
    let mut workload = build_workload(
        WorkloadKind::UnicodeCodeTables,
        WorkloadKind::UnicodeCodeTables.large_units(),
    );
    workload.warm_capture();
    let mut value = Catcode::Active;
    c.bench_function("snapshot_update/unicode_code_table_detached_page", |b| {
        b.iter(|| {
            let snapshot = workload.capture();
            workload.set_unicode_catcode('\u{10fffd}', value);
            value = if value == Catcode::Active {
                Catcode::Letter
            } else {
                Catcode::Active
            };
            black_box(snapshot);
        });
    });
}

fn deep_group_global_code_table_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_update/deep_group_global_code_table");
    for depth in [DEEP_GROUP_SMALL_DEPTH, DEEP_GROUP_LARGE_DEPTH] {
        let mut universe = deep_group_code_table_workload(depth);
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, _| {
            b.iter(|| {
                let snapshot = universe.snapshot();
                universe.set_catcode_global('\u{10fffc}', Catcode::Active);
                universe.rollback(&snapshot);
                black_box(&snapshot);
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    snapshot_capture,
    detached_code_table_write,
    deep_group_global_code_table_write
);
criterion_main!(benches);
