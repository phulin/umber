//! Retired `Executor` math scanner and input-stack dispatch front.

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use tex_expand::get_x_token_with_context;
use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, IntParam, TokParam};
use tex_state::glue::GlueSpec;
use tex_state::math::{MathField, MathListNode, NoadClass, NoadKind};
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

#[cfg(test)]
use super::display;
use super::display::*;
use super::legacy_scan::{self as scan, *};
use super::lower::*;
use super::support::*;

fn resume_after_display_alignment(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    active_directions: Vec<tex_state::node::Direction>,
) -> Result<(), ExecError> {
    let prev_graf = nest
        .enclosing_vertical_prev_graf()
        .checked_add(3)
        .expect("display-math prev_graf overflow");
    nest.set_enclosing_vertical_prev_graf(prev_graf);
    let next = loop {
        match input.next_traced_token(stores)? {
            Some(traced)
                if matches!(
                    traced.semantic_token(),
                    Token::Char {
                        cat: Catcode::Space,
                        ..
                    }
                ) => {}
            other => break other,
        }
    };
    match next {
        Some(traced) if is_par_or_end_group(stores, traced.semantic_token()) => {
            insert_traced_tokens(input, stores, [traced]);
        }
        Some(traced) => {
            nest.push(Mode::Horizontal)?;
            stores.push_paragraph_start_line(stores.current_input_line());
            nest.current_list_mutation().set_space_factor(1000);
            nest.current_list_mutation()
                .append(active_directions.iter().copied().map(Node::Direction));
            insert_traced_tokens(input, stores, [traced]);
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
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)
    )
}

fn resume_after_display(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    active_directions: Vec<tex_state::node::Direction>,
) -> Result<(), ExecError> {
    let prev_graf = nest
        .enclosing_vertical_prev_graf()
        .checked_add(3)
        .expect("display-math prev_graf overflow");
    nest.set_enclosing_vertical_prev_graf(prev_graf);
    nest.push(Mode::Horizontal)?;
    stores.push_paragraph_start_line(stores.current_input_line());
    nest.current_list_mutation().set_space_factor(1000);
    nest.current_list_mutation()
        .append(active_directions.iter().copied().map(Node::Direction));
    match input.next_traced_token(stores)? {
        Some(traced)
            if matches!(
                traced.semantic_token(),
                Token::Char {
                    cat: Catcode::Space,
                    ..
                }
            ) => {}
        Some(traced) => insert_traced_tokens(input, stores, [traced]),
        None => {}
    }
    build_page_after_display_resume(nest, stores)?;
    Ok(())
}

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
) -> Result<(), ExecError> {
    let origin = traced.origin();
    let math_shift_token = Token::Char {
        ch: '$',
        cat: Catcode::MathShift,
    };
    let math_shift_origin =
        stores.inserted_origin(InsertedOriginKind::ErrorRecovery, math_shift_token, origin);
    let math_shift = TracedTokenWord::pack(math_shift_token, math_shift_origin);
    // §1047: `back_input` returns the offending token as its own `backed_up`
    // level, and `ins_error` then puts the `$` above it as `inserted`, so the
    // report shows both levels in that order.
    crate::error_report::back_tokens(input, stores, [traced]);
    crate::error_report::ins_error(
        input,
        stores,
        [math_shift],
        "Missing $ inserted",
        &[
            "I've inserted a begin-math/end-math symbol since I think",
            "you left one out. Proceed, with fingers crossed.",
        ],
    )?;
    Ok(())
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
                traced.semantic_token(),
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
        assignments::flush_pending_hchars(nest, stores, execution.command_fuel())?;
    }
    if display {
        crate::paragraph_memo::publish_prepared_hlist(
            input,
            stores,
            execution,
            nest.current_list().nodes(),
            nest.enclosing_vertical_prev_graf(),
            crate::executor::ParagraphContinuation::Display,
        );
        let paragraph = assignments::interrupt_paragraph_for_display(nest, stores, execution)?;
        return enter_math_after_paragraph(nest, input, stores, execution, Some(paragraph));
    }
    // Inline math is fully lowered into the paragraph's retained line graph.
    // Publication adds its explicit math-state dependency projection.
    execution.mark_paragraph_inline_math();
    enter_math_after_paragraph(nest, input, stores, execution, None)
}

