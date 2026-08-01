//! Diagnostic and log-writing primitives.

use std::fmt::Write as _;

use tex_expand::{
    get_x_token_with_context, meaning_text, scan_dimen::DimensionDiagnostic,
    scan_int::IntegerDiagnostic, scan_the_text_with_context, token_text,
};
use tex_lex::InputStack;
use tex_state::env::banks::IntParam;
use tex_state::page::{PageContents, PageDimension};
use tex_state::print::Selector;
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::{PrintSink, Universe};

use crate::mode::ignored_depth;
use crate::node_dump::{DumpConfig, dump_node_list, dump_node_slice};

/// TeX82 §510's `<Terminate the current conditional and skip to \fi>` for an
/// `\or`/`\else`/`\fi` that matches no live `\if`.
pub(crate) fn report_extra_conditional(stores: &mut Universe, name: &str) -> Result<(), ExecError> {
    let context = show_context(stores, stores.input_summary());
    crate::error_report::report_error(
        stores,
        &format!("Extra \\{name}"),
        &["I'm ignoring this; it doesn't match any \\if."],
        context,
    )?;
    Ok(())
}

/// e-TeX's `\interactionmode` case of TeX82 §1243's `alter_integer`.
///
/// The parenthesized value is §91's `int_error`, which prints it as part of
/// the message line rather than as a second report.
pub(crate) fn report_bad_interaction_mode(
    stores: &mut Universe,
    value: i32,
) -> Result<(), ExecError> {
    let context = show_context(stores, stores.input_summary());
    crate::error_report::report_error(
        stores,
        &format!("Bad interaction mode ({value})"),
        &[
            "Modes are 0=batch, 1=nonstop, 2=scroll, and",
            "3=errorstop. Proceed, and I'll ignore this case.",
        ],
        context,
    )?;
    Ok(())
}

/// [`report_illegal_case_with_context`] for a caller whose input stack is the
/// gullet's rather than the canonical command core's.
pub(crate) fn report_illegal_case(
    stores: &mut Universe,
    token: Token,
    mode: Mode,
) -> Result<(), ExecError> {
    let context = show_context(stores, stores.input_summary());
    report_illegal_case_with_context(stores, token, mode, Some(context))?;
    Ok(())
}

/// TeX82 §1049's `you_cant` message followed by §1050's `report_illegal_case`.
pub(crate) fn report_illegal_case_with_context(
    stores: &mut Universe,
    token: Token,
    mode: Mode,
    context: Option<String>,
) -> Result<(), ExecError> {
    let command = tex_command::command_token_text(&mut stores.command_context(), token);
    let mode = mode_name(mode);
    // TeX82 §§82 and 1111: `report_illegal_case` installs help and then
    // calls the ordinary error routine. The context therefore precedes help
    // in every interaction mode, and §90 routes scrolled help to the log
    // instead of leaving it on the terminal.
    let mut report = stores.print_err(&format!("You can't use `{command}' in {mode}"));
    report.help(&[
        "Sorry, but I'm not programmed to handle this case;",
        "I'll just pretend that you didn't ask for it.",
        "If you're in the wrong mode, you might be able to",
        "return to the right one by typing `I}' or `I$' or `I\\par'.",
    ]);
    if let Some(context) = context {
        report.context(context);
    }
    Ok(report.error().jump_out()?)
}

const fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Vertical => "vertical mode",
        Mode::InternalVertical => "internal vertical mode",
        Mode::Horizontal => "horizontal mode",
        Mode::RestrictedHorizontal => "restricted horizontal mode",
        Mode::Math => "math mode",
        Mode::DisplayMath => "display math mode",
    }
}
use crate::{ExecError, push_tokens, push_traced_tokens};
use crate::{Mode, ModeNest};

/// TeX82 §370's `Complain about an undefined macro` report.
///
/// §370 reaches §82 with the offending control sequence still the last thing
/// read, so its context display ends the top line with it -- which is what
/// the help text means by "the control sequence at the end of the top line".
/// A caller that cannot supply that display passes `None` rather than an
/// empty string, so the report omits the context instead of printing a blank
/// one.
pub(crate) fn report_undefined_control_sequence(
    stores: &mut Universe,
    context: Option<String>,
) -> Result<(), ExecError> {
    let mut report = stores.print_err("Undefined control sequence");
    report.help(&[
        "The control sequence at the end of the top line",
        "of your error message was never \\def'ed. If you have",
        "misspelled it (e.g., `\\hobx'), type `I' and the correct",
        "spelling (e.g., `I\\hbox'). Otherwise just continue,",
        "and I'll forget about whatever was undefined.",
    ]);
    if let Some(context) = context {
        report.context(context);
    }
    report.error().jump_out()?;
    Ok(())
}

