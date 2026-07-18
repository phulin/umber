//! Math-mode stomach front-end.

use tex_expand::get_x_token_with_context;
use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, IntParam, TokParam};
use tex_state::glue::GlueSpec;
use tex_state::math::{MathField, MathFontSize, MathListNode, NoadClass, NoadKind};
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::provenance::InsertedOriginKind;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::assignments;
use crate::executor::sync_engine_state;
use crate::mode::DisplayInterrupt;
use crate::{
    DispatchAction, ExecError, Mode, ModeNest, insert_traced_tokens, leave_group_with_origin,
    push_tokens, push_traced_tokens,
};

mod display;
mod lower;
mod scan;
mod support;

use display::*;

#[cfg(test)]
pub(crate) fn testing_start_eq_no(
    nest: &mut ModeNest,
    stores: &mut Universe,
    primitive: UnexpandablePrimitive,
) -> Result<(), ExecError> {
    start_eq_no(nest, stores, primitive)
}

pub(crate) fn insert_dollar_sign(
    traced: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
) {
    let origin = traced.origin();
    let math_shift_token = Token::Char {
        ch: '$',
        cat: Catcode::MathShift,
    };
    let math_shift_origin =
        stores.inserted_origin(InsertedOriginKind::ErrorRecovery, math_shift_token, origin);
    let math_shift = TracedTokenWord::pack(math_shift_token, math_shift_origin);
    push_traced_tokens(input, stores, [math_shift, traced]);
    stores.world_mut().write_text(
        tex_state::PrintSink::TerminalAndLog,
        "\n! Missing $ inserted.\n\
         <inserted text>\n\
         <to be read again>\n\
         I've inserted a begin-math/end-math symbol since I think\n\
         you left one out. Proceed, with fingers crossed.\n",
    );
}
pub(crate) use lower::finish_math_list_node;
pub(crate) use lower::{finish_math_lists, finish_math_lists_owned};
use scan::*;
use support::*;

#[cfg(test)]
pub(crate) fn testing_finish_current_math_list(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> tex_state::ids::NodeListId {
    finish_current_math_list(nest, stores)
}

pub(crate) fn enter_math(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    debug_assert!(!matches!(
        nest.current_mode(),
        Mode::Vertical | Mode::InternalVertical
    ));
    let opening_mode = nest.current_mode();
    let can_display = !matches!(opening_mode, Mode::RestrictedHorizontal);
    let display = match input.next_traced_token(stores)? {
        Some(traced)
            if matches!(
                tex_expand::semantic_token(traced),
                Token::Char {
                    cat: Catcode::MathShift,
                    ..
                }
            ) && can_display =>
        {
            true
        }
        Some(traced) => {
            insert_traced_tokens(input, stores, [traced]);
            false
        }
        None => false,
    };
    if matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) {
        assignments::flush_pending_hchars(nest, stores)?;
    }
    // Math construction observes a broad family/code/style parameter surface
    // that is not yet represented in finished-line paragraph dependencies.
    execution.mark_paragraph_barrier(tex_state::ParagraphBarrierReason::DisplayMath);
    let interrupt = if display {
        crate::paragraph_memo::publish_prepared_hlist(
            input,
            stores,
            execution,
            nest.current_list().nodes(),
            nest.enclosing_vertical_prev_graf(),
        );
        let paragraph = assignments::interrupt_paragraph_for_display(nest, stores)?;
        let dimensions = assignments::display_line_dimensions(nest, stores);
        let pre_display_size = paragraph
            .last_line
            .as_ref()
            .map_or(Scaled::from_raw(-Scaled::MAX_DIMEN.raw()), |line| {
                pre_display_size(stores, line)
            });
        Some((
            pre_display_size,
            dimensions.width,
            dimensions.indent,
            paragraph.active_directions,
        ))
    } else {
        None
    };
    stores.enter_group_with_kind(tex_state::GroupKind::MathShift);
    if let Some((pre_display_size, display_width, display_indent, active_directions)) = &interrupt {
        stores.set_dimen_param(DimenParam::PRE_DISPLAY_SIZE, *pre_display_size);
        stores.set_dimen_param(DimenParam::DISPLAY_WIDTH, *display_width);
        stores.set_dimen_param(DimenParam::DISPLAY_INDENT, *display_indent);
        stores.set_int_param(
            IntParam::PRE_DISPLAY_DIRECTION,
            match active_directions.last() {
                Some(tex_state::node::Direction::BeginL) => 1,
                Some(tex_state::node::Direction::BeginR) => -1,
                _ => 0,
            },
        );
    }
    // tex.web `push_math(math_shift_group)` locally defines `\fam=-1` before
    // `\everymath`/`\everydisplay`, so variable-family mathcodes retain their
    // encoded family unless the formula explicitly selects another one.
    stores.set_int_param(tex_state::env::banks::IntParam::FAM, -1);
    nest.push(if display {
        Mode::DisplayMath
    } else {
        Mode::Math
    });
    if let Some((_, _, _, active_directions)) = interrupt {
        nest.current_list_mut()
            .set_display_interrupt(DisplayInterrupt { active_directions });
    }
    let every = stores.tok_param(if display {
        TokParam::EVERY_DISPLAY
    } else {
        TokParam::EVERY_MATH
    });
    let tokens = stores.tokens(every).to_vec();
    push_tokens(input, stores, tokens);
    sync_engine_state(execution, nest, stores);
    Ok(DispatchAction::Continue)
}

