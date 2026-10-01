use super::{
    AdmittedMacroDefinition, DefinitionAllocationError, DefinitionBuildError, DefinitionBuildPhase,
    DefinitionBuilder, DefinitionDestination,
};
use crate::generation::with_generation;
use crate::token::{Catcode, Token, TokenWord};

fn direct_definition<G>(
    arena: &mut super::DefinitionArena<G>,
    destination: DefinitionDestination,
    parameter: &[TokenWord],
    replacement: &[TokenWord],
) -> super::DefinitionRef<G> {
    let build = arena
        .begin_build(destination, crate::token::OriginId::UNKNOWN)
        .expect("definition transaction");
    for &word in parameter {
        arena.push_parameter(build, word).expect("parameter word");
    }
    arena.finish_parameters(build).expect("parameter boundary");
    for &word in replacement {
        arena
            .push_replacement(build, word)
            .expect("replacement word");
    }
    arena.seal_build(build).expect("sealed definition")
}

fn checked_builder(
    parameter_text: &[TokenWord],
    replacement_text: &[TokenWord],
) -> DefinitionBuilder {
    let mut builder = DefinitionBuilder::new();
    for &word in parameter_text {
        builder.push_parameter(word).expect("parameter word");
    }
    builder.finish_parameters().expect("parameter boundary");
    for &word in replacement_text {
        builder.push_replacement(word).expect("replacement word");
    }
    builder.seal().expect("sealed builder");
    builder
}

fn letter(ch: char) -> TokenWord {
    TokenWord::pack(Token::Char {
        ch,
        cat: Catcode::Letter,
    })
}

#[test]
fn definition_handles_stay_compact() {
    assert_eq!(std::mem::size_of::<super::DefinitionRef<()>>(), 8);
    assert_eq!(std::mem::size_of::<super::ResidentMacroBody<()>>(), 8);
}

#[test]
fn admitted_writer_round_trips_the_build_lane_without_copying() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let build = arena
            .begin_build(
                DefinitionDestination::Global,
                crate::token::OriginId::UNKNOWN,
            )
            .expect("definition transaction");
        let mut writer = arena
            .admit_build_writer(build, DefinitionBuildPhase::OpenParameters)
            .expect("writer admission");
        writer
            .push_parameter(TokenWord::pack(Token::param(1)))
            .expect("parameter");
        writer.finish_parameters().expect("parameter boundary");
        assert_eq!(
            arena.seal_build(build),
            Err(DefinitionBuildError::InvalidPhase),
            "an admitted writer owns the build"
        );
        let build = arena.release_build_writer(writer).expect("writer release");
        let mut writer = arena
            .admit_build_writer(build, DefinitionBuildPhase::OpenReplacement)
            .expect("writer readmission");
        writer.push_replacement(letter('a')).expect("replacement");
        writer
            .push_replacement(TokenWord::pack(Token::param(1)))
            .expect("replacement reference");
        let build = arena.release_build_writer(writer).expect("writer release");
        let definition = arena.seal_build(build).expect("sealed definition");
        let view = arena.get(definition);
        assert_eq!(view.parameter_text(), [TokenWord::pack(Token::param(1))]);
        assert_eq!(
            view.replacement_text(),
            [letter('a'), TokenWord::pack(Token::param(1))]
        );
    });
}

#[test]
fn aborted_build_publishes_nothing_and_releases_its_lane() {
    with_generation(|mut generation| {
        let baseline = generation.memory_accounting().words(false);
        let arena = generation.definitions_mut();
        let build = arena
            .begin_build(
                DefinitionDestination::Global,
                crate::token::OriginId::UNKNOWN,
            )
            .expect("definition transaction");
        arena.finish_parameters(build).expect("parameter boundary");
        arena.push_replacement(build, letter('x')).expect("word");
        arena.abort_build(build);
        assert!(arena.is_empty());
        let next = direct_definition(arena, DefinitionDestination::Global, &[], &[letter('y')]);
        assert_eq!(arena.get(next).replacement_text(), [letter('y')]);
        arena.restore_cursor(super::DefinitionArenaCursor {
            format_rows: 0,
            global_rows: 0,
        });
        assert_eq!(generation.memory_accounting().words(false), baseline);
    });
}