/// [`report_undefined_control_sequence`] for a caller holding a live gullet
/// input stack rather than the canonical command core's cursor.
pub(crate) fn report_undefined_control_sequence_in_input(
    input: &InputStack,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let context = show_context(stores, &input.summary());
    report_undefined_control_sequence(stores, Some(context))?;
    Ok(())
}

/// TeX82 §1128's no-alignment-in-progress branch of `align_error`.
pub(crate) fn report_misplaced_alignment_delimiter(
    stores: &mut Universe,
    token: Token,
    context: Option<String>,
) -> Result<(), ExecError> {
    let delimiter = tex_command::command_token_text(&mut stores.command_context(), token);
    let tab_mark = matches!(
        token,
        Token::Char {
            cat: Catcode::AlignmentTab,
            ..
        }
    );
    let mut report = stores.print_err("Misplaced ");
    report.print(&delimiter);
    if tab_mark {
        report.help(&[
            "I can't figure out why you would want to use a tab mark",
            "here. If you just want an ampersand, the remedy is",
            "simple: Just type `I\\&' now. But if some right brace",
            "up above has ended a previous alignment prematurely,",
            "you're probably due for more error messages, and you",
            "might try typing `S' now just to see what is salvageable.",
        ]);
    } else {
        report.help(&[
            "I can't figure out why you would want to use a tab mark",
            "or \\cr or \\span just now. If something like a right brace",
            "up above has ended a previous alignment prematurely,",
            "you're probably due for more error messages, and you",
            "might try typing `S' now just to see what is salvageable.",
        ]);
    }
    if let Some(context) = context {
        report.context(context);
    }
    report.error().jump_out()?;
    Ok(())
}

pub(crate) fn execute_show(input: &mut InputStack, stores: &mut Universe) -> Result<(), ExecError> {
    let token = tex_expand::get_token(input, &mut tex_state::ExpansionContext::new(stores))?
        .ok_or(ExecError::MissingToken { context: "\\show" })?;
    let token = tex_expand::semantic_token(token);
    let text = match token {
        Token::Cs(_)
        | Token::Char {
            cat: Catcode::Active,
            ..
        } => {
            format!(
                "\n> {}={}.\n",
                token_text(stores, token),
                show_meaning_text(stores, token)
            )
        }
        Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => {
            format!("\n> {}.\n", meaning_text(stores, token))
        }
    };
    write_diagnostic(stores, &text);
    Ok(())
}

pub(crate) fn execute_showthe(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let text = match scan_the_text_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        context,
    ) {
        Ok(text) => text,
        Err(tex_expand::ExpandError::UnsupportedTheTarget { context }) => {
            let token = tex_expand::semantic_token(context);
            let rendered = match token {
                Token::Char { ch, cat } => format!("{} character {ch}", catcode_name(cat)),
                _ => meaning_text(stores, token),
            };
            // TeX82 §428's `<Complain that \the can't do this; give zero
            // result>`. The zero below is §428's `cur_val:=0`, not a guess.
            crate::error_report::report_input_error(
                input,
                stores,
                &format!("You can't use `{rendered}' after \\the"),
                &["I'm forgetting what you said and using zero instead."],
            )?;
            "0".to_owned()
        }
        Err(error) => return Err(error.into()),
    };
    write_diagnostic(stores, &format!("\n> {text}.\n"));
    Ok(())
}

fn catcode_name(cat: Catcode) -> &'static str {
    match cat {
        Catcode::MathShift => "math shift",
        Catcode::BeginGroup => "begin-group",
        Catcode::EndGroup => "end-group",
        Catcode::AlignmentTab => "alignment tab",
        Catcode::Parameter => "macro parameter",
        Catcode::Superscript => "superscript",
        Catcode::Subscript => "subscript",
        Catcode::Space => "blank space",
        Catcode::Letter => "the letter",
        Catcode::Other => "the character",
        Catcode::Active => "active character",
        Catcode::Escape => "escape",
        Catcode::EndLine => "end of line",
        Catcode::Ignored => "ignored",
        Catcode::Comment => "comment",
        Catcode::Invalid => "invalid character",
    }
}

