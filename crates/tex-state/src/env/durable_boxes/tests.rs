use super::*;
use crate::node::Node;
use crate::node_region::NodePool;
use crate::page_node_arena::PageMaterialRegion;

macro_rules! page_arena {
    ($arena:ident, $pool:ident, $state:ident, $bytes:expr) => {
        let mut $pool = NodePool::with_chunk_bytes($bytes);
        let mut $state = PageMaterialRegion::new(&mut $pool);
        let mut $arena = PageMaterialArena::new(&mut $pool, &mut $state);
    };
}

fn owner(arena: &mut PageMaterialArena, penalty: i32) -> DurableNodeClosure {
    let root = arena
        .publish_owned([Node::Penalty(penalty)])
        .expect("page root");
    arena
        .copy_page_root_to_durable(root)
        .expect("durable owner")
}

fn current_region(state: &DurableBoxState, index: u16) -> Option<NodeRegionId> {
    state.metadata(index).map(DurableNodeMetadata::region)
}

#[test]
fn durable_owner_slots_reuse_storage_with_a_fresh_incarnation() {
    page_arena!(arena, pool, region, 64);
    let mut owners = DurableOwnerStore::default();
    let first_owner = owner(&mut arena, 7);
    let first_region = first_owner.region_id();
    let first = owners.insert(first_owner);
    owners.retire(&mut arena, first);

    let second_owner = owner(&mut arena, 9);
    let second_region = second_owner.region_id();
    let second = owners.insert(second_owner);
    assert_eq!(second.slot, first.slot);
    assert_ne!(second.incarnation, first.incarnation);
    assert!(!arena.durable_region_is_live(first_region));
    assert_eq!(owners.owner(second).region_id(), second_region);
    owners.retire(&mut arena, second);
}

