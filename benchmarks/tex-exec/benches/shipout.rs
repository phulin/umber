use criterion::{BatchSize, Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_exec::Executor;
use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::glue::Order;
use tex_state::math::{MathField, MathListNode, MathNoad, NoadClass, NoadKind};
use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

const NODE_COUNT: usize = 1_024;

fn shipout(c: &mut Criterion) {
    let mut group = c.benchmark_group("shipout_lowering");
    group.throughput(Throughput::Elements(NODE_COUNT as u64));
    group.bench_function("ordinary_hlist", |b| {
        b.iter_batched(ordinary_shipout, run_shipout, BatchSize::SmallInput);
    });
    group.bench_function("deferred_math_lists", |b| {
        b.iter_batched(deferred_math_shipout, run_shipout, BatchSize::SmallInput);
    });
    group.finish();
}

fn ordinary_shipout() -> (Universe, InputStack) {
    let mut stores = prepared_universe();
    let nodes = (0..NODE_COUNT)
        .map(|index| Node::Penalty(index as i32))
        .collect::<Vec<_>>();
    install_box(&mut stores, &nodes);
    (stores, shipout_input())
}

fn deferred_math_shipout() -> (Universe, InputStack) {
    let mut stores = prepared_universe();
    let content = stores.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::Empty,
    ))]);
    let list = MathListNode {
        display: false,
        content,
    };
    let nodes = vec![Node::MathList(list); NODE_COUNT];
    install_box(&mut stores, &nodes);
    (stores, shipout_input())
}

fn prepared_universe() -> Universe {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_exec::install_unexpandable_primitives(&mut stores);
    stores
}

fn install_box(stores: &mut Universe, nodes: &[Node]) {
    let children = stores.freeze_node_list(nodes);
    let root = Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }));
    let root_list = stores.freeze_node_list(&[root]);
    stores.set_box_reg(0, root_list);
}

fn shipout_input() -> InputStack {
    InputStack::new(MemoryInput::new("\\shipout\\box0\\end"))
}

fn run_shipout((mut stores, mut input): (Universe, InputStack)) {
    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("benchmark shipout succeeds");
    black_box(stats.shipped_artifacts);
    black_box(stores);
}

criterion_group!(benches, shipout);
criterion_main!(benches);