pub(crate) fn execute_showtokens(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let tokens = tex_expand::scan::scan_general_text_with_expanded_open_with_driver(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        context,
    )?;
    write_diagnostic(
        stores,
        &format!(
            "\n> {}.\n",
            tokens_text(stores, stores.tokens(tokens.token_list()))
        ),
    );
    Ok(())
}

pub(crate) fn execute_showgroups(stores: &mut Universe) {
    let kinds = stores.group_kinds().collect::<Vec<_>>();
    let mut text = String::new();
    text.push('\n');
    for (index, kind) in kinds.iter().enumerate().rev() {
        let level = index + 1;
        text.push_str("### ");
        text.push_str(group_kind_text(*kind));
        text.push_str(" (level ");
        text.push_str(&level.to_string());
        text.push_str(") (");
        text.push_str(kind.start_text());
        text.push_str(")\n");
    }
    text.push_str("### bottom level\n\n! OK.\n");
    write_diagnostic(stores, &text);
}

/// Detached e-TeX [49.1292] rendering record for one save level.
///
/// The command core resolves each level's mode/list relationship before
/// opening the diagnostic channel. Rendering therefore cannot mutate either
/// the save stack or the executor-owned semantic nest, unlike WEB's temporary
/// reassignment of `save_ptr`, `cur_level`, and `cur_group`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShowGroupFrame {
    pub(crate) kind: tex_state::GroupKind,
    pub(crate) level: usize,
    pub(crate) entered_line: u32,
    pub(crate) context: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShowGroupsDiagnostic {
    pub(crate) frames: Vec<ShowGroupFrame>,
}

pub(crate) fn render_showgroups(diagnostic: &ShowGroupsDiagnostic) -> String {
    let mut text = String::from("\n");
    for frame in diagnostic.frames.iter().rev() {
        text.push_str("\n### ");
        text.push_str(group_kind_text(frame.kind));
        text.push_str(" (level ");
        text.push_str(&frame.level.to_string());
        text.push(')');
        if frame.entered_line != 0 {
            text.push_str(" entered at line ");
            text.push_str(&frame.entered_line.to_string());
        }
        text.push_str(" (");
        text.push_str(&frame.context);
        text.push(')');
    }
    text.push_str("\n### bottom level");
    text
}

/// Emits e-TeX 2.6 [49.1292]'s `show_save_groups` display through the shared
/// §245 diagnostic selector, followed by §1293's ordinary show completion.
pub(crate) fn execute_canonical_showgroups(
    stores: &mut Universe,
    diagnostic: &ShowGroupsDiagnostic,
    context: String,
) -> Result<(), ExecError> {
    {
        let mut output = stores.begin_diagnostic();
        output.print_nl("").print_ln();
        for frame in diagnostic.frames.iter().rev() {
            output
                .print_nl("### ")
                .print(group_kind_text(frame.kind))
                .print(" (level ")
                .print_int(i32::try_from(frame.level).unwrap_or(i32::MAX))
                .print_char(')');
            if frame.entered_line != 0 {
                output
                    .print(" entered at line ")
                    .print_int(i32::try_from(frame.entered_line).unwrap_or(i32::MAX));
            }
            output.print(" (").print(&frame.context).print_char(')');
        }
        output.print_nl("### bottom level");
        output.end(true);
    }
    complete_show(stores, true, Some(context))?;
    Ok(())
}

pub(crate) fn execute_showifs(input: &InputStack, stores: &mut Universe) {
    let conditions = input.conditions().collect::<Vec<_>>();
    let mut text = String::new();
    text.push('\n');
    for (index, condition) in conditions.iter().enumerate().rev() {
        text.push_str("### level ");
        text.push_str(&(index + 1).to_string());
        text.push_str(": ");
        if condition.inverted() {
            text.push_str("\\unless");
        }
        text.push_str(if_type_text(condition.if_type()));
        text.push('\n');
    }
    text.push_str("\n! OK.\n");
    write_diagnostic(stores, &text);
}

