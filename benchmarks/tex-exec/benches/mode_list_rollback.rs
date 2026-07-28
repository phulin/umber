use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

const RETAINED_NODES: usize = 16_384;
const STEPS: usize = 1_024;

fn node(index: usize) -> Node {
    Node::Kern {
        amount: Scaled::from_raw(index as i32),
        kind: KernKind::Explicit,
    }
}

fn mode_list_rollback(c: &mut Criterion) {
    let retained = (0..RETAINED_NODES).map(node).collect::<Vec<_>>();
    let mut group = c.benchmark_group("mode_list_rollback");
    group.throughput(Throughput::Elements(STEPS as u64));

    group.bench_function("cow_snapshot_successful_appends", |b| {
        b.iter_batched(
            || Arc::new(retained.clone()),
            |mut live| {
                for index in 0..STEPS {
                    let rollback_root = Arc::clone(&live);
                    Arc::make_mut(&mut live).push(node(index));
                    black_box(rollback_root);
                }
                black_box(live)
            },
            criterion::BatchSize::SmallInput,
        )
    });

    for prefix_len in [0, RETAINED_NODES, RETAINED_NODES * 4] {
        group.bench_with_input(
            BenchmarkId::new("journal_successful_appends", prefix_len),
            &prefix_len,
            |b, &prefix_len| {
                let mut live = Vec::with_capacity(prefix_len + STEPS);
                live.extend((0..prefix_len).map(node));
                b.iter(|| {
                    let rollback_length = live.len();
                    for index in 0..STEPS {
                        live.push(node(index));
                    }
                    black_box(&live);
                    live.truncate(rollback_length);
                })
            },
        );
    }
    group.finish();
}

criterion_group!(benches, mode_list_rollback);
criterion_main!(benches);
