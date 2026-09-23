//! Owned, borrowed, and arena tape analysis and materialization.

use super::*;

#[test]
fn paragraph_tape_analyzes_twenty_thousand_nested_replacements_iteratively() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let mut replacement = universe.publish_page_nodes(&[rule(1)]);
    for _ in 0..20_000 {
        replacement = universe.publish_page_nodes(&[Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: replacement,
            physical_replace_count: 0,
        }]);
    }
    let nodes = vec![
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: replacement,
            physical_replace_count: 0,
        },
        Node::Penalty(-10_000),
    ];
    let parameters = params(100);
    let tape = ParagraphTape::analyze_borrowed(&universe, &nodes, &parameters);

    assert_eq!(tape.break_sites.len(), 2);
    assert_eq!(tape.break_sites[1].breakpoint.line_width.natural.raw(), 1);
}

#[test]
fn paired_materialization_cursor_preserves_physical_diagnostic_topology() {
    let mut universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
    let parameters = params(100);
    let semantic = universe.publish_owned_page_nodes(vec![Node::Penalty(1), Node::Penalty(2)]);
    let physical = universe.publish_owned_page_nodes(vec![
        Node::Penalty(10),
        Node::Penalty(11),
        Node::Penalty(12),
    ]);
    let tape = ParagraphTape::analyze_arena_projection_ids(
        &universe,
        semantic,
        physical,
        Some(vec![0, 2, 3]),
        &parameters,
    );
    let breaks = vec![
        BreakDecision {
            position: 1,
            penalty: 1,
            hyphenated: false,
        },
        BreakDecision {
            position: 2,
            penalty: -10_000,
            hyphenated: false,
        },
    ];
    let mut materializer = LineMaterializer::new(
        tape,
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
        .expect("first planned line materializes");

    assert_eq!(first.nodes[0], Node::Penalty(1));
    assert_eq!(
        first.physical_nodes[0..2],
        [Node::Penalty(10), Node::Penalty(11)]
    );
}

#[test]
fn borrowed_paragraph_tape_materializes_from_immutable_source_and_overlays_par_fill() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let original = GlueSpec {
        width: sp(3),
        stretch: sp(1),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
    let replacement = GlueSpec {
        width: sp(9),
        stretch: sp(2),
        ..GlueSpec::ZERO
    };
    let nodes = vec![
        rule(1),
        Node::Glue {
            spec: original,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
        Node::Penalty(-10_000),
    ];
    let mut tape = ParagraphTape::analyze_borrowed(&universe, &nodes, &params(100));
    tape.replace_last_par_fill(replacement);
    let mut materializer = LineMaterializer::new(
        tape,
        vec![BreakDecision {
            position: nodes.len(),
            penalty: EJECT_PENALTY,
            hyphenated: false,
        }],
        PostLineBreakParams {
            empty_list: empty,
            left_skip: GlueSpec::ZERO,
            right_skip: GlueSpec::ZERO,
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
    let line = materializer
        .materialize_next(&universe, Vec::new())
        .expect("borrowed line materializes");

    assert!(matches!(
        &nodes[1],
        Node::Glue { spec, .. } if *spec == original
    ));
    assert!(line.nodes.iter().any(|node| matches!(
        node,
        Node::Glue {
            spec,
            kind: GlueKind::ParFillSkip,
            ..
        } if *spec == replacement
    )));
}

#[test]
fn owned_slice_and_composite_arena_paragraphs_agree() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let source = vec![
        rule(12),
        Node::Glue {
            spec: GlueSpec {
                width: sp(4),
                stretch: sp(20),
                ..GlueSpec::ZERO
            },
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(13),
        Node::Penalty(EJECT_PENALTY),
    ];
    let pieces = source
        .chunks(1)
        .map(|chunk| universe.publish_owned_page_nodes(chunk.to_vec()))
        .collect::<Vec<_>>();
    let sequence = universe.compose_page_node_sequences(&pieces);
    let arena_view = universe
        .page_node_sequence(sequence)
        .expect("paragraph resolves");
    let source_nodes = arena_view
        .iter()
        .map(|node| node.to_owned())
        .collect::<Vec<_>>();
    let line_params = params(18);
    let slice_tape = ParagraphTape::analyze_borrowed(&universe, &source, &line_params);
    let arena_tape = ParagraphTape::analyze_arena(&universe, arena_view, &line_params);

    assert_eq!(arena_tape.break_sites, slice_tape.break_sites);
    assert_eq!(arena_tape.materialization, slice_tape.materialization);
    let slice_plan = break_hyphenated_tape(&universe, &slice_tape, &line_params);
    let arena_plan = break_hyphenated_tape(&universe, &arena_tape, &line_params);
    assert_eq!(arena_plan, slice_plan);

    let post_params = PostLineBreakParams {
        empty_list: empty,
        left_skip: GlueSpec::ZERO,
        right_skip: GlueSpec::ZERO,
        interline_penalty: 0,
        club_penalty: 0,
        widow_penalties: ordinary_widow_penalties(0, Vec::new()),
        broken_penalty: 0,
        prev_graf: 0,
        interline_penalties: Vec::new(),
        club_penalties: Vec::new(),
        shape: LineShape::natural(sp(18)),
    };
    let mut slice_materializer =
        LineMaterializer::new(slice_tape, slice_plan.breaks, post_params.clone());
    let mut arena_materializer = LineMaterializer::new(arena_tape, arena_plan.breaks, post_params);
    loop {
        let slice_line = slice_materializer.materialize_next(&universe, Vec::new());
        let arena_line = arena_materializer.materialize_next(&universe, Vec::new());
        assert_eq!(arena_line, slice_line);
        if arena_line.is_none() {
            break;
        }
    }
    assert_eq!(
        universe
            .page_node_sequence(sequence)
            .expect("source remains live")
            .iter()
            .map(|node| node.to_owned())
            .collect::<Vec<_>>(),
        source_nodes,
        "analysis and materialization retain the original arena payload"
    );
}

#[test]
fn coordinate_paragraph_tape_reborrows_arena_between_execution_steps() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let left = universe.publish_owned_page_nodes(vec![rule(8)]);
    let right = universe.publish_owned_page_nodes(vec![
        Node::Glue {
            spec: GlueSpec {
                width: sp(2),
                stretch: sp(10),
                ..GlueSpec::ZERO
            },
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(9),
        Node::Penalty(EJECT_PENALTY),
    ]);
    let sequence = universe.compose_page_node_sequences(&[left, right]);
    let line_params = params(12);
    let tape = ParagraphTape::analyze_arena_id(&universe, sequence, &line_params);
    let plan = break_hyphenated_tape(&universe, &tape, &line_params);

    // The tape owns only a coordinate and compact analysis scratch. The page
    // arena can keep appending between analysis and materialization.
    let _unrelated = universe.publish_owned_page_nodes(vec![Node::Penalty(77)]);
    let mut materializer = LineMaterializer::new(
        tape,
        plan.breaks,
        PostLineBreakParams {
            empty_list: empty,
            left_skip: GlueSpec::ZERO,
            right_skip: GlueSpec::ZERO,
            interline_penalty: 0,
            club_penalty: 0,
            widow_penalties: ordinary_widow_penalties(0, Vec::new()),
            broken_penalty: 0,
            prev_graf: 0,
            interline_penalties: Vec::new(),
            club_penalties: Vec::new(),
            shape: LineShape::natural(sp(12)),
        },
    );
    let lines = std::iter::from_fn(|| materializer.materialize_next(&universe, Vec::new()))
        .collect::<Vec<_>>();
    assert!(!lines.is_empty());
    assert_eq!(
        universe
            .page_node_sequence(sequence)
            .expect("coordinate source remains live")
            .iter()
            .map(|node| node.to_owned())
            .collect::<Vec<_>>(),
        [
            rule(8),
            Node::Glue {
                spec: GlueSpec {
                    width: sp(2),
                    stretch: sp(10),
                    ..GlueSpec::ZERO
                },
                kind: GlueKind::Normal,
                leader: None,
            },
            rule(9),
            Node::Penalty(EJECT_PENALTY),
        ]
    );
}

#[test]
fn coordinate_paragraph_tape_keeps_distinct_physical_channel_in_arena() {
    let mut universe = TestState::new();
    let empty = universe.publish_page_nodes(&[]);
    let semantic = universe.publish_owned_page_nodes(vec![Node::Penalty(1), Node::Penalty(3)]);
    let physical = universe.publish_owned_page_nodes(vec![
        Node::Penalty(1),
        Node::Penalty(2),
        Node::Penalty(3),
    ]);
    let tape = ParagraphTape::analyze_arena_projection_ids(
        &universe,
        semantic,
        physical,
        Some(vec![0, 2, 3]),
        &params(100),
    );
    let mut materializer = LineMaterializer::new(
        tape,
        vec![BreakDecision {
            position: 2,
            penalty: EJECT_PENALTY,
            hyphenated: false,
        }],
        PostLineBreakParams {
            empty_list: empty,
            left_skip: GlueSpec::ZERO,
            right_skip: GlueSpec::ZERO,
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
    let line = materializer
        .materialize_next(&universe, Vec::new())
        .expect("projected line materializes");

    assert_eq!(&line.nodes[..2], [Node::Penalty(1), Node::Penalty(3)]);
    assert_eq!(
        &line.physical_nodes[..3],
        [Node::Penalty(1), Node::Penalty(2), Node::Penalty(3)]
    );
}

#[test]
fn materialized_final_line_preserves_two_direct_and_four_frozen_lig_ptr_cells() {
    let mut universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
    let lig = |ch, orig: [char; 2]| Node::Lig {
        font: NULL_FONT,
        ch,
        orig: orig.to_vec(),
        left_hit: false,
        right_hit: false,
        origins: vec![OriginId::UNKNOWN; 2],
    };
    let bb = universe.publish_page_nodes(&[lig('A', ['B', 'B'])]);
    let ca = universe.publish_page_nodes(&[lig('\u{82}', ['C', 'A'])]);
    let character = |ch| Node::Char {
        font: NULL_FONT,
        ch,
        origin: OriginId::UNKNOWN,
    };
    let disc = |replace| Node::Disc {
        kind: DiscKind::AutomaticHyphen,
        pre: empty,
        post: empty,
        replace,
        physical_replace_count: 1,
    };
    let nodes = vec![character('A'), character('/'), disc(bb), disc(ca)];
    let tape = ParagraphTape::analyze_borrowed(&universe, &nodes, &params(100));
    let mut materializer = LineMaterializer::new(
        tape,
        vec![BreakDecision {
            position: 4,
            penalty: EJECT_PENALTY,
            hyphenated: false,
        }],
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
    let line = materializer
        .materialize_next(&universe, Vec::new())
        .expect("final line");

    assert_eq!(
        tex_state::node_sequence::direct_high_cell_overlap(
            &line.high_cell_lineages,
            &line.physical_high_cell_lineages,
        ),
        6
    );
    assert_eq!(
        line.high_cell_lineages
            .iter()
            .filter(|lineage| matches!(
                lineage,
                tex_state::node_sequence::DirectHighCellLineage::Frozen {
                    role: tex_state::node_sequence::FrozenListRole::Replace,
                    ..
                }
            ))
            .count(),
        4
    );
}
