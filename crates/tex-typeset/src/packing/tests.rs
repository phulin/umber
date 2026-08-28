use super::*;
use crate::test_state::TestState;
use tex_fonts::metrics::CharTag;
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont};
use tex_state::glue::GlueSpec;
use tex_state::node::{GlueKind, KernKind, LeaderPayload, Whatsit};

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
fn badness_is_bounded_and_monotonic() {
    assert_eq!(badness(sp(0), sp(0)), 0);
    assert_eq!(badness(sp(1), sp(0)), INF_BAD);
    assert_eq!(badness(sp(1290), sp(297)), 8189);
    assert_eq!(badness(sp(1291), sp(297)), INF_BAD);

    let mut previous = badness(sp(0), sp(297));
    for excess in 1..=5_000 {
        let current = badness(sp(excess), sp(297));
        assert!(
            (0..=INF_BAD).contains(&current),
            "badness({excess}, 297) was {current}"
        );
        assert!(
            current >= previous,
            "badness decreased from {previous} to {current} at excess {excess}"
        );
        previous = current;
    }

    for excess in [1, 1_291, 7_230_584, 7_230_585, i32::MAX] {
        let mut previous = badness(sp(excess), sp(0));
        for shrink in 1..=5_000 {
            let current = badness(sp(excess), sp(shrink));
            assert!(
                (0..=INF_BAD).contains(&current),
                "badness({excess}, {shrink}) was {current}"
            );
            assert!(
                current <= previous,
                "badness increased from {previous} to {current} at shrink {shrink} for excess {excess}"
            );
            previous = current;
        }
    }
}

fn width_font(name: &str, salt: u8) -> LoadedFont {
    let mut characters = vec![None; 256];
    for code in 0_u8..=u8::MAX {
        if code % 11 != 0 {
            characters[usize::from(code)] = Some(CharMetrics {
                width: sp(i32::from(code).wrapping_mul(7919).wrapping_sub(700_000)),
                height: sp(i32::from(code % 17)),
                depth: sp(i32::from(code % 7)),
                italic_correction: sp(0),
                tag: CharTag::None,
            });
        }
    }
    LoadedFont::new(
        name,
        name,
        [salt; 8],
        0,
        sp(10),
        sp(10),
        vec![sp(0); 7],
        FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
    )
}

fn scalar_hlist(state: &impl TypesetState, nodes: &[Node]) -> Measurement {
    let mut out = Measurement::ZERO;
    for node in nodes.iter().map(NodeRef::from) {
        match node {
            NodeRef::Char { font, ch, .. } | NodeRef::Lig { font, ch, .. } => {
                if let Ok(code) = u8::try_from(ch as u32)
                    && let Some(metric) = state.font_char_metrics(font, code)
                {
                    out.width = add(out.width, metric.width);
                    out.height = out.height.max(metric.height);
                    out.depth = out.depth.max(metric.depth);
                }
            }
            NodeRef::Kern { amount, .. } => out.width = add(out.width, amount),
            NodeRef::Glue { spec, .. } => {
                out.observe_horizontal(MetricEvent::Glue(spec), MetricOverflow::PACKING);
            }
            NodeRef::MathOn(width) | NodeRef::MathOff(width) => out.width = add(out.width, width),
            NodeRef::Penalty(_) => {}
            _ => {}
        }
    }
    out
}

