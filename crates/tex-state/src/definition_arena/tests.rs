use super::{
    AdmittedMacroDefinition, DefinitionAllocationError, DefinitionBuildError, DefinitionBuildPhase,
    DefinitionBuilder,
};
use crate::generation::with_generation;
use crate::token::{Catcode, Token, TokenWord};

fn direct_definition<G>(
    arena: &mut super::DefinitionArena<G>,
    parameter: &[TokenWord],
    replacement: &[TokenWord],
) -> super::PublishedDefinition<G> {
    let build = arena
        .begin_build(crate::token::OriginId::UNKNOWN)
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
fn admitted_writer_round_trips_the_build_lane_without_copying() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let build = arena
            .begin_build(crate::token::OriginId::UNKNOWN)
            .expect("definition transaction");
        let mut writer = arena
            .admit_build_writer(build, DefinitionBuildPhase::OpenParameters)
            .expect("writer admission");
        writer
            .push_parameter(TokenWord::pack(Token::param(1)))
            .expect("parameter");
        writer.finish_parameters().expect("parameter boundary");
        assert_eq!(
            arena.seal_build(build).err(),
            Some(DefinitionBuildError::InvalidPhase),
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
        let definition_publication = definition;
        let definition = definition_publication.reference();
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
            .begin_build(crate::token::OriginId::UNKNOWN)
            .expect("definition transaction");
        arena.finish_parameters(build).expect("parameter boundary");
        arena.push_replacement(build, letter('x')).expect("word");
        arena.abort_build(build);
        assert!(arena.is_empty());
        let next = direct_definition(arena, &[], &[letter('y')]);
        let next_publication = next;
        let next = next_publication.reference();
        assert_eq!(arena.get(next).replacement_text(), [letter('y')]);
        drop(next_publication);
        assert_eq!(generation.memory_accounting().words(false), baseline);
    });
}

#[test]
fn admitted_macro_definition_classifies_simple_matching_and_empty() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let empty = direct_definition(arena, &[], &[]);
        let empty_publication = empty;
        let empty = empty_publication.reference();
        let simple = direct_definition(arena, &[], &[letter('x')]);
        let simple_publication = simple;
        let simple = simple_publication.reference();
        let matching = direct_definition(
            arena,
            &[TokenWord::pack(Token::param(1))],
            &[TokenWord::pack(Token::param(1))],
        );
        let matching_publication = matching;
        let matching = matching_publication.reference();
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
        let id_publication = id;
        let id = id_publication.reference();

        let view = generation.definitions().get(id);
        assert_eq!(view.parameter_text(), parameter);
        assert_eq!(view.replacement_text(), replacement);
        assert_eq!(view.parameter_pattern().parameter_count(), 1);
        assert_eq!(view.parameter_pattern().marker_index(0), Some(0));
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
        let first_publication = first;
        let first = first_publication.reference();
        let second = generation
            .definitions_mut()
            .allocate(&[], &text)
            .expect("test fixture is valid");
        let second_publication = second;
        let second = second_publication.reference();

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
fn sealed_builder_moves_between_generations_before_owning_publication() {
    let parameter = [TokenWord::pack(Token::param(1)), letter('x')];
    let replacement = [TokenWord::pack(Token::param(1)), letter('y')];
    let builder = with_generation(|_| checked_builder(&parameter, &replacement));
    with_generation(|mut generation| {
        let mut builder = builder;
        let publication = generation
            .definitions_mut()
            .publish(&mut builder)
            .expect("publication in receiving generation");
        let reference = publication.reference();
        let view = generation.definitions().get(reference);
        assert_eq!(view.parameter_text(), parameter);
        assert_eq!(view.replacement_text(), replacement);
        assert_eq!(builder.phase(), DefinitionBuildPhase::Published);
        drop(publication);
        assert!(
            generation
                .definitions()
                .admit_macro_body(reference)
                .is_none()
        );
        assert_eq!(view.replacement_text(), replacement);
    });
}

#[test]
fn invalid_parameter_program_does_not_publish_a_partial_row() {
    with_generation(|mut generation| {
        let too_many = std::array::from_fn::<_, 10, _>(|index| {
            TokenWord::pack(Token::param((index + 1).min(9) as u8))
        });
        let cursor = generation.definitions().len();
        let accounting = generation.memory_accounting().words(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            generation.definitions_mut().allocate(&too_many, &[])
        }));

        assert_eq!(
            result
                .expect("malformed definition is an ordinary error")
                .err(),
            Some(DefinitionAllocationError::InvalidDefinition)
        );
        assert!(generation.definitions().is_empty());
        assert_eq!(generation.definitions().len(), cursor);
        assert_eq!(generation.memory_accounting().words(false), accounting);

        let valid = generation
            .definitions_mut()
            .allocate(&[], &[])
            .expect("test fixture is valid");
        let valid_publication = valid;
        let valid = valid_publication.reference();
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
        let definition_publication = definition;
        let definition = definition_publication.reference();
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
        let after_failure_publication = after_failure;
        let after_failure = after_failure_publication.reference();
        let reference = generation
            .definitions_mut()
            .publish(&mut reference)
            .expect("reference row");
        let reference_publication = reference;
        let reference = reference_publication.reference();
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
        let ordinary_publication = ordinary;
        let ordinary = ordinary_publication.reference();

        let mut streamed = DefinitionBuilder::new();
        streamed.push_parameter(a).expect("parameter");
        streamed.finish_parameters().expect("boundary");
        streamed.push_replacement(b).expect("replacement");
        streamed.seal().expect("seal");
        let streamed = generation
            .definitions_mut()
            .publish(&mut streamed)
            .expect("streamed publication");
        let streamed_publication = streamed;
        let streamed = streamed_publication.reference();
        assert_ne!(ordinary, streamed);
        assert!(generation.definitions().contents_equal(ordinary, ordinary));
        assert!(generation.definitions().contents_equal(ordinary, streamed));

        let differently_framed = generation
            .definitions_mut()
            .allocate(&[a, b], &[])
            .expect("different framing");
        let differently_framed_publication = differently_framed;
        let differently_framed = differently_framed_publication.reference();
        assert!(
            !generation
                .definitions()
                .contents_equal(ordinary, differently_framed)
        );

        let different_replacement = generation
            .definitions_mut()
            .allocate(&[a], &[a])
            .expect("different replacement");
        let different_replacement_publication = different_replacement;
        let different_replacement = different_replacement_publication.reference();
        assert!(
            !generation
                .definitions()
                .contents_equal(ordinary, different_replacement)
        );
    });
}
