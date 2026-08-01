use super::*;
use tex_state::glue::Order;
use tex_state::ids::{GlueId, NodeListId};
use tex_state::node::{GlueKind, UnsetKind, UnsetNodeFields};

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw * Scaled::UNITY)
}

fn box_node(width: i32, height: i32, empty: NodeListId) -> BoxNode {
    BoxNode::new(BoxNodeFields {
        width: sp(width),
        height: sp(height),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: empty,
    })
}

#[derive(Clone, Copy)]
struct GlueTotals {
    stretch: i32,
    stretch_order: Order,
    shrink: i32,
    shrink_order: Order,
}

fn unset_cell(
    kind: UnsetKind,
    natural: i32,
    span_count: u16,
    glue: GlueTotals,
    empty: NodeListId,
) -> UnsetNode {
    let (width, height) = match kind {
        UnsetKind::HBox => (sp(natural), sp(1)),
        UnsetKind::VBox => (sp(1), sp(natural)),
    };
    UnsetNode::new(UnsetNodeFields {
        kind,
        width,
        height,
        depth: Scaled::from_raw(0),
        span_count,
        stretch: sp(glue.stretch),
        stretch_order: glue.stretch_order,
        shrink: sp(glue.shrink),
        shrink_order: glue.shrink_order,
        children: empty,
    })
}

#[test]
fn set_alignment_list_extends_running_rules_and_offsets_display_rules() {
    let mut stores = Universe::new_with_plain_catcodes();
    let empty = stores.freeze_node_list(&[]);

    let horizontal = set_alignment_nodes(
        AlignmentKind::HAlign,
        &[
            Node::Rule {
                width: None,
                height: Some(sp(2)),
                depth: Some(sp(1)),
            },
            Node::Rule {
                width: Some(sp(7)),
                height: Some(sp(3)),
                depth: Some(sp(1)),
            },
        ],
        &ResolvedWidths {
            columns: vec![sp(11)],
            tabskips: vec![GlueId::ZERO, GlueId::ZERO],
        },
        &Prototype {
            box_node: box_node(11, 13, empty),
        },
        empty,
        Scaled::from_raw(0),
        &mut stores,
    )
    .expect("running horizontal rules resolve");
    assert!(matches!(
        horizontal.as_slice(),
        [
            Node::Rule { width: Some(width), height: Some(_), depth: Some(_) },
            Node::Rule { width: Some(fixed), height: Some(_), depth: Some(_) },
        ] if *width == sp(11) && *fixed == sp(7)
    ));

    let vertical = set_alignment_nodes(
        AlignmentKind::VAlign,
        &[Node::Rule {
            width: Some(sp(2)),
            height: None,
            depth: Some(sp(1)),
        }],
        &ResolvedWidths {
            columns: vec![sp(13)],
            tabskips: vec![GlueId::ZERO, GlueId::ZERO],
        },
        &Prototype {
            box_node: box_node(11, 13, empty),
        },
        empty,
        Scaled::from_raw(0),
        &mut stores,
    )
    .expect("running vertical rules resolve");
    assert!(matches!(
        vertical.as_slice(),
        [Node::Rule { height: Some(height), .. }] if *height == sp(13)
    ));

    // TeX82 §806's second half: a nonzero §800 `o` wraps the rule in an
    // `hpack(q,natural)` whose `shift_amount` is `o`, because a rule node has
    // no shift field of its own.
    let shifted = set_alignment_nodes(
        AlignmentKind::HAlign,
        &[Node::Rule {
            width: None,
            height: Some(sp(2)),
            depth: Some(sp(1)),
        }],
        &ResolvedWidths {
            columns: vec![sp(11)],
            tabskips: vec![GlueId::ZERO, GlueId::ZERO],
        },
        &Prototype {
            box_node: box_node(11, 13, empty),
        },
        empty,
        sp(5),
        &mut stores,
    )
    .expect("display running rules resolve");
    let [Node::HList(wrapper)] = shifted.as_slice() else {
        panic!("a display alignment rule must be wrapped in a shifted hbox");
    };
    assert_eq!(wrapper.shift, sp(5));
    assert_eq!(wrapper.width, sp(11));
    let children = stores.nodes(wrapper.children).testing_decoded();
    let [
        Node::Rule {
            width: Some(width), ..
        },
    ] = children
    else {
        panic!("the wrapper must hold the one running rule");
    };
    assert_eq!(*width, sp(11));
}