#[test]
fn compact_char_runs_differentially_match_scalar_mixed_lists() {
    let mut universe = TestState::new();
    let fonts = [
        universe.intern_font(width_font("width-a", 1)),
        universe.intern_font(width_font("width-b", 2)),
    ];
    let glue = GlueSpec {
        width: sp(31337),
        ..GlueSpec::ZERO
    };
    let mut seed = 0x8bad_f00d_dead_beef_u64;
    for case in 0..256 {
        let mut nodes = Vec::new();
        for _ in 0..(case % 97) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let font = fonts[(seed as usize >> 8) & 1];
            let code = seed as u8;
            match (seed >> 16) % 13 {
                0 => nodes.push(Node::Char {
                    font,
                    ch: '\u{100}',
                    origin: tex_state::token::OriginId::UNKNOWN,
                }),
                1 => nodes.push(Node::Lig {
                    font,
                    ch: char::from(code),
                    orig: vec!['a', 'b'],
                    origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
                    left_hit: false,
                    right_hit: false,
                }),
                2 => nodes.push(Node::Kern {
                    amount: sp((seed as i32) % 100_000),
                    kind: KernKind::Font,
                }),
                3 => nodes.push(Node::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                }),
                4 => nodes.push(Node::Penalty(seed as i32)),
                _ => nodes.push(Node::Char {
                    font,
                    ch: char::from(code),
                    origin: tex_state::token::OriginId::UNKNOWN,
                }),
            }
        }
        let id = universe.publish_page_nodes(&nodes);
        let view = universe
            .page_node_list(id)
            .expect("test list belongs to the page arena")
            .nodes();
        let fast = measure_hlist(&universe, view);
        let owned = view.iter().cloned().collect::<Vec<_>>();
        let scalar = scalar_hlist(&universe, &owned);
        let params = HpackParams {
            hbadness: case % 10_001,
            hfuzz: sp(case),
            overfull_rule: sp(0),
        };
        let spec = PackSpec::Natural;
        let decoded = plan_hpack_nodes(&universe, &nodes, spec, params).finish(id);
        let compact = hpack(&universe, id, spec, params);
        assert_eq!(fast.width, scalar.width, "width case {case}");
        assert_eq!(fast.height, scalar.height, "height case {case}");
        assert_eq!(fast.depth, scalar.depth, "depth case {case}");
        assert_eq!(fast.has_glue, scalar.has_glue, "glue case {case}");
        assert_eq!(decoded, compact, "packing case {case}");
    }
}

