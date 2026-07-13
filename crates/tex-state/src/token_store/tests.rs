use super::{TokenListBuilder, TokenSemanticId, TokenStore, TokenStoreMark};
use crate::ids::TokenListId;
use crate::interner::Symbol;
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use proptest::prelude::*;
use std::collections::HashMap;

#[test]
fn semantic_identity_is_one_word_per_token_list() {
    assert_eq!(core::mem::size_of::<TokenSemanticId>(), 8);
    let store = TokenStore::new();
    assert_eq!(store.semantic_ids.len(), store.spans.len());
}

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

#[test]
fn traced_projection_hashes_and_interns_like_owned_tokens() {
    let mut store = TokenStore::new();
    let tokens = [
        Token::Char {
            ch: '🦀',
            cat: Catcode::Other,
        },
        Token::Cs(Symbol::new(9)),
        Token::param(3),
        Token::frozen_end_template(),
        Token::frozen_endv(),
    ];
    let traced: Vec<_> = tokens
        .iter()
        .copied()
        .enumerate()
        .map(|(index, token)| TracedTokenWord::pack(token, OriginId::from_raw(index as u32)))
        .collect();

    assert_eq!(
        store.content_hash(&tokens),
        store
            .hash_state
            .hash_one(super::TracedTokenProjection(&traced))
    );
    let direct = store.intern_traced(&traced);
    assert_eq!(store.get(direct), tokens);
    assert_eq!(store.intern(&tokens), direct);
}

#[test]
fn clone_preserves_keyed_content_hash_state() {
    let mut original = TokenStore::new();
    let tokens = [char_token('x'), char_token('y')];
    let original_id = original.intern(&tokens);
    let mut cloned = original.clone();

    assert_eq!(original.content_hash(&tokens), cloned.content_hash(&tokens));
    assert_eq!(cloned.intern(&tokens), original_id);
}

#[test]
fn fork_preserves_inherited_ids_and_separates_new_allocations() {
    let mut parent = TokenStore::new();
    let inherited = parent.intern(&[char_token('i')]);
    let mut child = parent.clone();

    assert_eq!(child.get(inherited), &[char_token('i')]);

    let parent_only = parent.intern(&[char_token('p')]);
    let child_only = child.intern(&[char_token('c')]);
    assert_eq!(parent_only.raw(), child_only.raw());
    assert!(!child.contains(parent_only));
    assert!(!parent.contains(child_only));
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
    assert_ne!(reinserted, truncated);
    assert!(!store.contains(truncated));
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
fn same_hash_bucket_still_compares_token_list_content() {
    let mut store = TokenStore::new();
    let existing = [char_token('a')];
    let distinct = [char_token('b')];
    let existing_id = store.intern(&existing);
    let distinct_hash = store.content_hash(&distinct);

    store
        .index
        .entry(TokenSemanticId(distinct_hash))
        .or_default()
        .push(existing_id);

    let distinct_id = store.intern(&distinct);

    assert_ne!(distinct_id, existing_id);
    assert_eq!(store.get(existing_id), existing);
    assert_eq!(store.get(distinct_id), distinct);
}

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
                let id = store
                    .resolve_stored(TokenListId::new(raw as u32))
                    .expect("model slot should resolve to a live token-list identity");
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