fn enter_math_after_paragraph(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    paragraph: Option<assignments::ParagraphBreakResult>,
) -> Result<DispatchAction, ExecError> {
    let display = paragraph.is_some();
    let interrupt = paragraph.map(|paragraph| {
        let dimensions = assignments::display_line_dimensions(nest, stores);
        let pre_display_size = paragraph
            .last_line
            .as_ref()
            .map_or(Scaled::from_raw(-Scaled::MAX_DIMEN.raw()), |line| {
                pre_display_size(stores, line)
            });
        (
            pre_display_size,
            dimensions.width,
            dimensions.indent,
            paragraph.active_directions,
        )
    });
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
    })?;
    if let Some((_, _, _, active_directions)) = interrupt {
        nest.current_list_mutation()
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

pub(crate) fn enter_display_after_reused_paragraph(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    last_line: Option<tex_state::node::BoxNode>,
    active_directions: Vec<tex_state::node::Direction>,
) -> Result<(), ExecError> {
    let paragraph = assignments::ParagraphBreakResult {
        last_line,
        active_directions,
        finished_nodes: Vec::new(),
        line_count: 0,
    };
    let _ = enter_math_after_paragraph(nest, input, stores, execution, Some(paragraph))?;
    Ok(())
}

pub(crate) fn dispatch_math_token_with_context(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    let token = traced.semantic_token();
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
                // §1064's `off_save` for `math_group`: `back_input` returns the
                // math shift as its own `backed_up` level and `ins_list` puts
                // the `}` that matches the open group above it as `inserted`.
                crate::error_report::back_tokens(input, stores, [traced]);
                crate::error_report::ins_error(
                    input,
                    stores,
                    [TracedTokenWord::pack(right_brace, inserted)],
                    "Missing } inserted",
                    &OFF_SAVE_HELP,
                )?;
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
            nest.current_list_mutation().push(Node::MathNoad(noad));
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
                    // §1069's `extra_right_brace`, whose message names the
                    // terminator the open group wanted -- `$` here.
                    crate::error_report::report_input_error(
                        input,
                        stores,
                        "Extra }, or forgotten $",
                        &[
                            "I've deleted a group-closing symbol because it seems to be",
                            "spurious, as in `$x}$'. But perhaps the } is legitimate and",
                            "you forgot something else, as in `\\hbox{$x}'. In such cases",
                            "the way to recover is to insert both the forgotten and the",
                            "deleted material, e.g., by typing `I$}'.",
                        ],
                    )?;
                } else {
                    return Err(error);
                }
            } else {
                execution.paragraph_group_exited(stores);
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
            append_mathcode_char(nest, input, stores, execution, ch, traced.origin())?;
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
        crate::error_report::report_input_error(
            input,
            stores,
            "Missing \\endgroup inserted",
            &OFF_SAVE_HELP,
        )?;
        leave_group_with_origin(input, stores, tex_state::GroupKind::SemiSimple, origin)?;
        execution.paragraph_group_exited(stores);
    }
    if stores.innermost_group_kind().is_none() {
        // Malformed input can leave a math nest beneath the semisimple group
        // that `off_save` has just removed. Re-establish the matching tracked
        // boundary before finishing the nest so mode and environment state
        // remain synchronized for checkpointing.
        stores.enter_group_with_kind(tex_state::GroupKind::MathShift);
    }
    if close_missing_left_group(nest, input, stores, execution.command_fuel())? {
        return finish_math(nest, input, stores, execution, origin);
    }
    if nest.current_mode() == Mode::Math && nest.current_list().display_eq_no().is_some() {
        return finish_equation_number(nest, input, stores, execution, origin);
    }
    let display = nest.current_mode() == Mode::DisplayMath;
    if display {
        check_second_math_shift(input, stores, execution)?;
    }
    let mut content = finish_current_math_list(nest, stores);
    // TeX82 §1194 checks all three sizes of families 2 and 3 before
    // `fin_mlist`, even when the current mlist is empty. An empty formula (or
    // an empty equation-number script) must not bypass the check merely
    // because conversion would not otherwise consult a math font.
    if reject_invalid_math_fonts_in_input(input, stores)? {
        content = stores.freeze_node_list(&[]);
    }
    let mut level =
        crate::assignments::commit_current_list(nest, stores, execution.command_fuel())?;
    if display {
        let conversion_error_context = MathConversionErrorContext::new(
            crate::diagnostics::show_context(stores, &input.summary()),
        );
        let interrupt = level.list_mutation().take_display_interrupt().ok_or(
            ExecError::UnimplementedTypesetting {
                mode: Mode::DisplayMath,
                token: Token::Cs(stores.intern("display").symbol()),
                origin: OriginId::UNKNOWN,
                operation: "display interrupt state",
            },
        )?;
        finish_display_math(nest, stores, content, None, Some(&conversion_error_context))?;
        if stores.innermost_group_kind() == Some(tex_state::GroupKind::MathShift) {
            leave_group_with_origin(input, stores, tex_state::GroupKind::MathShift, origin)?;
            execution.paragraph_group_exited(stores);
        }
        resume_after_display(nest, input, stores, interrupt.active_directions)?;
    } else {
        let insert_penalties = nest.current_mode() == Mode::Horizontal;
        let (nodes, family_mask) = finish_inline_math_list_node(
            stores,
            MathListNode { display, content },
            insert_penalties,
            MathConversionErrorContext::new(crate::diagnostics::show_context(
                stores,
                &input.summary(),
            )),
        );
        execution.record_paragraph_math_families(family_mask);
        nest.current_list_mutation().append(nodes);
        // tex.web `Finish math in text`: an inline formula resets sentence
        // spacing before the math-shift group is unsaved.
        nest.current_list_mutation().set_space_factor(1000);
        leave_group_with_origin(input, stores, tex_state::GroupKind::MathShift, origin)?;
        execution.paragraph_group_exited(stores);
    }
    Ok(DispatchAction::Continue)
}

