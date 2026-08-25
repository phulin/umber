use super::{
    NodeArena, NodeArenaError, NodeListId, NodeRelocationScratch, PageLifetime, ScratchLifetime,
};
use crate::glue::Order;
use crate::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
use crate::scaled::{GlueSetRatio, Scaled};

enum Durable {}

fn boxed<List>(children: List) -> Node<List> {
    Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(20),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))
}

#[test]
fn explicit_roots_promote_exact_closure_once() {
    let mut page = NodeArena::<PageLifetime>::new();
    let child = page
        .publish(vec![Node::Penalty(7)])
        .expect("test fixture is valid");
    let parent = page
        .publish(vec![boxed(child)])
        .expect("test fixture is valid");
    let _unrelated = page
        .publish(vec![Node::Penalty(99)])
        .expect("test fixture is valid");

    let mut durable = NodeArena::<Durable>::new();
    let roots = page
        .promote_into(&[parent, parent], &mut durable)
        .expect("test fixture is valid");

    assert_eq!(roots[0], roots[1]);
    assert_eq!(durable.len(), 2, "only parent and child escape");
    let parent = durable.get(roots[0]).expect("test fixture is valid");
    let Node::HList(boxed) = &parent.nodes()[0] else {
        panic!("promoted parent lost its box shape");
    };
    assert_eq!(
        durable
            .get(boxed.children)
            .expect("test fixture is valid")
            .nodes(),
        [Node::Penalty(7)]
    );
}

#[test]
fn sparse_relocation_maps_bound_capacity_by_live_keys_in_both_directions() {
    let page_value = NodeListId::<PageLifetime>::from_row(9, 1, 11);
    let durable_value = NodeListId::<Durable>::from_row(13, 1, 17);
    let mut durable_to_page = NodeRelocationScratch::<Durable, PageLifetime>::default();
    durable_to_page.begin();
    for key in [7, 1_000_000_007, usize::MAX - 1] {
        durable_to_page.marks.set_state(key, 2);
        durable_to_page.relocation.insert(key, page_value);
        assert_eq!(durable_to_page.relocation.get(key), Some(page_value));
    }
    assert_eq!(durable_to_page.capacities(), (16, 0, 16));

    let mut page_to_durable = NodeRelocationScratch::<PageLifetime, Durable>::default();
    page_to_durable.begin();
    for key in [usize::MAX - 3, 1_000_000_009, 5] {
        page_to_durable.marks.set_state(key, 2);
        page_to_durable.relocation.insert(key, durable_value);
        assert_eq!(page_to_durable.relocation.get(key), Some(durable_value));
    }
    assert_eq!(page_to_durable.capacities(), (16, 0, 16));
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_sparse_relocation_traversal_allocates_nothing() {
    let mut source = NodeArena::<PageLifetime>::new();
    let child = source
        .publish(vec![Node::Penalty(7)])
        .expect("test fixture is valid");
    let root = source
        .publish(vec![boxed(child), boxed(child)])
        .expect("test fixture is valid");
    let mut scratch = NodeRelocationScratch::<PageLifetime, Durable>::default();
    let traverse = |scratch: &mut NodeRelocationScratch<PageLifetime, Durable>| {
        scratch.begin();
        source
            .postorder_sparse(root, &mut scratch.marks, &mut scratch.order)
            .expect("test fixture is valid");
        assert_eq!(scratch.order, [child, root]);
        scratch.order.clear();
    };
    traverse(&mut scratch);

    let owner = crate::measurement::HotCoreAllocationOwner::SemanticApply;
    let before = crate::measurement::hot_core_thread_allocation_measurement(owner);
    {
        let _scope = crate::measurement::hot_core_allocation_scope(owner);
        for _ in 0..128 {
            traverse(&mut scratch);
        }
    }
    let after = crate::measurement::hot_core_thread_allocation_measurement(owner);
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
}

#[test]
fn operation_scratch_promotes_only_declared_page_roots() {
    let mut scratch = NodeArena::<ScratchLifetime>::new();
    let child = scratch
        .publish(vec![Node::Penalty(3)])
        .expect("test fixture is valid");
    let root = scratch
        .publish(vec![boxed(child)])
        .expect("test fixture is valid");
    let _discarded_probe = scratch
        .publish(vec![Node::Penalty(90)])
        .expect("test fixture is valid");
    let mut page = NodeArena::<PageLifetime>::new();

    let promoted = scratch
        .promote_into(&[root], &mut page)
        .expect("test fixture is valid")[0];

    assert_eq!(page.len(), 2);
    let Node::HList(boxed) = &page.get(promoted).expect("test fixture is valid").nodes()[0] else {
        panic!("promoted operation root lost its box shape")
    };
    assert_eq!(
        page.get(boxed.children)
            .expect("test fixture is valid")
            .nodes(),
        [Node::Penalty(3)]
    );
}

#[cfg(feature = "profiling")]
#[test]
fn checkpoint_fork_shares_rows_without_physical_graph_copy() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let root = arena
        .publish(vec![Node::Penalty(7)])
        .expect("test fixture is valid");
    let baseline = crate::measurement::node_graph_census();

    let fork = arena.fork();
    let delta = crate::measurement::node_graph_census().saturating_sub(baseline);

    assert_eq!(delta.checkpoint_shared_rows, 1);
    assert_eq!(delta.physical_copy_rows, 0);
    assert_eq!(delta.physical_copy_nodes, 0);
    assert_eq!(
        std::rc::Rc::strong_count(arena.segments[0].as_ref().expect("segment")),
        2
    );
    assert_eq!(
        fork.get(root)
            .expect("shared coordinate remains live")
            .nodes(),
        [Node::Penalty(7)]
    );
}

