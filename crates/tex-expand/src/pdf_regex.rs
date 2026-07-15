use posix_regex::PosixRegexBuilder;
use posix_regex::compile::Error as RegexError;
use tex_lex::InputStack;
use tex_state::ExpansionState;
use tex_state::token::TracedTokenWord;

use crate::{Dispatch, ExpandError, ExpansionContext, ExpansionMode};

const DEFAULT_SUBCOUNT: u32 = 10;

pub(crate) fn execute_match(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let case_insensitive = crate::scan_helpers::scan_optional_keyword_with_mode_and_context(
        input, stores, expansion, mode, "icase",
    )?;
    let slot_count = if crate::scan_helpers::scan_optional_keyword_with_mode_and_context(
        input, stores, expansion, mode, "subcount",
    )? {
        let scanned = crate::scan_int::scan_int_with_mode_and_context(
            input, stores, expansion, mode, context,
        )?;
        if let Some(diagnostic) = scanned.diagnostic() {
            stores.report_expansion_diagnostic(&format!("\n! {diagnostic}.\n"));
        }
        u32::try_from(scanned.value()).unwrap_or(DEFAULT_SUBCOUNT)
    } else {
        DEFAULT_SUBCOUNT
    };

    let mut pattern =
        crate::pdf_strings::scan_expanded_bytes(input, stores, expansion, mode, context)?;
    let mut haystack =
        crate::pdf_strings::scan_expanded_bytes(input, stores, expansion, mode, context)?;
    truncate_at_nul(&mut pattern);
    truncate_at_nul(&mut haystack);

    let regex = match PosixRegexBuilder::new(&pattern)
        .extended(true)
        .with_default_classes()
        .compile()
    {
        Ok(regex) => regex.case_insensitive(case_insensitive),
        Err(error) => {
            stores.report_expansion_diagnostic(&format!(
                "\npdfTeX warning: pdftex: \\pdfmatch: {}\n",
                regex_error_message(&pattern, &error)
            ));
            return Ok(crate::pdf_strings::render_bytes(stores, b"-1", context));
        }
    };

    let result = regex.matches(&haystack, Some(1)).into_iter().next();
    let matched = result.is_some();
    let captures = result
        .map(|captures| {
            captures
                .iter()
                .take(slot_count as usize)
                .map(|capture| {
                    capture.and_then(|(start, end)| {
                        Some((u32::try_from(start).ok()?, u32::try_from(end).ok()?))
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    stores.set_pdf_match_state(haystack, captures, slot_count, matched);
    Ok(crate::pdf_strings::render_bytes(
        stores,
        if matched { b"1" } else { b"0" },
        context,
    ))
}

pub(crate) fn execute_last_match(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let scanned =
        crate::scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?;
    if let Some(diagnostic) = scanned.diagnostic() {
        stores.report_expansion_diagnostic(&format!("\n! {diagnostic}.\n"));
    }
    let index = if scanned.value() < 0 {
        stores
            .report_expansion_diagnostic(&format!("\n! Bad match number ({}).\n", scanned.value()));
        0
    } else {
        scanned.value() as u32
    };

    let mut output = Vec::new();
    if let Some((start, capture)) = stores.pdf_match_capture(index) {
        output.extend_from_slice(start.to_string().as_bytes());
        output.extend_from_slice(b"->");
        output.extend_from_slice(capture);
    } else {
        output.extend_from_slice(b"-1->");
    }
    Ok(crate::pdf_strings::render_bytes(stores, &output, context))
}

fn truncate_at_nul(bytes: &mut Vec<u8>) {
    if let Some(index) = bytes.iter().position(|&byte| byte == 0) {
        bytes.truncate(index);
    }
}

fn regex_error_message(pattern: &[u8], error: &RegexError) -> &'static str {
    if unmatched_open(pattern, b'[', b']') {
        "brackets ([ ]) not balanced"
    } else if unmatched_open(pattern, b'(', b')') {
        "parentheses not balanced"
    } else if pattern.ends_with(b"\\") {
        "trailing backslash (\\)"
    } else {
        match error {
            RegexError::IllegalRange => "invalid character range",
            RegexError::LeadingRepetition | RegexError::EmptyRepetition => {
                "repetition-operator operand invalid"
            }
            RegexError::UnclosedRepetition | RegexError::IntegerOverflow => {
                "invalid repetition count(s)"
            }
            _ => "invalid regular expression",
        }
    }
}

fn unmatched_open(pattern: &[u8], open: u8, close: u8) -> bool {
    let mut depth = 0_u32;
    let mut escaped = false;
    for &byte in pattern {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == open {
            depth = depth.saturating_add(1);
        } else if byte == close && depth > 0 {
            depth -= 1;
        }
    }
    depth != 0
}