/// tex.web §1197's ``<Check that another `$' follows>``.
///
/// A display closes with two math shifts; the second is mandatory, and §327's
/// `back_error` returns anything else for the enclosing mode to read again.
fn check_second_math_shift(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    const HELP: [&str; 2] = [
        "The `$' that I just saw supposedly matches a previous `$$'.",
        "So I shall assume that you typed `$$' both times.",
    ];
    const MESSAGE: &str = "Display math should end with $$";
    match get_x_token_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
    )? {
        Some(traced)
            if matches!(
                traced.semantic_token(),
                Token::Char {
                    cat: Catcode::MathShift,
                    ..
                }
            ) => {}
        Some(traced) => {
            crate::error_report::back_error(input, stores, traced, MESSAGE, &HELP)?;
        }
        None => {
            crate::error_report::report_input_error(input, stores, MESSAGE, &HELP)?;
        }
    }
    Ok(())
}

fn finish_equation_number(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    origin: OriginId,
) -> Result<DispatchAction, ExecError> {
    check_second_math_shift(input, stores, execution)?;

    let mut content = finish_current_math_list(nest, stores);
    // This is §1194's first, equation-number-side check. It precedes
    // `fin_mlist` and is unconditional, just like the display-side check
    // below.
    let font_failure = reject_invalid_math_fonts_in_input(input, stores)?;
    if font_failure {
        content = stores.freeze_node_list(&[]);
    }
    let mut eq_level =
        crate::assignments::commit_current_list(nest, stores, execution.command_fuel())?;
    let mut eq_no = eq_level
        .list_mutation()
        .take_display_eq_no()
        .expect("equation-number mode carries its enclosing display");
    if font_failure {
        eq_no.display = stores.freeze_node_list(&[]);
    }
    let conversion_error_context =
        MathConversionErrorContext::new(crate::diagnostics::show_context(stores, &input.summary()));
    let finished_eq_no = finish_eq_no(stores, eq_no.side, content, Some(&conversion_error_context));
    leave_group_with_origin(input, stores, tex_state::GroupKind::MathShift, origin)?;
    execution.paragraph_group_exited(stores);

    // TeX82 §1194 repeats the check for the saved display mlist after boxing
    // the equation number.
    if reject_invalid_math_fonts_in_input(input, stores)? {
        eq_no.display = stores.freeze_node_list(&[]);
    }
    let mut display_level =
        crate::assignments::commit_current_list(nest, stores, execution.command_fuel())?;
    let interrupt = display_level
        .list_mutation()
        .take_display_interrupt()
        .ok_or(ExecError::UnimplementedTypesetting {
            mode: Mode::DisplayMath,
            token: Token::Cs(stores.intern("display").symbol()),
            origin: OriginId::UNKNOWN,
            operation: "display interrupt state",
        })?;
    let conversion_error_context =
        MathConversionErrorContext::new(crate::diagnostics::show_context(stores, &input.summary()));
    finish_display_math(
        nest,
        stores,
        eq_no.display,
        Some(finished_eq_no),
        Some(&conversion_error_context),
    )?;
    if stores.innermost_group_kind() == Some(tex_state::GroupKind::MathShift) {
        leave_group_with_origin(input, stores, tex_state::GroupKind::MathShift, origin)?;
        execution.paragraph_group_exited(stores);
    }
    resume_after_display(nest, input, stores, interrupt.active_directions)?;
    Ok(DispatchAction::Continue)
}

