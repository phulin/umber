use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_state::token::Catcode;
use tex_state_benchmarks::{WORKLOADS, WorkloadKind, build_workload};

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

criterion_group!(benches, snapshot_capture, detached_code_table_write);
criterion_main!(benches);
