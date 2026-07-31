//! The engine state a crate-internal executor test runs against.
//!
//! tex.web §75 starts a job in `error_stop_mode`, and §82 enters §83's dialog
//! on that alone. A memory `World` with no terminal lines pushed into it is a
//! terminal at end of file, which §71 answers with
//! `fatal_error("End of file on the terminal!")` -- so a test that raises any
//! recoverable error would end its job on the dialog rather than recover and
//! keep executing.
//!
//! That is the right behavior for an interactive job, and it is what
//! `umber2-er8c` restored. It is simply not what these tests are about: they
//! exercise the stomach and its recoveries, not the terminal. So they run the
//! job the way a `\nonstopmode` document does -- which is also what
//! `scripts/build-tex82-document-traces.sh` captures the reference traces
//! under, and what the minifixture corpus selects, for exactly this reason.
//!
//! A test that *is* about the dialog sets the mode itself, right after
//! constructing the `Universe`.

/// A [`tex_state::Universe`] in `\nonstopmode`, with §83's dialog off.
#[must_use]
pub(crate) fn universe() -> tex_state::Universe {
    with_nonstop_mode(tex_state::Universe::new())
}

/// [`universe`] with plain TeX's category codes already installed.
#[must_use]
pub(crate) fn universe_with_plain_catcodes() -> tex_state::Universe {
    with_nonstop_mode(tex_state::Universe::new_with_plain_catcodes())
}

/// [`universe_with_plain_catcodes`] over an explicit memory `World`, for a
/// test that seeds files into it.
#[must_use]
pub(crate) fn memory_universe_with_plain_catcodes() -> tex_state::Universe {
    with_nonstop_mode(
        tex_state::Universe::with_world(tex_state::World::memory()).with_plain_catcodes(),
    )
}

fn with_nonstop_mode(mut universe: tex_state::Universe) -> tex_state::Universe {
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    universe
}
