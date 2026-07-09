use tex_expand::ExpansionHooks;
use tex_fonts::{LigKernChar, LigKernCommand};
use tex_lex::{InputSource, InputStack};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::{DiscKind, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::{PrintSink, Universe};
use tex_typeset::{INF_BAD, PackSpec, VpackParams, vpack};

use super::paragraph::ensure_horizontal_for_character;
use super::*;
use crate::vertical::{append_vertical_contribution, build_page_if_outer_vertical};
use crate::{ExecError, Mode, ModeNest};

pub(crate) fn try_append_character(
    nest: &mut ModeNest,
    token: Token,
    stores: &mut Universe,
) -> Result<bool, ExecError> {
    match (nest.current_mode(), token) {
        (Mode::RestrictedHorizontal | Mode::Horizontal, Token::Char { ch, cat }) => {
            if cat == Catcode::Space {
                append_space(nest, stores)?;
            } else {
                append_hchar(nest, stores, ch);
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub(crate) fn append_given_char<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    ch: char,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    match nest.current_mode() {
        Mode::RestrictedHorizontal | Mode::Horizontal => {
            append_hchar(nest, stores, ch);
            Ok(())
        }
        Mode::Vertical | Mode::InternalVertical => {
            ensure_horizontal_for_character(nest, input, stores)?;
            append_hchar(nest, stores, ch);
            Ok(())
        }
        mode => Err(ExecError::UnimplementedTypesetting {
            mode,
            token: Token::Char {
                ch,
                cat: Catcode::Other,
            },
            operation: "character",
        }),
    }
}

pub(crate) fn flush_pending_hchars(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let no_boundary = nest.current_list().no_boundary();
    let pending = nest.current_list_mut().take_pending_hchars();
    if pending.is_empty() {
        return Ok(());
    }
    nest.current_list_mut().set_no_boundary(false);
    let mut nodes = reconstitute(stores, &pending, no_boundary);
    nest.current_list_mut().append(nodes.drain(..));
    Ok(())
}

pub(super) fn execute_hmode_material<S, H>(
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    match primitive {
        UnexpandablePrimitive::Char => {
            let value = scan_i32(input, stores, hooks)?;
            let ch = char::from_u32(value as u32).ok_or(ExecError::InvalidCode {
                context: "\\char",
                value,
            })?;
            append_given_char(nest, input, stores, ch)?;
        }
        UnexpandablePrimitive::HFil
        | UnexpandablePrimitive::HFill
        | UnexpandablePrimitive::HSs
        | UnexpandablePrimitive::HFilNeg => {
            flush_pending_hchars(nest, stores)?;
            let spec = match primitive {
                UnexpandablePrimitive::HFil => infinite_glue(Order::Fil, false, false),
                UnexpandablePrimitive::HFill => infinite_glue(Order::Fill, false, false),
                UnexpandablePrimitive::HSs => infinite_glue(Order::Fil, false, true),
                UnexpandablePrimitive::HFilNeg => infinite_glue(Order::Fil, true, false),
                _ => unreachable!(),
            };
            let spec = stores.intern_glue(spec);
            nest.current_list_mut().push(Node::Glue {
                spec,
                kind: GlueKind::Normal,
                leader: None,
            });
        }
        UnexpandablePrimitive::Penalty => {
            flush_pending_hchars(nest, stores)?;
            let penalty = scan_i32(input, stores, hooks)?;
            append_vertical_contribution(nest, stores, Node::Penalty(penalty));
            build_page_if_outer_vertical(nest, stores)?;
        }
        UnexpandablePrimitive::VRule => {
            flush_pending_hchars(nest, stores)?;
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                ensure_horizontal_for_character(nest, input, stores)?;
            }
            nest.current_list_mut()
                .push(scan_rule_node(input, stores, hooks, primitive)?);
            nest.current_list_mut().set_space_factor(1000);
        }
        UnexpandablePrimitive::ControlSpace => append_control_space(nest, input, stores)?,
        UnexpandablePrimitive::ItalicCorrection => append_italic_correction(nest, stores)?,
        UnexpandablePrimitive::Discretionary => {
            flush_pending_hchars(nest, stores)?;
            let pre = scan_hlist_group(input, stores, hooks, "\\discretionary pre")?;
            let post = scan_hlist_group(input, stores, hooks, "\\discretionary post")?;
            let replace = scan_hlist_group(input, stores, hooks, "\\discretionary replace")?;
            nest.current_list_mut().push(Node::Disc {
                kind: DiscKind::Discretionary,
                pre,
                post,
                replace,
            });
        }
        UnexpandablePrimitive::DiscretionaryHyphen => {
            flush_pending_hchars(nest, stores)?;
            let font = stores.current_font();
            let hyphen = u8::try_from(stores.font_hyphen_char(font))
                .ok()
                .map(char::from)
                .unwrap_or('-');
            let pre = stores.freeze_node_list(&[Node::Char { font, ch: hyphen }]);
            let empty = stores.freeze_node_list(&[]);
            nest.current_list_mut().push(Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                pre,
                post: empty,
                replace: empty,
            });
        }
        UnexpandablePrimitive::NoBoundary => nest.current_list_mut().set_no_boundary(true),
        UnexpandablePrimitive::SpaceFactor => {
            skip_optional_equals_x(input, stores, hooks)?;
            let value = scan_i32(input, stores, hooks)?;
            if !(1..=32767).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\spacefactor",
                    value,
                });
            }
            nest.current_list_mut().set_space_factor(value);
        }
        UnexpandablePrimitive::Accent => execute_accent(nest, input, stores, hooks)?,
        UnexpandablePrimitive::Mark => {
            flush_pending_hchars(nest, stores)?;
            let tokens = scan_general_text_expanded_with_driver(input, stores, hooks)?;
            append_vertical_contribution(nest, stores, Node::Mark { class: 0, tokens });
        }
        UnexpandablePrimitive::VAdjust => execute_vadjust(nest, input, stores, hooks)?,
        UnexpandablePrimitive::Insert => execute_insert(nest, input, stores, hooks)?,
        _ => unreachable!("caller restricts hmode material primitives"),
    }
    Ok(())
}

