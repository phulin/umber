//! Source-free diagnostic reporting and rendering.

use std::fmt::Write as _;

use tex_command::{DimensionDiagnostic, FatalError};
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::IntParam;
use tex_state::page::{PageContents, PageDimension, PageInsertion, PageInsertionStatus};
use tex_state::print::Selector;
use tex_state::token::{Catcode, Token};
use tex_state::{CommandContext, PrintSink, Universe};

use crate::mode::ignored_depth;
use crate::node_dump::{DumpConfig, dump_node_slice};

#[cfg(test)]
mod tests;

/// Detached command-owned values needed by execution-time diagnostics.
///
/// Execution barriers populate this from command and mode state. Hot kernels
/// carry the value beside their admitted [`CommandContext`] and never recover
/// input, source, or mode ownership while reporting.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExecutionDiagnosticContext {
    pub(crate) current_line: i32,
    pub(crate) pack_begin_line: i32,
    pub(crate) output_routine_active: bool,
    pub(crate) output_context: String,
}

impl ExecutionDiagnosticContext {
    pub(crate) fn new(
        current_line: i32,
        pack_begin_line: i32,
        output_routine_active: bool,
        output_context: impl Into<String>,
    ) -> Self {
        Self {
            current_line,
            pack_begin_line,
            output_routine_active,
            output_context: output_context.into(),
        }
    }

    pub(crate) fn source_free(output_context: impl Into<String>) -> Self {
        Self {
            output_context: output_context.into(),
            ..Self::default()
        }
    }

    pub(crate) fn with_pack_begin_line(&self, pack_begin_line: i32) -> Self {
        Self {
            pack_begin_line,
            ..self.clone()
        }
    }
}

/// Renders TeX82 §§94--95's irrecoverable reports before §93 `succumb`.
///
/// The other `FatalError` variants already arrive after their output has been
/// emitted: §82's hundred-error branch prints its own notice, §93
/// `fatal_error` composes `Emergency stop`, and §84's `X` deliberately
/// prints nothing. Capacity and consistency failures instead originate as
/// typed errors below main control, so the one `jump_out` boundary must
/// compose their report while the triggering command's input context is
/// still live.
pub(crate) fn report_irrecoverable_error<G>(
    stores: &mut Universe<G>,
    fatal: FatalError,
    context: String,
) {
    let mut report = match fatal {
        FatalError::CapacityExceeded { resource, amount } => {
            let mut report = stores.print_err("TeX capacity exceeded, sorry [");
            report
                .print(resource)
                .print_char('=')
                .print_int(amount)
                .print_char(']')
                .help(&[
                    "If you really absolutely need more capacity,",
                    "you can ask a wizard to enlarge me.",
                ]);
            report
        }
        FatalError::Confusion { site } => {
            if stores.world().error_channel().history()
                < tex_state::print::ErrorHistory::ErrorMessageIssued
            {
                let mut report = stores.print_err("This can't happen (");
                report
                    .print(site)
                    .print_char(')')
                    .help(&["I'm broken. Please show this to someone who can fix can fix"]);
                report
            } else {
                let mut report = stores.print_err("I can't go on meeting you like this");
                report.help(&[
                    "One of your faux pas seems to have wounded me deeply...",
                    "in fact, I'm barely conscious. Please fix it and try again.",
                ]);
                report
            }
        }
        FatalError::TooManyErrors | FatalError::EmergencyStop { .. } | FatalError::Quit => return,
    };
    report.context(context);
    report.succumb();
}

/// e-TeX's `\interactionmode` case of TeX82 §1243's `alter_integer`.
///
/// The parenthesized value is §91's `int_error`, which prints it as part of
/// the message line rather than as a second report.
/// Reports a bad interaction mode with the command processor's live input
/// context carried across the scan/apply boundary.
pub(crate) fn report_bad_interaction_mode_with_context<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    value: i32,
    context: String,
) -> Result<(), ExecError> {
    crate::error_report::report_error(
        stores,
        diagnostic_effects,
        &format!("Bad interaction mode ({value})"),
        &[
            "Modes are 0=batch, 1=nonstop, 2=scroll, and",
            "3=errorstop. Proceed, and I'll ignore this case.",
        ],
        context,
    )?;
    Ok(())
}

