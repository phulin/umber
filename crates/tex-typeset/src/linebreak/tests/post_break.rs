//! Post-break materialization, penalties, and migrated nodes.

use super::*;

/// tex.web §§879--885: a taken discretionary contributes its `pre_break`
/// list to the line being closed and its `post_break` list to the next line.
#[test]
fn chosen_discretionary_transplants_nonempty_pre_and_post_lists() {
    let mut universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
    let pre = universe.publish_page_nodes(&[rule(11), kern(12)]);
    let post = universe.publish_page_nodes(&[rule(21), kern(22)]);
    let replacement = universe.publish_page_nodes(&[rule(99)]);
    let nodes = vec![
        rule(1),
        Node::Disc {
            kind: DiscKind::ExplicitHyphen,
            pre,
            post,
            replace: replacement,
            physical_replace_count: 1,
        },
        rule(2),
        Node::Penalty(EJECT_PENALTY),
    ];
    let breaks = vec![
        BreakDecision {
            position: 2,
            penalty: 0,
            hyphenated: true,
        },
        BreakDecision {
            position: nodes.len(),
            penalty: EJECT_PENALTY,
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
            shape: LineShape::natural(sp(100)),
        },
    );

    assert!(matches!(
        lines[0].nodes.as_slice(),
        [
            Node::Rule { width: Some(original), .. },
            Node::Disc {
                pre: cleared_pre,
                post: cleared_post,
                replace: cleared_replace,
                ..
            },
            Node::Rule { width: Some(pre_rule), .. },
            Node::Kern { amount: pre_kern, kind: KernKind::Explicit },
            Node::Glue { kind: GlueKind::RightSkip, .. },
        ] if original.raw() == 1
            && *cleared_pre == empty
            && *cleared_post == empty
            && *cleared_replace == empty
            && pre_rule.raw() == 11
            && pre_kern.raw() == 12
    ));
    assert!(matches!(
        lines[1].nodes.as_slice(),
        [
            Node::Rule { width: Some(post_rule), .. },
            Node::Kern { amount: post_kern, kind: KernKind::Explicit },
            Node::Rule { width: Some(next), .. },
            Node::Penalty(EJECT_PENALTY),
            Node::Glue { kind: GlueKind::RightSkip, .. },
        ] if post_rule.raw() == 21 && post_kern.raw() == 22 && next.raw() == 2
    ));
}

/// tex.web §§822 and 851: after a discretionary break, line measurement
/// replaces the unbroken text by the post-break list. A nonempty post-break
/// list can therefore make the following line exactly fit on the first pass.
#[test]
fn discretionary_post_break_width_participates_in_the_next_line() {
    let mut universe = TestState::new();
    let pre = universe.publish_page_nodes(&[rule(7)]);
    let post = universe.publish_page_nodes(&[rule(4)]);
    let replace = universe.publish_page_nodes(&[rule(6)]);
    let nodes = vec![
        rule(3),
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre,
            post,
            replace,
            physical_replace_count: 1,
        },
        rule(6),
        Node::Penalty(EJECT_PENALTY),
    ];
    let mut parameters = params(13);
    parameters.pretolerance = 0;
    parameters.left_skip.width = sp(3);

    let (plan, trace) = try_line_break_without_hyphenation_traced(&universe, &nodes, &parameters);

    assert_eq!(
        plan.expect("the post-break line fits at pretolerance zero")
            .breaks
            .iter()
            .map(|decision| decision.position)
            .collect::<Vec<_>>(),
        vec![2, 4]
    );
    assert!(trace.iter().any(|event| matches!(
        event,
        LineBreakTrace::Feasible {
            breakpoint: TraceBreakpoint::Paragraph,
            via: 1,
            badness: Some(0),
            ..
        }
    )));
}

