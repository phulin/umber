use super::{
    DefinitionAllocationError, DefinitionBuildError, DefinitionBuildPhase, DefinitionBuilder,
    DefinitionIdentityPolicy,
};
use crate::generation::with_generation;
use crate::token::{Catcode, Token, TokenWord};

fn direct_definition<G>(
    arena: &mut super::DefinitionArena<G>,
    destination: super::DefinitionDestination,
    parameter: &[TokenWord],
    replacement: &[TokenWord],
) -> super::DefinitionId<G> {
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
fn definition_key_fits_the_coordinated_compact_boundary() {
    assert!(std::mem::size_of::<super::DefinitionId<()>>() <= 16);
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
fn direct_definition_seals_the_transactional_destination_without_a_body_copy() {
    with_generation(|mut generation| {
        let word = TokenWord::pack(Token::frozen_relax());
        let arena = generation.definitions_mut();
        let build = arena
            .begin_build(
                super::DefinitionDestination::Global,
                crate::token::OriginId::UNKNOWN,
            )
            .expect("definition transaction");
        arena.finish_parameters(build).expect("parameter boundary");
        arena
            .push_replacement(build, word)
            .expect("replacement word");
        let before = arena.global.words.as_ptr();
        let definition = arena.seal_build(build).expect("sealed definition");
        assert_eq!(arena.global.words.as_ptr(), before);
        assert_eq!(arena.get(definition).replacement_text(), [word]);
    });
}

#[test]
fn local_region_retires_only_after_its_last_live_command_lease_drains() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("local definition group");
        let local = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let lease = arena.lease(local);
        arena.end_group();
        assert_eq!(arena.get(local).replacement_text().len(), 1);
        drop(lease);
        assert!(
            arena.locals[(local.region() - 3) as usize]
                .data
                .borrow()
                .is_none()
        );
    });
}

#[test]
fn local_region_retires_only_after_its_checkpoint_lease_drains() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("local definition group");
        let local = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let checkpoint = arena.checkpoint_lease();
        arena.end_group();
        assert_eq!(arena.get(local).replacement_text().len(), 1);
        drop(checkpoint);
        assert!(
            arena.locals[(local.region() - 3) as usize]
                .data
                .borrow()
                .is_none()
        );
    });
}

#[test]
fn local_to_global_promotion_copies_once_and_reuses_the_global_key() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("local definition group");
        let local = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let first = arena.promote_global(local).expect("first promotion");
        let second = arena.promote_global(local).expect("reused promotion");
        assert_eq!(first, second);
        assert_eq!(first.region(), super::GLOBAL_REGION);
        assert_eq!(arena.global.headers.len(), 1);
        let before = arena.retirement_counters();
        arena.end_group();
        let after = arena.retirement_counters();
        assert_eq!(after.promotions_reclaimed - before.promotions_reclaimed, 1);
        assert_eq!(arena.get(first).replacement_text().len(), 1);
    });
}

#[test]
fn group_region_work_is_constant_for_sequential_and_nested_depths() {
    for groups in [1_u64, 64, 4_096] {
        with_generation(|mut generation| {
            let arena = generation.definitions_mut();
            let before = arena.retirement_counters();
            for _ in 0..groups {
                arena.begin_group().expect("sequential definition group");
                arena.end_group();
            }
            let after_sequential = arena.retirement_counters();
            assert_eq!(
                after_sequential.group_region_inspections - before.group_region_inspections,
                groups
            );
            assert_eq!(
                after_sequential.regions_reclaimed - before.regions_reclaimed,
                groups
            );

            let before_history_probe = arena.retirement_counters();
            arena.begin_group().expect("group after retired history");
            arena.end_group();
            let after_history_probe = arena.retirement_counters();
            assert_eq!(
                after_history_probe.group_region_inspections
                    - before_history_probe.group_region_inspections,
                1,
                "one group inspects only its own region after {groups} retired regions"
            );
            assert_eq!(
                after_history_probe.regions_reclaimed - before_history_probe.regions_reclaimed,
                1
            );
        });

        with_generation(|mut generation| {
            let arena = generation.definitions_mut();
            let before = arena.retirement_counters();
            for _ in 0..groups {
                arena.begin_group().expect("nested definition group");
            }
            for _ in 0..groups {
                arena.end_group();
            }
            let after = arena.retirement_counters();
            assert_eq!(
                after.group_region_inspections - before.group_region_inspections,
                groups
            );
            assert_eq!(after.regions_reclaimed - before.regions_reclaimed, groups);
        });
    }
}

#[test]
fn final_lease_release_reclaims_only_its_exact_retired_region() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let mut leases = Vec::new();
        let mut definitions = Vec::new();
        for _ in 0..4_096 {
            arena.begin_group().expect("leased definition group");
            let definition = direct_definition(
                arena,
                super::DefinitionDestination::Local,
                &[],
                &[TokenWord::pack(Token::frozen_relax())],
            );
            definitions.push(definition);
            leases.push(arena.lease(definition));
            arena.end_group();
        }

        let selected = 2_048;
        let selected_definition = definitions[selected];
        let neighbor = definitions[selected - 1];
        let before = arena.retirement_counters();
        drop(leases.swap_remove(selected));
        let after = arena.retirement_counters();
        assert_eq!(
            after.lease_release_region_inspections - before.lease_release_region_inspections,
            1
        );
        assert_eq!(after.regions_reclaimed - before.regions_reclaimed, 1);
        assert_eq!(after.rows_reclaimed - before.rows_reclaimed, 1);
        assert!(
            arena.locals[(selected_definition.region() - 3) as usize]
                .data
                .borrow()
                .is_none()
        );
        assert_eq!(arena.get(neighbor).replacement_text().len(), 1);
    });
}

