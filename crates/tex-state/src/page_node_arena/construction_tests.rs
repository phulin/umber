use super::*;

fn push(arena: &mut PageMaterialArena<'_>, slot: &mut ModePageListSlot, value: i32) {
    arena
        .construct_mode_node(slot, |destination| destination.penalty(value))
        .expect("direct append");
}

fn penalties(arena: &PageMaterialArena<'_>, span: PageListSpan) -> Vec<i32> {
    arena
        .node_cursor(span.list())
        .expect("retained root")
        .iter()
        .map(|node| match node.to_owned() {
            Node::Penalty(value) => value,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

#[test]
fn direct_mode_tail_packs_nodes_and_keeps_retained_prefix_length_bounded() {
    let mut pool = NodePool::with_chunk_bytes(512);
    let mut state = PageMaterialRegion::new(&mut pool);
    let mut arena = PageMaterialArena::new(&mut pool, &mut state);
    arena.enable_semantic_identity();
    let capacity = arena.payload_chunk_capacity();
    let before = arena.counters();
    let mut slot = ModePageListSlot::default();
    for value in 0..capacity {
        push(&mut arena, &mut slot, value as i32);
    }
    let retained = slot.span();
    assert_eq!(
        arena.counters().direct_blocks_allocated - before.direct_blocks_allocated,
        1,
        "one actual list fills its tail instead of creating one block per node"
    );
    for value in capacity..capacity + 3 {
        push(&mut arena, &mut slot, value as i32);
    }
    assert_eq!(
        arena.counters().direct_blocks_allocated - before.direct_blocks_allocated,
        2
    );
    assert_eq!(
        penalties(&arena, retained),
        (0..capacity as i32).collect::<Vec<_>>()
    );
    assert_eq!(
        penalties(&arena, slot.span()),
        (0..(capacity + 3) as i32).collect::<Vec<_>>()
    );
    let expected = (0..(capacity + 3) as i32)
        .map(Node::<PageListId>::Penalty)
        .collect::<Vec<_>>();
    assert_eq!(
        slot.list().semantic_identity(),
        Some(SemanticSequenceIdentity::from_nodes(&expected).raw())
    );
}

#[test]
fn interleaved_direct_mode_roots_preserve_both_lists_and_conservative_history() {
    let mut pool = NodePool::with_chunk_bytes(512);
    let mut state = PageMaterialRegion::new(&mut pool);
    let mut arena = PageMaterialArena::new(&mut pool, &mut state);
    let mut parent = ModePageListSlot::default();
    let mut child = ModePageListSlot::default();
    push(&mut arena, &mut parent, 1);
    let retained = parent.span();
    push(&mut arena, &mut child, 2);
    push(&mut arena, &mut child, 3);
    push(&mut arena, &mut parent, 4);
    assert_eq!(penalties(&arena, retained), [1]);
    assert_eq!(penalties(&arena, parent.span()), [1, 4]);
    assert_eq!(penalties(&arena, child.span()), [2, 3]);
    let mut restored = ModePageListSlot {
        span: parent.span(),
        move_enabled: false,
    };
    push(&mut arena, &mut restored, 5);
    assert!(
        !restored.move_enabled,
        "appending does not acquire move authority over restored history"
    );
    assert_eq!(penalties(&arena, parent.span()), [1, 4]);
    assert_eq!(penalties(&arena, restored.span()), [1, 4, 5]);
}

#[test]
fn failed_direct_mode_node_restores_the_actual_tail_and_accepts_retry() {
    let mut pool = NodePool::with_chunk_bytes(512);
    let mut state = PageMaterialRegion::new(&mut pool);
    let mut arena = PageMaterialArena::new(&mut pool, &mut state);
    let mut foreign_pool = NodePool::with_chunk_bytes(512);
    let mut foreign_state = PageMaterialRegion::new(&mut foreign_pool);
    let mut foreign = PageMaterialArena::new(&mut foreign_pool, &mut foreign_state);
    arena.enable_semantic_identity();
    let child = foreign
        .publish_owned([Node::Penalty(91)])
        .expect("foreign child");
    let mut slot = ModePageListSlot::default();
    push(&mut arena, &mut slot, 1);
    let before = slot.span();
    let rejected = arena.construct_mode_node(&mut slot, |destination| {
        destination.owned(Node::Disc {
            kind: crate::node::DiscKind::Discretionary,
            pre: child,
            post: PageListId::empty(),
            replace: PageListId::empty(),
            physical_replace_count: 0,
        })
    });
    assert!(matches!(rejected, Err(ForkArenaError::InvalidRegion)));
    assert_eq!(slot.span(), before);
    push(&mut arena, &mut slot, 2);
    assert_eq!(penalties(&arena, before), [1]);
    assert_eq!(penalties(&arena, slot.span()), [1, 2]);
    assert_eq!(
        slot.list().semantic_identity(),
        Some(
            SemanticSequenceIdentity::from_nodes(&[
                Node::<PageListId>::Penalty(1),
                Node::Penalty(2)
            ])
            .raw()
        )
    );
}
