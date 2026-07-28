//! Private static primitive dispatch families.

mod etex;
mod pdf_files;
mod pdf_random;
mod pdf_regex;
mod pdf_strings;
mod pdftex;
mod prefixed;
mod tex;

pub(crate) use prefixed::is_prefixed_command;