#[test]
fn local_frontier_capture_rejects_an_active_build() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let frontier_1 = arena.local_frontier().expect("group");
        let build = arena
            .begin_build(
                DefinitionDestination::Local,
                crate::token::OriginId::UNKNOWN,
            )
            .expect("local build");
        assert_eq!(
            arena.local_frontier(),
            Err(DefinitionAllocationError::InvalidDefinition),
            "a group cannot open across an active build"
        );
        arena.finish_parameters(build).expect("parameter boundary");
        let local = arena.seal_build(build).expect("sealed local");
        assert!(arena.get(local).replacement_text().is_empty());
        arena.retire_local_suffix(frontier_1);
        assert!(arena.is_empty());
    });
}

#[test]
fn group_exit_drops_only_its_own_local_rows() {
    with_generation(|mut generation| {
        let baseline = generation.memory_accounting().words(false);
        let arena = generation.definitions_mut();
        let frontier_2 = arena.local_frontier().expect("outer group");
        let outer = direct_definition(arena, DefinitionDestination::Local, &[], &[letter('o')]);
        let frontier_3 = arena.local_frontier().expect("empty group");
        arena.retire_local_suffix(frontier_3);
        let frontier_4 = arena.local_frontier().expect("inner group");
        let inner = direct_definition(arena, DefinitionDestination::Local, &[], &[letter('i')]);
        let global = direct_definition(arena, DefinitionDestination::Global, &[], &[letter('g')]);
        assert_eq!(arena.len(), 3);
        arena.retire_local_suffix(frontier_4);
        assert_eq!(arena.len(), 2);
        assert_eq!(arena.get(outer).replacement_text(), [letter('o')]);
        assert_eq!(arena.get(global).replacement_text(), [letter('g')]);
        assert!(arena.entry(inner).is_none());
        arena.retire_local_suffix(frontier_2);
        assert_eq!(arena.len(), 1);
        arena.restore_cursor(super::DefinitionArenaCursor {
            format_rows: 0,
            global_rows: 0,
        });
        assert_eq!(generation.memory_accounting().words(false), baseline);
    });
}

#[test]
fn executing_local_body_outlives_its_group() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let frontier_5 = arena.local_frontier().expect("local definition group");
        let parameters = [
            TokenWord::pack(Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            }),
            TokenWord::pack(Token::param(1)),
        ];
        let words = [
            TokenWord::from_raw(17),
            TokenWord::from_raw(23),
            TokenWord::from_raw(29),
        ];
        let definition =
            direct_definition(arena, DefinitionDestination::Local, &parameters, &words);
        let body = arena
            .admit_macro_body(definition)
            .expect("resident local body");
        assert_eq!(body.profile_region_owner_count(), 2);

        arena.retire_local_suffix(frontier_5);

        assert!(arena.entry(definition).is_none());
        assert_eq!(body.profile_region_owner_count(), 1);
        assert_eq!(body.parameter_len(), parameters.len());
        assert_eq!(body.parameter_word(0), Some(parameters[0]));
        assert_eq!(body.parameter_word(1), Some(parameters[1]));
        assert_eq!(body.parameter_word(2), None);
        assert_eq!(body.len(), words.len());
        for (position, expected) in words.iter().copied().enumerate() {
            assert_eq!(body.word(position), Some(expected));
            assert_eq!(body.word_at(position as u32), Some(expected));
        }
        assert_eq!(body.word(3), None);
        assert_eq!(body.word_at(3), None);
        assert_eq!(body.replacement_from(1), &words[1..]);
        assert!(body.replacement_from(4).is_empty());
    });
}

#[test]
fn admitted_macro_definition_classifies_simple_matching_and_empty() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let empty = direct_definition(arena, DefinitionDestination::Global, &[], &[]);
        let simple = direct_definition(arena, DefinitionDestination::Global, &[], &[letter('x')]);
        let matching = direct_definition(
            arena,
            DefinitionDestination::Global,
            &[TokenWord::pack(Token::param(1))],
            &[TokenWord::pack(Token::param(1))],
        );
        assert!(matches!(
            arena.admit_macro_definition(empty),
            Some(AdmittedMacroDefinition::SimpleMacro { body: None, .. })
        ));
        assert!(matches!(
            arena.admit_macro_definition(simple),
            Some(AdmittedMacroDefinition::SimpleMacro { body: Some(_), .. })
        ));
        assert!(matches!(
            arena.admit_macro_definition(matching),
            Some(AdmittedMacroDefinition::MatchingMacro { body }) if body.parameter_len() == 1
        ));
    });
}

