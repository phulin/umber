//! Definition rows, opaque references, and the resident macro body.

use core::marker::PhantomData;
use core::num::{NonZeroU32, NonZeroU64};
use std::rc::Rc;

use crate::macro_definition::{MacroParameterPattern, MacroParameterPatternBuilder};
use crate::state_hash::StateHasher;
use crate::token::TokenWord;

const DEFINITION_IDENTITY_V2_DOMAIN: u64 = 0x6465_6669_6e69_7432;
const PARAMETER_START: u8 = 0;
const PARAMETER_END: u8 = 1;
const REPLACEMENT_START: u8 = 2;
const REPLACEMENT_END: u8 = 3;

/// Compact opaque reference to one immutable macro definition.
///
/// The packed region and row are private to this store. Callers may copy and
/// compare the reference, but cannot decode or manufacture storage
/// coordinates.
pub struct DefinitionRef<G> {
    packed: NonZeroU64,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for DefinitionRef<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for DefinitionRef<G> {}

impl<G> PartialEq for DefinitionRef<G> {
    fn eq(&self, other: &Self) -> bool {
        self.packed == other.packed
    }
}

impl<G> Eq for DefinitionRef<G> {}

impl<G> core::hash::Hash for DefinitionRef<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.packed.hash(state);
    }
}

impl<G> core::fmt::Debug for DefinitionRef<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionRef(..)")
    }
}

impl<G> DefinitionRef<G> {
    pub(super) fn new(region: u32, row: usize) -> Self {
        let row = u32::try_from(row + 1).expect("definition rows fit u32");
        let row = NonZeroU32::new(row).expect("definition row is nonzero");
        let packed = (u64::from(region) << 32) | u64::from(row.get());
        Self {
            packed: NonZeroU64::new(packed).expect("definition reference is nonzero"),
            _brand: PhantomData,
        }
    }

    #[inline(always)]
    pub(super) const fn row_index(self) -> usize {
        (self.packed.get() as u32 - 1) as usize
    }

    #[inline(always)]
    pub(super) const fn region(self) -> u32 {
        (self.packed.get() >> 32) as u32
    }
}

/// One row owner of an immutable definition payload.
#[derive(Clone)]
pub(super) struct DefinitionEntry(Rc<DefinitionPayload>);

/// Words, compiled matcher, and origin share exactly one semantic lifetime.
pub(super) struct DefinitionPayload {
    pub(super) words: Box<[TokenWord]>,
    pub(super) parameter_len: u32,
    pub(super) origin: crate::token::OriginId,
    pub(super) pattern: MacroParameterPattern,
    accounting: crate::memory_accounting::MemoryAccounting,
    memory_words: usize,
}

impl core::ops::Deref for DefinitionEntry {
    type Target = DefinitionPayload;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DefinitionEntry {
    pub(super) fn new(
        words: Box<[TokenWord]>,
        parameter_len: u32,
        origin: crate::token::OriginId,
        pattern: MacroParameterPatternBuilder,
        accounting: crate::memory_accounting::MemoryAccounting,
    ) -> Self {
        let pattern = pattern.finish(&words[..parameter_len as usize]);
        let header = (core::mem::size_of::<DefinitionPayload>()
            + 2 * core::mem::size_of::<usize>())
        .div_ceil(core::mem::size_of::<usize>());
        let memory_words = header
            + ((words.len() + pattern.failure_len()) * core::mem::size_of::<TokenWord>())
                .div_ceil(core::mem::size_of::<usize>());
        accounting.allocate_shared_dynamic(memory_words);
        Self(Rc::new(DefinitionPayload {
            words,
            parameter_len,
            origin,
            pattern,
            accounting,
            memory_words,
        }))
    }

    pub(super) fn contents_equal(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
            || (self.parameter_len == other.parameter_len && self.words == other.words)
    }
}

impl DefinitionPayload {
    #[inline(always)]
    pub(super) fn parameter_words(&self) -> &[TokenWord] {
        &self.words[..self.parameter_len as usize]
    }

