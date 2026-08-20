//! Copy-only macro identities and borrowed runtime-registry views.

use crate::ids::{MacroDefinitionId, OriginListId, TokenListId};
use crate::meaning::MeaningFlags;
use crate::provenance::OriginRef;
use crate::token::{OriginId, RootedTracedTokenWord, Token, TracedTokenWord};

const MACRO_PARAMETER_SLOTS: usize = 9;

/// Allocation-free index of parameter markers in macro parameter text.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroParameterPattern {
    offsets: [u32; MACRO_PARAMETER_SLOTS],
    widths: [u8; MACRO_PARAMETER_SLOTS],
    count: u8,
}

impl MacroParameterPattern {
    pub fn from_tokens(tokens: &[Token]) -> Self {
        Self::from_token_iter(tokens.iter().copied())
    }

    pub(crate) fn from_traced_words(words: &[TracedTokenWord]) -> Self {
        Self::from_token_iter(words.iter().map(|word| word.semantic_token()))
    }

    fn from_token_iter(tokens: impl Iterator<Item = Token>) -> Self {
        let mut offsets = [0; MACRO_PARAMETER_SLOTS];
        let mut widths = [0; MACRO_PARAMETER_SLOTS];
        let mut count = 0_usize;
        let mut previous = None;
        for (index, token) in tokens.enumerate() {
            if matches!(token, Token::Param(_)) {
                assert!(
                    count < MACRO_PARAMETER_SLOTS,
                    "macro has more than nine parameters"
                );
                let has_spelled_marker = matches!(
                    previous,
                    Some(Token::Char {
                        cat: crate::token::Catcode::Parameter,
                        ..
                    })
                );
                offsets[count] = u32::try_from(index - usize::from(has_spelled_marker))
                    .expect("token list length exceeds u32");
                widths[count] = if has_spelled_marker { 2 } else { 1 };
                count += 1;
            }
            previous = Some(token);
        }
        Self {
            offsets,
            widths,
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
        let start = self.offsets[parameter] as usize + usize::from(self.widths[parameter]);
        let end = if parameter + 1 < self.parameter_count() {
            self.offsets[parameter + 1] as usize
        } else {
            token_count
        };
        (start, end)
    }

    #[must_use]
    pub const fn marker_index(&self, parameter: usize) -> Option<usize> {
        assert!(parameter < self.parameter_count());
        if self.widths[parameter] == 2 {
            Some(self.offsets[parameter] as usize)
        } else {
            None
        }
    }
}

/// Public semantic macro-body aggregate used at the Universe boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroMeaning {
    flags: MeaningFlags,
    parameter_text: TokenListId,
    replacement_text: TokenListId,
}

impl MacroMeaning {
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

/// Diagnostic provenance captured while scanning one definition occurrence.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MacroDefinitionProvenance {
    definition_origin: OriginRef,
    parameter_origins: crate::provenance::OriginListRef,
    replacement_origins: crate::provenance::OriginListRef,
}

impl MacroDefinitionProvenance {
    #[must_use]
    pub const fn new(
        definition_origin: OriginRef,
        parameter_origins: crate::provenance::OriginListRef,
        replacement_origins: crate::provenance::OriginListRef,
    ) -> Self {
        Self {
            definition_origin,
            parameter_origins,
            replacement_origins,
        }
    }

    #[must_use]
    pub fn unknown() -> Self {
        Self {
            definition_origin: OriginRef::unknown(),
            parameter_origins: crate::provenance::OriginListRef::empty(),
            replacement_origins: crate::provenance::OriginListRef::empty(),
        }
    }

    #[must_use]
    pub fn definition_origin(&self) -> OriginId {
        self.definition_origin.id()
    }

    #[must_use]
    pub const fn definition_ref(&self) -> &OriginRef {
        &self.definition_origin
    }

    #[must_use]
    pub fn parameter_origins(&self) -> OriginListId {
        self.parameter_origins.id()
    }

    #[must_use]
    pub const fn parameter_ref(&self) -> crate::provenance::OriginListRef {
        self.parameter_origins
    }

    #[must_use]
    pub fn replacement_origins(&self) -> OriginListId {
        self.replacement_origins.id()
    }

