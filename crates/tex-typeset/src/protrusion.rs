//! Pure pdfTeX character-protrusion edge discovery and line materialization.

use tex_state::font::PdfFontCode;
use tex_state::node::{GlueKind, KernKind, MarginKernSide, Node};
use tex_state::node_arena::NodeCursor;
use tex_state::scaled::Scaled;

use crate::TypesetState;

/// Signed protrusion available at the two edges of a candidate line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineProtrusion {
    pub left: Scaled,
    pub right: Scaled,
    margin_stretch: Scaled,
    margin_shrink: Scaled,
}

impl LineProtrusion {
    /// The signed amount added to pdfTeX's line-breaking shortfall.
    #[must_use]
    pub fn total(self) -> Scaled {
        self.left
            .checked_add(self.right)
            .expect("the two glyph-edge protrusions fit Scaled")
    }

    /// pdfTeX's expansion-capacity variation for the two marginal kerns.
    #[must_use]
    pub(crate) const fn margin_variation(self) -> (Scaled, Scaled) {
        (self.margin_stretch, self.margin_shrink)
    }
}

/// Finds the candidate line's protruding edge characters.
#[must_use]
pub fn line_protrusion(state: &impl TypesetState, nodes: &[Node]) -> LineProtrusion {
    line_protrusion_cursor(state, NodeCursor::owned(nodes), 0, nodes.len())
}

#[must_use]
pub(crate) fn line_protrusion_cursor(
    state: &impl TypesetState,
    nodes: NodeCursor<'_>,
    start: usize,
    end: usize,
) -> LineProtrusion {
    let left = edge_glyph_cursor(state, nodes, start, end, Edge::Left);
    let right = edge_glyph_cursor(state, nodes, start, end, Edge::Right);
    let zero = Scaled::from_raw(0);
    let left_variation = left.map_or((zero, zero), |glyph| margin_kern_variation(state, glyph));
    let right_variation = right.map_or((zero, zero), |glyph| margin_kern_variation(state, glyph));
    LineProtrusion {
        left: left.map_or(zero, |glyph| glyph_width(state, glyph, Edge::Left)),
        right: right.map_or(zero, |glyph| glyph_width(state, glyph, Edge::Right)),
        margin_stretch: left_variation
            .0
            .checked_add(right_variation.0)
            .expect("the two left-protrusion stretch variations fit Scaled"),
        margin_shrink: left_variation
            .1
            .checked_add(right_variation.1)
            .expect("the two left-protrusion shrink variations fit Scaled"),
    }
}

/// pdftex.web §822 deliberately uses `left_pw` for both edge characters when
/// calculating the marginal-kern variation. For a ligature, `char_pw` records
/// the embedded `lig_char` word. Re-entering `char_pw` rejects that low-memory
/// word, while the scratch character built from it is a regular character and
/// retains its left-protrusion code. Ordinary character rows cancel exactly.
fn margin_kern_variation(state: &impl TypesetState, glyph: Glyph) -> (Scaled, Scaled) {
    if !glyph.is_ligature {
        return (Scaled::from_raw(0), Scaled::from_raw(0));
    }
    let Some(spec) = state.font_expansion_spec(glyph.font) else {
        return (Scaled::from_raw(0), Scaled::from_raw(0));
    };
    let efcode = state.pdf_font_code(PdfFontCode::Ef, glyph.font, glyph.code);
    let amount = glyph_width(state, glyph, Edge::Left);
    let stretch = if spec.discrete_ratio(1000, efcode) == 0 {
        Scaled::from_raw(0)
    } else {
        amount
            .checked_neg()
            .expect("a legal left protrusion can be negated")
    };
    let shrink = if spec.discrete_ratio(-1000, efcode) == 0 {
        Scaled::from_raw(0)
    } else {
        amount
    };
    (stretch, shrink)
}

/// Inserts pdfTeX's final signed margin-kern nodes around line material.
///
/// The input is the post-line-break list, so named left/right skip glue is
/// already present. Margin kerns sit inside those skips, exactly as in
/// pdfTeX's `post_line_break`. `material_start` is the first node after the
/// synthetic finalized left boundary.
pub fn insert_margin_kerns(
    state: &impl TypesetState,
    nodes: &mut Vec<Node>,
    material_start: usize,
) {
    // pdftex.web §1061 discovers the right marginal character before it
    // inserts `\rightskip`. The finalized-list adapters already carry that
    // boundary (and terminal `\parfillskip`), so neither may block the scan.
    let right_boundary = right_margin_position(nodes);
    let right = edge_glyph_cursor(
        state,
        NodeCursor::owned(nodes),
        0,
        right_boundary,
        Edge::Right,
    );
    if let Some(glyph) = right.filter(|glyph| glyph_width(state, *glyph, Edge::Right).raw() != 0) {
        let amount = glyph_width(state, glyph, Edge::Right);
        nodes.insert(
            right_boundary,
            Node::MarginKern {
                amount: amount
                    .checked_neg()
                    .expect("a legal protrusion can be negated"),
                side: MarginKernSide::Right,
                font: glyph.font,
                ch: glyph.code,
            },
        );
    }
    let left = edge_glyph_cursor(
        state,
        NodeCursor::owned(nodes),
        material_start,
        nodes.len(),
        Edge::Left,
    );
    if let Some(glyph) = left.filter(|glyph| glyph_width(state, *glyph, Edge::Left).raw() != 0) {
        let amount = glyph_width(state, glyph, Edge::Left);
        let at =
            edge_position(state, nodes, Edge::Left).unwrap_or_else(|| leading_left_skip_end(nodes));
        nodes.insert(
            at,
            Node::MarginKern {
                amount: amount
                    .checked_neg()
                    .expect("a legal protrusion can be negated"),
                side: MarginKernSide::Left,
                font: glyph.font,
                ch: glyph.code,
            },
        );
    }
}