pub(crate) fn group_kind_text(kind: tex_state::GroupKind) -> &'static str {
    kind.group_text()
}

fn if_type_text(if_type: u8) -> &'static str {
    match if_type {
        1 => "\\if",
        2 => "\\ifcat",
        3 => "\\ifnum",
        4 => "\\ifdim",
        5 => "\\ifodd",
        6 => "\\ifvmode",
        7 => "\\ifhmode",
        8 => "\\ifmmode",
        9 => "\\ifinner",
        10 => "\\ifvoid",
        11 => "\\ifhbox",
        12 => "\\ifvbox",
        13 => "\\ifx",
        14 => "\\ifeof",
        15 => "\\iftrue",
        16 => "\\iffalse",
        17 => "\\ifcase",
        18 => "\\ifdefined",
        19 => "\\ifcsname",
        20 => "\\iffontchar",
        _ => "\\if",
    }
}

pub(crate) fn execute_showbox(
    stores: &mut Universe,
    index: u16,
    context: String,
) -> Result<(), ExecError> {
    // TeX82 §1296's `<Show the current contents of a box>`: `begin_diagnostic`
    // and `print_nl("> \box"); print_int; print_char("=")`, then `show_box`
    // or `"void"`.
    let mut text = format!("> \\box{index}=\n");
    if let Some(id) = stores.box_reg(index) {
        text.push_str(&dump_node_list(stores, id, DumpConfig::read(stores)));
    } else {
        text.push_str("void\n");
    }
    let mut diagnostic = stores.begin_diagnostic();
    // A single smart newline, not an unconditional one: `show_box`'s own
    // open is `print_nl("> \box")`, unlike `show_activities`/`show_ifs`'s
    // `print_nl(""); print_ln`.
    diagnostic.print_nl("").print_rendered(&text);
    diagnostic.end(true);
    complete_show(stores, true, Some(context))?;
    Ok(())
}

/// TeX82 §1298's `<Complete a potentially long \show command>` followed by
/// §1293's `common_ending`.
///
/// Every `\show` family member ends here. `long` selects §1298, which only
/// the two `begin_diagnostic` forms (`\showbox`, `\showlists`) fall through
/// to; `\show` and `\showthe` `goto common_ending` and skip it.
pub(crate) fn complete_show(
    stores: &mut Universe,
    long: bool,
    context: Option<String>,
) -> Result<(), ExecError> {
    let tracing_online = stores.int_param(tex_state::env::banks::IntParam::TRACING_ONLINE);
    let interactive = stores.interaction_mode() == tex_state::InteractionMode::ErrorStop;
    if !interactive {
        // §1293's `decr(error_count)`, undoing §82's own increment so that
        // showing something never counts toward the 100-error limit.
        stores.world_mut().error_channel_mut().clear_error_count();
    }
    let mut report = if long {
        stores.print_err("OK")
    } else {
        stores.error_report()
    };
    if long && report.selector() == Selector::TermAndLog && tracing_online <= 0 {
        // §1298's remaining half: `if selector=term_and_log then if
        // tracing_online<=0 then begin selector:=term_only;
        // print(" (see the transcript file)"); selector:=term_and_log; end`.
        // The dump above went through `begin_diagnostic`'s own redirect to
        // `log_only` under this exact condition, so the terminal never saw
        // it; this note, printed to the terminal alone, is what tells the
        // user where it went.
        report.set_selector(Selector::TermOnly);
        report.print(" (see the transcript file)");
        report.set_selector(Selector::TermAndLog);
    }
    if let Some(context) = context {
        // TeX82 §1293's common ending calls §82 `error`, which always calls
        // `show_context` before either prompting or scrolling. The command
        // core captures this while its input cursor is still live.
        report.context(context);
    }
    if !interactive {
        report.help(&[]);
    } else if tracing_online > 0 {
        report.help(&[
            "This isn't an error message; I'm just \\showing something.",
            "Type `I\\show...' to show more (e.g., \\show\\cs,",
            "\\showthe\\count10, \\showbox255, \\showlists).",
        ]);
    } else {
        report.help(&[
            "This isn't an error message; I'm just \\showing something.",
            "Type `I\\show...' to show more (e.g., \\show\\cs,",
            "\\showthe\\count10, \\showbox255, \\showlists).",
            "And type `I\\tracingonline=1\\show...' to show boxes and",
            "lists on your terminal as well as in the transcript file.",
        ]);
    }
    report.error().jump_out()?;
    Ok(())
}

