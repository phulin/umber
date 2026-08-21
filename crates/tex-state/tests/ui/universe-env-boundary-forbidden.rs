use tex_state::interner::InternerBudget;

fn main() {
    let budget = InternerBudget::new(16, 16, 256).unwrap();
    tex_state::with_universe(budget, |universe| {
        let _ = universe.live_state();
        let _ = universe.admitted();
    })
    .unwrap();
}