pub(crate) fn dispatch_math_token_with_context(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    let token = tex_expand::semantic_token(traced);
    let origin = traced.origin();
    match token {
        Token::Char {
            cat: Catcode::MathShift,
            ..
        } => {
            if stores.innermost_group_kind() == Some(tex_state::GroupKind::Math) {
                let right_brace = Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                };
                let inserted =
                    stores.inserted_origin(InsertedOriginKind::ErrorRecovery, right_brace, origin);
                input.back_input_alignment_token(traced);
                crate::insert_traced_tokens(
                    input,
                    stores,
                    [TracedTokenWord::pack(right_brace, inserted), traced],
                );
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    "\n! Missing } inserted.\nI've inserted something that you may have forgotten.\n",
                );
                Ok(DispatchAction::Continue)
            } else {
                finish_math(nest, input, stores, execution, origin)
            }
        }
        Token::Char {
            cat: Catcode::Space,
            ..
        } => Ok(DispatchAction::Continue),
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => {
            let noad = scan::scan_math_atom_group_after_open(nest, input, stores, execution)?;
            nest.current_list_mut().push(Node::MathNoad(noad));
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        } => {
            if let Err(error) =
                leave_group_with_origin(input, stores, tex_state::GroupKind::Simple, origin)
            {
                if matches!(error, ExecError::ExtraRightBraceOrForgottenDollar { .. }) {
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        "\n! Extra }, or forgotten $.\nI've deleted a group-closing symbol because it seems to be\nspurious, as in `$x}$'. But perhaps the } is legitimate and\nyou forgot something else, as in `\\hbox{$x}'.\n",
                    );
                } else {
                    return Err(error);
                }
            }
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            cat: Catcode::Superscript,
            ..
        } => {
            attach_script(nest, input, stores, execution, true)?;
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            cat: Catcode::Subscript,
            ..
        } => {
            attach_script(nest, input, stores, execution, false)?;
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
            append_mathcode_char(nest, input, stores, ch, traced.origin())?;
            Ok(DispatchAction::Continue)
        }
        Token::Cs(symbol) => dispatch_math_control(nest, traced, symbol, input, stores, execution),
        Token::Param(_) | Token::Frozen(_) => Ok(DispatchAction::NotConsumed),
    }
}

