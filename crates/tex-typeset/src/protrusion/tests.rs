use super::*;
use crate::test_state::TestState;
use tex_fonts::metrics::CharTag;
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont};
use tex_state::font::FontExpansion;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, KernKind, Sign};
use tex_state::scaled::GlueSetRatio;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

fn protruding_font() -> LoadedFont {
    let mut characters = vec![None; 256];
    let metrics = CharMetrics {
        width: sp(0),
        height: sp(0),
        depth: sp(0),
        italic_correction: sp(0),
        tag: CharTag::None,
    };
    characters[usize::from(b'A')] = Some(metrics);
    characters[usize::from(b'.')] = Some(metrics);
    let mut parameters = vec![sp(0); 7];
    parameters[5] = sp(10 * 65_536);
    LoadedFont::new(
        "microtype",
        "microtype.tfm",
        [42; 8],
        0,
        sp(10 * 65_536),
        sp(10 * 65_536),
        parameters,
        FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
    )
}

fn character(font: tex_state::ids::FontId, ch: char) -> Node {
    Node::Char {
        font,
        ch,
        origin: tex_state::token::OriginId::UNKNOWN,
    }
}

fn ligature(font: tex_state::ids::FontId, ch: char) -> Node {
    Node::Lig {
        font,
        ch,
        orig: vec![ch],
        left_hit: false,
        right_hit: false,
        origins: vec![tex_state::token::OriginId::UNKNOWN],
    }
}

fn hlist(
    children: tex_state::node_arena::PageListId,
    width: Scaled,
    height: Scaled,
    depth: Scaled,
) -> Node {
    Node::HList(BoxNode::new(BoxNodeFields {
        width,
        height,
        depth,
        shift: sp(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))
}

#[test]
fn computes_pdftex_edge_amounts_from_font_quad_and_codes() {
    let mut state = TestState::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    state.set_pdf_font_code(PdfFontCode::Rp, font, b'.', 700);

    let protrusion = line_protrusion(&state, &[character(font, 'A'), character(font, '.')]);
    assert_eq!(protrusion.left, sp(5 * 65_536));
    assert_eq!(protrusion.right, sp(7 * 65_536));
    assert_eq!(protrusion.total(), sp(12 * 65_536));
}

#[test]
fn zero_width_stretch_glue_blocks_edge_discovery() {
    let mut state = TestState::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Rp, font, b'.', 700);
    let par_fill = GlueSpec {
        stretch: sp(65_536),
        stretch_order: tex_state::glue::Order::Fil,
        ..GlueSpec::ZERO
    };

    let protrusion = line_protrusion(
        &state,
        &[
            character(font, '.'),
            Node::Glue {
                spec: par_fill,
                kind: GlueKind::ParFillSkip,
                leader: None,
            },
        ],
    );

    assert_eq!(protrusion.right, sp(0));
}

/// pdftex.web §§24540--24615 retain a blocking node found while descending
/// through a nonempty hlist. The parent box's dimensions do not turn that
/// blocker into transparent material.
#[test]
fn nested_hlist_preserves_blocking_child_at_both_edges() {
    let mut state = TestState::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    state.set_pdf_font_code(PdfFontCode::Rp, font, b'A', 500);
    let stretch_glue = GlueSpec {
        stretch: sp(65_536),
        stretch_order: Order::Fil,
        ..GlueSpec::ZERO
    };
    let children = state.publish_page_nodes(&[Node::Glue {
        spec: stretch_glue,
        kind: GlueKind::Normal,
        leader: None,
    }]);
    let box_node = hlist(children, sp(0), sp(0), sp(0));

    assert_eq!(
        line_protrusion(&state, &[box_node.clone(), character(font, 'A')]).left,
        sp(0)
    );
    assert_eq!(
        line_protrusion(&state, &[character(font, 'A'), box_node]).right,
        sp(0)
    );
}

#[test]
fn nested_hlist_only_skips_when_its_contents_are_transparent() {
    let mut state = TestState::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    state.set_pdf_font_code(PdfFontCode::Rp, font, b'A', 500);
    let children = state.publish_page_nodes(&[
        Node::Penalty(123),
        Node::Kern {
            amount: sp(0),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: GlueSpec::ZERO,
            kind: GlueKind::Normal,
            leader: None,
        },
    ]);
    // Canonical edge discovery descends into a nonempty hlist regardless of
    // its dimensions, then resumes at the parent only if every child skips.
    let box_node = hlist(children, sp(7), sp(3), sp(2));

    assert_eq!(
        line_protrusion(&state, &[box_node.clone(), character(font, 'A')]).left,
        sp(5 * 65_536)
    );
    assert_eq!(
        line_protrusion(&state, &[character(font, 'A'), box_node]).right,
        sp(5 * 65_536)
    );
}

#[test]
fn empty_hlist_with_vertical_extent_blocks_edge_discovery() {
    let mut state = TestState::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    state.set_pdf_font_code(PdfFontCode::Rp, font, b'A', 500);
    let empty = state.publish_page_nodes(&[]);

    let tall = hlist(empty, sp(0), sp(1), sp(0));
    assert_eq!(
        line_protrusion(&state, &[tall.clone(), character(font, 'A')]).left,
        sp(0)
    );
    assert_eq!(
        line_protrusion(&state, &[character(font, 'A'), tall]).right,
        sp(0)
    );
}

#[test]
fn margin_variation_uses_left_codes_only_for_expandable_ligature_edges() {
    let mut state = TestState::new();
    let font = state.intern_font(protruding_font());
    state
        .configure_font_expansion(
            font,
            FontExpansion {
                stretch: 20,
                shrink: 20,
                step: 1,
                auto_expand: true,
            },
        )
        .expect("font expansion configuration is valid");
    for code in [b'A', b'.'] {
        state.set_pdf_font_code(PdfFontCode::Ef, font, code, 1000);
    }
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'.', 300);

    let protrusion = line_protrusion(&state, &[character(font, 'A'), ligature(font, '.')]);
    assert_eq!(
        protrusion.margin_variation(),
        (sp(-3 * 65_536), sp(3 * 65_536))
    );

    let protrusion = line_protrusion(&state, &[ligature(font, 'A'), ligature(font, '.')]);
    assert_eq!(
        protrusion.margin_variation(),
        (sp(-8 * 65_536), sp(8 * 65_536))
    );

    state.set_pdf_font_code(PdfFontCode::Ef, font, b'.', 0);
    let protrusion = line_protrusion(&state, &[character(font, 'A'), ligature(font, '.')]);
    assert_eq!(protrusion.margin_variation(), (sp(0), sp(0)));
}

