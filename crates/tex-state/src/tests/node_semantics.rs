use crate::math::{MathChar, MathField, MathNoad, NoadKind};
use crate::node::{BoxLr, BoxNode, BoxNodeFields, DiscKind, Node, Sign};
use crate::node_arena::PageListId;
use crate::scaled::{GlueSetRatio, Scaled};
use crate::token::OriginId;
use std::hash::{Hash, Hasher};

fn semantic_hash(node: &Node) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    node.hash(&mut hasher);
    hasher.finish()
}

fn assert_same_semantics(left: Node, right: Node) {
    assert_eq!(left, right);
    assert_eq!(semantic_hash(&left), semantic_hash(&right));
}

#[test]
fn equality_and_hash_exclude_every_diagnostic_sidecar() {
    let sourced = OriginId::from_raw(41);
    assert_same_semantics(
        Node::Char {
            font: crate::font::NULL_FONT,
            ch: 'x',
            origin: OriginId::UNKNOWN,
        },
        Node::Char {
            font: crate::font::NULL_FONT,
            ch: 'x',
            origin: sourced,
        },
    );
    assert_same_semantics(
        Node::Lig {
            font: crate::font::NULL_FONT,
            ch: 'X',
            orig: vec!['a', 'b'],
            left_hit: false,
            right_hit: false,
            origins: vec![OriginId::UNKNOWN; 2],
        },
        Node::Lig {
            font: crate::font::NULL_FONT,
            ch: 'X',
            orig: vec!['a', 'b'],
            left_hit: false,
            right_hit: false,
            origins: vec![sourced],
        },
    );
    assert_same_semantics(
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: PageListId::empty(),
            post: PageListId::empty(),
            replace: PageListId::empty(),
            physical_replace_count: 1,
        },
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: PageListId::empty(),
            post: PageListId::empty(),
            replace: PageListId::empty(),
            physical_replace_count: 7,
        },
    );

    let boxed = |diagnostic_children, allocator_high_cell_overlap| {
        let mut boxed = BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(10),
            height: Scaled::from_raw(20),
            depth: Scaled::from_raw(3),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: crate::glue::Order::Normal,
            children: PageListId::empty(),
        });
        boxed.diagnostic_children = diagnostic_children;
        boxed.allocator_high_cell_overlap = allocator_high_cell_overlap;
        Node::HList(boxed)
    };
    assert_same_semantics(boxed(None, 0), boxed(Some(PageListId::empty()), 12));

    let math = |origin| {
        let character = MathChar {
            family: 2,
            character: 'x',
            origin,
        };
        Node::MathNoad(MathNoad::new(
            NoadKind::Accent { accent: character },
            MathField::MathChar(character),
        ))
    };
    assert_same_semantics(math(OriginId::UNKNOWN), math(sourced));
}

#[test]
fn equality_retains_semantic_fields() {
    assert_ne!(
        Node::<PageListId>::Char {
            font: crate::font::NULL_FONT,
            ch: 'x',
            origin: OriginId::UNKNOWN,
        },
        Node::<PageListId>::Char {
            font: crate::font::NULL_FONT,
            ch: 'y',
            origin: OriginId::from_raw(41),
        },
    );
}