#[test]
fn local_promotion_copies_once_and_forgets_dropped_rows() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let frontier_6 = arena.local_frontier().expect("outer group");
        let outer = direct_definition(arena, DefinitionDestination::Local, &[], &[letter('o')]);
        let frontier_7 = arena.local_frontier().expect("inner group");
        let inner = direct_definition(arena, DefinitionDestination::Local, &[], &[letter('i')]);
        let promoted_outer = arena.promote_global(outer).expect("outer promotion");
        let promoted_inner = arena.promote_global(inner).expect("inner promotion");
        assert_eq!(arena.promote_global(outer), Ok(promoted_outer));
        assert_eq!(arena.promote_global(promoted_outer), Ok(promoted_outer));
        arena.retire_local_suffix(frontier_7);

        let replacement =
            direct_definition(arena, DefinitionDestination::Local, &[], &[letter('r')]);
        assert_eq!(replacement, inner, "the dropped local row is reused");
        let promoted_replacement = arena.promote_global(replacement).expect("promotion");
        assert_ne!(promoted_replacement, promoted_inner);
        assert_eq!(arena.promote_global(outer), Ok(promoted_outer));
        arena.retire_local_suffix(frontier_6);

        assert_eq!(arena.get(promoted_outer).replacement_text(), [letter('o')]);
        assert_eq!(arena.get(promoted_inner).replacement_text(), [letter('i')]);
        assert_eq!(
            arena.get(promoted_replacement).replacement_text(),
            [letter('r')]
        );
    });
}

#[test]
fn checkpoint_rejection_restores_detached_rows_at_their_numbers() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let root = direct_definition(arena, DefinitionDestination::Global, &[], &[letter('r')]);
        let checkpoint = arena.cursor();
        let accepted = direct_definition(arena, DefinitionDestination::Global, &[], &[letter('a')]);
        let tail = arena.begin_checkpoint_candidate(checkpoint);
        let candidate = direct_definition(arena, DefinitionDestination::Global, &[], &[]);
        assert_eq!(candidate, accepted, "the candidate reuses the detached row");
        assert!(
            !arena.current_and_accepted_contents_equal(&tail, candidate, accepted),
            "a reused row number compares against the detached accepted row"
        );
        let equal = direct_definition(arena, DefinitionDestination::Global, &[], &[letter('a')]);
        assert!(arena.current_and_accepted_contents_equal(&tail, equal, accepted));
        assert!(arena.current_and_accepted_contents_equal(&tail, root, root));
        arena.reject_checkpoint_candidate(checkpoint, tail);
        assert_eq!(arena.get(root).replacement_text(), [letter('r')]);
        assert_eq!(arena.get(accepted).replacement_text(), [letter('a')]);
        assert_eq!(arena.len(), 2);
    });
}

#[test]
fn checkpoint_acceptance_releases_the_detached_rows() {
    with_generation(|mut generation| {
        let checkpoint = generation.definitions().cursor();
        let baseline = generation.memory_accounting().words(false);
        let arena = generation.definitions_mut();
        let _accepted =
            direct_definition(arena, DefinitionDestination::Global, &[], &[letter('a')]);
        let tail = arena.begin_checkpoint_candidate(checkpoint);
        arena.accept_checkpoint_candidate(tail);
        assert!(arena.is_empty());
        assert_eq!(generation.memory_accounting().words(false), baseline);
    });
}

#[test]
fn restore_discards_local_rows_and_an_active_build() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let checkpoint = arena.cursor();
        let local = direct_definition(arena, DefinitionDestination::Local, &[], &[letter('l')]);
        let body = arena.admit_macro_body(local).expect("body");
        let build = arena
            .begin_build(
                DefinitionDestination::Local,
                crate::token::OriginId::UNKNOWN,
            )
            .expect("build");
        arena.restore_cursor(checkpoint);
        assert!(arena.is_empty());
        assert_eq!(
            arena.finish_parameters(build),
            Err(DefinitionBuildError::InvalidPhase)
        );

        assert_eq!(body.word(0), Some(letter('l')));
    });
}

#[test]
fn complete_rows_resolve_by_direct_id() {
    with_generation(|mut generation| {
        let parameter = [
            TokenWord::pack(Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            }),
            TokenWord::pack(Token::param(1)),
            TokenWord::pack(Token::Char {
                ch: b'x'.into(),
                cat: Catcode::Letter,
            }),
        ];
        let replacement = [TokenWord::pack(Token::param(1))];
        let id = generation
            .definitions_mut()
            .allocate(&parameter, &replacement)
            .expect("test fixture is valid");

        let view = generation.definitions().get(id);
        assert_eq!(view.parameter_text(), parameter);
        assert_eq!(view.replacement_text(), replacement);
        assert_eq!(view.parameter_pattern().parameter_count(), 1);
        assert_eq!(view.parameter_pattern().marker_index(0), Some(0));
    });
}