fn finish_math(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    origin: OriginId,
) -> Result<DispatchAction, ExecError> {
    // `off_save` inserts the terminator required by an intervening group and
    // then retries the math shift (tex.web §1027). TRIP deliberately leaves
    // a `\begingroup` open before a later `$`.
    while stores.innermost_group_kind() == Some(tex_state::GroupKind::SemiSimple) {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Missing \\endgroup inserted.\nI've inserted something that you may have forgotten.\n",
        );
        leave_group_with_origin(input, stores, tex_state::GroupKind::SemiSimple, origin)?;
    }
    if stores.innermost_group_kind().is_none() {
        // Malformed input can leave a math nest beneath the semisimple group
        // that `off_save` has just removed. Re-establish the matching tracked
        // boundary before finishing the nest so mode and environment state
        // remain synchronized for checkpointing.
        stores.enter_group_with_kind(tex_state::GroupKind::MathShift);
    }
    if close_missing_left_group(nest, stores)? {
        return finish_math(nest, input, stores, execution, origin);
    }
    if nest.current_mode() == Mode::Math && nest.current_list().display_eq_no().is_some() {
        return finish_equation_number(nest, input, stores, execution, origin);
    }
    let display = nest.current_mode() == Mode::DisplayMath;
    if display {
        match get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )? {
            Some(traced)
                if matches!(
                    tex_expand::semantic_token(traced),
                    Token::Char {
                        cat: Catcode::MathShift,
                        ..
                    }
                ) => {}
            Some(traced) => {
                push_traced_tokens(input, stores, [traced]);
                report_math_error(stores, "Display math should end with $$");
            }
            None => report_math_error(stores, "Display math should end with $$"),
        }
    }
    let mut content = finish_current_math_list(nest, stores);
    let font_failure = math_list_requires_font_parameters(stores, content)
        .then(|| math_font_failure(stores))
        .flatten();
    if let Some(failure) = font_failure {
        content = stores.freeze_node_list(&[]);
        stores
            .world_mut()
            .write_text(tex_state::PrintSink::TerminalAndLog, failure.diagnostic());
    }
    let mut level = nest.pop()?;
    if display {
        let interrupt = level.list_mut().take_display_interrupt().ok_or(
            ExecError::UnimplementedTypesetting {
                mode: Mode::DisplayMath,
                token: Token::Cs(stores.intern("display").symbol()),
                origin: OriginId::UNKNOWN,
                operation: "display interrupt state",
            },
        )?;
        finish_display_math(nest, stores, content, None)?;
        if stores.innermost_group_kind() == Some(tex_state::GroupKind::MathShift) {
            leave_group_with_origin(input, stores, tex_state::GroupKind::MathShift, origin)?;
        }
        resume_after_display(nest, input, stores, interrupt.active_directions)?;
    } else {
        let insert_penalties = nest.current_mode() == Mode::Horizontal;
        let nodes =
            finish_math_list_node(stores, MathListNode { display, content }, insert_penalties);
        nest.current_list_mut().append(nodes);
        // tex.web `Finish math in text`: an inline formula resets sentence
        // spacing before the math-shift group is unsaved.
        nest.current_list_mut().set_space_factor(1000);
        leave_group_with_origin(input, stores, tex_state::GroupKind::MathShift, origin)?;
    }
    Ok(DispatchAction::Continue)
}

fn finish_equation_number(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    origin: OriginId,
) -> Result<DispatchAction, ExecError> {
    match get_x_token_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
    )? {
        Some(traced)
            if matches!(
                tex_expand::semantic_token(traced),
                Token::Char {
                    cat: Catcode::MathShift,
                    ..
                }
            ) => {}
        Some(traced) => {
            push_traced_tokens(input, stores, [traced]);
            report_math_error(stores, "Display math should end with $$");
        }
        None => report_math_error(stores, "Display math should end with $$"),
    }

    let mut content = finish_current_math_list(nest, stores);
    let font_failure = math_list_requires_font_parameters(stores, content)
        .then(|| math_font_failure(stores))
        .flatten();
    if let Some(failure) = font_failure {
        content = stores.freeze_node_list(&[]);
        stores
            .world_mut()
            .write_text(tex_state::PrintSink::TerminalAndLog, failure.diagnostic());
    }
    let mut eq_level = nest.pop()?;
    let mut eq_no = eq_level
        .list_mut()
        .take_display_eq_no()
        .expect("equation-number mode carries its enclosing display");
    if font_failure.is_some() {
        eq_no.display = stores.freeze_node_list(&[]);
    }
    let finished_eq_no = finish_eq_no(stores, eq_no.side, content);
    leave_group_with_origin(input, stores, tex_state::GroupKind::MathShift, origin)?;

    let mut display_level = nest.pop()?;
    let interrupt = display_level.list_mut().take_display_interrupt().ok_or(
        ExecError::UnimplementedTypesetting {
            mode: Mode::DisplayMath,
            token: Token::Cs(stores.intern("display").symbol()),
            origin: OriginId::UNKNOWN,
            operation: "display interrupt state",
        },
    )?;
    finish_display_math(nest, stores, eq_no.display, Some(finished_eq_no))?;
    if stores.innermost_group_kind() == Some(tex_state::GroupKind::MathShift) {
        leave_group_with_origin(input, stores, tex_state::GroupKind::MathShift, origin)?;
    }
    resume_after_display(nest, input, stores, interrupt.active_directions)?;
    Ok(DispatchAction::Continue)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MathFontFailure {
    Symbol,
    Extension,
}

