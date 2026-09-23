//! Paragraph analysis, breakpoint metrics, passes, and penalty arrays.

use super::*;

#[test]
fn active_candidate_reuses_break_site_metrics_compactly() {
    assert_eq!(std::mem::size_of::<Candidate>(), 80);
    assert_eq!(
        std::mem::size_of::<BreakSite>(),
        std::mem::size_of::<Breakpoint>(),
        "ordinary break sites retain no eager diagnostic projection"
    );

    let mut next_width = Widths::zero();
    next_width.natural = tex_arith::WideScaled::from_scaled(sp(37));
    let sites = [BreakSite {
        breakpoint: Breakpoint {
            position: 5,
            protrusion_end: 4,
            penalty: 0,
            hyphenated: false,
            add_width: Widths::zero(),
            line_width: Widths::zero(),
            next_position: 8,
            next_width,
        },
    }];
    let candidate = Candidate {
        serial: 1,
        position: 5,
        break_site: 0,
        penalty: 0,
        line: 1,
        fitness: Fitness::Decent,
        path_demerits: 0,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: sp(0),
        line_glue: sp(0),
    };

    assert_eq!(candidate.width_position(&sites), 8);
    assert_eq!(candidate.start_width(&sites), next_width);
}

#[test]
fn direct_page_chunk_analysis_matches_slice_layout_semantics() {
    let mut universe = TestState::new();
    let mut nodes = Vec::new();
    for index in 0..128 {
        nodes.push(Node::Kern {
            amount: sp(index),
            kind: KernKind::Explicit,
        });
        nodes.push(Node::Glue {
            spec: GlueSpec {
                width: sp(1),
                ..GlueSpec::ZERO
            },
            kind: GlueKind::Normal,
            leader: None,
        });
        nodes.push(Node::Penalty((index % 17) - 8));
    }
    let list = universe.publish_page_nodes(&nodes);
    let cursor = universe.page_nodes(list);
    let params = params(1_000);

    let mut direct = LegalBreakpoints::new(&universe, cursor, &params);
    let direct_breakpoints = direct.collect_direct();
    let direct_materialization = direct.materialization;
    let mut slice = LegalBreakpoints::new(&universe, NodeCursor::owned(&nodes), &params);
    let slice_breakpoints = slice.collect_direct();

    assert_eq!(direct_breakpoints, slice_breakpoints);
    assert_eq!(direct_materialization, slice.materialization);
}

#[test]
fn ordinary_breakpoint_analysis_crosses_each_block_once_at_required_sizes() {
    fn delta(
        after: tex_state::node_arena::NodeTraversalCounters,
        before: tex_state::node_arena::NodeTraversalCounters,
    ) -> tex_state::node_arena::NodeTraversalCounters {
        tex_state::node_arena::NodeTraversalCounters {
            index_resolutions: after
                .index_resolutions
                .saturating_sub(before.index_resolutions),
            index_predecessor_steps: after
                .index_predecessor_steps
                .saturating_sub(before.index_predecessor_steps),
            forward_chunk_crossings: after
                .forward_chunk_crossings
                .saturating_sub(before.forward_chunk_crossings),
        }
    }

    for values in [1_usize, 4_096] {
        let mut universe = TestState::new();
        let nodes = (0..values)
            .map(|index| {
                if index % 3 == 1 {
                    Node::Glue {
                        spec: GlueSpec {
                            width: sp(1),
                            ..GlueSpec::ZERO
                        },
                        kind: GlueKind::Normal,
                        leader: None,
                    }
                } else if index % 3 == 2 {
                    Node::Penalty(0)
                } else {
                    rule(1)
                }
            })
            .collect::<Vec<_>>();
        let list = universe.publish_page_nodes(&nodes);
        let cursor = universe.page_nodes(list);
        let parameters = params(1_000);

        let before = cursor.testing_traversal_counters();
        let mut analyzer = LegalBreakpoints::new(&universe, cursor, &parameters);
        let breakpoints = analyzer.collect_direct();
        let sequential = delta(cursor.testing_traversal_counters(), before);
        assert!(!breakpoints.is_empty());
        assert_eq!(sequential.index_resolutions, 0);
        assert_eq!(sequential.index_predecessor_steps, 0);

        let reference_before = cursor.testing_traversal_counters();
        let mut visited = 0;
        let _: core::ops::ControlFlow<core::convert::Infallible> = cursor
            .try_for_each_direct_range(0..cursor.len(), |_, _| {
                visited += 1;
                core::ops::ControlFlow::Continue(())
            });
        let reference = delta(cursor.testing_traversal_counters(), reference_before);
        assert_eq!(visited, values);
        assert_eq!(
            sequential.forward_chunk_crossings,
            reference.forward_chunk_crossings
        );

        let positional_before = cursor.testing_traversal_counters();
        assert!(cursor.get(0).is_some());
        let positional = delta(cursor.testing_traversal_counters(), positional_before);
        assert_eq!(positional.index_resolutions, 1);
        if values == 4_096 {
            assert!(positional.index_predecessor_steps > 0);
        }
        eprintln!(
            "LINEBREAK_TRAVERSAL_SCALE values={values} sequential_index_resolutions={} sequential_predecessor_steps={} sequential_block_crossings={} positional_index_resolutions={} positional_predecessor_steps={}",
            sequential.index_resolutions,
            sequential.index_predecessor_steps,
            sequential.forward_chunk_crossings,
            positional.index_resolutions,
            positional.index_predecessor_steps,
        );
    }
}

