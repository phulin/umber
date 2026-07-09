use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::Order;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{BoxNode, GlueKind, KernKind, Node, Sign};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_typeset::math::{MathParams, Style, mlist_to_hlist};
use tex_typeset::{HpackParams, PackSpec, hpack as hpack_nodes};

use crate::mode::{DisplayEqNo, DisplayInterrupt, EqNoSide};
use crate::vertical::{
    append_node_to_vertical_list, append_vertical_contribution, build_page_if_outer_vertical,
};
use crate::{ExecError, Mode, ModeNest, push_tokens};

use super::lower::lower_math_hlist;
use super::scan::finish_current_math_list;

pub(super) fn start_eq_no(
    nest: &mut ModeNest,
    stores: &mut Universe,
    primitive: UnexpandablePrimitive,
) -> Result<(), ExecError> {
    if nest.current_mode() != Mode::DisplayMath || nest.current_list().display_eq_no().is_some() {
        return Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token: Token::Cs(stores.intern(if primitive == UnexpandablePrimitive::EqNo {
                "eqno"
            } else {
                "leqno"
            })),
            operation: "equation number",
        });
    }
    let display = finish_current_math_list(nest, stores);
    let side = if primitive == UnexpandablePrimitive::LeftEqNo {
        EqNoSide::Left
    } else {
        EqNoSide::Right
    };
    nest.current_list_mut()
        .set_display_eq_no(DisplayEqNo { side, display });
    Ok(())
}