#[test]
fn content_identity_survives_allocation_order_and_local_promotion() {
    with_generation(|mut generation| {
        assert!(generation.enable_semantic_identity());
        let arena = generation.definitions_mut();
        let body = [TokenWord::pack(Token::frozen_relax())];
        let local = direct_definition(arena, super::DefinitionDestination::Local, &[], &body);
        let local_identity = arena
            .get(local)
            .semantic_identity()
            .expect("semantic identity enabled");

        let _unrelated = direct_definition(
            arena,
            super::DefinitionDestination::Global,
            &[],
            &[TokenWord::pack(Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            })],
        );
        let equivalent = direct_definition(arena, super::DefinitionDestination::Global, &[], &body);
        let promoted = arena.promote_global(local).expect("local promotion");

        assert_eq!(
            arena.get(equivalent).semantic_identity(),
            Some(local_identity)
        );
        assert_eq!(
            arena.get(promoted).semantic_identity(),
            Some(local_identity)
        );
        assert_ne!(
            local, promoted,
            "promotion still changes storage coordinate"
        );
    });
}

#[test]
fn equal_definitions_receive_distinct_ids() {
    with_generation(|mut generation| {
        let text = [TokenWord::pack(Token::frozen_relax())];
        let first = generation
            .definitions_mut()
            .allocate(&[], &text)
            .expect("test fixture is valid");
        let second = generation
            .definitions_mut()
            .allocate(&[], &text)
            .expect("test fixture is valid");

        assert_ne!(first, second);
        assert_eq!(generation.definitions().len(), 2);
        assert_eq!(generation.definitions().get(first).replacement_text(), text);
        assert_eq!(
            generation.definitions().get(second).replacement_text(),
            text
        );
    });
}

#[test]
fn definition_region_truncation_releases_complete_suffix() {
    with_generation(|mut generation| {
        let baseline = generation.memory_accounting().words(false);
        let cursor = generation.definitions().cursor();
        let id = generation
            .definitions_mut()
            .allocate(&[], &[TokenWord::pack(Token::frozen_relax())])
            .expect("published definition");
        assert_eq!(
            generation.definitions().get(id).replacement_text(),
            [TokenWord::pack(Token::frozen_relax())]
        );
        generation.definitions_mut().restore_cursor(cursor);
        assert_eq!(generation.memory_accounting().words(false), baseline);
    });
}

#[test]
fn one_way_builder_transfer_prevents_cross_generation_aliasing() {
    let word = TokenWord::pack(Token::frozen_relax());
    let mut builder = checked_builder(&[], &[word]);
    with_generation(|mut first_generation| {
        let first_baseline = first_generation.memory_accounting().words(false);
        let first_cursor = first_generation.definitions().cursor();
        let first = first_generation
            .definitions_mut()
            .publish(&mut builder)
            .expect("first publication transfers the builder allocation");
        assert_ne!(
            first_generation.memory_accounting().words(false),
            first_baseline
        );

        with_generation(|mut second_generation| {
            let second_baseline = second_generation.memory_accounting().words(false);
            let empty_second_cursor = second_generation.definitions().cursor();
            let second_cursor = second_generation.definitions().cursor();
            assert_eq!(
                second_generation.definitions_mut().publish(&mut builder),
                Err(DefinitionAllocationError::InvalidDefinition),
                "the transferred allocation cannot be republished in another generation"
            );
            assert_eq!(second_generation.definitions().cursor(), second_cursor);
            assert_eq!(
                second_generation.memory_accounting().words(false),
                second_baseline
            );

            builder.reset();
            builder.finish_parameters().expect("empty parameter text");
            builder.seal().expect("empty replacement text");
            let second = second_generation
                .definitions_mut()
                .publish(&mut builder)
                .expect("reset builder owns a new allocation for the second generation");
            assert!(
                second_generation
                    .definitions()
                    .get(second)
                    .replacement_text()
                    .is_empty()
            );
            second_generation
                .definitions_mut()
                .restore_cursor(empty_second_cursor);
            assert_eq!(
                second_generation.memory_accounting().words(false),
                second_baseline
            );
        });

        assert_eq!(
            first_generation.definitions().get(first).replacement_text(),
            [word]
        );
        first_generation
            .definitions_mut()
            .restore_cursor(first_cursor);
        assert_eq!(
            first_generation.memory_accounting().words(false),
            first_baseline
        );
    });
}

