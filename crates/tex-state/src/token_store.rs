//! Copy-only token-list identities and borrowed runtime-registry views.

use core::hash::{Hash, Hasher};

use crate::ContentHash;
use crate::ids::TokenListId;
use crate::state_hash::{StateHashFragment, StateHasher};
use crate::token::Token;

/// Versioned, allocation-independent identity of one immutable token sequence.
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
}

impl Hash for TokenSemanticId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.fingerprint);
    }
}

/// Current token semantic-identity scheme.
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

/// Reusable scanner scratch. It is not a live-state owner.
#[derive(Clone, Debug, Default)]
pub struct TokenListBuilder {
    buf: Vec<Token>,
}

impl TokenListBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, token: Token) {
        self.buf.push(token);
    }

    pub fn extend_from_slice(&mut self, tokens: &[Token]) {
        self.buf.extend_from_slice(tokens);
    }

    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }

    #[must_use]
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn as_slice(&self) -> &[Token] {
        &self.buf
    }
}

/// Copy-only semantic identity for a token list in the runtime value registry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TokenListRef {
    id: TokenListId,
}

#[cfg(any(test, feature = "testing"))]
pub fn testing_empty_token_list_ref() -> TokenListRef {
    TokenListRef::new(TokenListId::EMPTY)
}

impl TokenListRef {
    pub(crate) const fn new(id: TokenListId) -> Self {
        Self { id }
    }

    #[must_use]
    pub const fn id(&self) -> TokenListId {
        self.id
    }
}

/// Borrowed token-list payload admitted by the owning aggregate registry.
pub struct TokenListView<'a> {
    pub(crate) inner: crate::hot_core::arena::store::RuntimeTokenListView<'a>,
}

impl<'a> TokenListView<'a> {
    pub(crate) const fn new(
        inner: crate::hot_core::arena::store::RuntimeTokenListView<'a>,
    ) -> Self {
        Self { inner }
    }

    #[must_use]
    pub const fn id(&self) -> TokenListId {
        self.inner.coordinate().id()
    }

    #[must_use]
    pub const fn tokens(&self) -> &'a [Token] {
        self.inner.tokens()
    }

    pub(crate) const fn semantic_id(&self) -> TokenSemanticId {
        self.inner.semantic_id()
    }

    pub(crate) fn traced_word(&self, index: usize) -> Option<crate::token::TracedTokenWord> {
        self.inner.traced_word(index)
    }
}

impl AsRef<[Token]> for TokenListView<'_> {
    fn as_ref(&self) -> &[Token] {
        self.tokens()
    }
}

impl core::ops::Deref for TokenListView<'_> {
    type Target = [Token];

    fn deref(&self) -> &Self::Target {
        self.tokens()
    }
}

impl core::fmt::Debug for TokenListView<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_list().entries(self.tokens()).finish()
    }
}

impl PartialEq<&[Token]> for TokenListView<'_> {
    fn eq(&self, other: &&[Token]) -> bool {
        self.tokens() == *other
    }
}

impl PartialEq for TokenListView<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.tokens() == other.tokens()
    }
}

impl<const N: usize> PartialEq<[Token; N]> for TokenListView<'_> {
    fn eq(&self, other: &[Token; N]) -> bool {
        self.tokens() == other
    }
}

impl<const N: usize> PartialEq<&[Token; N]> for TokenListView<'_> {
    fn eq(&self, other: &&[Token; N]) -> bool {
        self.tokens() == *other
    }
}
