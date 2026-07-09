use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use tex_expand::{get_x_token, install_expandable_primitives};
use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
use tex_state::SourceId;
use tex_state::Universe;
use tex_state::glue::Order;
use tex_state::ids::OriginListId;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::Meaning;
use tex_state::meaning::MeaningFlags;
use tex_state::node::{BoxNode, BoxNodeFields, KernKind, Node, Sign};
use tex_state::provenance::ProvenanceStats;
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::{Catcode, Token};

const GROUP_SIZES: [usize; 3] = [4, 64, 512];
const ROLLBACK_TOTAL_CELLS: [usize; 2] = [1024, 4096];
const ROLLBACK_SLICE_WRITES: [usize; 3] = [4, 64, 512];
const PAGE_DISTINCT_CELLS: usize = 500;
const PAGE_TOTAL_WRITES: usize = 5_000;
const SOURCE_HEAVY_LINES: usize = 512;
const SOURCE_HEAVY_LINE: &str = "alpha beta gamma delta epsilon zeta eta theta";
const MACRO_CALLS: usize = 2_048;
const MACRO_BODY_LEN: usize = 16;
const SCANNER_REPETITIONS: usize = 1_024;
const TRANSIENT_BOX_OVERWRITES: usize = 20_000;

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

fn transient_box_overwrite_checkpoint(c: &mut Criterion) {
    c.bench_function("checkpoint_state_hash/transient_box_overwrites", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let _ = stores.snapshot();
                for amount in 0..TRANSIENT_BOX_OVERWRITES {
                    let children = stores.freeze_node_list(&[Node::Kern {
                        amount: Scaled::from_raw(amount as i32),
                        kind: KernKind::Explicit,
                    }]);
                    let list =
                        stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
                            width: Scaled::from_raw(amount as i32),
                            height: Scaled::from_raw(0),
                            depth: Scaled::from_raw(0),
                            shift: Scaled::from_raw(0),
                            display: false,
                            glue_set: GlueSetRatio::ZERO,
                            glue_sign: Sign::Normal,
                            glue_order: Order::Normal,
                            children,
                        }))]);
                    stores.set_box_reg(0, list);
                }
                stores
            },
            |mut stores| black_box(stores.snapshot().state_hash()),
            BatchSize::LargeInput,
        );
    });
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

