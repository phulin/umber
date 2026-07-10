use super::{NodeArena, NodeListBuilder, NodeRef, preflight_capacity};
use crate::glue::Order;
use crate::ids::{FontId, GlueId, NodeListId, TokenListId};
use crate::math::{
    FractionThickness, MathChoice, MathField, MathFraction, MathListNode, MathNoad, MathStyle,
    NoadClass, NoadKind,
};
use crate::node::{
    BoxNode, BoxNodeFields, DiscKind, GlueKind, KernKind, LeaderPayload, Node, Sign, UnsetKind,
    UnsetNode, UnsetNodeFields, Whatsit,
};
use crate::scaled::{GlueSetRatio, Scaled};

#[test]
fn node_layout_baseline() {
    assert_eq!(std::mem::size_of::<Node>(), 64);
    assert_eq!(std::mem::size_of::<BoxNode>(), 40);
    assert_eq!(std::mem::size_of::<crate::node::UnsetNode>(), 40);
    assert_eq!(std::mem::size_of::<crate::node::Whatsit>(), 48);
    assert_eq!(std::mem::size_of::<NodeListId>(), 8);
}

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
    let Some(NodeRef::HList(middle_box)) = arena.get(middle_id, &survivors).first() else {
        panic!("middle list should contain one hlist")
    };
    assert_eq!(middle_box.children, inner_id);
    assert_eq!(middle_box.glue_set, GlueSetRatio::ZERO);
    let Some(NodeRef::VList(outer_box)) = arena.get(outer_id, &survivors).first() else {
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
        arena.get(replacement, &survivors).first(),
        Some(NodeRef::Char {
            font: FontId::testing_new(1),
            ch: 'c',
        })
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

#[test]
fn every_inline_kind_uses_only_one_word_and_no_sidecar() {
    let mut arena = NodeArena::new();
    let nodes = vec![
        Node::Char {
            font: FontId::testing_new(u32::MAX),
            ch: '\u{10ffff}',
        },
        Node::Lig {
            font: FontId::testing_new(7),
            ch: '\u{ff}',
            orig: ('\0', '\u{fe}'),
        },
        Node::Kern {
            amount: Scaled::from_raw(i32::MIN),
            kind: KernKind::Mu,
        },
        Node::Glue {
            spec: GlueId::testing_new(u32::MAX),
            kind: GlueKind::NonScript,
            leader: None,
        },
        Node::Penalty(i32::MAX),
        Node::MathOn(Scaled::from_raw(i32::MIN)),
        Node::MathOff(Scaled::from_raw(i32::MAX)),
        Node::MathStyle(MathStyle::ScriptScript),
        Node::Nonscript,
    ];
    let id = arena.append(&nodes);
    assert_eq!(arena.get_epoch(id), nodes);
    assert_eq!(arena.storage.testing_sidecar_lengths(), [0; 13]);
    assert_eq!(arena.storage.testing_tags(), (0_u8..=8).collect::<Vec<_>>());
}

#[test]
fn byte_char_runs_stop_at_fonts_unicode_ligatures_and_other_nodes() {
    let mut arena = NodeArena::new();
    let f1 = FontId::testing_new(1);
    let f2 = FontId::testing_new(2);
    let id = arena.append(&[
        Node::Char { font: f1, ch: 'a' },
        Node::Char {
            font: f1,
            ch: '\u{ff}',
        },
        Node::Char { font: f2, ch: 'b' },
        Node::Char {
            font: f2,
            ch: '\u{100}',
        },
        Node::Char { font: f2, ch: 'c' },
        Node::Lig {
            font: f2,
            ch: 'd',
            orig: ('c', 'd'),
        },
        Node::Kern {
            amount: scaled(1),
            kind: KernKind::Font,
        },
    ]);
    let list = arena.get_epoch(id);
    let first = list.char_run(0).expect("first run");
    assert_eq!(first.font(), f1);
    assert_eq!(first.codes().collect::<Vec<_>>(), vec![b'a', 255]);
    assert_eq!(
        list.char_run(2)
            .expect("second run")
            .codes()
            .collect::<Vec<_>>(),
        vec![b'b']
    );
    assert!(list.char_run(3).is_none());
    assert_eq!(
        list.char_run(4)
            .expect("post-Unicode run")
            .codes()
            .collect::<Vec<_>>(),
        vec![b'c']
    );
    assert!(list.char_run(5).is_none());
    assert!(list.char_run(list.len()).is_none());
}

#[test]
fn every_rare_kind_round_trips_through_its_sidecar() {
    let mut arena = NodeArena::new();
    let empty = arena.append(&[]);
    let box_node = BoxNode::new(BoxNodeFields {
        width: scaled(1),
        height: scaled(2),
        depth: scaled(3),
        shift: scaled(4),
        display: true,
        glue_set: GlueSetRatio::from_raw(5),
        glue_sign: Sign::Shrinking,
        glue_order: Order::Fill,
        children: empty,
    });
    let unset = UnsetNode::new(UnsetNodeFields {
        kind: UnsetKind::VBox,
        width: scaled(6),
        height: scaled(7),
        depth: scaled(8),
        span_count: 9,
        stretch: scaled(10),
        stretch_order: Order::Filll,
        shrink: scaled(11),
        shrink_order: Order::Fil,
        children: empty,
    });
    let nodes = vec![
        Node::HList(box_node),
        Node::VList(box_node),
        Node::Unset(unset),
        Node::Rule {
            width: Some(scaled(12)),
            height: None,
            depth: Some(scaled(13)),
        },
        Node::Glue {
            spec: GlueId::testing_new(2),
            kind: GlueKind::Leaders,
            leader: Some(LeaderPayload::Rule {
                width: None,
                height: Some(scaled(14)),
                depth: None,
            }),
        },
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty,
            post: empty,
            replace: empty,
        },
        Node::Mark {
            class: u16::MAX,
            tokens: TokenListId::testing_new(3),
        },
        Node::Ins {
            class: 4,
            size: scaled(15),
            split_top_skip: GlueId::testing_new(5),
            split_max_depth: scaled(16),
            floating_penalty: -17,
            content: empty,
        },
        Node::Whatsit(Whatsit::Language {
            language: 18,
            left_hyphen_min: 2,
            right_hyphen_min: 3,
        }),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )),
        Node::FractionNoad(MathFraction {
            numerator: empty,
            denominator: empty,
            thickness: FractionThickness::Explicit(scaled(19)),
            left_delimiter: Some(20),
            right_delimiter: None,
        }),
        Node::MathChoice(MathChoice {
            display: empty,
            text: empty,
            script: empty,
            script_script: empty,
        }),
        Node::MathList(MathListNode {
            display: true,
            content: empty,
        }),
        Node::Adjust(empty),
    ];
    let id = arena.append(&nodes);
    assert_eq!(arena.get_epoch(id), nodes);
    assert_eq!(
        arena.storage.testing_sidecar_lengths(),
        [2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]
    );
    assert_eq!(
        arena.storage.testing_tags(),
        (9_u8..=22).collect::<Vec<_>>()
    );
}

