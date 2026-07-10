use super::*;
use tex_state::Universe;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{DiscKind, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

fn params(width: i32) -> LineBreakParams {
    LineBreakParams {
        pretolerance: 100,
        tolerance: 1000,
        line_penalty: 10,
        hyphen_penalty: 50,
        ex_hyphen_penalty: 50,
        adj_demerits: 10_000,
        double_hyphen_demerits: 10_000,
        final_hyphen_demerits: 5_000,
        emergency_stretch: sp(0),
        looseness: 0,
        left_skip: GlueSpec::ZERO,
        right_skip: GlueSpec::ZERO,
        shape: LineShape::natural(sp(width)),
    }
}

fn kern(width: i32) -> Node {
    Node::Kern {
        amount: sp(width),
        kind: KernKind::Explicit,
    }
}

fn rule(width: i32) -> Node {
    Node::Rule {
        width: Some(sp(width)),
        height: None,
        depth: None,
    }
}

#[test]
fn breaks_at_legal_glue() {
    let mut universe = Universe::new();
    let glue = universe.intern_glue(GlueSpec {
        width: sp(10),
        stretch: sp(10),
        stretch_order: Order::Normal,
        shrink: sp(5),
        shrink_order: Order::Normal,
    });
    let nodes = vec![
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
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
        },
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
    ];
    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, params(30), &mut hook);
    assert_eq!(
        result.breaks.last().map(|br| br.position),
        Some(nodes.len())
    );
}

#[test]
fn line_break_includes_left_and_right_skip_in_background_widths() {
    let mut universe = Universe::new();
    let break_glue = universe.intern_glue(GlueSpec::ZERO);
    let nodes = vec![
        rule(80),
        Node::Glue {
            spec: break_glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(80),
    ];
    let mut params = params(100);
    params.left_skip = GlueSpec {
        width: sp(10),
        ..GlueSpec::ZERO
    };
    params.right_skip = params.left_skip;

    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, params, &mut hook);

    assert_eq!(result.breaks[0].position, 2);
    assert_eq!(result.breaks.len(), 2);
}

#[test]
fn equal_demerits_prefer_later_route_in_same_line_and_fitness_class() {
    let candidate = |position, fitness| Candidate {
        position,
        width_position: position,
        penalty: 0,
        line: 2,
        fitness,
        demerits: 221,
        path_demerits: 221,
        previous: Some(0),
        hyphenated: false,
    };
    let candidates = vec![
        candidate(0, Fitness::Decent),
        candidate(4, Fitness::Decent),
        candidate(6, Fitness::Decent),
        candidate(6, Fitness::Loose),
    ];
    let mut best = Vec::new();

    record_best_candidate(&mut best, &candidates, 1);
    record_best_candidate(&mut best, &candidates, 2);
    record_best_candidate(&mut best, &candidates, 3);

    assert_eq!(best, vec![2, 3]);
}

#[test]
fn parshape_repeats_last_line_and_overrides_hanging() {
    let shape = LineShape {
        hsize: sp(100),
        parshape: Some(ParagraphShape {
            lines: vec![
                LineShapeEntry {
                    indent: sp(3),
                    width: sp(40),
                },
                LineShapeEntry {
                    indent: sp(5),
                    width: sp(30),
                },
            ],
        }),
        hang_indent: sp(20),
        hang_after: 0,
        line_offset: 0,
    };

    assert_eq!(
        shape.dimensions(1),
        LineDimensions {
            indent: sp(3),
            width: sp(40),
        }
    );
    assert_eq!(
        shape.dimensions(3),
        LineDimensions {
            indent: sp(5),
            width: sp(30),
        }
    );
}

#[test]
fn hangindent_selects_affected_lines() {
    let mut shape = LineShape {
        hsize: sp(100),
        parshape: None,
        hang_indent: sp(25),
        hang_after: 1,
        line_offset: 0,
    };
    assert_eq!(
        shape.dimensions(1),
        LineDimensions {
            indent: sp(0),
            width: sp(100),
        }
    );
    assert_eq!(
        shape.dimensions(2),
        LineDimensions {
            indent: sp(25),
            width: sp(75),
        }
    );

    shape.hang_indent = sp(-25);
    shape.hang_after = -2;
    assert_eq!(
        shape.dimensions(1),
        LineDimensions {
            indent: sp(0),
            width: sp(75),
        }
    );
    assert_eq!(
        shape.dimensions(3),
        LineDimensions {
            indent: sp(0),
            width: sp(100),
        }
    );
}

#[test]
fn break_glue_does_not_contribute_to_preceding_line_width() {
    let mut universe = Universe::new();
    let glue = universe.intern_glue(GlueSpec {
        width: sp(1000),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
    let nodes = vec![
        rule(20),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
        },
        rule(20),
    ];
    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, params(20), &mut hook);
    assert_eq!(result.breaks.first().map(|br| br.position), Some(2));
}

