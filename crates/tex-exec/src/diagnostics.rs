//! Diagnostic and log-writing primitives.

use tex_expand::{
    ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks, meaning_text,
    scan_dimen::DimensionDiagnostic, scan_int::IntegerDiagnostic, scan_the_text_with_hooks,
    token_text,
};
use tex_lex::{InputSource, InputStack};
use tex_state::env::banks::IntParam;
use tex_state::page::{PageContents, PageDimension};
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::{PrintSink, Universe};

use crate::mode::IGNORE_DEPTH;
use crate::node_dump::{DumpConfig, dump_node_list, dump_node_slice};
pub(crate) fn report_illegal_case(stores: &mut Universe, token: Token, mode: Mode) {
    let command = match token {
        Token::Cs(symbol) => format!("\\{}", stores.resolve(symbol)),
        _ => format!("{token:?}"),
    };
    let mode = match mode {
        Mode::Vertical => "vertical mode",
        Mode::InternalVertical => "internal vertical mode",
        Mode::Horizontal => "horizontal mode",
        Mode::RestrictedHorizontal => "restricted horizontal mode",
        Mode::Math => "math mode",
        Mode::DisplayMath => "display math mode",
    };
    stores.world_mut().write_text(
        tex_state::PrintSink::TerminalAndLog,
        &format!(
            "\n! You can't use `{command}' in {mode}.\nSorry, but I'm not programmed to handle this case;\nI'll just pretend that you didn't ask for it.\nIf you're in the wrong mode, you might be able to\nreturn to the right one by typing `I}}' or `I$' or `I\\par'.\n"
        ),
    );
}
use crate::{ExecError, push_tokens};
use crate::{Mode, ModeNest};

