//! Immutable macro-definition storage.
//!
//! Macro meanings keep one compact operand in Env. The operand names a frozen
//! macro definition here, and the definition names separately frozen parameter
//! text and replacement-body token lists. Diagnostic provenance for a
//! definition is stored beside the semantic definition and is not part of
//! [`MacroMeaning`].

use crate::ids::{MacroDefinitionId, OriginListId, TokenListId};
use crate::meaning::MeaningFlags;
use crate::token::OriginId;

/// Public macro meaning aggregate used at the Universe boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroMeaning {
    flags: MeaningFlags,
    parameter_text: TokenListId,
    replacement_text: TokenListId,
}

impl MacroMeaning {
    /// Creates a macro meaning over already-frozen token lists.
    #[must_use]
    pub const fn new(
        flags: MeaningFlags,
        parameter_text: TokenListId,
        replacement_text: TokenListId,
    ) -> Self {
        Self {
            flags,
            parameter_text,
            replacement_text,
        }
    }

    #[must_use]
    pub const fn flags(self) -> MeaningFlags {
        self.flags
    }

    #[must_use]
    pub const fn parameter_text(self) -> TokenListId {
        self.parameter_text
    }

    #[must_use]
    pub const fn replacement_text(self) -> TokenListId {
        self.replacement_text
    }

    #[must_use]
    pub const fn semantic_eq(self, other: Self) -> bool {
        self.flags.bits() == other.flags.bits()
            && self.parameter_text.raw() == other.parameter_text.raw()
            && self.replacement_text.raw() == other.replacement_text.raw()
    }
}

/// Diagnostic provenance captured while scanning a macro definition.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroDefinitionProvenance {
    definition_origin: OriginId,
    parameter_origins: OriginListId,
    replacement_origins: OriginListId,
}

impl MacroDefinitionProvenance {
    /// Creates a definition-provenance side-table record.
    #[must_use]
    pub const fn new(
        definition_origin: OriginId,
        parameter_origins: OriginListId,
        replacement_origins: OriginListId,
    ) -> Self {
        Self {
            definition_origin,
            parameter_origins,
            replacement_origins,
        }
    }

    /// Unknown provenance used when side-table data is absent or stale.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            definition_origin: OriginId::UNKNOWN,
            parameter_origins: OriginListId::EMPTY,
            replacement_origins: OriginListId::EMPTY,
        }
    }

    #[must_use]
    pub const fn definition_origin(self) -> OriginId {
        self.definition_origin
    }

    #[must_use]
    pub const fn parameter_origins(self) -> OriginListId {
        self.parameter_origins
    }

    #[must_use]
    pub const fn replacement_origins(self) -> OriginListId {
        self.replacement_origins
    }
}

/// A rollback watermark for the macro-definition store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MacroStoreMark {
    pub(crate) definitions: u32,
}

/// Immutable macro-definition table.
#[derive(Clone, Debug)]
pub struct MacroStore {
    definitions: Vec<MacroMeaning>,
    provenance: Vec<Option<MacroDefinitionProvenance>>,
}

impl MacroStore {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            definitions: Vec::new(),
            provenance: Vec::new(),
        }
    }

    pub(crate) fn intern_with_provenance(
        &mut self,
        meaning: MacroMeaning,
        provenance: Option<MacroDefinitionProvenance>,
    ) -> MacroDefinitionId {
        let id = MacroDefinitionId::new(u32_len(
            self.definitions.len(),
            "macro definition table exceeds u32 entries",
        ));
        self.definitions.push(meaning);
        self.provenance.push(provenance);
        id
    }

    #[must_use]
    pub(crate) fn get(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.definitions
            .get(id.raw() as usize)
            .copied()
            .expect("macro definition id is not live")
    }

    #[must_use]
    pub(crate) fn provenance(&self, id: MacroDefinitionId) -> Option<MacroDefinitionProvenance> {
        self.provenance.get(id.raw() as usize).copied().flatten()
    }

    #[must_use]
    pub(crate) fn contains(&self, id: MacroDefinitionId) -> bool {
        (id.raw() as usize) < self.definitions.len()
    }

    #[must_use]
    pub(crate) fn watermark(&self) -> MacroStoreMark {
        MacroStoreMark {
            definitions: u32_len(
                self.definitions.len(),
                "macro definition table exceeds u32 entries",
            ),
        }
    }

    pub(crate) fn truncate_to(&mut self, mark: MacroStoreMark) {
        let definitions = mark.definitions as usize;
        assert!(
            definitions <= self.definitions.len(),
            "macro-store mark has too many definitions"
        );
        self.definitions.truncate(definitions);
        self.provenance.truncate(definitions);
    }
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}
