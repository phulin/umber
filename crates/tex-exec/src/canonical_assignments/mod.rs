//! State-only assignment identity, registration, admissibility, and write helpers.
//!
//! This module owns the canonical portion that can act on an already
//! classified assignment without either legacy token/scanner front.

#[cfg(test)]
mod admissibility;
mod primitives;
pub(crate) mod tracing;
#[cfg(test)]
mod variable_access;

#[cfg(test)]
use tex_state::ids::FontId;
#[cfg(test)]
use tex_state::page::{PageDimension, PageInteger};

#[cfg(test)]
pub(crate) use admissibility::{is_assignment_primitive, math_allows_mode_independent_primitive};
pub use primitives::{
    install_etex_unexpandable_primitives, install_unexpandable_primitives,
    register_etex_unexpandable_primitives, register_unexpandable_primitives,
};
#[cfg(test)]
pub(crate) use variable_access::*;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Variable {
    IntRegister(u16),
    DimenRegister(u16),
    GlueRegister(u16),
    MuGlueRegister(u16),
    ToksRegister(u16),
    IntParam(u16),
    DimenParam(u16),
    GlueParam(u16),
    MuGlueParam(u16),
    TokParam(u16),
    PageDimension(PageDimension),
    PageInteger(PageInteger),
    FontDimen(FontId, u32),
    FontHyphenChar(FontId),
    FontSkewChar(FontId),
}
