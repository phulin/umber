//! Immutable hash-consed token-list storage.
//!
//! Token-list watermarks are crate-private so rollback can stay coupled to
//! the aggregate `Universe` boundary.

use crate::ContentHash;
use crate::ids::TokenListId;
use crate::patch_domain::{
    PatchAllocationDomain, PatchHandle, PatchRoot, PatchRootAnchor, PatchRootLease,
};
#[cfg(any(test, feature = "testing"))]
use crate::reachable_value::LookupWork;
use crate::reachable_value::{ReachableValuePool, ReachableValueRef};
use crate::state_hash::{StateHashFragment, StateHasher};
use crate::token::{Token, TracedTokenWord};
#[cfg(test)]
use ahash::RandomState;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) enum FrozenTokenLookup {
    Legacy(crate::frozen_lookup::FrozenLookup),
    Direct(crate::frozen_lookup::DirectFrozenLookup),
}

/// Versioned, allocation-independent identity of one immutable token sequence.
///
/// Control sequences contribute their namespace and spelling through the
/// interner's semantic atom; compact runtime symbol keys never participate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TokenSemanticId {
    fingerprint: u64,
    identity: ContentHash,
}

impl TokenSemanticId {
    #[must_use]
    pub(crate) const fn value(self) -> u64 {
        self.fingerprint
    }

    #[must_use]
    pub(crate) const fn fragment(self) -> StateHashFragment {
        StateHashFragment::from_parts(self.fingerprint, self.identity)
    }

    pub(crate) fn apply(self, hasher: &mut StateHasher) {
        hasher.u64(self.fingerprint);
        hasher.semantic_identity(self.identity);
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing(fingerprint: u64) -> Self {
        Self {
            fingerprint,
            identity: crate::state_hash::semantic_identity_bytes(
                b"umber-testing-token-id",
                &fingerprint.to_le_bytes(),
            ),
        }
    }
}

impl Hash for TokenSemanticId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.fingerprint);
    }
}

/// Current token semantic-identity scheme. Changing token tags, symbol-atom
/// semantics, or the hash framing requires a new version and checkpoint-hash
/// migration notes.
pub(crate) const TOKEN_SEMANTIC_ID_VERSION: u8 = 2;
const TOKEN_STREAM_V2_DOMAIN: u64 = 0x746f_6b32_5f73_7472;
const TOKEN_ID_V2_DOMAIN: u64 = 0x746f_6b32_5f69_6465;

pub(crate) struct TokenSemanticIdBuilder {
    stream: StateHasher,
    len: usize,
}

impl TokenSemanticIdBuilder {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            stream: StateHasher::new(TOKEN_STREAM_V2_DOMAIN),
            len: 0,
        }
    }

    pub(crate) fn push(&mut self, token: Token, symbol_atom: Option<(u64, ContentHash)>) {
        match token {
            Token::Char { ch, cat } => {
                self.stream.tag(0);
                self.stream.u32(ch as u32);
                self.stream.u8(cat as u8);
            }
            Token::Cs(_) => {
                self.stream.tag(1);
                let (fingerprint, identity) =
                    symbol_atom.expect("control-sequence token requires semantic atom");
                self.stream.u64(fingerprint);
                self.stream.semantic_identity(identity);
            }
            Token::Param(slot) => {
                self.stream.tag(2);
                self.stream.u8(slot);
            }
            Token::Frozen(crate::token::FrozenToken::END_TEMPLATE) => self.stream.tag(3),
            Token::Frozen(crate::token::FrozenToken::END_V) => self.stream.tag(4),
            Token::Frozen(crate::token::FrozenToken::EXPANDED_TEXT_BOUNDARY) => self.stream.tag(6),
            Token::Frozen(crate::token::FrozenToken::RELAX) => self.stream.tag(7),
            Token::Frozen(crate::token::FrozenToken::UNDEFINED_CONTROL_SEQUENCE) => {
                self.stream.tag(8)
            }
            Token::Frozen(frozen) => {
                self.stream.tag(5);
                self.stream.u16(
                    frozen
                        .primitive_index()
                        .expect("non-sentinel frozen token must identify a primitive"),
                );
            }
        }
        self.len += 1;
    }

    #[must_use]
    pub(crate) fn finish(self) -> TokenSemanticId {
        let mut hasher = StateHasher::new(TOKEN_ID_V2_DOMAIN);
        hasher.u8(TOKEN_SEMANTIC_ID_VERSION);
        hasher.usize(self.len);
        self.stream.finish_fragment().apply(&mut hasher);
        let fragment = hasher.finish_fragment();
        TokenSemanticId {
            fingerprint: fragment.fingerprint(),
            identity: fragment.identity(),
        }
    }
}

