use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use tex_expand::{get_x_token, install_expandable_primitives};
use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
use tex_state::ProvenanceResolver;
use tex_state::SourceId;
use tex_state::Universe;
use tex_state::glue::Order;
use tex_state::ids::OriginListId;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::math::{MathChar, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::meaning::Meaning;
use tex_state::meaning::MeaningFlags;
use tex_state::node::{BoxNode, BoxNodeFields, KernKind, Node, Sign};
use tex_state::provenance::ProvenanceStats;
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::{Catcode, Token, TracedTokenWord};

const GROUP_SIZES: [usize; 3] = [4, 64, 512];
const ROLLBACK_TOTAL_CELLS: [usize; 2] = [1024, 4096];
const ROLLBACK_SLICE_WRITES: [usize; 3] = [4, 64, 512];
const PAGE_DISTINCT_CELLS: usize = 500;
const PAGE_TOTAL_WRITES: usize = 5_000;
const SOURCE_HEAVY_LINES: usize = 512;
const SOURCE_HEAVY_LINE: &str = "alpha beta gamma delta epsilon zeta eta theta";
const MIXED_UTF8_LINE: &str = "alpha βήτα 世界 café naïve 🦀 zeta";
const CONTROL_SEQUENCE_LINE: &str = "\\alpha\\beta\\gamma\\delta\\epsilon\\zeta\\eta\\theta";
const LONG_LINE_SCALARS: usize = 65_536;
const MACRO_CALLS: usize = 2_048;
const MACRO_BODY_LEN: usize = 16;
const SCANNER_REPETITIONS: usize = 1_024;
const TRANSIENT_BOX_OVERWRITES: usize = 20_000;
const DEEP_BOX_LOCALITY_JOURNAL: usize = 20_000;
const ALLOCATION_GRAPH_DEPTH: usize = 128;
const ALLOCATION_LIST_LEN: usize = 1_024;
const PAGE_QUEUE_LEN: usize = 65_536;

fn page_contribution_queue(c: &mut Criterion) {
    c.bench_function("page_contribution_queue/drain_65536", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                for index in 0..PAGE_QUEUE_LEN {
                    stores.append_page_contribution(Node::Penalty(index as i32));
                }
                stores
            },
            |mut stores| {
                while let Some(node) = stores.pop_page_contribution_front() {
                    black_box(node);
                }
            },
            BatchSize::LargeInput,
        );
    });
}

fn allocation_node_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_node_append");
    for shape in ["inline", "box", "math", "mixed"] {
        group.throughput(Throughput::Elements(ALLOCATION_LIST_LEN as u64));
        group.bench_function(shape, |b| {
            b.iter_batched(
                || allocation_append_case(shape),
                |(mut stores, nodes)| black_box(stores.freeze_node_list(&nodes)),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn allocation_append_case(shape: &str) -> (Universe, Vec<Node>) {
    let mut stores = Universe::new();
    let empty = stores.freeze_node_list(&[]);
    let font = stores.current_font();
    let nodes = (0..ALLOCATION_LIST_LEN)
        .map(|index| match shape {
            "inline" => Node::Kern {
                amount: Scaled::from_raw(index as i32),
                kind: KernKind::Explicit,
            },
            "box" => Node::HList(benchmark_box(empty, index as i32)),
            "math" => Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::MathChar(MathChar {
                    family: (index % 16) as u8,
                    character: char::from(b'a' + (index % 26) as u8),
                }),
            )),
            "mixed" => match index % 4 {
                0 => Node::Char {
                    font,
                    ch: char::from(b'a' + (index % 26) as u8),
                },
                1 => Node::HList(benchmark_box(empty, index as i32)),
                2 => Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::Empty,
                )),
                _ => Node::Rule {
                    width: Some(Scaled::from_raw(index as i32)),
                    height: None,
                    depth: None,
                },
            },
            _ => unreachable!("known allocation append shape"),
        })
        .collect();
    (stores, nodes)
}