#[test]
fn single_line_break_retains_ordered_allocator_phases() {
    let universe = TestState::new();
    let nodes = vec![rule(10), Node::Penalty(EJECT_PENALTY)];
    let plan = try_line_break_without_hyphenation(&universe, &nodes, &params(10))
        .expect("the forced one-line paragraph breaks");

    assert_eq!(
        plan.memory.search,
        vec![
            BreakMemoryEvent::Allocate {
                owner: BreakMemoryOwner::Active(0),
                words: 3,
            },
            BreakMemoryEvent::Free(BreakMemoryOwner::Active(0)),
            BreakMemoryEvent::Allocate {
                owner: BreakMemoryOwner::Passive(0),
                words: 2,
            },
            BreakMemoryEvent::Allocate {
                owner: BreakMemoryOwner::Active(1),
                words: 3,
            },
        ]
    );
    assert_eq!(
        plan.memory.cleanup,
        vec![
            BreakMemoryEvent::Free(BreakMemoryOwner::Active(1)),
            BreakMemoryEvent::Free(BreakMemoryOwner::Passive(0)),
        ]
    );
}

#[test]
fn tracing_omits_initial_second_pass_label_but_records_emergency_transition() {
    // TeX82 §816 begins the diagnostic silently when `pretolerance<0`;
    // `@secondpass` names only the transition from a failed first pass.
    let universe = TestState::new();
    let nodes = vec![rule(100), Node::Penalty(EJECT_PENALTY)];
    let mut parameters = params(10);
    parameters.pretolerance = -1;
    parameters.tolerance = -1;
    parameters.emergency_stretch = sp(100);

    let (plan, trace) = line_break_hyphenated_traced(&universe, &nodes, &parameters, Vec::new());
    assert!(!plan.breaks.is_empty());
    assert!(
        !trace
            .iter()
            .any(|event| matches!(event, LineBreakTrace::Pass(LineBreakPass::Second)))
    );
    assert!(
        trace
            .iter()
            .any(|event| matches!(event, LineBreakTrace::Pass(LineBreakPass::Emergency)))
    );
}

