//! Pure pdfTeX character-protrusion edge discovery and line materialization.

use tex_state::font::PdfFontCode;
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;

use crate::TypesetState;

/// Signed protrusion available at the two edges of a candidate line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineProtrusion {
    pub left: Scaled,
    pub right: Scaled,
}

impl LineProtrusion {
    /// The signed amount added to pdfTeX's line-breaking shortfall.
    #[must_use]
    pub fn total(self) -> Scaled {
        self.left
            .checked_add(self.right)
            .expect("the two glyph-edge protrusions fit Scaled")
    }
}

/// Finds the candidate line's protruding edge characters.
#[must_use]
pub fn line_protrusion(state: &impl TypesetState, nodes: &[Node]) -> LineProtrusion {
    LineProtrusion {
        left: edge_glyph(state, nodes, Edge::Left).map_or(Scaled::from_raw(0), |glyph| {
            glyph_width(state, glyph, Edge::Left)
        }),
        right: edge_glyph(state, nodes, Edge::Right).map_or(Scaled::from_raw(0), |glyph| {
            glyph_width(state, glyph, Edge::Right)
        }),
    }
}

/// Inserts pdfTeX's final signed margin-kern nodes around line material.
///
/// The input is the post-line-break list, so named left/right skip glue is
/// already present. Margin kerns sit inside those skips, exactly as in
/// pdfTeX's `post_line_break`.
pub fn insert_margin_kerns(state: &impl TypesetState, nodes: &mut Vec<Node>) {
    let protrusion = line_protrusion(state, nodes);
    if protrusion.right.raw() != 0 {
        let at = right_margin_position(nodes);
        nodes.insert(
            at,
            Node::Kern {
                amount: protrusion
                    .right
                    .checked_neg()
                    .expect("a legal protrusion can be negated"),
                kind: KernKind::RightMargin,
            },
        );
    }
    if protrusion.left.raw() != 0 {
        let at =
            edge_position(state, nodes, Edge::Left).unwrap_or_else(|| leading_left_skip_end(nodes));
        nodes.insert(
            at,
            Node::Kern {
                amount: protrusion
                    .left
                    .checked_neg()
                    .expect("a legal protrusion can be negated"),
                kind: KernKind::LeftMargin,
            },
        );
    }
}

fn edge_position(state: &impl TypesetState, nodes: &[Node], edge: Edge) -> Option<usize> {
    match edge {
        Edge::Left => nodes
            .iter()
            .enumerate()
            .find_map(|(index, node)| match search_node(state, node, edge) {
                Search::Glyph(_) => Some(Some(index)),
                Search::Skip => None,
                Search::Block => Some(None),
            })
            .flatten(),
        Edge::Right => nodes
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, node)| match search_node(state, node, edge) {
                Search::Glyph(_) => Some(Some(index)),
                Search::Skip => None,
                Search::Block => Some(None),
            })
            .flatten(),
    }
}

fn leading_left_skip_end(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .take_while(|node| {
            matches!(
                node,
                Node::Glue {
                    kind: GlueKind::LeftSkip,
                    ..
                } | Node::Direction(_)
            )
        })
        .count()
}

fn right_margin_position(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .rposition(|node| {
            matches!(
                node,
                Node::Glue {
                    kind: GlueKind::ParFillSkip | GlueKind::RightSkip,
                    ..
                } | Node::Direction(_)
            )
        })
        .map_or(nodes.len(), |mut index| {
            while index > 0
                && matches!(
                    nodes[index - 1],
                    Node::Glue {
                        kind: GlueKind::ParFillSkip | GlueKind::RightSkip,
                        ..
                    } | Node::Direction(_)
                )
            {
                index -= 1;
            }
            index
        })
}

#[derive(Clone, Copy)]
enum Edge {
    Left,
    Right,
}

#[derive(Clone, Copy)]
struct Glyph {
    font: tex_state::ids::FontId,
    code: u8,
}

enum Search {
    Glyph(Glyph),
    Skip,
    Block,
}

fn edge_glyph(state: &impl TypesetState, nodes: &[Node], edge: Edge) -> Option<Glyph> {
    match edge {
        Edge::Left => {
            for node in nodes {
                match search_node(state, node, edge) {
                    Search::Glyph(glyph) => return Some(glyph),
                    Search::Skip => {}
                    Search::Block => return None,
                }
            }
        }
        Edge::Right => {
            for node in nodes.iter().rev() {
                match search_node(state, node, edge) {
                    Search::Glyph(glyph) => return Some(glyph),
                    Search::Skip => {}
                    Search::Block => return None,
                }
            }
        }
    }
    None
}

fn search_node(state: &impl TypesetState, node: &Node, edge: Edge) -> Search {
    match node {
        Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => u8::try_from(*ch as u32)
            .map_or(Search::Block, |code| {
                Search::Glyph(Glyph { font: *font, code })
            }),
        Node::HList(box_node) => {
            let children = state.nodes(box_node.children).to_vec();
            edge_glyph(state, &children, edge).map_or_else(
                || {
                    if box_node.width.raw() == 0 {
                        Search::Skip
                    } else {
                        Search::Block
                    }
                },
                Search::Glyph,
            )
        }
        Node::Disc {
            pre, post, replace, ..
        } => {
            let list = match edge {
                Edge::Left if !state.nodes(*post).is_empty() => *post,
                Edge::Right if !state.nodes(*pre).is_empty() => *pre,
                _ => *replace,
            };
            let children = state.nodes(list).to_vec();
            edge_glyph(state, &children, edge).map_or(Search::Skip, Search::Glyph)
        }
        Node::Kern { amount, .. } | Node::MathOn(amount) | Node::MathOff(amount)
            if amount.raw() == 0 =>
        {
            Search::Skip
        }
        Node::Glue { spec, .. } if state.glue(*spec).width.raw() == 0 => Search::Skip,
        Node::Penalty(_)
        | Node::Mark { .. }
        | Node::Ins { .. }
        | Node::Whatsit(_)
        | Node::Direction(_)
        | Node::Adjust(_)
        | Node::Nonscript => Search::Skip,
        Node::VList(_)
        | Node::Unset(_)
        | Node::Rule { .. }
        | Node::Kern { .. }
        | Node::Glue { .. }
        | Node::MathOn(_)
        | Node::MathOff(_)
        | Node::MathNoad(_)
        | Node::FractionNoad(_)
        | Node::MathStyle(_)
        | Node::MathChoice(_)
        | Node::MathList(_) => Search::Block,
    }
}

fn glyph_width(state: &impl TypesetState, glyph: Glyph, edge: Edge) -> Scaled {
    let table = match edge {
        Edge::Left => PdfFontCode::Lp,
        Edge::Right => PdfFontCode::Rp,
    };
    let code = state.pdf_font_code(table, glyph.font, glyph.code);
    round_scaled_ratio(state.font_parameter_value(glyph.font, 6), code, 1000)
}

fn round_scaled_ratio(value: Scaled, numerator: i32, denominator: i32) -> Scaled {
    let product = i64::from(value.raw()) * i64::from(numerator);
    let denominator = i64::from(denominator);
    let rounded = if product >= 0 {
        (product + denominator / 2) / denominator
    } else {
        -((-product + denominator / 2) / denominator)
    };
    Scaled::from_raw(i32::try_from(rounded).unwrap_or(if rounded < 0 {
        i32::MIN
    } else {
        i32::MAX
    }))
}

#[cfg(test)]
mod tests;
