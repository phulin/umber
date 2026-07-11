//! Immutable hash-consed token-list storage.
//!
//! Token-list watermarks are crate-private so rollback can stay coupled to
//! the aggregate `Universe` boundary.

use crate::ids::TokenListId;
use crate::token::Token;
use ahash::RandomState;
use std::collections::HashMap;
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::collections::hash_map::DefaultHasher;
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::Hash;
use std::hash::{BuildHasherDefault, Hasher};

type TokenIndex = HashMap<u64, Vec<TokenListId>, BuildHasherDefault<PrehashedU64Hasher>>;

/// Identity hasher for an index key that is already a keyed content hash.
#[derive(Default)]
struct PrehashedU64Hasher(u64);

impl Hasher for PrehashedU64Hasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // `TokenIndex` has only u64 keys, whose `Hash` implementation calls
        // `write_u64`. Keep a valid fallback for the general Hasher contract.
        let mut value = 0xcbf2_9ce4_8422_2325_u64;
        for &byte in bytes {
            value ^= u64::from(byte);
            value = value.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 = value;
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

/// A rollback watermark for the token store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TokenStoreMark {
    pub(crate) spans: u32,
    tokens: u32,
}

/// An owned scratch buffer for building a token list before freezing it.
#[derive(Clone, Debug)]
pub struct TokenListBuilder {
    buf: Vec<Token>,
}

impl TokenListBuilder {
    /// Creates an empty reusable token-list builder.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Appends one token to the unfinished list.
    pub fn push(&mut self, token: Token) {
        self.buf.push(token);
    }

    /// Returns the number of tokens currently buffered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns whether the builder currently holds no tokens.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Clears the unfinished list without interning it.
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Interns the current token list and clears the builder for reuse.
    pub(crate) fn finish(&mut self, store: &mut TokenStore) -> TokenListId {
        let id = store.intern(&self.buf);
        self.buf.clear();
        id
    }
}

/// Hash-consed immutable token-list arena.
#[derive(Clone, Debug)]
pub struct TokenStore {
    arena: Vec<Token>,
    spans: Vec<(u32, u32)>,
    index: TokenIndex,
    hash_state: RandomState,
    index_dirty: bool,
}

impl TokenStore {
    /// Creates a token store containing the canonical empty list.
    #[must_use]
    pub(crate) fn new() -> Self {
        let hash_state = RandomState::new();
        let mut store = Self {
            arena: Vec::new(),
            spans: vec![(0, 0)],
            index: TokenIndex::default(),
            hash_state,
            index_dirty: false,
        };
        store
            .index
            .entry(store.content_hash(&[]))
            .or_default()
            .push(Self::empty_id());
        store
    }

    /// Creates a fresh owned scratch builder.
    #[must_use]
    pub(crate) fn builder() -> TokenListBuilder {
        TokenListBuilder::new()
    }

    /// Returns the canonical empty token-list id.
    #[must_use]
    pub const fn empty_id() -> TokenListId {
        TokenListId::new(0)
    }

    /// Interns `tokens`, returning a dense id for the live token-list content.
    pub(crate) fn intern(&mut self, tokens: &[Token]) -> TokenListId {
        if tokens.is_empty() {
            return Self::empty_id();
        }

        if self.index_dirty {
            self.rebuild_index();
        }

        let hash = self.content_hash(tokens);
        if let Some(candidates) = self.index.get(&hash) {
            for &id in candidates {
                // Hash collisions are safe because the candidate span is
                // compared by content before the id is reused.
                if self.get(id) == tokens {
                    return id;
                }
            }
        }

        let start = u32_len(self.arena.len(), "token arena exceeds u32 entries");
        let len = u32_len(tokens.len(), "token list exceeds u32 entries");
        let id = TokenListId::new(u32_len(
            self.spans.len(),
            "token-list spans exceed u32 entries",
        ));

        self.arena.extend_from_slice(tokens);
        self.spans.push((start, len));
        self.index.entry(hash).or_default().push(id);
        id
    }

    /// Reads a live frozen token list.
    #[must_use]
    pub(crate) fn get(&self, id: TokenListId) -> &[Token] {
        let index = id.raw() as usize;
        assert!(index < self.spans.len(), "token list id is not live");
        let (start, len) = self.spans[index];
        let start = start as usize;
        let end = start + len as usize;
        assert!(end <= self.arena.len(), "token-list span exceeds arena");
        &self.arena[start..end]
    }

    /// Returns whether `id` names a currently-live token-list slot.
    #[must_use]
    pub(crate) fn contains(&self, id: TokenListId) -> bool {
        (id.raw() as usize) < self.spans.len()
    }

    /// Takes a rollback watermark for aggregate snapshots.
    #[must_use]
    pub(crate) fn watermark(&self) -> TokenStoreMark {
        TokenStoreMark {
            spans: u32_len(self.spans.len(), "token-list spans exceed u32 entries"),
            tokens: u32_len(self.arena.len(), "token arena exceeds u32 entries"),
        }
    }

    /// Truncates to a previously-taken aggregate snapshot watermark.
    pub(crate) fn truncate_to(&mut self, mark: TokenStoreMark) {
        let spans = mark.spans as usize;
        let tokens = mark.tokens as usize;
        assert!(spans >= 1, "token-store mark removes the empty list");
        assert!(
            spans <= self.spans.len(),
            "token-store mark has too many spans"
        );
        assert!(
            tokens <= self.arena.len(),
            "token-store mark has too many tokens"
        );
        assert!(
            self.spans[..spans]
                .last()
                .is_some_and(|&(start, len)| start + len == mark.tokens),
            "token-store mark does not point to a span boundary"
        );

        self.spans.truncate(spans);
        self.arena.truncate(tokens);
        self.index_dirty = true;
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub(crate) fn testing_state_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.spans.len().hash(&mut hasher);
        for raw in 0..self.spans.len() {
            let id = TokenListId::new(u32_len(raw, "token-list spans exceed u32 entries"));
            self.get(id).hash(&mut hasher);
        }
        hasher.finish()
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for raw in 0..self.spans.len() {
            let id = TokenListId::new(u32_len(raw, "token-list spans exceed u32 entries"));
            let hash = self.content_hash(self.get(id));
            self.index.entry(hash).or_default().push(id);
        }
        self.index_dirty = false;
    }

    fn content_hash(&self, tokens: &[Token]) -> u64 {
        self.hash_state.hash_one(tokens)
    }
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests;