#[test]
fn checkpoint_fork_seals_a_coarse_segment_and_appends_independently() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let shared = arena
        .publish(vec![Node::Penalty(7)])
        .expect("test fixture is valid");
    let mut fork = arena.fork();
    let source_only = arena
        .publish(vec![Node::Penalty(8)])
        .expect("test fixture is valid");
    let fork_only = fork
        .publish(vec![Node::Penalty(9)])
        .expect("test fixture is valid");

    assert_eq!(arena.segments.len(), 2);
    assert_eq!(fork.segments.len(), 2);
    assert_eq!(
        arena.get(shared).expect("shared row").nodes(),
        [Node::Penalty(7)]
    );
    assert_eq!(
        fork.get(shared).expect("shared row").nodes(),
        [Node::Penalty(7)]
    );
    assert_eq!(
        arena.get(source_only).expect("source row").nodes(),
        [Node::Penalty(8)]
    );
    assert_eq!(
        fork.get(fork_only).expect("fork row").nodes(),
        [Node::Penalty(9)]
    );
    assert!(arena.get(fork_only).is_err());
    assert!(fork.get(source_only).is_err());
}

#[test]
fn rollback_cursor_is_owner_checked_and_truncates_only_suffix() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let retained = arena
        .publish(vec![Node::Penalty(1)])
        .expect("test fixture is valid");
    let cursor = arena.cursor();
    let rejected = arena
        .publish(vec![Node::Penalty(2)])
        .expect("test fixture is valid");

    arena.truncate(cursor).expect("test fixture is valid");
    assert_eq!(
        arena.get(retained).expect("test fixture is valid").nodes(),
        [Node::Penalty(1)]
    );
    assert!(matches!(
        arena.get(rejected),
        Err(NodeArenaError::InvalidList)
    ));

    let foreign = NodeArena::<PageLifetime>::new().cursor();
    assert_eq!(arena.truncate(foreign), Err(NodeArenaError::ForeignCursor));
}

#[test]
fn nested_regions_release_only_their_strict_suffix() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let retained = arena
        .publish(vec![Node::Penalty(1)])
        .expect("test fixture is valid");
    let outer = arena.begin_region();
    let outer_list = arena
        .publish(vec![Node::Penalty(2)])
        .expect("test fixture is valid");
    let inner = arena.begin_region();
    let inner_child = arena
        .publish(vec![Node::Penalty(3)])
        .expect("test fixture is valid");
    let inner_alias = arena
        .publish(vec![boxed(inner_child), boxed(inner_child)])
        .expect("test fixture is valid");

    arena.release_region(inner).expect("nested suffix is valid");
    assert!(arena.get(retained).is_ok());
    assert!(arena.get(outer_list).is_ok());
    assert!(matches!(
        arena.get(inner_child),
        Err(NodeArenaError::InvalidList)
    ));
    assert!(matches!(
        arena.get(inner_alias),
        Err(NodeArenaError::InvalidList)
    ));

    arena.release_region(outer).expect("outer suffix is valid");
    assert!(arena.get(retained).is_ok());
    assert!(matches!(
        arena.get(outer_list),
        Err(NodeArenaError::InvalidList)
    ));
}