#[test]
fn retired_region_release_work_tracks_only_its_own_rows() {
    for rows in [1_u64, 64, 4_096] {
        with_generation(|mut generation| {
            let arena = generation.definitions_mut();
            arena.begin_group().expect("row-scaled definition group");
            let first = direct_definition(
                arena,
                super::DefinitionDestination::Local,
                &[],
                &[TokenWord::pack(Token::frozen_relax())],
            );
            for _ in 1..rows {
                direct_definition(
                    arena,
                    super::DefinitionDestination::Local,
                    &[],
                    &[TokenWord::pack(Token::frozen_relax())],
                );
            }
            let lease = arena.lease(first);
            arena.end_group();
            let before = arena.retirement_counters();
            drop(lease);
            let after = arena.retirement_counters();
            assert_eq!(
                after.lease_release_region_inspections - before.lease_release_region_inspections,
                1
            );
            assert_eq!(after.regions_reclaimed - before.regions_reclaimed, 1);
            assert_eq!(after.rows_reclaimed - before.rows_reclaimed, rows);
        });
    }
}

#[test]
fn checkpoint_leases_inspect_only_the_explicit_active_region_stack() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        for _ in 0..4_096 {
            arena.begin_group().expect("retired history group");
            arena.end_group();
        }
        for _ in 0..64 {
            arena.begin_group().expect("active checkpoint group");
        }
        let before = arena.retirement_counters();
        let checkpoint = arena.checkpoint_lease();
        let after_capture = arena.retirement_counters();
        assert_eq!(
            after_capture.checkpoint_region_inspections - before.checkpoint_region_inspections,
            64
        );
        drop(checkpoint);
        let after_release = arena.retirement_counters();
        assert_eq!(
            after_release.lease_release_region_inspections
                - after_capture.lease_release_region_inspections,
            64
        );
    });
}

#[test]
fn checkpoint_rejection_restores_detached_global_and_local_definition_suffixes() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("local definition group");
        let root = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let checkpoint = arena.cursor();
        let accepted = direct_definition(
            arena,
            super::DefinitionDestination::Global,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let tail = arena.begin_checkpoint_candidate(checkpoint);
        let candidate = direct_definition(arena, super::DefinitionDestination::Global, &[], &[]);
        assert!(arena.get(candidate).replacement_text().is_empty());
        arena.reject_checkpoint_candidate(checkpoint, tail);
        assert_eq!(arena.get(root).replacement_text().len(), 1);
        assert_eq!(arena.get(accepted).replacement_text().len(), 1);
    });
}

#[test]
fn checkpoint_rejection_discards_only_candidate_promotion_mappings() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("outer local group");
        let source = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        arena.begin_group().expect("checkpoint child group");
        let checkpoint = arena.cursor();
        let tail = arena.begin_checkpoint_candidate(checkpoint);
        let rejected = arena.promote_global(source).expect("candidate promotion");
        assert_eq!(arena.get(rejected).replacement_text().len(), 1);
        arena.reject_checkpoint_candidate(checkpoint, tail);

        let replacement = arena.promote_global(source).expect("replacement promotion");
        assert_eq!(replacement.format_index(), 0);
        assert_eq!(arena.get(replacement).replacement_text().len(), 1);
    });
}

#[test]
fn checkpoint_acceptance_discards_only_detached_prior_promotion_mappings() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("outer local group");
        let source = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        arena.begin_group().expect("checkpoint child group");
        let checkpoint = arena.cursor();
        let prior = arena.promote_global(source).expect("prior promotion");
        let tail = arena.begin_checkpoint_candidate(checkpoint);
        let candidate = arena.promote_global(source).expect("candidate promotion");
        assert_eq!(candidate.format_index(), prior.format_index());
        arena.accept_checkpoint_candidate(tail);
        assert_eq!(
            arena.promote_global(source).expect("reused candidate"),
            candidate
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
    let mut builder = checked_builder(DefinitionIdentityPolicy::Disabled, &[], &[word]);
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

            builder.reset(DefinitionIdentityPolicy::Disabled);
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
            generation
                .definitions()
                .get(after_failure)
                .semantic_identity(),
            generation.definitions().get(reference).semantic_identity()
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
        assert_eq!(
            generation.definitions().get(ordinary).semantic_identity(),
            generation.definitions().get(streamed).semantic_identity()
        );

        let differently_framed = generation
            .definitions_mut()
            .allocate(&[a, b], &[])
            .expect("different framing");
        assert_ne!(
            generation.definitions().get(ordinary).semantic_identity(),
            generation
                .definitions()
                .get(differently_framed)
                .semantic_identity()
        );
        assert_eq!(
            generation.definitions().get(ordinary).semantic_identity(),
            Some(10_092_552_631_538_213_390),
            "definition identity v2 known vector"
        );
    });
}
