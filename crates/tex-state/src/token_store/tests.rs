use super::{FrozenTokenLookup, TokenListBuilder, TokenSemanticId, TokenStore, TokenStoreMark};
use crate::ids::TokenListId;
use crate::interner::Symbol;
use crate::patch_domain::PatchAllocationDomain;
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use ahash::AHashMap;
use proptest::prelude::*;

#[test]
fn semantic_identity_carries_a_strong_digest_per_token_list() {
    assert_eq!(core::mem::size_of::<TokenSemanticId>(), 40);
    let store = TokenStore::new();
    assert_eq!(
        store
            .owner(TokenListId::EMPTY)
            .expect("empty owner")
            .tokens(),
        []
    );
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
    assert_eq!(store.slot_len(), 1);
    assert_eq!(store.testing_pool_shape().0, 1);
}

#[test]
fn fresh_initex_store_skips_frozen_prefix_lookup() {
    // TeX82 §§202--203 create token lists directly in INITEX; there is no
    // preloaded format prefix to search. Large definitions must therefore not
    // pay a second whole-list hash pass for an empty frozen lookup.
    let fresh = TokenStore::new();
    assert!(!fresh.has_frozen_lists());

    let loaded = TokenStore::from_frozen(
        Vec::new(),
        vec![(0, 0)],
        vec![fresh.semantic_id(TokenListId::EMPTY)],
        FrozenTokenLookup::Direct(crate::frozen_lookup::DirectFrozenLookup::empty()),
    )
    .expect("canonical empty frozen prefix");
    assert!(loaded.has_frozen_lists());
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
    assert!(
        store
            .owner(first)
            .expect("first owner")
            .ptr_eq(&store.owner(second).expect("second owner"))
    );
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
fn repeated_retry_rollback_preserves_roots_and_bounded_weak_pool_capacity() {
    const FORMAT_LISTS: u32 = 2_048;
    const ATTEMPT_LISTS: u32 = 256;
    const RETRIES: usize = 32;

    let mut store = TokenStore::new();
    let token_list = |raw: u32| {
        [
            char_token(char::from_u32(0x1_0000 + raw).expect("test character is valid")),
            char_token('!'),
        ]
    };
    for raw in 0..FORMAT_LISTS {
        store.intern(&token_list(raw));
    }
    let retained = store.intern(&[char_token('k')]);
    let mark = store.watermark();

    // Warm the append-only arenas and index to the attempt high-water mark.
    // Later retries should reuse those capacities rather than accumulating
    // the format prefix or stale attempt identities again.
    for raw in FORMAT_LISTS..FORMAT_LISTS + ATTEMPT_LISTS {
        store.intern(&token_list(raw));
    }
    store.truncate_to(mark);
    let warmed_shape = store.testing_pool_shape();

    for _ in 0..RETRIES {
        let first_attempt = store.intern(&token_list(FORMAT_LISTS));
        for raw in FORMAT_LISTS + 1..FORMAT_LISTS + ATTEMPT_LISTS {
            store.intern(&token_list(raw));
        }

        store.truncate_to(mark);

        assert!(!store.contains(first_attempt));
        assert_eq!(store.get(retained), &[char_token('k')]);
        assert_eq!(store.intern(&[char_token('k')]), retained);
        let shape = store.testing_pool_shape();
        assert_eq!(shape.0, warmed_shape.0);
        assert_eq!(shape.1, warmed_shape.1);
        assert!(shape.2 <= 1_024);
        assert!(shape.3 <= warmed_shape.3.max(2_048));
        assert!(shape.4 <= 64);
    }
}

#[test]
#[should_panic(expected = "token list id has no live typed owner")]
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
    let collision = TokenSemanticId::testing(7);
    let existing_id = store.testing_intern_with_semantic_id(&existing, collision, 0, None, None);
    let distinct_id = store.testing_intern_with_semantic_id(&distinct, collision, 0, None, None);

    assert_ne!(distinct_id, existing_id);
    assert_eq!(store.get(existing_id), existing);
    assert_eq!(store.get(distinct_id), distinct);
}