impl MathFontFailure {
    const fn diagnostic(self) -> &'static str {
        match self {
            Self::Symbol => {
                "\n! Math formula deleted: Insufficient symbol fonts.\n\
                 Sorry, but I can't typeset math unless \\textfont 2\n\
                 and \\scriptfont 2 and \\scriptscriptfont 2 have all\n\
                 the \\fontdimen values needed in math symbol fonts.\n"
            }
            Self::Extension => {
                "\n! Math formula deleted: Insufficient extension fonts.\n\
                 Sorry, but I can't typeset math unless \\textfont 3\n\
                 and \\scriptfont 3 and \\scriptscriptfont 3 have all\n\
                 the \\fontdimen values needed in math extension fonts.\n"
            }
        }
    }
}

fn math_font_failure(stores: &Universe) -> Option<MathFontFailure> {
    const SIZES: [MathFontSize; 3] = [
        MathFontSize::Text,
        MathFontSize::Script,
        MathFontSize::ScriptScript,
    ];
    if SIZES.into_iter().any(|size| {
        let font = stores.math_family_font(size, 2);
        stores.font_parameter_count(font) < 22
            && !matches!(
                stores.font(font).math_metrics_source(),
                tex_fonts::MathMetricsSource::OpenType(_)
            )
    }) {
        return Some(MathFontFailure::Symbol);
    }
    if SIZES.into_iter().any(|size| {
        let font = stores.math_family_font(size, 3);
        stores.font_parameter_count(font) < 13
            && !matches!(
                stores.font(font).math_metrics_source(),
                tex_fonts::MathMetricsSource::OpenType(_)
            )
    }) {
        return Some(MathFontFailure::Extension);
    }
    None
}

fn math_list_requires_font_parameters(
    stores: &Universe,
    content: tex_state::ids::NodeListId,
) -> bool {
    stores.nodes(content).into_iter().any(|node| {
        matches!(
            node.to_owned(),
            Node::MathNoad(_) | Node::FractionNoad(_) | Node::MathChoice(_) | Node::MathList(_)
        )
    })
}

#[cfg(test)]
pub(crate) fn testing_math_font_failure(stores: &Universe) -> Option<&'static str> {
    math_font_failure(stores).map(|failure| match failure {
        MathFontFailure::Symbol => "symbol",
        MathFontFailure::Extension => "extension",
    })
}

fn dispatch_math_control(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    symbol: tex_state::interner::Symbol,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    let token = tex_expand::semantic_token(traced);
    let origin = traced.origin();
    let meaning = stores.meaning(symbol);
    execution.record_meaning(symbol, meaning);
    if crate::dispatch::replay_unexpanded_command(traced, meaning, input, stores) {
        return Ok(DispatchAction::Continue);
    }
    match meaning {
        Meaning::Relax => Ok(DispatchAction::Continue),
        Meaning::Undefined => Err(ExecError::UndefinedControlSequence {
            name: stores.resolve(symbol).to_owned(),
            origin,
        }),
        Meaning::CharGiven(ch) => {
            append_mathcode_char(nest, input, stores, ch, origin)?;
            Ok(DispatchAction::Continue)
        }
        Meaning::MathCharGiven(value) => {
            append_math_char_code(nest, stores, u32::from(value), origin)?;
            Ok(DispatchAction::Continue)
        }
        Meaning::CharToken { ch, cat } => dispatch_math_token_with_context(
            nest,
            TracedTokenWord::pack(Token::Char { ch, cat }, origin),
            input,
            stores,
            execution,
        ),
        Meaning::UnexpandablePrimitive(primitive) => {
            dispatch_math_primitive(primitive, traced, nest, input, stores, execution)
        }
        Meaning::ExpandablePrimitive(primitive) => match primitive {
            ExpandablePrimitive::Fi | ExpandablePrimitive::Else | ExpandablePrimitive::Or => {
                Err(ExecError::ExtraConditionalControl { primitive, origin })
            }
            ExpandablePrimitive::EndCsName => {
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    "\n! Extra \\endcsname.\nI'm ignoring this control sequence.\n",
                );
                Ok(DispatchAction::Continue)
            }
            _ => Err(ExecError::UnexpectedExpandableDelivery {
                token,
                primitive,
                origin,
            }),
        },
        Meaning::Macro { .. } => Err(ExecError::UnexpectedMacroDelivery {
            name: stores.resolve(symbol).to_owned(),
            origin,
        }),
        meaning if assignments::is_assignment_target_meaning(meaning) => {
            assignments::execute_assignment_meaning(meaning, traced, input, stores, execution)
        }
        Meaning::Font(id) => {
            stores.set_current_font_selector(symbol, id);
            Ok(DispatchAction::Continue)
        }
        Meaning::Unknown(raw) => Err(ExecError::UnsupportedCommand {
            token,
            opcode: raw.op(),
            origin,
        }),
        _ => Ok(DispatchAction::NotConsumed),
    }
}

