use tex_state::interner::InternerBudget;
use tex_state::node::Node;
use tex_state::node_arena::PageListId;

fn main() {
    let budget = InternerBudget::new(16, 16, 256).unwrap();
    tex_state::with_universe(budget, |universe| {
        let mut transaction = universe.begin_shipout();
        let scratch = transaction.begin_shipout_scratch_list();
        transaction.push_shipout_scratch_node(scratch, Node::Penalty(1));
        let _checkpointable: PageListId = scratch;
    })
    .unwrap();
}
