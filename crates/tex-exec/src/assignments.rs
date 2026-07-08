//! Assignment primitives owned by main control.

use tex_expand::scan::{scan_toks, scan_toks_expanded};
use tex_expand::{
    ExpandError, ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks, scan_int,
};
use tex_lex::{InputSource, InputStack, LexError, TokenListReplayKind};
use tex_state::env::banks::IntParam;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::stores::Stores;
use tex_state::token::{Catcode, Token};

use crate::{DispatchAction, ExecError, Mode, dispatch_delivered_token};

/// Executes a delivered token if it is an assignment/prefix primitive.
pub fn try_execute_assignment<S, H>(
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<bool, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let Token::Cs(symbol) = token else {
        return Ok(false);
    };
    if !matches!(
        stores.meaning(symbol),
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Def
                | UnexpandablePrimitive::Edef
                | UnexpandablePrimitive::Gdef
                | UnexpandablePrimitive::Xdef
                | UnexpandablePrimitive::Let
                | UnexpandablePrimitive::FutureLet
                | UnexpandablePrimitive::GlobalDefs
                | UnexpandablePrimitive::Global
                | UnexpandablePrimitive::Long
                | UnexpandablePrimitive::Outer
                | UnexpandablePrimitive::Protected
        )
    ) {
        return Ok(false);
    }
    match dispatch_delivered_token(Mode::Vertical, token, input, stores, hooks)? {
        DispatchAction::Continue => Ok(true),
        DispatchAction::NotConsumed => Ok(false),
    }
}

/// Installs this crate's unexpandable primitive meanings.
pub fn install_unexpandable_primitives(stores: &mut Stores) {
    for (name, primitive) in [
        ("def", UnexpandablePrimitive::Def),
        ("edef", UnexpandablePrimitive::Edef),
        ("gdef", UnexpandablePrimitive::Gdef),
        ("xdef", UnexpandablePrimitive::Xdef),
        ("let", UnexpandablePrimitive::Let),
        ("futurelet", UnexpandablePrimitive::FutureLet),
        ("globaldefs", UnexpandablePrimitive::GlobalDefs),
        ("global", UnexpandablePrimitive::Global),
        ("long", UnexpandablePrimitive::Long),
        ("outer", UnexpandablePrimitive::Outer),
        ("protected", UnexpandablePrimitive::Protected),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
}

pub(crate) fn execute_unexpandable<S, H>(
    primitive: UnexpandablePrimitive,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut prefixes = Prefixes::default();
    let command = accumulate_prefixes(primitive, &mut prefixes, input, stores)?;
    execute_prefixed_command(command, prefixes, input, stores, hooks)?;
    Ok(DispatchAction::Continue)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Prefixes {
    global: bool,
    flags: MeaningFlags,
}

impl Default for Prefixes {
    fn default() -> Self {
        Self {
            global: false,
            flags: MeaningFlags::EMPTY,
        }
    }
}

fn accumulate_prefixes<S>(
    mut primitive: UnexpandablePrimitive,
    prefixes: &mut Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<UnexpandablePrimitive, ExecError>
where
    S: InputSource,
{
    loop {
        match primitive {
            UnexpandablePrimitive::Global => prefixes.global = true,
            UnexpandablePrimitive::Long => prefixes.flags = prefixes.flags | MeaningFlags::LONG,
            UnexpandablePrimitive::Outer => prefixes.flags = prefixes.flags | MeaningFlags::OUTER,
            UnexpandablePrimitive::Protected => {
                prefixes.flags = prefixes.flags | MeaningFlags::PROTECTED;
            }
            _ => return Ok(primitive),
        }

        let token = next_non_space_raw(input, stores)?.ok_or(ExecError::MissingPrefixedCommand)?;
        let Token::Cs(symbol) = token else {
            return Err(ExecError::PrefixWithNonAssignment { token });
        };
        primitive = match stores.meaning(symbol) {
            Meaning::UnexpandablePrimitive(primitive) => primitive,
            _ => return Err(ExecError::PrefixWithNonAssignment { token }),
        };
    }
}

fn execute_prefixed_command<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    match primitive {
        UnexpandablePrimitive::Def
        | UnexpandablePrimitive::Edef
        | UnexpandablePrimitive::Gdef
        | UnexpandablePrimitive::Xdef => execute_def(primitive, prefixes, input, stores, hooks),
        UnexpandablePrimitive::Let => execute_let(prefixes, input, stores),
        UnexpandablePrimitive::FutureLet => execute_futurelet(prefixes, input, stores),
        UnexpandablePrimitive::GlobalDefs => execute_globaldefs(prefixes, input, stores, hooks),
        UnexpandablePrimitive::Global
        | UnexpandablePrimitive::Long
        | UnexpandablePrimitive::Outer
        | UnexpandablePrimitive::Protected => unreachable!("prefixes are accumulated first"),
    }
}

fn execute_def<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let target = scan_control_sequence(input, stores, "macro definition")?;
    let expanded = matches!(
        primitive,
        UnexpandablePrimitive::Edef | UnexpandablePrimitive::Xdef
    );
    let global = prefixes.global
        || matches!(
            primitive,
            UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
        );
    let scanned = if expanded {
        scan_toks_expanded(input, stores, prefixes.flags, hooks)?
    } else {
        scan_toks(input, stores, prefixes.flags)?
    };
    if apply_globaldefs(global, stores) {
        stores.set_macro_meaning_global(target, scanned.meaning());
    } else {
        stores.set_macro_meaning(target, scanned.meaning());
    }
    Ok(())
}

fn execute_let<S>(
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    reject_macro_prefixes(prefixes)?;
    let target = scan_control_sequence(input, stores, "\\let")?;
    let rhs = scan_optional_equals_one_space(input, stores)?;
    let meaning = token_meaning_for_let(rhs, stores)?;
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    Ok(())
}

fn execute_futurelet<S>(
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    reject_macro_prefixes(prefixes)?;
    let target = scan_control_sequence(input, stores, "\\futurelet")?;
    let first = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\futurelet lookahead",
    })?;
    let second = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\futurelet lookahead",
    })?;
    let meaning = token_meaning_for_let(second, stores)?;
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    push_tokens(input, stores, [first, second]);
    Ok(())
}