fn execute_insert<S, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    flush_pending_hchars(nest, stores)?;
    let value = scan_i32(input, stores, hooks)?;
    if !(0..=254).contains(&value) {
        return Err(ExecError::InvalidCode {
            context: "\\insert",
            value,
        });
    }
    let opener = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "\\insert group",
    })?;
    if !is_begin_group(opener) {
        return Err(ExecError::MissingToken {
            context: "\\insert group",
        });
    }

    let mut inner = ModeNest::new();
    inner.push(Mode::InternalVertical);
    scan_box_group(&mut inner, input, stores, hooks)?;
    let level = inner.pop()?;
    let content = stores.freeze_node_list(level.list().nodes());
    let packed = vpack(
        stores,
        content,
        PackSpec::Natural,
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: Scaled::MAX_DIMEN,
            box_max_depth: Scaled::MAX_DIMEN,
        },
    );
    let size = packed
        .node
        .height
        .checked_add(packed.node.depth)
        .ok_or(ExecError::ArithmeticOverflow)?;

    append_vertical_contribution(
        nest,
        stores,
        Node::Ins {
            class: value as u16,
            size,
            split_top_skip: stores.glue_param(GlueParam::SPLIT_TOP_SKIP),
            split_max_depth: stores.dimen_param(DimenParam::SPLIT_MAX_DEPTH),
            floating_penalty: stores.int_param(IntParam::FLOATING_PENALTY),
            content,
        },
    );
    build_page_if_outer_vertical(nest, stores)?;
    Ok(())
}

fn execute_vadjust<S, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    if nest.current_mode() != Mode::Horizontal {
        return Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token: Token::Cs(stores.intern("vadjust")),
            operation: "\\vadjust",
        });
    }
    flush_pending_hchars(nest, stores)?;
    let opener = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "\\vadjust group",
    })?;
    if !is_begin_group(opener) {
        return Err(ExecError::MissingToken {
            context: "\\vadjust group",
        });
    }
    let mut inner = ModeNest::new();
    inner.push(Mode::InternalVertical);
    scan_box_group(&mut inner, input, stores, hooks)?;
    let level = inner.pop()?;
    let content = stores.freeze_node_list(level.list().nodes());
    nest.current_list_mut().push(Node::Adjust(content));
    Ok(())
}

