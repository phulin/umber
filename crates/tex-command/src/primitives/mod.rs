//! Private static primitive dispatch families.

mod etex;
mod pdf_files;
mod pdf_random;
mod pdf_regex;
mod pdf_strings;
mod pdftex;
mod prefixed;
mod registry;
mod tex;

pub(crate) use prefixed::is_prefixed_command;
pub use prefixed::is_prefixed_command as exceeds_max_non_prefixed_command;
pub use registry::{
    install_tex82_expandable_primitives, register_tex82_expandable_primitives,
};
