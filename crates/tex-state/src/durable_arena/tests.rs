use crate::generation::with_generation;
use crate::glue::GlueSpec;
use crate::provenance::{OriginRecord, SyntheticOrigin, SyntheticOriginKind};
use crate::token::{Token, TokenWord};

fn collect_words<G>(view: super::TokenListView<G>) -> Vec<TokenWord> {
    view.iter().collect()
}

#[test]
fn typed_arenas_publish_and_resolve_distinct_rows() {
    with_generation(|mut generation| {
        let words = [TokenWord::pack(Token::frozen_relax())];
        let first_tokens = generation
            .token_lists_mut()
            .allocate(&words)
            .expect("test fixture is valid");
        let second_tokens = generation
            .token_lists_mut()
            .allocate(&words)
            .expect("test fixture is valid");
        let first_glue = generation
            .glue_mut()
            .allocate(GlueSpec::ZERO)
            .expect("test fixture is valid");
        let second_glue = generation
            .glue_mut()
            .allocate(GlueSpec::ZERO)
            .expect("test fixture is valid");
        let provenance = OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Test));
        let first_origin = generation
            .provenance_mut()
            .allocate(provenance)
            .expect("test fixture is valid");
        let second_origin = generation
            .provenance_mut()
            .allocate(provenance)
            .expect("test fixture is valid");

        assert_ne!(first_tokens, second_tokens);
        assert_ne!(first_glue, second_glue);
        assert_ne!(first_origin, second_origin);
        assert_eq!(
            collect_words(generation.token_lists().get(first_tokens)),
            words
        );
        assert_eq!(generation.glue().get(first_glue), GlueSpec::ZERO);
        assert_eq!(generation.provenance().get(first_origin), provenance);
    });
}

#[test]
fn definition_words_and_durable_lists_have_separate_storage() {
    with_generation(|mut generation| {
        let word = TokenWord::pack(Token::frozen_relax());
        let definition = generation
            .definitions_mut()
            .allocate(&[], &[word])
            .expect("test fixture is valid");
        let list = generation
            .token_lists_mut()
            .allocate(&[word])
            .expect("test fixture is valid");

        assert_eq!(
            generation.definitions().get(definition).replacement_text(),
            [word]
        );
        assert_eq!(collect_words(generation.token_lists().get(list)), [word]);
        assert_eq!(generation.definitions().len(), 1);
        assert_eq!(generation.token_lists().len(), 1);
    });
}

#[test]
fn nested_builders_seal_distinct_interleaved_sequences_in_place() {
    with_generation(|mut generation| {
        let arena = generation.token_lists_mut();
        let parent = arena.begin_builder().expect("parent builder");
        arena
            .push_builder_word(&parent, TokenWord::from_raw(1))
            .expect("parent prefix");
        let child = arena.begin_builder().expect("child builder");
        arena
            .push_builder_word(&child, TokenWord::from_raw(2))
            .expect("child word");
        let child = arena.seal_builder(child).expect("seal child");
        arena
            .push_builder_word(&parent, TokenWord::from_raw(3))
            .expect("parent suffix");
        let parent = arena.seal_builder(parent).expect("seal parent");

        assert_eq!(collect_words(arena.get(child)), [TokenWord::from_raw(2)]);
        assert_eq!(
            collect_words(arena.get(parent)),
            [TokenWord::from_raw(1), TokenWord::from_raw(3)]
        );
        assert_eq!(arena.retained_chunk_len(), 2);
        assert_eq!(arena.retained_builder_slot_len(), 2);
    });
}

#[test]
fn cross_chunk_replay_restarts_and_streaming_identity_are_exact() {
    use core::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    with_generation(|mut generation| {
        let expected = (0..(super::TOKEN_CHUNK_WORDS * 2 + 7))
            .map(|raw| TokenWord::from_raw(raw as u32))
            .collect::<Vec<_>>();
        let id = generation
            .token_lists_mut()
            .allocate(&expected)
            .expect("multi-chunk list");
        let arena = generation.token_lists();
        let view = arena.get(id.clone());
        assert_eq!(view.iter().collect::<Vec<_>>(), expected);
        assert_eq!(view.iter().collect::<Vec<_>>(), expected);

        let mut cursor = view.cursor();
        for expected in &expected {
            assert_eq!(cursor.next_word(), Some(*expected));
        }
        assert_eq!(cursor.next_word(), None);

        let mut view_hash = DefaultHasher::new();
        view.hash(&mut view_hash);
        let mut expected_hash = DefaultHasher::new();
        expected.len().hash(&mut expected_hash);
        for word in &expected {
            word.hash(&mut expected_hash);
        }
        assert_eq!(view_hash.finish(), expected_hash.finish());
        assert_eq!(
            vec![id.capture_format()],
            vec![expected.iter().map(|word| word.raw()).collect::<Vec<_>>()]
        );
        assert_eq!(arena.retained_chunk_len(), 3);
    });
}

