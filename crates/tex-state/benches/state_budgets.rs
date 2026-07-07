use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_state::env::Env;
use tex_state::interner::Interner;
use tex_state::meaning::Meaning;
use tex_state::stores::Stores;

const GROUP_SIZES: [usize; 3] = [4, 64, 512];
const PAGE_DISTINCT_CELLS: usize = 500;
const PAGE_TOTAL_WRITES: usize = 5_000;

fn meaning_lookup(c: &mut Criterion) {
    let mut interner = Interner::new();
    let symbol = interner.intern("warm-cell");
    let mut env = Env::new();
    env.set(symbol, Meaning::Relax);

    c.bench_function("meaning_lookup/warm_cell_hit", |b| {
        b.iter(|| black_box(env.get(black_box(symbol))));
    });
}

fn barrier_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("barrier_write");

    group.bench_function("journal_push_path", |b| {
        let mut interner = Interner::new();
        let symbol = interner.intern("push-path");
        let mut env = Env::new();
        let mut operand = 0_u64;

        b.iter(|| {
            operand = operand.wrapping_add(1);
            env.set(black_box(symbol), black_box(raw_meaning(operand)));
            env.bump_epoch();
        });

        black_box(env);
    });

    group.bench_function("already_stamped_skip_path", |b| {
        let mut interner = Interner::new();
        let symbol = interner.intern("skip-path");
        let mut env = Env::new();
        env.set(symbol, Meaning::Relax);
        let mut operand = 0_u64;

        b.iter(|| {
            operand = operand.wrapping_add(1);
            env.set(black_box(symbol), black_box(raw_meaning(operand)));
        });

        black_box(env);
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
                let mut interner = Interner::new();
                let symbols = (0..write_count)
                    .map(|index| interner.intern(&format!("group-cell-{index}")))
                    .collect::<Vec<_>>();
                let mut env = Env::new();

                b.iter(|| {
                    env.enter_group();
                    for (index, &symbol) in symbols.iter().enumerate() {
                        env.set(black_box(symbol), black_box(raw_meaning(index as u64)));
                    }
                    black_box(env.leave_group());
                });

                black_box(env);
            },
        );
    }

    group.finish();
}

fn synthetic_page_journal_volume(c: &mut Criterion) {
    let bytes = synthetic_page_journal_bytes();
    let mut group = c.benchmark_group("synthetic_page");
    group.throughput(Throughput::Bytes(bytes as u64));

    group.bench_function("500_distinct_cells_5000_total_writes", |b| {
        let symbols = synthetic_page_symbols();

        b.iter(|| {
            let bytes = write_synthetic_page(&symbols);
            black_box(bytes);
        });
    });

    group.finish();
}

fn synthetic_page_journal_bytes() -> usize {
    write_synthetic_page(&synthetic_page_symbols())
}

fn synthetic_page_symbols() -> Vec<tex_state::interner::Symbol> {
    let mut interner = Interner::new();
    (0..PAGE_DISTINCT_CELLS)
        .map(|index| interner.intern(&format!("page-cell-{index}")))
        .collect()
}

fn write_synthetic_page(symbols: &[tex_state::interner::Symbol]) -> usize {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();

    for write_index in 0..PAGE_TOTAL_WRITES {
        let symbol = symbols[write_index % symbols.len()];
        stores.with_env_mut(|env| {
            env.set(
                black_box(symbol),
                black_box(raw_meaning(write_index as u64)),
            );
        });
    }

    stores.env_journal_bytes_since(snapshot)
}

fn raw_meaning(operand: u64) -> Meaning {
    Meaning::CharGiven(char::from_u32(32 + (operand as u32 % 95)).expect("ASCII graphic"))
}

criterion_group!(
    benches,
    meaning_lookup,
    barrier_write,
    group_cycle,
    synthetic_page_journal_volume
);
criterion_main!(benches);