fn dispatch_math_primitive(
    primitive: UnexpandablePrimitive,
    traced: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    let token = tex_expand::semantic_token(traced);
    let origin = traced.origin();
    match primitive {
        UnexpandablePrimitive::Par
        | UnexpandablePrimitive::EndGraf
        | UnexpandablePrimitive::End
        | UnexpandablePrimitive::Dump => {
            insert_dollar_sign(traced, input, stores);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::SpaceFactor => {
            crate::diagnostics::report_illegal_case(stores, token, nest.current_mode());
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Indent | UnexpandablePrimitive::NoIndent => {
            if primitive == UnexpandablePrimitive::Indent {
                let box_node = assignments::make_indent_box(stores);
                let list = stores.freeze_node_list(&[box_node]);
                append_noad(
                    nest,
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::SubBox(list),
                );
            }
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::QuitVMode => Ok(DispatchAction::Continue),
        UnexpandablePrimitive::MoveLeft | UnexpandablePrimitive::MoveRight => {
            // These shifts are vertical-list commands. TeX's illegal-case
            // dispatch in math mode ignores the command without scanning its
            // dimension/box operands.
            crate::diagnostics::report_illegal_case(stores, token, nest.current_mode());
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::VSkip
        | UnexpandablePrimitive::VFil
        | UnexpandablePrimitive::VFill
        | UnexpandablePrimitive::VSs
        | UnexpandablePrimitive::VFilNeg => {
            // TeX.web §1044 classifies mmode+vskip as a missing-math-shift
            // case: close math first, then rescan the vertical command.
            insert_dollar_sign(traced, input, stores);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MathChar => {
            let code = scan_math_char_code(input, stores, execution, traced)?;
            append_math_char_code(nest, stores, code, traced.origin())?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Char => {
            let value = assignments::scan_i32(input, stores, execution, traced)?;
            let ch = u8::try_from(value)
                .map(char::from)
                .map_err(|_| ExecError::InvalidCode {
                    context: "\\char",
                    value,
                })?;
            append_mathcode_char(nest, input, stores, ch, traced.origin())?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Delimiter => {
            let delimiter = scan_delimiter_code(input, stores, execution, traced)?;
            // TeX82 treats a standalone \delimiter as the math character in
            // the high 15 bits; the low 12 bits only name its large variant.
            append_math_char_code(nest, stores, delimiter >> 12, traced.origin())?;
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
            let field = scan_math_field(nest, input, stores, execution)?;
            append_noad(nest, noad_kind_for_constructor(primitive), field);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Underline | UnexpandablePrimitive::Overline => {
            let field = scan_math_field(nest, input, stores, execution)?;
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
            start_fraction(primitive, traced, nest, input, stores, execution)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Radical => {
            let delimiter = scan_delimiter_code(input, stores, execution, traced)?;
            let field = scan_math_field(nest, input, stores, execution)?;
            append_noad(nest, NoadKind::Radical { delimiter }, field);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Accent | UnexpandablePrimitive::MathAccent => {
            if primitive == UnexpandablePrimitive::Accent {
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    "\n! Please use \\mathaccent for accents in math mode.\nI'm treating this as \\mathaccent.\n",
                );
            }
            let accent = math_char_from_code(
                scan_math_char_code(input, stores, execution, traced)?,
                stores,
                traced.origin(),
            )?;
            let field = scan_math_field(nest, input, stores, execution)?;
            append_noad(nest, NoadKind::Accent { accent }, field);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::VCenter => {
            let field = scan_vcenter_field(traced, input, stores, execution)?;
            append_noad(nest, NoadKind::VCenter, field);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::HBox
        | UnexpandablePrimitive::VBox
        | UnexpandablePrimitive::VTop
        | UnexpandablePrimitive::VSplit
        | UnexpandablePrimitive::Box
        | UnexpandablePrimitive::Copy
        | UnexpandablePrimitive::Raise
        | UnexpandablePrimitive::Lower => {
            if let Some(node) =
                assignments::scan_math_box(primitive, traced, nest, input, stores, execution)?
            {
                let list = stores.freeze_node_list(&[node]);
                append_noad(
                    nest,
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::SubBox(list),
                );
            }
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::LastBox
        | UnexpandablePrimitive::UnHBox
        | UnexpandablePrimitive::UnHCopy
        | UnexpandablePrimitive::Leaders
        | UnexpandablePrimitive::CLeaders
        | UnexpandablePrimitive::XLeaders => assignments::execute_unexpandable_with_context(
            primitive, traced, nest, input, stores, execution,
        ),
        UnexpandablePrimitive::HSkip
        | UnexpandablePrimitive::HFil
        | UnexpandablePrimitive::HFill
        | UnexpandablePrimitive::HSs
        | UnexpandablePrimitive::HFilNeg => {
            let spec = if primitive == UnexpandablePrimitive::HSkip {
                assignments::scan_glue_id(input, stores, execution, false, traced)?
            } else {
                let spec = assignments::fixed_infinite_glue(primitive);
                stores.intern_glue(spec)
            };
            nest.current_list_mut().push(Node::Glue {
                spec,
                kind: GlueKind::Normal,
                leader: None,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MSkip => {
            let spec = assignments::scan_glue_id(input, stores, execution, true, traced)?;
            nest.current_list_mut().push(Node::Glue {
                spec,
                kind: GlueKind::MuSkip,
                leader: None,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MKern => {
            let amount = scan_mu_dimen(input, stores, execution, traced)?;
            nest.current_list_mut().push(Node::Kern {
                amount,
                kind: KernKind::Mu,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Kern => {
            let amount = assignments::scan_scaled(input, stores, execution, traced)?;
            nest.current_list_mut().push(Node::Kern {
                amount,
                kind: KernKind::Explicit,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::ItalicCorrection => {
            nest.current_list_mut().push(Node::Kern {
                amount: Scaled::from_raw(0),
                kind: KernKind::Explicit,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::VRule => {
            let rule = assignments::scan_rule_node(input, stores, execution, primitive, traced)?;
            nest.current_list_mut().push(rule);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::NonScript => {
            let spec = stores.intern_glue(GlueSpec::ZERO);
            nest.current_list_mut().push(Node::Glue {
                spec,
                kind: GlueKind::NonScript,
                leader: None,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Penalty => {
            let penalty = assignments::scan_i32(input, stores, execution, traced)?;
            nest.current_list_mut().push(Node::Penalty(penalty));
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MathChoice => {
            append_math_choice(nest, input, stores, execution)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::EqNo | UnexpandablePrimitive::LeftEqNo => {
            if nest.current_mode() == Mode::DisplayMath {
                start_eq_no(nest, stores, primitive)?;
            } else {
                // `eq_no` is privileged in tex.web §1147. In ordinary
                // (negative) math mode TeX reports the illegal case and
                // ignores it; this is reached after non-math recovery has
                // inserted `$` and replayed the command.
                crate::diagnostics::report_illegal_case(stores, token, nest.current_mode());
            }
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::HAlign if nest.current_mode() == Mode::DisplayMath => {
            finish_display_halign(traced, nest, input, stores, execution)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Left => {
            start_left_group(nest, input, stores, execution)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Right => {
            finish_left_group(nest, input, stores, execution)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Middle => {
            append_middle_delimiter(nest, input, stores, execution)?;
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
        primitive if assignments::math_allows_mode_independent_primitive(primitive) => {
            assignments::execute_unexpandable_with_context(
                primitive, traced, nest, input, stores, execution,
            )
        }
        _ => Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token,
            origin,
            operation: "math primitive",
        }),
    }
}

fn finish_display_halign(
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    while stores.innermost_group_kind() == Some(tex_state::GroupKind::SemiSimple) {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Missing \\endgroup inserted.\nI've inserted something that you may have forgotten.\n",
        );
        leave_group_with_origin(
            input,
            stores,
            tex_state::GroupKind::SemiSimple,
            context.origin(),
        )?;
    }
    if !nest.current_list().nodes().is_empty() || nest.current_list().display_eq_no().is_some() {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Improper \\halign inside $$'s.\nDisplays can use special alignments (like \\eqalignno)\nonly if nothing but the alignment itself is between $$'s.\nSo I've deleted the formulas that preceded this alignment.\n",
        );
        let _ = nest.current_list_mut().take_nodes();
        let _ = nest.current_list_mut().take_display_eq_no();
    }
    let mut level = nest.pop()?;
    let interrupt =
        level
            .list_mut()
            .take_display_interrupt()
            .ok_or(ExecError::UnimplementedTypesetting {
                mode: Mode::DisplayMath,
                token: Token::Cs(stores.intern("display").symbol()),
                origin: OriginId::UNKNOWN,
                operation: "display interrupt state",
            })?;
    let nodes = crate::align::execute_display_halign(context, nest, input, stores, execution)?;
    finish_display_alignment_assignments(input, stores, execution)?;
    let closing_origin = consume_display_alignment_closer(input, stores, context.origin())?;
    finish_display_alignment(nest, stores, nodes)?;
    leave_group_with_origin(
        input,
        stores,
        tex_state::GroupKind::MathShift,
        closing_origin,
    )?;
    resume_after_display_alignment(nest, input, stores, interrupt.active_directions)
}

fn finish_display_alignment_assignments(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    loop {
        let Some(first) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        else {
            return Ok(());
        };
        if matches!(
            tex_expand::semantic_token(first),
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            continue;
        }
        let mut command = vec![first];
        let meaning = loop {
            let token = tex_expand::semantic_token(*command.last().expect("command token"));
            let Token::Cs(symbol) = token else {
                push_traced_tokens(input, stores, command);
                return Ok(());
            };
            let meaning = stores.meaning(symbol);
            if meaning == Meaning::Relax && command.len() == 1 {
                command.clear();
                break Meaning::Relax;
            }
            if matches!(
                meaning,
                Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Global
                        | UnexpandablePrimitive::Long
                        | UnexpandablePrimitive::Outer
                        | UnexpandablePrimitive::Protected
                )
            ) {
                let Some(next) = get_x_token_with_context(
                    input,
                    &mut tex_state::ExpansionContext::new(stores),
                    execution,
                )?
                else {
                    push_traced_tokens(input, stores, command);
                    return Ok(());
                };
                command.push(next);
                continue;
            }
            break meaning;
        };

        if command.is_empty() {
            continue;
        }

        if matches!(
            meaning,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetBox)
        ) {
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! Improper \\setbox.\nSorry, \\setbox is not allowed after \\halign in a display,\nor between \\accent and an accented character.\n",
            );
            if let Some(next) = get_x_token_with_context(
                input,
                &mut tex_state::ExpansionContext::new(stores),
                execution,
            )? && !matches!(
                tex_expand::semantic_token(next),
                Token::Char {
                    ch: '=',
                    cat: Catcode::Other,
                }
            ) {
                push_traced_tokens(input, stores, [next]);
            }
            return Ok(());
        }

        let first = command.remove(0);
        if !command.is_empty() {
            push_traced_tokens(input, stores, command);
        }
        if !assignments::try_execute_assignment(first, input, stores, execution)? {
            push_traced_tokens(input, stores, [first]);
            return Ok(());
        }
    }
}

fn consume_display_alignment_closer(
    input: &mut InputStack,
    stores: &mut Universe,
    fallback_origin: OriginId,
) -> Result<OriginId, ExecError> {
    let closing_origin = match input.next_traced_token(stores)? {
        Some(traced)
            if matches!(
                tex_expand::semantic_token(traced),
                Token::Char {
                    cat: Catcode::MathShift,
                    ..
                }
            ) =>
        {
            traced.origin()
        }
        Some(traced) => {
            insert_traced_tokens(input, stores, [traced]);
            report_math_error(stores, "Missing $$ inserted");
            return Ok(fallback_origin);
        }
        None => {
            report_math_error(stores, "Missing $$ inserted");
            return Ok(fallback_origin);
        }
    };

    match input.next_traced_token(stores)? {
        Some(traced)
            if matches!(
                tex_expand::semantic_token(traced),
                Token::Char {
                    cat: Catcode::MathShift,
                    ..
                }
            ) =>
        {
            Ok(closing_origin)
        }
        Some(traced) => {
            insert_traced_tokens(input, stores, [traced]);
            report_math_error(stores, "Missing $$ inserted");
            Ok(closing_origin)
        }
        None => {
            report_math_error(stores, "Missing $$ inserted");
            Ok(closing_origin)
        }
    }
}
