use super::{
    DefinitionAllocationError, DefinitionBuildError, DefinitionBuildPhase, DefinitionBuilder,
};
use crate::generation::with_generation;
use crate::token::{Catcode, Token, TokenWord};

fn direct_definition<G>(
    arena: &mut super::DefinitionArena<G>,
    destination: super::DefinitionDestination,
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

fn origin(raw: u32) -> crate::token::OriginId {
    crate::token::OriginId::from_raw(raw)
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

#[test]
fn definition_key_fits_the_coordinated_compact_boundary() {
    assert_eq!(std::mem::size_of::<super::DefinitionRef<()>>(), 8);
    assert_eq!(
        std::mem::size_of::<super::DefinitionCheckpointLease<()>>(),
        std::mem::size_of::<super::LocalRegionPin<()>>(),
        "one current-region checkpoint pin has no stale multi-region container"
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
        let before = std::rc::Rc::as_ptr(
            arena
                .global
                .owner
                .as_ref()
                .expect("word push creates owner"),
        );
        let definition = arena.seal_build(build).expect("sealed definition");
        assert_eq!(
            std::rc::Rc::as_ptr(arena.global.owner.as_ref().expect("sealed owner")),
            before
        );
        assert_eq!(arena.get(definition).replacement_text(), [word]);
    });
}

#[test]
fn small_definition_stays_in_its_region_inline_prefix() {
    with_generation(|mut generation| {
        let word = TokenWord::pack(Token::frozen_relax());
        let arena = generation.definitions_mut();
        arena.begin_group().expect("local definition group");
        let definition = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[word; super::INLINE_DEFINITION_WORD_CAPACITY],
        );
        let region = arena.region(definition.region()).expect("local region");
        assert!(
            region
                .owner
                .as_ref()
                .expect("definition words own storage")
                .overflow_words
                .borrow()
                .is_empty(),
            "the bounded common case does not allocate a 4,096-word overflow block"
        );
        assert_eq!(
            arena.get(definition).replacement_text().len(),
            super::INLINE_DEFINITION_WORD_CAPACITY
        );
        drop(region);
        arena.end_group();
    });
}

#[test]
fn definition_overflow_keeps_direct_stable_chunk_reads() {
    with_generation(|mut generation| {
        let replacement = (0..super::INLINE_DEFINITION_WORD_CAPACITY + 1)
            .map(|index| TokenWord::from_raw(index as u32 + 1))
            .collect::<Vec<_>>();
        let definition = generation
            .definitions_mut()
            .allocate(&[], &replacement)
            .expect("overflow definition");
        let owner = generation
            .definitions()
            .global
            .owner
            .as_ref()
            .expect("definition words own storage");
        assert_eq!(owner.overflow_words.borrow().len(), 1);
        assert_eq!(
            generation.definitions().get(definition).replacement_text(),
            replacement
        );
    });
}

#[test]
fn direct_definition_seals_provenance_in_the_only_header_write() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let build = arena
            .begin_build(super::DefinitionDestination::Global, origin(1))
            .expect("definition transaction");
        arena.finish_parameters(build).expect("parameter boundary");
        arena
            .set_build_origin(build, origin(2))
            .expect("final scan provenance");
        let definition = arena.seal_build(build).expect("sealed definition");
        assert_eq!(arena.get(definition).definition_origin(), origin(2));
    });
}

