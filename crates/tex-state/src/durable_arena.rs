//! Typed publishers for generation-durable immutable values.

use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::sync::atomic::{AtomicU32, Ordering};
use std::rc::Rc;

use crate::generation::ArenaToken;
use crate::glue::GlueSpec;
use crate::memory_accounting::MemoryAccounting;
use crate::provenance::OriginRecord;
use crate::token::TokenWord;

#[cfg(test)]
#[path = "durable_arena/tests.rs"]
mod tests;

pub(super) enum TokenListNamespace {}
pub(super) enum GlueNamespace {}
pub(super) enum ProvenanceNamespace {}

const TOKEN_CHUNK_WORDS: usize = 64;
const NO_CHUNK: u32 = u32::MAX;
static NEXT_TOKEN_STORE_OWNER: AtomicU32 = AtomicU32::new(1);

fn fresh_token_store_owner() -> u32 {
    NEXT_TOKEN_STORE_OWNER
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |owner| {
            owner.checked_add(1)
        })
        .expect("generation token-store owner domain exhausted")
}

macro_rules! semantic_dense_id {
    ($name:ident) => {
        pub struct $name<G> {
            packed: u64,
            _brand: PhantomData<fn(&G) -> &G>,
        }

        impl<G> Clone for $name<G> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<G> Copy for $name<G> {}

        impl<G> PartialEq for $name<G> {
            fn eq(&self, other: &Self) -> bool {
                self.packed as u32 == other.packed as u32
            }
        }

        impl<G> Eq for $name<G> {}

        impl<G> core::hash::Hash for $name<G> {
            fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
                (self.packed as u32).hash(state);
            }
        }

        impl<G> core::fmt::Debug for $name<G> {
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(..)"))
            }
        }

        impl<G> $name<G> {
            fn from_row_with_semantic(row: NonZeroU32, semantic_identity: u64) -> Self {
                Self {
                    packed: u64::from(row.get())
                        | (u64::from((semantic_identity as u32).max(1)) << 32),
                    _brand: PhantomData,
                }
            }

            fn index(self) -> usize {
                self.packed as u32 as usize - 1
            }

            pub(crate) fn format_index(self) -> u32 {
                self.packed as u32 - 1
            }

            pub(crate) const fn semantic_identity(self) -> Option<u64> {
                match self.packed >> 32 {
                    0 => None,
                    identity => Some(identity),
                }
            }
        }
    };
}

semantic_dense_id!(GlueId);

pub struct ProvenanceId<G> {
    row: NonZeroU32,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for ProvenanceId<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> Copy for ProvenanceId<G> {}
impl<G> PartialEq for ProvenanceId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row
    }
}
impl<G> Eq for ProvenanceId<G> {}
impl<G> core::hash::Hash for ProvenanceId<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.row.hash(state);
    }
}
impl<G> core::fmt::Debug for ProvenanceId<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ProvenanceId(..)")
    }
}
impl<G> ProvenanceId<G> {
    fn from_row(row: NonZeroU32) -> Self {
        Self {
            row,
            _brand: PhantomData,
        }
    }
    fn index(self) -> usize {
        self.row.get() as usize - 1
    }
    pub(crate) fn format_index(self) -> u32 {
        self.row.get() - 1
    }
}

/// Shared owner of one immutable stored token list.
///
/// The wrapper is generation branded and deliberately non-`Copy`. Cloning it
/// records a genuine semantic alias through non-atomic shared ownership.
pub struct TokenListId<G> {
    packed_identity: u64,
    words: Rc<[TokenWord]>,
    accounting: MemoryAccounting,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for TokenListId<G> {
    fn clone(&self) -> Self {
        Self {
            packed_identity: self.packed_identity,
            words: Rc::clone(&self.words),
            accounting: self.accounting.clone(),
            _brand: PhantomData,
        }
    }
}

impl<G> Drop for TokenListId<G> {
    fn drop(&mut self) {
        if Rc::strong_count(&self.words) == 1 {
            self.accounting
                .release_shared_dynamic(token_list_memory_words(self.words.len()));
        }
    }
}

impl<G> PartialEq for TokenListId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.packed_identity as u32 == other.packed_identity as u32
    }
}

