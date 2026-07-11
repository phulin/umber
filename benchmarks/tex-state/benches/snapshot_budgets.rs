use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_state_benchmarks::{WORKLOADS, build_workload};

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

criterion_group!(benches, snapshot_capture);
criterion_main!(benches);
