use super::*;
use tex_state::Universe;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{DiscKind, GlueKind, KernKind, Node, Whatsit};
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
        last_line_fit: 0,
        pdf_adjust_spacing: 0,
        expansion_steps: None,
        pdf_protrude_chars: 0,
        left_skip: GlueSpec::ZERO,
        right_skip: GlueSpec::ZERO,
        par_fill_skip: GlueSpec::ZERO,
        shape: LineShape::natural(sp(width)),
    }
}

#[test]
fn pdf_image_reference_contributes_width_to_line_measurement() {
    let mut universe = Universe::new();
    let image = Node::Whatsit(Whatsit::PdfRefXImage {
        object: 1,
        width: sp(30),
        height: sp(20),
        depth: sp(5),
    });

    let decoded = line_widths_nodes(&universe, std::slice::from_ref(&image));
    assert_eq!(decoded.natural, tex_arith::WideScaled::from_scaled(sp(30)));

    let list = universe.freeze_node_list(&[image]);
    let compact = line_widths_view(&universe, universe.nodes(list), 0, 1);
    assert_eq!(compact.natural, tex_arith::WideScaled::from_scaled(sp(30)));
}

#[test]
fn etex_penalty_arrays_repeat_and_use_forward_and_reverse_indexes() {
    let mut universe = Universe::new();
    let empty = universe.freeze_node_list(&[]);
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
        left_skip: tex_state::ids::GlueId::ZERO,
        right_skip: tex_state::ids::GlueId::ZERO,
        interline_penalty: 99,
        club_penalty: 999,
        widow_penalty: 9999,
        broken_penalty: 0,
        prev_graf: 2,
        interline_penalties: vec![8, 7, 6],
        club_penalties: vec![200, 100],
        widow_penalties: vec![2000, 1000],
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
fn etex_last_line_fit_uses_previous_lines_finite_glue_ratio() {
    let mut universe = Universe::new();
    let finite = universe.intern_glue(GlueSpec {
        width: sp(5 * Scaled::UNITY),
        stretch: sp(20 * Scaled::UNITY),
        stretch_order: Order::Normal,
        shrink: sp(4 * Scaled::UNITY),
        shrink_order: Order::Normal,
    });
    let par_fill_spec = GlueSpec {
        width: sp(0),
        stretch: sp(Scaled::UNITY),
        stretch_order: Order::Fill,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
    let par_fill = universe.intern_glue(par_fill_spec);
    let mut nodes = Vec::new();
    for index in 0..5 {
        nodes.push(rule(30 * Scaled::UNITY));
        if index != 4 {
            nodes.push(Node::Glue {
                spec: finite,
                kind: GlueKind::Normal,
                leader: None,
            });
        }
    }
    nodes.push(Node::Penalty(INF_PENALTY));
    nodes.push(Node::Glue {
        spec: par_fill,
        kind: GlueKind::ParFillSkip,
        leader: None,
    });

    let mut parameters = params(110 * Scaled::UNITY);
    parameters.pretolerance = 9_000;
    parameters.last_line_fit = 500;
    parameters.par_fill_skip = par_fill_spec;
    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, parameters.clone(), &mut hook);
    assert_eq!(
        result.last_line_fill.map(|spec| spec.width),
        Some(sp(42 * Scaled::UNITY + Scaled::UNITY / 2))
    );

    parameters.last_line_fit = 1_000;
    let result = line_break(&universe, &nodes, parameters.clone(), &mut hook);
    assert_eq!(
        result.last_line_fill.map(|spec| spec.width),
        Some(sp(40 * Scaled::UNITY))
    );

    // The e-TeX manual requires finite left/right-skip stretch. An infinite
    // component in the background disables the extension entirely.
    parameters.right_skip = par_fill_spec;
    let result = line_break(&universe, &nodes, parameters, &mut hook);
    assert_eq!(result.last_line_fill, None);
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
fn paragraph_prefix_widths_remain_exact_past_i32_max() {
    let mut universe = Universe::new();
    let zero = universe.intern_glue(GlueSpec::ZERO);
    let mut nodes = Vec::new();
    for index in 0..6 {
        nodes.push(rule(700_000_000));
        if index != 5 {
            nodes.push(Node::Glue {
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

#[test]
fn final_pass_keeps_last_active_route_when_every_route_is_overfull() {
    let mut universe = Universe::new();
    let glue = universe.intern_glue(GlueSpec::ZERO);
    let nodes = vec![
        rule(100),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(1_000),
    ];
    let mut hook = NoHyphenation;

    let result = line_break(&universe, &nodes, params(100), &mut hook);

    assert_eq!(
        result.breaks.last().map(|br| br.position),
        Some(nodes.len())
    );
}

#[test]
fn consecutive_discardable_breakpoints_do_not_form_a_backwards_chain() {
    let mut universe = Universe::new();
    let zero = universe.intern_glue(GlueSpec::ZERO);
    let empty = universe.freeze_node_list(&[]);
    let nodes = vec![rule(1), Node::Penalty(0), Node::Penalty(0), rule(1)];
    let mut break_params = params(100);
    break_params.looseness = 2;
    let mut hook = NoHyphenation;

    let result = line_break(&universe, &nodes, break_params, &mut hook);
    let lines = post_line_break(
        &universe,
        &nodes,
        &result.breaks,
        PostLineBreakParams {
            empty_list: empty,
            left_skip: zero,
            right_skip: zero,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalty: 0,
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            widow_penalties: Vec::new(),
            shape: LineShape::natural(sp(100)),
        },
    );

    assert!(!lines.is_empty());
    assert!(
        result
            .breaks
            .windows(2)
            .all(|pair| pair[0].position < pair[1].position)
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
        serial: position,
        position,
        width_position: position,
        start_width: Widths::zero(),
        penalty: 0,
        line: 2,
        fitness,
        path_demerits: 221,
        passive: None,
        previous: Some(0),
        hyphenated: false,
        line_shortfall: sp(0),
        line_glue: sp(0),
    };
    let candidates = [
        candidate(0, Fitness::Decent),
        candidate(4, Fitness::Decent),
        candidate(6, Fitness::Decent),
        candidate(6, Fitness::Loose),
    ];
    let mut active = Vec::new();

    record_best_route(&mut active, 0, candidates[1]);
    record_best_route(&mut active, 0, candidates[2]);
    record_best_route(&mut active, 0, candidates[3]);

    assert_eq!(
        active
            .iter()
            .map(|candidate| candidate.position)
            .collect::<Vec<_>>(),
        vec![6, 6]
    );
}

#[test]
fn active_list_order_matches_tex_for_equal_demerit_discretionary_routes() {
    let mut universe = Universe::new();
    let empty = universe.freeze_node_list(&[]);
    let nonempty = universe.freeze_node_list(&[kern(0)]);
    let right_skip = GlueSpec {
        stretch: sp(1),
        stretch_order: Order::Fil,
        ..GlueSpec::ZERO
    };
    let par_fill = universe.intern_glue(GlueSpec::ZERO);
    let disc = |pre| Node::Disc {
        kind: DiscKind::ExplicitHyphen,
        pre,
        post: empty,
        replace: empty,
    };
    // This is the equal-demerit shape used by TRIP's line-breaking test.
    // TeX keeps active nodes ordered by line number and reverse breakpoint
    // position, selecting the early (2, 6) route rather than (6, 13).
    let nodes = vec![
        kern(0),
        disc(nonempty),
        kern(0),
        rule(0),
        disc(empty),
        disc(nonempty),
        kern(0),
        rule(0),
        rule(0),
        disc(empty),
        kern(0),
        rule(0),
        disc(nonempty),
        Node::Penalty(10_000),
        Node::Glue {
            spec: par_fill,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
    ];
    let mut p = params(20);
    p.line_penalty = 1;
    p.hyphen_penalty = 88;
    p.ex_hyphen_penalty = 89;
    p.double_hyphen_demerits = 1_000;
    p.final_hyphen_demerits = 100_000;
    p.looseness = 2;
    p.right_skip = right_skip;
    let mut hook = NoHyphenation;

    let result = line_break(&universe, &nodes, p, &mut hook);

    assert_eq!(
        result
            .breaks
            .iter()
            .map(|decision| decision.position)
            .collect::<Vec<_>>(),
        vec![2, 6, 15]
    );
}

#[test]
fn easy_line_active_nodes_accumulate_in_source_order() {
    let candidate = |position| Candidate {
        serial: position,
        position,
        width_position: position,
        start_width: Widths::zero(),
        penalty: 0,
        line: 9,
        fitness: Fitness::Decent,
        path_demerits: 0,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: sp(0),
        line_glue: sp(0),
    };
    let candidates = [candidate(0), candidate(14), candidate(15)];
    let p = params(100);
    let mut active = vec![candidates[2], candidates[1]];

    sort_active_candidates(&mut active, &p, tex_easy_line(&p));

    assert_eq!(
        active
            .iter()
            .map(|candidate| candidate.position)
            .collect::<Vec<_>>(),
        vec![14, 15]
    );
}

#[test]
fn incremental_active_merge_matches_full_total_order() {
    let candidate = |serial, line, position| Candidate {
        serial,
        position,
        width_position: position,
        start_width: Widths::zero(),
        penalty: 0,
        line,
        fitness: Fitness::Decent,
        path_demerits: 0,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: sp(0),
        line_glue: sp(0),
    };
    let p = params(100);
    let easy_line = tex_easy_line(&p);
    let mut seed = 0x9e37_79b9_u64;
    for case in 0..256 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let survivor_count = (seed as usize >> 8) % 32;
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let winner_count = (seed as usize >> 8) % 12;
        let mut next_candidate = |serial| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let line = 1 + (seed as usize >> 16) % 12;
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let position = (seed as usize >> 16) % 500;
            candidate(serial, line, position)
        };
        let mut survivors = (0..survivor_count)
            .map(&mut next_candidate)
            .collect::<Vec<_>>();
        sort_active_candidates(&mut survivors, &p, easy_line);
        let winners = (0..winner_count)
            .map(|index| next_candidate(10_000 + case * 16 + index))
            .collect::<Vec<_>>();

        let mut expected = survivors.clone();
        expected.extend_from_slice(&winners);
        sort_active_candidates(&mut expected, &p, easy_line);

        let mut actual = survivors;
        let winner_start = actual.len();
        actual.extend_from_slice(&winners);
        let mut scratch = Vec::new();
        merge_active_candidates(
            &mut actual,
            survivor_count,
            winner_start,
            winner_count,
            &mut scratch,
            &p,
            easy_line,
        );
        assert_eq!(
            actual
                .iter()
                .map(|candidate| candidate.serial)
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|candidate| candidate.serial)
                .collect::<Vec<_>>(),
            "partition {case}"
        );
    }
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
fn discardable_tail_does_not_create_an_empty_final_line() {
    let mut universe = Universe::new();
    let trailing = universe.intern_glue(GlueSpec {
        width: sp(10),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(10),
        shrink_order: Order::Normal,
    });
    let par_fill = universe.intern_glue(GlueSpec {
        width: sp(0),
        stretch: sp(1),
        stretch_order: Order::Fil,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
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
    let mut universe = Universe::new();
    let empty = universe.freeze_node_list(&[]);
    let hyphen = universe.freeze_node_list(&[rule(5)]);
    let par_fill = universe.intern_glue(GlueSpec {
        width: sp(0),
        stretch: sp(1),
        stretch_order: Order::Fil,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
    let nodes = vec![
        rule(20),
        Node::Disc {
            kind: DiscKind::ExplicitHyphen,
            pre: hyphen,
            post: empty,
            replace: empty,
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
fn unmet_looseness_retries_after_the_pretolerance_pass() {
    let mut universe = Universe::new();
    let break_glue = universe.intern_glue(GlueSpec {
        width: sp(0),
        stretch: sp(100),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
    let par_fill = universe.intern_glue(GlueSpec {
        width: sp(0),
        stretch: sp(1),
        stretch_order: Order::Fil,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
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
    let breakpoints = legal_breakpoints(&universe, &nodes, &params(15));

    assert_eq!(breakpoints.first().map(|br| br.position), Some(2));
    let zero = universe.intern_glue(GlueSpec::ZERO);
    let empty = universe.freeze_node_list(&[]);
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
            widow_penalty: 0,
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            widow_penalties: Vec::new(),
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
fn math_boundaries_suppress_internal_glue_and_kern_breaks() {
    let mut universe = Universe::new();
    let glue = universe.intern_glue(GlueSpec {
        width: sp(10),
        stretch: sp(10),
        stretch_order: Order::Normal,
        shrink: sp(5),
        shrink_order: Order::Normal,
    });
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
fn discretionary_penalty_depends_on_pre_break_text() {
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
        },
        kern(20),
        rule(1),
    ];
    let breakpoints = legal_breakpoints(&universe, &nodes, &params);
    assert_eq!(breakpoints.first().map(|br| br.penalty), Some(654));
}

#[test]
fn font_kern_is_not_discarded_at_start_of_next_line() {
    let nodes = [Node::Kern {
        amount: sp(1),
        kind: KernKind::Font,
    }];

    assert_eq!(next_width_position(&nodes, 0), 0);
}

#[test]
fn existing_discretionary_is_available_on_the_pretolerance_pass() {
    struct UnexpectedHyphenation;

    impl HyphenationHook<Universe> for UnexpectedHyphenation {
        fn hyphenate(&mut self, _nodes: &[Node]) -> Vec<Node> {
            panic!("a feasible first pass must not invoke automatic hyphenation")
        }
    }

    let mut universe = Universe::new();
    let pre = universe.freeze_node_list(&[kern(1)]);
    let empty = universe.freeze_node_list(&[]);
    let par_fill = universe.intern_glue(GlueSpec {
        width: sp(0),
        stretch: sp(1),
        stretch_order: Order::Fil,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
    let nodes = vec![
        kern(20),
        Node::Disc {
            kind: DiscKind::ExplicitHyphen,
            pre,
            post: empty,
            replace: empty,
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
        width_position: 0,
        start_width: Widths::zero(),
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
        width_position: 1,
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
    let mut universe = Universe::new();
    let empty_glue = universe.intern_glue(GlueSpec::ZERO);
    let empty = universe.freeze_node_list(&[]);
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
            empty_list: empty,
            left_skip: empty_glue,
            right_skip: empty_glue,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalty: 0,
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            widow_penalties: Vec::new(),
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
fn post_line_break_closes_and_resumes_open_tex_xet_segments() {
    use tex_state::node::Direction;

    let mut universe = Universe::new();
    let zero = universe.intern_glue(GlueSpec::ZERO);
    let empty = universe.freeze_node_list(&[]);
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
            widow_penalty: 0,
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            widow_penalties: Vec::new(),
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
fn post_line_break_clears_materialized_unbroken_discretionary_replacement() {
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
            empty_list: empty,
            left_skip: zero,
            right_skip: zero,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalty: 0,
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            widow_penalties: Vec::new(),
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
        ] if first.raw() == 3 && *retained_replacement == empty && second.raw() == 7
    ));
}

#[test]
fn line_materializer_reuses_the_returned_line_buffer() {
    let mut universe = Universe::new();
    let zero = universe.intern_glue(GlueSpec::ZERO);
    let empty = universe.freeze_node_list(&[]);
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
    let mut materializer = LineMaterializer::new(
        nodes,
        breaks,
        PostLineBreakParams {
            empty_list: empty,
            left_skip: zero,
            right_skip: zero,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalty: 0,
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            widow_penalties: Vec::new(),
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
    let mut universe = Universe::new();
    let zero = universe.intern_glue(GlueSpec::ZERO);
    let empty = universe.freeze_node_list(&[]);
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
            empty_list: empty,
            left_skip: zero,
            right_skip: zero,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalty: 0,
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            widow_penalties: Vec::new(),
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
            widow_penalty: 0,
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            widow_penalties: Vec::new(),
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