#[test]
fn rollback_truncates_words_and_every_sidecar_without_a_decoded_mirror() {
    let mut arena = NodeArena::new();
    let empty = arena.append(&[]);
    let mark = arena.watermark();
    let rare = [
        Node::Adjust(empty),
        Node::Rule {
            width: None,
            height: None,
            depth: None,
        },
    ];
    let dropped = arena.append(&rare);
    assert_eq!(arena.storage.testing_sidecar_lengths()[2], 1);
    assert_eq!(arena.storage.testing_sidecar_lengths()[12], 1);
    arena.truncate_to(mark);
    assert!(!arena.contains(dropped));
    assert_eq!(arena.storage.testing_sidecar_lengths(), [0; 13]);
    assert!(arena.storage.all_nodes().is_empty());
}

#[test]
fn capacity_preflight_accepts_boundary_without_mutation() {
    assert_eq!(preflight_capacity(u32::MAX - 1, 1, "overflow"), u32::MAX);
}

#[test]
#[should_panic(expected = "sidecar overflow")]
fn capacity_preflight_rejects_overflow_before_publication() {
    let _ = preflight_capacity(u32::MAX, 1, "sidecar overflow");
}

#[test]
#[should_panic(expected = "ligature glyph exceeds TFM byte domain")]
fn ligature_inline_encoding_rejects_non_tfm_character() {
    let mut arena = NodeArena::new();
    arena.append(&[Node::Lig {
        font: FontId::testing_new(0),
        ch: '\u{100}',
        orig: ('a', 'b'),
    }]);
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
