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