impl<G> Eq for TokenListId<G> {}

impl<G> core::hash::Hash for TokenListId<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        (self.packed_identity as u32).hash(state);
    }
}

impl<G> core::fmt::Debug for TokenListId<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("TokenListId(..)")
    }
}

impl<G> TokenListId<G> {
    fn from_words(
        serial: NonZeroU32,
        words: Rc<[TokenWord]>,
        accounting: MemoryAccounting,
        semantic_identity: u64,
    ) -> Self {
        let memory_words = token_list_memory_words(words.len());
        accounting.allocate_shared_dynamic(memory_words);
        Self {
            packed_identity: u64::from(serial.get())
                | (u64::from((semantic_identity as u32).max(u32::from(semantic_identity != 0)))
                    << 32),
            words,
            accounting,
            _brand: PhantomData,
        }
    }

    pub(crate) const fn semantic_identity(&self) -> Option<u64> {
        match self.packed_identity >> 32 {
            0 => None,
            identity => Some(identity),
        }
    }

    /// Borrows one word from this immutable packed token span.
    #[must_use]
    pub fn word_at(&self, index: usize) -> Option<TokenWord> {
        self.words.get(index).copied()
    }

    pub(crate) const fn format_index(&self) -> u32 {
        self.packed_identity as u32 - 1
    }

    pub(crate) fn capture_format(&self) -> Vec<u32> {
        self.words.iter().map(|word| word.raw()).collect()
    }

    #[cfg(test)]
    pub(crate) fn semantic_owner_count(&self) -> usize {
        Rc::strong_count(&self.words)
    }

    pub(crate) fn format_validation_coordinate(index: u32) -> Option<Self> {
        index
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .map(|serial| {
                Self::from_words(
                    serial,
                    Rc::from(Vec::<TokenWord>::new().into_boxed_slice()),
                    MemoryAccounting::default(),
                    0,
                )
            })
    }
}

fn token_list_memory_words(word_len: usize) -> usize {
    word_len
        .checked_add(1)
        .expect("validated token-list length has a canonical word count")
}

impl<G> ProvenanceId<G> {
    pub(crate) fn from_origin_index(index: u32) -> Option<Self> {
        index
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .map(Self::from_row)
    }
}

/// Failure to stage a complete durable row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableAllocationError {
    CapacityOverflow,
    AllocationFailed,
}