fn append_space(nest: &mut ModeNest, stores: &mut Universe) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores)?;
    let sf = nest.current_list().space_factor();
    let spec = if sf >= 2000 {
        nonzero_glue_param_or_font_space(stores, GlueParam::XSPACE_SKIP, sf)
    } else {
        nonzero_glue_param_or_font_space(stores, GlueParam::SPACE_SKIP, sf)
    };
    let id = stores.intern_glue(spec);
    nest.current_list_mut().push(Node::Glue {
        spec: id,
        kind: GlueKind::Normal,
        leader: None,
    });
    Ok(())
}

fn append_control_space<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        ensure_horizontal_for_character(nest, input, stores)?;
    }
    flush_pending_hchars(nest, stores)?;
    let spec = normal_font_space(stores);
    let id = stores.intern_glue(spec);
    nest.current_list_mut().push(Node::Glue {
        spec: id,
        kind: GlueKind::Normal,
        leader: None,
    });
    nest.current_list_mut().set_space_factor(1000);
    Ok(())
}

fn append_hchar(nest: &mut ModeNest, stores: &mut Universe, ch: char) {
    let font = stores.current_font();
    if let Ok(code) = u8::try_from(ch as u32)
        && stores.font_char_exists(font, code)
    {
        nest.current_list_mut().push_pending_hchar(font, ch);
        update_space_factor(nest, stores, ch);
        return;
    }
    report_missing_character(stores, font, ch);
}

pub(crate) fn reconstitute(
    stores: &Universe,
    pending: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
) -> Vec<Node> {
    let mut out = Vec::new();
    let mut chars: Vec<_> = pending
        .iter()
        .map(|entry| ReChar {
            font: entry.font,
            ch: entry.ch,
            orig_first: entry.ch,
            orig_last: entry.ch,
        })
        .collect();
    let mut i = 0;
    while i < chars.len() {
        let current = chars[i];
        if !no_left_boundary
            && i == 0
            && let Some(node) = boundary_command_node(stores, current.into_pending(), true)
        {
            out.push(node);
        }
        if i + 1 < chars.len()
            && current.font == chars[i + 1].font
            && let (Ok(left), Ok(right)) = (font_code(current.ch), font_code(chars[i + 1].ch))
            && let Some(command) = stores.lig_kern_command(
                current.font,
                LigKernChar::Char(left),
                LigKernChar::Char(right),
            )
        {
            match command {
                LigKernCommand::Kern(amount) => {
                    out.push(Node::Char {
                        font: current.font,
                        ch: current.ch,
                    });
                    out.push(Node::Kern {
                        amount,
                        kind: KernKind::Font,
                    });
                    i += 1;
                    continue;
                }
                LigKernCommand::Ligature(lig) if lig.delete_next => {
                    chars[i] = ReChar {
                        font: current.font,
                        ch: char::from(lig.replacement),
                        orig_first: current.orig_first,
                        orig_last: chars[i + 1].orig_last,
                    };
                    chars.remove(i + 1);
                    continue;
                }
                LigKernCommand::Ligature(_) => {}
            }
        }
        if current.orig_first == current.ch && current.orig_last == current.ch {
            out.push(Node::Char {
                font: current.font,
                ch: current.ch,
            });
        } else {
            out.push(Node::Lig {
                font: current.font,
                ch: current.ch,
                orig: (current.orig_first, current.orig_last),
            });
        }
        i += 1;
    }
    out
}

#[derive(Clone, Copy)]
struct ReChar {
    font: tex_state::ids::FontId,
    ch: char,
    orig_first: char,
    orig_last: char,
}

impl ReChar {
    fn into_pending(self) -> crate::mode::PendingHChar {
        crate::mode::PendingHChar {
            font: self.font,
            ch: self.ch,
        }
    }
}

