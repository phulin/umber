fn cross<Prior, Current>(
    prior: tex_state::TokenListId<Prior>,
    current: &mut Option<tex_state::TokenListId<Current>>,
) {
    *current = Some(prior);
}

fn main() {}