#[test]
fn tracing_reports_a_line_class_champion_before_the_next_class_feasible_route() {
    // TeX82 §§851--854 creates the first class's active node when traversal
    // reaches the next line number, before reporting that next route.
    let universe = TestState::new();
    let stretch = GlueSpec {
        stretch: sp(100),
        ..GlueSpec::ZERO
    };
    let nodes = vec![
        rule(40),
        Node::Glue {
            spec: stretch,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(40),
        Node::Penalty(EJECT_PENALTY),
    ];
    let mut parameters = params(100);
    parameters.pretolerance = 10_000;
    parameters.left_skip.stretch = sp(100);
    parameters.looseness = -1;

    let (_, trace) = line_break_hyphenated_traced(&universe, &nodes, &parameters, Vec::new());
    let terminal = trace
        .iter()
        .position(|event| {
            matches!(
                event,
                LineBreakTrace::Feasible {
                    breakpoint: TraceBreakpoint::Paragraph,
                    via: 0,
                    ..
                }
            )
        })
        .expect("the initial route reaches the forced paragraph break");

    assert!(
        matches!(
            trace.get(terminal + 1),
            Some(LineBreakTrace::Active { line: 1, .. })
        ),
        "{trace:?}"
    );
    assert!(
        matches!(
            trace.get(terminal + 2),
            Some(LineBreakTrace::Feasible {
                breakpoint: TraceBreakpoint::Paragraph,
                via,
                ..
            }) if *via != 0
        ),
        "{trace:?}"
    );
}

#[test]
fn tracing_active_lines_include_the_previous_paragraph_offset() {
    // TeX82 §§816/854 initializes active-node line numbers at `prev_graf+1`.
    let universe = TestState::new();
    let nodes = vec![rule(100), Node::Penalty(EJECT_PENALTY)];
    let mut parameters = params(100);
    parameters.pretolerance = 10_000;
    parameters.shape.line_offset = 3;

    let (_, trace) = line_break_hyphenated_traced(&universe, &nodes, &parameters, Vec::new());

    assert!(
        trace
            .iter()
            .any(|event| matches!(event, LineBreakTrace::Active { line: 4, .. })),
        "{trace:?}"
    );
}

/// tex.web §828: positive `emergency_stretch` keeps the tolerance threshold
/// and obtains a real feasible route instead of the final-pass artificial one.
#[test]
fn positive_emergency_stretch_uses_the_real_tolerance_route() {
    let universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let nodes = vec![
        rule(100),
        Node::Glue {
            spec: zero,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(200),
        Node::Penalty(EJECT_PENALTY),
    ];
    let mut parameters = params(200);
    parameters.pretolerance = -1;
    parameters.tolerance = 100;
    parameters.emergency_stretch = sp(100);

    let (plan, trace) = line_break_hyphenated_traced(&universe, &nodes, &parameters, Vec::new());

    assert_eq!(plan.breaks.last().map(|br| br.position), Some(nodes.len()));
    assert_eq!(
        trace
            .iter()
            .filter_map(|event| match event {
                LineBreakTrace::Pass(pass) => Some(*pass),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [LineBreakPass::Emergency]
    );
    assert!(
        trace.iter().any(|event| matches!(
            event,
            LineBreakTrace::Feasible {
                badness: Some(100),
                demerits: Some(_),
                breakpoint: TraceBreakpoint::Glue,
                ..
            }
        )),
        "{trace:?}"
    );
}

/// tex.web §831: penalties at or above `inf_penalty` inhibit a break,
/// while values at or below `eject_penalty` are normalized to a forced break.
#[test]
fn penalty_boundaries_match_infinite_and_eject_semantics() {
    let universe = TestState::new();
    let cases = [
        (-10_001, Some(EJECT_PENALTY)),
        (-10_000, Some(EJECT_PENALTY)),
        (-9_999, Some(-9_999)),
        (9_999, Some(9_999)),
        (10_000, None),
        (10_001, None),
    ];

    for (input, expected) in cases {
        let nodes = vec![rule(1), Node::Penalty(input), rule(1)];
        let breakpoints = legal_breakpoints(&universe, &nodes, &params(100));
        assert_eq!(
            breakpoints
                .iter()
                .find(|breakpoint| breakpoint.position == 2)
                .map(|breakpoint| breakpoint.penalty),
            expected,
            "penalty {input}"
        );
    }
}

#[test]
fn pdf_image_reference_contributes_width_to_line_measurement() {
    let mut universe = TestState::new();
    let image = Node::Whatsit(Whatsit::PdfRefXImage {
        object: 1,
        width: sp(30),
        height: sp(20),
        depth: sp(5),
    });

    let decoded = line_widths_nodes(&universe, std::slice::from_ref(&image));
    assert_eq!(decoded.natural, tex_arith::WideScaled::from_scaled(sp(30)));

    let list = universe.publish_page_nodes(&[image]);
    let compact = line_widths_view(&universe, &list, 0, 1, false);
    assert_eq!(compact.natural, tex_arith::WideScaled::from_scaled(sp(30)));
}

#[test]
fn base_whatsit_line_visitation_is_zero_width_and_never_a_breakpoint() {
    // TeX82 §1362: line-break traversal recognizes base whatsits without
    // measuring, breaking, executing, or reordering them. Language-state
    // interpretation belongs to the executor's pre-hyphenation visit.
    let universe = TestState::new();
    let tokens = tex_state::node::NodeTokenKey::default();
    let whatsits = vec![
        Node::Whatsit(Whatsit::OpenOut {
            slot: tex_state::StreamSlot::new(15),
            path: "visit.tex".into(),
        }),
        Node::Whatsit(Whatsit::DeferredWrite {
            sink: tex_state::PrintSink::Log,
            tokens,
        }),
        Node::Whatsit(Whatsit::CloseOut {
            slot: Some(tex_state::StreamSlot::new(0)),
        }),
        Node::Whatsit(Whatsit::CloseOut { slot: None }),
        Node::Whatsit(Whatsit::Special {
            class: "dvi".into(),
            payload: b"visit".to_vec(),
        }),
        Node::Whatsit(Whatsit::Language {
            language: 7,
            left_hyphen_min: 2,
            right_hyphen_min: 3,
        }),
    ];
    assert_eq!(
        line_widths_nodes(&universe, &whatsits),
        widths::Widths::zero()
    );

    let mut paragraph = whatsits.clone();
    paragraph.push(Node::Penalty(EJECT_PENALTY));
    let breakpoints = legal_breakpoints(&universe, &paragraph, &params(100));
    assert_eq!(breakpoints.len(), 1);
    assert_eq!(breakpoints[0].position, paragraph.len());
    assert_eq!(&paragraph[..whatsits.len()], whatsits);
    assert!(tokens.is_empty());
}

#[test]
fn etex_penalty_arrays_repeat_and_use_forward_and_reverse_indexes() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let zero = GlueSpec::ZERO;
    let breaks = vec![
        BreakDecision {
            position: 1,
            penalty: 0,
            hyphenated: false,
        },
        BreakDecision {
            position: 2,
            penalty: 0,
            hyphenated: false,
        },
        BreakDecision {
            position: 3,
            penalty: 0,
            hyphenated: false,
        },
        BreakDecision {
            position: 4,
            penalty: -10_000,
            hyphenated: false,
        },
    ];
    let post = PostLineBreakParams {
        empty_list: empty,
        left_skip: zero,
        right_skip: zero,
        interline_penalty: 99,
        club_penalty: 999,
        widow_penalties: ordinary_widow_penalties(9999, vec![2000, 1000]),
        broken_penalty: 0,
        prev_graf: 2,
        interline_penalties: vec![8, 7, 6],
        club_penalties: vec![200, 100],
        shape: LineShape::natural(sp(100)),
    };

    // Interline indexes include prev_graf (and hence repeat 6 here); club
    // indexes run forward, while widow indexes run backward from the end.
    assert_eq!(
        post::line_penalty_after(0, &breaks, false, &post),
        Some(1206)
    );
    assert_eq!(
        post::line_penalty_after(1, &breaks, false, &post),
        Some(1106)
    );
    assert_eq!(
        post::line_penalty_after(2, &breaks, false, &post),
        Some(2106)
    );
}

/// e-TeX 2.6 change [49.889] selects the display-widow family only for the
/// partial paragraph immediately before display math. Array indexes count
/// backward from that partial paragraph's end and repeat their final value.
#[test]
fn etex_display_widow_selector_survives_to_post_line_break() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let zero = GlueSpec::ZERO;
    let breaks = (1..=4)
        .map(|position| BreakDecision {
            position,
            penalty: if position == 4 { EJECT_PENALTY } else { 0 },
            hyphenated: false,
        })
        .collect::<Vec<_>>();
    let mut params = PostLineBreakParams {
        empty_list: empty,
        left_skip: zero,
        right_skip: zero,
        interline_penalty: 7,
        club_penalty: 0,
        widow_penalties: WidowPenalties {
            selector: WidowPenaltySelector::Ordinary,
            ordinary: PenaltySequence {
                fallback: 300,
                values: vec![2_000, 1_000],
            },
            display: PenaltySequence {
                fallback: 310,
                values: vec![2_200, 1_100, 0],
            },
        },
        broken_penalty: 0,
        prev_graf: 0,
        interline_penalties: Vec::new(),
        club_penalties: Vec::new(),
        shape: LineShape::natural(sp(100)),
    };
    let nodes = vec![rule(1), rule(2), rule(3), rule(4)];

    let penalties = |params: &PostLineBreakParams| {
        post_line_break(&universe, &nodes, &breaks, params.clone())
            .into_iter()
            .map(|line| line.penalty_after)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        penalties(&params),
        vec![Some(1_007), Some(1_007), Some(2_007), None]
    );

    params.widow_penalties.selector = WidowPenaltySelector::DisplayInterrupted;
    assert_eq!(
        penalties(&params),
        vec![Some(7), Some(1_107), Some(2_207), None]
    );

    params.widow_penalties.ordinary.values.clear();
    params.widow_penalties.display.values.clear();
    params.widow_penalties.selector = WidowPenaltySelector::Ordinary;
    assert_eq!(penalties(&params), vec![Some(7), Some(7), Some(307), None]);
    params.widow_penalties.selector = WidowPenaltySelector::DisplayInterrupted;
    assert_eq!(penalties(&params), vec![Some(7), Some(7), Some(317), None]);

    let one_line = [BreakDecision {
        position: 1,
        penalty: EJECT_PENALTY,
        hyphenated: false,
    }];
    assert_eq!(post::line_penalty_after(0, &one_line, false, &params), None);
}
