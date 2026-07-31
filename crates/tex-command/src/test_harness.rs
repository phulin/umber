//! The engine state a crate-internal scanner test runs against.
//!
//! tex.web §75 starts a job in `error_stop_mode`, and §82 enters §83's dialog
//! on that alone. A memory `World` with no terminal lines pushed into it is a
//! terminal at end of file, which §71 answers with
//! `fatal_error("End of file on the terminal!")` -- so a scanner test that
//! raises any recoverable error would end its job on the dialog rather than
//! recover and keep scanning.
//!
//! That is the right behavior for an interactive job, and it is what
//! `umber2-er8c` restored. It is simply not what these tests are about: they
//! exercise §§413-460's scanners and their recoveries, not the terminal. So
//! they run the job the way a `\nonstopmode` document does, which is also
//! what the minifixture corpus does for the same reason.

/// A [`tex_state::Universe`] in `\nonstopmode`, with §75's dialog off.
#[must_use]
pub(crate) fn universe() -> tex_state::Universe {
    let mut universe = tex_state::Universe::new();
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    universe
}

/// [`universe`] with plain TeX's category codes already installed.
#[must_use]
pub(crate) fn universe_with_plain_catcodes() -> tex_state::Universe {
    let mut universe = tex_state::Universe::new_with_plain_catcodes();
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    universe
}
