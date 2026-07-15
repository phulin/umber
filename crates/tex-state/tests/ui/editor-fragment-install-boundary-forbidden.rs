use tex_state::{FragmentStore, Universe};

fn main() {
    let mut universe = Universe::new();
    universe.install_editor_fragments(FragmentStore::new());
}