#[test]
fn overwrite_without_retained_history_retires_the_old_owner() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let first = owner(&mut arena, 11);
    let first_id = first.region_id();
    state
        .assign(
            &mut arena,
            0,
            Some(first),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("initial assignment");
    let second = owner(&mut arena, 13);
    let second_id = second.region_id();
    state
        .assign(
            &mut arena,
            0,
            Some(second),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("overwrite");

    assert!(!arena.durable_region_is_live(first_id));
    assert_eq!(current_region(&state, 0), Some(second_id));
    state.retire_all(&mut arena);
}

#[test]
fn retained_checkpoint_rewinds_and_reject_restores_exact_owners() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let accepted = owner(&mut arena, 17);
    let accepted_id = accepted.region_id();
    state
        .assign(
            &mut arena,
            3,
            Some(accepted),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("accepted assignment");
    let checkpoint = state.checkpoint_cursor();
    let head = owner(&mut arena, 19);
    let head_id = head.region_id();
    state
        .assign(
            &mut arena,
            3,
            Some(head),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("accepted head");

    let accepted_tail = state
        .begin_checkpoint_candidate(&mut arena, checkpoint)
        .expect("candidate");
    assert_eq!(current_region(&state, 3), Some(accepted_id));
    let candidate = owner(&mut arena, 23);
    let candidate_id = candidate.region_id();
    state
        .assign(
            &mut arena,
            3,
            Some(candidate),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("candidate assignment");
    state.reject_checkpoint_candidate(&mut arena, checkpoint, accepted_tail);

    assert_eq!(current_region(&state, 3), Some(head_id));
    assert!(!arena.durable_region_is_live(candidate_id));
    assert!(arena.durable_region_is_live(accepted_id));
    state.retire_all(&mut arena);
}

#[test]
fn operation_rollback_moves_the_original_owner_back_without_copying() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let original = owner(&mut arena, 29);
    let original_id = original.region_id();
    state
        .assign(
            &mut arena,
            7,
            Some(original),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("original assignment");
    let operation = state.begin_operation();
    let replacement = owner(&mut arena, 31);
    let replacement_id = replacement.region_id();
    let before = arena.durable_transition_counters();
    state
        .replace(&mut arena, 7, Some(replacement))
        .expect("operation replacement");
    state.rollback_operation(&mut arena, operation);

    assert_eq!(current_region(&state, 7), Some(original_id));
    assert!(!arena.durable_region_is_live(replacement_id));
    assert_eq!(arena.durable_transition_counters(), before);
    state.retire_all(&mut arena);
}

#[test]
fn active_operation_take_uses_a_rollbackable_zero_copy_loan() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let original = owner(&mut arena, 33);
    let original_address = arena
        .durable_list(&original)
        .expect("original durable list")
        .testing_node_address(0)
        .expect("original payload address");
    state
        .assign(
            &mut arena,
            8,
            Some(original),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("original assignment");
    let operation = state.begin_operation();
    let before = arena.durable_transition_counters();

    let page = state
        .take_to_page(&mut arena, 8)
        .expect("transfer loan")
        .expect("occupied register");
    assert!(state.metadata(8).is_none());
    assert_eq!(
        arena
            .list(page)
            .expect("loaned page list")
            .testing_node_address(0),
        Some(original_address)
    );
    assert_eq!(
        arena
            .durable_transition_counters()
            .history_preservation_nodes_copied,
        before.history_preservation_nodes_copied
    );

    state.rollback_operation(&mut arena, operation);
    let restored = state.value(8).expect("rollback restores durable owner");
    assert_eq!(
        arena
            .durable_list(restored)
            .expect("restored durable list")
            .testing_node_address(0),
        Some(original_address)
    );
    assert_eq!(
        arena
            .durable_transition_counters()
            .history_preservation_nodes_copied,
        before.history_preservation_nodes_copied
    );
    state.retire_all(&mut arena);
}

#[test]
fn operation_rollback_restores_the_maintained_semantic_root() {
    page_arena!(arena, pool, region, 64);
    arena.enable_semantic_identity();
    let mut state = DurableBoxState::new();
    assert!(state.enable_semantic_identity());
    let original = owner(&mut arena, 31);
    state
        .assign(
            &mut arena,
            8,
            Some(original),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("original assignment");
    let original_identity = state.semantic_identity_root();
    let operation = state.begin_operation();
    let replacement = owner(&mut arena, 37);
    state
        .replace(&mut arena, 8, Some(replacement))
        .expect("operation replacement");

    assert_ne!(state.semantic_identity_root(), original_identity);
    state.rollback_operation(&mut arena, operation);

    assert_eq!(state.semantic_identity_root(), original_identity);
    state.retire_all(&mut arena);
}

#[test]
fn local_group_restore_moves_the_saved_owner_back() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let original = owner(&mut arena, 37);
    let original_id = original.region_id();
    state
        .assign(
            &mut arena,
            9,
            Some(original),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("original assignment");
    state.begin_group(2);
    let local = owner(&mut arena, 41);
    let local_id = local.region_id();
    state
        .assign(
            &mut arena,
            9,
            Some(local),
            super::super::AssignmentScope::Local,
            2,
        )
        .expect("local assignment");
    state.end_group(&mut arena, 2).expect("group restore");

    assert_eq!(current_region(&state, 9), Some(original_id));
    assert!(!arena.durable_region_is_live(local_id));
    state.retire_all(&mut arena);
}

#[test]
fn checkpoint_accept_drops_the_superseded_owner_only_once() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let old = owner(&mut arena, 43);
    let old_id = old.region_id();
    state
        .assign(
            &mut arena,
            12,
            Some(old),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("old assignment");
    let checkpoint = state.checkpoint_cursor();
    let superseded = owner(&mut arena, 47);
    let superseded_id = superseded.region_id();
    state
        .assign(
            &mut arena,
            12,
            Some(superseded),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("accepted suffix");
    let accepted_tail = state
        .begin_checkpoint_candidate(&mut arena, checkpoint)
        .expect("candidate");
    let current = owner(&mut arena, 53);
    let current_id = current.region_id();
    state
        .assign(
            &mut arena,
            12,
            Some(current),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("candidate suffix");
    state.accept_checkpoint_candidate(&mut arena, accepted_tail);

    assert_eq!(current_region(&state, 12), Some(current_id));
    assert!(!arena.durable_region_is_live(superseded_id));
    assert!(arena.durable_region_is_live(old_id));
    state.retire_all(&mut arena);
}

#[test]
fn checkpoint_restore_reopens_group_and_restores_exact_live_owner() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let outer = owner(&mut arena, 59);
    state
        .assign(
            &mut arena,
            18,
            Some(outer),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("outer assignment");
    state.begin_group(2);
    let local = owner(&mut arena, 61);
    let local_id = local.region_id();
    state
        .assign(
            &mut arena,
            18,
            Some(local),
            super::super::AssignmentScope::Local,
            2,
        )
        .expect("local assignment");
    let checkpoint = state.checkpoint_cursor();

    state.end_group(&mut arena, 2).expect("accepted group exit");
    state.restore(&mut arena, checkpoint);

    assert_eq!(current_region(&state, 18), Some(local_id));
    state
        .end_group(&mut arena, 2)
        .expect("restored group topology exits again");
    state.retire_all(&mut arena);
}

#[test]
fn candidate_reject_redoes_accepted_group_exit_with_exact_owner() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let outer = owner(&mut arena, 67);
    let outer_id = outer.region_id();
    state
        .assign(
            &mut arena,
            21,
            Some(outer),
            super::super::AssignmentScope::Global,
            LEVEL_ONE,
        )
        .expect("outer assignment");
    state.begin_group(2);
    let local = owner(&mut arena, 71);
    state
        .assign(
            &mut arena,
            21,
            Some(local),
            super::super::AssignmentScope::Local,
            2,
        )
        .expect("local assignment");
    let checkpoint = state.checkpoint_cursor();
    state.end_group(&mut arena, 2).expect("accepted group exit");
    let accepted = state
        .begin_checkpoint_candidate(&mut arena, checkpoint)
        .expect("candidate");
    let candidate = owner(&mut arena, 73);
    let candidate_id = candidate.region_id();
    state
        .replace(&mut arena, 21, Some(candidate))
        .expect("candidate replacement");

    state.reject_checkpoint_candidate(&mut arena, checkpoint, accepted);

    assert_eq!(current_region(&state, 21), Some(outer_id));
    assert!(!arena.durable_region_is_live(candidate_id));
    state.retire_all(&mut arena);
}

#[test]
fn released_checkpoint_prefix_retires_only_obsolete_alternates_and_rebases_cursors() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let obsolete = owner(&mut arena, 73);
    let obsolete_id = obsolete.region_id();
    state
        .replace(&mut arena, 25, Some(obsolete))
        .expect("obsolete value");
    let root = state.checkpoint_cursor();
    let first = owner(&mut arena, 79);
    let first_id = first.region_id();
    state
        .replace(&mut arena, 25, Some(first))
        .expect("first value");
    let floor = state.checkpoint_cursor();
    let current = owner(&mut arena, 89);
    let current_id = current.region_id();
    state
        .replace(&mut arena, 25, Some(current))
        .expect("current value");
    let before = arena.durable_transition_counters();

    let released = state
        .release_checkpoint_prefix(&mut arena, Some(floor), None)
        .expect("durable prefix release");

    assert_eq!(released.checkpoint_entries, 2);
    assert_eq!(released.retained_groups, 0);
    assert!(!state.validates_cursor(root));
    assert!(state.validates_cursor(floor));
    assert!(!arena.durable_region_is_live(obsolete_id));
    assert!(arena.durable_region_is_live(first_id));
    assert!(arena.durable_region_is_live(current_id));
    assert_eq!(arena.durable_transition_counters(), before);

    state.restore(&mut arena, floor);
    assert_eq!(current_region(&state, 25), Some(first_id));
    assert!(!arena.durable_region_is_live(current_id));
    state.retire_all(&mut arena);
}

#[test]
fn released_prefix_keeps_candidate_rejection_exact() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let root = state.checkpoint_cursor();
    let selected_value = owner(&mut arena, 97);
    let selected_id = selected_value.region_id();
    state
        .replace(&mut arena, 27, Some(selected_value))
        .expect("selected value");
    let selected = state.checkpoint_cursor();
    let accepted_value = owner(&mut arena, 101);
    let accepted_id = accepted_value.region_id();
    state
        .replace(&mut arena, 27, Some(accepted_value))
        .expect("accepted value");
    let mut accepted = state
        .begin_checkpoint_candidate(&mut arena, selected)
        .expect("candidate");
    let candidate_value = owner(&mut arena, 103);
    let candidate_id = candidate_value.region_id();
    state
        .replace(&mut arena, 27, Some(candidate_value))
        .expect("candidate value");

    let released = state
        .release_checkpoint_prefix(&mut arena, Some(selected), Some(&mut accepted))
        .expect("candidate prefix release");
    assert_eq!(released.checkpoint_entries, 1);
    assert!(!state.validates_cursor(root));
    assert!(state.validates_cursor(selected));
    state.reject_checkpoint_candidate(&mut arena, selected, accepted);

    assert_eq!(current_region(&state, 27), Some(accepted_id));
    assert!(arena.durable_region_is_live(selected_id));
    assert!(!arena.durable_region_is_live(candidate_id));
    state.retire_all(&mut arena);
}

#[test]
fn released_prefix_keeps_candidate_acceptance_restorable() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let obsolete = owner(&mut arena, 107);
    let obsolete_id = obsolete.region_id();
    state
        .replace(&mut arena, 29, Some(obsolete))
        .expect("obsolete value");
    let obsolete_cursor = state.checkpoint_cursor();
    let selected_value = owner(&mut arena, 109);
    let selected_id = selected_value.region_id();
    state
        .replace(&mut arena, 29, Some(selected_value))
        .expect("selected value");
    let selected = state.checkpoint_cursor();
    let accepted_value = owner(&mut arena, 113);
    let accepted_id = accepted_value.region_id();
    state
        .replace(&mut arena, 29, Some(accepted_value))
        .expect("accepted value");
    let mut accepted = state
        .begin_checkpoint_candidate(&mut arena, selected)
        .expect("candidate");
    let candidate_value = owner(&mut arena, 127);
    let candidate_id = candidate_value.region_id();
    state
        .replace(&mut arena, 29, Some(candidate_value))
        .expect("candidate value");

    state
        .release_checkpoint_prefix(&mut arena, Some(selected), Some(&mut accepted))
        .expect("candidate prefix release");
    assert!(!state.validates_cursor(obsolete_cursor));
    state.accept_checkpoint_candidate(&mut arena, accepted);

    assert_eq!(current_region(&state, 29), Some(candidate_id));
    assert!(!arena.durable_region_is_live(obsolete_id));
    assert!(!arena.durable_region_is_live(accepted_id));
    assert!(arena.durable_region_is_live(selected_id));
    state.restore(&mut arena, selected);
    assert_eq!(current_region(&state, 29), Some(selected_id));
    state.retire_all(&mut arena);
}

#[test]
fn group_local_replacement_survives_checkpoint_prefix_release() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let outer = owner(&mut arena, 131);
    let outer_id = outer.region_id();
    state
        .replace(&mut arena, 31, Some(outer))
        .expect("outer value");
    state.begin_group(2);
    let old_local = owner(&mut arena, 137);
    state
        .assign(
            &mut arena,
            31,
            Some(old_local),
            super::super::AssignmentScope::Local,
            2,
        )
        .expect("old local value");
    let old_group_cursor = state.checkpoint_cursor();
    state.end_group(&mut arena, 2).expect("old group exit");
    let floor = state.checkpoint_cursor();

    state.begin_group(2);
    let live_local = owner(&mut arena, 139);
    let live_local_id = live_local.region_id();
    state
        .assign(
            &mut arena,
            31,
            Some(live_local),
            super::super::AssignmentScope::Local,
            2,
        )
        .expect("live local value");
    let before = arena.durable_transition_counters();

    let released = state
        .release_checkpoint_prefix(&mut arena, Some(floor), None)
        .expect("group prefix release");

    assert_eq!(released.retained_groups, 1);
    assert!(!state.validates_cursor(old_group_cursor));
    assert_eq!(current_region(&state, 31), Some(live_local_id));
    assert!(arena.durable_region_is_live(outer_id));
    assert_eq!(arena.durable_transition_counters(), before);
    state.end_group(&mut arena, 2).expect("live group exit");
    assert_eq!(current_region(&state, 31), Some(outer_id));
    state.retire_all(&mut arena);
}

#[test]
fn releasing_last_checkpoint_unpins_but_preserves_an_active_group() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let outer = owner(&mut arena, 149);
    let outer_id = outer.region_id();
    state
        .replace(&mut arena, 33, Some(outer))
        .expect("outer value");
    state.begin_group(2);
    let local = owner(&mut arena, 151);
    state
        .assign(
            &mut arena,
            33,
            Some(local),
            super::super::AssignmentScope::Local,
            2,
        )
        .expect("local value");
    let _released = state.checkpoint_cursor();

    state
        .release_checkpoint_prefix(&mut arena, None, None)
        .expect("last checkpoint release");

    assert!(!state.groups[0].checkpoint_pinned);
    assert!(arena.durable_region_is_live(outer_id));
    state.end_group(&mut arena, 2).expect("group exit");
    assert_eq!(current_region(&state, 33), Some(outer_id));
    assert!(state.retained_groups.is_empty());
    state.retire_all(&mut arena);
}

