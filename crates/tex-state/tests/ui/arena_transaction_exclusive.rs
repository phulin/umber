use tex_state::env::AssignmentScope;
use tex_state::interner::InternerBudget;

fn main() {
    let budget = InternerBudget::new(16, 16, 256).unwrap();
    tex_state::with_universe(budget, |universe| {
        let context = universe.command_context().unwrap();
        universe
            .assign_count(0, 1, AssignmentScope::Global)
            .unwrap();
        drop(context);
    })
    .unwrap();
}
