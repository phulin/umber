//! Storage-independent TeX code-table values.
//!
//! Live code tables are the page/index dense banks in [`crate::env::DenseState`].
//! This module retains only the semantic value vocabulary used at command and
//! cold detachment boundaries.

pub use crate::env::CodeTableKind;

pub type LcCode = u32;
pub type UcCode = u32;
pub type SfCode = u16;
pub type MathCode = u32;
pub type DelCode = i32;

/// Per-family assignment generations maintained by higher-level dependency
/// projection, not by an alternate code-table store.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CodeTableGenerations {
    pub catcode: u32,
    pub lccode: u32,
    pub uccode: u32,
    pub sfcode: u32,
    pub mathcode: u32,
    pub delcode: u32,
}