#[test]
fn mathoff_breaks_only_before_following_glue_and_zeroes_break_width() {
    let mut universe = Universe::new();
    let glue = universe.intern_glue(GlueSpec {
        width: sp(1000),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
    let nodes = vec![
        rule(10),
        Node::MathOff(sp(5)),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
        },
        rule(10),
    ];
    let breakpoints = legal_breakpoints(&universe, &nodes, &params(15), false);

    assert_eq!(breakpoints.first().map(|br| br.position), Some(2));
    let zero = universe.intern_glue(GlueSpec::ZERO);
    let breaks = vec![
        BreakDecision {
            position: 2,
            penalty: 0,
            hyphenated: false,
        },
        BreakDecision {
            position: nodes.len(),
            penalty: -10_000,
            hyphenated: false,
        },
    ];
    let lines = post_line_break(
        &universe,
        &nodes,
        &breaks,
        PostLineBreakParams {
            left_skip: zero,
            right_skip: zero,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalty: 0,
            broken_penalty: 0,
            shape: LineShape::natural(sp(15)),
        },
    );
    assert!(
        lines[0]
            .nodes
            .iter()
            .any(|node| matches!(node, Node::MathOff(width) if width.raw() == 0))
    );

    let nodes_without_glue = vec![rule(10), Node::MathOff(sp(5)), rule(10)];
    let breakpoints = legal_breakpoints(&universe, &nodes_without_glue, &params(15), false);
    assert!(!breakpoints.iter().any(|br| br.position == 2));
}

#[test]
fn final_pass_deactivates_unshrinkable_active_line() {
    let mut universe = Universe::new();
    let glue = universe.intern_glue(GlueSpec {
        width: sp(10),
        stretch: sp(10),
        stretch_order: Order::Normal,
        shrink: sp(5),
        shrink_order: Order::Normal,
    });
    let nodes = vec![
        rule(30),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
        },
        rule(30),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
        },
        rule(30),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
        },
        rule(30),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
        },
        rule(30),
    ];
    let mut params = params(100);
    params.pretolerance = -1;
    params.tolerance = 200;
    params.emergency_stretch = sp(0);

    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, params, &mut hook);

    assert!(result.breaks.len() > 1, "{:?}", result.breaks);
    assert_ne!(
        result.breaks.first().map(|br| br.position),
        Some(nodes.len())
    );
}

#[test]
fn discretionary_penalty_comes_from_source_kind() {
    let mut universe = Universe::new();
    let pre = universe.freeze_node_list(&[kern(0)]);
    let empty = universe.freeze_node_list(&[]);
    let mut params = params(20);
    params.pretolerance = -1;
    params.hyphen_penalty = 321;
    params.ex_hyphen_penalty = 654;
    let nodes = vec![
        kern(20),
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre,
            post: empty,
            replace: empty,
        },
        kern(20),
    ];
    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, params.clone(), &mut hook);
    assert_eq!(result.breaks.first().map(|br| br.penalty), Some(321));

    let nodes = vec![
        kern(20),
        Node::Disc {
            kind: DiscKind::ExplicitHyphen,
            pre,
            post: empty,
            replace: empty,
        },
        kern(20),
    ];
    let result = line_break(&universe, &nodes, params, &mut hook);
    assert_eq!(result.breaks.first().map(|br| br.penalty), Some(654));
}

#[test]
fn final_hyphen_demerits_apply_to_penultimate_hyphenated_line() {
    let mut universe = Universe::new();
    let empty = universe.freeze_node_list(&[]);
    let nodes = vec![
        kern(20),
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty,
            post: empty,
            replace: empty,
        },
        kern(20),
    ];
    let mut base = params(20);
    base.pretolerance = -1;
    base.hyphen_penalty = 0;
    base.final_hyphen_demerits = 0;
    let mut hook = NoHyphenation;
    let without = line_break(&universe, &nodes, base.clone(), &mut hook).demerits;
    base.final_hyphen_demerits = 1234;
    let with = line_break(&universe, &nodes, base, &mut hook).demerits;
    assert_eq!(with - without, 1234);
}

#[test]
fn final_hyphen_demerits_rank_terminal_routes_before_candidate_pruning() {
    let mut params = params(100);
    params.final_hyphen_demerits = 5_000;
    let active = |path_demerits, hyphenated| Candidate {
        position: 0,
        width_position: 0,
        penalty: 0,
        line: 9,
        fitness: Fitness::Decent,
        demerits: path_demerits,
        path_demerits,
        previous: None,
        hyphenated,
    };
    let terminal = Breakpoint {
        position: 1,
        width_position: 1,
        penalty: EJECT_PENALTY,
        hyphenated: false,
        add_width: Widths::zero(),
    };
    let unhyphenated = active(12_886, false);
    let hyphenated = active(10_566, true);

    let plain_demerits = compute_demerits(
        &params,
        &unhyphenated,
        0,
        EJECT_PENALTY,
        Fitness::Decent,
        terminal,
        true,
    );
    let hyphenated_demerits = compute_demerits(
        &params,
        &hyphenated,
        0,
        EJECT_PENALTY,
        Fitness::Decent,
        terminal,
        true,
    );

    assert_eq!(plain_demerits, 12_986);
    assert_eq!(hyphenated_demerits, 15_666);
}