#[test]
fn invalid_parameter_program_does_not_publish_a_partial_row() {
    with_generation(|mut generation| {
        let too_many = std::array::from_fn::<_, 10, _>(|index| {
            TokenWord::pack(Token::param((index + 1).min(9) as u8))
        });
        let cursor = generation.definitions().cursor();
        let accounting = generation.memory_accounting().words(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            generation.definitions_mut().allocate(&too_many, &[])
        }));

        assert_eq!(
            result.expect("malformed definition is an ordinary error"),
            Err(DefinitionAllocationError::InvalidDefinition)
        );
        assert!(generation.definitions().is_empty());
        assert_eq!(generation.definitions().cursor(), cursor);
        assert_eq!(generation.memory_accounting().words(false), accounting);

        let valid = generation
            .definitions_mut()
            .allocate(&[], &[])
            .expect("test fixture is valid");
        assert!(
            generation
                .definitions()
                .get(valid)
                .replacement_text()
                .is_empty()
        );
    });
}

#[test]
fn checked_builder_accepts_custom_markers_and_all_nine_parameters() {
    with_generation(|mut generation| {
        let mut parameters = Vec::new();
        for slot in 1..=9 {
            parameters.push(TokenWord::pack(Token::Char {
                ch: '@',
                cat: Catcode::Parameter,
            }));
            parameters.push(TokenWord::pack(Token::param(slot)));
        }
        let replacement = [TokenWord::pack(Token::param(9))];
        let mut builder = checked_builder(&parameters, &replacement);
        let definition = generation
            .definitions_mut()
            .publish(&mut builder)
            .expect("publish checked program");
        let view = generation.definitions().get(definition);
        let reference = crate::macro_definition::MacroParameterPattern::from_words(&parameters)
            .expect("independent one-shot parameter program");
        assert_eq!(view.parameter_pattern(), &reference);
        assert_eq!(view.parameter_pattern().parameter_count(), 9);
        assert_eq!(view.parameter_pattern().marker_index(0), Some(0));
        assert_eq!(view.parameter_pattern().marker_index(8), Some(16));
        assert_eq!(view.replacement_text(), replacement);
    });
}

#[test]
fn builder_checks_monotonic_phases_and_replacement_references() {
    let mut builder = DefinitionBuilder::new();
    assert_eq!(builder.phase(), DefinitionBuildPhase::OpenParameters);
    builder
        .push_parameter(TokenWord::pack(Token::param(1)))
        .expect("first parameter");
    assert_eq!(
        builder.push_parameter(TokenWord::pack(Token::param(3))),
        Err(DefinitionBuildError::InvalidProgram(
            crate::macro_definition::MacroParameterProgramError::NonSequentialParameter {
                expected: 2,
                found: 3,
            }
        ))
    );
    assert_eq!(builder.parameter_text().len(), 1);
    builder.finish_parameters().expect("parameter boundary");
    assert_eq!(
        builder.push_replacement(TokenWord::pack(Token::param(2))),
        Err(DefinitionBuildError::InvalidProgram(
            crate::macro_definition::MacroParameterProgramError::InvalidReplacementParameter {
                highest: 1,
                found: 2,
            }
        ))
    );
    assert!(builder.replacement_text().is_empty());
    builder
        .push_replacement(TokenWord::pack(Token::param(1)))
        .expect("declared replacement reference");
    builder.seal().expect("seal");
    assert_eq!(builder.phase(), DefinitionBuildPhase::Sealed);
    assert_eq!(builder.seal(), Err(DefinitionBuildError::InvalidPhase));
}

#[test]
fn zero_parameter_builder_has_one_monotonic_boundary() {
    let mut builder = DefinitionBuilder::new();
    builder.finish_parameters().expect("empty parameter text");
    builder
        .push_replacement(TokenWord::pack(Token::frozen_relax()))
        .expect("replacement word");
    builder.seal().expect("sealed definition");
    assert!(builder.parameter_text().is_empty());
    assert_eq!(builder.replacement_text().len(), 1);
    assert_eq!(builder.phase(), DefinitionBuildPhase::Sealed);
}

