use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_command::{RegisteredSourceKind, SourceRegistration};
use tex_exec::{MainControl, MainControlStep};
use tex_exec_benchmarks::prepare_plain_catcodes;
use tex_state::glue::Order;
use tex_state::interner::InternerBudget;
use tex_state::math::{MathField, MathListNode, MathNoad, NoadClass, NoadKind};
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::{Universe, with_universe};

const NODE_COUNT: usize = 1_024;

#[derive(Clone, Copy)]
enum Shape {
    Ordinary,
    DeferredMath,
}

fn shipout(c: &mut Criterion) {
    let mut group = c.benchmark_group("shipout_generation_episode");
    group.throughput(Throughput::Elements(NODE_COUNT as u64));
    group.bench_function("ordinary_hlist", |b| {
        b.iter(|| run_shipout(Shape::Ordinary));
    });
    group.bench_function("deferred_math_lists", |b| {
        b.iter(|| run_shipout(Shape::DeferredMath));
    });
    group.finish();
}

fn run_shipout(shape: Shape) {
    let budget =
        InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024).expect("benchmark interner budget");
    with_universe(budget, |stores| {
        prepare_plain_catcodes(stores);
        match shape {
            Shape::Ordinary => {
                let nodes = (0..NODE_COUNT)
                    .map(|index| Node::Penalty(index as i32))
                    .collect::<Vec<_>>();
                install_box(stores, nodes);
            }
            Shape::DeferredMath => {
                let mut context = stores.command_context().expect("command context");
                let content = context.publish_page_nodes(vec![Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::Empty,
                ))]);
                let list = MathListNode {
                    display: false,
                    content,
                };
                drop(context);
                install_box(stores, vec![Node::MathList(list); NODE_COUNT]);
            }
        }
        let mut control = shipout_input(stores);
        loop {
            match control.step(stores).expect("benchmark shipout succeeds") {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
            }
        }
        black_box(stores.world().artifact_commits().len());
    })
    .expect("shipout benchmark universe");
}

fn install_box<G>(stores: &mut Universe<G>, nodes: Vec<Node>) {
    let mut context = stores.command_context().expect("command context");
    let children = context.publish_page_nodes(nodes);
    let root = Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }));
    let root_list = context.publish_page_nodes(vec![root]);
    context
        .assign_page_box_global(0, root_list)
        .expect("benchmark box promotes");
}

fn shipout_input<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let mut control = MainControl::tex82_initex(stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"\\shipout\\box0\\end".to_vec(),
        ))
        .expect("benchmark source registers");
    control
}

criterion_group!(benches, shipout);
criterion_main!(benches);
