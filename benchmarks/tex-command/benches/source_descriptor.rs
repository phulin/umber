use std::sync::Arc;

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_state::source_map::SourceDescriptor;

const SOURCE_BYTES: usize = 1024 * 1024;

fn source_descriptor(c: &mut Criterion) {
    let bytes = Arc::<[u8]>::from(vec![b'x'; SOURCE_BYTES]);
    let retained = SourceDescriptor::generated(Arc::clone(&bytes));
    let mut group = c.benchmark_group("generated_source_descriptor");
    group.throughput(Throughput::Bytes(SOURCE_BYTES as u64));
    group.bench_function("rebuild_identity", |b| {
        b.iter(|| SourceDescriptor::generated(black_box(Arc::clone(&bytes))))
    });
    group.bench_function("clone_retained_identity", |b| {
        b.iter(|| black_box(&retained).clone())
    });
    group.finish();
}

criterion_group!(benches, source_descriptor);
criterion_main!(benches);
