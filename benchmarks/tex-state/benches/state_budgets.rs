use criterion::{BatchSize, Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_state::cell::{BankTag, CellId};
use tex_state::meaning::{Meaning, ResolvedMeaning};
use tex_state::node::Node;
use tex_state::node_arena::PageNodeArena;
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

        let write_mark = universe.journal_cursor().expect("write cursor");
        {
            let mut context = universe.command_context().expect("write context");
            context
                .assign_count(0, 1, AssignmentScope::Global)
                .expect("prime write slice");
        }
        universe
            .restore_state(write_mark)
            .expect("restore priming write");
        let mut value = 0_i32;
        c.bench_function("direct_state/same_cell_global_write", |b| {
            b.iter(|| {
                for _ in 0..WARM_WRITES {
                    value = value.wrapping_add(1);
                    {
                        let mut context = universe.command_context().expect("write context");
                        context
                            .assign_count(0, black_box(value), AssignmentScope::Global)
                            .expect("write admitted count");
                    }
                    universe
                        .restore_state(write_mark)
                        .expect("discard operation-local journal slice");
                }
                black_box(universe.count(0).expect("read restored count"));
            });
        });
    })
    .expect("state benchmark universe");
}

fn page_contribution_queue(c: &mut Criterion) {
    with_universe(engine_budget(), |universe| {
        let mut context = universe.command_context().expect("command context");
        for index in 0..PAGE_QUEUE_LEN {
            context.append_page_contribution(Node::Penalty(index as i32));
        }
        while context.pop_page_contribution_front().is_some() {}

        c.bench_function("page_contribution_queue/warmed_roundtrip", |b| {
            b.iter(|| {
                for index in 0..PAGE_QUEUE_LEN {
                    context.append_page_contribution(Node::Penalty(index as i32));
                }
                while let Some(node) = context.pop_page_contribution_front() {
                    black_box(node);
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
        let root = universe.publish_page_nodes(&[Node::Penalty(17)]);
        universe.assign_page_box_global(0, root);
        let mark = universe.journal_cursor().expect("node journal cursor");
        let mut group = c.benchmark_group("node_graph");
        group.throughput(Throughput::Elements(WARM_WRITES as u64));
        group.bench_function("warmed_transfer_and_alias", |b| {
            b.iter(|| {
                for _ in 0..WARM_WRITES {
                    let alias = universe.copy_box_to_page(0).expect("live box alias");
                    universe.replace_page_box(0, alias);
                    universe.restore_state(mark).expect("restore node write");
                    black_box(alias);
                }
            });
        });
        group.finish();
    })
    .expect("node graph benchmark universe");
}

fn node_graph_copy_control(c: &mut Criterion) {
    let mut source = PageNodeArena::new();
    let root = source.publish(vec![Node::Penalty(17)]).expect("source row");
    let mut destination = PageNodeArena::new();
    let mark = destination.cursor();
    let _ = source
        .promote_into(&[root], &mut destination)
        .expect("warm physical copy");
    destination.truncate(mark).expect("reset warm copy");
    let mut group = c.benchmark_group("node_graph");
    group.throughput(Throughput::Elements(WARM_WRITES as u64));
    group.bench_function("physical_copy_control", |b| {
        b.iter(|| {
            for _ in 0..WARM_WRITES {
                black_box(
                    source
                        .promote_into(&[root], &mut destination)
                        .expect("physical graph copy"),
                );
                destination.truncate(mark).expect("reset physical copy");
            }
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    dependency_recording,
    direct_state_access,
    page_contribution_queue,
    node_graph_transfer,
    node_graph_copy_control,
    coarse_generation_lifecycle,
);
criterion_main!(benches);