fn provenance_source_lexing(c: &mut Criterion) {
    let input = source_heavy_text();
    let token_count = source_heavy_token_count(&input);
    let mut group = c.benchmark_group("provenance_source_lexing");
    group.throughput(Throughput::Elements(token_count as u64));

    group.bench_function("semantic_only_readonly", |b| {
        b.iter_batched(
            || {
                (
                    Universe::new(),
                    InputStack::new(MemoryInput::new(input.clone())),
                )
            },
            |(stores, mut input)| {
                let mut count = 0_usize;
                while let Some(token) = input
                    .next_token_readonly(&stores)
                    .expect("source lexing should succeed")
                {
                    black_box(token);
                    count += 1;
                }
                black_box(count);
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("traced_source_origins", |b| {
        b.iter_batched(
            || {
                (
                    Universe::new(),
                    InputStack::new(MemoryInput::new(input.clone())),
                )
            },
            |(mut stores, mut input)| {
                let before = stores.provenance_stats();
                let mut count = 0_usize;
                while let Some(token) = input
                    .next_traced_token(&mut stores)
                    .expect("source lexing should succeed")
                {
                    black_box(token);
                    count += 1;
                }
                black_box((count, stores.provenance_stats().saturating_sub(before)));
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn provenance_expansion(c: &mut Criterion) {
    let mut group = c.benchmark_group("provenance_expansion");
    group.throughput(Throughput::Elements(MACRO_CALLS as u64));

    group.bench_function("macro_body_replay_invocation_origins", |b| {
        b.iter_batched(
            macro_heavy_case,
            |(mut stores, mut input, baseline)| {
                let count = drain_expansion(&mut stores, &mut input);
                black_box((count, stores.provenance_stats().saturating_sub(baseline)));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("scanner_number_runs", |b| {
        b.iter_batched(
            scanner_heavy_case,
            |(mut stores, mut input, baseline)| {
                let count = drain_expansion(&mut stores, &mut input);
                black_box((count, stores.provenance_stats().saturating_sub(baseline)));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("generated_value_origin_sharing", |b| {
        b.iter_batched(
            generated_run_case,
            |(mut stores, mut input, baseline)| {
                let count = drain_expansion(&mut stores, &mut input);
                black_box((count, stores.provenance_stats().saturating_sub(baseline)));
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn provenance_memory_invariants(c: &mut Criterion) {
    let mut group = c.benchmark_group("provenance_memory");

    group.bench_function("macro_long_run_arena_growth", |b| {
        b.iter(|| black_box(macro_long_run_growth()));
    });

    group.bench_function("rollback_truncates_discarded_fork", |b| {
        b.iter(|| black_box(discarded_fork_growth_after_rollback()));
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

fn source_heavy_text() -> String {
    let mut input = String::new();
    for _ in 0..SOURCE_HEAVY_LINES {
        input.push_str(SOURCE_HEAVY_LINE);
        input.push('\n');
    }
    input
}

fn source_heavy_token_count(input: &str) -> usize {
    let stores = Universe::new();
    let mut stack = InputStack::new(MemoryInput::new(input.to_owned()));
    let mut count = 0;
    while stack
        .next_token_readonly(&stores)
        .expect("source lexing should succeed")
        .is_some()
    {
        count += 1;
    }
    count
}

fn macro_heavy_case() -> (Universe, InputStack<MemoryInput>, ProvenanceStats) {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("hotmacro");
    let params = stores.intern_token_list(&[]);
    let body_tokens = (0..MACRO_BODY_LEN)
        .map(|index| char_token(char::from(b'a' + (index % 26) as u8)))
        .collect::<Vec<_>>();
    let body = stores.intern_token_list(&body_tokens);
    let definition_origin = stores.source_origin(SourceId::new(1), 0, 1, 1);
    let body_origins = stores.allocate_repeated_origin_list(definition_origin, body_tokens.len());
    stores.set_macro_meaning_with_provenance(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        MacroDefinitionProvenance::new(definition_origin, OriginListId::EMPTY, body_origins),
    );

    let call_tokens = vec![Token::Cs(macro_cs); MACRO_CALLS];
    let calls = stores.intern_token_list(&call_tokens);
    let call_origin = stores.source_origin(SourceId::new(1), 80, 2, 1);
    let call_origins = stores.allocate_repeated_origin_list(call_origin, call_tokens.len());
    let baseline = stores.provenance_stats();
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(calls, call_origins, TokenListReplayKind::Inserted);
    (stores, input, baseline)
}

fn scanner_heavy_case() -> (Universe, InputStack<MemoryInput>, ProvenanceStats) {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    let number = stores.symbol("number").expect("number primitive");
    let mut tokens = Vec::with_capacity(SCANNER_REPETITIONS * 7);
    for _ in 0..SCANNER_REPETITIONS {
        tokens.push(Token::Cs(number));
        for digit in ['1', '2', '3', '4', '5'] {
            tokens.push(char_token(digit));
        }
        tokens.push(space_token());
    }
    traced_token_list_input(stores, tokens)
}

fn generated_run_case() -> (Universe, InputStack<MemoryInput>, ProvenanceStats) {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    let roman = stores
        .symbol("romannumeral")
        .expect("romannumeral primitive");
    let mut tokens = Vec::with_capacity(SCANNER_REPETITIONS * 6);
    for _ in 0..SCANNER_REPETITIONS {
        tokens.push(Token::Cs(roman));
        for digit in ['3', '8', '8', '8'] {
            tokens.push(char_token(digit));
        }
        tokens.push(space_token());
    }
    traced_token_list_input(stores, tokens)
}

fn traced_token_list_input(
    mut stores: Universe,
    tokens: Vec<Token>,
) -> (Universe, InputStack<MemoryInput>, ProvenanceStats) {
    let token_list = stores.intern_token_list(&tokens);
    let origin = stores.source_origin(SourceId::new(2), 0, 1, 1);
    let origins = stores.allocate_repeated_origin_list(origin, tokens.len());
    let baseline = stores.provenance_stats();
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(token_list, origins, TokenListReplayKind::Inserted);
    (stores, input, baseline)
}

fn drain_expansion(stores: &mut Universe, input: &mut InputStack<MemoryInput>) -> usize {
    let mut count = 0;
    while let Some(token) = get_x_token(input, stores).expect("expansion should succeed") {
        black_box(token);
        count += 1;
    }
    count
}

fn macro_long_run_growth() -> ProvenanceStats {
    let (mut stores, mut input, baseline) = macro_heavy_case();
    let count = drain_expansion(&mut stores, &mut input);
    assert_eq!(count, MACRO_CALLS * MACRO_BODY_LEN);
    stores.provenance_stats().saturating_sub(baseline)
}

fn discarded_fork_growth_after_rollback() -> ProvenanceStats {
    let (mut stores, mut input, baseline) = generated_run_case();
    let snapshot = stores.snapshot();
    let _ = drain_expansion(&mut stores, &mut input);
    stores.rollback(&snapshot);
    stores.provenance_stats().saturating_sub(baseline)
}

fn char_token(ch: char) -> Token {
    let cat = match ch {
        '0'..='9' | '[' | ']' | '!' | '<' | '=' | '>' | '-' => Catcode::Other,
        _ => Catcode::Letter,
    };
    Token::Char { ch, cat }
}

fn space_token() -> Token {
    Token::Char {
        ch: ' ',
        cat: Catcode::Space,
    }
}

criterion_group!(
    benches,
    meaning_lookup,
    barrier_write,
    snapshot_take,
    checkpoint_state_hash,
    transient_box_overwrite_checkpoint,
    group_cycle,
    rollback_scaling,
    group_global_compaction,
    synthetic_page_journal_volume,
    provenance_source_lexing,
    provenance_expansion,
    provenance_memory_invariants
);
criterion_main!(benches);