/// A rollback watermark for the token store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TokenStoreMark {
    arena_slots: u32,
    arena_allocations: u32,
    packed_allocations: u32,
    patch_allocations: u32,
    #[cfg(any(test, feature = "testing"))]
    testing_detached_roots: u32,
}

#[derive(Debug)]
struct PackedTokenPair {
    ids: [TokenListId; 2],
    liveness: [Arc<()>; 2],
    semantic_ids: [TokenSemanticId; 2],
    parameter_len: u32,
    tokens: Box<[Token]>,
}

#[derive(Clone, Debug)]
struct PackedTokenListRef {
    pair: Arc<PackedTokenPair>,
    _liveness: Option<Arc<()>>,
    index: u8,
}

impl PackedTokenListRef {
    fn rooted(&self) -> Self {
        Self {
            pair: Arc::clone(&self.pair),
            _liveness: Some(Arc::clone(&self.pair.liveness[self.index as usize])),
            index: self.index,
        }
    }

    fn id(&self) -> TokenListId {
        self.pair.ids[self.index as usize]
    }

    fn semantic_id(&self) -> TokenSemanticId {
        self.pair.semantic_ids[self.index as usize]
    }

    fn tokens(&self) -> &[Token] {
        let split = self.pair.parameter_len as usize;
        if self.index == 0 {
            &self.pair.tokens[..split]
        } else {
            &self.pair.tokens[split..]
        }
    }
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
    #[cfg_attr(not(any(test, feature = "testing")), allow(dead_code))]
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

#[derive(Debug, Eq, PartialEq)]
struct TokenListValue {
    tokens: Box<[Token]>,
    semantic_id: TokenSemanticId,
}

impl TokenListValue {
    fn logical_bytes(&self) -> usize {
        core::mem::size_of::<Self>().saturating_add(
            self.tokens
                .len()
                .saturating_mul(core::mem::size_of::<Token>()),
        )
    }
}

/// One strong exact-content owner paired with its timeline-local coordinate.
#[derive(Clone, Debug)]
pub struct TokenListRef {
    value: Option<ReachableValueRef<TokenListValue>>,
    packed: Option<PackedTokenListRef>,
    patch_root: Option<PatchRootLease>,
}

#[cfg(any(test, feature = "testing"))]
pub fn testing_empty_token_list_ref() -> TokenListRef {
    TokenStore::new()
        .owner(TokenListId::EMPTY)
        .expect("test token store owns the canonical empty list")
}

impl TokenListRef {
    /// Returns the compact physical coordinate carried beside this owner.
    #[must_use]
    pub fn id(&self) -> TokenListId {
        self.packed.as_ref().map_or_else(
            || TokenListId::from_identity(self.exact_value().identity()),
            PackedTokenListRef::id,
        )
    }

    /// Borrows the immutable semantic token sequence.
    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        self.packed
            .as_ref()
            .map_or_else(|| self.exact_value().value().tokens.as_ref(), PackedTokenListRef::tokens)
    }

    pub(crate) fn semantic_id(&self) -> TokenSemanticId {
        self.packed.as_ref().map_or_else(
            || self.exact_value().value().semantic_id,
            PackedTokenListRef::semantic_id,
        )
    }

    fn shared(&self) -> Arc<TokenListValue> {
        self.exact_value().shared()
    }

    fn exact_value(&self) -> &ReachableValueRef<TokenListValue> {
        self.value
            .as_ref()
            .expect("arena-backed token list has no exact value")
    }

    #[cfg(test)]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared(), &other.shared())
    }

    #[cfg(test)]
    pub(crate) fn strong_count(&self) -> usize {
        self.exact_value().strong_count()
    }
}

impl std::ops::Deref for TokenListRef {
    type Target = [Token];

    fn deref(&self) -> &Self::Target {
        self.tokens()
    }
}

