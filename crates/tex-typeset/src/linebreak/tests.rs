use super::*;
use crate::test_state::TestState;
use tex_fonts::metrics::CharTag;
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont};
use tex_state::font::FontExpansion;
use tex_state::font::NULL_FONT;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{DiscKind, GlueKind, KernKind, Node, Whatsit};
use tex_state::scaled::Scaled;
use tex_state::token::OriginId;
use tex_state::token::{Catcode, Token};

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

fn ordinary_widow_penalties(fallback: i32, values: Vec<i32>) -> WidowPenalties {
    WidowPenalties {
        selector: WidowPenaltySelector::Ordinary,
        ordinary: PenaltySequence { fallback, values },
        display: PenaltySequence {
            fallback: 0,
            values: Vec::new(),
        },
    }
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
    let tokens =
        tex_state::node::NodeTokenList::new([tex_state::token::TokenWord::pack(Token::Char {
            ch: 'w',
            cat: Catcode::Letter,
        })]);
    let whatsits = vec![
        Node::Whatsit(Whatsit::OpenOut {
            slot: tex_state::StreamSlot::new(15),
            path: "visit.tex".into(),
        }),
        Node::Whatsit(Whatsit::DeferredWrite {
            sink: tex_state::PrintSink::Log,
            tokens: tokens.clone(),
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
    assert_eq!(
        tokens.words(),
        [tex_state::token::TokenWord::pack(Token::Char {
            ch: 'w',
            cat: Catcode::Letter,
        })]
    );
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

fn microtype_font(name: &str, width: i32) -> LoadedFont {
    let mut characters = vec![None; 256];
    for code in [b'A', b'B', b'C', b'D', b'-', b'.'] {
        characters[usize::from(code)] = Some(CharMetrics {
            width: sp(width),
            height: sp(0),
            depth: sp(0),
            italic_correction: sp(0),
            tag: CharTag::None,
        });
    }
    let mut parameters = vec![sp(0); 7];
    parameters[5] = sp(width);
    LoadedFont::new(
        name,
        format!("{name}.tfm"),
        [width as u8; 8],
        0,
        sp(width),
        sp(width),
        parameters,
        FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
    )
}

fn microtype_char(font: tex_state::ids::FontId, ch: char) -> Node {
    Node::Char {
        font,
        ch,
        origin: OriginId::UNKNOWN,
    }
}

/// pdftex.web §§20580--21220 and §§24321--26029: adjustment/protrusion mode
/// 1 affects only selected-line materialization, while mode 2 participates in
/// `try_break`. Keep exact winners and demerits for finite stretch, finite
/// shrink, discretionary, and mixed-font candidates.
#[test]
fn pdftex_hz_modes_have_the_exact_scoring_and_breakpoint_matrix() {
    let mut universe = TestState::new();
    let first = universe.intern_font(microtype_font("first", 100));
    let second = universe.intern_font(microtype_font("second", 80));
    for font in [first, second] {
        universe
            .configure_font_expansion(
                font,
                FontExpansion {
                    stretch: 500,
                    shrink: 500,
                    step: 100,
                    auto_expand: true,
                },
            )
            .expect("microtype font expansion configuration is valid");
        for code in [b'A', b'B', b'C', b'D', b'-', b'.'] {
            universe.set_pdf_font_code(tex_state::PdfFontCode::Ef, font, code, 1000);
            universe.set_pdf_font_code(tex_state::PdfFontCode::Lp, font, code, 500);
            universe.set_pdf_font_code(tex_state::PdfFontCode::Rp, font, code, 500);
        }
    }
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(20),
        stretch_order: Order::Normal,
        shrink: sp(10),
        shrink_order: Order::Normal,
    };
    let empty = universe.publish_page_nodes(&[]);
    let pre = universe.publish_page_nodes(&[microtype_char(first, '-')]);
    let scenarios = [
        (
            "stretch",
            230,
            vec![
                microtype_char(first, 'A'),
                Node::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'B'),
                Node::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'C'),
            ],
            [
                ([4, 5].as_slice(), 22_100),
                ([4, 5].as_slice(), 22_100),
                ([5].as_slice(), 0),
                ([4, 5].as_slice(), 22_100),
                ([4, 5].as_slice(), 22_100),
                ([5].as_slice(), 0),
                ([5].as_slice(), 2_704),
                ([5].as_slice(), 2_704),
                ([5].as_slice(), 225),
            ],
        ),
        (
            "shrink",
            230,
            vec![
                microtype_char(first, 'A'),
                Node::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'B'),
                Node::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'C'),
                Node::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'D'),
            ],
            [
                ([4, 7].as_slice(), 22_100),
                ([4, 7].as_slice(), 22_100),
                ([6, 7].as_slice(), 0),
                ([4, 7].as_slice(), 22_100),
                ([4, 7].as_slice(), 22_100),
                ([6, 7].as_slice(), 0),
                ([7].as_slice(), 100),
                ([7].as_slice(), 100),
                ([7].as_slice(), 1_600),
            ],
        ),
        (
            "discretionary",
            200,
            vec![
                microtype_char(first, 'A'),
                Node::Disc {
                    kind: DiscKind::ExplicitHyphen,
                    pre,
                    post: empty,
                    replace: empty,
                    physical_replace_count: 0,
                },
                microtype_char(first, 'B'),
                Node::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'C'),
            ],
            [
                ([2, 5].as_slice(), 19_700),
                ([2, 5].as_slice(), 19_700),
                ([5].as_slice(), 0),
                ([2, 5].as_slice(), 19_700),
                ([2, 5].as_slice(), 19_700),
                ([5].as_slice(), 0),
                ([2, 5].as_slice(), 19_700),
                ([2, 5].as_slice(), 19_700),
                ([2, 5].as_slice(), 2_600),
            ],
        ),
        (
            "mixed-font",
            180,
            vec![
                microtype_char(first, 'A'),
                Node::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(second, 'B'),
                Node::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'C'),
            ],
            [
                ([4, 5].as_slice(), 12_100),
                ([4, 5].as_slice(), 12_100),
                ([5].as_slice(), 0),
                ([4, 5].as_slice(), 12_100),
                ([4, 5].as_slice(), 12_100),
                ([5].as_slice(), 0),
                ([5].as_slice(), 1_936),
                ([5].as_slice(), 1_936),
                ([5].as_slice(), 1_936),
            ],
        ),
    ];
    for (name, width, nodes, expected) in scenarios {
        let mut index = 0;
        for adjust in 0..=2 {
            for protrude in 0..=2 {
                let mut p = params(width);
                p.pretolerance = -1;
                p.tolerance = 500;
                p.pdf_adjust_spacing = adjust;
                p.expansion_steps = (adjust > 1).then_some((5, 5));
                p.pdf_protrude_chars = protrude;
                let mut hook = NoHyphenation;
                let result = line_break(&universe, &nodes, p, &mut hook);
                let positions = result
                    .breaks
                    .iter()
                    .map(|br| br.position)
                    .collect::<Vec<_>>();
                assert_eq!(
                    (positions.as_slice(), result.demerits),
                    expected[index],
                    "{name}: adjust={adjust}, protrude={protrude}"
                );
                index += 1;
            }
        }
    }
}