fn next_row(len: usize) -> Result<NonZeroU32, DurableAllocationError> {
    len.checked_add(1)
        .and_then(|row| u32::try_from(row).ok())
        .and_then(NonZeroU32::new)
        .ok_or(DurableAllocationError::CapacityOverflow)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TokenChunk {
    words: [TokenWord; TOKEN_CHUNK_WORDS],
    len: u8,
    next: u32,
}

impl Default for TokenChunk {
    fn default() -> Self {
        Self {
            words: [TokenWord::from_raw(0); TOKEN_CHUNK_WORDS],
            len: 0,
            next: NO_CHUNK,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BuilderSlot {
    serial: u64,
    head: u32,
    tail: u32,
    len: u32,
    live: bool,
}

impl Default for BuilderSlot {
    fn default() -> Self {
        Self {
            serial: 0,
            head: NO_CHUNK,
            tail: NO_CHUNK,
            len: 0,
            live: false,
        }
    }
}

/// An unpublished destination-directed durable token-list construction.
///
/// Constructors and coordinates are private. The invariant brand prevents a
/// builder from being used through another admitted generation.
#[must_use = "a durable token-list builder must be sealed or discarded"]
pub struct TokenListBuilder<G> {
    slot: u32,
    serial: u64,
    _brand: PhantomData<fn(&G) -> &G>,
}

/// A stable sequential coordinate into one sealed durable token list.
///
/// The fields are private so suspended command state can retain the typed
/// coordinate but cannot forge a cross-list or cross-generation position.
#[derive(Debug)]
pub struct TokenListCursor<G> {
    list: TokenListId<G>,
    offset: u32,
}

impl<G> Clone for TokenListCursor<G> {
    fn clone(&self) -> Self {
        Self {
            list: self.list.clone(),
            offset: self.offset,
        }
    }
}
impl<G> TokenListCursor<G> {
    #[must_use]
    pub fn list(&self) -> TokenListId<G> {
        self.list.clone()
    }

    /// Delivers and advances one word through this cursor's already-owned
    /// immutable payload. The cursor itself is the lifetime proof, so the hot
    /// sequential path needs no temporary shared-owner clone or arena lookup.
    #[must_use]
    pub fn next_word(&mut self) -> Option<TokenWord> {
        let word = self.current_word()?;
        self.offset += 1;
        Some(word)
    }

    /// Borrows the word at the current sequential position without consuming
    /// it. This supports canonical lookahead without cloning the exact owner.
    #[must_use]
    pub fn current_word(&self) -> Option<TokenWord> {
        self.list.words.get(self.offset as usize).copied()
    }

    /// Advances a cursor after a successful borrowed lookahead.
    #[must_use]
    pub fn advance(&mut self) -> bool {
        if self.offset as usize >= self.list.words.len() {
            return false;
        }
        self.offset += 1;
        true
    }
}
impl<G> PartialEq for TokenListCursor<G> {
    fn eq(&self, other: &Self) -> bool {
        self.list == other.list && self.offset == other.offset
    }
}
impl<G> Eq for TokenListCursor<G> {}
impl<G> core::hash::Hash for TokenListCursor<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.list.hash(state);
        self.offset.hash(state);
    }
}

/// Cheap owning sequential view of one sealed durable token list.
pub struct TokenListView<G> {
    list: TokenListId<G>,
}

impl<G> Clone for TokenListView<G> {
    fn clone(&self) -> Self {
        Self {
            list: self.list.clone(),
        }
    }
}

impl<G> TokenListView<G> {
    #[must_use]
    pub fn len(&self) -> usize {
        self.list.words.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.words.is_empty()
    }

    /// Transfers this immutable span's owner without another shared-owner
    /// clone. Input uses this once while adapting a durable list to its
    /// uniform packed span handle.
    #[must_use]
    pub fn into_id(self) -> TokenListId<G> {
        self.list
    }

    #[must_use]
    pub fn cursor(&self) -> TokenListCursor<G> {
        TokenListCursor {
            list: self.list.clone(),
            offset: 0,
        }
    }

    #[must_use]
    pub fn iter(&self) -> TokenListWords<G> {
        TokenListWords {
            cursor: self.cursor(),
        }
    }

    /// Cold random access for bounded diagnostic projections.
    /// Sequential runtime replay must retain and advance a cursor instead.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<TokenWord> {
        let mut words = self.iter();
        words.nth(index)
    }
}

impl<G> core::fmt::Debug for TokenListView<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<G> PartialEq for TokenListView<G> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<G> Eq for TokenListView<G> {}

impl<G> core::hash::Hash for TokenListView<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        for word in self.iter() {
            word.hash(state);
        }
    }
}

/// Allocation-free owning iterator over a sealed durable token list.
pub struct TokenListWords<G> {
    cursor: TokenListCursor<G>,
}

impl<G> Clone for TokenListWords<G> {
    fn clone(&self) -> Self {
        Self {
            cursor: self.cursor.clone(),
        }
    }
}

impl<G> Iterator for TokenListWords<G> {
    type Item = TokenWord;

