use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::Order;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{BoxNode, GlueKind, KernKind, Node, Sign};
use tex_state::scaled::{
    Scaled, saturating_add as scaled_add, saturating_mul as scaled_mul,
    saturating_sub as scaled_sub,
};
use tex_state::token::{Catcode, OriginId, Token};
use tex_typeset::PackSpec;
use tex_typeset::math::{MathParams, Style};

use crate::assignments::hpack_with_overfull_rule;
use crate::mode::{DisplayEqNo, EqNoSide};
use crate::packing_params::{hpack as hpack_nodes, hpack_params};
use crate::vertical::{
    append_node_to_vertical_list, append_vertical_contribution, build_page_if_outer_vertical,
};
use crate::{ExecError, Mode, ModeNest};

use super::lower::convert_math_hlist;
use super::scan::finish_current_math_list;

pub(super) fn start_eq_no(
    nest: &mut ModeNest,
    stores: &mut Universe,
    primitive: UnexpandablePrimitive,
) -> Result<(), ExecError> {
    if nest.current_mode() != Mode::DisplayMath {
        return Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token: Token::Cs(
                stores
                    .intern(if primitive == UnexpandablePrimitive::EqNo {
                        "eqno"
                    } else {
                        "leqno"
                    })
                    .symbol(),
            ),
            origin: OriginId::UNKNOWN,
            operation: "equation number",
        });
    }
    let display = finish_current_math_list(nest, stores);
    let side = if primitive == UnexpandablePrimitive::LeftEqNo {
        EqNoSide::Left
    } else {
        EqNoSide::Right
    };
    stores.enter_group_with_kind(tex_state::GroupKind::MathShift);
    stores.set_int_param(IntParam::FAM, -1);
    // TeX.web enters negative math mode for the equation number. Keeping this
    // as a real mode level is important: the first `$` closes the equation
    // number and the following `$` closes the enclosing display, and both the
    // mode nest and save stack must remain snapshot-coverable between them.
    nest.push(Mode::Math);
    nest.current_list_mut()
        .set_display_eq_no(DisplayEqNo { side, display });
    Ok(())
}

pub(super) struct FinishedEqNo {
    pub side: EqNoSide,
    pub boxed: BoxNode,
}

pub(super) fn finish_eq_no(
    stores: &mut Universe,
    side: EqNoSide,
    content: tex_state::ids::NodeListId,
) -> FinishedEqNo {
    let params = MathParams::read(stores);
    let nodes = convert_math_hlist(stores, content, Style::TEXT, false, &params);
    let list = stores.freeze_node_list(&nodes);
    let mut boxed = hpack_nodes(stores, list, PackSpec::Natural, hpack_params(stores)).node;
    boxed.display = true;
    FinishedEqNo { side, boxed }
}