#[test]
fn pdftex_hz_mode_two_is_inert_without_pdftex_font_configuration() {
    let universe = TestState::new();
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(20),
        stretch_order: Order::Normal,
        shrink: sp(10),
        shrink_order: Order::Normal,
    };
    let nodes = vec![
        rule(100),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(100),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(100),
    ];
    let run = |adjust, protrude| {
        let mut p = params(230);
        p.pretolerance = -1;
        p.tolerance = 500;
        p.pdf_adjust_spacing = adjust;
        p.expansion_steps = (adjust > 1).then_some((5, 5));
        p.pdf_protrude_chars = protrude;
        let mut hook = NoHyphenation;
        let result = line_break(&universe, &nodes, p, &mut hook);
        (
            result
                .breaks
                .iter()
                .map(|br| br.position)
                .collect::<Vec<_>>(),
            result.demerits,
        )
    };
    assert_eq!(run(2, 2), run(0, 0));
}

fn last_line_fit_paragraph() -> (TestState, Vec<Node>, LineBreakParams) {
    let universe = TestState::new();
    let finite = GlueSpec {
        width: sp(5 * Scaled::UNITY),
        stretch: sp(20 * Scaled::UNITY),
        stretch_order: Order::Normal,
        shrink: sp(4 * Scaled::UNITY),
        shrink_order: Order::Normal,
    };
    let par_fill_spec = GlueSpec {
        width: sp(0),
        stretch: sp(Scaled::UNITY),
        stretch_order: Order::Fill,
        shrink: sp(0),
        shrink_order: Order::Normal,
    };
    let par_fill = par_fill_spec;
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
    (universe, nodes, parameters)
}

