use tex_state::Universe;

fn main() {
    let mut universe = Universe::new();
    let transaction = universe.begin_box_build();
    universe.set_count(0, 1);
    drop(transaction);
}
