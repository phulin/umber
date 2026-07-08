use std::path::Path;

use tex_state::ExpansionState;

fn generic_expansion_code<S: ExpansionState>(stores: &mut S) {
    let mut input = stores.input_open_context();
    let _ = input.read_input_file(Path::new("main.tex"));
}

fn main() {}