    #[must_use]
    pub const fn replacement_ref(&self) -> crate::provenance::OriginListRef {
        self.replacement_origins
    }
}

/// Copy-only semantic identity for a macro definition in the runtime registry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroDefinitionRef {
    id: MacroDefinitionId,
}

impl MacroDefinitionRef {
    pub(crate) const fn new(id: MacroDefinitionId) -> Self {
        Self { id }
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_new(raw: u32) -> Self {
        Self::new(MacroDefinitionId::new(raw))
    }

    #[must_use]
    pub const fn id(&self) -> MacroDefinitionId {
        self.id
    }

    #[must_use]
    pub const fn raw(&self) -> u32 {
        self.id.raw()
    }
}

/// Borrowed macro body and provenance admitted by the runtime value registry.
pub struct MacroDefinitionView<'a> {
    pub(crate) inner: crate::hot_core::arena::store::RuntimeMacroView<'a>,
}

impl core::fmt::Debug for MacroDefinitionView<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.meaning().fmt(formatter)
    }
}

impl PartialEq for MacroDefinitionView<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.meaning().semantic_eq(other.meaning())
    }
}

impl PartialEq<MacroMeaning> for MacroDefinitionView<'_> {
    fn eq(&self, other: &MacroMeaning) -> bool {
        self.meaning().semantic_eq(*other)
    }
}

impl<'a> MacroDefinitionView<'a> {
    pub(crate) const fn new(inner: crate::hot_core::arena::store::RuntimeMacroView<'a>) -> Self {
        Self { inner }
    }

    #[must_use]
    pub const fn id(&self) -> MacroDefinitionId {
        self.inner.coordinate().id()
    }

    #[must_use]
    pub const fn meaning(&self) -> MacroMeaning {
        self.inner.meaning()
    }

    #[must_use]
    pub const fn flags(&self) -> MeaningFlags {
        self.inner.meaning().flags()
    }

    #[must_use]
    pub const fn parameter_text(&self) -> TokenListId {
        self.inner.meaning().parameter_text()
    }

    #[must_use]
    pub const fn replacement_text(&self) -> TokenListId {
        self.inner.meaning().replacement_text()
    }

    #[must_use]
    pub const fn parameter_pattern(&self) -> MacroParameterPattern {
        self.inner.parameter_pattern()
    }

    #[must_use]
    pub fn parameter_tokens(&self) -> &[Token] {
        self.inner.parameter_text().tokens()
    }

    #[must_use]
    pub fn replacement_tokens(&self) -> &[Token] {
        self.inner.replacement_text().tokens()
    }

    #[must_use]
    pub fn parameter_token(&self, index: usize) -> Option<Token> {
        self.parameter_tokens().get(index).copied()
    }

    #[must_use]
    pub fn parameter_traced_word(&self, index: usize) -> Option<TracedTokenWord> {
        self.inner.parameter_traced_word(index)
    }

    #[must_use]
    pub fn parameter_len(&self) -> usize {
        self.parameter_tokens().len()
    }

    #[must_use]
    pub fn replacement_len(&self) -> usize {
        self.replacement_tokens().len()
    }

    #[must_use]
    pub fn replacement_traced_word(&self, index: usize) -> Option<TracedTokenWord> {
        self.inner.replacement_traced_word(index)
    }

    #[must_use]
    pub fn replacement_word(
        &self,
        index: usize,
        resolve: impl FnMut(OriginId) -> Option<OriginRef>,
    ) -> Option<RootedTracedTokenWord> {
        self.inner.replacement_text().rooted_word(index, resolve)
    }

    #[must_use]
    pub const fn definition_origin(&self) -> OriginId {
        self.inner.definition_origin()
    }

    pub(crate) fn has_provenance(&self) -> bool {
        self.inner.has_provenance()
    }

    #[must_use]
    pub const fn observation_operand(&self) -> i64 {
        self.inner.observation_operand()
    }

    #[must_use]
    pub const fn allocation_serial(&self) -> u64 {
        self.inner.allocation_serial()
    }

    #[must_use]
    pub fn semantic_eq(&self, other: Self) -> bool {
        self.meaning().semantic_eq(other.meaning())
    }
}
