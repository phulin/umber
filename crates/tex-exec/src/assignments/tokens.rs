use super::*;

pub(super) fn reject_macro_prefixes(prefixes: Prefixes) -> Result<(), ExecError> {
    if prefixes.flags != MeaningFlags::EMPTY {
        return Err(ExecError::PrefixWithNonDefinition);
    }
    Ok(())
}

pub(super) fn reject_all_prefixes(prefixes: Prefixes) -> Result<(), ExecError> {
    if prefixes.global || prefixes.flags != MeaningFlags::EMPTY {
        return Err(ExecError::PrefixWithNonDefinition);
    }
    Ok(())
}

pub(super) fn apply_globaldefs(explicit_global: bool, stores: &Universe) -> bool {
    match stores.int_param(IntParam::GLOBAL_DEFS).cmp(&0) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => explicit_global,
    }
}

pub(super) fn skip_optional_equals_x<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let token = loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
        else {
            return Err(ExecError::MissingToken {
                context: "assignment value",
            });
        };
        if !is_space(token) {
            break token;
        }
    };
    if !is_other_equals(token) {
        push_tokens(input, stores, [token]);
    } else {
        let Some(next) = get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
        else {
            return Ok(());
        };
        if !is_space(next) {
            push_tokens(input, stores, [next]);
        }
    }
    Ok(())
}

pub(super) fn scan_control_sequence<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    context: &'static str,
) -> Result<Symbol, ExecError>
where
    S: InputSource,
{
    let token =
        next_non_space_raw(input, stores)?.ok_or(ExecError::MissingControlSequence { context })?;
    match token {
        Token::Cs(symbol) => Ok(symbol),
        _ => Err(ExecError::ExpectedControlSequence { context, token }),
    }
}

pub(super) fn scan_optional_equals_one_space<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<Token, ExecError>
where
    S: InputSource,
{
    let first = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\let right-hand side",
    })?;
    if !is_other_equals(first) {
        return Ok(first);
    }
    let next = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\let right-hand side",
    })?;
    if is_space(next) {
        input.next_token(stores)?.ok_or(ExecError::MissingToken {
            context: "\\let right-hand side",
        })
    } else {
        Ok(next)
    }
}

pub(super) fn token_meaning_for_let(token: Token, stores: &Universe) -> Result<Meaning, ExecError> {
    match token {
        Token::Cs(symbol) => Ok(stores.meaning(symbol)),
        Token::Char { ch, .. } => Ok(Meaning::CharGiven(ch)),
        Token::Param(_) => Err(ExecError::InvalidLetRhs { token }),
    }
}

pub(super) fn next_non_space_raw<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<Option<Token>, LexError>
where
    S: InputSource,
{
    loop {
        let Some(token) = input.next_token(stores)? else {
            return Ok(None);
        };
        if !is_space(token) {
            return Ok(Some(token));
        }
    }
}

pub(super) fn push_tokens<S, I>(input: &mut InputStack<S>, stores: &mut Universe, tokens: I)
where
    S: InputSource,
    I: IntoIterator<Item = Token>,
{
    let tokens: Vec<_> = tokens.into_iter().collect();
    let token_list = stores.intern_token_list(&tokens);
    input.push_token_list(token_list, TokenListReplayKind::Inserted);
}

pub(crate) fn is_space(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::Space,
            ..
        }
    )
}

pub(crate) fn is_begin_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    )
}

pub(crate) fn is_end_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    )
}

pub(super) fn is_other_equals(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            ch: '=',
            cat: Catcode::Other
        }
    )
}
