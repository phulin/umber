use tex_state::ExpansionState;

fn scanner_like_helper(stores: &mut impl ExpansionState) {
    let _ = stores.input_open_context();
}

fn main() {}