pub(super) fn finish_display_math<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    interrupt: DisplayInterrupt,
    content: tex_state::ids::NodeListId,
    eq_no: Option<DisplayEqNo>,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let (display_content, eq_no_content, left_eq_no) = match eq_no {
        Some(eq_no) => (eq_no.display, Some(content), eq_no.side == EqNoSide::Left),
        None => (content, None, false),
    };
    // AppG rule 22
    let params = MathParams::read(stores);
    let display_hlist = mlist_to_hlist(stores, display_content, Style::DISPLAY, false, &params);
    let display_nodes = lower_math_hlist(stores, &display_hlist);
    let shrink = hlist_shrink(stores, &display_nodes);
    let display_list = stores.freeze_node_list(&display_nodes);
    let mut display_box = hpack_nodes(
        stores,
        display_list,
        PackSpec::Natural,
        HpackParams::read(stores),
    )
    .node;
    display_box.display = true;
    let natural_display_width = display_box.width;

    let mut eq_box = eq_no_content.map(|content| {
        let eq_hlist = mlist_to_hlist(stores, content, Style::TEXT, false, &params);
        let eq_nodes = lower_math_hlist(stores, &eq_hlist);
        let eq_list = stores.freeze_node_list(&eq_nodes);
        let mut node = hpack_nodes(
            stores,
            eq_list,
            PackSpec::Natural,
            HpackParams::read(stores),
        )
        .node;
        node.display = true;
        node
    });

    // TeX.web after_math variables: w=display width, z=line width, s=line indent,
    // e=eqno width, q=eqno width plus math quad, d=center displacement.
    // The display parameters are ordinary scoped assignments while the display
    // is being scanned, so read their finish-time values and use the interrupt
    // record only to restore the enclosing state afterward.
    let z = stores.dimen_param(DimenParam::DISPLAY_WIDTH);
    let s = stores.dimen_param(DimenParam::DISPLAY_INDENT);
    let pre_display_size = stores.dimen_param(DimenParam::PRE_DISPLAY_SIZE);
    let mut w = natural_display_width;
    let mut e = eq_box
        .as_ref()
        .map_or(Scaled::from_raw(0), |boxed| boxed.width);
    let q = if eq_box.is_some() {
        e + params.text.symbols.math_quad
    } else {
        Scaled::from_raw(0)
    };

    if w.raw().saturating_add(q.raw()) > z.raw() {
        if e.raw() != 0 && display_can_shrink_with_eqno(w, q, z, shrink) {
            display_box = hpack_nodes(
                stores,
                display_list,
                PackSpec::Exactly(z - q),
                HpackParams::read(stores),
            )
            .node;
            display_box.display = true;
        } else {
            e = Scaled::from_raw(0);
            if w > z {
                display_box = hpack_nodes(
                    stores,
                    display_list,
                    PackSpec::Exactly(z),
                    HpackParams::read(stores),
                )
                .node;
                display_box.display = true;
            }
        }
        w = display_box.width;
    }

    let mut d = Scaled::from_raw(tex_half(z.raw().saturating_sub(w.raw())));
    if e.raw() > 0 && d.raw() < e.raw().saturating_mul(2) {
        d = Scaled::from_raw(tex_half(
            z.raw().saturating_sub(w.raw()).saturating_sub(e.raw()),
        ));
        if display_nodes
            .first()
            .is_some_and(|node| matches!(node, Node::Glue { .. }))
        {
            d = Scaled::from_raw(0);
        }
    }

    append_vertical_contribution(
        nest,
        stores,
        Node::Penalty(stores.int_param(IntParam::PRE_DISPLAY_PENALTY)),
    );
    let mut above = GlueParam::ABOVE_DISPLAY_SKIP;
    let mut below = Some(GlueParam::BELOW_DISPLAY_SKIP);
    if d.raw().saturating_add(s.raw()) > pre_display_size.raw() && !left_eq_no {
        above = GlueParam::ABOVE_DISPLAY_SHORT_SKIP;
        below = Some(GlueParam::BELOW_DISPLAY_SHORT_SKIP);
    }

    if left_eq_no && e.raw() == 0 {
        if let Some(mut boxed) = eq_box.take() {
            boxed.shift = s;
            append_node_to_vertical_list(nest, stores, Node::HList(boxed))?;
            append_vertical_contribution(nest, stores, Node::Penalty(10_000));
        }
    } else {
        let spec = stores.glue_param(above);
        append_vertical_contribution(
            nest,
            stores,
            Node::Glue {
                spec,
                kind: above_display_glue_kind(above),
                leader: None,
            },
        );
    }

    let mut display_line = display_box;
    if e.raw() != 0
        && let Some(eq_box) = eq_box.take()
    {
        let kern = Node::Kern {
            amount: Scaled::from_raw(
                z.raw()
                    .saturating_sub(w.raw())
                    .saturating_sub(e.raw())
                    .saturating_sub(d.raw()),
            ),
            kind: KernKind::Font,
        };
        let children = if left_eq_no {
            d = Scaled::from_raw(0);
            vec![Node::HList(eq_box), kern, Node::HList(display_line)]
        } else {
            vec![Node::HList(display_line), kern, Node::HList(eq_box)]
        };
        let list = stores.freeze_node_list(&children);
        display_line = hpack_nodes(stores, list, PackSpec::Natural, HpackParams::read(stores)).node;
    }
    display_line.shift = s + d;
    append_node_to_vertical_list(nest, stores, Node::HList(display_line))?;

    if let Some(mut boxed) = eq_box
        && e.raw() == 0
        && !left_eq_no
    {
        append_vertical_contribution(nest, stores, Node::Penalty(10_000));
        boxed.shift = s + z - boxed.width;
        append_node_to_vertical_list(nest, stores, Node::HList(boxed))?;
        below = None;
    }

    append_vertical_contribution(
        nest,
        stores,
        Node::Penalty(stores.int_param(IntParam::POST_DISPLAY_PENALTY)),
    );
    if let Some(below) = below {
        let spec = stores.glue_param(below);
        append_vertical_contribution(
            nest,
            stores,
            Node::Glue {
                spec,
                kind: below_display_glue_kind(below),
                leader: None,
            },
        );
    }

    restore_display_dimensions(stores, interrupt);
    resume_after_display(nest, input, stores)?;
    Ok(())
}

