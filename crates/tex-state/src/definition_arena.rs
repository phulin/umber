//! Immutable, generation-branded macro-definition storage.

use core::marker::PhantomData;
use core::num::NonZeroU32;
use std::rc::Rc;

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
    serial: NonZeroU32,
    words: Rc<[TokenWord]>,
    parameter_len: u32,
    parameters: MacroParameterPattern,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for DefinitionId<G> {
    fn clone(&self) -> Self {
        Self {
            serial: self.serial,
            words: Rc::clone(&self.words),
            parameter_len: self.parameter_len,
            parameters: self.parameters,
            _brand: PhantomData,
        }
    }
}

impl<G> PartialEq for DefinitionId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.serial == other.serial
    }
}

impl<G> Eq for DefinitionId<G> {}

impl<G> core::hash::Hash for DefinitionId<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.serial.hash(state);
    }
}

impl<G> core::fmt::Debug for DefinitionId<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionId(..)")
    }
}

impl<G> DefinitionId<G> {
    pub(crate) const fn format_index(&self) -> u32 {
        self.serial.get() - 1
    }

    pub(crate) fn capture_format(&self) -> crate::format::schema::FormatDefinition {
        crate::format::schema::FormatDefinition {
            parameter_text: self.words[..self.parameter_len as usize]
                .iter()
                .map(|word| word.raw())
                .collect(),
            replacement_text: self.words[self.parameter_len as usize..]
                .iter()
                .map(|word| word.raw())
                .collect(),
        }
    }

    #[cfg(test)]
    pub(crate) fn semantic_owner_count(&self) -> usize {
        Rc::strong_count(&self.words)
    }
}

/// Failure to stage a complete definition row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionAllocationError {
    CapacityOverflow,
    AllocationFailed,
}

/// Publisher for immutable shared definitions.
///
/// Published payloads leave the publisher in their generation-branded owner;
/// this value retains only the monotonic serial used by cold format capture.
pub(crate) struct DefinitionArena<G> {
    next_serial: u32,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> DefinitionArena<G> {
    pub(super) fn new(_token: ArenaToken<G, DefinitionNamespace>) -> Self {
        Self {
            next_serial: 0,
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
        self.allocate_from_iter(
            parameter_text.iter().copied(),
            replacement_text.iter().copied(),
        )
    }

    /// Atomically publishes one definition from exact-size token streams.
    ///
    /// Promotion can thereby transform traced attempt words directly into
    /// generation storage without allocating intermediate token vectors.
    /// Both destination vectors reserve before the first iterator advances or
    /// either logical length changes.
    pub(crate) fn allocate_from_iter<Parameters, Replacement>(
        &mut self,
        parameter_text: Parameters,
        replacement_text: Replacement,
    ) -> Result<DefinitionId<G>, DefinitionAllocationError>
    where
        Parameters: Clone + ExactSizeIterator<Item = TokenWord>,
        Replacement: ExactSizeIterator<Item = TokenWord>,
    {
        let parameter_len = parameter_text.len();
        let replacement_len = replacement_text.len();
        let serial = self
            .next_serial
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let final_word_len = parameter_len
            .checked_add(replacement_len)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        u32::try_from(final_word_len).map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        let parameters = MacroParameterPattern::from_word_iter(parameter_text.clone());
        let words = parameter_text.chain(replacement_text).collect::<Rc<[_]>>();
        self.next_serial = serial.get();
        Ok(DefinitionId {
            serial,
            words,
            parameter_len: parameter_len as u32,
            parameters,
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
        (self.next_serial as usize)
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let _ = words;
        Ok(())
    }

    /// Moves an owner into a direct immutable view.
    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: DefinitionId<G>) -> DefinitionView<G> {
        DefinitionView { id }
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.next_serial as usize
    }

    #[must_use]
    pub(crate) fn tex_memory_words(&self, id: DefinitionId<G>) -> usize {
        let definition = self.get(id);
        definition
            .parameter_text()
            .len()
            .saturating_add(definition.replacement_text().len())
            .saturating_add(2)
            .saturating_add(usize::from(
                !definition.parameter_text().is_empty()
                    || !definition.replacement_text().is_empty(),
            ))
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.next_serial == 0
    }
}

/// Owning view of one complete immutable definition.
pub struct DefinitionView<G> {
    id: DefinitionId<G>,
}

impl<G> DefinitionView<G> {
    #[must_use]
    pub const fn parameter_pattern(&self) -> MacroParameterPattern {
        self.id.parameters
    }

    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        &self.id.words[..self.id.parameter_len as usize]
    }

    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        &self.id.words[self.id.parameter_len as usize..]
    }
}
