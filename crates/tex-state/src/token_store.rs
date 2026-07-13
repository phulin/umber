//! Immutable hash-consed token-list storage.
//!
//! Token-list watermarks are crate-private so rollback can stay coupled to
//! the aggregate `Universe` boundary.

use crate::identity::{IdentityAllocator, IdentityMark};
use crate::ids::TokenListId;
use crate::state_hash::StateHasher;
use crate::token::{Token, TracedTokenWord};
#[cfg(test)]
use ahash::RandomState;
use std::collections::HashMap;
#[cfg(test)]
use std::hash::Hash;
use std::hash::{BuildHasherDefault, Hasher};

type TokenIndex =
    HashMap<TokenSemanticId, Vec<TokenListId>, BuildHasherDefault<PrehashedU64Hasher>>;

/// Versioned, allocation-independent identity of one immutable token sequence.
///
/// Control sequences contribute their namespace and spelling through the
/// interner's semantic atom; compact runtime symbol keys never participate.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub(crate) struct TokenSemanticId(u64);

impl TokenSemanticId {
    #[must_use]
    pub(crate) const fn value(self) -> u64 {
        self.0
    }
}

/// Current token semantic-identity scheme. Changing token tags, symbol-atom
/// semantics, or the hash framing requires a new version and checkpoint-hash
/// migration notes.
pub(crate) const TOKEN_SEMANTIC_ID_VERSION: u8 = 1;
const TOKEN_STREAM_V1_DOMAIN: u64 = 0x746f_6b31_5f73_7472;
const TOKEN_ID_V1_DOMAIN: u64 = 0x746f_6b31_5f69_6465;

pub(crate) struct TokenSemanticIdBuilder {
    stream: StateHasher,
    len: usize,
}

impl TokenSemanticIdBuilder {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            stream: StateHasher::new(TOKEN_STREAM_V1_DOMAIN),
            len: 0,
        }
    }

    pub(crate) fn push(&mut self, token: Token, symbol_atom: Option<u64>) {
        match token {
            Token::Char { ch, cat } => {
                self.stream.tag(0);
                self.stream.u32(ch as u32);
                self.stream.u8(cat as u8);
            }
            Token::Cs(_) => {
                self.stream.tag(1);
                self.stream
                    .u64(symbol_atom.expect("control-sequence token requires semantic atom"));
            }
            Token::Param(slot) => {
                self.stream.tag(2);
                self.stream.u8(slot);
            }
            Token::Frozen(crate::token::FrozenToken::END_TEMPLATE) => self.stream.tag(3),
            Token::Frozen(crate::token::FrozenToken::END_V) => self.stream.tag(4),
            Token::Frozen(_) => unreachable!("invalid frozen token payload"),
        }
        self.len += 1;
    }

    #[must_use]
    pub(crate) fn finish(self) -> TokenSemanticId {
        let mut hasher = StateHasher::new(TOKEN_ID_V1_DOMAIN);
        hasher.u8(TOKEN_SEMANTIC_ID_VERSION);
        hasher.usize(self.len);
        hasher.u64(self.stream.finish());
        TokenSemanticId(hasher.finish())
    }
}

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
    identities: IdentityMark,
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

    /// Appends a contiguous immutable token span.
    pub fn extend_from_slice(&mut self, tokens: &[Token]) {
        self.buf.extend_from_slice(tokens);
    }

    /// Reserves capacity when the caller already knows the remaining size.
    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional);
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

    /// Borrows the unfinished semantic token sequence for aggregate validation.
    #[must_use]
    pub(crate) fn as_slice(&self) -> &[Token] {
        &self.buf
    }

    /// Interns the current token list and clears the builder for reuse.
    #[cfg(test)]
    pub(crate) fn finish(&mut self, store: &mut TokenStore) -> TokenListId {
        let id = store.intern(&self.buf);
        self.clear();
        id
    }
}

/// Hash-consed immutable token-list arena.
#[derive(Debug)]
pub struct TokenStore {
    arena: Vec<Token>,
    spans: Vec<(u32, u32)>,
    semantic_ids: Vec<TokenSemanticId>,
    index: TokenIndex,
    #[cfg(test)]
    hash_state: RandomState,
    index_dirty: bool,
    identities: IdentityAllocator,
}