    fn next(&mut self) -> Option<Self::Item> {
        self.cursor.next_word()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.cursor.list.words.len() - self.cursor.offset as usize;
        (len, Some(len))
    }
}
impl<G> ExactSizeIterator for TokenListWords<G> {}

impl<G> IntoIterator for TokenListView<G> {
    type Item = TokenWord;
    type IntoIter = TokenListWords<G>;

    fn into_iter(self) -> Self::IntoIter {
        TokenListWords {
            cursor: TokenListCursor {
                list: self.list,
                offset: 0,
            },
        }
    }
}

/// Durable token-list publisher, physically separate from definition text.
///
/// Builder chunks are reusable scratch for publication. Sealing copies the
/// words into their final shared allocation and immediately recycles the
/// builder chain. The publisher retains one owner so node token coordinates
/// remain valid until rollback removes their row or the generation retires.
pub(crate) struct TokenListArena<G> {
    owner: u32,
    next_serial: u32,
    next_publication_serial: u32,
    published: Vec<PublishedTokenList>,
    chunks: Vec<TokenChunk>,
    builder_slots: Vec<BuilderSlot>,
    free_builder_slots: Vec<u32>,
    free_chunk_head: u32,
    next_builder_serial: u64,
    accounting: MemoryAccounting,
    semantic_identity_enabled: bool,
    _brand: PhantomData<fn(&G) -> &G>,
}

struct PublishedTokenList {
    words: Rc<[TokenWord]>,
    publication_serial: u32,
    accounting: MemoryAccounting,
}

impl Drop for PublishedTokenList {
    fn drop(&mut self) {
        if Rc::strong_count(&self.words) == 1 {
            self.accounting
                .release_shared_dynamic(token_list_memory_words(self.words.len()));
        }
    }
}

pub(crate) struct AcceptedTokenListTail {
    published: Vec<PublishedTokenList>,
}

impl<G> TokenListArena<G> {
    pub(crate) const fn cursor(&self) -> u32 {
        self.next_serial
    }

    pub(crate) fn restore_cursor(&mut self, cursor: u32) {
        assert!(
            cursor <= self.next_serial,
            "token-list cursor {cursor} is beyond publisher head {}",
            self.next_serial
        );
        assert!(self.builder_slots.iter().all(|slot| !slot.live));
        self.published.truncate(cursor as usize);
        self.next_serial = cursor;
        self.chunks.clear();
        self.builder_slots.clear();
        self.free_builder_slots.clear();
        self.free_chunk_head = NO_CHUNK;
        self.next_builder_serial = 1;
    }

    pub(super) fn new(
        _token: ArenaToken<G, TokenListNamespace>,
        accounting: MemoryAccounting,
    ) -> Self {
        Self {
            owner: fresh_token_store_owner(),
            next_serial: 0,
            next_publication_serial: 1,
            published: Vec::new(),
            chunks: Vec::new(),
            builder_slots: Vec::new(),
            free_builder_slots: Vec::new(),
            free_chunk_head: NO_CHUNK,
            next_builder_serial: 1,
            accounting,
            semantic_identity_enabled: false,
            _brand: PhantomData,
        }
    }

    /// Cold/source-admission wrapper which streams once into final chunks.
    /// Runtime scanners must use the destination-directed builder methods.
    pub(crate) fn allocate(
        &mut self,
        words: &[TokenWord],
    ) -> Result<TokenListId<G>, DurableAllocationError> {
        self.allocate_from_iter(words.iter().copied())
    }

    /// Publishes one cold token list from an exact-size word stream.
    ///
    /// Format admission uses this path to translate packed wire words directly
    /// into their final generation chunks without an intermediate word vector.
    pub(crate) fn allocate_from_iter<Words>(
        &mut self,
        words: Words,
    ) -> Result<TokenListId<G>, DurableAllocationError>
    where
        Words: ExactSizeIterator<Item = TokenWord>,
    {
        self.reserve_batch(1, words.len())?;
        let builder = self.begin_builder()?;
        for word in words {
            if let Err(error) = self.push_builder_word(&builder, word) {
                let _ = self.discard_builder(builder);
                return Err(error);
            }
        }
        self.seal_builder(builder)
    }

