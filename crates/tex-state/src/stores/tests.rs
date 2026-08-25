use super::{DynamicMemoryScratch, StateCore};
use crate::env::AssignmentScope;
use crate::generation::with_generation;
use crate::glue::GlueSpec;
use crate::interner::{Interner, InternerBudget};
use crate::meaning::{MeaningFlags, MeaningWord, ResolvedMeaning};
use crate::node::Node;
use crate::token::{Catcode, Token, TokenWord};

#[cfg(feature = "profiling")]
#[global_allocator]
static PROFILING_ALLOCATOR: crate::measurement::HotCoreAllocator =
    crate::measurement::HotCoreAllocator;

#[test]
fn admitted_view_resolves_every_generation_typed_value_directly() {
    with_generation(|generation| {
        let mut core = StateCore::new(generation).expect("state core");
        let replacement = [TokenWord::pack(Token::frozen_relax())];
        let (definition, tokens, glue) = {
            let mut admitted = core.admit_mut().expect("unique generation");
            let definition = admitted
                .allocate_definition(&[], &replacement)
                .expect("definition");
            let tokens = admitted
                .allocate_token_list(&replacement)
                .expect("token list");
            let glue = admitted.allocate_glue(GlueSpec::ZERO).expect("glue");
            (definition, tokens, glue)
        };

        let admitted = core.admit();
        assert_eq!(
            admitted.definition(definition).replacement_text(),
            replacement
        );
        assert_eq!(
            admitted.token_list(tokens).iter().collect::<Vec<_>>(),
            replacement
        );
        assert_eq!(admitted.glue(glue), GlueSpec::ZERO);
    });
}

#[test]
fn generation_ids_install_in_dense_state_without_per_value_owners() {
    with_generation(|generation| {
        let mut names = Interner::new(InternerBudget::new(8, 8, 128).expect("budget"));
        let symbol = names.intern("macro").expect("intern");
        let mut core = StateCore::new(generation).expect("state core");
        let definition = {
            let mut admitted = core.admit_mut().expect("unique generation");
            admitted
                .state()
                .admit_symbol(symbol.symbol())
                .expect("symbol admission");
            let definition = admitted.allocate_definition(&[], &[]).expect("definition");
            admitted
                .state()
                .assign_meaning(
                    symbol.symbol(),
                    MeaningWord::macro_definition(MeaningFlags::LONG, definition.clone()),
                    AssignmentScope::Global,
                )
                .expect("meaning assignment");
            definition
        };

        assert_eq!(
            core.admit()
                .state()
                .meaning(symbol.symbol())
                .expect("meaning"),
            ResolvedMeaning::Macro {
                flags: MeaningFlags::LONG,
                definition,
            }
        );
    });
}

#[test]
fn retirement_releases_one_complete_generation_bundle() {
    with_generation(|generation| {
        let mut core = StateCore::new(generation).expect("state core");
        {
            let mut admitted = core.admit_mut().expect("unique generation");
            admitted.allocate_definition(&[], &[]).expect("definition");
            admitted.allocate_token_list(&[]).expect("token list");
            admitted.allocate_glue(GlueSpec::ZERO).expect("glue");
            admitted
                .state()
                .assign_count(0, 7, AssignmentScope::Global)
                .expect("assignment");
        }
        let retired = core.retire().expect("unique generation");
        assert_eq!(retired.generation.definitions, 1);
        assert_eq!(retired.generation.token_lists, 1);
        assert_eq!(retired.generation.glue_values, 1);
        assert_eq!(retired.generation.provenance_records, 0);
        assert_eq!(retired.journal_entries, 1);
    });
}

#[test]
fn immutable_last_owner_and_node_suffix_release_update_exact_count() {
    with_generation(|generation| {
        let mut core = StateCore::new(generation).expect("state core");
        let cursor = core.durable_node_cursor();
        let (definition, tokens) = {
            let mut admitted = core.admit_mut().expect("unique generation");
            let definition = admitted
                .allocate_definition(&[], &[TokenWord::pack(Token::frozen_relax())])
                .expect("definition");
            let tokens = admitted
                .allocate_token_list(&[TokenWord::pack(Token::frozen_relax())])
                .expect("token list");
            admitted
                .nodes_mut()
                .publish(vec![Node::Char {
                    font: crate::font::NULL_FONT,
                    ch: 'x',
                    origin: crate::token::OriginId::UNKNOWN,
                }])
                .expect("node list");
            (definition, tokens)
        };

        assert_eq!(
            core.admit()
                .current_dynamic_memory_words(false)
                .expect("constant-time count"),
            14 + 4 + 2 + 1
        );
        let definition_alias = definition.clone();
        let token_alias = tokens.clone();
        drop(definition);
        drop(tokens);
        assert_eq!(
            core.admit()
                .current_dynamic_memory_words(false)
                .expect("aliases retain payloads"),
            14 + 4 + 2 + 1
        );
        drop(definition_alias);
        drop(token_alias);
        assert_eq!(
            core.admit()
                .current_dynamic_memory_words(false)
                .expect("final drops release payloads"),
            15
        );
        core.truncate_durable_nodes(cursor).expect("release suffix");
        assert_eq!(
            core.admit()
                .current_dynamic_memory_words(false)
                .expect("node release updates count"),
            14
        );
    });
}

