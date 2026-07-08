use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use tex_state::Universe;
use tex_state::meaning::Meaning;

const GROUP_SIZES: [usize; 3] = [4, 64, 512];
const ROLLBACK_TOTAL_CELLS: [usize; 2] = [1024, 4096];
const ROLLBACK_SLICE_WRITES: [usize; 3] = [4, 64, 512];
const PAGE_DISTINCT_CELLS: usize = 500;
const PAGE_TOTAL_WRITES: usize = 5_000;

fn meaning_lookup(c: &mut Criterion) {
    let mut stores = Universe::new();
    let symbol = stores.intern("warm-cell");
    stores.set_meaning(symbol, Meaning::Relax);

    c.bench_function("meaning_lookup/warm_cell_hit", |b| {
        b.iter(|| black_box(stores.meaning(black_box(symbol))));
    });
}

fn barrier_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("barrier_write");

    group.bench_function("journal_push_path", |b| {
        let mut stores = Universe::new();
        let symbol = stores.intern("push-path");
        let mut operand = 0_u64;

        b.iter(|| {
            operand = operand.wrapping_add(1);
            stores.set_meaning(black_box(symbol), black_box(raw_meaning(operand)));
            stores.enter_group();
            black_box(stores.leave_group());
        });

        black_box(stores);
    });

    group.bench_function("already_stamped_skip_path", |b| {
        let mut stores = Universe::new();
        let symbol = stores.intern("skip-path");
        stores.set_meaning(symbol, Meaning::Relax);
        let mut operand = 0_u64;

        b.iter(|| {
            operand = operand.wrapping_add(1);
            stores.set_meaning(black_box(symbol), black_box(raw_meaning(operand)));
        });

        black_box(stores);
    });

    group.finish();
}

fn snapshot_take(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_take");

    group.bench_function("steady_empty_slice", |b| {
        let mut stores = Universe::new();
        b.iter(|| {
            let snapshot = stores.snapshot();
            black_box(snapshot.state_hash());
        });
        black_box(stores);
    });

    group.finish();
}

fn checkpoint_state_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("checkpoint_state_hash");

    group.bench_function("after_synthetic_page", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let symbols = synthetic_page_symbols(&mut stores);
                for write_index in 0..PAGE_TOTAL_WRITES {
                    let symbol = symbols[write_index % symbols.len()];
                    stores.set_meaning(symbol, raw_meaning(write_index as u64));
                }
                stores
            },
            |mut stores| {
                let snapshot = stores.snapshot();
                black_box(snapshot.state_hash());
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn group_cycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("group_cycle");

    for write_count in GROUP_SIZES {
        group.throughput(Throughput::Elements(write_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(write_count),
            &write_count,
            |b, &write_count| {
                let mut stores = Universe::new();
                let symbols = (0..write_count)
                    .map(|index| stores.intern(&format!("group-cell-{index}")))
                    .collect::<Vec<_>>();

                b.iter(|| {
                    stores.enter_group();
                    for (index, &symbol) in symbols.iter().enumerate() {
                        stores.set_meaning(black_box(symbol), black_box(raw_meaning(index as u64)));
                    }
                    black_box(stores.leave_group());
                });

                black_box(stores);
            },
        );
    }

    group.finish();
}

fn rollback_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("rollback_scaling");

    for total_cells in ROLLBACK_TOTAL_CELLS {
        for slice_writes in ROLLBACK_SLICE_WRITES {
            group.throughput(Throughput::Elements(slice_writes as u64));
            group.bench_with_input(
                BenchmarkId::new(format!("total_{total_cells}"), slice_writes),
                &(total_cells, slice_writes),
                |b, &(total_cells, slice_writes)| {
                    b.iter_batched_ref(
                        || rollback_case(total_cells, slice_writes),
                        |(stores, snapshot)| {
                            stores.rollback(black_box(snapshot));
                            black_box(stores);
                        },
                        BatchSize::SmallInput,
                    );
                },
            );
        }
    }

    group.finish();
}

fn group_global_compaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("group_global_compaction");

    group.bench_function("mixed_global_local_same_cell", |b| {
        let mut stores = Universe::new();
        let symbol = stores.intern("global-compaction-cell");
        let mut operand = 0_u64;

        b.iter(|| {
            operand = operand.wrapping_add(1);
            stores.enter_group();
            stores.set_meaning(black_box(symbol), black_box(raw_meaning(operand)));
            stores.set_meaning_global(black_box(symbol), black_box(raw_meaning(operand + 1)));
            stores.set_meaning(black_box(symbol), black_box(raw_meaning(operand + 2)));
            stores.set_meaning_global(black_box(symbol), black_box(raw_meaning(operand + 3)));
            black_box(stores.leave_group());
        });

        black_box(stores);
    });

    group.finish();
}

fn synthetic_page_journal_volume(c: &mut Criterion) {
    let bytes = synthetic_page_journal_bytes();
    let mut group = c.benchmark_group("synthetic_page");
    group.throughput(Throughput::Bytes(bytes as u64));

    group.bench_function("500_distinct_cells_5000_total_writes", |b| {
        b.iter(|| {
            let bytes = write_synthetic_page();
            black_box(bytes);
        });
    });

    group.finish();
}

fn synthetic_page_journal_bytes() -> usize {
    write_synthetic_page()
}

fn synthetic_page_symbols(stores: &mut Universe) -> Vec<tex_state::interner::Symbol> {
    (0..PAGE_DISTINCT_CELLS)
        .map(|index| stores.intern(&format!("page-cell-{index}")))
        .collect()
}

fn write_synthetic_page() -> usize {
    let mut stores = Universe::new();
    let symbols = synthetic_page_symbols(&mut stores);
    let snapshot = stores.snapshot();

    for write_index in 0..PAGE_TOTAL_WRITES {
        let symbol = symbols[write_index % symbols.len()];
        stores.set_meaning(
            black_box(symbol),
            black_box(raw_meaning(write_index as u64)),
        );
    }

    stores.env_journal_bytes_since(&snapshot)
}

fn rollback_case(total_cells: usize, slice_writes: usize) -> (Universe, tex_state::Snapshot) {
    let mut stores = Universe::new();
    let symbols = (0..total_cells)
        .map(|index| stores.intern(&format!("rollback-cell-{index}")))
        .collect::<Vec<_>>();

    for (index, &symbol) in symbols.iter().enumerate() {
        stores.set_meaning(symbol, raw_meaning(index as u64));
    }

    let snapshot = stores.snapshot();
    for (write_index, &symbol) in symbols.iter().take(slice_writes).enumerate() {
        stores.set_meaning(symbol, raw_meaning((write_index + total_cells) as u64));
    }

    (stores, snapshot)
}

fn raw_meaning(operand: u64) -> Meaning {
    Meaning::CharGiven(char::from_u32(32 + (operand as u32 % 95)).expect("ASCII graphic"))
}

criterion_group!(
    benches,
    meaning_lookup,
    barrier_write,
    snapshot_take,
    checkpoint_state_hash,
    group_cycle,
    rollback_scaling,
    group_global_compaction,
    synthetic_page_journal_volume
);
criterion_main!(benches);