#[test]
fn aborted_direct_definition_discards_unpublished_provenance_and_words() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let first = arena
            .begin_build(super::DefinitionDestination::Global, origin(11))
            .expect("first definition transaction");
        arena.finish_parameters(first).expect("parameter boundary");
        arena
            .push_replacement(first, TokenWord::pack(Token::frozen_relax()))
            .expect("unpublished replacement");
        arena
            .set_build_origin(first, origin(12))
            .expect("unpublished provenance");
        arena.abort_build(first);

        let second = arena
            .begin_build(super::DefinitionDestination::Global, origin(21))
            .expect("replacement definition transaction");
        arena.finish_parameters(second).expect("parameter boundary");
        let definition = arena.seal_build(second).expect("sealed definition");
        let view = arena.get(definition);
        assert_eq!(view.definition_origin(), origin(21));
        assert!(view.replacement_text().is_empty());
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
        assert!(arena.region(local.region()).is_none());
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
        assert!(arena.region(local.region()).is_none());
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
            assert_eq!(
                after_sequential.group_entry_slot_inspections - before.group_entry_slot_inspections,
                groups
            );
            assert_eq!(
                after_sequential.local_slot_chunk_allocations - before.local_slot_chunk_allocations,
                1,
                "sequential groups reuse one coarse slot chunk"
            );

            let before_history_probe = arena.retirement_counters();
            arena.begin_group().expect("group after retired history");
            let after_history_entry = arena.retirement_counters();
            assert_eq!(
                after_history_entry.group_entry_slot_inspections
                    - before_history_probe.group_entry_slot_inspections,
                1,
                "group entry addresses one reusable slot after {groups} retired regions"
            );
            assert_eq!(
                after_history_entry.local_slot_chunk_allocations
                    - before_history_probe.local_slot_chunk_allocations,
                0,
                "group entry allocates no chunk after {groups} retired regions"
            );
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
fn reused_local_slot_incarnation_rejects_a_stale_definition_key() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("first local group");
        let stale = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        arena.end_group();
        assert!(arena.region(stale.region()).is_none());

        arena.begin_group().expect("reused local group");
        let current = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        assert_eq!(
            stale.region() & u32::from(u16::MAX),
            current.region() & u32::from(u16::MAX)
        );
        assert_ne!(stale, current);
        assert!(arena.region(stale.region()).is_none());
        assert_eq!(arena.get(current).replacement_text().len(), 1);
        arena.end_group();
    });
}

#[test]
fn exhausted_local_slot_incarnation_never_wraps_into_an_aba_alias() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("initial local group");
        let address = super::local_region_address(arena.active_local)
            .expect("local key")
            .0;
        arena.end_group();
        arena
            .local_slots
            .store
            .borrow_mut()
            .slot_mut(address)
            .expect("reusable slot")
            .incarnation = u16::MAX - 1;

        arena.begin_group().expect("last safe incarnation");
        let exhausted = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        assert_eq!(
            super::local_region_address(exhausted.region()),
            Some((address, u16::MAX))
        );
        arena.end_group();

        arena.begin_group().expect("different reusable address");
        let replacement = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        assert_ne!(
            super::local_region_address(replacement.region())
                .expect("replacement local key")
                .0,
            address
        );
        assert!(arena.region(exhausted.region()).is_none());
        arena.end_group();
    });
}

#[test]
fn one_checkpoint_child_lease_transitively_pins_and_releases_its_parent_chain() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("parent group");
        let parent = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        arena.begin_group().expect("child group");
        let child = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let checkpoint = arena.checkpoint_lease();
        arena.end_group();
        arena.end_group();
        assert!(arena.region(child.region()).is_some());
        assert!(arena.region(parent.region()).is_some());

        let before = arena.retirement_counters();
        drop(checkpoint);
        let after = arena.retirement_counters();
        assert_eq!(
            after.lease_release_region_inspections - before.lease_release_region_inspections,
            1
        );
        assert_eq!(after.regions_reclaimed - before.regions_reclaimed, 2);
        assert!(arena.region(child.region()).is_none());
        assert!(arena.region(parent.region()).is_none());
    });
}

#[test]
fn deep_checkpoint_release_reclaims_the_parent_chain_iteratively() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let depth = 16_384_u64;
        for _ in 0..depth {
            arena.begin_group().expect("deep checkpoint group");
        }
        let checkpoint = arena.checkpoint_lease();
        for _ in 0..depth {
            arena.end_group();
        }

        let before = arena.retirement_counters();
        drop(checkpoint);
        let after = arena.retirement_counters();
        assert_eq!(after.regions_reclaimed - before.regions_reclaimed, depth);
        assert_eq!(
            after.lease_release_region_inspections - before.lease_release_region_inspections,
            1,
            "one final checkpoint release drains only its structural ancestor chain"
        );
        assert!(arena.local_slots.is_empty());
    });
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
        assert!(arena.region(selected_definition.region()).is_none());
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
fn checkpoint_lease_pins_the_parent_chain_through_one_active_region() {
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
            1
        );
        drop(checkpoint);
        let after_release = arena.retirement_counters();
        assert_eq!(
            after_release.lease_release_region_inspections
                - after_capture.lease_release_region_inspections,
            1
        );
    });
}

