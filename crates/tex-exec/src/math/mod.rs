//! Math-mode stomach front-end.

use tex_expand::{ExpansionHooks, ReadRecorder};
use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::math::{MathChar, MathField, MathListNode, NoadClass, NoadKind};
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, KernKind, Node, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::{Catcode, Token};
use tex_typeset::math::{
    FrozenHList, MathBox, MathGlueKind, MathNode, MathParams, Style, mlist_to_hlist,
};
use tex_typeset::{HpackParams, PackSpec, hpack as hpack_nodes};

use crate::assignments;
use crate::executor::sync_engine_state;
use crate::mode::{DisplayEqNo, DisplayInterrupt, EqNoSide};
use crate::vertical::{
    append_node_to_vertical_list, append_vertical_contribution, build_page_if_outer_vertical,
};
use crate::{DispatchAction, ExecError, Mode, ModeNest, push_tokens};

mod scan;
mod support;

use scan::*;
use support::*;

pub(crate) fn finish_math_list_node(stores: &mut Universe, list: MathListNode) -> Vec<Node> {
    let params = MathParams::read(stores);
    let style = if list.display {
        Style::DISPLAY
    } else {
        Style::TEXT
    };
    let hlist = mlist_to_hlist(stores, list.content, style, !list.display, &params);
    let mut nodes = Vec::new();
    if !list.display {
        let surround = stores.dimen_param(DimenParam::MATH_SURROUND);
        nodes.push(Node::MathOn(surround));
    }
    nodes.extend(lower_math_hlist(stores, &hlist));
    if !list.display {
        let surround = stores.dimen_param(DimenParam::MATH_SURROUND);
        nodes.push(Node::MathOff(surround));
    }
    nodes
}

pub(crate) fn finish_math_lists(stores: &mut Universe, nodes: &[Node]) -> Vec<Node> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            Node::MathList(list) => out.extend(finish_math_list_node(stores, *list)),
            node => out.push(node.clone()),
        }
    }
    out
}

fn lower_math_hlist(stores: &mut Universe, hlist: &FrozenHList) -> Vec<Node> {
    hlist
        .nodes
        .iter()
        .map(|node| lower_math_node(stores, node))
        .collect()
}

fn lower_math_node(stores: &mut Universe, node: &MathNode) -> Node {
    match node {
        MathNode::Char { font, ch, .. } => Node::Char {
            font: *font,
            ch: *ch,
        },
        MathNode::Kern { amount, kind } => Node::Kern {
            amount: *amount,
            kind: *kind,
        },
        MathNode::Glue { spec, kind } => Node::Glue {
            spec: stores.intern_glue(*spec),
            kind: lower_math_glue_kind(*kind),
        },
        MathNode::Penalty(penalty) => Node::Penalty(*penalty),
        MathNode::Rule {
            width,
            height,
            depth,
        } => Node::Rule {
            width: *width,
            height: *height,
            depth: *depth,
        },
        MathNode::HList(boxed) => Node::HList(lower_math_box(stores, boxed)),
        MathNode::VList(boxed) => Node::VList(lower_math_box(stores, boxed)),
        MathNode::Opaque(node) => node.clone(),
    }
}

fn lower_math_box(stores: &mut Universe, boxed: &MathBox) -> BoxNode {
    let lowered = lower_math_hlist(stores, &boxed.list);
    let children = stores.freeze_node_list(&lowered);
    BoxNode::new(BoxNodeFields {
        width: boxed.width,
        height: boxed.height,
        depth: boxed.depth,
        shift: boxed.shift,
        display: false,
        glue_set: GlueSetRatio::from_raw(0),
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    })
}

fn lower_math_glue_kind(kind: MathGlueKind) -> GlueKind {
    match kind {
        MathGlueKind::NonScript => GlueKind::NonScript,
        MathGlueKind::MuSkip => GlueKind::MuSkip,
        MathGlueKind::ThinMuSkip => GlueKind::ThinMuSkip,
        MathGlueKind::MedMuSkip => GlueKind::MedMuSkip,
        MathGlueKind::ThickMuSkip => GlueKind::ThickMuSkip,
        MathGlueKind::Normal | MathGlueKind::Source => GlueKind::Normal,
    }
}