impl PartialEq<&[Token]> for TokenListRef {
    fn eq(&self, other: &&[Token]) -> bool {
        self.tokens() == *other
    }
}

impl<const N: usize> PartialEq<[Token; N]> for TokenListRef {
    fn eq(&self, other: &[Token; N]) -> bool {
        self.tokens() == other
    }
}

impl<const N: usize> PartialEq<&[Token; N]> for TokenListRef {
    fn eq(&self, other: &&[Token; N]) -> bool {
        self.tokens() == *other
    }
}

impl PartialEq<Vec<Token>> for TokenListRef {
    fn eq(&self, other: &Vec<Token>) -> bool {
        self.tokens() == other
    }
}

impl PartialEq<&Vec<Token>> for TokenListRef {
    fn eq(&self, other: &&Vec<Token>) -> bool {
        self.tokens() == other.as_slice()
    }
}

impl PartialEq for TokenListRef {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for TokenListRef {}

impl Hash for TokenListRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

/// Reachability-owned immutable token values.
#[derive(Debug)]
pub struct TokenStore {
    pool: ReachableValuePool<TokenSemanticId, TokenListValue>,
    frozen_roots: Arc<[TokenListRef]>,
    frozen_lookup: FrozenTokenLookup,
    frozen_len: u32,
    patch_handles: HashMap<TokenListId, PatchHandle<TokenListValue>>,
    patch_root_leases: HashMap<TokenListId, PatchRootAnchor>,
    patch_order: Vec<TokenListId>,
    packed_locations: Vec<Option<PackedTokenListRef>>,
    packed_allocations: Vec<[Option<TokenListId>; 2]>,
    /// Explicit detached owners used only by legacy test construction APIs.
    /// Production interning returns `TokenListRef` and never enters this row.
    #[cfg(any(test, feature = "testing"))]
    testing_detached_roots: Vec<TokenListRef>,
    #[cfg(test)]
    hash_state: RandomState,
}

impl Clone for TokenStore {
    fn clone(&self) -> Self {
        debug_assert!(
            self.patch_handles.is_empty(),
            "private token allocations cannot cross a generation fork"
        );
        Self {
            pool: self.pool.clone(),
            frozen_roots: Arc::clone(&self.frozen_roots),
            frozen_lookup: self.frozen_lookup.clone(),
            frozen_len: self.frozen_len,
            patch_handles: HashMap::new(),
            patch_root_leases: HashMap::new(),
            patch_order: Vec::new(),
            packed_locations: self.packed_locations.clone(),
            packed_allocations: Vec::new(),
            #[cfg(any(test, feature = "testing"))]
            testing_detached_roots: self.testing_detached_roots.clone(),
            #[cfg(test)]
            hash_state: self.hash_state.clone(),
        }
    }
}

impl TokenStore {
    #[must_use]
    pub(crate) fn requires_legacy_frozen_key(&self) -> bool {
        matches!(self.frozen_lookup, FrozenTokenLookup::Legacy(_))
    }

    #[must_use]
    pub(crate) const fn has_frozen_lists(&self) -> bool {
        self.frozen_len != 0
    }

    /// Creates a token store containing the immortal canonical empty list.
    #[must_use]
    pub(crate) fn new() -> Self {
        let (pool, roots) = ReachableValuePool::from_fixed_values(
            vec![TokenListValue {
                tokens: Box::new([]),
                semantic_id: TokenSemanticIdBuilder::new().finish(),
            }],
            1,
        );
        Self {
            pool,
            frozen_roots: Arc::from(
                roots
                    .into_iter()
                    .map(|value| TokenListRef {
                        value: Some(value),
                        packed: None,
                        patch_root: None,
                    })
                    .collect::<Vec<_>>(),
            ),
            frozen_lookup: FrozenTokenLookup::Direct(
                crate::frozen_lookup::DirectFrozenLookup::empty(),
            ),
            frozen_len: 0,
            patch_handles: HashMap::new(),
            patch_root_leases: HashMap::new(),
            patch_order: Vec::new(),
            packed_locations: Vec::new(),
            packed_allocations: Vec::new(),
            #[cfg(any(test, feature = "testing"))]
            testing_detached_roots: Vec::new(),
            #[cfg(test)]
            hash_state: RandomState::new(),
        }
    }

