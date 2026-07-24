//! Private immutable engine and character profile state.

/// Canonical command families installed before a job begins.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[allow(dead_code)] // all canonical profiles are established before dispatch exists
pub(crate) enum CommandDialect {
    #[default]
    Tex82,
    Etex26,
    Pdftex14027,
}

/// Character domain selected for the complete lifetime of a job.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[allow(dead_code)] // the extension mode is established before tokenization exists
pub(crate) enum CharacterMode {
    #[default]
    EightBitExact,
    UnicodeExtended,
}

/// Immutable semantic profile retained in persistent command state.
///
/// The profile is private until profile installation is implemented. Keeping
/// it in [`crate::CommandState`] establishes that it is semantic state, rather
/// than request policy or a runtime acceleration setting.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct CommandProfile {
    pub(crate) dialect: CommandDialect,
    pub(crate) characters: CharacterMode,
}
