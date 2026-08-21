use super::{NodeArena, NodeArenaError, NodeListId, PageLifetime, ScratchLifetime};
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
    let child = page.publish(vec![Node::Penalty(7)]).unwrap();
    let parent = page.publish(vec![boxed(child)]).unwrap();
    let _unrelated = page.publish(vec![Node::Penalty(99)]).unwrap();

    let mut durable = NodeArena::<Durable>::new();
    let roots = page.promote_into(&[parent, parent], &mut durable).unwrap();

    assert_eq!(roots[0], roots[1]);
    assert_eq!(durable.len(), 2, "only parent and child escape");
    let parent = durable.get(roots[0]).unwrap();
    let Node::HList(boxed) = &parent.nodes()[0] else {
        panic!("promoted parent lost its box shape");
    };
    assert_eq!(
        durable.get(boxed.children).unwrap().nodes(),
        [Node::Penalty(7)]
    );
}

#[test]
fn operation_scratch_promotes_only_declared_page_roots() {
    let mut scratch = NodeArena::<ScratchLifetime>::new();
    let child = scratch.publish(vec![Node::Penalty(3)]).unwrap();
    let root = scratch.publish(vec![boxed(child)]).unwrap();
    let _discarded_probe = scratch.publish(vec![Node::Penalty(90)]).unwrap();
    let mut page = NodeArena::<PageLifetime>::new();

    let promoted = scratch.promote_into(&[root], &mut page).unwrap()[0];

    assert_eq!(page.len(), 2);
    let Node::HList(boxed) = &page.get(promoted).unwrap().nodes()[0] else {
        panic!("promoted operation root lost its box shape")
    };
    assert_eq!(
        page.get(boxed.children).unwrap().nodes(),
        [Node::Penalty(3)]
    );
}

#[test]
fn rollback_cursor_is_owner_checked_and_truncates_only_suffix() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let retained = arena.publish(vec![Node::Penalty(1)]).unwrap();
    let cursor = arena.cursor();
    let rejected = arena.publish(vec![Node::Penalty(2)]).unwrap();

    arena.truncate(cursor).unwrap();
    assert_eq!(arena.get(retained).unwrap().nodes(), [Node::Penalty(1)]);
    assert!(matches!(
        arena.get(rejected),
        Err(NodeArenaError::InvalidList)
    ));

    let foreign = NodeArena::<PageLifetime>::new().cursor();
    assert_eq!(arena.truncate(foreign), Err(NodeArenaError::ForeignCursor));
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
    let list = first.publish(vec![Node::Penalty(4)]).unwrap();
    let second = NodeArena::<PageLifetime>::new();

    assert_eq!(second.get(list).unwrap_err(), NodeArenaError::InvalidList);
}

#[test]
fn completed_closure_release_preserves_unrelated_mode_rows() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let mode = arena.publish(vec![Node::Penalty(7)]).unwrap();
    let child = arena.publish(vec![Node::Penalty(11)]).unwrap();
    let page = arena.publish(vec![boxed(child)]).unwrap();

    arena.release_closure(page).unwrap();

    assert_eq!(arena.get(mode).unwrap().nodes(), [Node::Penalty(7)]);
    assert_eq!(arena.get(page).unwrap_err(), NodeArenaError::InvalidList);
    assert_eq!(arena.get(child).unwrap_err(), NodeArenaError::InvalidList);
}

#[test]
fn released_row_reuse_issues_a_fresh_list_generation() {
    let mut arena = NodeArena::<PageLifetime>::new();
    let released = arena.publish(vec![Node::Penalty(1)]).unwrap();
    arena.release_closure(released).unwrap();
    let replacement = arena.publish(vec![Node::Penalty(2)]).unwrap();

    assert_ne!(released, replacement);
    assert_eq!(
        arena.get(released).unwrap_err(),
        NodeArenaError::InvalidList
    );
    assert_eq!(arena.get(replacement).unwrap().nodes(), [Node::Penalty(2)]);
}