    /// Reserves a complete promotion batch without publishing a row.
    pub(crate) fn reserve_batch(
        &mut self,
        rows: usize,
        words: usize,
    ) -> Result<(), DurableAllocationError> {
        (self.next_serial as usize)
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        let chunks = rows
            .min(words)
            .checked_add(words / TOKEN_CHUNK_WORDS)
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        self.chunks
            .len()
            .checked_add(chunks)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        self.chunks
            .try_reserve(chunks)
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        self.published
            .try_reserve(rows)
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        if self.builder_slots.is_empty() {
            self.builder_slots
                .try_reserve(1)
                .map_err(|_| DurableAllocationError::AllocationFailed)?;
            self.free_builder_slots
                .try_reserve(1)
                .map_err(|_| DurableAllocationError::AllocationFailed)?;
        }
        Ok(())
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: TokenListId<G>) -> TokenListView<G> {
        TokenListView { list: id }
    }

    #[must_use]
    pub(crate) fn node_words(&self, key: crate::node::NodeTokenKey) -> Option<&[TokenWord]> {
        if key.is_empty() {
            return (key == crate::node::NodeTokenKey::default()).then_some(&[]);
        }
        let [
            owner,
            block_ordinal,
            incarnation,
            offset,
            len,
            publication_serial,
        ] = key.coordinates();
        if owner != self.owner || incarnation != 1 || offset != 0 {
            return None;
        }
        let published = self.published.get(block_ordinal as usize)?;
        if published.publication_serial != publication_serial
            || published.words.len() != len as usize
        {
            return None;
        }
        Some(&published.words)
    }

    pub(crate) fn node_key(&self, id: &TokenListId<G>) -> Option<crate::node::NodeTokenKey> {
        let block_ordinal = id.format_index();
        let published = self.published.get(block_ordinal as usize)?;
        if !Rc::ptr_eq(&published.words, &id.words) {
            return None;
        }
        if published.words.is_empty() {
            return Some(crate::node::NodeTokenKey::default());
        }
        Some(crate::node::NodeTokenKey::new(
            self.owner,
            block_ordinal,
            1,
            0,
            u32::try_from(published.words.len()).ok()?,
            published.publication_serial,
        ))
    }

    pub(crate) fn append_node_words_to_builder(
        &mut self,
        builder: &TokenListBuilder<G>,
        key: crate::node::NodeTokenKey,
    ) -> Result<(), DurableAllocationError> {
        if key.is_empty() {
            return (key == crate::node::NodeTokenKey::default())
                .then_some(())
                .ok_or(DurableAllocationError::CapacityOverflow);
        }
        self.node_words(key)
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        let words = Rc::clone(&self.published[key.coordinates()[1] as usize].words);
        for &word in words.iter() {
            self.push_builder_word(builder, word)?;
        }
        Ok(())
    }

    pub(crate) fn begin_checkpoint_candidate(&mut self, cursor: u32) -> AcceptedTokenListTail {
        assert!(cursor <= self.next_serial);
        let published = self.published.split_off(cursor as usize);
        self.next_serial = cursor;
        AcceptedTokenListTail { published }
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        cursor: u32,
        mut tail: AcceptedTokenListTail,
    ) {
        self.restore_cursor(cursor);
        self.published.append(&mut tail.published);
        self.next_serial = self.published.len() as u32;
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self, _tail: AcceptedTokenListTail) {}