pub(crate) fn execute_show<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let token = tex_expand::get_token(input, stores)?
        .ok_or(ExecError::MissingToken { context: "\\show" })?;
    let token = tex_expand::semantic_token(token);
    let text = match token {
        Token::Cs(_) => {
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

pub(crate) fn execute_showthe<S, H>(
    context: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let text = match scan_the_text_with_hooks(input, stores, &mut recorder, hooks, context) {
        Ok(text) => text,
        Err(tex_expand::ExpandError::UnsupportedTheTarget { context }) => {
            let token = tex_expand::semantic_token(context);
            let rendered = match token {
                Token::Char { ch, cat } => format!("{} character {ch}", catcode_name(cat)),
                _ => meaning_text(stores, token),
            };
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                &format!(
                    "\n! You can't use `{rendered}' after \\the.\nI'm forgetting what you said and using zero instead.\n"
                ),
            );
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

pub(crate) fn execute_showtokens<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let tokens = scan_balanced_raw_text(input, stores, "\\showtokens")?;
    write_diagnostic(stores, &format!("\n> {}.\n", tokens_text(stores, &tokens)));
    Ok(())
}

pub(crate) fn execute_showbox(stores: &mut Universe, index: u16) {
    let mut text = format!("\n> \\box{index}=\n");
    if let Some(id) = stores.box_reg(index) {
        text.push_str(&dump_node_list(stores, id, DumpConfig::read(stores)));
    } else {
        text.push_str("void\n");
    }
    text.push_str("\n! OK.\n");
    write_diagnostic(stores, &text);
}

pub(crate) fn execute_message<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    error: bool,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let tokens = scan_balanced_expanded_text(input, stores, hooks, "\\message")?;
    let text = print_text_with_newlinechar(stores, &tokens_text(stores, &tokens));
    if error {
        write_diagnostic(stores, &format!("\n! {text}.\n"));
    } else {
        let mut output = write_wrapped_message(&text);
        output.push(' ');
        write_diagnostic(stores, &output);
    }
    Ok(())
}

pub(crate) fn execute_showlists(stores: &mut Universe, nest: &ModeNest) {
    let mut text = String::new();
    text.push('\n');
    let summary = nest.summary();
    for (index, level) in summary.levels().iter().enumerate().rev() {
        text.push_str("### ");
        text.push_str(mode_text(level.mode()));
        text.push_str(" mode entered at line 0\n");
        if index == 0 && level.mode() == Mode::Vertical {
            if !stores.current_page_nodes().is_empty() {
                text.push_str("### current page:\n");
                text.push_str(&dump_node_slice(
                    stores,
                    stores.current_page_nodes(),
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
                    Some(depth) if depth.raw() > IGNORE_DEPTH.raw() => {
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
    text.push_str("\n! OK.\n");
    write_diagnostic(stores, &text);
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

pub(crate) fn execute_showhyphens<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let tokens = scan_balanced_expanded_text(input, stores, hooks, "\\showhyphens")?;
    let mut words = Vec::new();
    let mut current = String::new();
    for token in tokens {
        match token {
            Token::Char {
                ch: _,
                cat: Catcode::Space,
            } => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            Token::Char { ch, .. } => {
                if let Some(lower) = char::from_u32(stores.lccode(ch)).filter(|&ch| ch != '\0') {
                    current.push(lower);
                } else if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
        }
    }
    if !current.is_empty() {
        words.push(current);
    }

    let left = stores.int_param(IntParam::LEFT_HYPHEN_MIN).max(0) as usize;
    let right = stores.int_param(IntParam::RIGHT_HYPHEN_MIN).max(0) as usize;
    let mut lines = String::new();
    lines.push('\n');
    for word in words {
        let positions = stores.hyphen_positions(&word, left, right);
        lines.push_str(&hyphenated_word_text(&word, &positions));
        lines.push('\n');
    }
    lines.push_str("! OK.\n");
    write_diagnostic(stores, &lines);
    Ok(())
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

pub(crate) fn report_page_infinite_shrinkage(stores: &mut Universe) {
    write_diagnostic(
        stores,
        "\n! Infinite glue shrinkage found on current page.\n\
The page about to be output contains some infinitely\n\
shrinkable glue, e.g., `\\vss' or `\\vskip 0pt minus 1fil'.\n\
Such glue doesn't belong there; but you can safely proceed,\n\
since the offensive shrinkability has been made finite.\n",
    );
}

pub(crate) fn report_split_infinite_shrinkage(stores: &mut Universe) {
    write_diagnostic(
        stores,
        "\n! Infinite glue shrinkage found in box being split.\n\
The box you are \\vsplitting contains some infinitely\n\
shrinkable glue, e.g., `\\vss' or `\\vskip 0pt minus 1fil'.\n\
Such glue doesn't belong there; but you can safely proceed,\n\
since the offensive shrinkability has been made finite.\n",
    );
}

pub(crate) fn report_insertion_skip_infinite_shrinkage(stores: &mut Universe, class: u16) {
    write_diagnostic(
        stores,
        &format!(
            "\n! Infinite glue shrinkage inserted from \\skip{class}.\n\
The correction glue for page breaking with insertions\n\
must have finite shrinkability. But you may proceed,\n\
since the offensive shrinkability has been made finite.\n"
        ),
    );
}

pub(crate) fn execute_change_case<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    uppercase: bool,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let mut tokens = scan_balanced_raw_text(
        input,
        stores,
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

pub(crate) fn execute_ignorespaces<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    loop {
        let Some(token) = input.next_token(stores)? else {
            return Ok(());
        };
        if !is_space(token) {
            push_tokens(input, stores, [token]);
            return Ok(());
        }
    }
}

fn show_meaning_text(stores: &Universe, token: Token) -> String {
    let text = meaning_text(stores, token);
    if let Some(rest) = text.strip_prefix("macro:") {
        format!("macro:\n{rest}")
    } else if let Some(rest) = text.strip_prefix("protectedmacro:") {
        format!("protectedmacro:\n{rest}")
    } else {
        text
    }
}

fn scan_balanced_raw_text<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    context: &'static str,
) -> Result<Vec<Token>, ExecError>
where
    S: InputSource,
{
    let open = next_non_space_raw(input, stores)?.ok_or(ExecError::MissingToken { context })?;
    if !is_begin_group(open) {
        return Err(ExecError::MissingToken { context });
    }
    let mut depth = 1usize;
    let mut tokens = Vec::new();
    while let Some(token) = input.next_token(stores)? {
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

fn scan_balanced_expanded_text<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: &'static str,
) -> Result<Vec<Token>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let open =
        next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken { context })?;
    if !is_begin_group(open) {
        return Err(ExecError::MissingToken { context });
    }
    let mut recorder = NoopRecorder;
    let mut depth = 1usize;
    let mut tokens = Vec::new();
    while let Some(token) =
        get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
            .map(tex_expand::semantic_token)
    {
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

fn next_non_space_raw<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<Option<Token>, ExecError>
where
    S: InputSource,
{
    while let Some(token) = input.next_token(stores)? {
        if !is_space(token) {
            return Ok(Some(token));
        }
    }
    Ok(None)
}

fn next_non_space_x<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<Option<Token>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    while let Some(token) =
        get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
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

/// Appends TeX82's printable token form, including the separator that
/// `print_cs` emits after a control word.
pub(crate) fn append_token_show_text(stores: &Universe, token: Token, text: &mut String) {
    text.push_str(&token_text(stores, token));
    if let Token::Cs(symbol) = token {
        let name = stores.resolve(symbol);
        if stores.control_sequence_kind(symbol) == tex_state::interner::ControlSequenceKind::Named
            && name.chars().all(|ch| ch.is_ascii_alphabetic())
        {
            text.push(' ');
        }
    }
}

fn hyphenated_word_text(word: &str, positions: &[usize]) -> String {
    let mut out = String::new();
    for (index, ch) in word.chars().enumerate() {
        if positions.contains(&index) {
            out.push('-');
        }
        out.push(ch);
    }
    if positions.contains(&word.chars().count()) {
        out.push('-');
    }
    out
}

fn write_wrapped_message(text: &str) -> String {
    let mut output = String::new();
    let mut column = 0usize;
    for ch in text.chars() {
        if column == 79 {
            output.push('\n');
            column = 0;
        }
        output.push(ch);
        column += 1;
    }
    output
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
