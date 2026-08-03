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

pub(super) fn skip_optional_equals_x(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let traced = loop {
        let Some(token) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        else {
            return Err(ExecError::MissingToken {
                context: "assignment value",
            });
        };
        if !is_space(token.semantic_token()) {
            break token;
        }
    };
    if !is_other_equals(traced.semantic_token()) {
        tex_expand::back_input(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            [traced],
        );
    } else {
        let Some(next) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        else {
            return Ok(());
        };
        if !is_space(next.semantic_token()) {
            tex_expand::back_input(input, &mut tex_state::ExpansionContext::new(stores), [next]);
        }
    }
    Ok(())
}

/// tex.web §1215's `help5` for `get_r_token`.
const MISSING_CONTROL_SEQUENCE_HELP: [&str; 5] = [
    "Please don't say `\\def cs{...}', say `\\def\\cs{...}'.",
    "I've inserted an inaccessible control sequence so that your",
    "definition will be completed without mixing me up too badly.",
    "You can recover graciously from this error, if you're",
    "careful; see exercise 27.2 in The TeXbook.",
];

/// tex.web §1215's `back_input; cur_tok:=frozen_protection; ins_error`.
///
/// The offending token goes back as its own `backed_up` level and the frozen
/// `\inaccessible` is inserted above it, so the report names both, and the
/// caller's restarted scan reads the insertion as the definition's target.
fn insert_inaccessible_control_sequence(
    input: &mut InputStack,
    stores: &mut Universe,
    traced: TracedTokenWord,
) -> Result<(), ExecError> {
    let inaccessible = Token::Cs(stores.intern("inaccessible").symbol());
    let origin = stores.inserted_origin(
        InsertedOriginKind::ErrorRecovery,
        inaccessible,
        traced.origin(),
    );
    crate::error_report::back_tokens(input, stores, [traced]);
    crate::error_report::ins_error(
        input,
        stores,
        [TracedTokenWord::pack(inaccessible, origin)],
        "Missing control sequence inserted",
        &MISSING_CONTROL_SEQUENCE_HELP,
    )?;
    Ok(())
}

pub(super) fn scan_definition_target(
    input: &mut InputStack,
    stores: &mut Universe,
    context: &'static str,
) -> Result<Symbol, ExecError> {
    let traced = next_non_space_traced_raw(input, stores)?
        .ok_or(ExecError::MissingControlSequence { context })?;
    let token = traced.semantic_token();
    match token {
        Token::Cs(symbol) => Ok(symbol),
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => Ok(active_character_symbol(stores, ch)),
        _ => {
            // §1215's `goto restart`: the insertion is what the restarted
            // scan reads, so it, not a directly returned symbol, is the
            // target -- and the token that provoked the error stays queued
            // behind it exactly as TeX leaves it.
            insert_inaccessible_control_sequence(input, stores, traced)?;
            scan_definition_target(input, stores, context)
        }
    }
}

pub(super) struct TracedDefinitionTarget {
    pub symbol: Symbol,
    pub traced: TracedTokenWord,
    pub origin: OriginId,
}

pub(super) fn scan_traced_definition_target(
    input: &mut InputStack,
    stores: &mut Universe,
    context: &'static str,
) -> Result<TracedDefinitionTarget, ExecError> {
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
            // `get_r_token` recovery used by macro definitions: restarting
            // the scan is what gives the target the inserted token's own
            // origin rather than a synthesized copy of it.
            insert_inaccessible_control_sequence(input, stores, traced)?;
            return scan_traced_definition_target(input, stores, context);
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

pub(super) fn scan_optional_equals_one_space(
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<TracedTokenWord, ExecError> {
    let first = loop {
        let token = input
            .next_traced_token(stores)?
            .ok_or(ExecError::MissingToken {
                context: "\\let right-hand side",
            })?;
        if !is_space(token.semantic_token()) {
            break token;
        }
    };
    if !is_other_equals(first.semantic_token()) {
        return Ok(first);
    }
    let next = input
        .next_traced_token(stores)?
        .ok_or(ExecError::MissingToken {
            context: "\\let right-hand side",
        })?;
    if is_space(next.semantic_token()) {
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
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Meaning, ExecError> {
    let token = traced.semantic_token();
    match token {
        Token::Cs(symbol) => {
            let meaning = stores.meaning(symbol);
            execution.record_meaning(symbol, meaning);
            Ok(meaning)
        }
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => tex_state::ExpansionState::active_character_symbol(stores, ch).map_or(
            Ok(Meaning::Undefined),
            |symbol| {
                let meaning = stores.meaning(symbol);
                execution.record_meaning(symbol.symbol(), meaning);
                Ok(meaning)
            },
        ),
        Token::Char { ch, cat } => Ok(Meaning::CharToken { ch, cat }),
        token if token.is_frozen_end_template() || token.is_frozen_endv() => Ok(
            Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::EndTemplate),
        ),
        Token::Param(_) | Token::Frozen(_) => Err(ExecError::InvalidLetRhs {
            token,
            origin: traced.origin(),
        }),
    }
}

pub(super) fn next_non_space_traced_raw(
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<Option<TracedTokenWord>, LexError> {
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

pub(super) fn push_tokens<I>(input: &mut InputStack, stores: &mut Universe, tokens: I)
where
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