/// e-TeX change-file sections 38.827 and 38.851--38.855: positive values
/// below 1000 scale the inherited finite-glue ratio, while 1000 and larger
/// use it unchanged. Nonpositive values retain TeX's ordinary final glue.
#[test]
fn etex_last_line_fit_numeric_boundaries_preserve_break_and_artifact_state() {
    let (universe, nodes, parameters) = last_line_fit_paragraph();
    let cases = [
        (-1, &[6, 11][..], 244, None),
        (0, &[6, 11][..], 244, None),
        (
            1,
            &[6, 11][..],
            244,
            Some(45 * Scaled::UNITY - (5 * Scaled::UNITY + 500) / 1000),
        ),
        (
            500,
            &[6, 11][..],
            244,
            Some(42 * Scaled::UNITY + Scaled::UNITY / 2),
        ),
        (1_000, &[6, 11][..], 288, Some(40 * Scaled::UNITY)),
        (1_001, &[6, 11][..], 288, Some(40 * Scaled::UNITY)),
        (i32::MAX, &[6, 11][..], 288, Some(40 * Scaled::UNITY)),
    ];

    for (last_line_fit, expected_breaks, expected_demerits, expected_width) in cases {
        let mut parameters = parameters.clone();
        parameters.last_line_fit = last_line_fit;
        let mut hook = NoHyphenation;
        let result = line_break(&universe, &nodes, parameters, &mut hook);
        assert_eq!(
            result
                .breaks
                .iter()
                .map(|br| br.position)
                .collect::<Vec<_>>(),
            expected_breaks,
            "last_line_fit={last_line_fit}"
        );
        assert_eq!(
            result.last_line_fill.map(|spec| spec.width.raw()),
            expected_width,
            "last_line_fit={last_line_fit}"
        );
        assert_eq!(
            result.demerits, expected_demerits,
            "last_line_fit={last_line_fit}"
        );
        assert_eq!(
            result
                .last_line_fill
                .map(|spec| (spec.stretch.raw(), spec.stretch_order)),
            expected_width.map(|_| (0, Order::Fill)),
            "last_line_fit={last_line_fit}"
        );
    }
}

