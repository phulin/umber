use tex_state::stores::Stores;

fn main() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    stores.rollback(snapshot);
}