pub(crate) fn execute_message(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    error: bool,
) -> Result<(), ExecError> {
    let tokens = scan_balanced_expanded_text(context, input, stores, execution)?;
    let text = print_text_with_newlinechar(stores, &message_tokens_text(stores, &tokens));
    if error {
        write_diagnostic(stores, &format!("\n! {text}.\n"));
    } else {
        // TeX82 §1279: break before the message when it cannot fit on the
        // rest of the line, otherwise separate it with a space. The message
        // itself goes out through §59's `slow_print`, whose per-character
        // §58 wrapping `World::write_text` now performs for every printable
        // sink, so this decides only the leading break or space.
        let column = diagnostic_print_column(stores);
        let mut output = String::new();
        if column + text.chars().count() > tex_state::print::MAX_PRINT_LINE - 2 {
            output.push('\n');
        } else if column > 0 {
            output.push(' ');
        }
        output.push_str(&text);
        write_diagnostic(stores, &output);
    }
    Ok(())
}

pub(crate) fn execute_showlists(
    stores: &mut Universe,
    nest: &ModeNest,
    context: String,
) -> Result<(), ExecError> {
    let mut text = String::new();
    let summary = nest.summary();
    for (index, level) in summary.levels().iter().enumerate().rev() {
        text.push_str("### ");
        text.push_str(mode_text(level.mode()));
        text.push_str(" mode entered at line 0\n");
        if index == 0 && level.mode() == Mode::Vertical {
            if stores.current_page_len() != 0 {
                let current_page = stores.current_page_nodes();
                text.push_str("### current page:\n");
                text.push_str(&dump_node_slice(
                    stores,
                    &current_page,
                    DumpConfig::read(stores),
                ));
                if stores.page_contents() != PageContents::Empty {
                    text.push_str("total height ");
                    push_page_totals(stores, &mut text);
                    text.push_str("\ngoal height ");
                    text.push_str(&crate::node_dump::format_scaled_for_diagnostics(
                        stores.page_dimension(PageDimension::Goal),
                    ));
                    text.push('\n');
                }
            }
            if !stores.page_contributions().is_empty() {
                text.push_str("### recent contributions:\n");
                let contributions: Vec<_> = stores.page_contributions().iter().cloned().collect();
                text.push_str(&dump_node_slice(
                    stores,
                    &contributions,
                    DumpConfig::read(stores),
                ));
            }
        } else if !level.list().nodes().is_empty() {
            if index == 0 {
                text.push_str("### recent contributions:\n");
            }
            text.push_str(&dump_node_slice(
                stores,
                level.list().nodes(),
                DumpConfig::read(stores),
            ));
        }
        match level.mode() {
            Mode::Vertical | Mode::InternalVertical => {
                text.push_str("prevdepth ");
                match level.list().prev_depth() {
                    Some(depth) if depth.raw() > ignored_depth(stores).raw() => {
                        text.push_str(&crate::node_dump::format_scaled_for_diagnostics(depth));
                    }
                    _ => text.push_str("ignored"),
                }
                if level.list().prev_graf() != 0 {
                    text.push_str(", prevgraf ");
                    text.push_str(&level.list().prev_graf().to_string());
                    text.push_str(" line");
                    if level.list().prev_graf() != 1 {
                        text.push('s');
                    }
                }
                text.push('\n');
            }
            Mode::Horizontal | Mode::RestrictedHorizontal => {
                text.push_str("spacefactor ");
                text.push_str(&level.list().raw_space_factor().to_string());
                text.push('\n');
            }
            Mode::Math | Mode::DisplayMath => {}
        }
    }
    // §218's `show_activities` opens with `print_nl(""); print_ln`, not the
    // single smart `print_nl` `show_box` uses: the forced blank line is why
    // `\showlists`, unlike `\showbox`, always separates its dump from
    // whatever the terminal/log column held before it ran.
    let mut diagnostic = stores.begin_diagnostic();
    diagnostic.print_nl("").print_ln();
    diagnostic.print_rendered(&text);
    diagnostic.end(true);
    complete_show(stores, true, Some(context))?;
    Ok(())
}