    /// Installs a validated frozen token arena as one explicit immutable base.
    pub(crate) fn from_frozen(
        arena: Vec<Token>,
        spans: Vec<(u32, u32)>,
        semantic_ids: Vec<TokenSemanticId>,
        frozen_lookup: FrozenTokenLookup,
    ) -> Result<Self, &'static str> {
        if spans.len() != semantic_ids.len() {
            return Err("frozen token column length mismatch");
        }
        if spans.first().copied() != Some((0, 0)) || semantic_ids.is_empty() {
            return Err("missing frozen canonical empty token list");
        }
        let count = u32::try_from(spans.len()).map_err(|_| "frozen token-list capacity")?;
        let mut values = Vec::with_capacity(spans.len());
        let mut cursor = 0_u32;
        for ((start, len), semantic_id) in spans.into_iter().zip(semantic_ids) {
            if start != cursor {
                return Err("non-canonical frozen token-list span");
            }
            cursor = start
                .checked_add(len)
                .ok_or("frozen token-list span overflow")?;
            let tokens = arena
                .get(start as usize..cursor as usize)
                .ok_or("frozen token-list span out of bounds")?;
            values.push(TokenListValue {
                tokens: tokens.into(),
                semantic_id,
            });
        }
        if cursor as usize != arena.len() {
            return Err("unused frozen token words");
        }
        let (pool, roots) = ReachableValuePool::from_fixed_values(values, 1);
        let roots = roots
            .into_iter()
                .map(|value| TokenListRef {
                    value: Some(value),
                    packed: None,
                    patch_root: None,
            })
            .collect::<Vec<_>>();
        Ok(Self {
            pool,
            frozen_roots: Arc::from(roots),
            frozen_lookup,
            frozen_len: count,
            patch_handles: HashMap::new(),
            patch_root_leases: HashMap::new(),
            patch_order: Vec::new(),
            packed_locations: vec![None; count as usize],
            packed_allocations: Vec::new(),
            #[cfg(any(test, feature = "testing"))]
            testing_detached_roots: Vec::new(),
            #[cfg(test)]
            hash_state: RandomState::new(),
        })
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

