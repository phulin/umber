use super::*;
use tex_state::node::{GlueKind, KernKind};

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

#[test]
fn badness_matches_tex_web_boundaries() {
    assert_eq!(badness(sp(0), sp(0)), 0);
    assert_eq!(badness(sp(1), sp(0)), INF_BAD);
    assert_eq!(badness(sp(1), sp(1)), 100);
    assert_eq!(badness(sp(1), sp(4)), 2);
    assert_eq!(badness(sp(1290), sp(297)), 8189);
    assert_eq!(badness(sp(1291), sp(297)), INF_BAD);
    assert_eq!(badness(sp(7_230_584), sp(1)), INF_BAD);
    assert_eq!(badness(sp(7_230_585), sp(1_663_497)), 8189);
}

#[test]
fn hpack_sets_finite_stretch_order_and_ratio() {
    let mut universe = Universe::new();
    let glue = universe.intern_glue(GlueSpec {
        width: sp(10),
        stretch: sp(5),
        stretch_order: Order::Fil,
        shrink: sp(2),
        shrink_order: Order::Normal,
    });
    let list = universe.freeze_node_list(&[
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
        },
    ]);
    let packed = hpack(
        &universe,
        list,
        PackSpec::Exactly(sp(40)),
        HpackParams {
            hbadness: INF_BAD,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );
    assert_eq!(packed.node.width, sp(40));
    assert_eq!(packed.node.glue_sign, Sign::Stretching);
    assert_eq!(packed.node.glue_order, Order::Fil);
    assert_eq!(packed.node.glue_set, GlueSetRatio::from_raw(2_000_000));
}

#[test]
fn hpack_clamps_overfull_normal_shrink_ratio_to_one() {
    let mut universe = Universe::new();
    let glue = universe.intern_glue(GlueSpec {
        width: sp(10),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(2),
        shrink_order: Order::Normal,
    });
    let list = universe.freeze_node_list(&[
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
        },
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
    ]);

    let packed = hpack(
        &universe,
        list,
        PackSpec::Exactly(sp(40)),
        HpackParams {
            hbadness: 0,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );

    assert_eq!(packed.node.glue_sign, Sign::Shrinking);
    assert_eq!(packed.node.glue_order, Order::Normal);
    assert_eq!(packed.node.glue_set, GlueSetRatio::UNITY);
    assert_eq!(
        packed.diagnostics,
        vec![PackDiagnostic::Overfull { excess: sp(10) }]
    );
}

#[test]
fn hpack_measures_shifted_child_boxes() {
    let mut universe = Universe::new();
    let child = universe.freeze_node_list(&[]);
    let raised = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(5),
        height: sp(10),
        depth: sp(3),
        shift: sp(4),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }));
    let lowered = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(5),
        height: sp(10),
        depth: sp(3),
        shift: sp(-6),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }));
    let list = universe.freeze_node_list(&[raised, lowered]);

    let packed = hpack(
        &universe,
        list,
        PackSpec::Natural,
        HpackParams {
            hbadness: INF_BAD,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );

    assert_eq!(packed.node.height, sp(14));
    assert_eq!(packed.node.depth, sp(9));
}

#[test]
fn vpack_clamps_depth_to_box_max_depth() {
    let mut universe = Universe::new();
    let child = universe.freeze_node_list(&[]);
    let list = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(5),
        height: sp(10),
        depth: sp(8),
        shift: sp(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }))]);
    let packed = vpack(
        &universe,
        list,
        PackSpec::Natural,
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: sp(0),
            box_max_depth: sp(3),
        },
    );
    assert_eq!(packed.node.height, sp(15));
    assert_eq!(packed.node.depth, sp(3));
}

#[test]
fn vertical_spacing_consumes_previous_depth() {
    let mut universe = Universe::new();
    let child = universe.freeze_node_list(&[]);
    let glue = universe.intern_glue(GlueSpec {
        width: sp(7),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
    let hbox = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(6),
        height: sp(4),
        depth: sp(1),
        shift: sp(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }));
    let list = universe.freeze_node_list(&[
        hbox.clone(),
        Node::Glue {
            spec: glue,
            kind: GlueKind::BaselineSkip,
        },
        hbox,
    ]);
    let packed = vpack(
        &universe,
        list,
        PackSpec::Natural,
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: sp(0),
            box_max_depth: sp(100),
        },
    );
    assert_eq!(packed.node.height, sp(16));
    assert_eq!(packed.node.depth, sp(1));
}

#[test]
fn packed_box_can_round_trip_through_survivor_box_register() {
    let mut universe = Universe::new();
    let list = universe.freeze_node_list(&[Node::Kern {
        amount: sp(12),
        kind: KernKind::Explicit,
    }]);
    let packed = hpack(
        &universe,
        list,
        PackSpec::Natural,
        HpackParams {
            hbadness: INF_BAD,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );
    let boxed = universe.freeze_node_list(&[Node::HList(packed.node)]);
    universe.set_box_reg(0, boxed);
    let survivor = universe.box_reg(0).expect("box should be stored");
    assert!(matches!(universe.nodes(survivor), [Node::HList(_)]));
}
