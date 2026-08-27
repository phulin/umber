//! Immutable, generation-branded macro-definition storage.

use core::marker::PhantomData;
use core::num::NonZeroU32;
use thin_dst::ThinRc;

use crate::generation::ArenaToken;
use crate::macro_definition::MacroParameterPattern;
use crate::memory_accounting::MemoryAccounting;
use crate::token::TokenWord;

#[cfg(test)]
#[path = "definition_arena/tests.rs"]
mod tests;

pub(super) enum DefinitionNamespace {}

struct DefinitionWords<Parameters, Replacement> {
    parameters: Parameters,
    replacement: Replacement,
}

impl<Parameters, Replacement> Iterator for DefinitionWords<Parameters, Replacement>
where
    Parameters: ExactSizeIterator<Item = TokenWord>,
    Replacement: ExactSizeIterator<Item = TokenWord>,
{
    type Item = TokenWord;

    fn next(&mut self) -> Option<Self::Item> {
        self.parameters.next().or_else(|| self.replacement.next())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<Parameters, Replacement> ExactSizeIterator for DefinitionWords<Parameters, Replacement>
where
    Parameters: ExactSizeIterator<Item = TokenWord>,
    Replacement: ExactSizeIterator<Item = TokenWord>,
{
    fn len(&self) -> usize {
        self.parameters
            .len()
            .checked_add(self.replacement.len())
            .expect("validated definition word length")
    }
}

struct DefinitionHeader {
    serial: NonZeroU32,
    parameter_len: u32,
    parameters: MacroParameterPattern,
    accounting: MemoryAccounting,
    memory_words: usize,
}

impl Drop for DefinitionHeader {
    fn drop(&mut self) {
        self.accounting.release_shared_dynamic(self.memory_words);
    }
}

/// Dense coordinate of one immutable macro definition.
///
/// There is deliberately no raw constructor or integer projection. Only a
/// successful `DefinitionArena::allocate` can publish an id.
pub struct DefinitionId<G> {
    allocation: ThinRc<DefinitionHeader, TokenWord>,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for DefinitionId<G> {
    fn clone(&self) -> Self {
        Self {
            allocation: self.allocation.clone(),
            _brand: PhantomData,
        }
    }
}

impl<G> PartialEq for DefinitionId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.allocation.head.serial == other.allocation.head.serial
    }
}

impl<G> Eq for DefinitionId<G> {}

impl<G> core::hash::Hash for DefinitionId<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.allocation.head.serial.hash(state);
    }
}

impl<G> core::fmt::Debug for DefinitionId<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionId(..)")
    }
}

impl<G> DefinitionId<G> {
    /// Borrows the packed parameter text through this definition's existing
    /// owner.
    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        &self.allocation.slice[..self.allocation.head.parameter_len as usize]
    }

    /// Borrows the packed replacement text through this definition's existing
    /// owner.
    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        &self.allocation.slice[self.allocation.head.parameter_len as usize..]
    }

    /// Borrows one packed replacement word through this already-owned
    /// definition handle.
    ///
    /// Stored-token delivery keeps the handle in its input span, so this
    /// access neither clones the non-atomic owner nor reconstructs a
    /// [`DefinitionView`].
    #[must_use]
    pub fn replacement_word(&self, index: usize) -> Option<TokenWord> {
        self.allocation.slice[self.allocation.head.parameter_len as usize..]
            .get(index)
            .copied()
    }

    pub(crate) fn format_index(&self) -> u32 {
        self.allocation.head.serial.get() - 1
    }

    pub(crate) fn capture_format(&self) -> crate::format::schema::FormatDefinition {
        crate::format::schema::FormatDefinition {
            parameter_text: self.allocation.slice[..self.allocation.head.parameter_len as usize]
                .iter()
                .map(|word| word.raw())
                .collect(),
            replacement_text: self.allocation.slice[self.allocation.head.parameter_len as usize..]
                .iter()
                .map(|word| word.raw())
                .collect(),
        }
    }

    #[cfg(test)]
    pub(crate) fn semantic_owner_count(&self) -> usize {
        let owner: std::rc::Rc<thin_dst::ThinData<DefinitionHeader, TokenWord>> =
            self.allocation.clone().into();
        std::rc::Rc::strong_count(&owner) - 1
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
    accounting: MemoryAccounting,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> DefinitionArena<G> {
    pub(crate) fn fork(&self, accounting: MemoryAccounting) -> Self {
        Self {
            next_serial: self.next_serial,
            accounting,
            _brand: PhantomData,
        }
    }

    pub(crate) const fn cursor(&self) -> u32 {
        self.next_serial
    }

    pub(crate) fn restore_cursor(&mut self, cursor: u32) {
        assert!(
            cursor <= self.next_serial,
            "definition cursor is beyond the publisher"
        );
        self.next_serial = cursor;
    }

    pub(super) fn new(
        _token: ArenaToken<G, DefinitionNamespace>,
        accounting: MemoryAccounting,
    ) -> Self {
        Self {
            next_serial: 0,
            accounting,
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
        let memory_words = definition_memory_words(final_word_len);
        self.accounting.allocate_shared_dynamic(memory_words);
        let allocation = ThinRc::new(
            DefinitionHeader {
                serial,
                parameter_len: parameter_len as u32,
                parameters,
                accounting: self.accounting.clone(),
                memory_words,
            },
            DefinitionWords {
                parameters: parameter_text,
                replacement: replacement_text,
            },
        );
        self.next_serial = serial.get();
        Ok(DefinitionId {
            allocation,
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

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.next_serial == 0
    }
}

fn definition_memory_words(word_len: usize) -> usize {
    word_len
        .checked_add(2)
        .and_then(|words| words.checked_add(usize::from(word_len != 0)))
        .expect("validated definition length has a canonical word count")
}

/// Owning view of one complete immutable definition.
pub struct DefinitionView<G> {
    id: DefinitionId<G>,
}

impl<G> DefinitionView<G> {
    #[must_use]
    pub fn parameter_pattern(&self) -> MacroParameterPattern {
        self.id.allocation.head.parameters
    }

    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        self.id.parameter_text()
    }

    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        self.id.replacement_text()
    }
}