/// TeX82 §581's missing-character warning, including e-TeX 2.6 change
/// section 17.516's level-two terminal routing.
pub(crate) fn report_missing_character_warning<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    font: tex_state::ids::FontId,
    ch: char,
    etex_extended: bool,
) {
    if stores.int_param(tex_state::env::banks::IntParam::TRACING_LOST_CHARS) <= 0 {
        return;
    }
    // TeX82 §581's `char_warning` prints `font_name[f]` directly.  Unlike
    // `\fontname`, that stored external name never gains an `at <size>pt`
    // suffix when the font was loaded away from its design size.
    let font_name = stores.font_external_name(font).to_owned();
    let force_online =
        etex_extended && stores.int_param(tex_state::env::banks::IntParam::TRACING_LOST_CHARS) > 1;
    let mut diagnostic = if force_online {
        stores.begin_online_diagnostic(diagnostic_effects)
    } else {
        stores.begin_diagnostic(diagnostic_effects)
    };
    diagnostic
        .print_nl("Missing character: There is no ")
        .print_ascii(ch)
        .print(" in font ")
        .print(&font_name)
        .print_char('!');
    diagnostic.end(false);
}

/// TeX82 §1049's `you_cant` message followed by §1050's `report_illegal_case`.
pub(crate) fn report_illegal_case_with_context<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    token: Token,
    mode: Mode,
    context: Option<String>,
) -> Result<(), ExecError> {
    report_illegal_case(stores, diagnostic_effects, token, mode, context, None)
}

/// The same illegal-case report with a command-neutral site captured while
/// the delivered command was still live. Cold forbidden cases such as a
/// standalone `\badness` retain this site because their typed operation no
/// longer owns the original `CurrentCommand` at application time.
pub(crate) fn report_illegal_case_with_site<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    token: Token,
    mode: Mode,
    context: Option<String>,
    site: Option<tex_state::diagnostic::DiagnosticSite>,
) -> Result<(), ExecError> {
    report_illegal_case(stores, diagnostic_effects, token, mode, context, site)
}

fn report_illegal_case<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    token: Token,
    mode: Mode,
    context: Option<String>,
    site: Option<tex_state::diagnostic::DiagnosticSite>,
) -> Result<(), ExecError> {
    let command = tex_command::command_token_text(stores, token);
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
    Ok(report.error_and_defer_at(diagnostic_effects, site)?)
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
use crate::ExecError;
use crate::{Mode, ModeNest};

/// TeX82 §370's `Complain about an undefined macro` report.
///
/// §370 reaches §82 with the offending control sequence still the last thing
/// read, so its context display ends the top line with it -- which is what
/// the help text means by "the control sequence at the end of the top line".
/// A caller that cannot supply that display passes `None` rather than an
/// empty string, so the report omits the context instead of printing a blank
/// one.
pub(crate) fn report_undefined_control_sequence<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    context: Option<String>,
    site: Option<tex_state::diagnostic::DiagnosticSite>,
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
    report.error_and_defer_at(diagnostic_effects, site)?;
    Ok(())
}

/// TeX82 §1128's no-alignment-in-progress branch of `align_error`.
pub(crate) fn report_misplaced_alignment_delimiter<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    token: Token,
    context: Option<String>,
) -> Result<(), ExecError> {
    let delimiter = match token {
        // A category-5 character reaches §1128 as the character command
        // delivered by §1126, not as either control-sequence spelling that
        // shares `car_ret`'s command code.
        Token::Char {
            ch,
            cat: Catcode::EndLine,
        } => format!("end of line character {ch}"),
        _ => tex_command::command_token_text(stores, token),
    };
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
    report.error().defer_recovery(diagnostic_effects)?;
    Ok(())
}

/// TeX82 §1129's misplaced `\noalign` and `\omit` diagnostics.
pub(crate) fn report_misplaced_alignment_command<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    name: &str,
    help: &[&str],
    context: Option<String>,
) -> Result<(), ExecError> {
    let mut report = stores.print_err("Misplaced ");
    report.print_esc(name).help(help);
    if let Some(context) = context {
        report.context(context);
    }
    report.error().defer_recovery(diagnostic_effects)?;
    Ok(())
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
pub(crate) fn execute_showgroups<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    diagnostic: &ShowGroupsDiagnostic,
) {
    {
        let mut output = stores.begin_diagnostic(diagnostic_effects);
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
}

pub(crate) fn group_kind_text(kind: tex_state::GroupKind) -> &'static str {
    kind.group_text()
}

