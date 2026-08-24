use tex_state::interner::InternerBudget;
use tex_state::{ReachabilityStore, RetainedStateGeneration, World};

fn escaped_generation() -> RetainedStateGeneration<'static> {
    let store = ReachabilityStore::new(InternerBudget::new(8, 8, 128).unwrap());
    RetainedStateGeneration::new(&store, World::memory()).unwrap()
}

fn main() {
    let _ = escaped_generation();
}