/// e-TeX change-file section 38.846 prints the saved shortfall and glue (or
/// final adjustment) from every active node while last-line fitting is active.
#[test]
fn etex_last_line_fit_trace_retains_active_node_diagnostic_words() {
    let (universe, nodes, parameters) = last_line_fit_paragraph();

    let (_, trace) = try_line_break_without_hyphenation_traced(&universe, &nodes, &parameters);
    let active = trace
        .iter()
        .filter_map(|event| match event {
            LineBreakTrace::Active { last_line_fit, .. } => Some(*last_line_fit),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(!active.is_empty());
    assert!(active.iter().all(|evidence| evidence.is_some()));
    assert!(
        active
            .iter()
            .flatten()
            .any(|evidence| evidence.terminal && evidence.glue.raw() != 0),
        "{active:?}"
    );
}

/// e-TeX change-file section 38.827: the extension requires positive
/// infinite `par_fill_skip` stretch and finite `left_skip + right_skip`.
#[test]
fn etex_last_line_fit_enablement_requires_exact_infinite_fill_component() {
    let (universe, nodes, parameters) = last_line_fit_paragraph();
    let finite = GlueSpec {
        stretch: sp(Scaled::UNITY),
        stretch_order: Order::Normal,
        ..parameters.par_fill_skip
    };
    let zero = GlueSpec {
        stretch: sp(0),
        ..parameters.par_fill_skip
    };
    let negative = GlueSpec {
        stretch: sp(-Scaled::UNITY),
        ..parameters.par_fill_skip
    };
    let infinite_background = GlueSpec {
        stretch: sp(Scaled::UNITY),
        stretch_order: Order::Fil,
        ..GlueSpec::ZERO
    };
    let cases = [
        (finite, GlueSpec::ZERO, "finite par_fill_skip"),
        (zero, GlueSpec::ZERO, "zero par_fill_skip stretch"),
        (negative, GlueSpec::ZERO, "negative par_fill_skip stretch"),
        (
            parameters.par_fill_skip,
            infinite_background,
            "infinite right_skip stretch",
        ),
    ];

    for (par_fill_skip, right_skip, label) in cases {
        let mut parameters = parameters.clone();
        parameters.par_fill_skip = par_fill_skip;
        parameters.right_skip = right_skip;
        let mut hook = NoHyphenation;
        let result = line_break(&universe, &nodes, parameters, &mut hook);
        assert_eq!(result.breaks.last().map(|br| br.position), Some(11));
        assert_eq!(result.last_line_fill, None, "{label}");
    }
}

/// e-TeX change-file sections 38.851--38.855: the saved preceding-line
/// shortfall selects finite stretch or shrink, and every missing or infinite
/// component falls back to the ordinary last-line calculation.
#[test]
fn etex_last_line_fit_adjustment_and_disable_state_boundaries() {
    let (_, _, mut parameters) = last_line_fit_paragraph();
    parameters.last_line_fit = 500;
    let fit = LastLineFit::new(&parameters, Widths::zero());
    assert!(fit.enabled);

    let previous = |line_shortfall, line_glue| Candidate {
        serial: 1,
        position: 1,
        width_position: 1,
        start_width: Widths::zero(),
        penalty: 0,
        line: 1,
        fitness: Fitness::Decent,
        path_demerits: 0,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: sp(line_shortfall),
        line_glue: sp(line_glue),
    };
    let terminal_widths = |natural, stretch, shrink, extra_fill| {
        let mut widths = Widths::from_glue(GlueSpec {
            width: sp(natural),
            stretch: sp(stretch),
            stretch_order: Order::Normal,
            shrink: sp(shrink),
            shrink_order: Order::Normal,
        });
        widths.add_assign(Widths::from_glue(parameters.par_fill_skip));
        if extra_fill != 0 {
            widths.add_assign(Widths::from_glue(GlueSpec {
                stretch: sp(extra_fill),
                stretch_order: Order::Fil,
                ..GlueSpec::ZERO
            }));
        }
        widths
    };

    assert_eq!(
        fit.badness(
            &previous(20 * Scaled::UNITY, 40 * Scaled::UNITY),
            terminal_widths(70 * Scaled::UNITY, 30 * Scaled::UNITY, 0, 0),
            sp(100 * Scaled::UNITY),
        ),
        Some((2, Fitness::Decent, sp(15 * Scaled::UNITY / 2)))
    );
    assert_eq!(
        fit.badness(
            &previous(-20 * Scaled::UNITY, 10 * Scaled::UNITY),
            terminal_widths(70 * Scaled::UNITY, 0, 20 * Scaled::UNITY, 0),
            sp(100 * Scaled::UNITY),
        ),
        Some((100, Fitness::Tight, sp(-20 * Scaled::UNITY)))
    );
    assert_eq!(
        fit.adjusted_fill(&previous(30 * Scaled::UNITY, 15 * Scaled::UNITY / 2))
            .map(|spec| (spec.width.raw(), spec.stretch.raw())),
        Some((45 * Scaled::UNITY / 2, 0))
    );
    assert_eq!(
        fit.adjusted_fill(&previous(-10 * Scaled::UNITY, -20 * Scaled::UNITY))
            .map(|spec| (spec.width.raw(), spec.stretch.raw())),
        Some((10 * Scaled::UNITY, 0))
    );

    let disabled = [
        fit.badness(
            &previous(0, 40 * Scaled::UNITY),
            terminal_widths(70 * Scaled::UNITY, 30 * Scaled::UNITY, 0, 0),
            sp(100 * Scaled::UNITY),
        ),
        fit.badness(
            &previous(20 * Scaled::UNITY, 0),
            terminal_widths(70 * Scaled::UNITY, 30 * Scaled::UNITY, 0, 0),
            sp(100 * Scaled::UNITY),
        ),
        fit.badness(
            &previous(20 * Scaled::UNITY, 40 * Scaled::UNITY),
            terminal_widths(70 * Scaled::UNITY, 0, 0, 0),
            sp(100 * Scaled::UNITY),
        ),
        fit.badness(
            &previous(-20 * Scaled::UNITY, 10 * Scaled::UNITY),
            terminal_widths(110 * Scaled::UNITY, 0, 0, 0),
            sp(100 * Scaled::UNITY),
        ),
        fit.badness(
            &previous(20 * Scaled::UNITY, 40 * Scaled::UNITY),
            terminal_widths(70 * Scaled::UNITY, 30 * Scaled::UNITY, 0, Scaled::UNITY),
            sp(100 * Scaled::UNITY),
        ),
    ];
    assert_eq!(disabled, [None; 5]);
}

/// e-TeX change-file section 38.852: the special last-line computation is
/// reached only for a short line with infinite stretch. A long terminal line
/// retains TeX's ordinary finite-shrink badness even when the preceding line
/// saved a positive stretch ratio.
#[test]
fn etex_last_line_fit_preserves_ordinary_badness_for_long_terminal_line() {
    let (_, _, mut parameters) = last_line_fit_paragraph();
    parameters.last_line_fit = 500;
    let fit = LastLineFit::new(&parameters, Widths::zero());
    let previous = Candidate {
        serial: 1,
        position: 1,
        width_position: 1,
        start_width: Widths::zero(),
        penalty: 0,
        line: 1,
        fitness: Fitness::Decent,
        path_demerits: 0,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: sp(31 * Scaled::UNITY),
        line_glue: sp(20 * Scaled::UNITY),
    };
    let mut terminal_widths = Widths::from_glue(GlueSpec {
        width: sp(100 * Scaled::UNITY),
        stretch: sp(40 * Scaled::UNITY),
        stretch_order: Order::Normal,
        shrink: sp(8 * Scaled::UNITY),
        shrink_order: Order::Normal,
    });
    terminal_widths.add_assign(Widths::from_glue(parameters.par_fill_skip));
    let target = sp(96 * Scaled::UNITY);

    assert_eq!(fit.badness(&previous, terminal_widths, target), None);
    assert_eq!(
        line_badness(terminal_widths, target, Scaled::from_raw(0), None),
        12
    );
}

/// e-TeX change-file sections 38.851 and 38.863: a one-line paragraph has
/// no preceding finite-glue ratio to inherit, so its terminal artifact stays
/// ordinary even when the global enablement conditions hold.
#[test]
fn etex_last_line_fit_does_not_adjust_a_single_line_paragraph() {
    let universe = TestState::new();
    let par_fill_spec = GlueSpec {
        stretch: sp(Scaled::UNITY),
        stretch_order: Order::Fill,
        ..GlueSpec::ZERO
    };
    let par_fill = par_fill_spec;
    let nodes = vec![
        rule(30 * Scaled::UNITY),
        Node::Penalty(INF_PENALTY),
        Node::Glue {
            spec: par_fill,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
    ];
    let mut parameters = params(110 * Scaled::UNITY);
    parameters.last_line_fit = 500;
    parameters.par_fill_skip = par_fill_spec;
    let mut hook = NoHyphenation;

    let result = line_break(&universe, &nodes, parameters, &mut hook);

    assert_eq!(
        result
            .breaks
            .iter()
            .map(|br| br.position)
            .collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(result.demerits, 100);
    assert_eq!(result.last_line_fill, None);
}

/// e-TeX change-file sections 38.827 and 38.851--38.855, together with
/// TeX.web section 828: emergency stretch is finite background stretch and
/// therefore participates in both saved preceding-line glue and final fit.
#[test]
fn etex_last_line_fit_is_applied_on_the_emergency_final_pass() {
    let (universe, nodes, mut parameters) = last_line_fit_paragraph();
    parameters.pretolerance = -1;
    parameters.tolerance = 0;
    parameters.emergency_stretch = sp(20 * Scaled::UNITY);

    let (result, trace) = line_break_hyphenated_traced(&universe, &nodes, &parameters, Vec::new());

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
    assert_eq!(
        result
            .breaks
            .iter()
            .map(|br| br.position)
            .collect::<Vec<_>>(),
        [6, 11]
    );
    assert!(trace.iter().any(|event| matches!(
        event,
        LineBreakTrace::Feasible {
            breakpoint: TraceBreakpoint::Paragraph,
            demerits: None,
            ..
        }
    )));
    assert_eq!(
        result
            .last_line_fill
            .map(|spec| (spec.width.raw(), spec.stretch.raw())),
        Some((41 * Scaled::UNITY + 2 * Scaled::UNITY / 3, 0))
    );
}

#[test]
fn breaks_at_legal_glue() {
    let universe = TestState::new();
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(10),
        stretch_order: Order::Normal,
        shrink: sp(5),
        shrink_order: Order::Normal,
    };
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
        width_position: position,
        start_width: Widths::zero(),
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
        width_position: serial,
        start_width: Widths::zero(),
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
    let universe = TestState::new();
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
    let nodes = [Node::Kern {
        amount: sp(1),
        kind: KernKind::Font,
    }];

    assert_eq!(next_width_position(&nodes, 0), 0);
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
    let mark_tokens =
        tex_state::node::NodeTokenList::new([tex_state::token::TokenWord::pack(Token::Char {
            ch: 'm',
            cat: Catcode::Letter,
        })]);
    let adjust_content = universe.publish_page_nodes(&[kern(7)]);
    let nodes = vec![
        rule(10),
        Node::Mark {
            class: 0,
            tokens: mark_tokens.clone(),
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
    let mut materializer = LineMaterializer::from_nodes(
        nodes,
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
    let tape = ParagraphTape::analyze(
        &universe,
        tex_state::node_sequence::NodeSequence::mirrored(nodes),
        &parameters,
    );

    assert_eq!(tape.materialization.len(), tape.nodes().len());
    assert!(tape.break_sites.len() <= tape.nodes().len() + 1);
    assert_eq!(std::mem::size_of::<MaterializationAction>(), 1);
}

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
    let tape = ParagraphTape::analyze(
        &universe,
        tex_state::node_sequence::NodeSequence::mirrored(nodes),
        &parameters,
    );

    assert_eq!(tape.break_sites.len(), 2);
    assert_eq!(tape.break_sites[1].breakpoint.line_width.natural.raw(), 1);
}

#[test]
fn paired_materialization_cursor_preserves_physical_diagnostic_topology() {
    let mut universe = TestState::new();
    let zero = GlueSpec::ZERO;
    let empty = universe.publish_page_nodes(&[]);
    let parameters = params(100);
    let tape = ParagraphTape::analyze(
        &universe,
        tex_state::node_sequence::NodeSequence::from_projection(
            vec![Node::Penalty(1), Node::Penalty(2)],
            vec![Node::Penalty(10), Node::Penalty(11), Node::Penalty(12)],
            vec![0, 2, 3],
        ),
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
    let tape = ParagraphTape::analyze(
        &universe,
        tex_state::node_sequence::NodeSequence::from_channels(nodes.clone(), nodes),
        &params(100),
    );
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