/// tex.web §§886--887: glue, explicit and mu kerns, penalties, and math nodes
/// disappear before the next line, but a font kern terminates that discard.
#[test]
fn next_line_discards_all_discardables_but_retains_font_kern() {
    let mut universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
    let nodes = vec![
        rule(1),
        Node::Penalty(0),
        Node::Glue {
            spec: zero,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Kern {
            amount: sp(2),
            kind: KernKind::Explicit,
        },
        Node::Kern {
            amount: sp(3),
            kind: KernKind::Mu,
        },
        Node::Penalty(4),
        Node::MathOn(sp(5)),
        Node::MathOff(sp(6)),
        Node::Kern {
            amount: sp(7),
            kind: KernKind::Font,
        },
        rule(8),
        Node::Penalty(EJECT_PENALTY),
    ];
    let breaks = vec![
        BreakDecision {
            position: 2,
            penalty: 0,
            hyphenated: false,
        },
        BreakDecision {
            position: nodes.len(),
            penalty: EJECT_PENALTY,
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
            shape: LineShape::natural(sp(100)),
        },
    );

    assert!(matches!(
        lines[1].nodes.as_slice(),
        [
            Node::Kern { amount, kind: KernKind::Font },
            Node::Rule { width: Some(width), .. },
            Node::Penalty(EJECT_PENALTY),
            Node::Glue { kind: GlueKind::RightSkip, .. },
        ] if amount.raw() == 7 && width.raw() == 8
    ));
}

/// tex.web §890: on the only nonfinal boundary in a two-line paragraph, all
/// four scalar penalty contributions apply, including `broken_penalty`.
#[test]
fn two_line_penalty_after_combines_club_widow_and_broken_penalties() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let zero = GlueSpec::ZERO;
    let breaks = vec![
        BreakDecision {
            position: 1,
            penalty: 0,
            hyphenated: true,
        },
        BreakDecision {
            position: 2,
            penalty: EJECT_PENALTY,
            hyphenated: false,
        },
    ];
    let params = PostLineBreakParams {
        empty_list: empty,
        left_skip: zero,
        right_skip: zero,
        interline_penalty: 11,
        club_penalty: 101,
        widow_penalties: ordinary_widow_penalties(1_001, Vec::new()),
        broken_penalty: 10_001,
        prev_graf: 0,
        interline_penalties: Vec::new(),
        club_penalties: Vec::new(),
        shape: LineShape::natural(sp(100)),
    };

    assert_eq!(
        post::line_penalty_after(0, &breaks, true, &params),
        Some(11_114)
    );
    assert_eq!(post::line_penalty_after(1, &breaks, false, &params), None);
}

#[test]
fn post_line_break_closes_and_resumes_open_tex_xet_segments() {
    use tex_state::node::Direction;

    let mut universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
    let nodes = vec![
        Node::Direction(Direction::BeginR),
        rule(1),
        rule(2),
        rule(3),
        Node::Direction(Direction::EndR),
        Node::Penalty(10_000),
    ];
    let breaks = vec![
        BreakDecision {
            position: 3,
            penalty: 0,
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
            left_skip: zero,
            right_skip: zero,
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

    let directions = |line: &BrokenLine| {
        line.nodes
            .iter()
            .filter_map(|node| match node {
                Node::Direction(direction) => Some(*direction),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(directions(&lines[0]), [Direction::BeginR, Direction::EndR]);
    assert_eq!(directions(&lines[1]), [Direction::BeginR, Direction::EndR]);
}

#[test]
fn post_line_break_retains_materialized_unbroken_discretionary_replacement_count() {
    let mut universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
    let replacement = universe.publish_page_nodes(&[rule(7)]);
    let nodes = vec![
        rule(3),
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty,
            post: empty,
            replace: replacement,
            physical_replace_count: 1,
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
            shape: LineShape::natural(sp(100)),
        },
    );

    assert!(matches!(
        lines[0].nodes.as_slice(),
        [
            Node::Rule { width: Some(first), .. },
            Node::Disc { replace: retained_replacement, .. },
            Node::Rule { width: Some(second), .. },
            Node::Penalty(10_000),
            Node::Glue { kind: GlueKind::RightSkip, .. },
        ] if first.raw() == 3 && *retained_replacement == replacement && second.raw() == 7
    ));
}

#[test]
fn line_materializer_reuses_the_returned_line_buffer() {
    let mut universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
    let nodes = vec![rule(1), rule(2), rule(3), rule(4)];
    let breaks = vec![
        BreakDecision {
            position: 2,
            penalty: 0,
            hyphenated: false,
        },
        BreakDecision {
            position: 4,
            penalty: EJECT_PENALTY,
            hyphenated: false,
        },
    ];
    let mut materializer = LineMaterializer::from_borrowed_nodes(
        &nodes,
        breaks,
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
            shape: LineShape::natural(sp(100)),
        },
    );

    let first = materializer
        .materialize_next(&universe, Vec::new())
        .expect("first line");
    let allocation = first.nodes.as_ptr();
    let capacity = first.nodes.capacity();
    let second = materializer
        .materialize_next(&universe, first.nodes)
        .expect("second line");

    assert_eq!(second.nodes.as_ptr(), allocation);
    assert_eq!(second.nodes.capacity(), capacity);
    assert!(
        materializer
            .materialize_next(&universe, second.nodes)
            .is_none()
    );
}

#[test]
fn post_line_break_omits_only_zero_leftskip() {
    let mut universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
    let nonzero = GlueSpec {
        width: sp(3),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
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
            empty_list: empty,
            left_skip: nonzero,
            right_skip: zero,
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

#[test]
fn paragraph_tape_bounds_analysis_storage_for_large_paragraphs() {
    let universe = TestState::new();
    let glue = GlueSpec::ZERO;
    let mut nodes = Vec::with_capacity(100_000);
    for _ in 0..50_000 {
        nodes.push(rule(1));
        nodes.push(Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        });
    }
    let parameters = params(100);
    let tape = ParagraphTape::analyze_borrowed(&universe, &nodes, &parameters);

    assert_eq!(tape.materialization.len(), tape.nodes(&universe).len());
    assert!(tape.break_sites.len() <= tape.nodes(&universe).len() + 1);
    assert_eq!(std::mem::size_of::<MaterializationAction>(), 1);
}