fn boundary_command_node(
    stores: &Universe,
    current: crate::mode::PendingHChar,
    left: bool,
) -> Option<Node> {
    let code = font_code(current.ch).ok()?;
    let command = if left {
        stores.lig_kern_command(current.font, LigKernChar::Boundary, LigKernChar::Char(code))?
    } else {
        stores.lig_kern_command(current.font, LigKernChar::Char(code), LigKernChar::Boundary)?
    };
    match command {
        LigKernCommand::Kern(amount) => Some(Node::Kern {
            amount,
            kind: KernKind::Font,
        }),
        LigKernCommand::Ligature(lig) => Some(Node::Lig {
            font: current.font,
            ch: char::from(lig.replacement),
            orig: (current.ch, current.ch),
        }),
    }
}

fn update_space_factor(nest: &mut ModeNest, stores: &Universe, ch: char) {
    let sf = i32::from(stores.sfcode(ch));
    if sf == 0 {
        return;
    }
    let current = nest.current_list().space_factor();
    let next = if sf < 1000 && current > 1000 {
        1000
    } else {
        sf
    };
    nest.current_list_mut().set_space_factor(next);
}

fn nonzero_glue_param_or_font_space(
    stores: &Universe,
    override_param: GlueParam,
    space_factor: i32,
) -> GlueSpec {
    let override_spec = stores.glue(stores.glue_param(override_param));
    if override_spec != GlueSpec::ZERO {
        return override_spec;
    }
    let font = stores.current_font();
    let mut spec = GlueSpec {
        width: stores.font_parameter(font, 2),
        stretch: stores.font_parameter(font, 3),
        stretch_order: Order::Normal,
        shrink: stores.font_parameter(font, 4),
        shrink_order: Order::Normal,
    };
    if space_factor >= 2000 {
        spec.width = spec
            .width
            .checked_add(stores.font_parameter(font, 7))
            .unwrap_or(spec.width);
    }
    if space_factor != 1000 {
        spec.stretch = scale_by_factor(spec.stretch, space_factor, 1000);
        spec.shrink = scale_by_factor(spec.shrink, 1000, space_factor);
    }
    spec
}

fn normal_font_space(stores: &Universe) -> GlueSpec {
    let font = stores.current_font();
    GlueSpec {
        width: stores.font_parameter(font, 2),
        stretch: stores.font_parameter(font, 3),
        stretch_order: Order::Normal,
        shrink: stores.font_parameter(font, 4),
        shrink_order: Order::Normal,
    }
}

fn scale_by_factor(value: Scaled, num: i32, den: i32) -> Scaled {
    Scaled::from_raw(((i64::from(value.raw()) * i64::from(num)) / i64::from(den)) as i32)
}

pub(super) fn infinite_glue(order: Order, negative: bool, shrink: bool) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(0),
        stretch: Scaled::from_raw(if negative {
            -Scaled::UNITY
        } else {
            Scaled::UNITY
        }),
        stretch_order: order,
        shrink: if shrink {
            Scaled::from_raw(Scaled::UNITY)
        } else {
            Scaled::from_raw(0)
        },
        shrink_order: if shrink { order } else { Order::Normal },
    }
}

fn report_missing_character(stores: &mut Universe, font: tex_state::ids::FontId, ch: char) {
    if stores.int_param(IntParam::new(36)) <= 0 {
        return;
    }
    let text = format!(
        "Missing character: There is no {} in font {}!\n",
        ch.escape_default(),
        stores.font_name(font)
    );
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, &text);
}

