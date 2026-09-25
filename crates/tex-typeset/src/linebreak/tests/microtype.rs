//! pdfTeX font expansion and protrusion at line breaks.

use super::*;

/// pdftex.web §821 keeps kern expansion differences signed: a negative kern
/// becomes more negative at the stretch endpoint and therefore reduces the
/// paragraph's total stretch and shrink capacities.
#[test]
fn pdftex_font_kern_expansion_capacity_retains_signed_differences() {
    let widths = |kern| {
        let mut universe = TestState::new();
        let font = universe.intern_font(microtype_kern_font("signed-kern", 100, kern));
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
        universe.set_pdf_font_code(tex_state::PdfFontCode::Ef, font, b'A', 1000);
        universe.set_pdf_font_code(tex_state::PdfFontCode::Ef, font, b'B', 1000);
        let nodes = [
            microtype_char(font, 'A'),
            Node::Kern {
                amount: sp(kern),
                kind: KernKind::Font,
            },
            microtype_char(font, 'B'),
        ];
        line_widths_nodes(&universe, &nodes)
    };

    let negative = widths(-100);
    assert_eq!(negative.font_stretch.raw(), 50);
    assert_eq!(negative.font_shrink.raw(), 50);

    let positive = widths(100);
    assert_eq!(positive.font_stretch.raw(), 150);
    assert_eq!(positive.font_shrink.raw(), 150);
}

/// pdftex.web §823 ignores a discretionary node while the final
/// `hpack(..., cal_expand_ratio)` measures the replacement nodes that
/// post-line-break processing has already placed in the physical line.
#[test]
fn final_line_expansion_does_not_count_discretionary_replacement_twice() {
    let mut universe = TestState::new();
    let font = universe.intern_font(microtype_font("final-disc", 1_000));
    universe
        .configure_font_expansion(
            font,
            FontExpansion {
                stretch: 20,
                shrink: 20,
                step: 1,
                auto_expand: true,
            },
        )
        .expect("microtype font expansion configuration is valid");
    universe.set_pdf_font_code(tex_state::PdfFontCode::Ef, font, b'A', 1_000);

    let empty = universe.publish_page_nodes(&[]);
    let replacement = universe.publish_page_nodes(&[Node::Kern {
        amount: sp(-100),
        kind: KernKind::Font,
    }]);
    let mut line = vec![microtype_char(font, 'A'); 100];
    line.insert(
        50,
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty,
            post: empty,
            replace: replacement,
            physical_replace_count: 1,
        },
    );

    assert_eq!(plan_line_expansion(&universe, &line, sp(101_900)), 950);
}

/// pdftex.web §1025: a glue breakpoint itself is not part of the candidate
/// line. Right-edge discovery therefore starts with the preceding glyph,
/// whose protrusion can make the nonhyphenating pass feasible.
#[test]
fn pdftex_protrusion_scores_the_glyph_before_breakpoint_glue() {
    let mut universe = TestState::new();
    let font = universe.intern_font(microtype_font("punctuation-edge", 100));
    universe.set_pdf_font_code(tex_state::PdfFontCode::Rp, font, b'B', 100);
    let finite = GlueSpec {
        width: sp(10),
        stretch: sp(20),
        stretch_order: Order::Normal,
        shrink: sp(4),
        shrink_order: Order::Normal,
    };
    let break_glue = GlueSpec {
        width: sp(10),
        ..GlueSpec::ZERO
    };
    let par_fill = GlueSpec {
        stretch: sp(1),
        stretch_order: Order::Fill,
        ..GlueSpec::ZERO
    };
    let nodes = vec![
        microtype_char(font, 'A'),
        Node::Glue {
            origin: tex_state::node::GlueSpecOrigin::Owned,
            spec: finite,
            kind: GlueKind::Normal,
            leader: None,
        },
        microtype_char(font, 'B'),
        Node::Glue {
            origin: tex_state::node::GlueSpecOrigin::Owned,
            spec: break_glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        microtype_char(font, 'C'),
        Node::Penalty(INF_PENALTY),
        Node::Glue {
            origin: tex_state::node::GlueSpecOrigin::Owned,
            spec: par_fill,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
    ];

    let mut without = params(205);
    without.pdf_protrude_chars = 1;
    assert_eq!(
        try_line_break_without_hyphenation(&universe, &nodes, &without),
        None,
        "without scoring protrusion, five units exceed four units of shrink"
    );

    let mut with = without;
    with.pdf_protrude_chars = 2;
    let plan = try_line_break_without_hyphenation(&universe, &nodes, &with)
        .expect("B's ten-unit protrusion makes the first pass feasible");
    assert_eq!(
        plan.breaks
            .iter()
            .map(|decision| decision.position)
            .collect::<Vec<_>>(),
        vec![4, nodes.len()]
    );
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
        for code in *b"ABCD-." {
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
                    origin: tex_state::node::GlueSpecOrigin::Owned,
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'B'),
                Node::Glue {
                    origin: tex_state::node::GlueSpecOrigin::Owned,
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_ligature(first, 'C'),
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
                ([5].as_slice(), 144),
            ],
        ),
        (
            "shrink",
            230,
            vec![
                microtype_char(first, 'A'),
                Node::Glue {
                    origin: tex_state::node::GlueSpecOrigin::Owned,
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'B'),
                Node::Glue {
                    origin: tex_state::node::GlueSpecOrigin::Owned,
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'C'),
                Node::Glue {
                    origin: tex_state::node::GlueSpecOrigin::Owned,
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(first, 'D'),
            ],
            [
                ([4, 7].as_slice(), 22_100),
                ([4, 7].as_slice(), 22_100),
                ([6, 7].as_slice(), 144),
                ([4, 7].as_slice(), 22_100),
                ([4, 7].as_slice(), 22_100),
                ([6, 7].as_slice(), 144),
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
                    origin: tex_state::node::GlueSpecOrigin::Owned,
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
                ([2, 5].as_slice(), 8_084),
            ],
        ),
        (
            "mixed-font",
            180,
            vec![
                microtype_char(first, 'A'),
                Node::Glue {
                    origin: tex_state::node::GlueSpecOrigin::Owned,
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                microtype_char(second, 'B'),
                Node::Glue {
                    origin: tex_state::node::GlueSpecOrigin::Owned,
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
            origin: tex_state::node::GlueSpecOrigin::Owned,
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        rule(100),
        Node::Glue {
            origin: tex_state::node::GlueSpecOrigin::Owned,
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
