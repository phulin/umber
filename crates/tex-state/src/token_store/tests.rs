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