#[test]
fn warmed_regions_reuse_capacity_and_invalidate_every_alias() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let mut stale = Vec::new();
    for penalty in 0..64 {
        let region = arena.begin_region();
        let child = arena
            .publish(vec![Node::Penalty(penalty)])
            .expect("test fixture is valid");
        let alias = arena
            .publish(vec![boxed(child), boxed(child)])
            .expect("test fixture is valid");
        stale.push((child, alias));
        arena
            .release_region(region)
            .expect("nested suffix is valid");
        assert_eq!(arena.len(), 0);
    }
    assert!(arena.rows.capacity() >= 2);
    for (child, alias) in stale {
        assert!(matches!(arena.get(child), Err(NodeArenaError::InvalidList)));
        assert!(matches!(arena.get(alias), Err(NodeArenaError::InvalidList)));
    }
}

#[test]
fn retained_failed_region_remains_available_to_enclosing_rollback() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let enclosing = arena.cursor();
    let region = arena.begin_region();
    let restored = arena
        .publish(vec![Node::Penalty(9)])
        .expect("test fixture is valid");

    arena
        .retain_region(region)
        .expect("failed suffix returns to the enclosing owner");
    assert!(arena.get(restored).is_ok());
    arena
        .truncate(enclosing)
        .expect("enclosing rollback owns the returned suffix");
    assert!(matches!(
        arena.get(restored),
        Err(NodeArenaError::InvalidList)
    ));
}

#[test]
fn invalid_child_rejects_publication_without_growing_arena() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let invalid = NodeListId::from_row(arena.owner.wrapping_add(1), 1, 1);
    assert_eq!(
        arena.publish(vec![boxed(invalid)]),
        Err(NodeArenaError::InvalidList)
    );
    assert_eq!(arena.len(), 0);
}

#[test]
fn list_coordinate_is_owner_checked() {
    let mut first = NodeArena::<PageLifetime>::new();
    let list = first
        .publish(vec![Node::Penalty(4)])
        .expect("test fixture is valid");
    let second = NodeArena::<PageLifetime>::new();

    assert_eq!(
        second
            .get(list)
            .expect_err("invalid test fixture is rejected"),
        NodeArenaError::InvalidList
    );
}

#[test]
fn completed_closure_release_preserves_unrelated_mode_rows() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let mode = arena
        .publish(vec![Node::Penalty(7)])
        .expect("test fixture is valid");
    let child = arena
        .publish(vec![Node::Penalty(11)])
        .expect("test fixture is valid");
    let page = arena
        .publish(vec![boxed(child)])
        .expect("test fixture is valid");

    arena.release_closure(page).expect("test fixture is valid");

    assert_eq!(
        arena.get(mode).expect("test fixture is valid").nodes(),
        [Node::Penalty(7)]
    );
    assert_eq!(
        arena
            .get(page)
            .expect_err("invalid test fixture is rejected"),
        NodeArenaError::InvalidList
    );
    assert_eq!(
        arena
            .get(child)
            .expect_err("invalid test fixture is rejected"),
        NodeArenaError::InvalidList
    );
}

#[test]
fn released_row_reuse_issues_a_fresh_list_generation() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let released = arena
        .publish(vec![Node::Penalty(1)])
        .expect("test fixture is valid");
    arena
        .release_closure(released)
        .expect("test fixture is valid");
    let replacement = arena
        .publish(vec![Node::Penalty(2)])
        .expect("test fixture is valid");

    assert_ne!(released, replacement);
    assert_eq!(
        arena
            .get(released)
            .expect_err("invalid test fixture is rejected"),
        NodeArenaError::InvalidList
    );
    assert_eq!(
        arena
            .get(replacement)
            .expect("test fixture is valid")
            .nodes(),
        [Node::Penalty(2)]
    );
}
