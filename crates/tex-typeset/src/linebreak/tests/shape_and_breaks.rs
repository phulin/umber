//! Line shape, glue, kern, discretionary, and hyphen decisions.

use super::*;

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
    let universe = TestState::new();
    let glue = GlueSpec {
        width: sp(1000),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
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
fn discardable_tail_does_not_create_an_empty_final_line() {
    let universe = TestState::new();
    let trailing = GlueSpec {
        width: sp(10),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(10),
        shrink_order: Order::Normal,
    };
    let par_fill = GlueSpec {
        width: sp(0),
        stretch: sp(1),
        stretch_order: Order::Fil,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
    let nodes = vec![
        rule(100),
        Node::Glue {
            spec: trailing,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Glue {
            spec: trailing,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Penalty(10_000),
        Node::Glue {
            spec: par_fill,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
    ];

    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, params(100), &mut hook);

    assert_eq!(
        result.breaks,
        vec![BreakDecision {
            position: nodes.len(),
            penalty: -10_000,
            hyphenated: false,
        }]
    );
}

#[test]
fn looseness_can_select_empty_line_after_terminal_discretionary() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let hyphen = universe.publish_page_nodes(&[rule(5)]);
    let par_fill = GlueSpec {
        width: sp(0),
        stretch: sp(1),
        stretch_order: Order::Fil,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
    let nodes = vec![
        rule(20),
        Node::Disc {
            kind: DiscKind::ExplicitHyphen,
            pre: hyphen,
            post: empty,
            replace: empty,
            physical_replace_count: 0,
        },
        Node::Penalty(10_000),
        Node::Glue {
            spec: par_fill,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
    ];
    let mut p = params(20);
    p.looseness = 1;
    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, p, &mut hook);

    assert_eq!(result.breaks.len(), 2);
    assert_eq!(result.breaks[0].position, 2);
    assert_eq!(result.breaks[1].position, nodes.len());
}

#[test]
fn equal_demerit_easy_line_champion_uses_terminal_discretionary_route() {
    // TeX82 §848 makes every line after `easy_line` equivalent when
    // `\looseness=0`. Sections 851--854 retain one champion per equivalent
    // line/fitness class, and `d<=minimal_demerits` lets the later route via
    // this terminal discretionary replace the direct route.
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let par_fill = GlueSpec::ZERO;
    let nodes = vec![
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: empty,
            physical_replace_count: 0,
        },
        Node::Penalty(INF_PENALTY),
        Node::Glue {
            spec: par_fill,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
    ];
    let mut parameters = params(0);
    parameters.line_penalty = 0;
    parameters.hyphen_penalty = 0;
    parameters.ex_hyphen_penalty = 0;
    parameters.adj_demerits = 0;
    parameters.double_hyphen_demerits = 0;
    parameters.final_hyphen_demerits = 0;
    let mut hook = NoHyphenation;

    let equal = line_break(&universe, &nodes, parameters.clone(), &mut hook);
    assert_eq!(
        equal
            .breaks
            .iter()
            .map(|br| br.position)
            .collect::<Vec<_>>(),
        [1, nodes.len()]
    );

    parameters.line_penalty = 1;
    let unequal = line_break(&universe, &nodes, parameters, &mut hook);
    assert_eq!(
        unequal
            .breaks
            .iter()
            .map(|br| br.position)
            .collect::<Vec<_>>(),
        [nodes.len()],
        "a genuinely more expensive two-line route is not retained"
    );
}

#[test]
fn unmet_looseness_retries_after_the_pretolerance_pass() {
    let universe = TestState::new();
    let break_glue = GlueSpec {
        width: sp(0),
        stretch: sp(100),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
    let par_fill = GlueSpec {
        width: sp(0),
        stretch: sp(1),
        stretch_order: Order::Fil,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
    let nodes = vec![
        rule(10),
        Node::Glue {
            spec: break_glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(10),
        Node::Penalty(10_000),
        Node::Glue {
            spec: par_fill,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
    ];
    let mut p = params(100);
    p.pretolerance = 0;
    p.tolerance = 10_000;
    p.looseness = 1;
    let mut hook = NoHyphenation;

    let result = line_break(&universe, &nodes, p, &mut hook);

    assert_eq!(result.breaks.len(), 2);
}

#[test]
fn mathoff_breaks_only_before_following_glue_and_zeroes_break_width() {
    let mut universe = TestState::new();
    let glue = GlueSpec {
        width: sp(1000),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
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
    let breakpoints = legal_breakpoints(&universe, &nodes, &params(15));

    assert_eq!(breakpoints.first().map(|br| br.position), Some(2));
    assert_eq!(breakpoints[0].line_width.natural.raw(), 10);
    assert_eq!(breakpoints[0].next_width.natural.raw(), 1015);
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
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
            empty_list: empty,
            left_skip: zero,
            right_skip: zero,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalties: ordinary_widow_penalties(0, Vec::new()),
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
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
    let breakpoints = legal_breakpoints(&universe, &nodes_without_glue, &params(15));
    assert!(!breakpoints.iter().any(|br| br.position == 2));
}

#[test]
fn explicit_kern_break_scores_before_adding_the_kern() {
    // TeX82 §§822/866 tests the break before adding the explicit kern, then
    // discards that kern while constructing the following line's prefix.
    let mut universe = TestState::new();
    let glue = GlueSpec::ZERO;
    let nodes = vec![
        rule(10),
        kern(5),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(10),
    ];

    let breakpoints = legal_breakpoints(&universe, &nodes, &params(10));

    assert_eq!(breakpoints[0].position, 2);
    assert_eq!(breakpoints[0].line_width.natural.raw(), 10);
    assert_eq!(breakpoints[0].next_width.natural.raw(), 15);

    let empty = universe.publish_page_nodes(&[]);
    let lines = post_line_break(
        &universe,
        &nodes,
        &[
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
        ],
        PostLineBreakParams {
            empty_list: empty,
            left_skip: glue,
            right_skip: glue,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalties: ordinary_widow_penalties(0, Vec::new()),
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            shape: LineShape::natural(sp(10)),
        },
    );

    assert!(
        !lines[0]
            .nodes
            .iter()
            .any(|node| matches!(node, Node::Kern { .. })),
        "the chosen explicit kern is outside the prior line"
    );
    assert!(
        matches!(lines[1].nodes.first(), Some(Node::Rule { .. })),
        "the kern and following glue are discarded before the next line"
    );
}

#[test]
fn math_boundaries_suppress_internal_glue_and_kern_breaks() {
    let universe = TestState::new();
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(10),
        stretch_order: Order::Normal,
        shrink: sp(5),
        shrink_order: Order::Normal,
    };
    let nodes = vec![
        rule(10),
        Node::MathOn(sp(0)),
        rule(10),
        Node::Glue {
            spec: glue,
            kind: GlueKind::ThinMuSkip,
            leader: None,
        },
        rule(10),
        kern(5),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(10),
        Node::MathOff(sp(0)),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(10),
    ];

    let positions: Vec<_> = legal_breakpoints(&universe, &nodes, &params(50))
        .into_iter()
        .map(|breakpoint| breakpoint.position)
        .collect();

    assert_eq!(positions, vec![9, nodes.len()]);
}

#[test]
fn final_pass_deactivates_unshrinkable_active_line() {
    let universe = TestState::new();
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(10),
        stretch_order: Order::Normal,
        shrink: sp(5),
        shrink_order: Order::Normal,
    };
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
fn discretionary_penalty_depends_on_pre_break_text() {
    let mut universe = TestState::new();
    let pre = universe.publish_page_nodes(&[kern(0)]);
    let empty = universe.publish_page_nodes(&[]);
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
            physical_replace_count: 0,
        },
        kern(20),
        rule(1),
    ];
    let breakpoints = legal_breakpoints(&universe, &nodes, &params);
    assert_eq!(breakpoints.first().map(|br| br.penalty), Some(321));

    let nodes = vec![
        kern(20),
        Node::Disc {
            kind: DiscKind::ExplicitHyphen,
            pre: empty,
            post: empty,
            replace: empty,
            physical_replace_count: 0,
        },
        kern(20),
        rule(1),
    ];
    let breakpoints = legal_breakpoints(&universe, &nodes, &params);
    assert_eq!(breakpoints.first().map(|br| br.penalty), Some(654));
}

#[test]
fn font_kern_is_not_discarded_at_start_of_next_line() {
    let universe = TestState::new();
    let nodes = [
        Node::Penalty(0),
        Node::Kern {
            amount: sp(1),
            kind: KernKind::Font,
        },
        rule(1),
    ];
    let breakpoints = legal_breakpoints(&universe, &nodes, &params(100));

    assert_eq!(breakpoints[0].position, 1);
    assert_eq!(breakpoints[0].next_position, 1);
}

#[test]
fn existing_discretionary_is_available_on_the_pretolerance_pass() {
    struct UnexpectedHyphenation;

    impl HyphenationHook<TestState> for UnexpectedHyphenation {
        fn hyphenate(&mut self, _nodes: &[Node]) -> Vec<Node> {
            panic!("a feasible first pass must not invoke automatic hyphenation")
        }
    }

    let mut universe = TestState::new();
    let pre = universe.publish_page_nodes(&[kern(1)]);
    let empty = universe.publish_page_nodes(&[]);
    let par_fill = GlueSpec {
        width: sp(0),
        stretch: sp(1),
        stretch_order: Order::Fil,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
    let nodes = vec![
        kern(20),
        Node::Disc {
            kind: DiscKind::ExplicitHyphen,
            pre,
            post: empty,
            replace: empty,
            physical_replace_count: 0,
        },
        rule(20),
        Node::Penalty(10_000),
        Node::Glue {
            spec: par_fill,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
    ];
    let mut hook = UnexpectedHyphenation;

    let result = line_break(&universe, &nodes, params(21), &mut hook);

    assert!(result.breaks[0].hyphenated);
    assert_eq!(result.breaks[0].position, 2);
}

#[test]
fn final_hyphen_demerits_apply_to_penultimate_hyphenated_line() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let nodes = vec![
        kern(20),
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty,
            post: empty,
            replace: empty,
            physical_replace_count: 0,
        },
        rule(20),
    ];
    let mut base = params(20);
    base.pretolerance = -1;
    base.hyphen_penalty = 0;
    base.final_hyphen_demerits = 0;
    // Keep the direct terminal route feasible so the hyphenated route is
    // scored normally instead of using TeX's artificial-demerits fallback.
    base.right_skip = GlueSpec {
        width: sp(0),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(20),
        shrink_order: Order::Normal,
    };
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
        serial: 0,
        position: 0,
        break_site: INITIAL_BREAK_SITE,
        penalty: 0,
        line: 9,
        fitness: Fitness::Decent,
        path_demerits,
        passive: None,
        previous: None,
        hyphenated,
        line_shortfall: sp(0),
        line_glue: sp(0),
    };
    let terminal = Breakpoint {
        position: 1,
        protrusion_end: 1,
        penalty: EJECT_PENALTY,
        hyphenated: false,
        add_width: Widths::zero(),
        line_width: Widths::zero(),
        next_position: 1,
        next_width: Widths::zero(),
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
    let mut universe = TestState::new();
    let empty_glue = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
    let mark_tokens = tex_state::node::NodeTokenKey::default();
    let adjust_content = universe.publish_page_nodes(&[kern(7)]);
    let nodes = vec![
        rule(10),
        Node::Mark {
            class: 0,
            tokens: mark_tokens,
        },
        Node::Adjust(tex_state::node::AdjustNode::ordinary(adjust_content)),
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
            empty_list: empty,
            left_skip: empty_glue,
            right_skip: empty_glue,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalties: ordinary_widow_penalties(0, Vec::new()),
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            shape: LineShape::natural(sp(100)),
        },
    );

    assert_eq!(lines.len(), 2);
    assert!(matches!(
        lines[0].nodes.as_slice(),
        [
            Node::Rule { .. },
            Node::Mark { class: 0, tokens },
            Node::Adjust(adjust),
            Node::Penalty(-10_000),
            Node::Glue { .. },
        ] if tokens == &mark_tokens && !adjust.pre && adjust.content == adjust_content
    ));
}