impl Clone for TokenStore {
    fn clone(&self) -> Self {
        Self {
            arena: self.arena.clone(),
            spans: self.spans.clone(),
            semantic_ids: self.semantic_ids.clone(),
            index: self.index.clone(),
            #[cfg(test)]
            hash_state: self.hash_state.clone(),
            index_dirty: self.index_dirty,
            identities: self.identities.fork(),
        }
    }
}

impl TokenStore {
    /// Creates a token store containing the canonical empty list.
    #[must_use]
    pub(crate) fn new() -> Self {
        let mut store = Self {
            arena: Vec::new(),
            spans: vec![(0, 0)],
            semantic_ids: vec![TokenSemanticIdBuilder::new().finish()],
            index: TokenIndex::default(),
            #[cfg(test)]
            hash_state: RandomState::new(),
            index_dirty: false,
            identities: IdentityAllocator::new(1),
        };
        store
            .index
            .entry(store.semantic_id(TokenStore::empty_id()))
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
        TokenListId::EMPTY
    }

    /// Interns `tokens`, returning a dense id for the live token-list content.
    #[cfg(test)]
    pub(crate) fn intern(&mut self, tokens: &[Token]) -> TokenListId {
        let hash = self.content_hash(tokens);
        self.intern_with_semantic_id(tokens, TokenSemanticId(hash))
    }

    /// Interns tokens using their aggregate-computed canonical semantic identity.
    pub(crate) fn intern_with_semantic_id(
        &mut self,
        tokens: &[Token],
        semantic_id: TokenSemanticId,
    ) -> TokenListId {
        #[cfg(feature = "node-stats")]
        let capacity_before = self.arena.capacity();
        #[cfg(feature = "node-stats")]
        let semantic_capacity_before = self.semantic_ids.capacity();
        if tokens.is_empty() {
            #[cfg(feature = "node-stats")]
            crate::measurement::record_token_intern(tokens.len(), true, 0, 0);
            return Self::empty_id();
        }

        if self.index_dirty {
            self.rebuild_index();
        }

        if let Some(candidates) = self.index.get(&semantic_id) {
            for &id in candidates {
                // Hash collisions are safe because the candidate span is
                // compared by content before the id is reused.
                if self.get(id) == tokens {
                    #[cfg(feature = "node-stats")]
                    crate::measurement::record_token_intern(tokens.len(), true, 0, 0);
                    return id;
                }
            }
        }

        let start = u32_len(self.arena.len(), "token arena exceeds u32 entries");
        let len = u32_len(tokens.len(), "token list exceeds u32 entries");
        let id = self.allocate_id();

        self.arena.extend_from_slice(tokens);
        self.spans.push((start, len));
        self.semantic_ids.push(semantic_id);
        self.index.entry(semantic_id).or_default().push(id);
        #[cfg(feature = "node-stats")]
        crate::measurement::record_token_intern(
            tokens.len(),
            false,
            self.arena.capacity().saturating_sub(capacity_before) * core::mem::size_of::<Token>(),
            self.semantic_ids
                .capacity()
                .saturating_sub(semantic_capacity_before)
                * core::mem::size_of::<TokenSemanticId>(),
        );
        id
    }

    /// Interns the semantic projection of an already-validated traced slice.
    ///
    /// The caller owns aggregate token/origin liveness validation. Keeping the
    /// projection borrowed avoids materializing a second token vector on both
    /// hash-cons hits and misses.
    #[cfg(test)]
    pub(crate) fn intern_traced(&mut self, traced: &[TracedTokenWord]) -> TokenListId {
        let hash = self.hash_state.hash_one(TracedTokenProjection(traced));
        self.intern_traced_with_semantic_id(traced, TokenSemanticId(hash))
    }