fn allocation_graph_transfer(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_graph_transfer");

    group.bench_function("promote_fresh/deep", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let root = deep_epoch_graph(&mut stores, ALLOCATION_GRAPH_DEPTH);
                (stores, root)
            },
            |(mut stores, root)| {
                stores.set_box_reg(0, root);
                black_box(stores.box_reg(0))
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("promote_recycled/deep", |b| {
        b.iter_batched(
            recycled_promotion_case,
            |(mut stores, root)| {
                stores.set_box_reg(0, root);
                black_box(stores.box_reg(0))
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("promote_fresh/shared_dag", |b| {
        b.iter_batched(
            shared_graph_case,
            |(mut stores, root)| {
                stores.set_box_reg(0, root);
                black_box(stores.box_reg(0))
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("promote_fresh/mixed_ownership", |b| {
        b.iter_batched(
            mixed_ownership_case,
            |(mut stores, root)| {
                stores.set_box_reg(1, root);
                black_box(stores.box_reg(1))
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("clone_to_epoch/deep", |b| {
        b.iter_batched(
            survivor_deep_graph_case,
            |(mut stores, root)| black_box(stores.clone_node_list_to_epoch(root)),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn deep_journal_box_locality(c: &mut Criterion) {
    c.bench_function("box_locality/same_level_after_20000_entries", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let value = stores.freeze_node_list(&[Node::Penalty(1)]);
                stores.enter_group();
                stores.set_box_reg(0, value);
                for write in 0..DEEP_BOX_LOCALITY_JOURNAL {
                    let symbol = stores.intern(&format!("deepbox{write}"));
                    stores.set_meaning(symbol, Meaning::Relax);
                }
                stores
            },
            |mut stores| {
                let value = stores.box_reg(0).expect("benchmark box remains live");
                for _ in 0..1_000 {
                    stores.set_box_reg_same_level(0, value);
                }
                black_box(stores.box_reg(0))
            },
            BatchSize::LargeInput,
        );
    });
}

fn allocation_traced_freeze(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_traced_freeze");
    for &(name, len, preintern) in &[
        ("short_miss", 8, false),
        ("short_hit", 8, true),
        ("long_miss", 1_024, false),
        ("long_hit_distinct_origins", 1_024, true),
    ] {
        group.throughput(Throughput::Elements(len as u64));
        group.bench_function(name, |b| {
            b.iter_batched(
                || traced_freeze_case(len, preintern),
                |(mut stores, traced)| {
                    black_box(stores.finish_traced_token_list(&traced));
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn benchmark_box(children: tex_state::ids::NodeListId, value: i32) -> BoxNode {
    BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(value),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    })
}

fn deep_epoch_graph(stores: &mut Universe, depth: usize) -> tex_state::ids::NodeListId {
    let mut child = stores.freeze_node_list(&[Node::Penalty(0)]);
    for level in 0..depth {
        child = stores.freeze_node_list(&[Node::HList(benchmark_box(child, level as i32))]);
    }
    child
}

fn recycled_promotion_case() -> (Universe, tex_state::ids::NodeListId) {
    let mut stores = Universe::new();
    let first = deep_epoch_graph(&mut stores, ALLOCATION_GRAPH_DEPTH);
    stores.set_box_reg(0, first);
    let second = deep_epoch_graph(&mut stores, ALLOCATION_GRAPH_DEPTH / 2);
    stores.set_box_reg(0, second);
    let third = deep_epoch_graph(&mut stores, ALLOCATION_GRAPH_DEPTH);
    (stores, third)
}

fn shared_graph_case() -> (Universe, tex_state::ids::NodeListId) {
    let mut stores = Universe::new();
    let shared = deep_epoch_graph(&mut stores, 16);
    let root = stores.freeze_node_list(&[
        Node::HList(benchmark_box(shared, 1)),
        Node::VList(benchmark_box(shared, 2)),
    ]);
    (stores, root)
}

fn mixed_ownership_case() -> (Universe, tex_state::ids::NodeListId) {
    let mut stores = Universe::new();
    let survivor_source = deep_epoch_graph(&mut stores, 16);
    stores.set_box_reg(0, survivor_source);
    let survivor = stores.box_reg(0).expect("survivor should be live");
    let epoch = deep_epoch_graph(&mut stores, 16);
    let root = stores.freeze_node_list(&[
        Node::HList(benchmark_box(survivor, 1)),
        Node::VList(benchmark_box(epoch, 2)),
    ]);
    (stores, root)
}

fn survivor_deep_graph_case() -> (Universe, tex_state::ids::NodeListId) {
    let mut stores = Universe::new();
    let epoch = deep_epoch_graph(&mut stores, ALLOCATION_GRAPH_DEPTH);
    stores.set_box_reg(0, epoch);
    let survivor = stores.box_reg(0).expect("survivor should be live");
    (stores, survivor)
}

fn traced_freeze_case(len: usize, preintern: bool) -> (Universe, Vec<TracedTokenWord>) {
    let mut stores = Universe::new();
    let semantic = (0..len)
        .map(|index| char_token(char::from(b'a' + (index % 26) as u8)))
        .collect::<Vec<_>>();
    if preintern {
        stores.intern_token_list(&semantic);
    }
    let mut traced = Vec::with_capacity(len);
    for (index, token) in semantic.into_iter().enumerate() {
        let origin = stores.source_origin(SourceId::new(7), index as u64, 1, index as u32 + 1);
        traced.push(TracedTokenWord::pack(token, origin));
    }
    (stores, traced)
}

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

    group.bench_function("font_dense_box_tree", |b| {
        b.iter_batched(
            || {
                let mut stores = Universe::new();
                let _ = stores.snapshot();
                let font = stores.current_font();
                let chars = (0..ALLOCATION_LIST_LEN)
                    .map(|index| Node::Char {
                        font,
                        ch: char::from(b'a' + (index % 26) as u8),
                    })
                    .collect::<Vec<_>>();
                let list = stores.freeze_node_list(&chars);
                stores.set_box_reg(0, list);
                stores
            },
            |mut stores| black_box(stores.snapshot().state_hash()),
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

fn survivor_root_recycling(c: &mut Criterion) {
    c.bench_function("survivor_root_recycling/fixed_shape_overwrites", |b| {
        b.iter_batched(
            Universe::new,
            |mut stores| {
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
                assert!(
                    stores.testing_survivor_recycled_buffer_uses() >= TRANSIENT_BOX_OVERWRITES - 2
                );
                black_box(stores);
            },
            BatchSize::SmallInput,
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
    let mut group = c.benchmark_group("provenance_source_lexing");
    for (name, input, needs_control_sequences) in source_workloads() {
        let token_count = source_heavy_token_count(&input);
        group.throughput(Throughput::Elements(token_count as u64));
        group.bench_with_input(BenchmarkId::new("traced", name), &input, |b, input| {
            b.iter_batched(
                || {
                    (
                        source_universe(needs_control_sequences),
                        InputStack::new(MemoryInput::new(input.clone())),
                    )
                },
                |(mut stores, mut input)| {
                    black_box(drain_traced_source_timed(&mut stores, &mut input));
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn provenance_diagnostic_rendering(c: &mut Criterion) {
    let input = mixed_utf8_text();
    let mut group = c.benchmark_group("provenance_diagnostic_rendering");

    group.bench_function("cold", |b| {
        b.iter_batched(
            || diagnostic_case(input.clone()),
            |(stores, origin)| {
                black_box(
                    ProvenanceResolver::new(&stores)
                        .render_diagnostic("measured diagnostic", Some(origin)),
                );
            },
            BatchSize::SmallInput,
        );
    });

    let (stores, origin) = diagnostic_case(input);
    group.bench_function("repeated_warm", |b| {
        let resolver = ProvenanceResolver::new(&stores);
        b.iter(|| {
            black_box(resolver.render_diagnostic("measured diagnostic", Some(origin)));
        });
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
    if std::env::var_os("UMBER_PROVENANCE_REPORT").is_some() {
        print_provenance_report();
    }
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
        .map(|index| stores.intern(&format!("page-cell-{index}")).symbol())
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

fn mixed_utf8_text() -> String {
    repeated_lines(MIXED_UTF8_LINE)
}

fn control_sequence_text() -> String {
    repeated_lines(CONTROL_SEQUENCE_LINE)
}

fn repeated_lines(line: &str) -> String {
    let mut input = String::new();
    for _ in 0..SOURCE_HEAVY_LINES {
        input.push_str(line);
        input.push('\n');
    }
    input
}

fn source_workloads() -> Vec<(&'static str, String, bool)> {
    vec![
        ("ascii", source_heavy_text(), false),
        ("mixed_utf8", mixed_utf8_text(), false),
        (
            "single_long_line",
            format!("{}\n", "a".repeat(LONG_LINE_SCALARS)),
            false,
        ),
        ("control_sequences", control_sequence_text(), true),
    ]
}

fn drain_traced_source(
    stores: &mut Universe,
    input: &mut InputStack<MemoryInput>,
) -> (usize, usize, ProvenanceStats, ProvenanceStats) {
    let baseline = stores.provenance_stats();
    let mut count = 0;
    let mut direct = 0;
    while let Some(token) = input
        .next_traced_token(stores)
        .expect("source lexing should succeed")
    {
        count += 1;
        direct += usize::from(token.origin().is_direct_source());
        black_box(token);
    }
    let final_stats = stores.provenance_stats();
    (
        count,
        direct,
        final_stats.saturating_sub(baseline),
        final_stats.saturating_sub(baseline),
    )
}

fn drain_traced_source_timed(stores: &mut Universe, input: &mut InputStack<MemoryInput>) -> usize {
    let mut count = 0;
    while let Some(token) = input
        .next_traced_token(stores)
        .expect("source lexing should succeed")
    {
        black_box(token);
        count += 1;
    }
    count
}

fn diagnostic_case(input: String) -> (Universe, tex_state::token::OriginId) {
    let mut stores = source_universe(false);
    let mut stack = InputStack::new(MemoryInput::new(input));
    let token = stack
        .next_traced_token(&mut stores)
        .expect("diagnostic source should lex")
        .expect("diagnostic source should contain a token");
    (stores, token.origin())
}

fn print_provenance_report() {
    for (name, text, needs_control_sequences) in source_workloads() {
        let mut stores = source_universe(needs_control_sequences);
        let mut input = InputStack::new(MemoryInput::new(text));
        let (tokens, direct, live, peak) = drain_traced_source(&mut stores, &mut input);
        eprintln!(
            "provenance-report {name}: tokens={tokens} direct={direct} records={} spans={} entries={} regions={} backings={} live_bytes={} retained_bytes={} peak_live_bytes={} peak_retained_bytes={} cache_bytes=0",
            live.origin_records(),
            live.origin_list_spans(),
            live.origin_list_entries(),
            live.source_regions(),
            live.generated_source_backings(),
            live.estimated_bytes(),
            live.retained_bytes(),
            live.estimated_bytes(),
            peak.retained_bytes(),
        );
    }

    let (mut stores, mut input, baseline) = generated_run_case();
    let snapshot = stores.snapshot();
    let _ = drain_expansion(&mut stores, &mut input);
    let peak = stores.provenance_stats().saturating_sub(baseline);
    stores.rollback(&snapshot);
    let post = stores.provenance_stats().saturating_sub(baseline);
    eprintln!(
        "provenance-report rollback_reuse: peak_live_bytes={} peak_retained_bytes={} post_live_bytes={} post_retained_bytes={}",
        peak.estimated_bytes(),
        peak.retained_bytes(),
        post.estimated_bytes(),
        post.retained_bytes(),
    );
}

fn source_heavy_token_count(input: &str) -> usize {
    let stores = source_universe(input.contains('\\'));
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

fn source_universe(needs_control_sequences: bool) -> Universe {
    let mut stores = Universe::new();
    if needs_control_sequences {
        for name in [
            "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta",
        ] {
            stores.intern(name);
        }
    }
    stores
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

    let call_tokens = vec![Token::Cs(macro_cs.symbol()); MACRO_CALLS];
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
        tokens.push(Token::Cs(number.symbol()));
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
        tokens.push(Token::Cs(roman.symbol()));
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
    allocation_node_append,
    allocation_graph_transfer,
    deep_journal_box_locality,
    allocation_traced_freeze,
    page_contribution_queue,
    meaning_lookup,
    barrier_write,
    snapshot_take,
    checkpoint_state_hash,
    transient_box_overwrite_checkpoint,
    survivor_root_recycling,
    group_cycle,
    rollback_scaling,
    group_global_compaction,
    synthetic_page_journal_volume,
    provenance_source_lexing,
    provenance_expansion,
    provenance_memory_invariants,
    provenance_diagnostic_rendering
);
criterion_main!(benches);
