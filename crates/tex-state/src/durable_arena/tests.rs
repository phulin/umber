use crate::generation::with_generation;
use crate::glue::GlueSpec;
use crate::provenance::{OriginRecord, SyntheticOrigin, SyntheticOriginKind};
use crate::token::{Token, TokenWord};

fn collect_words<G>(view: super::TokenListView<'_, G>) -> Vec<TokenWord> {
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
        let view = arena.get(id);
        assert_eq!(view.iter().collect::<Vec<_>>(), expected);
        assert_eq!(view.iter().collect::<Vec<_>>(), expected);

        let mut cursor = view.cursor();
        for expected in &expected {
            assert_eq!(arena.cursor_word(cursor), Ok(*expected));
            arena.advance_cursor(&mut cursor).expect("advance cursor");
        }
        assert!(arena.cursor_word(cursor).is_err());

        let mut view_hash = DefaultHasher::new();
        view.hash(&mut view_hash);
        let mut expected_hash = DefaultHasher::new();
        expected.len().hash(&mut expected_hash);
        for word in &expected {
            word.hash(&mut expected_hash);
        }
        assert_eq!(view_hash.finish(), expected_hash.finish());
        assert_eq!(
            arena.capture_format_rows(),
            vec![expected.iter().map(|word| word.raw()).collect::<Vec<_>>()]
        );
        assert_eq!(arena.retained_chunk_len(), 3);
    });
}

#[test]
fn discarded_builder_reuses_its_chunk_chain_without_touching_sealed_rows() {
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
        assert_eq!(arena.retained_chunk_len(), 2);
        assert_eq!(arena.retained_builder_slot_len(), 1);
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_8192_build_seal_read_cycles_allocate_zero_heap() {
    with_generation(|mut generation| {
        let arena = generation.token_lists_mut();
        arena.reserve_batch(8_192, 8_192).expect("warm capacity");
        let owner = crate::measurement::HotCoreAllocationOwner::ArenaGrowth;
        let before = crate::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = crate::measurement::hot_core_allocation_scope(owner);
            for raw in 0..8_192_u32 {
                let builder = arena.begin_builder().expect("builder");
                arena
                    .push_builder_word(&builder, TokenWord::from_raw(raw))
                    .expect("append");
                let id = arena.seal_builder(builder).expect("seal");
                assert_eq!(arena.get(id).iter().next(), Some(TokenWord::from_raw(raw)));
            }
        }
        let after = crate::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(arena.len(), 8_192);
        assert_eq!(arena.retained_chunk_len(), 8_192);
        assert_eq!(arena.retained_builder_slot_len(), 1);
    });
}