#[test]
fn candidate_accept_preserves_earlier_retained_group_ancestry() {
    page_arena!(arena, pool, region, 64);
    let mut state = DurableBoxState::new();
    let outer = owner(&mut arena, 157);
    let outer_id = outer.region_id();
    state
        .replace(&mut arena, 35, Some(outer))
        .expect("outer value");
    state.begin_group(2);
    let local = owner(&mut arena, 163);
    let local_id = local.region_id();
    state
        .assign(
            &mut arena,
            35,
            Some(local),
            super::super::AssignmentScope::Local,
            2,
        )
        .expect("local value");
    let earlier = state.checkpoint_cursor();
    state.end_group(&mut arena, 2).expect("accepted group exit");
    let selected = state.checkpoint_cursor();
    let mut accepted = state
        .begin_checkpoint_candidate(&mut arena, selected)
        .expect("candidate");
    let candidate = owner(&mut arena, 167);
    state
        .replace(&mut arena, 36, Some(candidate))
        .expect("candidate value");

    state
        .release_checkpoint_prefix(&mut arena, Some(earlier), Some(&mut accepted))
        .expect("release before selected group base");
    state.accept_checkpoint_candidate(&mut arena, accepted);

    assert!(state.validates_cursor(earlier));
    state.restore(&mut arena, earlier);
    assert_eq!(current_region(&state, 35), Some(local_id));
    state
        .end_group(&mut arena, 2)
        .expect("restored earlier group exits");
    assert!(matches!(
        arena
            .durable_list(state.value(35).expect("restored outer value"))
            .expect("restored outer list")
            .get(0),
        Some(Node::Penalty(157))
    ));
    assert!(!arena.durable_region_is_live(outer_id));
    state.retire_all(&mut arena);
}