#[test]
fn loaded_base_owns_exact_content_and_future_append_uses_dynamic_slots() {
    let frozen = [char_token('f')];
    let hashes = [0, 17];
    let lookup = crate::frozen_lookup::decode_direct(
        &crate::frozen_lookup::encode_direct(&hashes).expect("lookup encodes"),
        &hashes,
    )
    .expect("lookup decodes");
    let mut store = TokenStore::from_frozen(
        frozen.to_vec(),
        vec![(0, 0), (0, 1)],
        vec![TokenSemanticId::testing(0), TokenSemanticId::testing(1)],
        FrozenTokenLookup::Direct(lookup),
    )
    .expect("frozen base installs");

    let frozen_id = store.testing_intern_with_semantic_id(
        &frozen,
        TokenSemanticId::testing(99),
        17,
        None,
        None,
    );
    assert_eq!(frozen_id.raw(), 1);
    assert_eq!(store.get(frozen_id), frozen);

    let appended = store.intern(&[char_token('n')]);
    assert_eq!(appended.raw(), 2);
    assert_eq!(store.get(appended), &[char_token('n')]);
    assert_eq!(store.get(frozen_id), frozen);
}

#[test]
fn owned_dynamic_value_dies_and_reuses_its_slot_with_a_fresh_generation() {
    let mut store = TokenStore::new();
    let first = store.testing_owned(&[char_token('a')], TokenSemanticId::testing(1), None);
    let stale = first.id();
    assert_eq!(first.tokens(), &[char_token('a')]);
    drop(first);

    let second = store.testing_owned(&[char_token('b')], TokenSemanticId::testing(2), None);
    assert_eq!(second.id().raw(), stale.raw());
    assert_ne!(second.id(), stale);
    assert!(!store.contains(stale));
    assert_eq!(second.tokens(), &[char_token('b')]);
}

#[test]
fn private_token_operation_rollback_releases_only_its_exact_suffix() {
    let mut store = TokenStore::new();
    let mut domain = PatchAllocationDomain::new();

    let first_operation = domain.begin_operation().expect("operation begins");
    let retained = store.testing_owned(
        &[char_token('k')],
        TokenSemanticId::testing(1),
        Some(&mut domain),
    );
    domain
        .commit_operation(first_operation)
        .expect("operation commits");
    let mark = store.watermark();
    let retained_stats = domain.stats();

    let failed_operation = domain.begin_operation().expect("operation begins");
    let failed = store.testing_owned(
        &[char_token('x'), char_token('y')],
        TokenSemanticId::testing(2),
        Some(&mut domain),
    );
    let failed_id = failed.id();
    drop(failed);
    store.truncate_to(mark);
    domain
        .rollback_operation(failed_operation)
        .expect("operation rolls back");

    assert_eq!(domain.stats(), retained_stats);
    assert_eq!(retained.tokens(), &[char_token('k')]);
    assert!(!store.contains(failed_id));
}

#[test]
fn private_token_acceptance_selects_exact_roots_and_drops_unselected_payloads() {
    let mut store = TokenStore::new();
    let mut domain = PatchAllocationDomain::new();
    let operation = domain.begin_operation().expect("operation begins");
    let selected = store.testing_owned(
        &[char_token('s')],
        TokenSemanticId::testing(1),
        Some(&mut domain),
    );
    let unselected = store.testing_owned(
        &[char_token('u'), char_token('u')],
        TokenSemanticId::testing(2),
        Some(&mut domain),
    );
    let unselected_id = unselected.id();
    // Neither a typed clone that disappears before enumeration nor a raw
    // payload clone can manufacture acceptance authority after the real root
    // disappears.
    let temporary_typed_clone = store
        .owner(unselected_id)
        .expect("temporary typed clone resolves while its owner is live");
    let unrelated_transient = unselected.shared();
    domain
        .commit_operation(operation)
        .expect("operation commits");
    drop(unselected);
    drop(temporary_typed_clone);

    let selected_roots = store.selected_patch_roots(&domain);
    assert_eq!(selected_roots.len(), 1);
    let accepted = domain
        .accept(selected_roots)
        .expect("selected root transfers");
    store.clear_patch_allocations();
    assert_eq!(accepted.len(), 1);
    assert_eq!(
        accepted.logical_bytes(),
        core::mem::size_of::<super::TokenListValue>() + core::mem::size_of::<Token>()
    );
    assert_eq!(selected.tokens(), &[char_token('s')]);
    assert!(!store.contains(unselected_id));
    assert_eq!(
        unrelated_transient.tokens.as_ref(),
        [char_token('u'), char_token('u')]
    );
}