pub(crate) fn enter_math<S, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let opening_mode = nest.current_mode();
    let can_display = !matches!(opening_mode, Mode::RestrictedHorizontal);
    let display = match input.next_token(stores)? {
        Some(
            token @ Token::Char {
                cat: Catcode::MathShift,
                ..
            },
        ) if can_display => {
            let _ = token;
            true
        }
        Some(token) => {
            push_tokens(input, stores, [token]);
            false
        }
        None => false,
    };
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        assignments::ensure_horizontal_for_character(nest, input, stores)?;
    }
    if matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) {
        assignments::flush_pending_hchars(nest, stores)?;
    }
    let interrupt = if display {
        let paragraph = assignments::interrupt_paragraph_for_display(nest, stores)?;
        let dimensions = assignments::display_line_dimensions(nest, stores);
        let pre_display_size = paragraph
            .last_line
            .as_ref()
            .map_or(Scaled::from_raw(-Scaled::MAX_DIMEN.raw()), |line| {
                pre_display_size(stores, line)
            });
        let interrupt = DisplayInterrupt {
            pre_display_size,
            display_width: dimensions.width,
            display_indent: dimensions.indent,
            saved_pre_display_size: stores.dimen_param(DimenParam::PRE_DISPLAY_SIZE),
            saved_display_width: stores.dimen_param(DimenParam::DISPLAY_WIDTH),
            saved_display_indent: stores.dimen_param(DimenParam::DISPLAY_INDENT),
        };
        stores.set_dimen_param(DimenParam::PRE_DISPLAY_SIZE, interrupt.pre_display_size);
        stores.set_dimen_param(DimenParam::DISPLAY_WIDTH, interrupt.display_width);
        stores.set_dimen_param(DimenParam::DISPLAY_INDENT, interrupt.display_indent);
        Some(interrupt)
    } else {
        None
    };
    nest.push(if display {
        Mode::DisplayMath
    } else {
        Mode::Math
    });
    if let Some(interrupt) = interrupt {
        nest.current_list_mut().set_display_interrupt(interrupt);
    }
    let every = stores.tok_param(if display {
        TokParam::EVERY_DISPLAY
    } else {
        TokParam::EVERY_MATH
    });
    let tokens = stores.tokens(every).to_vec();
    push_tokens(input, stores, tokens);
    sync_engine_state::<S, _>(hooks, nest, stores);
    Ok(DispatchAction::Continue)
}

