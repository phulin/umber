//! Retired compatibility facade for canonical command primitive installation.
//!
//! Command delivery, expansion, and scanning are owned by [`tex_command`].
//! This crate remains as a workspace member only until `umber2-johp.15`
//! removes the retired crate identity.

#![forbid(unsafe_code)]

pub use tex_command::{
    install_etex_expandable_primitives, install_latex_expandable_primitives,
    install_pdftex_expandable_primitives, install_tex82_expandable_primitives,
    register_etex_expandable_primitives, register_latex_expandable_primitives,
};