fn execute_accent<S, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    flush_pending_hchars(nest, stores)?;
    let accent_value = scan_i32(input, stores, hooks)?;
    let accent = u8::try_from(accent_value).map_err(|_| ExecError::InvalidCode {
        context: "\\accent",
        value: accent_value,
    })?;
    let base = scan_accent_base(input, stores, hooks)?;
    let font = stores.current_font();
    let accent_metrics = stores.font_char_metrics(font, accent);
    let base_metrics = stores.font_char_metrics(font, base);
    let shift = match (accent_metrics, base_metrics) {
        (Some(accent_metrics), Some(base_metrics)) => {
            Scaled::from_raw((base_metrics.width.raw() - accent_metrics.width.raw()) / 2)
        }
        _ => Scaled::from_raw(0),
    };
    nest.current_list_mut().push(Node::Kern {
        amount: shift,
        kind: KernKind::Accent,
    });
    nest.current_list_mut().push(Node::Char {
        font,
        ch: char::from(accent),
    });
    let back = accent_metrics
        .map(|metrics| Scaled::from_raw(-metrics.width.raw() - shift.raw()))
        .unwrap_or(Scaled::from_raw(-shift.raw()));
    nest.current_list_mut().push(Node::Kern {
        amount: back,
        kind: KernKind::Accent,
    });
    nest.current_list_mut().push(Node::Char {
        font,
        ch: char::from(base),
    });
    Ok(())
}

fn scan_accent_base<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<u8, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let token = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "\\accent base",
    })?;
    match token {
        Token::Char { ch, .. } => u8::try_from(ch as u32).map_err(|_| ExecError::InvalidCode {
            context: "\\accent base",
            value: ch as i32,
        }),
        Token::Cs(symbol) => match stores.meaning(symbol) {
            Meaning::CharGiven(ch) => u8::try_from(ch as u32).map_err(|_| ExecError::InvalidCode {
                context: "\\accent base",
                value: ch as i32,
            }),
            _ => Err(ExecError::MissingToken {
                context: "\\accent base",
            }),
        },
        Token::Param(_) => Err(ExecError::MissingToken {
            context: "\\accent base",
        }),
    }
}

pub(super) fn scan_rule_node<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    primitive: UnexpandablePrimitive,
) -> Result<Node, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let default_rule = Scaled::from_raw(26_214);
    let (mut width, mut height, mut depth) = if primitive == UnexpandablePrimitive::VRule {
        (Some(default_rule), None, None)
    } else {
        (None, Some(default_rule), Some(Scaled::from_raw(0)))
    };
    loop {
        if scan_optional_keyword_x(input, stores, hooks, "width")? {
            width = Some(scan_scaled(input, stores, hooks)?);
        } else if scan_optional_keyword_x(input, stores, hooks, "height")? {
            height = Some(scan_scaled(input, stores, hooks)?);
        } else if scan_optional_keyword_x(input, stores, hooks, "depth")? {
            depth = Some(scan_scaled(input, stores, hooks)?);
        } else {
            break;
        }
    }
    Ok(Node::Rule {
        width,
        height,
        depth,
    })
}

fn scan_hlist_group<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: &'static str,
) -> Result<tex_state::ids::NodeListId, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let opener =
        next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken { context })?;
    if !is_begin_group(opener) {
        return Err(ExecError::MissingToken { context });
    }
    let mut inner = ModeNest::new();
    inner.push(Mode::RestrictedHorizontal);
    scan_box_group(&mut inner, input, stores, hooks)?;
    let level = inner.pop()?;
    Ok(stores.freeze_node_list(level.list().nodes()))
}

fn append_italic_correction(nest: &mut ModeNest, stores: &mut Universe) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores)?;
    let Some((font, ch)) = last_font_char(nest.current_list().nodes()) else {
        return Ok(());
    };
    let Ok(code) = font_code(ch) else {
        return Ok(());
    };
    let Some(metrics) = stores.font_char_metrics(font, code) else {
        return Ok(());
    };
    if metrics.italic_correction.raw() != 0 {
        nest.current_list_mut().push(Node::Kern {
            amount: metrics.italic_correction,
            kind: KernKind::Explicit,
        });
    }
    Ok(())
}

fn last_font_char(nodes: &[Node]) -> Option<(tex_state::ids::FontId, char)> {
    match nodes.last()? {
        Node::Char { font, ch } | Node::Lig { font, ch, .. } => Some((*font, *ch)),
        _ => None,
    }
}

fn font_code(ch: char) -> Result<u8, ()> {
    u8::try_from(ch as u32).map_err(|_| ())
}
