//! Math-mode stomach front-end.

use tex_expand::{ExpansionHooks, ReadRecorder};
use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, TokParam};
use tex_state::glue::GlueSpec;
use tex_state::math::{MathChar, MathField, MathListNode, NoadClass, NoadKind};
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};

use crate::assignments;
use crate::executor::sync_engine_state;
use crate::mode::DisplayInterrupt;
use crate::{DispatchAction, ExecError, Mode, ModeNest, push_tokens};

mod display;
mod lower;
mod scan;
mod support;

use display::*;
#[cfg(test)]
pub(crate) use lower::finish_math_list_node;
pub(crate) use lower::finish_math_lists;
use scan::*;
use support::*;

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