fn push_page_totals(stores: &Universe, text: &mut String) {
    text.push_str(&crate::node_dump::format_scaled_for_diagnostics(
        stores.page_dimension(PageDimension::Total),
    ));
    for (dimension, suffix) in [
        (PageDimension::Stretch, ""),
        (PageDimension::FilStretch, "fil"),
        (PageDimension::FillStretch, "fill"),
        (PageDimension::FilllStretch, "filll"),
    ] {
        let value = stores.page_dimension(dimension);
        if value.raw() != 0 {
            text.push_str(" plus ");
            text.push_str(&crate::node_dump::format_scaled_for_diagnostics(value));
            text.push_str(suffix);
        }
    }
    let shrink = stores.page_dimension(PageDimension::Shrink);
    if shrink.raw() != 0 {
        text.push_str(" minus ");
        text.push_str(&crate::node_dump::format_scaled_for_diagnostics(shrink));
    }
}

fn mode_text(mode: Mode) -> &'static str {
    match mode {
        Mode::Vertical => "vertical",
        Mode::InternalVertical => "internal vertical",
        Mode::Horizontal => "horizontal",
        Mode::RestrictedHorizontal => "restricted horizontal",
        Mode::Math => "math",
        Mode::DisplayMath => "display math",
    }
}

pub(crate) fn report_dimension_diagnostic(stores: &mut Universe, diagnostic: DimensionDiagnostic) {
    match diagnostic {
        DimensionDiagnostic::IllegalMagnification { attempted } => {
            write_diagnostic(stores, &format!("\n! {diagnostic} ({attempted}).\n"))
        }
        DimensionDiagnostic::MissingNumber
        | DimensionDiagnostic::IllegalUnit { .. }
        | DimensionDiagnostic::IncompatibleGlueUnits
        | DimensionDiagnostic::TooLarge
        | DimensionDiagnostic::IncompatibleMagnification { .. } => {
            write_diagnostic(stores, &format!("\n! {diagnostic}.\n"));
        }
    }
}

pub(crate) fn report_integer_diagnostic(stores: &mut Universe, diagnostic: IntegerDiagnostic) {
    write_diagnostic(stores, &format!("\n! {diagnostic}.\n"));
}

pub(crate) fn report_dimension_diagnostics(
    stores: &mut Universe,
    diagnostics: impl IntoIterator<Item = DimensionDiagnostic>,
) {
    for diagnostic in diagnostics {
        report_dimension_diagnostic(stores, diagnostic);
    }
}

/// TeX82 §1004's `<Update the current page measurements with respect to the
/// glue or kern specified by node p>`.
pub(crate) fn report_page_infinite_shrinkage(stores: &mut Universe) -> Result<(), ExecError> {
    // The page builder runs between commands, so §82's display comes from the
    // published summary rather than a live stack the caller could hand over.
    let context = show_context(stores, stores.input_summary());
    crate::error_report::report_error(
        stores,
        "Infinite glue shrinkage found on current page",
        &[
            "The page about to be output contains some infinitely",
            "shrinkable glue, e.g., `\\vss' or `\\vskip 0pt minus 1fil'.",
            "Such glue doesn't belong there; but you can safely proceed,",
            "since the offensive shrinkability has been made finite.",
        ],
        context,
    )?;
    Ok(())
}

/// TeX82 §825's once-per-paragraph infinite-shrink recovery.
pub(crate) fn report_paragraph_infinite_shrinkage(stores: &mut Universe) -> Result<(), ExecError> {
    let context = show_context(stores, stores.input_summary());
    crate::error_report::report_error(
        stores,
        "Infinite glue shrinkage found in a paragraph",
        &[
            "The paragraph just ended includes some glue that has",
            "infinite shrinkability, e.g., `\\hskip 0pt minus 1fil'.",
            "Such glue doesn't belong there---it allows a paragraph",
            "of any length to fit on one line. But it's safe to proceed,",
            "since the offensive shrinkability has been made finite.",
        ],
        context,
    )?;
    Ok(())
}