pub(crate) fn execute_showbox<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    index: u16,
    profile: tex_command::CommandProfile,
) {
    // TeX82 §1296's `<Show the current contents of a box>`: `begin_diagnostic`
    // and `print_nl("> \box"); print_int; print_char("=")`, then `show_box`
    // or `"void"`.
    let mut text = format!("> \\box{index}=");
    if let Some(owner) = stores.copy_box_to_page(index) {
        // TeX82 §198's `show_box` enters `show_node_list`, whose first
        // visible node is opened by `print_ln`; the structural break belongs
        // only to this branch. Section 1296 prints `"void"` directly after
        // the equals sign when the register is null.
        text.push('\n');
        text.push_str(&crate::node_dump::dump_page_list(
            stores,
            owner,
            DumpConfig::read(stores).for_profile(profile),
        ));
    } else {
        text.push_str("void\n");
    }
    let mut diagnostic = stores.begin_diagnostic(diagnostic_effects);
    // A single smart newline, not an unconditional one: `show_box`'s own
    // open is `print_nl("> \box")`, unlike `show_activities`/`show_ifs`'s
    // `print_nl(""); print_ln`.
    diagnostic.print_nl("").print_rendered(&text);
    diagnostic.end(true);
}

/// TeX82 §1298's `<Complete a potentially long \show command>` followed by
/// §1293's `common_ending`.
///
/// Every `\show` family member ends here. `long` selects §1298, which only
/// the two `begin_diagnostic` forms (`\showbox`, `\showlists`) fall through
/// to; `\show` and `\showthe` `goto common_ending` and skip it.
pub(crate) fn complete_show<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    long: bool,
    context: Option<String>,
) -> Result<(), ExecError> {
    let tracing_online = stores.int_param(tex_state::env::banks::IntParam::TRACING_ONLINE);
    let interactive = stores.interaction_mode_value() == 3;
    if !interactive {
        // §1293's `decr(error_count)`, undoing §82's own increment so that
        // showing something never counts toward the 100-error limit.
        stores.clear_error_count();
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
    report.error().defer_recovery(diagnostic_effects)?;
    Ok(())
}

pub(crate) fn execute_showlists<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    nest: &ModeNest,
    profile: tex_command::CommandProfile,
) -> Result<(), ExecError> {
    let mut text = String::new();
    let levels = nest.levels();
    let output_routine_active = levels.iter().any(|level| level.entry_line() < 0);
    let page = page_activity_snapshot(stores, output_routine_active)?;
    let ignored_depth = ignored_depth(stores);
    for (index, level) in levels.iter().enumerate().rev() {
        text.push_str("### ");
        text.push_str(mode_text(level.mode()));
        text.push_str(" mode entered at line ");
        text.push_str(&level.entry_line().unsigned_abs().to_string());
        if level.entry_line() < 0 {
            text.push_str(" (\\output routine)");
        } else if matches!(level.mode(), Mode::Horizontal | Mode::RestrictedHorizontal)
            && (level.list().hyphen_language() != 0
                || level.list().left_hyphen_min() != 0
                || level.list().right_hyphen_min() != 0)
        {
            text.push_str(" (language");
            text.push_str(&level.list().hyphen_language().to_string());
            text.push_str(":hyphenmin");
            text.push_str(&level.list().left_hyphen_min().to_string());
            text.push(',');
            text.push_str(&level.list().right_hyphen_min().to_string());
            text.push(')');
        }
        text.push('\n');
        if index == 0 && level.mode() == Mode::Vertical {
            if !page.current_page.is_empty() {
                text.push_str("### current page:");
                // TeX82 §218 distinguishes the page retained while §1025's
                // output routine is active from the ordinary current page.
                if output_routine_active {
                    text.push_str(" (held over for next output)");
                }
                text.push('\n');
                text.push_str(&dump_node_slice(
                    stores,
                    &page.current_page,
                    DumpConfig::read(stores).for_profile(profile),
                ));
                if page.contents != PageContents::Empty {
                    text.push_str("total height ");
                    push_page_totals(&page, &mut text);
                    // TeX82 §218 uses `print_nl(" goal height ")`; the
                    // leading space is part of the diagnostic text.
                    text.push_str("\n goal height ");
                    text.push_str(&crate::node_dump::format_scaled_for_diagnostics(page.goal));
                    text.push('\n');
                    push_page_insertions(&page.insertions, &page.current_page, &mut text)?;
                }
            }
            if !page.contributions.is_empty() {
                text.push_str("### recent contributions:\n");
                text.push_str(&dump_node_slice(
                    stores,
                    &page.contributions,
                    DumpConfig::read(stores).for_profile(profile),
                ));
            }
        } else if let Some(nodes) = showlists_level_nodes(stores, levels, index) {
            if index == 0 {
                text.push_str("### recent contributions:\n");
            }
            let config = DumpConfig::read(stores).for_profile(profile);
            match nodes {
                ShowlistsNodes::Page(list) => {
                    text.push_str(&crate::node_dump::dump_page_list(stores, list, config));
                }
                ShowlistsNodes::Mode(nodes) => {
                    text.push_str(&crate::node_dump::dump_node_sequence_view(
                        stores, nodes, config,
                    ));
                }
            }
        }
        match level.mode() {
            Mode::Vertical | Mode::InternalVertical => {
                text.push_str("prevdepth ");
                match level.list().prev_depth() {
                    Some(depth) if depth.raw() > ignored_depth.raw() => {
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
                if level.mode() == Mode::Horizontal {
                    text.push_str(", current language ");
                    text.push_str(&level.list().hyphen_language().to_string());
                }
                text.push('\n');
            }
            Mode::Math | Mode::DisplayMath => {
                if let Some(fraction) = level.list().incomplete_fraction() {
                    text.push_str("this will begin denominator of:\n");
                    text.push_str(&crate::node_dump::dump_incomplete_fraction(
                        stores,
                        fraction,
                        DumpConfig::read(stores).for_profile(profile),
                    ));
                }
            }
        }
    }
    // §218's `show_activities` opens with `print_nl(""); print_ln`, not the
    // single smart `print_nl` `show_box` uses: the forced blank line is why
    // `\showlists`, unlike `\showbox`, always separates its dump from
    // whatever the terminal/log column held before it ran.
    let mut diagnostic = stores.begin_diagnostic(diagnostic_effects);
    diagnostic.print_nl("").print_ln();
    diagnostic.print_rendered(&text);
    diagnostic.end(true);
    Ok(())
}

struct PageActivitySnapshot {
    current_page: Vec<tex_state::node::Node>,
    contributions: Vec<tex_state::node::Node>,
    insertions: Vec<(PageInsertion, i32)>,
    contents: PageContents,
    goal: tex_state::scaled::Scaled,
    total: tex_state::scaled::Scaled,
    stretch: [tex_state::scaled::Scaled; 4],
    shrink: tex_state::scaled::Scaled,
}

/// Detaches the page-builder evidence before diagnostic formatting can call
/// back into the live engine. No page-arena borrow crosses the observer seam.
fn page_activity_snapshot<G>(
    stores: &CommandContext<'_, G>,
    output_routine_active: bool,
) -> Result<PageActivitySnapshot, ExecError> {
    let dimension =
        |dimension| stores.page_dimension_with_output_routine(dimension, output_routine_active);
    let insertions = stores
        .page_insertions()
        .iter()
        .map(|insertion| {
            let count = stores
                .count(insertion.class())
                .expect("page insertion class is an admitted count register");
            (insertion, count)
        })
        .collect();
    Ok(PageActivitySnapshot {
        current_page: stores
            .current_page_nodes()
            .map(|node| node.to_owned_with(std::convert::identity))
            .collect(),
        contributions: stores
            .page_contributions()
            .iter()
            .map(|node| node.to_owned_with(std::convert::identity))
            .collect(),
        insertions,
        contents: stores.page_contents(),
        goal: dimension(PageDimension::Goal),
        total: dimension(PageDimension::Total),
        stretch: [
            dimension(PageDimension::Stretch),
            dimension(PageDimension::FilStretch),
            dimension(PageDimension::FillStretch),
            dimension(PageDimension::FilllStretch),
        ],
        shrink: dimension(PageDimension::Shrink),
    })
}

/// Returns TeX82 §218's list root for one saved semantic nest level.
///
/// While §1194 scans an equation number, `fin_mlist(null)` has moved the
/// display mlist into the immediately inner math level's save record. TeX's
/// linked display-level head still roots that mlist for `show_activities`;
/// project Umber's typed `DisplayEqNo` owner back onto that level instead of
/// displaying the now-empty construction list.
enum ShowlistsNodes<'a> {
    Page(tex_state::node_arena::PageListId),
    Mode(tex_state::node_arena::NodeCursor<'a>),
}

fn showlists_level_nodes<'a, G>(
    stores: &'a CommandContext<'_, G>,
    levels: &[crate::mode::ModeLevelSummary],
    index: usize,
) -> Option<ShowlistsNodes<'a>> {
    let level = &levels[index];
    if level.mode() == Mode::DisplayMath
        && let Some(eq_no) = levels
            .get(index + 1)
            .and_then(|inner| inner.list().display_eq_no())
    {
        return Some(ShowlistsNodes::Page(eq_no.display.list()));
    }
    (!level.list().physical_nodes(stores).is_empty())
        .then(|| ShowlistsNodes::Mode(level.list().physical_nodes(stores)))
}