fn execute_globaldefs<S, H>(
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    reject_macro_prefixes(prefixes)?;
    skip_optional_equals_x(input, stores, hooks)?;
    let mut recorder = NoopRecorder;
    let value = scan_int::scan_int_with_recorder_and_hooks(input, stores, &mut recorder, hooks)
        .map_err(ExpandError::from)?
        .value();
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_int_param_global(IntParam::GLOBAL_DEFS, value);
    } else {
        stores.set_int_param(IntParam::GLOBAL_DEFS, value);
    }
    Ok(())
}

fn reject_macro_prefixes(prefixes: Prefixes) -> Result<(), ExecError> {
    if prefixes.flags != MeaningFlags::EMPTY {
        return Err(ExecError::PrefixWithNonDefinition);
    }
    Ok(())
}

fn apply_globaldefs(explicit_global: bool, stores: &Stores) -> bool {
    match stores.int_param(IntParam::GLOBAL_DEFS).cmp(&0) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => explicit_global,
    }
}

fn skip_optional_equals_x<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
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
    }
    Ok(())
}

fn scan_control_sequence<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
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

fn scan_optional_equals_one_space<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
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

fn token_meaning_for_let(token: Token, stores: &Stores) -> Result<Meaning, ExecError> {
    match token {
        Token::Cs(symbol) => Ok(stores.meaning(symbol)),
        Token::Char { ch, .. } => Ok(Meaning::CharGiven(ch)),
        Token::Param(_) => Err(ExecError::InvalidLetRhs { token }),
    }
}

fn next_non_space_raw<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
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

fn push_tokens<S, I>(input: &mut InputStack<S>, stores: &mut Stores, tokens: I)
where
    S: InputSource,
    I: IntoIterator<Item = Token>,
{
    let tokens: Vec<_> = tokens.into_iter().collect();
    let token_list = stores.intern_token_list(&tokens);
    input.push_token_list(token_list, TokenListReplayKind::Inserted);
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

fn is_other_equals(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            ch: '=',
            cat: Catcode::Other
        }
    )
}
