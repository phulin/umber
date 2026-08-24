use tex_state::ReachabilityStore;
use tex_state::interner::InternerBudget;

fn main() {
    let store = ReachabilityStore::new(InternerBudget::new(8, 8, 128).unwrap());
    let _ = store.epoch();
    let _ = &store.storage;
    let _: tex_state::reachability_store::ReachabilityGenerationKey;
}