/// Coordinate-only insertion plan for an arena-backed finalized line.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MarginKernPlan {
    pub left: Option<(usize, Node)>,
    pub right: Option<(usize, Node)>,
}

/// Plans margin-kern insertion without materializing the source list.
///
/// `material_start` is the first node after the synthetic finalized left
/// boundary; named glue at or after that coordinate remains semantic material.
#[must_use]
pub fn plan_margin_kerns(
    state: &impl TypesetState,
    nodes: NodeCursor<'_>,
    material_start: usize,
) -> MarginKernPlan {
    // pdftex.web §1061 performs right-edge discovery before adding
    // `\rightskip`. Use the exact finalized suffix boundary for the same
    // transition; terminal `\parfillskip` is outside the searched material.
    let right_boundary = right_margin_position_cursor(nodes);
    let right = edge_glyph_cursor(state, nodes, 0, right_boundary, Edge::Right);
    let right = right
        .filter(|glyph| glyph_width(state, *glyph, Edge::Right).raw() != 0)
        .map(|glyph| {
            let amount = glyph_width(state, glyph, Edge::Right);
            (
                right_boundary,
                Node::MarginKern {
                    amount: amount
                        .checked_neg()
                        .expect("a legal protrusion can be negated"),
                    side: MarginKernSide::Right,
                    font: glyph.font,
                    ch: glyph.code,
                },
            )
        });
    // pdftex.web §1061 discovers the left marginal character before
    // prepending `\leftskip`. This planner receives an already finalized
    // line, so begin at the caller's exact material boundary rather than
    // filtering named glue that may also occur inside the material.
    let left = edge_glyph_cursor(state, nodes, material_start, nodes.len(), Edge::Left);
    let left = left
        .filter(|glyph| glyph_width(state, *glyph, Edge::Left).raw() != 0)
        .map(|glyph| {
            let amount = glyph_width(state, glyph, Edge::Left);
            let at = edge_position_cursor(state, nodes, Edge::Left)
                .unwrap_or_else(|| leading_left_skip_end_cursor(nodes));
            (
                at,
                Node::MarginKern {
                    amount: amount
                        .checked_neg()
                        .expect("a legal protrusion can be negated"),
                    side: MarginKernSide::Left,
                    font: glyph.font,
                    ch: glyph.code,
                },
            )
        });
    MarginKernPlan { left, right }
}

fn edge_position_cursor(
    state: &impl TypesetState,
    nodes: NodeCursor<'_>,
    edge: Edge,
) -> Option<usize> {
    match edge {
        Edge::Left => (0..nodes.len())
            .find_map(
                |index| match search_node(state, nodes.owned_node(index)?, edge) {
                    Search::Glyph(_) => Some(Some(index)),
                    Search::Skip => None,
                    Search::Block => Some(None),
                },
            )
            .flatten(),
        Edge::Right => (0..nodes.len())
            .rev()
            .find_map(
                |index| match search_node(state, nodes.owned_node(index)?, edge) {
                    Search::Glyph(_) => Some(Some(index)),
                    Search::Skip => None,
                    Search::Block => Some(None),
                },
            )
            .flatten(),
    }
}

fn leading_left_skip_end_cursor(nodes: NodeCursor<'_>) -> usize {
    (0..nodes.len())
        .take_while(|index| {
            matches!(
                nodes.owned_node(*index),
                Some(
                    Node::Glue {
                        kind: GlueKind::LeftSkip,
                        ..
                    } | Node::Direction(_)
                )
            )
        })
        .count()
}

fn right_margin_position_cursor(nodes: NodeCursor<'_>) -> usize {
    let Some(mut index) = (0..nodes.len()).rev().find(|index| {
        matches!(
            nodes.owned_node(*index),
            Some(
                Node::Glue {
                    kind: GlueKind::ParFillSkip | GlueKind::RightSkip,
                    ..
                } | Node::Direction(_)
            )
        )
    }) else {
        return nodes.len();
    };
    while index > 0
        && matches!(
            nodes.owned_node(index - 1),
            Some(
                Node::Glue {
                    kind: GlueKind::ParFillSkip | GlueKind::RightSkip,
                    ..
                } | Node::Direction(_)
            )
        )
    {
        index -= 1;
    }
    index
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
    is_ligature: bool,
}

enum Search {
    Glyph(Glyph),
    Skip,
    Block,
}