#[test]
fn content_identity_survives_allocation_order_and_local_promotion() {
    with_generation(|mut generation| {
        assert!(generation.enable_semantic_identity());
        let arena = generation.definitions_mut();
        arena.begin_group().expect("local definition group");
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
fn checkpoint_rejection_restores_active_leased_region_in_deeper_head_suffix() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("checkpoint outer group");
        let checkpoint = arena.cursor();
        arena.begin_group().expect("accepted child group");
        let accepted = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let lease = arena.lease(accepted);

        let tail = arena.begin_checkpoint_candidate(checkpoint);
        let candidate = direct_definition(arena, super::DefinitionDestination::Local, &[], &[]);
        assert_ne!(candidate.region(), accepted.region());
        arena.reject_checkpoint_candidate(checkpoint, tail);

        assert_eq!(arena.get(accepted).replacement_text().len(), 1);
        let restored_child =
            direct_definition(arena, super::DefinitionDestination::Local, &[], &[]);
        assert_eq!(restored_child.region(), accepted.region());
        drop(lease);
        arena.end_group();
        arena.end_group();
    });
}

#[test]
fn checkpoint_acceptance_retires_only_active_leased_region_in_deeper_head_suffix() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("checkpoint outer group");
        let checkpoint = arena.cursor();
        arena.begin_group().expect("accepted child group");
        let accepted = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let lease = arena.lease(accepted);

        let tail = arena.begin_checkpoint_candidate(checkpoint);
        let candidate = direct_definition(arena, super::DefinitionDestination::Local, &[], &[]);
        arena.accept_checkpoint_candidate(tail);

        assert_eq!(arena.get(accepted).replacement_text().len(), 1);
        assert_ne!(candidate.region(), accepted.region());
        drop(lease);
        assert!(arena.region(accepted.region()).is_none());
        arena.end_group();
    });
}

#[test]
fn checkpoint_acceptance_reactivates_leased_region_below_the_head_depth() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("outer group");
        arena.begin_group().expect("checkpoint child group");
        let checkpoint = arena.cursor();
        let checkpoint_lease = arena.checkpoint_lease();
        arena.end_group();

        let tail = arena.begin_checkpoint_candidate(checkpoint);
        let candidate = direct_definition(arena, super::DefinitionDestination::Local, &[], &[]);
        assert_eq!(candidate.region(), checkpoint.active_local);
        arena.accept_checkpoint_candidate(tail);
        drop(checkpoint_lease);
        assert!(arena.region(candidate.region()).is_some());
        arena.end_group();
        arena.end_group();
    });
}

#[test]
fn checkpoint_rejection_restores_parent_written_after_ending_checkpoint_child() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("parent A");
        let parent_root = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        arena.begin_group().expect("checkpoint child B");
        let child_root = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let checkpoint = arena.cursor();
        let checkpoint_lease = arena.checkpoint_lease();

        arena.end_group();
        let accepted_parent = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let tail = arena.begin_checkpoint_candidate(checkpoint);
        let rejected_child =
            direct_definition(arena, super::DefinitionDestination::Local, &[], &[]);
        arena.reject_checkpoint_candidate(checkpoint, tail);

        assert_eq!(arena.active_local, parent_root.region());
        assert_eq!(arena.get(parent_root).replacement_text().len(), 1);
        assert_eq!(arena.get(accepted_parent).replacement_text().len(), 1);
        assert_eq!(arena.get(child_root).replacement_text().len(), 1);
        assert!(
            arena
                .region(rejected_child.region())
                .is_some_and(|region| rejected_child.row_index() as usize >= region.headers.len())
        );
        drop(checkpoint_lease);
        arena.end_group();
    });
}

#[test]
fn checkpoint_acceptance_keeps_restored_child_and_discards_later_parent_write() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("parent A");
        let parent_root = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        arena.begin_group().expect("checkpoint child B");
        let checkpoint = arena.cursor();
        let checkpoint_lease = arena.checkpoint_lease();

        arena.end_group();
        let discarded_parent = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        let tail = arena.begin_checkpoint_candidate(checkpoint);
        let accepted_child = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[TokenWord::pack(Token::frozen_relax())],
        );
        arena.accept_checkpoint_candidate(tail);

        assert_eq!(arena.active_local, accepted_child.region());
        assert_eq!(arena.get(accepted_child).replacement_text().len(), 1);
        assert_eq!(
            arena
                .region(parent_root.region())
                .expect("parent remains structurally pinned")
                .headers
                .len(),
            1,
            "the parent row written after the checkpoint is not part of the candidate"
        );
        assert_eq!(discarded_parent.row_index(), 1);
        drop(checkpoint_lease);
        arena.end_group();
        arena.end_group();
    });
}