    pub(crate) fn begin_builder(&mut self) -> Result<TokenListBuilder<G>, DurableAllocationError> {
        let slot = if let Some(slot) = self.free_builder_slots.pop() {
            slot
        } else {
            let slot = u32::try_from(self.builder_slots.len())
                .map_err(|_| DurableAllocationError::CapacityOverflow)?;
            self.builder_slots
                .try_reserve(1)
                .map_err(|_| DurableAllocationError::AllocationFailed)?;
            let needed_free_capacity = self.builder_slots.len() + 1;
            self.free_builder_slots
                .try_reserve(
                    needed_free_capacity.saturating_sub(self.free_builder_slots.capacity()),
                )
                .map_err(|_| DurableAllocationError::AllocationFailed)?;
            self.builder_slots.push(BuilderSlot::default());
            slot
        };
        let serial = self.next_builder_serial;
        self.next_builder_serial = self.next_builder_serial.wrapping_add(1).max(1);
        self.builder_slots[slot as usize] = BuilderSlot {
            serial,
            live: true,
            ..BuilderSlot::default()
        };
        Ok(TokenListBuilder {
            slot,
            serial,
            _brand: PhantomData,
        })
    }

    /// Appends the final semantic word in O(1). Token origins deliberately do
    /// not belong to durable token-list identity; scanner sinks extract the
    /// token lane from `TracedTokenWord` at emission and append it here.
    pub(crate) fn push_builder_word(
        &mut self,
        builder: &TokenListBuilder<G>,
        word: TokenWord,
    ) -> Result<(), DurableAllocationError> {
        let slot = self.builder_slot(builder)?;
        let mut head = slot.head;
        let mut tail = slot.tail;
        let len = slot
            .len
            .checked_add(1)
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        let needs_chunk =
            tail == NO_CHUNK || self.chunks[tail as usize].len as usize == TOKEN_CHUNK_WORDS;
        if needs_chunk {
            let chunk = self.allocate_chunk()?;
            if tail == NO_CHUNK {
                head = chunk;
            } else {
                self.chunks[tail as usize].next = chunk;
            }
            tail = chunk;
        }
        let chunk = &mut self.chunks[tail as usize];
        chunk.words[chunk.len as usize] = word;
        chunk.len += 1;
        let slot = self.builder_slot_mut(builder)?;
        slot.head = head;
        slot.tail = tail;
        slot.len = len;
        Ok(())
    }

    pub(crate) fn seal_builder(
        &mut self,
        builder: TokenListBuilder<G>,
    ) -> Result<TokenListId<G>, DurableAllocationError> {
        let serial = next_row(self.next_serial as usize)?;
        let slot = *self.builder_slot(&builder)?;
        let mut chunk = slot.head;
        let mut remaining = slot.len as usize;
        let words = std::iter::from_fn(|| {
            if remaining == 0 {
                return None;
            }
            let current = &self.chunks[chunk as usize];
            let consumed = slot.len as usize - remaining;
            let offset = consumed % TOKEN_CHUNK_WORDS;
            let word = current.words[offset];
            remaining -= 1;
            if offset + 1 == current.len as usize {
                chunk = current.next;
            }
            Some(word)
        })
        .collect::<Rc<[_]>>();
        let publication_serial = self.next_publication_serial;
        self.next_publication_serial = self
            .next_publication_serial
            .checked_add(1)
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        let semantic_identity = if self.semantic_identity_enabled {
            crate::state_hash::semantic_scalar_root(0x746f_6b65_6e73_7631, |hasher| {
                hasher.usize(words.len());
                for word in words.iter() {
                    hasher.u32(word.raw());
                }
            })
            .max(1)
        } else {
            0
        };
        self.release_builder_slot(builder, true)?;
        self.published.push(PublishedTokenList {
            words: Rc::clone(&words),
            publication_serial,
            accounting: self.accounting.clone(),
        });
        self.next_serial = serial.get();
        Ok(TokenListId::from_words(
            serial,
            words,
            self.accounting.clone(),
            semantic_identity,
        ))
    }