/// Whether a zero-valued glue node retains TeX's shared `zero_glue` pointer.
///
/// Parameter glue is canonicalized by TeX82 §1237 before these named nodes
/// are made, and `\nonscript` directly uses `zero_glue`. Scanned glue instead
/// owns a fresh specification even when its value is zero; leader and
/// explicit math-skip nodes retain that scanned identity in their kinds.
const fn shares_zero_glue(kind: GlueKind) -> bool {
    match kind {
        GlueKind::Normal
        | GlueKind::Leaders
        | GlueKind::Cleaders
        | GlueKind::Xleaders
        | GlueKind::MuSkip => false,
        GlueKind::SpaceSkip
        | GlueKind::XSpaceSkip
        | GlueKind::TabSkip
        | GlueKind::BaselineSkip
        | GlueKind::LineSkip
        | GlueKind::TopSkip
        | GlueKind::SplitTopSkip
        | GlueKind::LeftSkip
        | GlueKind::RightSkip
        | GlueKind::ParSkip
        | GlueKind::ParFillSkip
        | GlueKind::AboveDisplaySkip
        | GlueKind::BelowDisplaySkip
        | GlueKind::AboveDisplayShortSkip
        | GlueKind::BelowDisplayShortSkip
        | GlueKind::ThinMuSkip
        | GlueKind::MedMuSkip
        | GlueKind::ThickMuSkip
        | GlueKind::NonScript => true,
    }
}

fn edge_glyph_cursor(
    state: &impl TypesetState,
    nodes: NodeCursor<'_>,
    start: usize,
    end: usize,
    edge: Edge,
) -> Option<Glyph> {
    match edge_search_cursor(state, nodes, start, end, edge) {
        Search::Glyph(glyph) => Some(glyph),
        Search::Skip | Search::Block => None,
    }
}

fn edge_search_cursor(
    state: &impl TypesetState,
    nodes: NodeCursor<'_>,
    start: usize,
    end: usize,
    edge: Edge,
) -> Search {
    let start = start.min(nodes.len());
    let end = end.min(nodes.len()).max(start);
    match edge {
        Edge::Left => {
            for index in start..end {
                let node = nodes
                    .owned_node(index)
                    .expect("edge index belongs to source");
                match search_node(state, node, edge) {
                    Search::Glyph(glyph) => return Search::Glyph(glyph),
                    Search::Skip => {}
                    Search::Block => return Search::Block,
                }
            }
        }
        Edge::Right => {
            for index in (start..end).rev() {
                let node = nodes
                    .owned_node(index)
                    .expect("edge index belongs to source");
                match search_node(state, node, edge) {
                    Search::Glyph(glyph) => return Search::Glyph(glyph),
                    Search::Skip => {}
                    Search::Block => return Search::Block,
                }
            }
        }
    }
    Search::Skip
}

fn search_node(state: &impl TypesetState, node: &Node, edge: Edge) -> Search {
    match node {
        Node::Char { font, ch, .. } => u8::try_from(*ch as u32).map_or(Search::Block, |code| {
            Search::Glyph(Glyph {
                font: *font,
                code,
                is_ligature: false,
            })
        }),
        Node::Lig { font, ch, .. } => u8::try_from(*ch as u32).map_or(Search::Block, |code| {
            Search::Glyph(Glyph {
                font: *font,
                code,
                is_ligature: true,
            })
        }),
        Node::HList(box_node) => {
            let children = state.page_nodes(box_node.children);
            if children.is_empty() {
                if box_node.width.raw() == 0
                    && box_node.height.raw() == 0
                    && box_node.depth.raw() == 0
                {
                    Search::Skip
                } else {
                    Search::Block
                }
            } else {
                // pdftex.web §1003 descends into every nonempty hlist. A
                // blocking child must remain blocking when the search returns
                // to the parent; only exhausting a list is equivalent to
                // skipping it.
                edge_search_cursor(state, children, 0, children.len(), edge)
            }
        }
        Node::Disc {
            pre, post, replace, ..
        } => {
            let list = match edge {
                Edge::Left if !post.is_empty() => post,
                Edge::Right if !pre.is_empty() => pre,
                _ => replace,
            };
            let children = state.page_nodes(*list);
            edge_glyph_cursor(state, children, 0, children.len(), edge)
                .map_or(Search::Skip, Search::Glyph)
        }
        Node::Kern { amount, kind }
            if amount.raw() == 0 || matches!(kind, KernKind::Font | KernKind::Auto) =>
        {
            Search::Skip
        }
        Node::MathOn(amount) | Node::MathOff(amount) if amount.raw() == 0 => Search::Skip,
        // pdftex.web §1003 recognizes pointer identity with the shared
        // `zero_glue`, not merely an equal-valued specification. Parameter
        // and `\nonscript` kinds retain that identity when their value is
        // zero; scanned explicit glue and leader/muskip specifications are
        // fresh nodes and remain blocking even when all dimensions are zero.
        Node::Glue { spec, kind, .. }
            if *spec == tex_state::glue::GlueSpec::ZERO && shares_zero_glue(*kind) =>
        {
            Search::Skip
        }
        Node::Penalty(_)
        | Node::MarginKern { .. }
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