/// tex.web §1195's `<Check that the necessary fonts for math symbols are
/// present>`, reporting through §82 with the live input stack's context.
pub(crate) fn reject_invalid_math_fonts_in_input(
    input: &InputStack,
    stores: &mut Universe,
) -> Result<bool, ExecError> {
    let Some(failure) = super::math_font_failure(stores) else {
        return Ok(false);
    };
    let (message, help) = failure.report();
    crate::error_report::report_input_error(input, stores, message, &help)?;
    Ok(true)
}

fn dispatch_math_control(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    symbol: tex_state::interner::Symbol,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    let token = traced.semantic_token();
    let origin = traced.origin();
    let meaning = stores.meaning(symbol);
    execution.record_meaning(symbol, meaning);
    match meaning {
        Meaning::Relax => Ok(DispatchAction::Continue),
        Meaning::Undefined => Err(ExecError::UndefinedControlSequence {
            name: stores.resolve(symbol).to_owned(),
            origin,
        }),
        Meaning::CharGiven(ch) => {
            append_mathcode_char(nest, input, stores, execution, ch, origin)?;
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
                // §1135's `cs_error`.
                crate::error_report::report_input_error(
                    input,
                    stores,
                    "Extra \\endcsname",
                    &["I'm ignoring this, since I wasn't doing a \\csname."],
                )?;
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
    let token = traced.semantic_token();
    let origin = traced.origin();
    match primitive {
        UnexpandablePrimitive::Par | UnexpandablePrimitive::End | UnexpandablePrimitive::Dump => {
            insert_dollar_sign(traced, input, stores)?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::SpaceFactor => {
            crate::diagnostics::report_illegal_case(stores, token, nest.current_mode())?;
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
            crate::diagnostics::report_illegal_case(stores, token, nest.current_mode())?;
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::VSkip
        | UnexpandablePrimitive::VFil
        | UnexpandablePrimitive::VFill
        | UnexpandablePrimitive::VSs
        | UnexpandablePrimitive::VFilNeg => {
            // TeX.web §1044 classifies mmode+vskip as a missing-math-shift
            // case: close math first, then rescan the vertical command.
            insert_dollar_sign(traced, input, stores)?;
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
            append_mathcode_char(nest, input, stores, execution, ch, traced.origin())?;
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
            apply_limit_switch(nest, input, stores, primitive)?;
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
                // §1165's `<Complain that the user should have said
                // \mathaccent>`; scanning then continues as `\mathaccent`.
                crate::error_report::report_input_error(
                    input,
                    stores,
                    "Please use \\mathaccent for accents in math mode",
                    &[
                        "I'm changing \\accent to \\mathaccent here; wish me luck.",
                        "(Accents are not the same in formulas as they are in text.)",
                    ],
                )?;
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
            nest.current_list_mutation().push(Node::Glue {
                spec,
                kind: GlueKind::Normal,
                leader: None,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MSkip => {
            let spec = assignments::scan_glue_id(input, stores, execution, true, traced)?;
            nest.current_list_mutation().push(Node::Glue {
                spec,
                kind: GlueKind::MuSkip,
                leader: None,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::MKern => {
            let amount = scan_mu_dimen(input, stores, execution, traced)?;
            nest.current_list_mutation().push(Node::Kern {
                amount,
                kind: KernKind::Mu,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Kern => {
            let amount = assignments::scan_scaled(input, stores, execution, traced)?;
            nest.current_list_mutation().push(Node::Kern {
                amount,
                kind: KernKind::Explicit,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::ItalicCorrection => {
            // TeX82 §1113: `mmode+ital_corr: tail_append(new_kern(0));` --
            // unlike hmode's italic correction, this never overrides
            // `new_kern`'s default `normal` subtype to `explicit`, so it must
            // not become a legal kern-then-glue line-break point the way an
            // explicit `\kern` or hmode `\/` would.
            nest.current_list_mutation().push(Node::Kern {
                amount: Scaled::from_raw(0),
                kind: KernKind::Font,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::VRule => {
            let rule = assignments::scan_rule_node(input, stores, execution, primitive, traced)?;
            nest.current_list_mutation().push(rule);
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::NonScript => {
            let spec = stores.intern_glue(GlueSpec::ZERO);
            nest.current_list_mutation().push(Node::Glue {
                spec,
                kind: GlueKind::NonScript,
                leader: None,
            });
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::Penalty => {
            let penalty = assignments::scan_i32(input, stores, execution, traced)?;
            nest.current_list_mutation().push(Node::Penalty(penalty));
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
                crate::diagnostics::report_illegal_case(stores, token, nest.current_mode())?;
            }
            Ok(DispatchAction::Continue)
        }
        UnexpandablePrimitive::HAlign => {
            if nest.current_mode() == Mode::DisplayMath {
                finish_display_halign(traced, nest, input, stores, execution)?;
            } else {
                // TeX.web §1130 dispatches `mmode+halign` through
                // `privileged`. Inline math is negative mmode, so §1051
                // diagnoses the illegal case and ignores only `\halign`.
                crate::diagnostics::report_illegal_case(stores, token, nest.current_mode())?;
            }
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
            nest.current_list_mutation()
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
        crate::error_report::report_input_error(
            input,
            stores,
            "Missing \\endgroup inserted",
            &OFF_SAVE_HELP,
        )?;
        leave_group_with_origin(
            input,
            stores,
            tex_state::GroupKind::SemiSimple,
            context.origin(),
        )?;
        execution.paragraph_group_exited(stores);
    }
    if !nest.current_list().nodes().is_empty() || nest.current_list().display_eq_no().is_some() {
        // §1206's `<Check for improper alignment in displayed math>`.
        crate::error_report::report_input_error(
            input,
            stores,
            "Improper \\halign inside $$'s",
            &[
                "Displays can use special alignments (like \\eqalignno)",
                "only if nothing but the alignment itself is between $$'s.",
                "So I've deleted the formulas that preceded this alignment.",
            ],
        )?;
        let _ = nest.current_list_mutation().take_nodes();
        let _ = nest.current_list_mutation().take_display_eq_no();
    }
    let mut level =
        crate::assignments::commit_current_list(nest, stores, execution.command_fuel())?;
    let interrupt = level.list_mutation().take_display_interrupt().ok_or(
        ExecError::UnimplementedTypesetting {
            mode: Mode::DisplayMath,
            token: Token::Cs(stores.intern("display").symbol()),
            origin: OriginId::UNKNOWN,
            operation: "display interrupt state",
        },
    )?;
    let nodes = crate::align::execute_display_halign(context, nest, input, stores, execution)?;
    finish_display_alignment_assignments(input, stores, execution)?;
    let closing_origin =
        consume_display_alignment_closer(input, stores, execution, context.origin())?;
    finish_display_alignment(nest, stores, nodes)?;
    leave_group_with_origin(
        input,
        stores,
        tex_state::GroupKind::MathShift,
        closing_origin,
    )?;
    execution.paragraph_group_exited(stores);
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
            first.semantic_token(),
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            continue;
        }
        let mut command = vec![first];
        let meaning = loop {
            let token = (*command.last().expect("command token")).semantic_token();
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
            // §1241 scans the register and optional equals before consulting
            // `set_box_allowed`; only the box command and its body remain input.
            let context = *command.last().expect("setbox command token");
            let _ = assignments::scan_setbox_target(input, stores, execution, context)?;
            // §1241's `set_box` guard on `set_box_allowed`.
            crate::error_report::report_input_error(
                input,
                stores,
                "Improper \\setbox",
                &[
                    "Sorry, \\setbox is not allowed after \\halign in a display,",
                    "or between \\accent and an accented character.",
                ],
            )?;
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

/// tex.web §1206's `<Finish an alignment in a display>` closing `$$`.
///
/// A first token that is not a math shift takes §1206's own `<Pontificate
/// about improper alignment in display>`; once one has been seen, the second
/// `$` is §1197's mandatory one.
fn consume_display_alignment_closer(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    fallback_origin: OriginId,
) -> Result<OriginId, ExecError> {
    const HELP: [&str; 2] = [
        "Displays can use special alignments (like \\eqalignno)",
        "only if nothing but the alignment itself is between $$'s.",
    ];
    let closing_origin = match input.next_traced_token(stores)? {
        Some(traced)
            if matches!(
                traced.semantic_token(),
                Token::Char {
                    cat: Catcode::MathShift,
                    ..
                }
            ) =>
        {
            traced.origin()
        }
        Some(traced) => {
            crate::error_report::back_error(input, stores, traced, "Missing $$ inserted", &HELP)?;
            return Ok(fallback_origin);
        }
        None => {
            crate::error_report::report_input_error(input, stores, "Missing $$ inserted", &HELP)?;
            return Ok(fallback_origin);
        }
    };
    check_second_math_shift(input, stores, execution)?;
    Ok(closing_origin)
}
