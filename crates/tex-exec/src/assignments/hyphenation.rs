use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::token::{Catcode, Token};

use super::*;
use crate::ExecError;
#[cfg(test)]
pub(crate) use crate::canonical_paragraph_end::test_hyphenated_word;
use crate::canonical_paragraph_end::{
    apply_hyphenation_exceptions, apply_patterns, parse_pattern_word, pattern_capacity_error,
    report_apply_diagnostics,
};

pub(super) fn execute_patterns(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let words = scan_hyphenation_words(input, stores, execution, "\\patterns")?;
    let patterns = words
        .iter()
        .map(|word| parse_pattern_word(stores, word).0)
        .collect();
    let diagnostics = apply_patterns(stores, patterns).map_err(pattern_capacity_error)?;
    report_apply_diagnostics(stores, diagnostics)?;
    Ok(())
}

pub(super) fn execute_hyphenation(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let words = scan_hyphenation_words(input, stores, execution, "\\hyphenation")?;
    let diagnostics = apply_hyphenation_exceptions(stores, words);
    report_apply_diagnostics(stores, diagnostics)?;
    Ok(())
}

fn scan_hyphenation_words(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: &'static str,
) -> Result<Vec<Vec<char>>, ExecError> {
    let open = loop {
        let traced = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        .ok_or(ExecError::MissingToken { context })?;
        let token = traced.semantic_token();
        if is_space(token) {
            continue;
        }
        if let Token::Cs(symbol) = token
            && stores.meaning(symbol) == Meaning::Relax
        {
            continue;
        }
        break token;
    };
    if !is_begin_group(open) {
        return Err(ExecError::MissingToken { context });
    }
    let mut words = Vec::new();
    let mut current = Vec::new();
    let mut depth = 1usize;
    while let Some(traced) = get_x_token_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
    )? {
        let token = traced.semantic_token();
        if is_begin_group(token) {
            depth += 1;
            continue;
        }
        if is_end_group(token) {
            depth -= 1;
            if depth == 0 {
                if !current.is_empty() {
                    words.push(current);
                }
                return Ok(words);
            }
            continue;
        }
        match token {
            Token::Char {
                cat: Catcode::Space,
                ..
            } => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            Token::Char { ch, .. } => current.push(ch),
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {}
        }
    }
    Err(ExecError::MissingToken { context })
}