    /// Interns traced tokens using their aggregate-computed canonical semantic identity.
    pub(crate) fn intern_traced_with_semantic_id(
        &mut self,
        traced: &[TracedTokenWord],
        semantic_id: TokenSemanticId,
    ) -> TokenListId {
        #[cfg(feature = "node-stats")]
        let capacity_before = self.arena.capacity();
        #[cfg(feature = "node-stats")]
        let semantic_capacity_before = self.semantic_ids.capacity();
        if traced.is_empty() {
            #[cfg(feature = "node-stats")]
            crate::measurement::record_token_intern(0, true, 0, 0);
            return Self::empty_id();
        }

        if self.index_dirty {
            self.rebuild_index();
        }

        if let Some(candidates) = self.index.get(&semantic_id) {
            for &id in candidates {
                let candidate = self.get(id);
                if candidate.len() == traced.len()
                    && candidate
                        .iter()
                        .zip(traced)
                        .all(|(&token, &word)| word.token() == Some(token))
                {
                    #[cfg(feature = "node-stats")]
                    crate::measurement::record_token_intern(traced.len(), true, 0, 0);
                    return id;
                }
            }
        }

        let start = u32_len(self.arena.len(), "token arena exceeds u32 entries");
        let len = u32_len(traced.len(), "token list exceeds u32 entries");
        let id = self.allocate_id();

        self.arena.reserve(traced.len());
        self.spans.reserve(1);
        self.arena.extend(traced.iter().map(|word| {
            word.token()
                .expect("validated traced token became invalid during interning")
        }));
        self.spans.push((start, len));
        self.semantic_ids.push(semantic_id);
        self.index.entry(semantic_id).or_default().push(id);
        #[cfg(feature = "node-stats")]
        crate::measurement::record_token_intern(
            traced.len(),
            false,
            self.arena.capacity().saturating_sub(capacity_before) * core::mem::size_of::<Token>(),
            self.semantic_ids
                .capacity()
                .saturating_sub(semantic_capacity_before)
                * core::mem::size_of::<TokenSemanticId>(),
        );
        id
    }

    /// Reads a live frozen token list.
    #[must_use]
    pub(crate) fn get(&self, id: TokenListId) -> &[Token] {
        assert!(
            self.identities.contains(id.identity()),
            "token list id is not live"
        );
        let index = id.raw() as usize;
        assert!(index < self.spans.len(), "token list id is not live");
        let (start, len) = self.spans[index];
        let start = start as usize;
        let end = start + len as usize;
        assert!(end <= self.arena.len(), "token-list span exceeds arena");
        &self.arena[start..end]
    }

    /// Returns the canonical semantic identity stored with a live token list.
    pub(crate) fn semantic_id(&self, id: TokenListId) -> TokenSemanticId {
        assert!(
            self.identities.contains(id.identity()),
            "token list id is not live"
        );
        self.semantic_ids[id.raw() as usize]
    }

    /// Returns whether `id` names a currently-live token-list slot.
    #[must_use]
    pub(crate) fn contains(&self, id: TokenListId) -> bool {
        self.identities.contains(id.identity())
    }

    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn resolve_stored(&self, id: TokenListId) -> Option<TokenListId> {
        if self.contains(id) {
            return Some(id);
        }
        if !id.is_stored() {
            return None;
        }
        self.identities
            .identity_at(id.raw())
            .map(TokenListId::from_identity)
    }

    /// Takes a rollback watermark for aggregate snapshots.
    #[must_use]
    pub(crate) fn watermark(&self) -> TokenStoreMark {
        TokenStoreMark {
            spans: u32_len(self.spans.len(), "token-list spans exceed u32 entries"),
            tokens: u32_len(self.arena.len(), "token arena exceeds u32 entries"),
            identities: self.identities.watermark(),
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

        self.identities
            .rollback(mark.identities)
            .expect("token identity mark must name a retained ancestor");
        self.spans.truncate(spans);
        self.semantic_ids.truncate(spans);
        self.arena.truncate(tokens);
        self.index_dirty = true;
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for raw in 0..self.spans.len() {
            let id = self.id_at(u32_len(raw, "token-list spans exceed u32 entries"));
            let semantic_id = self.semantic_id(id);
            self.index.entry(semantic_id).or_default().push(id);
        }
        self.index_dirty = false;
    }

    #[cfg(test)]
    fn content_hash(&self, tokens: &[Token]) -> u64 {
        self.hash_state.hash_one(tokens)
    }

    fn allocate_id(&mut self) -> TokenListId {
        let identity = self
            .identities
            .allocate()
            .expect("token-list identity capacity exhausted");
        assert_eq!(identity.slot() as usize, self.spans.len());
        TokenListId::from_identity(identity)
    }

    fn id_at(&self, raw: u32) -> TokenListId {
        TokenListId::from_identity(
            self.identities
                .identity_at(raw)
                .expect("token-list slot is not live"),
        )
    }
}

#[cfg(test)]
struct TracedTokenProjection<'a>(&'a [TracedTokenWord]);

#[cfg(test)]
impl Hash for TracedTokenProjection<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        for word in self.0 {
            word.token()
                .expect("traced token projection contains an invalid semantic token")
                .hash(state);
        }
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