#[test]
#[should_panic(expected = "packed dimension overflow must be reported, not saturated")]
fn packing_overflow_fails_loudly() {
    let mut universe = TestState::new();
    let nodes = vec![
        Node::Kern {
            amount: sp(i32::MAX),
            kind: KernKind::Explicit,
        },
        Node::Kern {
            amount: sp(1),
            kind: KernKind::Explicit,
        },
    ];
    let id = universe.publish_page_nodes(&nodes);
    let _ = hpack(
        &universe,
        id,
        PackSpec::Natural,
        HpackParams {
            hbadness: 0,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );
}

#[test]
fn hpack_records_zero_badness_for_empty_underfull_box() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let empty_packed = hpack(
        &universe,
        empty,
        PackSpec::Exactly(sp(10)),
        HpackParams {
            hbadness: 0,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );
    assert_eq!(empty_packed.badness, 0);
    assert!(empty_packed.diagnostics.is_empty());

    let zero_glue = GlueSpec {
        width: sp(0),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
    let list = universe.publish_page_nodes(&[Node::Glue {
        spec: zero_glue,
        kind: GlueKind::Normal,
        leader: None,
    }]);
    let glue_packed = hpack(
        &universe,
        list,
        PackSpec::Exactly(sp(10)),
        HpackParams {
            hbadness: 0,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );
    assert_eq!(glue_packed.badness, INF_BAD);

    let kern_list = universe.publish_page_nodes(&[Node::Kern {
        amount: sp(1),
        kind: KernKind::Explicit,
    }]);
    let kern_packed = hpack(
        &universe,
        kern_list,
        PackSpec::Exactly(sp(10)),
        HpackParams {
            hbadness: INF_BAD,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );
    assert_eq!(kern_packed.badness, INF_BAD);
}

#[test]
fn hpack_sets_finite_stretch_order_and_ratio() {
    let mut universe = TestState::new();
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(5),
        stretch_order: Order::Fil,
        shrink: sp(2),
        shrink_order: Order::Normal,
    };
    let list = universe.publish_page_nodes(&[
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
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

/// tex.web §§645--646: `additional` makes the target `x+natural`, and
/// the first nonzero glue total found while descending `filll..normal` wins.
#[test]
fn spread_target_and_highest_glue_order() {
    let mut universe = TestState::new();
    let glues = [
        (Order::Normal, 2),
        (Order::Fil, 3),
        (Order::Fill, 4),
        (Order::Filll, 5),
    ]
    .map(|(order, amount)| GlueSpec {
        width: sp(1),
        stretch: sp(amount),
        stretch_order: order,
        shrink: sp(amount),
        shrink_order: order,
    });
    let nodes: Vec<_> = glues
        .into_iter()
        .map(|spec| Node::Glue {
            spec,
            kind: GlueKind::Normal,
            leader: None,
        })
        .collect();
    let list = universe.publish_page_nodes(&nodes);
    let params = HpackParams {
        hbadness: INF_BAD,
        hfuzz: sp(0),
        overfull_rule: sp(0),
    };

    let stretched = hpack(&universe, list, PackSpec::Spread(sp(10)), params);
    assert_eq!(stretched.node.width, sp(14));
    assert_eq!(stretched.node.glue_sign, Sign::Stretching);
    assert_eq!(stretched.node.glue_order, Order::Filll);
    assert_eq!(
        stretched.node.glue_set,
        GlueSetRatio::from_scaled_ratio(sp(10), sp(5))
    );

    let shrunk = hpack(&universe, list, PackSpec::Spread(sp(-10)), params);
    assert_eq!(shrunk.node.width, sp(-6));
    assert_eq!(shrunk.node.glue_sign, Sign::Shrinking);
    assert_eq!(shrunk.node.glue_order, Order::Filll);
    assert_eq!(
        shrunk.node.glue_set,
        GlueSetRatio::from_scaled_ratio(sp(10), sp(5))
    );
}

#[test]
fn hpack_infinite_shrink_has_zero_badness_and_no_diagnostic() {
    let mut universe = TestState::new();
    let hss = GlueSpec {
        width: sp(0),
        stretch: sp(1),
        stretch_order: Order::Fil,
        shrink: sp(1),
        shrink_order: Order::Fil,
    };
    let list = universe.publish_page_nodes(&[
        Node::Glue {
            spec: hss,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
    ]);

    let packed = hpack(
        &universe,
        list,
        PackSpec::Exactly(sp(0)),
        HpackParams {
            hbadness: 0,
            hfuzz: sp(0),
            overfull_rule: sp(5),
        },
    );

    assert_eq!(packed.badness, 0);
    assert_eq!(packed.node.glue_sign, Sign::Shrinking);
    assert_eq!(packed.node.glue_order, Order::Fil);
    assert!(packed.diagnostics.is_empty());
}

#[test]
fn leader_glue_participates_in_packing_like_ordinary_glue() {
    let mut universe = TestState::new();
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(5),
        stretch_order: Order::Normal,
        shrink: sp(2),
        shrink_order: Order::Normal,
    };
    let empty = universe.publish_page_nodes(&[]);
    let payload = LeaderPayload::HList(BoxNode::new(BoxNodeFields {
        width: sp(3),
        height: sp(1),
        depth: sp(0),
        shift: sp(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: empty,
    }));
    let hlist = universe.publish_page_nodes(&[Node::Glue {
        spec: glue,
        kind: GlueKind::Xleaders,
        leader: Some(payload),
    }]);

    let packed = hpack(
        &universe,
        hlist,
        PackSpec::Exactly(sp(20)),
        HpackParams {
            hbadness: INF_BAD,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );

    assert_eq!(packed.node.width, sp(20));
    assert_eq!(packed.node.height, sp(1));
    assert_eq!(packed.node.depth, sp(0));
    assert_eq!(packed.node.glue_sign, Sign::Stretching);
    assert_eq!(packed.node.glue_order, Order::Normal);
    assert_eq!(packed.node.glue_set, GlueSetRatio::from_raw(2_000_000));

    let vlist = universe.publish_page_nodes(&[Node::Glue {
        spec: glue,
        kind: GlueKind::Cleaders,
        leader: Some(LeaderPayload::Rule {
            width: Some(sp(4)),
            height: Some(sp(1)),
            depth: Some(sp(0)),
        }),
    }]);
    let packed = vpack(
        &universe,
        vlist,
        PackSpec::Exactly(sp(20)),
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: sp(0),
            box_max_depth: sp(0),
        },
    );

    assert_eq!(packed.node.height, sp(20));
    assert_eq!(packed.node.width, sp(4));
    assert_eq!(packed.node.glue_sign, Sign::Stretching);
    assert_eq!(packed.node.glue_order, Order::Normal);
    assert_eq!(packed.node.glue_set, GlueSetRatio::from_raw(2_000_000));
}

#[test]
fn hpack_clamps_overfull_normal_shrink_ratio_to_one() {
    let mut universe = TestState::new();
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(2),
        shrink_order: Order::Normal,
    };
    let list = universe.publish_page_nodes(&[
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
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
        vec![PackDiagnostic::Overfull { excess: sp(8) }]
    );
}

#[test]
fn hpack_reports_insufficient_normal_shrink_even_below_infinite_badness() {
    let mut universe = TestState::new();
    let glue = GlueSpec {
        width: sp(8),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(4),
        shrink_order: Order::Normal,
    };
    let list = universe.publish_page_nodes(&[
        Node::Kern {
            amount: sp(9),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
    ]);

    let packed = hpack(
        &universe,
        list,
        PackSpec::Exactly(sp(10)),
        HpackParams {
            hbadness: 0,
            hfuzz: sp(2),
            overfull_rule: sp(5),
        },
    );

    assert_eq!(packed.node.glue_set, GlueSetRatio::UNITY);
    assert_eq!(packed.badness, OVERFULL_BADNESS);
    assert_eq!(
        packed.diagnostics,
        vec![PackDiagnostic::Overfull { excess: sp(3) }]
    );
}

#[test]
fn vpack_records_overfull_badness_when_normal_shrink_is_insufficient() {
    let mut universe = TestState::new();
    let list = universe.publish_page_nodes(&[Node::Kern {
        amount: sp(20),
        kind: KernKind::Explicit,
    }]);
    let packed = vpack(
        &universe,
        list,
        PackSpec::Exactly(sp(10)),
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: sp(20),
            box_max_depth: sp(100),
        },
    );

    assert_eq!(packed.badness, OVERFULL_BADNESS);
    assert!(packed.diagnostics.is_empty());
}

#[test]
fn hpack_measures_shifted_child_boxes() {
    let mut universe = TestState::new();
    let child = universe.publish_page_nodes(&[]);
    let raised = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(5),
        height: sp(10),
        depth: sp(3),
        shift: sp(-4),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }));
    let lowered = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(5),
        height: sp(10),
        depth: sp(3),
        shift: sp(6),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }));
    let list = universe.publish_page_nodes(&[raised, lowered]);

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
fn vpack_measures_shifted_child_width() {
    let mut universe = TestState::new();
    let child = universe.publish_page_nodes(&[]);
    let list = universe.publish_page_nodes(&[Node::VList(BoxNode::new(BoxNodeFields {
        width: sp(4),
        height: sp(9),
        depth: sp(7),
        shift: sp(2),
        box_lr: tex_state::node::BoxLr::Normal,
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
            box_max_depth: sp(100),
        },
    );

    assert_eq!(packed.node.width, sp(6));
}

#[test]
fn pdf_image_reference_contributes_its_declared_box_dimensions() {
    let mut universe = TestState::new();
    let image = Node::Whatsit(Whatsit::PdfRefXImage {
        object: 1,
        width: sp(30),
        height: sp(20),
        depth: sp(5),
    });

    let hplan = plan_hpack_nodes(
        &universe,
        std::slice::from_ref(&image),
        PackSpec::Natural,
        HpackParams {
            hbadness: INF_BAD,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );
    assert_eq!(
        (hplan.width, hplan.height, hplan.depth),
        (sp(30), sp(20), sp(5))
    );

    let list = universe.publish_page_nodes(&[image]);
    let hbox = hpack(
        &universe,
        list,
        PackSpec::Natural,
        HpackParams {
            hbadness: INF_BAD,
            hfuzz: sp(0),
            overfull_rule: sp(0),
        },
    );
    assert_eq!(
        (hbox.node.width, hbox.node.height, hbox.node.depth),
        (sp(30), sp(20), sp(5))
    );

    let vbox = vpack(
        &universe,
        list,
        PackSpec::Natural,
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: sp(0),
            box_max_depth: sp(100),
        },
    );
    assert_eq!(
        (vbox.node.width, vbox.node.height, vbox.node.depth),
        (sp(30), sp(20), sp(5))
    );
}

#[test]
fn vpack_clamps_depth_to_box_max_depth() {
    let mut universe = TestState::new();
    let child = universe.publish_page_nodes(&[]);
    let list = universe.publish_page_nodes(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(5),
        height: sp(10),
        depth: sp(8),
        shift: sp(0),
        box_lr: tex_state::node::BoxLr::Normal,
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
fn vtop_with_leading_glue_has_zero_height() {
    let mut universe = TestState::new();
    let child = universe.publish_page_nodes(&[]);
    let glue = GlueSpec {
        width: sp(7),
        ..GlueSpec::ZERO
    };
    let list = universe.publish_page_nodes(&[
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::HList(BoxNode::new(BoxNodeFields {
            width: sp(5),
            height: sp(10),
            depth: sp(3),
            shift: sp(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: child,
        })),
    ]);

    let packed = vtop(
        &universe,
        list,
        PackSpec::Natural,
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: sp(0),
            box_max_depth: sp(100),
        },
    );

    assert_eq!(packed.node.height, sp(0));
    assert_eq!(packed.node.depth, sp(20));
}

#[test]
fn vtop_preserves_total_size_when_first_box_exceeds_target() {
    let mut universe = TestState::new();
    let child = universe.publish_page_nodes(&[]);
    let list = universe.publish_page_nodes(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(5),
        height: sp(10),
        depth: sp(3),
        shift: sp(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }))]);

    let packed = vtop(
        &universe,
        list,
        PackSpec::Exactly(sp(5)),
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: sp(0),
            box_max_depth: sp(100),
        },
    );

    assert_eq!(packed.node.height, sp(10));
    assert_eq!(packed.node.depth, sp(-2));
}

#[test]
fn vertical_spacing_consumes_previous_depth() {
    let mut universe = TestState::new();
    let child = universe.publish_page_nodes(&[]);
    let glue = GlueSpec {
        width: sp(7),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
    let hbox = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(6),
        height: sp(4),
        depth: sp(1),
        shift: sp(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }));
    let list = universe.publish_page_nodes(&[
        hbox.clone(),
        Node::Glue {
            spec: glue,
            kind: GlueKind::BaselineSkip,

            leader: None,
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
fn packed_box_can_round_trip_through_structural_box_register() {
    let mut universe = TestState::new();
    let list = universe.publish_page_nodes(&[Node::Kern {
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
    let boxed = universe.publish_page_nodes(&[Node::HList(packed.node)]);
    universe.assign_page_box_local(0, boxed);
    let owner = universe.copy_box_to_page(0).expect("box should be stored");
    assert!(matches!(
        universe
            .page_node_list(owner)
            .expect("copied box belongs to the page arena")
            .get(0),
        Some(NodeRef::HList(_))
    ));
}