#[test]
fn materialize_spanned_cell_adds_tabskip_and_empty_boxes() {
    let mut stores = Universe::new_with_plain_catcodes();
    let empty = stores.freeze_node_list(&[]);
    let middle = stores.intern_glue(GlueSpec {
        width: sp(1),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });
    let cell = unset_cell(
        UnsetKind::HBox,
        10,
        2,
        GlueTotals {
            stretch: 0,
            stretch_order: Order::Normal,
            shrink: 0,
            shrink_order: Order::Normal,
        },
        empty,
    );
    let row_children = stores.freeze_node_list(&[
        tabskip_node(GlueId::ZERO),
        Node::Unset(cell),
        tabskip_node(GlueId::ZERO),
    ]);
    let row = Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind: UnsetKind::HBox,
        width: sp(10),
        height: sp(2),
        depth: sp(1),
        span_count: 1,
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
        children: row_children,
    }));
    let resolved = ResolvedWidths {
        columns: vec![sp(4), sp(5)],
        tabskips: vec![GlueId::ZERO, middle, GlueId::ZERO],
    };
    let prototype = Prototype {
        box_node: box_node(10, 2, empty),
    };

    let set = set_alignment_nodes(
        AlignmentKind::HAlign,
        &[row],
        &resolved,
        &prototype,
        empty,
        sp(5),
        &mut stores,
    )
    .expect("spanned row sets");
    let [Node::HList(row)] = set.as_slice() else {
        panic!("unset row must become one hlist");
    };
    // TeX82 §807 closes with `shift_amount(q):=o`, the same §800 offset §806
    // gives a running rule.
    assert_eq!(row.shift, sp(5));
    let children = stores.nodes(row.children).testing_decoded();
    let [
        Node::Glue {
            kind: GlueKind::TabSkip,
            ..
        },
        Node::HList(first),
        Node::Glue {
            spec,
            kind: GlueKind::TabSkip,
            ..
        },
        Node::HList(blank),
        Node::Glue {
            kind: GlueKind::TabSkip,
            ..
        },
    ] = children
    else {
        panic!("span must add one tabskip/empty-box pair, got {children:?}");
    };
    assert_eq!(first.width, sp(4));
    assert_eq!(stores.glue(*spec).width, sp(1));
    assert_eq!(blank.width, sp(5));
    assert!(stores.nodes(blank.children).is_empty());
}

#[test]
fn set_alignment_preserves_final_node_order_and_running_rules() {
    // TeX82 §§803--807 traverse the alignment list in place: non-row nodes
    // retain their relative positions, running rules use prototype dimensions,
    // and each unset row becomes a set box at that same position.
    let mut stores = Universe::new_with_plain_catcodes();
    let empty = stores.freeze_node_list(&[]);
    let cell = Node::Unset(unset_cell(
        UnsetKind::HBox,
        4,
        1,
        GlueTotals {
            stretch: 0,
            stretch_order: Order::Normal,
            shrink: 0,
            shrink_order: Order::Normal,
        },
        empty,
    ));
    let children =
        stores.freeze_node_list(&[tabskip_node(GlueId::ZERO), cell, tabskip_node(GlueId::ZERO)]);
    let row = Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind: UnsetKind::HBox,
        width: sp(4),
        height: sp(2),
        depth: sp(1),
        span_count: 1,
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
        children,
    }));
    let marker = Node::Penalty(731);
    let set = set_alignment_nodes(
        AlignmentKind::HAlign,
        &[
            marker.clone(),
            Node::Rule {
                width: None,
                height: Some(sp(2)),
                depth: Some(sp(1)),
            },
            row,
            marker.clone(),
        ],
        &ResolvedWidths {
            columns: vec![sp(4)],
            tabskips: vec![GlueId::ZERO; 2],
        },
        &Prototype {
            box_node: box_node(9, 2, empty),
        },
        empty,
        Scaled::from_raw(0),
        &mut stores,
    )
    .expect("final traversal succeeds");
    assert!(
        matches!(set.as_slice(), [Node::Penalty(731), Node::Rule { width: Some(width), .. }, Node::HList(_), Node::Penalty(731)] if *width == sp(9))
    );
}

#[test]
fn convert_unset_cell_computes_tex82_glue_ratio_matrix() {
    let mut stores = Universe::new_with_plain_catcodes();
    let empty = stores.freeze_node_list(&[]);
    let ordinary = unset_cell(
        UnsetKind::HBox,
        5,
        1,
        GlueTotals {
            stretch: 10,
            stretch_order: Order::Fil,
            shrink: 2,
            shrink_order: Order::Normal,
        },
        empty,
    );

    assert_eq!(
        cell_glue_setting(AlignmentKind::HAlign, &ordinary, sp(5))
            .expect("equal target sets normally"),
        GlueSetting {
            ratio: GlueSetRatio::ZERO,
            sign: Sign::Normal,
            order: Order::Normal,
        }
    );
    assert_eq!(
        cell_glue_setting(AlignmentKind::HAlign, &ordinary, sp(10))
            .expect("larger target stretches"),
        GlueSetting {
            ratio: GlueSetRatio::from_ratio_parts(1, 2),
            sign: Sign::Stretching,
            order: Order::Fil,
        }
    );
    assert_eq!(
        cell_glue_setting(AlignmentKind::HAlign, &ordinary, Scaled::from_raw(0))
            .expect("excess normal shrink saturates"),
        GlueSetting {
            ratio: GlueSetRatio::UNITY,
            sign: Sign::Shrinking,
            order: Order::Normal,
        }
    );

    let infinite = unset_cell(
        UnsetKind::VBox,
        5,
        1,
        GlueTotals {
            stretch: 0,
            stretch_order: Order::Normal,
            shrink: 2,
            shrink_order: Order::Fill,
        },
        empty,
    );
    assert_eq!(
        cell_glue_setting(AlignmentKind::VAlign, &infinite, Scaled::from_raw(0))
            .expect("infinite-order shrink is not capped"),
        GlueSetting {
            ratio: GlueSetRatio::from_ratio_parts(5, 2),
            sign: Sign::Shrinking,
            order: Order::Fill,
        }
    );
}
