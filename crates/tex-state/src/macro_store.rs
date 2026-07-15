//! Immutable macro-definition storage.
//!
//! Macro meanings keep one compact operand in Env. The operand names a frozen
//! macro definition here, and the definition names separately frozen parameter
//! text and replacement-body token lists. Diagnostic provenance for a
//! definition is stored beside the semantic definition and is not part of
//! [`MacroMeaning`].

use std::sync::Arc;

use crate::identity::{IdentityAllocator, IdentityMark};
use crate::ids::{MacroDefinitionId, OriginListId, TokenListId};
use crate::meaning::MeaningFlags;
use crate::token::{OriginId, Token};

const MACRO_PARAMETER_SLOTS: usize = 9;

/// Allocation-free index of parameter markers in frozen macro parameter text.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MacroParameterPattern {
    tokens: Arc<[Token]>,
    offsets: [u32; MACRO_PARAMETER_SLOTS],
    count: u8,
}

impl MacroParameterPattern {
    pub fn from_tokens(tokens: &[Token]) -> Self {
        let mut offsets = [0; MACRO_PARAMETER_SLOTS];
        let mut count = 0_usize;
        for (index, token) in tokens.iter().enumerate() {
            if matches!(token, Token::Param(_)) {
                assert!(
                    count < MACRO_PARAMETER_SLOTS,
                    "macro has more than nine parameters"
                );
                offsets[count] = u32::try_from(index).expect("token list length exceeds u32");
                count += 1;
            }
        }
        Self {
            tokens: Arc::from(tokens),
            offsets,
            count: count as u8,
        }
    }

    #[must_use]
    pub const fn parameter_count(&self) -> usize {
        self.count as usize
    }

    #[must_use]
    pub fn leading_end(&self, token_count: usize) -> usize {
        if self.count == 0 {
            token_count
        } else {
            self.offsets[0] as usize
        }
    }

    #[must_use]
    pub fn delimiter_bounds(&self, parameter: usize, token_count: usize) -> (usize, usize) {
        assert!(parameter < self.parameter_count());
        let start = self.offsets[parameter] as usize + 1;
        let end = if parameter + 1 < self.parameter_count() {
            self.offsets[parameter + 1] as usize
        } else {
            token_count
        };
        (start, end)
    }

    #[must_use]
    pub fn leading(&self) -> &[Token] {
        &self.tokens[..self.leading_end(self.tokens.len())]
    }

    #[must_use]
    pub fn delimiter(&self, parameter: usize) -> &[Token] {
        let (start, end) = self.delimiter_bounds(parameter, self.tokens.len());
        &self.tokens[start..end]
    }
}

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
    identities: IdentityMark,
}

/// Immutable macro-definition table.
#[derive(Debug)]
pub struct MacroStore {
    definitions: Vec<MacroMeaning>,
    parameter_patterns: Vec<MacroParameterPattern>,
    provenance: Vec<Option<MacroDefinitionProvenance>>,
    identities: IdentityAllocator,
}

impl Clone for MacroStore {
    fn clone(&self) -> Self {
        Self {
            definitions: self.definitions.clone(),
            parameter_patterns: self.parameter_patterns.clone(),
            provenance: self.provenance.clone(),
            identities: self.identities.fork(),
        }
    }
}

impl MacroStore {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            definitions: Vec::new(),
            parameter_patterns: Vec::new(),
            provenance: Vec::new(),
            identities: IdentityAllocator::new(0),
        }
    }

    pub(crate) fn intern_with_provenance(
        &mut self,
        meaning: MacroMeaning,
        parameter_pattern: MacroParameterPattern,
        provenance: Option<MacroDefinitionProvenance>,
    ) -> MacroDefinitionId {
        let id = MacroDefinitionId::from_identity(
            self.identities
                .allocate()
                .expect("macro definition table exceeds u32 entries"),
        );
        self.definitions.push(meaning);
        self.parameter_patterns.push(parameter_pattern);
        self.provenance.push(provenance);
        id
    }

    #[must_use]
    pub(crate) fn get(&self, id: MacroDefinitionId) -> MacroMeaning {
        assert!(self.contains(id), "macro definition id is not live");
        self.definitions
            .get(id.raw() as usize)
            .copied()
            .expect("macro definition id is not live")
    }

    #[must_use]
    pub(crate) fn parameter_pattern(&self, id: MacroDefinitionId) -> MacroParameterPattern {
        assert!(self.contains(id), "macro definition id is not live");
        self.parameter_patterns[id.raw() as usize].clone()
    }

    #[must_use]
    pub(crate) fn provenance(&self, id: MacroDefinitionId) -> Option<MacroDefinitionProvenance> {
        assert!(self.contains(id), "macro definition id is not live");
        self.provenance.get(id.raw() as usize).copied().flatten()
    }

    #[must_use]
    pub(crate) fn contains(&self, id: MacroDefinitionId) -> bool {
        self.identities.contains(id.identity())
    }

    #[must_use]
    pub(crate) fn resolve_stored(&self, id: MacroDefinitionId) -> Option<MacroDefinitionId> {
        if self.contains(id) {
            return Some(id);
        }
        if !id.is_stored() {
            return None;
        }
        self.identities
            .identity_at(id.raw())
            .map(MacroDefinitionId::from_identity)
    }

    #[must_use]
    pub(crate) fn watermark(&self) -> MacroStoreMark {
        MacroStoreMark {
            definitions: u32_len(
                self.definitions.len(),
                "macro definition table exceeds u32 entries",
            ),
            identities: self.identities.watermark(),
        }
    }

    pub(crate) fn truncate_to(&mut self, mark: MacroStoreMark) {
        let definitions = mark.definitions as usize;
        assert!(
            definitions <= self.definitions.len(),
            "macro-store mark has too many definitions"
        );
        self.identities
            .rollback(mark.identities)
            .expect("macro-store mark is not an ancestor");
        self.definitions.truncate(definitions);
        self.parameter_patterns.truncate(definitions);
        self.provenance.truncate(definitions);
    }
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}
