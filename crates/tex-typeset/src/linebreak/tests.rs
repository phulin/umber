use super::*;
use crate::test_state::TestState;
use tex_fonts::metrics::CharTag;
use tex_fonts::{CharMetrics, FontMetrics, LigKernCommand, LigKernInstruction, LoadedFont};
use tex_state::font::FontExpansion;
use tex_state::font::NULL_FONT;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{DiscKind, GlueKind, KernKind, Node, Whatsit};
use tex_state::scaled::Scaled;
use tex_state::token::OriginId;

#[path = "tests/analysis.rs"]
mod analysis;
#[path = "tests/last_line_fit.rs"]
mod last_line_fit;
#[path = "tests/microtype.rs"]
mod microtype;
#[path = "tests/paragraph_tape.rs"]
mod paragraph_tape;
#[path = "tests/post_break.rs"]
mod post_break;
#[path = "tests/route_selection.rs"]
mod route_selection;
#[path = "tests/shape_and_breaks.rs"]
mod shape_and_breaks;
#[path = "tests/tracing.rs"]
mod tracing;

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
    for code in *b"ABCD-." {
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

fn microtype_kern_font(name: &str, width: i32, kern: i32) -> LoadedFont {
    let mut characters = vec![None; 256];
    characters[usize::from(b'A')] = Some(CharMetrics {
        width: sp(width),
        height: sp(0),
        depth: sp(0),
        italic_correction: sp(0),
        tag: CharTag::LigKern {
            program_index: 0,
            start_index: 0,
        },
    });
    characters[usize::from(b'B')] = Some(CharMetrics {
        width: sp(width),
        height: sp(0),
        depth: sp(0),
        italic_correction: sp(0),
        tag: CharTag::None,
    });
    LoadedFont::new(
        name,
        format!("{name}.tfm"),
        [width as u8; 8],
        0,
        sp(width),
        sp(width),
        vec![sp(0); 7],
        FontMetrics::new(
            characters,
            vec![LigKernInstruction {
                skip_byte: 128,
                next_char: b'B',
                command: Some(LigKernCommand::Kern(sp(kern))),
            }],
            None,
            None,
            Vec::new(),
        ),
    )
}

fn microtype_char(font: tex_state::ids::FontId, ch: char) -> Node {
    Node::Char {
        font,
        ch,
        origin: OriginId::UNKNOWN,
    }
}

fn microtype_ligature(font: tex_state::ids::FontId, ch: char) -> Node {
    Node::Lig {
        font,
        ch,
        orig: vec![ch],
        left_hit: false,
        right_hit: false,
        origins: vec![OriginId::UNKNOWN],
    }
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
                origin: tex_state::node::GlueSpecOrigin::Owned,
                spec: finite,
                kind: GlueKind::Normal,
                leader: None,
            });
        }
    }
    nodes.push(Node::Penalty(INF_PENALTY));
    nodes.push(Node::Glue {
        origin: tex_state::node::GlueSpecOrigin::Owned,
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
