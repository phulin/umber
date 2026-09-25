//! Feasible-break displays and discretionary trace successors.

use super::*;

#[test]
fn tracing_display_includes_the_feasible_glue_breakpoint() {
    // TeX82 §851's temporary `link(cur_p):=null` includes `cur_p` in
    // `short_display`; for a glue breakpoint, §175 renders that node as a
    // trailing space. Width measurement still ends before the glue.
    let universe = TestState::new();
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(10),
        ..GlueSpec::ZERO
    };
    let nodes = vec![
        rule(20),
        Node::Glue {
            origin: tex_state::node::GlueSpecOrigin::Owned,
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(20),
        Node::Penalty(EJECT_PENALTY),
    ];

    let mut parameters = params(30);
    parameters.left_skip.stretch = sp(10);
    let (_, trace) = line_break_hyphenated_traced(&universe, &nodes, &parameters, Vec::new());

    assert!(
        trace.iter().any(|event| matches!(
            event,
            LineBreakTrace::Feasible {
                display,
                breakpoint: TraceBreakpoint::Glue,
                ..
            } if display == &(0..2)
        )),
        "{trace:?}"
    );
}

#[test]
fn tracing_display_retains_structural_successors_after_discretionary_cluster() {
    // TeX82 §§851/855 temporarily hide linked replacement nodes while the
    // current discretionary is displayed. The pure breaker's detached cursor
    // must still begin its next fragment at the structural successor.
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let first_replace = universe.publish_page_nodes(&[]);
    let second_replace = universe.publish_page_nodes(&[kern(2), rule(3)]);
    let nodes = vec![
        rule(1),
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: first_replace,
            physical_replace_count: 0,
        },
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: second_replace,
            physical_replace_count: 2,
        },
        kern(1),
        kern(2),
        rule(3),
        Node::Penalty(EJECT_PENALTY),
    ];
    let mut parameters = params(100);
    parameters.pretolerance = 10_000;
    let (_, trace) = try_line_break_without_hyphenation_traced(&universe, &nodes, &parameters);
    let displays = trace
        .iter()
        .filter_map(|event| match event {
            LineBreakTrace::Feasible { display, .. } if !display.is_empty() => {
                Some(display.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(displays.contains(&(0..2)), "{trace:?}");
    assert!(displays.contains(&(2..5)), "{trace:?}");
    assert!(displays.contains(&(3..7)), "{trace:?}");
}

#[test]
fn tracing_display_does_not_repeat_successors_rendered_with_a_discretionary_cluster() {
    // The negative control has replacement material on the preceding disc.
    // The extended current slice therefore renders nodes beyond its own
    // hidden replacement and advances §851's `printed_node` through them.
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let first_replace = universe.publish_page_nodes(&[kern(1)]);
    let second_replace = universe.publish_page_nodes(&[kern(2), rule(3)]);
    let nodes = vec![
        rule(1),
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: first_replace,
            physical_replace_count: 1,
        },
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: second_replace,
            physical_replace_count: 2,
        },
        kern(1),
        kern(2),
        rule(3),
        Node::Penalty(EJECT_PENALTY),
    ];
    let mut parameters = params(100);
    parameters.pretolerance = 10_000;
    let (_, trace) = try_line_break_without_hyphenation_traced(&universe, &nodes, &parameters);
    let displays = trace
        .iter()
        .filter_map(|event| match event {
            LineBreakTrace::Feasible { display, .. } if !display.is_empty() => {
                Some(display.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(displays.contains(&(2..6)), "{trace:?}");
    assert!(displays.contains(&(6..7)), "{trace:?}");
    assert!(!displays.contains(&(3..7)), "{trace:?}");
}

#[test]
fn tracing_display_includes_automatic_discretionary_replacement_after_font_kern() {
    // TeX82's reconstitution can leave the replaced ligature in the
    // discretionary after a font kern; §851 displays that replacement before
    // reporting the feasible discretionary.
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let replace = universe.publish_page_nodes(&[rule(2)]);
    let nodes = vec![
        rule(1),
        Node::Kern {
            amount: sp(1),
            kind: KernKind::Font,
        },
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty,
            post: empty,
            replace,
            physical_replace_count: 1,
        },
        Node::Kern {
            amount: sp(1),
            kind: KernKind::Font,
        },
        rule(2),
        Node::Penalty(EJECT_PENALTY),
    ];
    let mut parameters = params(100);
    parameters.pretolerance = 10_000;
    let (_, trace) = try_line_break_without_hyphenation_traced(&universe, &nodes, &parameters);

    assert!(
        trace.iter().any(|event| matches!(
            event,
            LineBreakTrace::Feasible {
                display_suffix: Some(suffix),
                breakpoint: TraceBreakpoint::Discretionary,
                ..
            } if *suffix == replace
        )),
        "{trace:?}"
    );
}

#[test]
fn paragraph_prefix_widths_remain_exact_past_i32_max() {
    let universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let mut nodes = Vec::new();
    for index in 0..6 {
        nodes.push(rule(700_000_000));
        if index != 5 {
            nodes.push(Node::Glue {
                origin: tex_state::node::GlueSpecOrigin::Owned,
                spec: zero,
                kind: GlueKind::Normal,
                leader: None,
            });
        }
    }
    let mut hook = NoHyphenation;

    let result = line_break(&universe, &nodes, params(700_000_000), &mut hook);

    assert_eq!(
        result
            .breaks
            .iter()
            .map(|decision| decision.position)
            .collect::<Vec<_>>(),
        vec![2, 4, 6, 8, 10, 11]
    );
}
