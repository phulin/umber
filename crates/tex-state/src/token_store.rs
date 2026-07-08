//! Immutable hash-consed token-list storage.
//!
//! Token-list watermarks are crate-private so rollback can stay coupled to
//! the aggregate `Universe` boundary.

use crate::ids::TokenListId;
use crate::token::Token;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// A rollback watermark for the token store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TokenStoreMark {
    spans: u32,
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
    index: HashMap<u64, Vec<TokenListId>>,
    index_dirty: bool,
}

impl TokenStore {
    /// Creates a token store containing the canonical empty list.
    #[must_use]
    pub(crate) fn new() -> Self {
        let mut store = Self {
            arena: Vec::new(),
            spans: vec![(0, 0)],
            index: HashMap::new(),
            index_dirty: false,
        };
        store
            .index
            .entry(content_hash(&[]))
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

        let hash = content_hash(tokens);
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
            let hash = content_hash(self.get(id));
            self.index.entry(hash).or_default().push(id);
        }
        self.index_dirty = false;
    }
}

fn content_hash(tokens: &[Token]) -> u64 {
    let mut hasher = DefaultHasher::new();
    // PERF: revisit hasher (fastpaths epic).
    tokens.hash(&mut hasher);
    hasher.finish()
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{TokenListBuilder, TokenStore, TokenStoreMark};
    use crate::ids::TokenListId;
    use crate::interner::Symbol;
    use crate::token::{Catcode, Token};
    use proptest::prelude::*;
    use std::collections::HashMap;

    #[test]
    fn empty_list_is_canonical_and_allocates_no_tokens() {
        let mut store = TokenStore::new();

        let first = store.intern(&[]);
        let mut builder = TokenListBuilder::new();
        let second = builder.finish(&mut store);

        assert_eq!(first, TokenStore::empty_id());
        assert_eq!(second, TokenStore::empty_id());
        assert_eq!(store.get(first), &[]);
        assert!(store.arena.is_empty());
        assert_eq!(store.spans, vec![(0, 0)]);
    }

    #[test]
    fn get_slice_round_trips_interned_tokens() {
        let mut store = TokenStore::new();
        let tokens = vec![
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            Token::Cs(Symbol::new(4)),
            Token::param(1),
        ];

        let id = store.intern(&tokens);

        assert_eq!(store.get(id), tokens.as_slice());
    }

    #[test]
    fn hash_consing_same_content_twice_returns_same_id() {
        let mut store = TokenStore::new();
        let tokens = [
            Token::Char {
                ch: 'x',
                cat: Catcode::Other,
            },
            Token::Cs(Symbol::new(9)),
        ];

        let first = store.intern(&tokens);
        let second = store.intern(&tokens);

        assert_eq!(first, second);
    }

    proptest! {
        #[test]
        fn ifx_as_id_compare_structurally_equal_lists_share_id(tokens in token_vec()) {
            let mut store = TokenStore::new();
            let mut left = TokenListBuilder::new();
            let mut right = TokenListBuilder::new();

            for &token in &tokens {
                left.push(token);
            }
            for &token in &tokens {
                right.push(token);
            }

            let left_id = left.finish(&mut store);
            let right_id = right.finish(&mut store);

            prop_assert_eq!(left_id, right_id);
        }

        #[test]
        fn structurally_different_lists_get_different_ids(
            (left, right) in (token_vec(), token_vec()).prop_filter(
                "lists must differ",
                |(left, right)| left != right,
            )
        ) {
            let mut store = TokenStore::new();

            let left_id = store.intern(&left);
            let right_id = store.intern(&right);

            prop_assert_ne!(left_id, right_id);
        }
    }

    #[test]
    fn truncate_then_reintern_reuses_dense_token_list_id() {
        let mut store = TokenStore::new();
        let kept = store.intern(&[char_token('k')]);
        let mark = store.watermark();
        let truncated = store.intern(&[char_token('t')]);
        assert_eq!(truncated.raw(), 2);

        store.truncate_to(mark);
        assert_eq!(store.get(kept), &[char_token('k')]);

        let reinserted = store.intern(&[char_token('t')]);
        assert_eq!(reinserted.raw(), truncated.raw());
        assert_eq!(store.get(reinserted), &[char_token('t')]);
    }

    #[test]
    #[should_panic(expected = "token list id is not live")]
    fn stale_token_list_panics_after_truncation() {
        let mut store = TokenStore::new();
        let mark = store.watermark();
        let stale = store.intern(&[char_token('x')]);

        store.truncate_to(mark);

        let _ = store.get(stale);
    }

    #[test]
    #[ignore = "No deterministic DefaultHasher collision fixture is known yet; intern compares candidate slices after hash lookup."]
    fn content_hash_collision_safety_needs_deterministic_fixture() {}

    #[derive(Clone, Debug)]
    enum Op {
        Intern(Vec<Token>),
        Build(Vec<Token>),
        Mark,
        TruncateToMark(usize),
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            token_vec().prop_map(Op::Intern),
            token_vec().prop_map(Op::Build),
            Just(Op::Mark),
            any::<usize>().prop_map(Op::TruncateToMark),
        ]
    }

    proptest! {
        #[test]
        fn arbitrary_build_intern_and_truncate_sequences_match_naive_model(
            ops in prop::collection::vec(op_strategy(), 0..256)
        ) {
            let mut store = TokenStore::new();
            let mut model: Vec<Vec<Token>> = vec![Vec::new()];
            let mut model_index: HashMap<Vec<Token>, usize> = HashMap::from([(Vec::new(), 0)]);
            let mut marks: Vec<(TokenStoreMark, usize)> = vec![(store.watermark(), model.len())];

            for op in ops {
                match op {
                    Op::Intern(tokens) => {
                        let id = store.intern(&tokens);
                        let expected = model_id(&mut model, &mut model_index, &tokens);
                        prop_assert_eq!(id.raw() as usize, expected);
                    }
                    Op::Build(tokens) => {
                        let mut builder = TokenListBuilder::new();
                        for token in &tokens {
                            builder.push(*token);
                        }
                        let id = builder.finish(&mut store);
                        let expected = model_id(&mut model, &mut model_index, &tokens);
                        prop_assert_eq!(id.raw() as usize, expected);
                        prop_assert!(builder.is_empty());
                    }
                    Op::Mark => {
                        marks.push((store.watermark(), model.len()));
                    }
                    Op::TruncateToMark(raw_index) => {
                        let index = raw_index % marks.len();
                        let (mark, model_len) = marks[index];
                        store.truncate_to(mark);
                        model.truncate(model_len);
                        model_index = rebuild_model_index(&model);
                        marks.retain(|&(_, len)| len <= model_len);
                    }
                }

                prop_assert_eq!(store.spans.len(), model.len());
                for (raw, expected) in model.iter().enumerate() {
                    let id = TokenListId::new(raw as u32);
                    prop_assert_eq!(store.get(id), expected.as_slice());
                    prop_assert_eq!(store.intern(expected).raw() as usize, raw);
                }
            }
        }
    }

    fn model_id(
        model: &mut Vec<Vec<Token>>,
        index: &mut HashMap<Vec<Token>, usize>,
        tokens: &[Token],
    ) -> usize {
        if let Some(&id) = index.get(tokens) {
            return id;
        }
        let id = model.len();
        let tokens = tokens.to_vec();
        model.push(tokens.clone());
        index.insert(tokens, id);
        id
    }

    fn rebuild_model_index(model: &[Vec<Token>]) -> HashMap<Vec<Token>, usize> {
        model
            .iter()
            .cloned()
            .enumerate()
            .map(|(id, tokens)| (tokens, id))
            .collect()
    }

    fn token_vec() -> impl Strategy<Value = Vec<Token>> {
        prop::collection::vec(token_strategy(), 0..24)
    }

    fn token_strategy() -> impl Strategy<Value = Token> {
        prop_oneof![
            (any::<char>(), catcode_strategy()).prop_map(|(ch, cat)| Token::Char { ch, cat }),
            (0_u32..64).prop_map(|raw| Token::Cs(Symbol::new(raw))),
            (1_u8..=9).prop_map(Token::Param),
        ]
    }

    fn catcode_strategy() -> impl Strategy<Value = Catcode> {
        (0_u8..=15).prop_map(|raw| match raw {
            0 => Catcode::Escape,
            1 => Catcode::BeginGroup,
            2 => Catcode::EndGroup,
            3 => Catcode::MathShift,
            4 => Catcode::AlignmentTab,
            5 => Catcode::EndLine,
            6 => Catcode::Parameter,
            7 => Catcode::Superscript,
            8 => Catcode::Subscript,
            9 => Catcode::Ignored,
            10 => Catcode::Space,
            11 => Catcode::Letter,
            12 => Catcode::Other,
            13 => Catcode::Active,
            14 => Catcode::Comment,
            15 => Catcode::Invalid,
            _ => unreachable!("strategy bounds catcodes to 0..=15"),
        })
    }

    fn char_token(ch: char) -> Token {
        Token::Char {
            ch,
            cat: Catcode::Letter,
        }
    }
}
