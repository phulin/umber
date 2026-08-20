use super::*;
use crate::FreezeNodeListRefForTest;
use tex_fonts::metrics::CharTag;
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont};
use tex_state::Universe;
use tex_state::glue::GlueSpec;
use tex_state::node::KernKind;

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
        [42; 32],
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
        origin: tex_state::provenance::OriginRef::unknown(),
    }
}

#[test]
fn computes_pdftex_edge_amounts_from_font_quad_and_codes() {
    let mut state = Universe::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    state.set_pdf_font_code(PdfFontCode::Rp, font, b'.', 700);

    let protrusion = line_protrusion(&state, &[character(font, 'A'), character(font, '.')]);
    assert_eq!(protrusion.left, sp(5 * 65_536));
    assert_eq!(protrusion.right, sp(7 * 65_536));
    assert_eq!(protrusion.total(), sp(12 * 65_536));
}

#[test]
fn materializes_margin_kerns_inside_paragraph_skip_glue() {
    let mut state = Universe::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    state.set_pdf_font_code(PdfFontCode::Rp, font, b'.', 700);
    let zero = state.intern_glue(GlueSpec::ZERO);
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
    let mut state = Universe::new();
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
    let mut state = Universe::new();
    let font = state.intern_font(protruding_font());
    state.set_pdf_font_code(PdfFontCode::Lp, font, b'A', 500);
    state.set_pdf_font_code(PdfFontCode::Rp, font, b'A', 500);
    let empty = state.freeze_node_list_ref_for_test(&[]);
    let zero_glue = state.intern_glue(GlueSpec::ZERO);
    let wide_glue = state.intern_glue(GlueSpec {
        width: sp(1),
        ..GlueSpec::ZERO
    });

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
            pre: empty.clone(),
            post: empty.clone(),
            replace: empty.clone(),
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