#[test]
fn injected_reserve_failure_preserves_validated_contents_and_reusable_capacity() {
    let mut builder = DefinitionBuilder::new();
    let phase = builder.phase();
    let capacity = builder.capacity();
    builder.force_next_reserve_failure();
    assert_eq!(
        builder.push_parameter(TokenWord::pack(Token::param(1))),
        Err(DefinitionBuildError::AllocationFailed)
    );
    assert_eq!(builder.phase(), phase);
    assert_eq!(builder.capacity(), capacity);
    assert!(builder.words().is_empty());
    builder
        .push_parameter(TokenWord::pack(Token::param(1)))
        .expect("failed row remains reusable");
    builder.finish_parameters().expect("parameter boundary");
    builder
        .push_replacement(TokenWord::pack(Token::param(1)))
        .expect("replacement reference");
    builder.seal().expect("sealed row");

    let mut reference = checked_builder(
        &[TokenWord::pack(Token::param(1))],
        &[TokenWord::pack(Token::param(1))],
    );
    with_generation(|mut generation| {
        let after_failure = generation
            .definitions_mut()
            .publish(&mut builder)
            .expect("row after reserve failure");
        let reference = generation
            .definitions_mut()
            .publish(&mut reference)
            .expect("reference row");
        assert!(
            generation
                .definitions()
                .contents_equal(after_failure, reference)
        );
        assert_eq!(
            generation
                .definitions()
                .get(after_failure)
                .parameter_pattern(),
            generation.definitions().get(reference).parameter_pattern()
        );
    });
}

#[test]
fn distinct_refs_compare_validated_contents_and_boundaries_lazily() {
    with_generation(|mut generation| {
        let a = TokenWord::pack(Token::frozen_relax());
        let b = TokenWord::pack(Token::Char {
            ch: 'b',
            cat: Catcode::Letter,
        });
        let ordinary = generation
            .definitions_mut()
            .allocate(&[a], &[b])
            .expect("ordinary allocation");

        let mut streamed = DefinitionBuilder::new();
        streamed.push_parameter(a).expect("parameter");
        streamed.finish_parameters().expect("boundary");
        streamed.push_replacement(b).expect("replacement");
        streamed.seal().expect("seal");
        let streamed = generation
            .definitions_mut()
            .publish(&mut streamed)
            .expect("streamed publication");
        assert_ne!(ordinary, streamed);
        assert!(generation.definitions().contents_equal(ordinary, ordinary));
        assert!(generation.definitions().contents_equal(ordinary, streamed));

        let differently_framed = generation
            .definitions_mut()
            .allocate(&[a, b], &[])
            .expect("different framing");
        assert!(
            !generation
                .definitions()
                .contents_equal(ordinary, differently_framed)
        );

        let different_replacement = generation
            .definitions_mut()
            .allocate(&[a], &[a])
            .expect("different replacement");
        assert!(
            !generation
                .definitions()
                .contents_equal(ordinary, different_replacement)
        );
    });
}

#[test]
fn sealed_matcher_survives_promotion_group_exit_and_format_materialization() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let parameters = [
            letter('p'),
            TokenWord::pack(Token::param(1)),
            letter('a'),
            letter('b'),
            letter('a'),
            letter('b'),
        ];
        let frontier_10 = arena.local_frontier().expect("group");
        let local = direct_definition(
            arena,
            DefinitionDestination::Local,
            &parameters,
            &[TokenWord::pack(Token::param(1))],
        );
        let resident = arena.admit_macro_body(local).expect("resident definition");
        let promoted = arena.promote_global(local).expect("promotion");
        let wire = arena.get(promoted).capture_format();
        let imported = arena
            .publish_format_row(&wire)
            .expect("format materialization");
        let allocated = arena
            .allocate(&parameters, &[TokenWord::pack(Token::param(1))])
            .expect("detached builder publication");
        arena.retire_local_suffix(frontier_10);
        assert_eq!(
            resident.parameter_pattern(),
            arena.get(promoted).parameter_pattern()
        );
        assert_eq!(
            resident.parameter_pattern(),
            arena.get(imported).parameter_pattern()
        );
        assert_eq!(
            resident.parameter_pattern(),
            arena.get(allocated).parameter_pattern()
        );
        let delimiter = resident
            .parameter_pattern()
            .delimiter(0, resident.parameter_words());
        assert_eq!(delimiter.words(), &parameters[2..]);
        assert_eq!(delimiter.overlap(3, letter('b')), 3);
        assert_eq!(delimiter.overlap(3, letter('a')), 1);
    });
}
