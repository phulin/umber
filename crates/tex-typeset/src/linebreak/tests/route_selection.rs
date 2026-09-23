//! Active-route ordering, pruning, demerits, and width accounting.

use super::*;

#[test]
fn final_pass_keeps_last_active_route_when_every_route_is_overfull() {
    let universe = TestState::new();
    let glue = GlueSpec::ZERO;
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
    let mut universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
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
            widow_penalties: ordinary_widow_penalties(0, Vec::new()),
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
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

/// tex.web §§822/851--854: advancing a break-width cursor across
/// discardable material does not suppress a syntactically later breakpoint.
#[test]
fn glue_route_is_considered_at_immediately_following_forced_penalty() {
    let universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let nodes = vec![
        rule(1),
        Node::Glue {
            spec: zero,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Penalty(EJECT_PENALTY),
        Node::Penalty(INF_PENALTY),
    ];
    let mut parameters = params(100);
    parameters.pretolerance = 10_000;

    let (plan, trace) = try_line_break_without_hyphenation_traced(&universe, &nodes, &parameters);
    plan.expect("the unhyphenated pass finds the forced break");
    let glue_serial = trace
        .iter()
        .find_map(|event| match event {
            LineBreakTrace::Active {
                serial,
                previous: 0,
                ..
            } => Some(*serial),
            _ => None,
        })
        .expect("the glue breakpoint creates an active route");

    assert!(
        trace.iter().any(|event| matches!(
            event,
            LineBreakTrace::Feasible {
                breakpoint: TraceBreakpoint::Penalty,
                via,
                penalty: EJECT_PENALTY,
                ..
            } if *via == glue_serial
        )),
        "{trace:?}"
    );
}

#[test]
fn line_break_includes_left_and_right_skip_in_background_widths() {
    let universe = TestState::new();
    let break_glue = GlueSpec::ZERO;
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
        break_site: INITIAL_BREAK_SITE,
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

    record_best_route(&mut active, 0, candidates[1], None);
    record_best_route(&mut active, 0, candidates[2], None);
    record_best_route(&mut active, 0, candidates[3], None);

    assert_eq!(
        active
            .iter()
            .map(|candidate| candidate.position)
            .collect::<Vec<_>>(),
        vec![6, 6]
    );
}

#[test]
fn winner_lookup_replaces_only_the_latest_line_class_champion() {
    let candidate = |position, line, fitness, path_demerits| Candidate {
        serial: position,
        position,
        break_site: INITIAL_BREAK_SITE,
        penalty: 0,
        line,
        fitness,
        path_demerits,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: sp(0),
        line_glue: sp(0),
    };
    let mut active = Vec::new();
    for line in 1..=4_096 {
        record_best_route(
            &mut active,
            0,
            candidate(line, line, Fitness::Decent, 1_000),
            None,
        );
    }

    record_best_route(
        &mut active,
        0,
        candidate(8_192, 4_096, Fitness::Decent, 999),
        None,
    );
    record_best_route(
        &mut active,
        0,
        candidate(8_193, 4_096, Fitness::Loose, 1_001),
        None,
    );

    assert_eq!(active.len(), 4_097);
    assert_eq!(active[4_095].position, 8_192);
    assert_eq!(active[4_096].position, 8_193);
}

#[test]
fn equivalent_line_classes_discard_noncompetitive_fitness_routes() {
    let candidate = |serial, line, fitness, path_demerits| Candidate {
        serial,
        position: serial,
        break_site: INITIAL_BREAK_SITE,
        penalty: 0,
        line,
        fitness,
        path_demerits,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: sp(0),
        line_glue: sp(0),
    };
    let mut active = vec![
        candidate(1, 1, Fitness::Loose, 2_704),
        candidate(2, 2, Fitness::Decent, 100_000_782),
        candidate(3, 3, Fitness::Tight, 3_000),
    ];

    let retained = retain_competitive_routes(&mut active, 0, 10_000, 0);

    assert_eq!(retained, 2);
    assert_eq!(
        active
            .iter()
            .map(|candidate| candidate.serial)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
}

#[test]
fn active_list_order_matches_tex_for_equal_demerit_discretionary_routes() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let nonempty = universe.publish_page_nodes(&[kern(0)]);
    let right_skip = GlueSpec {
        stretch: sp(1),
        stretch_order: Order::Fil,
        ..GlueSpec::ZERO
    };
    let par_fill = GlueSpec::ZERO;
    let disc = |pre| Node::Disc {
        kind: DiscKind::ExplicitHyphen,
        pre,
        post: empty,
        replace: empty,
        physical_replace_count: 0,
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
    let candidate = |serial, line, fitness, position| Candidate {
        serial,
        position,
        break_site: INITIAL_BREAK_SITE,
        penalty: 0,
        line,
        fitness,
        path_demerits: 0,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: sp(0),
        line_glue: sp(0),
    };
    let candidates = [
        candidate(0, 0, Fitness::Decent, 0),
        candidate(1, 2, Fitness::Tight, 14),
        candidate(2, 3, Fitness::Decent, 14),
        candidate(3, 2, Fitness::Loose, 15),
    ];
    let p = params(100);
    let mut active = vec![candidates[3], candidates[1], candidates[2]];

    sort_active_candidates(&mut active, &p, tex_easy_line(&p));

    assert_eq!(
        active
            .iter()
            .map(|candidate| candidate.position)
            .collect::<Vec<_>>(),
        vec![14, 14, 15]
    );
    assert_eq!(
        active
            .iter()
            .map(|candidate| candidate.fitness)
            .collect::<Vec<_>>(),
        vec![Fitness::Decent, Fitness::Tight, Fitness::Loose],
        "TeX82 §§848/853 ignore raw line numbers beyond easy_line and retain fitness insertion order"
    );
}

#[test]
fn incremental_active_merge_matches_full_total_order() {
    let candidate = |serial, line, position| Candidate {
        serial,
        position,
        break_site: INITIAL_BREAK_SITE,
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