pub(super) fn finish_display_alignment<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    interrupt: DisplayInterrupt,
    nodes: Vec<Node>,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    append_vertical_contribution(
        nest,
        stores,
        Node::Penalty(stores.int_param(IntParam::PRE_DISPLAY_PENALTY)),
    );

    let above = GlueParam::ABOVE_DISPLAY_SKIP;
    let spec = stores.glue_param(above);
    append_vertical_contribution(
        nest,
        stores,
        Node::Glue {
            spec,
            kind: above_display_glue_kind(above),
            leader: None,
        },
    );

    let display_indent = stores.dimen_param(DimenParam::DISPLAY_INDENT);
    for node in nodes {
        let node = display_alignment_node(node, display_indent);
        append_vertical_contribution(nest, stores, node);
    }

    append_vertical_contribution(
        nest,
        stores,
        Node::Penalty(stores.int_param(IntParam::POST_DISPLAY_PENALTY)),
    );
    let spec = stores.glue_param(GlueParam::BELOW_DISPLAY_SKIP);
    append_vertical_contribution(
        nest,
        stores,
        Node::Glue {
            spec,
            kind: GlueKind::BelowDisplaySkip,
            leader: None,
        },
    );

    restore_display_dimensions(stores, interrupt);
    resume_after_display_alignment(nest, input, stores)?;
    Ok(())
}

fn display_alignment_node(mut node: Node, display_indent: Scaled) -> Node {
    if let Node::HList(box_node) | Node::VList(box_node) = &mut node {
        box_node.display = true;
        box_node.shift = display_indent;
    }
    node
}

fn resume_after_display_alignment<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let prev_graf = nest.enclosing_vertical_prev_graf().saturating_add(3);
    nest.set_enclosing_vertical_prev_graf(prev_graf);
    let par_shape = nest.current_list().par_shape().cloned();
    match input.next_token(stores)? {
        Some(Token::Char {
            cat: Catcode::Space,
            ..
        }) => {}
        Some(token) if is_par_or_end_group(stores, token) => push_tokens(input, stores, [token]),
        Some(token) => {
            nest.push(Mode::Horizontal);
            if let Some(shape) = par_shape {
                nest.current_list_mut().set_par_shape(shape);
            }
            nest.current_list_mut().set_space_factor(1000);
            push_tokens(input, stores, [token]);
        }
        None => {}
    }
    build_page_if_outer_vertical(nest, stores)?;
    Ok(())
}

fn is_par_or_end_group(stores: &Universe, token: Token) -> bool {
    if matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    ) {
        return true;
    }
    let Token::Cs(symbol) = token else {
        return false;
    };
    matches!(
        stores.meaning(symbol),
        tex_state::meaning::Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Par | UnexpandablePrimitive::EndGraf
        )
    )
}

fn above_display_glue_kind(param: GlueParam) -> GlueKind {
    if param == GlueParam::ABOVE_DISPLAY_SHORT_SKIP {
        GlueKind::AboveDisplayShortSkip
    } else {
        GlueKind::AboveDisplaySkip
    }
}

fn below_display_glue_kind(param: GlueParam) -> GlueKind {
    if param == GlueParam::BELOW_DISPLAY_SHORT_SKIP {
        GlueKind::BelowDisplayShortSkip
    } else {
        GlueKind::BelowDisplaySkip
    }
}

const fn tex_half(x: i32) -> i32 {
    if x % 2 != 0 && x > 0 {
        x / 2 + 1
    } else {
        x / 2
    }
}

fn resume_after_display<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let prev_graf = nest.enclosing_vertical_prev_graf().saturating_add(3);
    nest.set_enclosing_vertical_prev_graf(prev_graf);
    let par_shape = nest.current_list().par_shape().cloned();
    nest.push(Mode::Horizontal);
    if let Some(shape) = par_shape {
        nest.current_list_mut().set_par_shape(shape);
    }
    nest.current_list_mut().set_space_factor(1000);
    match input.next_token(stores)? {
        Some(Token::Char {
            cat: Catcode::Space,
            ..
        }) => {}
        Some(token) => push_tokens(input, stores, [token]),
        None => {}
    }
    build_page_if_outer_vertical(nest, stores)?;
    Ok(())
}

