//! Private static primitive dispatch families.

mod catalogue;
pub(crate) mod metadata;
mod prefixed;
mod registry;

pub use catalogue::{
    CatalogueValidationError, DocumentationFamily, ExpansionClass, GlueParameterDefault,
    InstallationPolicy, JobClockField, ParameterBankClass, ParameterCell, ParameterDefault,
    PrefixAdmissibility, PrimitiveCatalogue, PrimitiveDescriptor, PrimitiveOperand,
    PrimitiveOperandDomain, PrimitiveProfile, PrimitiveProfiles, PrimitiveSpelling, SpellingKind,
    WebIdentity,
};
pub(crate) use prefixed::is_prefixed_command;
pub use prefixed::is_prefixed_command as exceeds_max_non_prefixed_command;
pub use registry::{
    install_etex_expandable_primitives, install_etex_unexpandable_primitives,
    install_latex_expandable_primitives, install_pdftex_expandable_primitives,
    install_pdftex_unexpandable_primitives, install_tex82_expandable_primitives,
    install_tex82_unexpandable_primitives, register_etex_expandable_primitives,
    register_etex_unexpandable_primitives, register_latex_expandable_primitives,
    register_pdftex_expandable_primitives, register_pdftex_unexpandable_primitives,
    register_tex82_expandable_primitives, register_tex82_unexpandable_primitives,
};