    #[inline(always)]
    pub(super) fn replacement_words(&self) -> &[TokenWord] {
        &self.words[self.parameter_len as usize..]
    }
}

/// Executing macro replacement body.
///
/// Admission retains the definition payload once. The packed input frame
/// owns the sole logical position, and every read is one slice index; there
/// is no physical cursor to advance, rebuild, or restore.
pub struct ResidentMacroBody<G> {
    pub(super) payload: Rc<DefinitionPayload>,
    _brand: PhantomData<fn(&G) -> &G>,
}

/// One immutable macro definition admitted for activation.
///
/// A parameterless empty replacement retains no body until an exceptional
/// observed or recovery path explicitly requests the traditional empty input
/// row.
pub enum AdmittedMacroDefinition<G> {
    SimpleMacro { body: Option<ResidentMacroBody<G>> },
    MatchingMacro { body: ResidentMacroBody<G> },
}

impl<G> ResidentMacroBody<G> {
    #[inline(always)]
    pub(super) fn admit(entry: &DefinitionEntry) -> Self {
        Self {
            payload: Rc::clone(&entry.0),
            _brand: PhantomData,
        }
    }

    /// Number of replacement words.
    #[must_use]
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.payload.words.len() - self.payload.parameter_len as usize
    }

    #[must_use]
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of parameter-text words retained with this active macro.
    #[must_use]
    #[inline(always)]
    pub fn parameter_len(&self) -> usize {
        self.payload.parameter_len as usize
    }

    /// The whole definition: its parameter text, then its replacement.
    #[must_use]
    #[inline(always)]
    pub fn definition_words(&self) -> &[TokenWord] {
        &self.payload.words
    }

    /// Borrows the sealed matcher owned by this retained definition.
    #[must_use]
    #[inline(always)]
    pub fn parameter_pattern(&self) -> &MacroParameterPattern {
        &self.payload.pattern
    }

    #[must_use]
    #[inline(always)]
    pub fn parameter_words(&self) -> &[TokenWord] {
        self.payload.parameter_words()
    }

    /// Reads one parameter-text word.
    #[must_use]
    #[inline]
    pub fn parameter_word(&self, position: usize) -> Option<TokenWord> {
        self.payload.words[..self.payload.parameter_len as usize]
            .get(position)
            .copied()
    }

    /// Reads one replacement word at a logical frame position.
    #[must_use]
    #[inline(always)]
    pub fn word(&self, position: usize) -> Option<TokenWord> {
        self.replacement().get(position).copied()
    }

    /// Reads one replacement word at the packed frame's position.
    #[must_use]
    #[inline(always)]
    pub fn word_at(&self, position: u32) -> Option<TokenWord> {
        self.payload
            .words
            .get(self.payload.parameter_len as usize + position as usize)
            .copied()
    }

    /// Lends the replacement suffix beginning at a logical frame position.
    #[must_use]
    #[inline(always)]
    pub fn replacement_from(&self, position: u32) -> &[TokenWord] {
        self.replacement().get(position as usize..).unwrap_or(&[])
    }

    #[inline(always)]
    fn replacement(&self) -> &[TokenWord] {
        &self.payload.words[self.payload.parameter_len as usize..]
    }

    /// Number of live holders of this body's word span, including the
    /// definition row when it is still published.
    #[cfg(any(test, feature = "profiling", feature = "testing"))]
    #[doc(hidden)]
    #[must_use]
    pub fn profile_region_owner_count(&self) -> usize {
        Rc::strong_count(&self.payload)
    }
}

impl<G> Clone for ResidentMacroBody<G> {
    fn clone(&self) -> Self {
        Self {
            payload: Rc::clone(&self.payload),
            _brand: PhantomData,
        }
    }
}

impl<G> core::fmt::Debug for ResidentMacroBody<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ResidentMacroBody(..)")
    }
}

impl<G> PartialEq for ResidentMacroBody<G> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.payload, &other.payload)
    }
}

impl<G> Eq for ResidentMacroBody<G> {}

impl<G> core::hash::Hash for ResidentMacroBody<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        (Rc::as_ptr(&self.payload) as usize).hash(state);
    }
}