fn restore_display_dimensions(stores: &mut Universe, interrupt: DisplayInterrupt) {
    stores.set_dimen_param(
        DimenParam::PRE_DISPLAY_SIZE,
        interrupt.saved_pre_display_size,
    );
    stores.set_dimen_param(DimenParam::DISPLAY_WIDTH, interrupt.saved_display_width);
    stores.set_dimen_param(DimenParam::DISPLAY_INDENT, interrupt.saved_display_indent);
}

fn display_can_shrink_with_eqno(w: Scaled, q: Scaled, z: Scaled, shrink: ShrinkTotals) -> bool {
    w.raw()
        .saturating_sub(shrink.normal.raw())
        .saturating_add(q.raw())
        <= z.raw()
        || shrink.fil.raw() != 0
        || shrink.fill.raw() != 0
        || shrink.filll.raw() != 0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShrinkTotals {
    normal: Scaled,
    fil: Scaled,
    fill: Scaled,
    filll: Scaled,
}

fn hlist_shrink(stores: &Universe, nodes: &[Node]) -> ShrinkTotals {
    let mut totals = [Scaled::from_raw(0); 4];
    for node in nodes {
        if let Node::Glue { spec, .. } = node {
            let glue = stores.glue(*spec);
            totals[glue.shrink_order as usize] = totals[glue.shrink_order as usize] + glue.shrink;
        }
    }
    ShrinkTotals {
        normal: totals[Order::Normal as usize],
        fil: totals[Order::Fil as usize],
        fill: totals[Order::Fill as usize],
        filll: totals[Order::Filll as usize],
    }
}

pub(super) fn pre_display_size(stores: &Universe, line: &BoxNode) -> Scaled {
    let quad = stores.font_parameter(stores.current_font(), 6);
    let mut v = line.shift + quad + quad;
    let mut w = Scaled::from_raw(-Scaled::MAX_DIMEN.raw());
    for node in stores.nodes(line.children) {
        let (d, visible, glue_depends_on_set) = pre_display_node_width(stores, line, node);
        if glue_depends_on_set {
            v = Scaled::MAX_DIMEN;
        }
        if v < Scaled::MAX_DIMEN {
            v = v + d;
        }
        if visible {
            if v < Scaled::MAX_DIMEN {
                w = v;
            } else {
                return Scaled::MAX_DIMEN;
            }
        }
    }
    w
}

fn pre_display_node_width(stores: &Universe, line: &BoxNode, node: &Node) -> (Scaled, bool, bool) {
    match node {
        Node::Char { font, ch } | Node::Lig { font, ch, .. } => {
            let width = u8::try_from(*ch as u32)
                .ok()
                .and_then(|code| stores.font_char_metrics(*font, code))
                .map_or(Scaled::from_raw(0), |metrics| metrics.width);
            (width, true, false)
        }
        Node::HList(boxed) | Node::VList(boxed) => (boxed.width, true, false),
        Node::Rule { width, .. } => (width.unwrap_or(Scaled::from_raw(0)), true, false),
        Node::Kern { amount, .. } | Node::MathOn(amount) | Node::MathOff(amount) => {
            (*amount, false, false)
        }
        Node::Glue { spec, .. } => {
            let glue = stores.glue(*spec);
            let depends = match line.glue_sign {
                Sign::Stretching => {
                    line.glue_order == glue.stretch_order && glue.stretch.raw() != 0
                }
                Sign::Shrinking => line.glue_order == glue.shrink_order && glue.shrink.raw() != 0,
                Sign::Normal => false,
            };
            (glue.width, false, depends)
        }
        _ => (Scaled::from_raw(0), false, false),
    }
}
