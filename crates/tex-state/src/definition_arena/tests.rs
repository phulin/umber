use super::{
    DefinitionAllocationError, DefinitionBuildError, DefinitionBuildPhase, DefinitionBuilder,
    DefinitionIdentityPolicy,
};
use crate::generation::with_generation;
use crate::token::{Catcode, Token, TokenWord};
use std::rc::Rc;

fn checked_builder(
    policy: DefinitionIdentityPolicy,
    parameter_text: &[TokenWord],
    replacement_text: &[TokenWord],
) -> DefinitionBuilder {
    let mut builder = DefinitionBuilder::new(policy);
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

#[test]
fn definition_handle_is_one_pointer_sized_non_atomic_owner() {
    assert_eq!(
        std::mem::size_of::<super::DefinitionId<()>>(),
        std::mem::size_of::<usize>()
    );
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

        let owners = id.semantic_owner_count();
        assert_eq!(id.parameter_text(), parameter);
        assert_eq!(id.replacement_text(), replacement);
        assert_eq!(id.semantic_owner_count(), owners);

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
fn definition_aliases_release_exactly_on_owner_drop() {
    with_generation(|mut generation| {
        let baseline = generation.memory_accounting().words(false);
        let id = generation
            .definitions_mut()
            .allocate(&[], &[TokenWord::pack(Token::frozen_relax())])
            .expect("published definition");
        assert_eq!(id.semantic_owner_count(), 1);

        let alias = id.clone();
        assert_eq!(id.semantic_owner_count(), 2);
        let view = generation.definitions().get(alias);
        assert_eq!(id.semantic_owner_count(), 2);
        drop(view);
        assert_eq!(id.semantic_owner_count(), 1);
        drop(id);
        assert_eq!(generation.memory_accounting().words(false), baseline);
    });
}

#[test]
fn one_way_builder_transfer_prevents_cross_generation_aliasing() {
    let word = TokenWord::pack(Token::frozen_relax());
    let mut builder = checked_builder(DefinitionIdentityPolicy::Disabled, &[], &[word]);
    with_generation(|mut first_generation| {
        let first_baseline = first_generation.memory_accounting().words(false);
        let first = first_generation
            .definitions_mut()
            .publish(&mut builder)
            .expect("first publication transfers the builder allocation");
        assert_eq!(first.semantic_owner_count(), 1);
        assert_ne!(
            first_generation.memory_accounting().words(false),
            first_baseline
        );

        with_generation(|mut second_generation| {
            let second_baseline = second_generation.memory_accounting().words(false);
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

            builder.reset(DefinitionIdentityPolicy::Disabled);
            builder.finish_parameters().expect("empty parameter text");
            builder.seal().expect("empty replacement text");
            let second = second_generation
                .definitions_mut()
                .publish(&mut builder)
                .expect("reset builder owns a new allocation for the second generation");
            assert_eq!(second.semantic_owner_count(), 1);
            assert!(
                !Rc::ptr_eq(&first.data, &second.data),
                "a reset builder must not retain the prior generation's allocation"
            );
            assert!(second.replacement_text().is_empty());
            drop(second);
            assert_eq!(
                second_generation.memory_accounting().words(false),
                second_baseline
            );
        });

        assert_eq!(first.replacement_text(), [word]);
        drop(first);
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
        let mut builder = checked_builder(
            DefinitionIdentityPolicy::Disabled,
            &parameters,
            &replacement,
        );
        let definition = generation
            .definitions_mut()
            .publish(&mut builder)
            .expect("publish checked program");
        let view = generation.definitions().get(definition);
        let reference = crate::macro_definition::MacroParameterPattern::from_words(&parameters)
            .expect("independent one-shot parameter program");
        assert_eq!(view.parameter_pattern(), reference);
        assert_eq!(view.parameter_pattern().parameter_count(), 9);
        assert_eq!(view.parameter_pattern().marker_index(0), Some(0));
        assert_eq!(view.parameter_pattern().marker_index(8), Some(16));
        assert_eq!(view.replacement_text(), replacement);
    });
}

#[test]
fn builder_checks_monotonic_phases_and_replacement_references() {
    let mut builder = DefinitionBuilder::new(DefinitionIdentityPolicy::Disabled);
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
    let mut builder = DefinitionBuilder::new(DefinitionIdentityPolicy::Disabled);
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
fn injected_reserve_failure_preserves_metadata_identity_and_reusable_capacity() {
    let mut builder = DefinitionBuilder::new(DefinitionIdentityPolicy::Enabled);
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
        DefinitionIdentityPolicy::Enabled,
        &[TokenWord::pack(Token::param(1))],
        &[TokenWord::pack(Token::param(1))],
    );
    with_generation(|mut generation| {
        assert!(generation.enable_semantic_identity());
        let after_failure = generation
            .definitions_mut()
            .publish(&mut builder)
            .expect("row after reserve failure");
        let reference = generation
            .definitions_mut()
            .publish(&mut reference)
            .expect("reference row");
        assert_eq!(
            after_failure.semantic_identity(),
            reference.semantic_identity()
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
fn destination_policy_mismatch_changes_no_serial_or_accounting() {
    with_generation(|mut generation| {
        let mut builder = DefinitionBuilder::new(DefinitionIdentityPolicy::Enabled);
        builder.finish_parameters().expect("empty parameter text");
        builder.seal().expect("empty replacement text");
        let cursor = generation.definitions().cursor();
        let accounting = generation.memory_accounting().words(false);
        assert_eq!(
            generation.definitions_mut().publish(&mut builder),
            Err(DefinitionAllocationError::IdentityPolicyMismatch)
        );
        assert_eq!(generation.definitions().cursor(), cursor);
        assert_eq!(generation.memory_accounting().words(false), accounting);
    });
}

#[test]
fn v2_identity_is_shared_by_streaming_and_ordinary_paths_and_frames_boundaries() {
    with_generation(|mut generation| {
        assert!(generation.enable_semantic_identity());
        let a = TokenWord::pack(Token::frozen_relax());
        let b = TokenWord::pack(Token::Char {
            ch: 'b',
            cat: Catcode::Letter,
        });
        let ordinary = generation
            .definitions_mut()
            .allocate(&[a], &[b])
            .expect("ordinary allocation");

        let mut streamed = DefinitionBuilder::new(DefinitionIdentityPolicy::Enabled);
        streamed.push_parameter(a).expect("parameter");
        streamed.finish_parameters().expect("boundary");
        streamed.push_replacement(b).expect("replacement");
        streamed.seal().expect("seal");
        let streamed = generation
            .definitions_mut()
            .publish(&mut streamed)
            .expect("streamed publication");
        assert_eq!(ordinary.semantic_identity(), streamed.semantic_identity());

        let differently_framed = generation
            .definitions_mut()
            .allocate(&[a, b], &[])
            .expect("different framing");
        assert_ne!(
            ordinary.semantic_identity(),
            differently_framed.semantic_identity()
        );
        assert_eq!(
            ordinary.semantic_identity(),
            Some(10_092_552_631_538_213_390),
            "definition identity v2 known vector"
        );
    });
}