    pub(crate) fn enable_semantic_identity(&mut self) -> bool {
        if self.semantic_identity_enabled {
            return true;
        }
        if self.next_serial != 0 {
            return false;
        }
        self.semantic_identity_enabled = true;
        true
    }

    pub(crate) fn discard_builder(
        &mut self,
        builder: TokenListBuilder<G>,
    ) -> Result<(), DurableAllocationError> {
        self.release_builder_slot(builder, true)
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

    fn builder_slot(
        &self,
        builder: &TokenListBuilder<G>,
    ) -> Result<&BuilderSlot, DurableAllocationError> {
        self.builder_slots
            .get(builder.slot as usize)
            .filter(|slot| slot.live && slot.serial == builder.serial)
            .ok_or(DurableAllocationError::CapacityOverflow)
    }

    fn builder_slot_mut(
        &mut self,
        builder: &TokenListBuilder<G>,
    ) -> Result<&mut BuilderSlot, DurableAllocationError> {
        self.builder_slots
            .get_mut(builder.slot as usize)
            .filter(|slot| slot.live && slot.serial == builder.serial)
            .ok_or(DurableAllocationError::CapacityOverflow)
    }

    fn allocate_chunk(&mut self) -> Result<u32, DurableAllocationError> {
        if self.free_chunk_head != NO_CHUNK {
            let chunk = self.free_chunk_head;
            self.free_chunk_head = self.chunks[chunk as usize].next;
            self.chunks[chunk as usize] = TokenChunk::default();
            return Ok(chunk);
        }
        let chunk = u32::try_from(self.chunks.len())
            .map_err(|_| DurableAllocationError::CapacityOverflow)?;
        self.chunks
            .try_reserve(1)
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        self.chunks.push(TokenChunk::default());
        Ok(chunk)
    }

    fn release_builder_slot(
        &mut self,
        builder: TokenListBuilder<G>,
        release_chunks: bool,
    ) -> Result<(), DurableAllocationError> {
        let slot = self.builder_slot_mut(&builder)?;
        let head = slot.head;
        let tail = slot.tail;
        *slot = BuilderSlot::default();
        if release_chunks && head != NO_CHUNK {
            self.chunks[tail as usize].next = self.free_chunk_head;
            self.free_chunk_head = head;
        }
        self.free_builder_slots.push(builder.slot);
        Ok(())
    }

    #[cfg(test)]
    const fn retained_chunk_len(&self) -> usize {
        self.chunks.len()
    }

    #[cfg(test)]
    const fn retained_builder_slot_len(&self) -> usize {
        self.builder_slots.len()
    }
}

/// Durable immutable glue specifications.
pub(crate) struct GlueArena<G> {
    rows: Vec<GlueSpec>,
    semantic_identity_enabled: bool,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for GlueArena<G> {
    fn clone(&self) -> Self {
        let mut rows = Vec::with_capacity(self.rows.len().saturating_add(64));
        rows.extend_from_slice(&self.rows);
        Self {
            rows,
            semantic_identity_enabled: self.semantic_identity_enabled,
            _brand: PhantomData,
        }
    }
}

impl<G> GlueArena<G> {
    pub(super) fn new(_token: ArenaToken<G, GlueNamespace>) -> Self {
        Self {
            rows: Vec::new(),
            semantic_identity_enabled: false,
            _brand: PhantomData,
        }
    }

    pub(crate) fn allocate(
        &mut self,
        value: GlueSpec,
    ) -> Result<GlueId<G>, DurableAllocationError> {
        let row = next_row(self.rows.len())?;
        self.rows
            .try_reserve(1)
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        self.rows.push(value);
        let semantic_identity = if self.semantic_identity_enabled {
            crate::state_hash::semantic_scalar_root(0x676c_7565_5f76_3100, |hasher| {
                hasher.i32(value.width.raw());
                hasher.i32(value.stretch.raw());
                hasher.u8(value.stretch_order as u8);
                hasher.i32(value.shrink.raw());
                hasher.u8(value.shrink_order as u8);
            })
            .max(1)
        } else {
            0
        };
        Ok(GlueId::from_row_with_semantic(row, semantic_identity))
    }