/// TeX82 §976's `<Update the current height and depth measurements with
/// respect to a glue or kern node p>`.
pub(crate) fn report_split_infinite_shrinkage(stores: &mut Universe) -> Result<(), ExecError> {
    if stores.int_param(IntParam::IGNORE_PRIMITIVE_ERROR) & 1 != 0 {
        write_diagnostic(
            stores,
            "\nignored error: Infinite glue shrinkage found in box being split\n",
        );
        return Ok(());
    }
    let context = show_context(stores, stores.input_summary());
    crate::error_report::report_error(
        stores,
        "Infinite glue shrinkage found in box being split",
        &[
            "The box you are \\vsplitting contains some infinitely",
            "shrinkable glue, e.g., `\\vss' or `\\vskip 0pt minus 1fil'.",
            "Such glue doesn't belong there; but you can safely proceed,",
            "since the offensive shrinkability has been made finite.",
        ],
        context,
    )?;
    Ok(())
}

/// TeX82 §1009's `<Subtract the natural width of the insertion ...>`.
pub(crate) fn report_insertion_skip_infinite_shrinkage(
    stores: &mut Universe,
    class: u16,
) -> Result<(), ExecError> {
    let context = show_context(stores, stores.input_summary());
    crate::error_report::report_error(
        stores,
        &format!("Infinite glue shrinkage inserted from \\skip{class}"),
        &[
            "The correction glue for page breaking with insertions",
            "must have finite shrinkability. But you may proceed,",
            "since the offensive shrinkability has been made finite.",
        ],
        context,
    )?;
    Ok(())
}

pub(crate) fn execute_change_case(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    uppercase: bool,
) -> Result<(), ExecError> {
    let mut tokens = scan_balanced_raw_text(
        input,
        stores,
        execution,
        if uppercase {
            "\\uppercase"
        } else {
            "\\lowercase"
        },
    )?;
    for token in &mut tokens {
        let Token::Char { ch, .. } = token else {
            continue;
        };
        let mapped = if uppercase {
            stores.uccode(*ch)
        } else {
            stores.lccode(*ch)
        };
        if let Some(mapped) = char::from_u32(mapped).filter(|&mapped| mapped != '\0') {
            *ch = mapped;
        }
    }
    push_tokens(input, stores, tokens);
    Ok(())
}

pub(crate) fn execute_ignorespaces(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    loop {
        // TeX82's `any_mode(ignore_spaces)` branch calls `get_x_token`, so a
        // macro that initially expands to spaces is consumed through its first
        // non-space token. Reading raw input here changes tabular widths and
        // other layout whenever the whitespace comes from a macro argument.
        let Some(token) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        else {
            return Ok(());
        };
        if !is_space(tex_expand::semantic_token(token)) {
            push_traced_tokens(input, stores, [token]);
            return Ok(());
        }
    }
}

fn show_meaning_text(stores: &Universe, token: Token) -> String {
    let text = meaning_text(stores, token);
    if let Some((prefix, rest)) = text.split_once("macro:") {
        format!("{prefix}macro:\n{rest}")
    } else {
        text
    }
}

fn scan_balanced_raw_text(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: &'static str,
) -> Result<Vec<Token>, ExecError> {
    let open =
        next_non_space_x(input, stores, execution)?.ok_or(ExecError::MissingToken { context })?;
    if !is_begin_group(open) {
        return Err(ExecError::MissingToken { context });
    }
    let mut depth = 1usize;
    let mut tokens = Vec::new();
    while let Some(traced) =
        tex_expand::next_semantic_raw_token(input, &mut tex_state::ExpansionContext::new(stores))?
    {
        let token = tex_expand::semantic_token(traced);
        if is_begin_group(token) {
            depth += 1;
            tokens.push(token);
        } else if is_end_group(token) {
            depth -= 1;
            if depth == 0 {
                return Ok(tokens);
            }
            tokens.push(token);
        } else {
            tokens.push(token);
        }
    }
    Err(ExecError::MissingToken { context })
}

