//! Immutable, generation-branded macro-definition storage.

use core::marker::PhantomData;
use core::num::NonZeroU32;

use crate::generation::ArenaToken;
use crate::macro_definition::MacroParameterPattern;
use crate::token::TokenWord;

#[cfg(test)]
#[path = "definition_arena/tests.rs"]
mod tests;

pub(super) enum DefinitionNamespace {}

/// Dense coordinate of one immutable macro definition.
///
/// There is deliberately no raw constructor or integer projection. Only a
/// successful `DefinitionArena::allocate` can publish an id.
pub struct DefinitionId<G> {
    row: NonZeroU32,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for DefinitionId<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for DefinitionId<G> {}

impl<G> PartialEq for DefinitionId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row
    }
}

impl<G> Eq for DefinitionId<G> {}

impl<G> core::hash::Hash for DefinitionId<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.row.hash(state);
    }
}

impl<G> core::fmt::Debug for DefinitionId<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionId(..)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TokenSpan {
    start: u32,
    len: u32,
}

impl TokenSpan {
    fn checked(start: usize, len: usize) -> Option<Self> {
        let start = u32::try_from(start).ok()?;
        let len = u32::try_from(len).ok()?;
        start.checked_add(len)?;
        Some(Self { start, len })
    }

    fn resolve(self, words: &[TokenWord]) -> &[TokenWord] {
        let start = self.start as usize;
        &words[start..start + self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DefinitionRecord {
    parameter_text: TokenSpan,
    replacement_text: TokenSpan,
    parameters: MacroParameterPattern,
}

/// Failure to stage a complete definition row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionAllocationError {
    CapacityOverflow,
    AllocationFailed,
}

/// Append-only immutable definitions and the words which constitute them.
pub(crate) struct DefinitionArena<G> {
    rows: Vec<DefinitionRecord>,
    words: Vec<TokenWord>,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> DefinitionArena<G> {
    pub(super) fn new(_token: ArenaToken<G, DefinitionNamespace>) -> Self {
        Self {
            rows: Vec::new(),
            words: Vec::new(),
            _brand: PhantomData,
        }
    }

    /// Atomically publishes one complete definition.
    ///
    /// Both backing vectors reserve before either length changes. The row is
    /// appended only after both token spans have been copied into arena-owned
    /// storage, so no partially initialized definition can be resolved.
    pub(crate) fn allocate(
        &mut self,
        parameter_text: &[TokenWord],
        replacement_text: &[TokenWord],
    ) -> Result<DefinitionId<G>, DefinitionAllocationError> {
        let row_number = self
            .rows
            .len()
            .checked_add(1)
            .and_then(|row| u32::try_from(row).ok())
            .and_then(NonZeroU32::new)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let replacement_start = self
            .words
            .len()
            .checked_add(parameter_text.len())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let final_word_len = replacement_start
            .checked_add(replacement_text.len())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        u32::try_from(final_word_len).map_err(|_| DefinitionAllocationError::CapacityOverflow)?;

        let parameter_span = TokenSpan::checked(self.words.len(), parameter_text.len())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let replacement_span = TokenSpan::checked(replacement_start, replacement_text.len())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let parameters = MacroParameterPattern::from_words(parameter_text);
        let record = DefinitionRecord {
            parameter_text: parameter_span,
            replacement_text: replacement_span,
            parameters,
        };

        let appended_word_len = parameter_text
            .len()
            .checked_add(replacement_text.len())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.words
            .try_reserve(appended_word_len)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        self.rows
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;

        self.words.extend_from_slice(parameter_text);
        self.words.extend_from_slice(replacement_text);
        self.rows.push(record);
        Ok(DefinitionId {
            row: row_number,
            _brand: PhantomData,
        })
    }

    /// Reserves a complete promotion batch without changing any logical
    /// arena length. Callers validate the batch's final row and word extents
    /// before invoking this method.
    pub(crate) fn reserve_batch(
        &mut self,
        rows: usize,
        words: usize,
    ) -> Result<(), DefinitionAllocationError> {
        self.rows
            .len()
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.words
            .len()
            .checked_add(words)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.rows
            .try_reserve(rows)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        self.words
            .try_reserve(words)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)
    }

    /// Resolves an arena-issued id with one direct row access.
    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: DefinitionId<G>) -> DefinitionView<'_, G> {
        let record = &self.rows[id.row.get() as usize - 1];
        DefinitionView {
            record,
            words: &self.words,
            _brand: PhantomData,
        }
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Borrowed view of one complete definition row.
pub struct DefinitionView<'arena, G> {
    record: &'arena DefinitionRecord,
    words: &'arena [TokenWord],
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> DefinitionView<'_, G> {
    #[must_use]
    pub const fn parameter_pattern(&self) -> MacroParameterPattern {
        self.record.parameters
    }

    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        self.record.parameter_text.resolve(self.words)
    }

    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        self.record.replacement_text.resolve(self.words)
    }
}