pub(crate) fn dispatch_math_token_with_recorder<S, R, H>(
    nest: &mut ModeNest,
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    match token {
        Token::Char {
            cat: Catcode::MathShift,
            ..
        } => finish_math(nest, input, stores),
        Token::Char {
            cat: Catcode::Space,
            ..
        } => Ok(DispatchAction::Continue),
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => {
            let list = scan_math_group_after_open(nest, input, stores, recorder, hooks)?;
            append_noad(
                nest,
                NoadKind::Normal(NoadClass::Ord),
                MathField::SubMlist(list),
            );
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        } => Err(ExecError::TooManyRightBraces),
        Token::Char {
            cat: Catcode::Superscript,
            ..
        } => {
            attach_script(nest, input, stores, recorder, hooks, true)?;
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            cat: Catcode::Subscript,
            ..
        } => {
            attach_script(nest, input, stores, recorder, hooks, false)?;
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => {
            redispatch_active_char(input, stores, ch);
            Ok(DispatchAction::Continue)
        }
        Token::Char { ch, .. } => {
            append_mathcode_char(nest, input, stores, ch)?;
            Ok(DispatchAction::Continue)
        }
        Token::Cs(symbol) => {
            dispatch_math_control(nest, token, symbol, input, stores, recorder, hooks)
        }
        Token::Param(_) => Ok(DispatchAction::NotConsumed),
    }
}

fn finish_math<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
{
    if close_missing_left_group(nest, stores)? {
        return finish_math(nest, input, stores);
    }
    let display = nest.current_mode() == Mode::DisplayMath;
    if display {
        match input.next_token(stores)? {
            Some(Token::Char {
                cat: Catcode::MathShift,
                ..
            }) => {}
            Some(token) => {
                push_tokens(input, stores, [token]);
                report_math_error(stores, "Display math should end with $$");
            }
            None => report_math_error(stores, "Display math should end with $$"),
        }
    }
    let content = finish_current_math_list(nest, stores);
    let mut level = nest.pop()?;
    if display {
        let eq_no = level.list_mut().take_display_eq_no();
        let interrupt = level.list_mut().take_display_interrupt().ok_or(
            ExecError::UnimplementedTypesetting {
                mode: Mode::DisplayMath,
                token: Token::Cs(stores.intern("display")),
                operation: "display interrupt state",
            },
        )?;
        finish_display_math(nest, input, stores, interrupt, content, eq_no)?;
    } else {
        nest.current_list_mut()
            .push(Node::MathList(MathListNode { display, content }));
    }
    Ok(DispatchAction::Continue)
}

fn dispatch_math_control<S, R, H>(
    nest: &mut ModeNest,
    token: Token,
    symbol: tex_state::interner::Symbol,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let meaning = stores.meaning(symbol);
    recorder.record_meaning(symbol, meaning);
    match meaning {
        Meaning::Relax => Ok(DispatchAction::Continue),
        Meaning::Undefined => Err(ExecError::UndefinedControlSequence {
            name: stores.resolve(symbol).to_owned(),
        }),
        Meaning::CharGiven(ch) => {
            append_mathcode_char(nest, input, stores, ch)?;
            Ok(DispatchAction::Continue)
        }
        Meaning::MathCharGiven(value) => {
            append_math_char_code(nest, stores, u32::from(value))?;
            Ok(DispatchAction::Continue)
        }
        Meaning::UnexpandablePrimitive(primitive) => {
            dispatch_math_primitive(primitive, nest, input, stores, recorder, hooks)
        }
        Meaning::ExpandablePrimitive(primitive) => match primitive {
            ExpandablePrimitive::Fi | ExpandablePrimitive::Else | ExpandablePrimitive::Or => {
                Err(ExecError::ExtraConditionalControl(primitive))
            }
            ExpandablePrimitive::EndCsName => Err(ExecError::ExtraEndCsName),
            _ => Err(ExecError::UnexpectedExpandableDelivery { token, primitive }),
        },
        Meaning::Macro { .. } => Err(ExecError::UnexpectedMacroDelivery {
            name: stores.resolve(symbol).to_owned(),
        }),
        meaning if math_allows_assignment_meaning(meaning) => {
            assignments::execute_assignment_meaning(meaning, input, stores, hooks)
        }
        Meaning::Font(id) => {
            stores.set_current_font_selector(symbol, id);
            Ok(DispatchAction::Continue)
        }
        Meaning::Unknown(raw) => Err(ExecError::UnsupportedCommand {
            token,
            opcode: raw.op(),
        }),
        _ => Ok(DispatchAction::NotConsumed),
    }
}

fn dispatch_math_primitive<S, R, H>(
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    match primitive {
        UnexpandablePrimitive::Par | UnexpandablePrimitive::EndGraf => {
            report_math_error(stores, "Missing $ inserted");
            finish_math(nest, input, stores)
        }
        UnexpandablePrimitive::MathChar => {
            let code = scan_math_char_code(input, stores, hooks)?;
            append_math_char_code(nest, stores, code)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Delimiter => {
            let delimiter = scan_delimiter_code(input, stores, hooks)?;
            let ch = char::from_u32(delimiter & 0xff).unwrap_or('\0');
            append_noad(
                nest,
                NoadKind::Normal(NoadClass::Ord),
                MathField::MathChar(MathChar {
                    family: ((delimiter >> 8) & 0xf) as u8,
                    character: ch,
                }),
            );
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MathOrd
        | UnexpandablePrimitive::MathOp
        | UnexpandablePrimitive::MathBin
        | UnexpandablePrimitive::MathRel
        | UnexpandablePrimitive::MathOpen
        | UnexpandablePrimitive::MathClose
        | UnexpandablePrimitive::MathPunct
        | UnexpandablePrimitive::MathInner => {
            let field = scan_math_field(nest, input, stores, recorder, hooks)?;
            append_noad(nest, noad_kind_for_constructor(primitive), field);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Underline | UnexpandablePrimitive::Overline => {
            let field = scan_math_field(nest, input, stores, recorder, hooks)?;
            append_noad(
                nest,
                if primitive == UnexpandablePrimitive::Underline {
                    NoadKind::Underline
                } else {
                    NoadKind::Overline
                },
                field,
            );
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Limits
        | UnexpandablePrimitive::NoLimits
        | UnexpandablePrimitive::DisplayLimits => {
            apply_limit_switch(nest, stores, primitive);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Over
        | UnexpandablePrimitive::Atop
        | UnexpandablePrimitive::Above
        | UnexpandablePrimitive::OverWithDelims
        | UnexpandablePrimitive::AtopWithDelims
        | UnexpandablePrimitive::AboveWithDelims => {
            start_fraction(primitive, nest, input, stores, hooks)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Radical => {
            let delimiter = scan_delimiter_code(input, stores, hooks)?;
            let field = scan_math_field(nest, input, stores, recorder, hooks)?;
            append_noad(nest, NoadKind::Radical { delimiter }, field);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MathAccent => {
            let accent = math_char_from_code(scan_math_char_code(input, stores, hooks)?, stores)?;
            let field = scan_math_field(nest, input, stores, recorder, hooks)?;
            append_noad(nest, NoadKind::Accent { accent }, field);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::VCenter => {
            let field = scan_vcenter_field(input, stores, hooks)?;
            append_noad(nest, NoadKind::VCenter, field);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MSkip => {
            let spec = assignments::scan_glue_id(input, stores, hooks, true)?;
            nest.current_list_mut().push(Node::Glue {
                spec,
                kind: GlueKind::MuSkip,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MKern => {
            let amount = scan_mu_dimen(input, stores, hooks)?;
            nest.current_list_mut().push(Node::Kern {
                amount,
                kind: KernKind::Mu,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::NonScript => {
            let spec = stores.intern_glue(GlueSpec::ZERO);
            nest.current_list_mut().push(Node::Glue {
                spec,
                kind: GlueKind::NonScript,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MathChoice => {
            append_math_choice(nest, input, stores, recorder, hooks)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::EqNo | UnexpandablePrimitive::LeftEqNo => {
            start_eq_no(nest, stores, primitive)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Left => {
            start_left_group(nest, input, stores, hooks)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Right => {
            finish_left_group(nest, input, stores, hooks)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::DisplayStyle
        | UnexpandablePrimitive::TextStyle
        | UnexpandablePrimitive::ScriptStyle
        | UnexpandablePrimitive::ScriptScriptStyle => {
            nest.current_list_mut()
                .push(Node::MathStyle(style_for_primitive(primitive)));
            Ok(DispatchAction::Continue)
        }
        primitive if math_allows_assignment_primitive(primitive) => {
            assignments::execute_unexpandable_with_recorder(
                primitive, nest, input, stores, recorder, hooks,
            )
        }
        _ => Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token: Token::Cs(stores.intern(&format!("{primitive:?}"))),
            operation: "math primitive",
        }),
    }
}

fn start_eq_no(
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

fn finish_display_math<S>(
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
            },
        );
    }

    restore_display_dimensions(stores, interrupt);
    resume_after_display(nest, input, stores)?;
    Ok(())
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

fn pre_display_size(stores: &Universe, line: &BoxNode) -> Scaled {
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