fn scan_balanced_expanded_text(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Vec<Token>, ExecError> {
    let token_list = tex_expand::scan::scan_general_text_expanded_with_driver(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
        context,
    )?;
    Ok(stores.tokens(token_list).to_vec())
}

fn next_non_space_x(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<Token>, ExecError> {
    while let Some(token) = get_x_token_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
    )?
    .map(tex_expand::semantic_token)
    {
        if !is_space(token) {
            return Ok(Some(token));
        }
    }
    Ok(None)
}

fn tokens_text(stores: &Universe, tokens: &[Token]) -> String {
    let mut text = String::new();
    for &token in tokens {
        append_token_show_text(stores, token, &mut text);
    }
    text
}

fn message_tokens_text(stores: &Universe, tokens: &[Token]) -> String {
    let mut text = String::new();
    for &token in tokens {
        if let Token::Char { ch, .. } = token {
            text.push(ch);
        } else {
            tex_expand::append_token_string_text(stores, token, &mut text);
        }
    }
    text
}

/// Appends TeX82's printable token form, including the separator that
/// `print_cs` emits after a control word.
pub(crate) fn append_token_show_text(stores: &Universe, token: Token, text: &mut String) {
    tex_expand::append_token_show_text(stores, token, text);
}

/// tex.web §310's `show_context` display for the gullet's replay stack.
///
/// The implementation is [`tex_state::InputSummary::show_context`]; the
/// pseudoprint arithmetic it shares with the canonical command core's own
/// stack is [`tex_state::print::render_error_context`].
pub(crate) fn show_context(stores: &Universe, input: &tex_state::InputSummary) -> String {
    input.show_context(stores)
}

fn diagnostic_print_column(stores: &Universe) -> usize {
    stores
        .world()
        .stream_bufs()
        .terminal_partial_line()
        .chars()
        .count()
}

pub(crate) fn print_text_with_newlinechar(stores: &Universe, text: &str) -> String {
    let newlinechar = stores.int_param(IntParam::NEWLINE_CHAR);
    let Some(newline) = u32::try_from(newlinechar)
        .ok()
        .filter(|&code| code <= u8::MAX.into())
        .and_then(char::from_u32)
    else {
        return text.to_owned();
    };
    text.chars()
        .map(|ch| if ch == newline { '\n' } else { ch })
        .collect()
}

fn write_diagnostic(stores: &mut Universe, text: &str) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, text);
}

fn is_begin_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    )
}

fn is_end_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    )
}

fn is_space(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::Space,
            ..
        }
    )
}

/// web2c's `[53.1374]` change to tex.web: a successful `\openout` announces
/// the file it opened.
///
/// ```text
/// if log_opened and texmf_yesno('log_openout') then begin
///   old_setting:=selector;
///   if (tracing_online<=0) then selector:=log_only
///   else selector:=term_and_log;
///   print_nl("\openout"); print_int(j); print(" = `");
///   print_file_name(cur_name,cur_area,cur_ext); print("'.");
///   print_nl(""); print_ln;
///   selector:=old_setting;
/// end;
/// ```
///
/// The name is a literal backslash in the WEB string, not `print_esc`, so it
/// does not follow `\escapechar`. The closing `print_nl("")` plus `print_ln`
/// is what leaves the blank line the reference log shows after the notice.
///
/// Neither guard survives into Umber. `log_opened` is constantly true here --
/// see `tex_state::print`'s module documentation -- and `log_openout` is a
/// `texmf.cnf` knob whose distributed value is `t`, which is the setting the
/// pinned oracle logs were captured under.
///
/// The whole notice is written as one record rather than one per `print`
/// call: it is a fixed announcement with no interleaving, and the stream
/// tests that assert on `World::effect_records` are about stream
/// transitions, not about how many `print` calls compose a fixed line.
pub(crate) fn report_openout(stores: &mut tex_state::Universe, stream: u8, path: &str) {
    let terminal = stores.int_param(tex_state::env::banks::IntParam::TRACING_ONLINE) > 0;
    let sink = if terminal {
        tex_state::PrintSink::TerminalAndLog
    } else {
        tex_state::PrintSink::Log
    };
    let bufs = stores.world().stream_bufs();
    // §62's `print_nl` guard for the selector this notice installs.
    let line_is_open = !bufs.log_partial_line().is_empty()
        || (terminal && !bufs.terminal_partial_line().is_empty());
    let mut text = String::new();
    if line_is_open {
        text.push('\n');
    }
    let _ = write!(text, "\\openout{stream} = `{path}'.\n\n");
    stores.world_mut().write_text(sink, &text);
}
