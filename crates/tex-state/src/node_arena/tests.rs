use super::{NodeArena, NodeListBuilder};
use crate::glue::Order;
use crate::ids::{FontId, NodeListId};
use crate::node::{BoxNode, BoxNodeFields, Node, Sign};
use crate::scaled::{GlueSetRatio, Scaled};

#[test]
fn nested_lists_build_bottom_up_and_read_back() {
    let mut arena = NodeArena::new();
    let survivors = crate::survivor::SurvivorArena::new();

    let mut inner = NodeListBuilder::new();
    inner.push(Node::Char {
        font: FontId::testing_new(1),
        ch: 'x',
    });
    let inner_id = inner.finish(&mut arena);

    let mut middle = NodeListBuilder::new();
    middle.push(Node::HList(BoxNode::new(BoxNodeFields {
        width: scaled(10),
        height: scaled(7),
        depth: scaled(3),
        shift: scaled(1),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: inner_id,
    })));
    let middle_id = middle.finish(&mut arena);

    let mut outer = NodeListBuilder::new();
    outer.push(Node::VList(BoxNode::new(BoxNodeFields {
        width: scaled(20),
        height: scaled(9),
        depth: scaled(4),
        shift: scaled(0),
        display: false,
        glue_set: GlueSetRatio::from_raw(1_500_000),
        glue_sign: Sign::Stretching,
        glue_order: Order::Fil,
        children: middle_id,
    })));
    let outer_id = outer.finish(&mut arena);

    assert_eq!(
        arena.get(inner_id, &survivors),
        &[Node::Char {
            font: FontId::testing_new(1),
            ch: 'x'
        }]
    );
    let [Node::HList(middle_box)] = arena.get(middle_id, &survivors) else {
        panic!("middle list should contain one hlist")
    };
    assert_eq!(middle_box.children, inner_id);
    assert_eq!(middle_box.glue_set, GlueSetRatio::ZERO);
    let [Node::VList(outer_box)] = arena.get(outer_id, &survivors) else {
        panic!("outer list should contain one vlist")
    };
    assert_eq!(outer_box.children, middle_id);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "child node-list span must be frozen below the parent span")]
fn bottom_up_debug_assert_fires_on_hand_constructed_violation() {
    let mut arena = NodeArena::new();
    let future_id = NodeListId::testing_epoch(0, 1);

    let mut builder = NodeListBuilder::new();
    builder.push(Node::Adjust(future_id));

    let _ = builder.finish(&mut arena);
}

#[test]
fn watermark_truncation_drops_exactly_the_suffix() {
    let mut arena = NodeArena::new();
    let survivors = crate::survivor::SurvivorArena::new();
    let kept = one_char(&mut arena, 'a');
    let mark = arena.watermark();
    let dropped = one_char(&mut arena, 'b');

    assert_eq!(arena.get(dropped, &survivors).len(), 1);
    arena.truncate_to(mark);

    assert_eq!(arena.get(kept, &survivors).len(), 1);
    assert!(!arena.contains(dropped));
    let replacement = one_char(&mut arena, 'c');
    assert_eq!(replacement.start(), dropped.start());
    assert_eq!(
        arena.get(replacement, &survivors)[0],
        Node::Char {
            font: FontId::testing_new(1),
            ch: 'c',
        }
    );
}

#[test]
fn builder_reuse_after_finish_leaves_buffer_empty() {
    let mut arena = NodeArena::new();
    let survivors = crate::survivor::SurvivorArena::new();
    let mut builder = NodeListBuilder::new();

    builder.push(Node::MathOn(Scaled::from_raw(0)));
    let first = builder.finish(&mut arena);
    assert!(builder.is_empty());

    builder.push(Node::MathOff(Scaled::from_raw(0)));
    let second = builder.finish(&mut arena);

    assert_eq!(
        arena.get(first, &survivors),
        &[Node::MathOn(Scaled::from_raw(0))]
    );
    assert_eq!(
        arena.get(second, &survivors),
        &[Node::MathOff(Scaled::from_raw(0))]
    );
    assert!(builder.is_empty());
}

fn one_char(arena: &mut NodeArena, ch: char) -> NodeListId {
    let mut builder = NodeListBuilder::new();
    builder.push(Node::Char {
        font: FontId::testing_new(1),
        ch,
    });
    builder.finish(arena)
}

fn scaled(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}