    /// Interns `tokens` for legacy tests and publishes an explicit detached
    /// test owner. Production paths use `intern_owned_with_semantic_identity`.
    #[cfg(test)]
    pub(crate) fn intern(&mut self, tokens: &[Token]) -> TokenListId {
        let hash = self.content_hash(tokens);
        self.testing_intern_with_semantic_id(tokens, TokenSemanticId::testing(hash), 0, None, None)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_intern_with_semantic_id(
        &mut self,
        tokens: &[Token],
        semantic_id: TokenSemanticId,
        frozen_hash: u64,
        legacy_key: Option<&[u8]>,
        domain: Option<&mut PatchAllocationDomain>,
    ) -> TokenListId {
        let root = self.intern_owned_with_semantic_id(
            tokens,
            semantic_id,
            frozen_hash,
            legacy_key,
            domain,
        );
        self.testing_detached_roots.push(root.clone());
        root.id()
    }

    /// Interns tokens and returns the strong exact-content owner directly.
    pub(crate) fn intern_owned_with_semantic_identity(
        &mut self,
        tokens: &[Token],
        semantic_id: TokenSemanticId,
        frozen_hash: u64,
        legacy_key: Option<&[u8]>,
        domain: Option<&mut PatchAllocationDomain>,
    ) -> TokenListRef {
        self.intern_owned_with_semantic_id(tokens, semantic_id, frozen_hash, legacy_key, domain)
    }

    fn intern_owned_with_semantic_id(
        &mut self,
        tokens: &[Token],
        semantic_id: TokenSemanticId,
        frozen_hash: u64,
        legacy_key: Option<&[u8]>,
        domain: Option<&mut PatchAllocationDomain>,
    ) -> TokenListRef {
        if tokens.is_empty() {
            #[cfg(feature = "profiling")]
            crate::measurement::record_token_intern(0, true, 0, 0);
            return self.frozen_roots[0].clone();
        }
        if let Some(root) = self.find_frozen(tokens, frozen_hash, legacy_key) {
            #[cfg(feature = "profiling")]
            crate::measurement::record_token_intern(tokens.len(), true, 0, 0);
            return root;
        }
        if let Some(value) = self.pool.find_exact(&semantic_id, |candidate| {
            candidate.tokens.as_ref() == tokens
        }) {
            #[cfg(feature = "profiling")]
            crate::measurement::record_token_intern(tokens.len(), true, 0, 0);
            return TokenListRef {
                value: Some(value),
                packed: None,
                patch_root: None,
            };
        }

        let value = self.pool.insert_new(
            semantic_id,
            TokenListValue {
                tokens: tokens.into(),
                semantic_id,
            },
        );
        let mut root = TokenListRef {
            value: Some(value),
            packed: None,
            patch_root: None,
        };
        self.attach_patch_allocation(&mut root, domain);
        #[cfg(feature = "profiling")]
        crate::measurement::record_token_intern(
            tokens.len(),
            false,
            root.tokens()
                .len()
                .saturating_mul(core::mem::size_of::<Token>()),
            core::mem::size_of::<TokenSemanticId>(),
        );
        root
    }

    /// Interns traced tokens using their aggregate-computed canonical identity.
    #[cfg(test)]
    pub(crate) fn intern_traced_with_semantic_id(
        &mut self,
        traced: &[TracedTokenWord],
        semantic_id: TokenSemanticId,
        frozen_hash: u64,
        legacy_key: Option<&[u8]>,
        domain: Option<&mut PatchAllocationDomain>,
    ) -> TokenListId {
        let tokens = traced
            .iter()
            .map(|word| {
                word.token()
                    .expect("validated traced token became invalid during interning")
            })
            .collect::<Vec<_>>();
        self.testing_intern_with_semantic_id(&tokens, semantic_id, frozen_hash, legacy_key, domain)
    }

    /// Publishes one ordinary runtime token-list occurrence without consulting
    /// or extending the cold exact-content index.
    ///
    /// Empty lists still use the immortal canonical root. Nonempty runtime
    /// lists deliberately receive fresh physical coordinates; semantic
    /// equality is carried by `semantic_id`, while format and detached import
    /// continue to use the collision-checked interning entry point above.
    pub(crate) fn allocate_traced_owned_with_semantic_id(
        &mut self,
        traced: &[TracedTokenWord],
        semantic_id: TokenSemanticId,
        domain: Option<&mut PatchAllocationDomain>,
    ) -> TokenListRef {
        if traced.is_empty() {
            #[cfg(feature = "profiling")]
            crate::measurement::record_token_intern(0, true, 0, 0);
            return self.frozen_roots[0].clone();
        }
        let tokens = traced
            .iter()
            .map(|word| {
                word.token()
                    .expect("validated traced token became invalid during allocation")
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let value = self.pool.insert_unindexed(TokenListValue {
            tokens,
            semantic_id,
        });
        let mut root = TokenListRef {
            value: Some(value),
            packed: None,
            patch_root: None,
        };
        self.attach_patch_allocation(&mut root, domain);
        #[cfg(feature = "profiling")]
        crate::measurement::record_token_intern(
            traced.len(),
            false,
            traced.len().saturating_mul(core::mem::size_of::<Token>()),
            core::mem::size_of::<TokenSemanticId>(),
        );
        root
    }

    /// Publishes the parameter and replacement text of one ordinary macro as
    /// a single immutable arena payload. The two token coordinates share one
    /// allocation and do not enter the weak value graph or exact index.
    pub(crate) fn allocate_traced_pair(
        &mut self,
        parameter: &[TracedTokenWord],
        replacement: &[TracedTokenWord],
        semantic_ids: [TokenSemanticId; 2],
    ) -> (TokenListRef, TokenListRef) {
        let mut allocated = [None, None];
        let ids = core::array::from_fn(|index| {
            let words = [parameter, replacement][index];
            if words.is_empty() {
                TokenListId::EMPTY
            } else {
                let id = TokenListId::from_identity(self.pool.reserve_external());
                allocated[index] = Some(id);
                id
            }
        });
        let tokens = parameter
            .iter()
            .chain(replacement)
            .map(|word| {
                word.token()
                    .expect("validated traced token became invalid during arena allocation")
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let pair = Arc::new(PackedTokenPair {
            ids,
            liveness: std::array::from_fn(|_| Arc::new(())),
            semantic_ids,
            parameter_len: u32::try_from(parameter.len())
                .expect("macro parameter text exceeds u32"),
            tokens,
        });
        let roots = [0_u8, 1_u8].map(|index| {
            if ids[index as usize] == TokenListId::EMPTY {
                self.frozen_roots[0].clone()
            } else {
                let packed = PackedTokenListRef {
                    pair: Arc::clone(&pair),
                    _liveness: None,
                    index,
                };
                let slot = packed.id().raw() as usize;
                if self.packed_locations.len() <= slot {
                    self.packed_locations.resize(slot + 1, None);
                }
                assert!(self.packed_locations[slot].is_none());
                self.packed_locations[slot] = Some(packed.clone());
                TokenListRef {
                    value: None,
                    packed: Some(packed.rooted()),
                    patch_root: None,
                }
            }
        });
        self.packed_allocations.push(allocated);
        (roots[0].clone(), roots[1].clone())
    }

    #[cfg(test)]
    pub(crate) fn intern_traced(&mut self, traced: &[TracedTokenWord]) -> TokenListId {
        let hash = self.hash_state.hash_one(TracedTokenProjection(traced));
        self.intern_traced_with_semantic_id(traced, TokenSemanticId::testing(hash), 0, None, None)
    }

    fn find_frozen(
        &self,
        tokens: &[Token],
        frozen_hash: u64,
        legacy_key: Option<&[u8]>,
    ) -> Option<TokenListRef> {
        let matches = |raw| {
            self.frozen_roots
                .get(raw as usize)
                .filter(|root| root.tokens() == tokens)
                .cloned()
        };
        match &self.frozen_lookup {
            FrozenTokenLookup::Legacy(lookup) => {
                legacy_key.and_then(|key| lookup.get(key)).and_then(matches)
            }
            FrozenTokenLookup::Direct(lookup) => lookup.candidates(frozen_hash).find_map(matches),
        }
    }

    fn attach_patch_allocation(
        &mut self,
        root: &mut TokenListRef,
        domain: Option<&mut PatchAllocationDomain>,
    ) {
        let Some(domain) = domain else {
            return;
        };
        let handle = domain
            .allocate_shared(root.shared(), root.exact_value().value().logical_bytes())
            .expect("private token allocation belongs to the active operation");
        assert!(
            self.patch_handles.insert(root.id(), handle).is_none(),
            "new token value already has patch allocation metadata"
        );
        let lease = domain
            .install_root_lease(&self.patch_handles[&root.id()])
            .expect("new private token root belongs to the active domain");
        assert!(
            self.patch_root_leases
                .insert(root.id(), lease.anchor())
                .is_none()
        );
        root.patch_root = Some(lease);
        self.patch_order.push(root.id());
    }

    /// Reads a value while a typed owner keeps its weak slot live.
    #[must_use]
    pub(crate) fn get(&self, id: TokenListId) -> TokenListRef {
        self.resolved_owner(id)
            .expect("token list id has no live typed owner")
    }

    /// Returns the serialization projection at one compact slot.
    ///
    /// Dead slots project as the canonical empty list and are removed by the
    /// format closure pass. Live slots upgrade only because a typed owner
    /// already exists.
    #[must_use]
    pub(crate) fn stored_slot_tokens(&self, raw: u32) -> TokenListRef {
        if let Some(root) = self.frozen_roots.get(raw as usize) {
            return root.clone();
        }
        if let Some(root) = self
            .packed_locations
            .get(raw as usize)
            .and_then(Option::as_ref)
        {
            return TokenListRef {
                value: None,
                packed: Some(root.rooted()),
                patch_root: None,
            };
        }
        self.pool.resolve_slot(raw).map_or_else(
            || self.frozen_roots[0].clone(),
            |value| TokenListRef {
                value: Some(value),
                packed: None,
                patch_root: None,
            },
        )
    }

    /// Clones a strong owner for one currently-live coordinate.
    pub(crate) fn owner(&self, id: TokenListId) -> Option<TokenListRef> {
        if let Some(packed) = self
            .packed_locations
            .get(id.raw() as usize)
            .and_then(Option::as_ref)
            .filter(|packed| packed.id() == id)
        {
            return Some(TokenListRef {
                value: None,
                packed: Some(packed.rooted()),
                patch_root: None,
            });
        }
        self.frozen_root(id).cloned().or_else(|| {
            self.pool.resolve(id.identity()).map(|value| TokenListRef {
                value: Some(value),
                packed: None,
                patch_root: self
                    .patch_root_leases
                    .get(&id)
                    .map(PatchRootAnchor::lease),
            })
        })
    }

    /// Validates that an already-owned token list belongs to this timeline
    /// without reconstructing ownership through a weak slot.
    pub(crate) fn accepts_owner(&self, owner: &TokenListRef) -> bool {
        self.pool.contains_identity(owner.id().identity())
    }

    /// Clones the owner named either by a live identity or a compact stored
    /// coordinate. Stored words deliberately bypass the generation lookup:
    /// their reserved identity is only a format/Env projection of the slot.
    pub(crate) fn resolved_owner(&self, id: TokenListId) -> Option<TokenListRef> {
        if !id.is_stored() {
            return self.owner(id);
        }
        self.frozen_roots
            .get(id.raw() as usize)
            .cloned()
            .or_else(|| {
                self.packed_locations
                    .get(id.raw() as usize)
                    .and_then(Option::as_ref)
                    .map(|packed| TokenListRef {
                        value: None,
                        packed: Some(packed.rooted()),
                        patch_root: None,
                    })
            })
            .or_else(|| {
                self.pool.resolve_slot(id.raw()).map(|value| {
                    let resolved = TokenListId::from_identity(value.identity());
                    TokenListRef {
                        value: Some(value),
                        packed: None,
                        patch_root: self
                            .patch_root_leases
                            .get(&resolved)
                            .map(PatchRootAnchor::lease),
                    }
                })
            })
    }

    fn frozen_root(&self, id: TokenListId) -> Option<&TokenListRef> {
        self.frozen_roots
            .get(id.raw() as usize)
            .filter(|root| root.id() == id)
    }

    /// Returns the canonical semantic identity stored with a live token list.
    pub(crate) fn semantic_id(&self, id: TokenListId) -> TokenSemanticId {
        self.resolved_owner(id)
            .expect("token list id is not live")
            .semantic_id()
    }

    /// Returns whether `id` names a currently-live token-list slot.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn contains(&self, id: TokenListId) -> bool {
        self.owner(id).is_some()
    }

    #[must_use]
    pub(crate) fn resolve_stored(&self, id: TokenListId) -> Option<TokenListId> {
        self.resolved_owner(id).map(|owner| owner.id())
    }

    /// Takes a rollback watermark over weak slots and private metadata.
    #[must_use]
    pub(crate) fn watermark(&self) -> TokenStoreMark {
        TokenStoreMark {
            arena_slots: u32_len(self.pool.slot_len(), "token-list slots exceed u32 entries"),
            arena_allocations: u32_len(
                self.pool.allocation_mark(),
                "token-list allocation events exceed u32 entries",
            ),
            packed_allocations: u32_len(
                self.packed_allocations.len(),
                "packed token allocations exceed u32 entries",
            ),
            patch_allocations: u32_len(
                self.patch_order.len(),
                "token-list patch allocations exceed u32 entries",
            ),
            #[cfg(any(test, feature = "testing"))]
            testing_detached_roots: u32_len(
                self.testing_detached_roots.len(),
                "testing detached token roots exceed u32",
            ),
        }
    }

    /// Retires packed token coordinates whose macro arena roots have already
    /// been explicitly retired at this mutation boundary.
    pub(crate) fn prepare_runtime_allocation(&mut self) {
        let mut allocation_index = 0;
        while allocation_index < self.packed_allocations.len() {
            let allocation = self.packed_allocations[allocation_index];
            let is_unrooted = allocation.iter().flatten().all(|id| {
                self.packed_locations
                    .get(id.raw() as usize)
                    .and_then(Option::as_ref)
                    .filter(|packed| packed.id() == *id)
                    .is_none_or(|packed| {
                        Arc::strong_count(&packed.pair.liveness[packed.index as usize]) == 1
                    })
            });
            if !is_unrooted {
                allocation_index += 1;
                continue;
            }
            let allocation = self.packed_allocations.swap_remove(allocation_index);
            for id in allocation.into_iter().flatten() {
                let slot = id.raw() as usize;
                let packed = self.packed_locations[slot]
                    .take()
                    .expect("unrooted packed token location is live");
                assert_eq!(packed.id(), id);
                self.pool.release_external(id.identity());
            }
        }
    }

    /// Restores private metadata; dead weak slots are reclaimed on next intern.
    pub(crate) fn truncate_to(&mut self, mark: TokenStoreMark) {
        #[cfg(any(test, feature = "testing"))]
        self.testing_detached_roots
            .truncate(mark.testing_detached_roots as usize);
        while self.patch_order.len() > mark.patch_allocations as usize {
            let id = self.patch_order.pop().expect("patch order is nonempty");
            assert!(self.patch_handles.remove(&id).is_some());
            assert!(self.patch_root_leases.remove(&id).is_some());
        }
        while self.packed_allocations.len() > mark.packed_allocations as usize {
            let allocation = self
                .packed_allocations
                .pop()
                .expect("packed token allocation journal is nonempty");
            for id in allocation.into_iter().rev().flatten() {
                let slot = id.raw() as usize;
                let packed = self.packed_locations[slot]
                    .take()
                    .expect("packed token location is live");
                assert_eq!(packed.id(), id);
                self.pool.release_external(id.identity());
            }
        }
        self.pool
            .rollback_to_allocation_mark(mark.arena_allocations as usize);
    }

    pub(crate) fn slot_len(&self) -> u32 {
        u32_len(self.pool.slot_len(), "token-list slots exceed u32 entries")
    }

    pub(crate) fn selected_patch_roots(&self, domain: &PatchAllocationDomain) -> Vec<PatchRoot> {
        self.patch_order
            .iter()
            .filter_map(|id| self.patch_handles.get(id))
            .filter_map(|handle| {
                domain
                    .root_if_typed(handle)
                    .expect("typed token root belongs to the private domain")
            })
            .collect()
    }

    pub(crate) fn patch_allocation_count(&self) -> usize {
        self.patch_handles.len()
    }

    pub(crate) fn clear_patch_allocations(&mut self) {
        self.patch_handles.clear();
        self.patch_root_leases.clear();
        self.patch_order.clear();
        self.pool.prioritize_reclamation_from(0);
    }

    pub(crate) fn retire_unrooted_region_values(&mut self) {
        self.prepare_runtime_allocation();
        self.pool.prioritize_reclamation_from(0);
    }

    #[cfg(test)]
    fn content_hash(&self, tokens: &[Token]) -> u64 {
        self.hash_state.hash_one(tokens)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_pool_shape(&self) -> (usize, usize, usize, usize, usize, usize) {
        self.pool.testing_shape()
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_live_totals(&self) -> (usize, usize) {
        self.pool.testing_live_totals(TokenListValue::logical_bytes)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_owned(
        &mut self,
        tokens: &[Token],
        semantic_id: TokenSemanticId,
        domain: Option<&mut PatchAllocationDomain>,
    ) -> TokenListRef {
        self.intern_owned_with_semantic_id(tokens, semantic_id, 0, None, domain)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_resolved_owner(
        &self,
        id: TokenListId,
    ) -> (Option<TokenListRef>, LookupWork) {
        let mut work = LookupWork {
            fixed_root_probes: 1,
            ..LookupWork::default()
        };
        if let Some(root) = self
            .frozen_roots
            .get(id.raw() as usize)
            .filter(|root| id.is_stored() || root.id() == id)
        {
            return (Some(root.clone()), work);
        }
        let (value, pool_work) = if id.is_stored() {
            self.pool.testing_resolve_slot(id.raw())
        } else {
            self.pool.testing_resolve(id.identity())
        };
        work.generation_checks += pool_work.generation_checks;
        work.slot_probes += pool_work.slot_probes;
        work.owner_clones += pool_work.owner_clones;
        let root = value.map(|value| {
            work.patch_lease_probes += 1;
            let resolved = TokenListId::from_identity(value.identity());
            TokenListRef {
                value: Some(value),
                packed: None,
                patch_root: self
                    .patch_root_leases
                    .get(&resolved)
                    .map(PatchRootAnchor::lease),
            }
        });
        (root, work)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_collision_lookup(
        &self,
        tokens: &[Token],
        semantic_id: TokenSemanticId,
    ) -> (Option<TokenListRef>, LookupWork) {
        let (value, work) = self.pool.testing_find_exact(&semantic_id, |candidate| {
            candidate.tokens.as_ref() == tokens
        });
        (
            value.map(|value| TokenListRef {
                value: Some(value),
                packed: None,
                patch_root: None,
            }),
            work,
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