#[test]
fn materializes_margin_kerns_inside_paragraph_skip_glue() {
    let mut state = TestState::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    state.set_pdf_font_code(PdfFontCode::Rp, font, b'.', 700);
    let zero = GlueSpec::ZERO;
    let mut nodes = vec![
        Node::Glue {
            spec: zero,
            kind: GlueKind::LeftSkip,
            leader: None,
        },
        character(font, 'A'),
        character(font, '.'),
        Node::Glue {
            spec: zero,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
        Node::Glue {
            spec: zero,
            kind: GlueKind::RightSkip,
            leader: None,
        },
    ];

    insert_margin_kerns(&state, &mut nodes);

    assert!(matches!(
        nodes[1],
        Node::MarginKern {
            amount,
            side: MarginKernSide::Left,
            font: source_font,
            ch: b'A',
        } if amount == sp(-5 * 65_536) && source_font == font
    ));
    assert!(matches!(
        nodes[4],
        Node::MarginKern {
            amount,
            side: MarginKernSide::Right,
            font: source_font,
            ch: b'.',
        } if amount == sp(-7 * 65_536) && source_font == font
    ));
    assert!(matches!(
        nodes[5],
        Node::Glue {
            kind: GlueKind::ParFillSkip,
            ..
        }
    ));
}

#[test]
fn nonzero_material_blocks_edge_search() {
    let mut state = TestState::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    let nodes = [
        Node::Kern {
            amount: sp(1),
            kind: KernKind::Explicit,
        },
        character(font, 'A'),
    ];

    assert_eq!(line_protrusion(&state, &nodes).left, sp(0));
}

/// pdftex.web's `find_protchar_left`/`find_protchar_right` distinguish
/// transparent bookkeeping from material with horizontal extent. Keep the
/// complete distinction table here: it is easy for a newly added node kind to
/// accidentally inherit the wrong edge behavior from a catch-all arm.
#[test]
fn edge_search_distinguishes_transparent_zero_width_and_blocking_material() {
    let mut state = TestState::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    state.set_pdf_font_code(PdfFontCode::Rp, font, b'A', 500);
    let empty = state.publish_page_nodes(&[]);
    let zero_glue = GlueSpec::ZERO;
    let wide_glue = GlueSpec {
        width: sp(1),
        ..GlueSpec::ZERO
    };

    let transparent = [
        Node::Penalty(123),
        Node::Kern {
            amount: sp(0),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: zero_glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Disc {
            kind: tex_state::node::DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: empty,
            physical_replace_count: 0,
        },
    ];
    for node in transparent {
        assert_eq!(
            line_protrusion(&state, &[node.clone(), character(font, 'A')]).left,
            sp(5 * 65_536),
            "left edge should skip {node:?}"
        );
        assert_eq!(
            line_protrusion(&state, &[character(font, 'A'), node.clone()]).right,
            sp(5 * 65_536),
            "right edge should skip {node:?}"
        );
    }

    let blockers = [
        Node::Kern {
            amount: sp(1),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: wide_glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Rule {
            width: Some(sp(0)),
            height: Some(sp(0)),
            depth: Some(sp(0)),
        },
    ];
    for node in blockers {
        assert_eq!(
            line_protrusion(&state, &[node.clone(), character(font, 'A')]).left,
            sp(0),
            "left edge should stop at {node:?}"
        );
        assert_eq!(
            line_protrusion(&state, &[character(font, 'A'), node.clone()]).right,
            sp(0),
            "right edge should stop at {node:?}"
        );
    }
}