pub(super) fn finish_display_math(
    nest: &mut ModeNest,
    stores: &mut Universe,
    content: tex_state::ids::NodeListId,
    eq_no: Option<FinishedEqNo>,
) -> Result<(), ExecError> {
    let (display_content, mut eq_box, left_eq_no) = match eq_no {
        Some(eq_no) => (content, Some(eq_no.boxed), eq_no.side == EqNoSide::Left),
        None => (content, None, false),
    };
    // AppG rule 22
    let params = MathParams::read(stores);
    let display_nodes = convert_math_hlist(stores, display_content, Style::DISPLAY, false, &params);
    let shrink = hlist_shrink(stores, &display_nodes);
    let display_list = stores.freeze_node_list(&display_nodes);
    let mut display_box = hpack_nodes(
        stores,
        display_list,
        PackSpec::Natural,
        hpack_params(stores),
    )
    .node;
    display_box.display = true;
    let natural_display_width = display_box.width;

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

    if scaled_add(w, q) > z {
        if e.raw() != 0 && display_can_shrink_with_eqno(w, q, z, shrink) {
            display_box = hpack_nodes(
                stores,
                display_list,
                PackSpec::Exactly(z - q),
                hpack_params(stores),
            )
            .node;
            display_box.display = true;
        } else {
            e = Scaled::from_raw(0);
            if w > z {
                display_box = hpack_with_overfull_rule(stores, display_list, PackSpec::Exactly(z));
                display_box.display = true;
            }
        }
        w = display_box.width;
    }

    let mut d = Scaled::from_raw(tex_half(scaled_sub(z, w).raw()));
    if e.raw() > 0 && d < scaled_mul(2, e) {
        d = Scaled::from_raw(tex_half(scaled_sub(scaled_sub(z, w), e).raw()));
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
    if scaled_add(d, s) > pre_display_size && !left_eq_no {
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
            amount: scaled_sub(scaled_sub(scaled_sub(z, w), e), d),
            kind: KernKind::Font,
        };
        let children = if left_eq_no {
            d = Scaled::from_raw(0);
            vec![Node::HList(eq_box), kern, Node::HList(display_line)]
        } else {
            vec![Node::HList(display_line), kern, Node::HList(eq_box)]
        };
        let list = stores.freeze_node_list(&children);
        display_line = hpack_nodes(stores, list, PackSpec::Natural, hpack_params(stores)).node;
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

    Ok(())
}

pub(super) fn finish_display_alignment(
    nest: &mut ModeNest,
    stores: &mut Universe,
    nodes: Vec<Node>,
) -> Result<(), ExecError> {
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

    Ok(())
}

fn display_alignment_node(mut node: Node, display_indent: Scaled) -> Node {
    if let Node::HList(box_node) | Node::VList(box_node) = &mut node {
        box_node.display = true;
        box_node.shift = display_indent;
    }
    node
}

pub(super) fn resume_after_display_alignment(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    active_directions: Vec<tex_state::node::Direction>,
) -> Result<(), ExecError> {
    let prev_graf = nest.enclosing_vertical_prev_graf().saturating_add(3);
    nest.set_enclosing_vertical_prev_graf(prev_graf);
    let next = loop {
        match input.next_traced_token(stores)? {
            Some(traced)
                if matches!(
                    tex_expand::semantic_token(traced),
                    Token::Char {
                        cat: Catcode::Space,
                        ..
                    }
                ) => {}
            other => break other,
        }
    };
    match next {
        Some(traced) if is_par_or_end_group(stores, tex_expand::semantic_token(traced)) => {
            crate::insert_traced_tokens(input, stores, [traced]);
        }
        Some(traced) => {
            nest.push(Mode::Horizontal);
            nest.current_list_mut().set_space_factor(1000);
            nest.current_list_mut()
                .append(active_directions.iter().copied().map(Node::Direction));
            crate::insert_traced_tokens(input, stores, [traced]);
        }
        None => {}
    }
    build_page_after_display_resume(nest, stores)?;
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

pub(super) fn resume_after_display(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    active_directions: Vec<tex_state::node::Direction>,
) -> Result<(), ExecError> {
    let prev_graf = nest.enclosing_vertical_prev_graf().saturating_add(3);
    nest.set_enclosing_vertical_prev_graf(prev_graf);
    nest.push(Mode::Horizontal);
    nest.current_list_mut().set_space_factor(1000);
    nest.current_list_mut()
        .append(active_directions.iter().copied().map(Node::Direction));
    match input.next_traced_token(stores)? {
        Some(traced)
            if matches!(
                tex_expand::semantic_token(traced),
                Token::Char {
                    cat: Catcode::Space,
                    ..
                }
            ) => {}
        Some(traced) => crate::insert_traced_tokens(input, stores, [traced]),
        None => {}
    }
    build_page_after_display_resume(nest, stores)?;
    Ok(())
}

fn build_page_after_display_resume(
    nest: &ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    // TeX.web resumes a display by pushing horizontal mode and then calls
    // build_page when that new level sits directly above outer vertical mode.
    // Looking only at the current mode would miss the just-appended display
    // penalties and defer a forced break until unrelated later material.
    if nest.depth() == 2 && nest.current_mode() == Mode::Horizontal {
        crate::page_builder::build_page(stores)
    } else {
        build_page_if_outer_vertical(nest, stores)
    }
}

fn display_can_shrink_with_eqno(w: Scaled, q: Scaled, z: Scaled, shrink: ShrinkTotals) -> bool {
    scaled_add(scaled_sub(w, shrink.normal), q) <= z
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

fn pre_display_node_width(
    stores: &Universe,
    line: &BoxNode,
    node: tex_state::node_arena::NodeRef<'_>,
) -> (Scaled, bool, bool) {
    match node {
        tex_state::node_arena::NodeRef::Char { font, ch, .. }
        | tex_state::node_arena::NodeRef::Lig { font, ch, .. } => {
            let width = u8::try_from(ch as u32)
                .ok()
                .and_then(|code| stores.font_char_metrics(font, code))
                .map_or(Scaled::from_raw(0), |metrics| metrics.width);
            (width, true, false)
        }
        tex_state::node_arena::NodeRef::HList(boxed)
        | tex_state::node_arena::NodeRef::VList(boxed) => (boxed.width, true, false),
        tex_state::node_arena::NodeRef::Rule { width, .. } => {
            (width.unwrap_or(Scaled::from_raw(0)), true, false)
        }
        tex_state::node_arena::NodeRef::Kern { amount, .. }
        | tex_state::node_arena::NodeRef::MathOn(amount)
        | tex_state::node_arena::NodeRef::MathOff(amount) => (amount, false, false),
        tex_state::node_arena::NodeRef::Glue { spec, .. } => {
            let glue = stores.glue(spec);
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