#[test]
fn ten_thousand_bounded_live_redefinitions_plateau_every_pool_dimension() {
    let mut store = TokenStore::new();
    let mut current = store.testing_owned(
        &[char_token(char::from_u32(0x1_0000).expect("valid scalar"))],
        TokenSemanticId::testing(0),
        None,
    );
    for value in 1..=10_000_u32 {
        current = store.testing_owned(
            &[char_token(
                char::from_u32(0x1_0000 + value).expect("valid scalar"),
            )],
            TokenSemanticId::testing(u64::from(value)),
            None,
        );
    }
    let (slots, slot_capacity, index_keys, index_capacity, bucket_capacity, free) =
        store.testing_pool_shape();
    let (live_objects, _) = store.testing_live_totals();

    assert_eq!(current.tokens().len(), 1);
    assert_eq!(live_objects, 3, "empty, current, and one retired-at-next-mutation value remain");
    assert!(slots <= 3, "region slots did not plateau: {slots}");
    assert!(
        slot_capacity <= 4,
        "slot capacity did not plateau: {slot_capacity}"
    );
    assert!(index_keys <= 1_024);
    assert!(index_capacity <= 2_048);
    assert!(bucket_capacity <= 4);
    assert!(free <= 1);
}

#[test]
fn all_token_roots_live_grow_by_exact_objects_and_logical_bytes() {
    const ROOTS: usize = 2_048;
    let mut store = TokenStore::new();
    let baseline = store.testing_live_totals();
    let roots = (0..ROOTS)
        .map(|value| {
            store.testing_owned(
                &[
                    char_token(char::from_u32(0x1_0000 + value as u32).expect("valid scalar")),
                    char_token('!'),
                ],
                TokenSemanticId::testing(value as u64 + 1),
                None,
            )
        })
        .collect::<Vec<_>>();
    let grown = store.testing_live_totals();
    let bytes_per_root =
        core::mem::size_of::<super::TokenListValue>() + 2 * core::mem::size_of::<Token>();

    assert_eq!(grown.0 - baseline.0, ROOTS);
    assert_eq!(grown.1 - baseline.1, ROOTS * bytes_per_root);
    assert_eq!(store.testing_pool_shape().0 - baseline.0, ROOTS);
    assert_eq!(roots.len(), ROOTS);
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
        let mut model: Vec<(Vec<Token>, TokenListId)> =
            vec![(Vec::new(), TokenListId::EMPTY)];
        let mut model_index: AHashMap<Vec<Token>, TokenListId> =
            AHashMap::from([(Vec::new(), TokenListId::EMPTY)]);
        let mut marks: Vec<(TokenStoreMark, usize)> = vec![(store.watermark(), model.len())];

        for op in ops {
            match op {
                Op::Intern(tokens) => {
                    let id = store.intern(&tokens);
                    let expected = model_id(&mut model, &mut model_index, &tokens, id);
                    prop_assert_eq!(id, expected);
                }
                Op::Build(tokens) => {
                    let mut builder = TokenListBuilder::new();
                    for token in &tokens {
                        builder.push(*token);
                    }
                    let id = builder.finish(&mut store);
                    let expected = model_id(&mut model, &mut model_index, &tokens, id);
                    prop_assert_eq!(id, expected);
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

            for (expected, id) in &model {
                let id = store
                    .resolve_stored(*id)
                    .expect("model slot should resolve to a live token-list identity");
                prop_assert_eq!(store.get(id), expected.as_slice());
                prop_assert_eq!(store.intern(expected), id);
            }
        }
    }
}

fn model_id(
    model: &mut Vec<(Vec<Token>, TokenListId)>,
    index: &mut AHashMap<Vec<Token>, TokenListId>,
    tokens: &[Token],
    actual: TokenListId,
) -> TokenListId {
    if let Some(&id) = index.get(tokens) {
        return id;
    }
    let tokens = tokens.to_vec();
    model.push((tokens.clone(), actual));
    index.insert(tokens, actual);
    actual
}

fn rebuild_model_index(model: &[(Vec<Token>, TokenListId)]) -> AHashMap<Vec<Token>, TokenListId> {
    model.iter().cloned().collect()
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