/// TeX82 §218's insertion-record tail of `show_activities`.
fn push_page_insertions(
    insertions: &[(PageInsertion, i32)],
    current_page: &[tex_state::node::Node],
    text: &mut String,
) -> Result<(), ExecError> {
    for (insertion, count) in insertions {
        let _ = write!(text, "\\insert{} adds ", insertion.class());
        let scaled_height = crate::page_builder::scaled_insertion_size(insertion.height(), *count)?;
        text.push_str(&crate::node_dump::format_scaled_for_diagnostics(
            scaled_height,
        ));
        if let PageInsertionStatus::SplitUp {
            broken_ins_index, ..
        } = insertion.status()
        {
            let split_count = current_page
                .iter()
                .take(broken_ins_index.saturating_add(1))
                .filter(|node| {
                    matches!(node, tex_state::node::Node::Ins { class, .. } if *class == insertion.class())
                })
                .count();
            let _ = write!(text, ", #{split_count} might split");
        }
        text.push('\n');
    }
    Ok(())
}

fn push_page_totals(page: &PageActivitySnapshot, text: &mut String) {
    text.push_str(&crate::node_dump::format_scaled_for_diagnostics(page.total));
    for (value, suffix) in page.stretch.into_iter().zip(["", "fil", "fill", "filll"]) {
        if value.raw() != 0 {
            text.push_str(" plus ");
            text.push_str(&crate::node_dump::format_scaled_for_diagnostics(value));
            text.push_str(suffix);
        }
    }
    if page.shrink.raw() != 0 {
        text.push_str(" minus ");
        text.push_str(&crate::node_dump::format_scaled_for_diagnostics(
            page.shrink,
        ));
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

pub(crate) fn report_dimension_diagnostic<G>(
    stores: &mut Universe<G>,
    diagnostic: DimensionDiagnostic,
) {
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

/// TeX82 §1004's `<Update the current page measurements with respect to the
/// glue or kern specified by node p>`.
pub(crate) fn report_page_infinite_shrinkage<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    context: &ExecutionDiagnosticContext,
) -> Result<(), ExecError> {
    // TeX82 §1004 reaches §82's `error` while handling the command that
    // contributed this glue. Synchronous page building renders from that
    // borrowed live command only after selecting this recovery; replay after
    // a real suspension/publication boundary supplies the same detached text.
    crate::error_report::report_ordered_error(
        stores,
        diagnostic_effects,
        "Infinite glue shrinkage found on current page",
        &[
            "The page about to be output contains some infinitely",
            "shrinkable glue, e.g., `\\vss' or `\\vskip 0pt minus 1fil'.",
            "Such glue doesn't belong there; but you can safely proceed,",
            "since the offensive shrinkability has been made finite.",
        ],
        context.output_context.clone(),
    )?;
    Ok(())
}

/// TeX82 §825's once-per-paragraph infinite-shrink recovery.
pub(crate) fn report_paragraph_infinite_shrinkage<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    context: &ExecutionDiagnosticContext,
) -> Result<(), ExecError> {
    crate::error_report::report_error(
        stores,
        diagnostic_effects,
        "Infinite glue shrinkage found in a paragraph",
        &[
            "The paragraph just ended includes some glue that has",
            "infinite shrinkability, e.g., `\\hskip 0pt minus 1fil'.",
            "Such glue doesn't belong there---it allows a paragraph",
            "of any length to fit on one line. But it's safe to proceed,",
            "since the offensive shrinkability has been made finite.",
        ],
        context.output_context.clone(),
    )?;
    Ok(())
}

/// TeX82 §976's `<Update the current height and depth measurements with
/// respect to a glue or kern node p>`.
pub(crate) fn report_split_infinite_shrinkage<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    context: &ExecutionDiagnosticContext,
) -> Result<(), ExecError> {
    if stores.int_param(IntParam::IGNORE_PRIMITIVE_ERROR) & 1 != 0 {
        let mut diagnostic = stores.begin_online_diagnostic(diagnostic_effects);
        diagnostic
            .print_rendered("\nignored error: Infinite glue shrinkage found in box being split");
        diagnostic.end(false);
        return Ok(());
    }
    // TeX82 §976 is shared by command-time `\vsplit` and page-builder
    // insertion splitting. Both callers render the applicable command
    // context before crossing this diagnostic boundary.
    crate::error_report::report_error(
        stores,
        diagnostic_effects,
        "Infinite glue shrinkage found in box being split",
        &[
            "The box you are \\vsplitting contains some infinitely",
            "shrinkable glue, e.g., `\\vss' or `\\vskip 0pt minus 1fil'.",
            "Such glue doesn't belong there; but you can safely proceed,",
            "since the offensive shrinkability has been made finite.",
        ],
        context.output_context.clone(),
    )?;
    Ok(())
}

