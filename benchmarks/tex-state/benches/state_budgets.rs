use criterion::{BatchSize, Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_state::cell::{BankTag, CellId};
use tex_state::meaning::{Meaning, ResolvedMeaning};
use tex_state::node::Node;
use tex_state::{
    AssignmentScope, DependencyKey, DependencyRuntime, DependencyValue, ReachabilityStore,
    RetainedStateGeneration, World, with_universe,
};
use tex_state_benchmarks::{DIRECT_READS, PAGE_QUEUE_LEN, WARM_WRITES, engine_budget};

fn dependency_recording(c: &mut Criterion) {
    const READS: usize = 4_096;
    let key = DependencyKey::Cell(CellId::new(BankTag::Meaning, 7));
    let value = DependencyValue::Integer(42);
    let mut group = c.benchmark_group("dependency_recording");
    group.throughput(Throughput::Elements(READS as u64));

    group.bench_function("disabled", |b| {
        b.iter(|| {
            let mut runtime = DependencyRuntime::default();
            for _ in 0..READS {
                runtime.record(black_box(key), black_box(value.clone()));
            }
            black_box(runtime);
        });
    });
    group.bench_function("enabled_deduplicated", |b| {
        b.iter(|| {
            let mut runtime = DependencyRuntime::default();
            let token = runtime.begin_region().expect("begin dependency region");
            for _ in 0..READS {
                runtime.record(black_box(key), black_box(value.clone()));
            }
            black_box(
                runtime
                    .finish_region(token)
                    .expect("finish dependency region"),
            );
        });
    });
    group.finish();
}

fn direct_state_access(c: &mut Criterion) {
    with_universe(engine_budget(), |universe| {
        let symbol = universe
            .intern("state-budget-direct")
            .expect("intern benchmark symbol")
            .symbol();
        {
            let mut context = universe.command_context().expect("command context");
            context
                .assign_resolved_meaning(
                    symbol,
                    ResolvedMeaning::Static(Meaning::Relax),
                    AssignmentScope::Global,
                )
                .expect("install direct meaning");
            context
                .assign_count(0, 0, AssignmentScope::Global)
                .expect("warm count cell");
        }

        {
            let context = universe.command_context().expect("read context");
            c.bench_function("direct_state/meaning_and_count", |b| {
                b.iter(|| {
                    let mut checksum = 0_u64;
                    for index in 0..DIRECT_READS {
                        checksum ^= match context.meaning(black_box(symbol)) {
                            ResolvedMeaning::Static(Meaning::Relax) => index as u64,
                            _ => unreachable!("fixture meaning remains direct"),
                        };
                        checksum ^= context.count(0).expect("read count") as u64;
                    }
                    black_box(checksum);
                });
            });
        }

        let write_operation = universe
            .begin_state_operation()
            .expect("prime state operation");
        for value in 0..WARM_WRITES {
            let mut context = universe.command_context().expect("write context");
            context
                .assign_count(0, value as i32, AssignmentScope::Global)
                .expect("prime the rollback lane high-water");
        }
        universe
            .restore_state(write_operation)
            .expect("restore priming writes");
        let mut value = 0_i32;
        c.bench_function("direct_state/same_cell_global_write", |b| {
            b.iter(|| {
                let operation = universe
                    .begin_state_operation()
                    .expect("warmed state operation");
                for _ in 0..WARM_WRITES {
                    value = value.wrapping_add(1);
                    {
                        let mut context = universe.command_context().expect("write context");
                        context
                            .assign_count(0, black_box(value), AssignmentScope::Global)
                            .expect("write admitted count");
                    }
                }
                universe
                    .restore_state(operation)
                    .expect("discard the operation-local journal suffix");
                black_box(universe.count(0).expect("read restored count"));
            });
        });
    })
    .expect("state benchmark universe");
}

fn page_contribution_queue(c: &mut Criterion) {
    with_universe(engine_budget(), |universe| {
        {
            let mut transaction = universe.begin_shipout();
            let mut context = transaction.command_context().expect("page context");
            for index in 0..PAGE_QUEUE_LEN {
                context.append_page_contribution(Node::Penalty(index as i32));
            }
            while let Some(carrier) = context.pop_page_contribution_front() {
                context.discard_page_node(carrier);
            }
        }

        c.bench_function("page_contribution_queue/warmed_roundtrip", |b| {
            b.iter(|| {
                let mut transaction = universe.begin_shipout();
                let mut context = transaction.command_context().expect("page context");
                for index in 0..PAGE_QUEUE_LEN {
                    context.append_page_contribution(Node::Penalty(index as i32));
                }
                for index in 0..PAGE_QUEUE_LEN {
                    let carrier = context
                        .pop_page_contribution_front()
                        .expect("queued page contribution remains live");
                    assert_eq!(
                        context.page_carrier_node(&carrier),
                        &Node::Penalty(index as i32)
                    );
                    context.discard_page_node(carrier);
                }
            });
        });
    })
    .expect("page benchmark universe");
}

fn coarse_generation_lifecycle(c: &mut Criterion) {
    let store = ReachabilityStore::new(engine_budget());
    c.bench_function("coarse_generation/create_and_drop_candidate", |b| {
        b.iter_batched(
            || World::memory(),
            |world| {
                black_box(
                    RetainedStateGeneration::new(&store, world).expect("candidate generation"),
                );
            },
            BatchSize::SmallInput,
        );
    });
}

fn node_graph_transfer(c: &mut Criterion) {
    with_universe(engine_budget(), |universe| {
        {
            let mut transaction = universe.begin_shipout();
            let mut context = transaction.command_context().expect("page context");
            let nodes = (0..WARM_WRITES)
                .map(|index| Node::Penalty(index as i32))
                .collect();
            let nodes = context.publish_unique_page_nodes(nodes);
            context.append_unique_page_contributions(nodes);
        }

        let counters_before = universe.page_material_counters();
        {
            let mut transaction = universe.begin_shipout();
            let mut context = transaction.command_context().expect("page context");
            let nodes = (0..WARM_WRITES)
                .map(|index| Node::Penalty(index as i32))
                .collect();
            let nodes = context.publish_unique_page_nodes(nodes);
            context.append_unique_page_contributions(nodes);
            black_box(context.page_contribution_front());
        }
        let counters_after = universe.page_material_counters();
        assert_eq!(
            counters_after
                .new_semantic_nodes
                .saturating_sub(counters_before.new_semantic_nodes),
            WARM_WRITES as u64,
            "the unique PageBuilder root publishes every new semantic node once"
        );
        assert_eq!(
            counters_after
                .source_nodes_copied
                .saturating_sub(counters_before.source_nodes_copied),
            0,
            "same-region move-only PageBuilder appends do not copy node payload"
        );

        let mut group = c.benchmark_group("node_graph");
        group.throughput(Throughput::Elements(WARM_WRITES as u64));
        group.bench_function("warmed_unique_root_owner_transaction", |b| {
            b.iter_batched(
                || {
                    (0..WARM_WRITES)
                        .map(|index| Node::Penalty(index as i32))
                        .collect()
                },
                |nodes| {
                    let mut transaction = universe.begin_shipout();
                    let mut context = transaction.command_context().expect("page context");
                    let nodes = context.publish_unique_page_nodes(nodes);
                    context.append_unique_page_contributions(nodes);
                    black_box(context.page_contribution_front());
                },
                BatchSize::SmallInput,
            );
        });
        group.finish();
    })
    .expect("node graph benchmark universe");
}

criterion_group!(
    benches,
    dependency_recording,
    direct_state_access,
    page_contribution_queue,
    node_graph_transfer,
    coarse_generation_lifecycle,
);
criterion_main!(benches);
