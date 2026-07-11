use super::*;

pub(super) fn reject_macro_prefixes(prefixes: Prefixes) -> Result<(), ExecError> {
    if prefixes.flags != MeaningFlags::EMPTY {
        return Err(ExecError::PrefixWithNonDefinition { origin: None });
    }
    Ok(())
}

pub(super) fn reject_all_prefixes(prefixes: Prefixes) -> Result<(), ExecError> {
    if prefixes.global || prefixes.flags != MeaningFlags::EMPTY {
        return Err(ExecError::PrefixWithNonDefinition { origin: None });
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
            .map(tex_expand::semantic_token)
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
            .map(tex_expand::semantic_token)
        else {
            return Ok(());
        };
        if !is_space(next) {
            push_tokens(input, stores, [next]);
        }
    }
    Ok(())
}

pub(super) fn scan_definition_target<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    context: &'static str,
) -> Result<Symbol, ExecError>
where
    S: InputSource,
{
    let traced = next_non_space_traced_raw(input, stores)?
        .ok_or(ExecError::MissingControlSequence { context })?;
    let token = tex_expand::semantic_token(traced);
    match token {
        Token::Cs(symbol) => Ok(symbol),
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => Ok(active_character_symbol(stores, ch)),
        _ => {
            // TeX82 `get_r_token` backs up a non-definable token and inserts
            // its frozen inaccessible control sequence (tex.web §1215).
            push_traced_tokens(input, stores, [traced]);
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! Missing control sequence inserted.\nPlease don't say `\\def cs{...}', say `\\def\\cs{...}'.\nI've inserted an inaccessible control sequence so that your\ndefinition will be completed without mixing me up too badly.\nYou can recover graciously from this error, if you're\ncareful; see exercise 27.2 in The TeXbook.\n",
            );
            Ok(stores.intern("inaccessible").symbol())
        }
    }
}

pub(super) struct TracedDefinitionTarget {
    pub symbol: Symbol,
    pub traced: TracedTokenWord,
    pub origin: OriginId,
}

pub(super) fn scan_traced_definition_target<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    context: &'static str,
) -> Result<TracedDefinitionTarget, ExecError>
where
    S: InputSource,
{
    let traced = next_non_space_traced_raw(input, stores)?
        .ok_or(ExecError::MissingControlSequence { context })?;
    let token = traced
        .token()
        .expect("input stack must only deliver valid traced tokens");
    let symbol = match token {
        Token::Cs(symbol) => symbol,
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => active_character_symbol(stores, ch),
        _ => {
            // This is the provenance-preserving form of TeX.web §1215's
            // `get_r_token` recovery used by macro definitions.
            push_traced_tokens(input, stores, [traced]);
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! Missing control sequence inserted.\nPlease don't say `\\def cs{...}', say `\\def\\cs{...}'.\nI've inserted an inaccessible control sequence so that your\ndefinition will be completed without mixing me up too badly.\nYou can recover graciously from this error, if you're\ncareful; see exercise 27.2 in The TeXbook.\n",
            );
            let symbol = stores.intern("inaccessible");
            let inserted_token = Token::Cs(symbol.symbol());
            let origin = stores.inserted_origin(
                InsertedOriginKind::ErrorRecovery,
                inserted_token,
                traced.origin(),
            );
            return Ok(TracedDefinitionTarget {
                symbol: symbol.symbol(),
                traced: TracedTokenWord::pack(inserted_token, origin),
                origin,
            });
        }
    };
    Ok(TracedDefinitionTarget {
        symbol,
        traced,
        origin: traced.origin(),
    })
}

pub(crate) fn active_character_symbol(stores: &mut Universe, ch: char) -> Symbol {
    stores.intern_active_character(ch).symbol()
}

pub(super) fn scan_optional_equals_one_space<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<TracedTokenWord, ExecError>
where
    S: InputSource,
{
    let first = loop {
        let token = input
            .next_traced_token(stores)?
            .ok_or(ExecError::MissingToken {
                context: "\\let right-hand side",
            })?;
        if !is_space(tex_expand::semantic_token(token)) {
            break token;
        }
    };
    if !is_other_equals(tex_expand::semantic_token(first)) {
        return Ok(first);
    }
    let next = input
        .next_traced_token(stores)?
        .ok_or(ExecError::MissingToken {
            context: "\\let right-hand side",
        })?;
    if is_space(tex_expand::semantic_token(next)) {
        input
            .next_traced_token(stores)?
            .ok_or(ExecError::MissingToken {
                context: "\\let right-hand side",
            })
    } else {
        Ok(next)
    }
}

pub(super) fn token_meaning_for_let(
    traced: TracedTokenWord,
    stores: &Universe,
) -> Result<Meaning, ExecError> {
    let token = tex_expand::semantic_token(traced);
    match token {
        Token::Cs(symbol) => Ok(stores.meaning(symbol)),
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => stores
            .active_character_symbol(ch)
            .map_or(Ok(Meaning::Undefined), |symbol| Ok(stores.meaning(symbol))),
        Token::Char { ch, cat } => Ok(Meaning::CharToken { ch, cat }),
        Token::Frozen(tex_state::token::FrozenToken::EndTemplate) => Ok(
            Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::EndTemplate),
        ),
        Token::Param(_) | Token::Frozen(_) => Err(ExecError::InvalidLetRhs {
            token,
            origin: traced.origin(),
        }),
    }
}

pub(super) fn next_non_space_traced_raw<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<Option<TracedTokenWord>, LexError>
where
    S: InputSource,
{
    loop {
        let Some(token) = input.next_traced_token(stores)? else {
            return Ok(None);
        };
        let semantic = token
            .token()
            .expect("input stack must only deliver valid traced tokens");
        if !is_space(semantic) {
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

pub(crate) fn has_catcode_meaning(stores: &Universe, token: Token, expected: Catcode) -> bool {
    match token {
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => stores.active_character_symbol(ch).is_some_and(|symbol| {
            matches!(
                stores.meaning(symbol),
                Meaning::CharToken { cat, .. } if cat == expected
            )
        }),
        Token::Char { cat, .. } => cat == expected,
        Token::Cs(symbol) => matches!(
            stores.meaning(symbol),
            Meaning::CharToken { cat, .. } if cat == expected
        ),
        Token::Param(_) | Token::Frozen(_) => false,
    }
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