#[test]
fn checkpoint_settlement_visits_only_post_checkpoint_region_mutations() {
    for depth in [2_u64, 64, 4_096] {
        with_generation(|mut generation| {
            let arena = generation.definitions_mut();
            for _ in 0..depth {
                arena.begin_group().expect("nested group");
            }
            let checkpoint = arena.cursor();
            let checkpoint_lease = arena.checkpoint_lease();
            arena.end_group();
            direct_definition(
                arena,
                super::DefinitionDestination::Local,
                &[],
                &[TokenWord::pack(Token::frozen_relax())],
            );

            let before = arena.retirement_counters();
            let tail = arena.begin_checkpoint_candidate(checkpoint);
            let after = arena.retirement_counters();
            assert_eq!(
                after.checkpoint_region_inspections - before.checkpoint_region_inspections,
                2,
                "one ended child and one written parent are independent of depth {depth}"
            );
            arena.reject_checkpoint_candidate(checkpoint, tail);
            drop(checkpoint_lease);
            for _ in 1..depth {
                arena.end_group();
            }
        });
    }
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
        assert_eq!(replacement.row_index(), 0);
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
        assert_eq!(candidate.row_index(), prior.row_index());
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
        assert_eq!(view.parameter_pattern(), reference);
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
fn resident_local_body_retains_one_exact_region_after_group_retirement() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        arena.begin_group().expect("local definition group");
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
        let definition = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &parameters,
            &words,
        );
        let (_, _, body) = arena
            .admit_macro_body(definition)
            .expect("resident local body");
        assert_eq!(std::rc::Rc::strong_count(&body.owner), 2);

        arena.end_group();

        assert!(arena.region(definition.region()).is_none());
        assert_eq!(std::rc::Rc::strong_count(&body.owner), 1);
        assert_eq!(body.parameter_len(), parameters.len());
        assert_eq!(body.parameter_word(0), Some(parameters[0]));
        assert_eq!(body.parameter_word(1), Some(parameters[1]));
        assert_eq!(body.parameter_word(2), None);
        assert_eq!(body.word(0), Some(words[0]));
        assert_eq!(body.word(1), Some(words[1]));
        assert_eq!(body.word(2), Some(words[2]));
        assert_eq!(body.word(3), None);
    });
}

#[test]
fn resident_format_global_and_local_bodies_share_the_same_direct_read_contract() {
    with_generation(|mut generation| {
        let arena = generation.definitions_mut();
        let format_word = TokenWord::from_raw(31);
        let global_word = TokenWord::from_raw(37);
        let local_word = TokenWord::from_raw(41);
        let format = direct_definition(
            arena,
            super::DefinitionDestination::Format,
            &[],
            &[format_word],
        );
        let global = direct_definition(
            arena,
            super::DefinitionDestination::Global,
            &[],
            &[global_word],
        );
        arena.begin_group().expect("local definition group");
        let local = direct_definition(
            arena,
            super::DefinitionDestination::Local,
            &[],
            &[local_word],
        );
        let (_, _, format_body) = arena.admit_macro_body(format).expect("format body");
        let (_, _, global_body) = arena.admit_macro_body(global).expect("global body");
        let (_, _, local_body) = arena.admit_macro_body(local).expect("local body");
        arena.end_group();

        assert_eq!(format_body.word(0), Some(format_word));
        assert_eq!(global_body.word(0), Some(global_word));
        assert_eq!(local_body.word(0), Some(local_word));
    });
}