#[test]
fn constant_time_memory_count_tracks_aliases_and_final_release() {
    with_generation(|generation| {
        let mut names = Interner::new(InternerBudget::new(8, 8, 128).expect("budget"));
        let first = names.intern("first").expect("first symbol").symbol();
        let second = names.intern("second").expect("second symbol").symbol();
        let words = [
            TokenWord::pack(Token::frozen_relax()),
            TokenWord::pack(Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            }),
        ];
        let mut core = StateCore::new(generation).expect("state core");
        let nodes = {
            let mut admitted = core.admit_mut().expect("unique generation");
            admitted.state().admit_symbol(first).expect("first symbol");
            admitted
                .state()
                .admit_symbol(second)
                .expect("second symbol");
            let definition = admitted
                .allocate_definition(&words[..1], &words[1..])
                .expect("definition");
            let tokens = admitted.allocate_token_list(&words).expect("token list");
            let nodes = admitted
                .nodes_mut()
                .publish(vec![Node::Penalty(7)])
                .expect("node list");
            for symbol in [first, second] {
                admitted
                    .state()
                    .assign_meaning(
                        symbol,
                        MeaningWord::macro_definition(MeaningFlags::LONG, definition.clone()),
                        AssignmentScope::Global,
                    )
                    .expect("meaning assignment");
            }
            for register in [1, 2] {
                admitted
                    .state()
                    .assign_token_register(register, Some(tokens.clone()), AssignmentScope::Global)
                    .expect("token assignment");
            }
            for register in [3, 4] {
                admitted
                    .state()
                    .assign_box_register(register, Some(nodes), AssignmentScope::Global)
                    .expect("box assignment");
            }
            nodes
        };

        let admitted = core.admit();
        let expected = 14 + 5 + 3;
        let expected_copy = admitted
            .nodes
            .semantic_closure_tex_memory_words(nodes, true)
            .map(|(variable, dynamic)| (variable.saturating_mul(2), dynamic))
            .expect("materialized copied closure count");
        let mut scratch = DynamicMemoryScratch::default();
        for _ in 0..3 {
            assert_eq!(
                admitted
                    .copied_node_closure_tex_memory_words(nodes, true, &mut scratch)
                    .expect("borrowed copied closure count"),
                expected_copy,
            );
            assert_eq!(
                admitted
                    .current_dynamic_memory_words(true)
                    .expect("constant-time count"),
                expected,
            );
        }

        #[cfg(feature = "profiling")]
        {
            let owner = crate::measurement::HotCoreAllocationOwner::SemanticApply;
            let before = crate::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = crate::measurement::hot_core_allocation_scope(owner);
                for _ in 0..8_192 {
                    assert_eq!(
                        admitted
                            .copied_node_closure_tex_memory_words(nodes, true, &mut scratch)
                            .expect("warmed borrowed copied closure count"),
                        expected_copy,
                    );
                    assert_eq!(
                        admitted
                            .current_dynamic_memory_words(true)
                            .expect("warmed constant-time count"),
                        expected,
                    );
                }
            }
            let after = crate::measurement::hot_core_thread_allocation_measurement(owner);
            assert_eq!(after.calls - before.calls, 0);
            assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        }
    });
}

#[test]
fn reusable_marks_scale_with_reachable_high_water_not_sparse_raw_ids() {
    let mut marks = crate::node_arena::StampedIndexMap::default();
    marks.begin();
    assert!(marks.mark(7));
    assert!(marks.mark(1_000_000_007));
    assert!(!marks.mark(7));
    assert_eq!(marks.len(), 2);
    assert_eq!(marks.capacity(), 16);

    marks.begin();
    assert!(marks.mark(usize::MAX - 1));
    assert_eq!(marks.len(), 1);
    assert_eq!(marks.capacity(), 16);
}