/// TeX82 §1009's `<Subtract the natural width of the insertion ...>`.
pub(crate) fn report_insertion_skip_infinite_shrinkage<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    class: u16,
    context: &ExecutionDiagnosticContext,
) -> Result<(), ExecError> {
    crate::error_report::report_ordered_error(
        stores,
        diagnostic_effects,
        &format!("Infinite glue shrinkage inserted from \\skip{class}"),
        &[
            "The correction glue for page breaking with insertions",
            "must have finite shrinkability. But you may proceed,",
            "since the offensive shrinkability has been made finite.",
        ],
        context.output_context.clone(),
    )?;
    Ok(())
}

/// Appends TeX82's printable token form, including the separator that
/// `print_cs` emits after a control word.
pub(crate) fn append_token_show_text<G>(
    stores: &CommandContext<'_, G>,
    token: Token,
    text: &mut String,
) {
    stores.append_token_show_text(token, text);
}

pub(crate) fn print_text_with_newlinechar<G>(stores: &CommandContext<'_, G>, text: &str) -> String {
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

fn write_diagnostic<G>(stores: &mut Universe<G>, text: &str) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, text);
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
/// The whole notice is returned as one owned write rather than one fragment
/// per `print` call. The caller captures the three live print-state scalars
/// under admission, releases that context, and publishes this fixed record at
/// the outer effect boundary without exposing `World` to this helper.
#[must_use]
pub(crate) fn report_openout(
    tracing_online: i32,
    terminal_line_is_open: bool,
    log_line_is_open: bool,
    stream: u8,
    path: &str,
) -> (PrintSink, String) {
    let terminal = tracing_online > 0;
    let sink = if terminal {
        PrintSink::TerminalAndLog
    } else {
        PrintSink::Log
    };
    // §62's `print_nl` guard for the selector this notice installs.
    let line_is_open = log_line_is_open || (terminal && terminal_line_is_open);
    let mut text = String::new();
    if line_is_open {
        text.push('\n');
    }
    let _ = write!(text, "\\openout{stream} = `{path}'.\n\n");
    (sink, text)
}
