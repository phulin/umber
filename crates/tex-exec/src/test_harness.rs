//! Shared state constructors for active crate-internal executor tests.

/// A universe in `\nonstopmode`, so recovery tests do not enter TeX's
/// interactive terminal dialog at the first recoverable error.
#[must_use]
pub(crate) fn universe() -> tex_state::Universe {
    with_nonstop_mode(tex_state::Universe::new())
}

/// [`universe`] with plain TeX category codes installed.
#[must_use]
pub(crate) fn universe_with_plain_catcodes() -> tex_state::Universe {
    with_nonstop_mode(tex_state::Universe::new_with_plain_catcodes())
}

fn with_nonstop_mode(mut universe: tex_state::Universe) -> tex_state::Universe {
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    universe
}