#[test]
fn discarded_builder_reuses_the_chunk_released_after_publication() {
    with_generation(|mut generation| {
        let arena = generation.token_lists_mut();
        let sealed = arena
            .allocate(&[TokenWord::from_raw(7)])
            .expect("sealed row");
        let rejected = arena.begin_builder().expect("rejected builder");
        arena
            .push_builder_word(&rejected, TokenWord::from_raw(8))
            .expect("rejected word");
        arena.discard_builder(rejected).expect("discard builder");
        let replacement = arena.begin_builder().expect("replacement builder");
        arena
            .push_builder_word(&replacement, TokenWord::from_raw(9))
            .expect("replacement word");
        let replacement = arena.seal_builder(replacement).expect("seal replacement");

        assert_eq!(collect_words(arena.get(sealed)), [TokenWord::from_raw(7)]);
        assert_eq!(
            collect_words(arena.get(replacement)),
            [TokenWord::from_raw(9)]
        );
        assert_eq!(arena.retained_chunk_len(), 1);
        assert_eq!(arena.retained_builder_slot_len(), 1);
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_alias_and_read_cycles_allocate_zero_heap() {
    with_generation(|mut generation| {
        let arena = generation.token_lists_mut();
        let id = arena
            .allocate(&[TokenWord::from_raw(7)])
            .expect("published list");
        let owner = crate::measurement::HotCoreAllocationOwner::ArenaGrowth;
        let before = crate::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = crate::measurement::hot_core_allocation_scope(owner);
            for _ in 0..8_192 {
                let alias = id.clone();
                assert_eq!(arena.get(alias).iter().next(), Some(TokenWord::from_raw(7)));
            }
        }
        let after = crate::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(arena.len(), 1);
        assert_eq!(arena.retained_chunk_len(), 1);
        assert_eq!(arena.retained_builder_slot_len(), 1);
    });
}

#[test]
fn token_list_aliases_release_exactly_on_owner_drop() {
    with_generation(|mut generation| {
        let id = generation
            .token_lists_mut()
            .allocate(&[TokenWord::from_raw(7)])
            .expect("published list");
        // The generation store retains one immutable owner independently of
        // transient command handles.
        assert_eq!(id.semantic_owner_count(), 2);

        let alias = id.clone();
        assert_eq!(id.semantic_owner_count(), 3);
        let view = generation.token_lists().get(alias);
        assert_eq!(id.semantic_owner_count(), 3);
        let cursor = view.cursor();
        assert_eq!(id.semantic_owner_count(), 4);
        drop(cursor);
        drop(view);
        assert_eq!(id.semantic_owner_count(), 2);
    });
}

#[cfg(feature = "profiling")]
#[test]
fn node_key_publication_and_copy_allocate_zero_heap() {
    with_generation(|mut generation| {
        let id = generation
            .token_lists_mut()
            .allocate(&[TokenWord::from_raw(7), TokenWord::from_raw(9)])
            .expect("published list");
        let owner = crate::measurement::HotCoreAllocationOwner::SemanticApply;
        let before = crate::measurement::hot_core_thread_allocation_measurement(owner);
        let key = {
            let _scope = crate::measurement::hot_core_allocation_scope(owner);
            let key = generation
                .token_lists()
                .node_key(&id)
                .expect("published token list has a node coordinate");
            for _ in 0..8_192 {
                let copy = key;
                core::hint::black_box(copy);
            }
            key
        };
        let after = crate::measurement::hot_core_thread_allocation_measurement(owner);

        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(id.semantic_owner_count(), 2);
        assert_eq!(
            generation.token_lists().node_words(key),
            Some([TokenWord::from_raw(7), TokenWord::from_raw(9)].as_slice())
        );
    });
}

#[test]
fn node_token_keys_are_compact_copy_coordinates_with_exact_alias_replay() {
    assert_eq!(core::mem::size_of::<crate::node::NodeTokenKey>(), 24);
    assert_eq!(core::mem::align_of::<crate::node::NodeTokenKey>(), 4);
    assert!(!core::mem::needs_drop::<crate::node::NodeTokenKey>());

    with_generation(|mut generation| {
        let id = generation
            .token_lists_mut()
            .allocate(&[TokenWord::from_raw(7), TokenWord::from_raw(9)])
            .expect("published list");
        let key = generation
            .token_lists()
            .node_key(&id)
            .expect("published token list has a node coordinate");
        let alias = key;

        assert_eq!(key, alias);
        assert_eq!(id.semantic_owner_count(), 2);
        assert_eq!(
            generation.token_lists().node_words(alias),
            Some([TokenWord::from_raw(7), TokenWord::from_raw(9)].as_slice())
        );
    });
}

#[test]
fn generation_store_owns_node_words_and_releases_accounting_at_retirement() {
    let accounting = with_generation(|mut generation| {
        let accounting = generation.memory_accounting();
        let id = generation
            .token_lists_mut()
            .allocate(&[TokenWord::from_raw(7), TokenWord::from_raw(9)])
            .expect("published list");
        let key = generation
            .token_lists()
            .node_key(&id)
            .expect("node coordinate");
        drop(id);

        assert_eq!(accounting.words(false), (0, 3));
        assert_eq!(
            generation.token_lists().node_words(key),
            Some([TokenWord::from_raw(7), TokenWord::from_raw(9)].as_slice())
        );
        accounting
    });

    assert_eq!(accounting.words(false), (0, 0));
}

#[test]
fn token_coordinate_cutover_preserves_the_resident_node_stage_boundary() {
    assert_eq!(core::mem::size_of::<crate::node::Node>(), 168);
    assert!(core::mem::needs_drop::<crate::node::Node>());
    assert_eq!(core::mem::size_of::<crate::node::Whatsit>(), 56);
    assert_eq!(core::mem::size_of::<crate::node::PdfDestinationNode>(), 60);
    assert_eq!(core::mem::size_of::<crate::node::PdfThreadNode>(), 80);
}

#[test]
fn node_token_keys_reject_foreign_and_reused_rows() {
    with_generation(|mut generation| {
        let checkpoint = generation.token_lists().cursor();
        let first = generation
            .token_lists_mut()
            .allocate(&[TokenWord::from_raw(7)])
            .expect("first publication");
        let stale = generation
            .token_lists()
            .node_key(&first)
            .expect("first node key");
        let [owner, row, incarnation, offset, len, publication] = stale.coordinates();
        let foreign = crate::node::NodeTokenKey::new(
            owner.wrapping_add(1),
            row,
            incarnation,
            offset,
            len,
            publication,
        );
        assert_eq!(generation.token_lists().node_words(foreign), None);
        let malformed_empty = crate::node::NodeTokenKey::new(owner, row, 1, 0, 0, publication);
        assert_eq!(generation.token_lists().node_words(malformed_empty), None);
        assert_eq!(
            generation
                .token_lists()
                .node_words(crate::node::NodeTokenKey::default()),
            Some([].as_slice())
        );

        generation.token_lists_mut().restore_cursor(checkpoint);
        let replacement = generation
            .token_lists_mut()
            .allocate(&[TokenWord::from_raw(9)])
            .expect("replacement publication");
        let replacement_key = generation
            .token_lists()
            .node_key(&replacement)
            .expect("replacement node key");

        assert_eq!(generation.token_lists().node_words(stale), None);
        assert_eq!(
            generation.token_lists().node_words(replacement_key),
            Some([TokenWord::from_raw(9)].as_slice())
        );
    });
}

#[test]
fn node_token_keys_settle_accepted_and_candidate_suffixes_exactly() {
    with_generation(|mut generation| {
        let prefix = generation
            .token_lists_mut()
            .allocate(&[TokenWord::from_raw(1)])
            .expect("prefix publication");
        let checkpoint = generation.token_lists().cursor();
        let accepted = generation
            .token_lists_mut()
            .allocate(&[TokenWord::from_raw(2)])
            .expect("accepted publication");
        let prefix_key = generation
            .token_lists()
            .node_key(&prefix)
            .expect("prefix key");
        let accepted_key = generation
            .token_lists()
            .node_key(&accepted)
            .expect("accepted key");

        let tail = generation
            .token_lists_mut()
            .begin_checkpoint_candidate(checkpoint);
        let rejected = generation
            .token_lists_mut()
            .allocate(&[TokenWord::from_raw(3)])
            .expect("candidate publication");
        let rejected_key = generation
            .token_lists()
            .node_key(&rejected)
            .expect("candidate key");
        assert_eq!(generation.token_lists().node_words(accepted_key), None);
        generation
            .token_lists_mut()
            .reject_checkpoint_candidate(checkpoint, tail);

        assert_eq!(
            generation.token_lists().node_words(prefix_key),
            Some([TokenWord::from_raw(1)].as_slice())
        );
        assert_eq!(
            generation.token_lists().node_words(accepted_key),
            Some([TokenWord::from_raw(2)].as_slice())
        );
        assert_eq!(generation.token_lists().node_words(rejected_key), None);

        let tail = generation
            .token_lists_mut()
            .begin_checkpoint_candidate(checkpoint);
        let selected = generation
            .token_lists_mut()
            .allocate(&[TokenWord::from_raw(4)])
            .expect("selected publication");
        let selected_key = generation
            .token_lists()
            .node_key(&selected)
            .expect("selected key");
        generation
            .token_lists_mut()
            .accept_checkpoint_candidate(tail);

        assert_eq!(generation.token_lists().node_words(accepted_key), None);
        assert_eq!(
            generation.token_lists().node_words(selected_key),
            Some([TokenWord::from_raw(4)].as_slice())
        );
    });
}