#[test]
fn resident_body_walks_chunk_boundary_and_large_definition_directly() {
    with_generation(|mut generation| {
        let replacement = (0..super::DEFINITION_WORD_CHUNK_CAPACITY + 19)
            .map(|index| TokenWord::from_raw(index as u32 + 1))
            .collect::<Vec<_>>();
        let definition = generation
            .definitions_mut()
            .allocate(&[], &replacement)
            .expect("large definition");
        let (_, _, body) = generation
            .definitions()
            .admit_macro_body(definition)
            .expect("resident large body");

        for (position, expected) in replacement.iter().copied().enumerate() {
            assert_eq!(body.word(position), Some(expected));
        }
        assert_eq!(body.word(replacement.len()), None);
    });
}

#[test]
fn resident_body_read_work_is_exact_for_one_full_and_multiple_chunks() {
    for (words, expected_transitions) in [
        (1_usize, 0_u64),
        (super::DEFINITION_WORD_CHUNK_CAPACITY, 1),
        (super::DEFINITION_WORD_CHUNK_CAPACITY * 2 + 1, 2),
    ] {
        with_generation(|mut generation| {
            let replacement = (0..words)
                .map(|index| TokenWord::from_raw(index as u32 + 1))
                .collect::<Vec<_>>();
            let definition = generation
                .definitions_mut()
                .allocate(&[], &replacement)
                .expect("resident read fixture");
            super::reset_resident_macro_body_read_counters();
            let (_, _, body) = generation
                .definitions()
                .admit_macro_body(definition)
                .expect("resident body admission");
            let owner_count = body.profile_region_owner_count();
            for (position, expected) in replacement.iter().copied().enumerate() {
                assert_eq!(body.word(position), Some(expected));
            }
            assert_eq!(body.word(words), None);
            assert_eq!(body.profile_region_owner_count(), owner_count);
            assert_eq!(
                super::resident_macro_body_read_counters(),
                super::ResidentMacroBodyReadCounters {
                    admission_chunk_lookups: 1,
                    region_owner_acquisitions: 1,
                    direct_chunk_slot_reads: words as u64,
                    chunk_boundary_transitions: expected_transitions,
                    whole_body_copies: 0,
                }
            );
        });
    }
}

#[test]
fn resident_body_scalar_position_replays_exactly_after_chunk_crossing() {
    with_generation(|mut generation| {
        let words =
            super::INLINE_DEFINITION_WORD_CAPACITY + super::DEFINITION_WORD_CHUNK_CAPACITY + 3;
        let replacement = (0..words)
            .map(|index| TokenWord::from_raw(index as u32 + 1))
            .collect::<Vec<_>>();
        let definition = generation
            .definitions_mut()
            .allocate(&[], &replacement)
            .expect("rollback fixture");
        let (_, _, body) = generation
            .definitions()
            .admit_macro_body(definition)
            .expect("resident body");
        let checkpoint_position =
            super::INLINE_DEFINITION_WORD_CAPACITY + super::DEFINITION_WORD_CHUNK_CAPACITY - 1;
        for (expected_position, expected) in replacement
            .iter()
            .copied()
            .enumerate()
            .take(checkpoint_position)
        {
            assert_eq!(body.word(expected_position), Some(expected));
        }
        let before = body
            .word(checkpoint_position)
            .expect("word before boundary");
        let across = body
            .word(checkpoint_position + 1)
            .expect("word across boundary");
        assert_ne!(before, across);
        assert_eq!(
            body.word(checkpoint_position),
            Some(before),
            "the immutable coordinate repeats the same chunk-local read"
        );
    });
}

#[test]
fn differing_refs_compare_across_word_chunks_without_materializing_contents() {
    with_generation(|mut generation| {
        let replacement = (0..super::DEFINITION_WORD_CHUNK_CAPACITY + 7)
            .map(|index| TokenWord::from_raw(index as u32 + 1))
            .collect::<Vec<_>>();
        let first = generation
            .definitions_mut()
            .allocate(&[], &replacement)
            .expect("first large definition");
        let second = generation
            .definitions_mut()
            .allocate(&[], &replacement)
            .expect("second large definition");
        assert_ne!(first, second);
        assert!(generation.definitions().contents_equal(first, second));

        let mut changed = replacement;
        *changed.last_mut().expect("nonempty replacement") = TokenWord::pack(Token::Char {
            ch: 'z',
            cat: Catcode::Other,
        });
        let third = generation
            .definitions_mut()
            .allocate(&[], &changed)
            .expect("changed large definition");
        assert!(!generation.definitions().contents_equal(first, third));
    });
}