    pub(crate) fn enable_semantic_identity(&mut self) -> bool {
        if self.semantic_identity_enabled {
            return true;
        }
        if !self.rows.is_empty() {
            return false;
        }
        self.semantic_identity_enabled = true;
        true
    }

    /// Reserves a complete promotion batch without publishing a row.
    pub(crate) fn reserve_batch(&mut self, rows: usize) -> Result<(), DurableAllocationError> {
        self.rows
            .len()
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        self.rows
            .try_reserve(rows)
            .map_err(|_| DurableAllocationError::AllocationFailed)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: GlueId<G>) -> GlueSpec {
        self.rows[id.index()]
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        assert!(len <= self.rows.len(), "glue cursor is beyond the arena");
        self.rows.truncate(len);
    }

    pub(crate) fn split_off(&mut self, len: usize) -> Vec<GlueSpec> {
        assert!(len <= self.rows.len(), "glue cursor is beyond the arena");
        self.rows.split_off(len)
    }

    pub(crate) fn append_rows(&mut self, rows: &mut Vec<GlueSpec>) {
        self.rows.append(rows);
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn capture_format_rows(&self) -> Vec<crate::format::schema::FormatGlue> {
        self.rows
            .iter()
            .map(|value| crate::format::schema::FormatGlue {
                width: value.width.raw(),
                stretch: value.stretch.raw(),
                stretch_order: value.stretch_order as u8,
                shrink: value.shrink.raw(),
                shrink_order: value.shrink_order as u8,
            })
            .collect()
    }
}

/// Durable generation-local provenance records.
pub(crate) struct ProvenanceArena<G> {
    rows: Vec<OriginRecord>,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for ProvenanceArena<G> {
    fn clone(&self) -> Self {
        let mut rows = Vec::with_capacity(self.rows.len().saturating_add(64));
        rows.extend_from_slice(&self.rows);
        Self {
            rows,
            _brand: PhantomData,
        }
    }
}

impl<G> ProvenanceArena<G> {
    pub(super) fn new(_token: ArenaToken<G, ProvenanceNamespace>) -> Self {
        Self {
            rows: Vec::new(),
            _brand: PhantomData,
        }
    }

    pub(crate) fn allocate(
        &mut self,
        value: OriginRecord,
    ) -> Result<ProvenanceId<G>, DurableAllocationError> {
        let row = next_row(self.rows.len())?;
        self.rows
            .try_reserve(1)
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        self.rows.push(value);
        Ok(ProvenanceId::from_row(row))
    }

    pub(crate) fn coordinate_at(&self, index: u32) -> Option<ProvenanceId<G>> {
        ((index as usize) < self.rows.len()).then(|| {
            ProvenanceId::from_origin_index(index).expect("live provenance index is nonzero")
        })
    }

    /// Reserves a complete promotion batch without publishing a row.
    pub(crate) fn reserve_batch(&mut self, rows: usize) -> Result<(), DurableAllocationError> {
        self.rows
            .len()
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        self.rows
            .try_reserve(rows)
            .map_err(|_| DurableAllocationError::AllocationFailed)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: ProvenanceId<G>) -> OriginRecord {
        self.rows[id.index()]
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        assert!(
            len <= self.rows.len(),
            "provenance cursor is beyond the arena"
        );
        self.rows.truncate(len);
    }

    pub(crate) fn split_off(&mut self, len: usize) -> Vec<OriginRecord> {
        assert!(
            len <= self.rows.len(),
            "provenance cursor is beyond the arena"
        );
        self.rows.split_off(len)
    }

    pub(crate) fn append_rows(&mut self, rows: &mut Vec<OriginRecord>) {
        self.rows.append(rows);
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}