pub struct DefinitionView<G> {
    pub(super) entry: DefinitionEntry,
    pub(super) _brand: PhantomData<fn(&G) -> &G>,
}

#[derive(Clone, Copy)]
pub struct DefinitionWords<'a> {
    words: &'a [TokenWord],
}

impl<'a> DefinitionWords<'a> {
    #[must_use]
    pub const fn len(self) -> usize {
        self.words.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.words.is_empty()
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<TokenWord> {
        self.words.get(index).copied()
    }

    #[must_use]
    pub fn iter(self) -> DefinitionWordIter<'a> {
        DefinitionWordIter {
            words: self.words.iter(),
        }
    }

    #[must_use]
    pub const fn as_slice(self) -> &'a [TokenWord] {
        self.words
    }

    #[must_use]
    pub fn to_vec(self) -> Vec<TokenWord> {
        self.words.to_vec()
    }
}

impl core::fmt::Debug for DefinitionWords<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<T> PartialEq<T> for DefinitionWords<'_>
where
    T: AsRef<[TokenWord]> + ?Sized,
{
    fn eq(&self, other: &T) -> bool {
        self.words == other.as_ref()
    }
}

impl PartialEq for DefinitionWords<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.words == other.words
    }
}

impl Eq for DefinitionWords<'_> {}

impl<'a> IntoIterator for DefinitionWords<'a> {
    type Item = TokenWord;
    type IntoIter = DefinitionWordIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct DefinitionWordIter<'a> {
    words: core::slice::Iter<'a, TokenWord>,
}

impl Iterator for DefinitionWordIter<'_> {
    type Item = TokenWord;

    fn next(&mut self) -> Option<Self::Item> {
        self.words.next().copied()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.words.size_hint()
    }
}

impl ExactSizeIterator for DefinitionWordIter<'_> {}

impl<G> DefinitionView<G> {
    #[must_use]
    pub fn parameter_pattern(&self) -> &MacroParameterPattern {
        &self.entry.pattern
    }

    #[must_use]
    pub fn parameter_text(&self) -> DefinitionWords<'_> {
        DefinitionWords {
            words: self.entry.parameter_words(),
        }
    }

    #[must_use]
    pub fn replacement_text(&self) -> DefinitionWords<'_> {
        DefinitionWords {
            words: self.entry.replacement_words(),
        }
    }

    #[must_use]
    pub fn replacement_word(&self, index: usize) -> Option<TokenWord> {
        self.entry.replacement_words().get(index).copied()
    }

    /// Computes the allocation-independent content identity only at a cold
    /// semantic-state boundary. Definition construction and delivery never
    /// maintain or carry this value.
    pub(crate) fn semantic_identity(&self) -> Option<u64> {
        let mut identity = StateHasher::new(DEFINITION_IDENTITY_V2_DOMAIN);
        identity.tag(PARAMETER_START);
        for word in self.entry.parameter_words() {
            identity.u32(word.raw());
        }
        identity.tag(PARAMETER_END);
        identity.u32(self.entry.parameter_len);
        identity.tag(REPLACEMENT_START);
        let replacement = self.entry.replacement_words();
        for word in replacement {
            identity.u32(word.raw());
        }
        identity.tag(REPLACEMENT_END);
        identity.u32(replacement.len() as u32);
        Some(identity.finish().max(1))
    }

    #[must_use]
    pub fn definition_origin(&self) -> crate::token::OriginId {
        self.entry.origin
    }

    pub(crate) fn capture_format(&self) -> crate::format::schema::FormatDefinition {
        crate::format::schema::FormatDefinition {
            parameter_text: self
                .entry
                .parameter_words()
                .iter()
                .map(|word| word.raw())
                .collect(),
            replacement_text: self
                .entry
                .replacement_words()
                .iter()
                .map(|word| word.raw())
                .collect(),
        }
    }
}

impl Drop for DefinitionPayload {
    fn drop(&mut self) {
        self.accounting.release_shared_dynamic(self.memory_words);
    }
}