#[test]
fn post_line_break_keeps_migrating_nodes_for_execution_layer() {
    let mut universe = Universe::new();
    let empty_glue = universe.intern_glue(GlueSpec::ZERO);
    let mark_tokens = universe.intern_token_list(&[Token::Char {
        ch: 'm',
        cat: Catcode::Letter,
    }]);
    let adjust_content = universe.freeze_node_list(&[kern(7)]);
    let nodes = vec![
        rule(10),
        Node::Mark {
            class: 0,
            tokens: mark_tokens,
        },
        Node::Adjust(adjust_content),
        Node::Penalty(-10_000),
        rule(10),
        Node::Penalty(10_000),
    ];
    let breaks = vec![
        BreakDecision {
            position: 4,
            penalty: -10_000,
            hyphenated: false,
        },
        BreakDecision {
            position: 6,
            penalty: 10_000,
            hyphenated: false,
        },
    ];
    let lines = post_line_break(
        &universe,
        &nodes,
        &breaks,
        PostLineBreakParams {
            left_skip: empty_glue,
            right_skip: empty_glue,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalty: 0,
            broken_penalty: 0,
            shape: LineShape::natural(sp(100)),
        },
    );

    assert_eq!(lines.len(), 2);
    assert!(matches!(
        lines[0].nodes.as_slice(),
        [
            Node::Rule { .. },
            Node::Mark { class: 0, tokens },
            Node::Adjust(list),
            Node::Penalty(-10_000),
            Node::Glue { .. },
        ] if *tokens == mark_tokens && *list == adjust_content
    ));
}

#[test]
fn post_line_break_splices_unbroken_discretionary_replacement() {
    let mut universe = Universe::new();
    let zero = universe.intern_glue(GlueSpec::ZERO);
    let empty = universe.freeze_node_list(&[]);
    let replacement = universe.freeze_node_list(&[rule(7)]);
    let nodes = vec![
        rule(3),
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty,
            post: empty,
            replace: replacement,
        },
        Node::Penalty(10_000),
    ];
    let breaks = vec![BreakDecision {
        position: nodes.len(),
        penalty: 10_000,
        hyphenated: false,
    }];

    let lines = post_line_break(
        &universe,
        &nodes,
        &breaks,
        PostLineBreakParams {
            left_skip: zero,
            right_skip: zero,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalty: 0,
            broken_penalty: 0,
            shape: LineShape::natural(sp(100)),
        },
    );

    assert!(matches!(
        lines[0].nodes.as_slice(),
        [
            Node::Rule { width: Some(first), .. },
            Node::Rule { width: Some(second), .. },
            Node::Penalty(10_000),
            Node::Glue { kind: GlueKind::RightSkip, .. },
        ] if first.raw() == 3 && second.raw() == 7
    ));
}

#[test]
fn post_line_break_omits_only_zero_leftskip() {
    let mut universe = Universe::new();
    let zero = universe.intern_glue(GlueSpec::ZERO);
    let nonzero = universe.intern_glue(GlueSpec {
        width: sp(3),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
    let nodes = vec![rule(10), Node::Penalty(10_000)];
    let breaks = vec![BreakDecision {
        position: nodes.len(),
        penalty: 10_000,
        hyphenated: false,
    }];

    let zero_left = post_line_break(
        &universe,
        &nodes,
        &breaks,
        PostLineBreakParams {
            left_skip: zero,
            right_skip: zero,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalty: 0,
            broken_penalty: 0,
            shape: LineShape::natural(sp(100)),
        },
    );
    assert!(matches!(
        zero_left[0].nodes.as_slice(),
        [
            Node::Rule { .. },
            Node::Penalty(10_000),
            Node::Glue {
                spec,
                kind: GlueKind::RightSkip,

                leader: None,
            },
        ] if *spec == zero
    ));

    let nonzero_left = post_line_break(
        &universe,
        &nodes,
        &breaks,
        PostLineBreakParams {
            left_skip: nonzero,
            right_skip: zero,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalty: 0,
            broken_penalty: 0,
            shape: LineShape::natural(sp(100)),
        },
    );
    assert!(matches!(
        nonzero_left[0].nodes.as_slice(),
        [
            Node::Glue {
                spec: left,
                kind: GlueKind::LeftSkip,

                leader: None,
            },
            Node::Rule { .. },
            Node::Penalty(10_000),
            Node::Glue {
                spec: right,
                kind: GlueKind::RightSkip,

                leader: None,
            },
        ] if *left == nonzero && *right == zero
    ));
}
