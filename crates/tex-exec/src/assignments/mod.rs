//! Assignment primitives owned by main control.

use tex_expand::scan::{scan_toks, scan_toks_expanded};
use tex_expand::{
    ExpandError, ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks, scan_dimen,
    scan_glue, scan_int, scan_optional_keyword_with_hooks,
};
use tex_lex::{InputSource, InputStack, LexError, TokenListReplayKind};
use tex_state::code_tables::{DelCode, LcCode, MathCode, SfCode, UcCode};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::stores::{GroupKind, Stores};
use tex_state::token::{Catcode, Token};

use crate::{
    DispatchAction, ExecError, LogSink, Mode, NoopLogSink, diagnostics, dispatch_delivered_token,
    leave_group,
};

mod arithmetic;
mod macros;
mod primitives;
mod scanning;
mod tokens;
mod variables;

use arithmetic::*;
use macros::*;
pub use primitives::install_unexpandable_primitives;
use scanning::*;
use tokens::*;
use variables::*;

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
    let meaning = stores.meaning(symbol);
    if !is_assignment_meaning(meaning) {
        return Ok(false);
    }
    match dispatch_delivered_token(Mode::Vertical, token, input, stores, hooks)? {
        DispatchAction::Continue => Ok(true),
        DispatchAction::End => Ok(true),
        DispatchAction::NotConsumed => Ok(false),
    }
}

pub(crate) fn execute_unexpandable<S, H, L>(
    primitive: UnexpandablePrimitive,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
    log: &mut L,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
    L: LogSink,
{
    let mut prefixes = Prefixes::default();
    let command = accumulate_prefixes(
        PrefixedCommand::Primitive(primitive),
        &mut prefixes,
        input,
        stores,
    )?;
    if command == PrefixedCommand::Primitive(UnexpandablePrimitive::End) {
        reject_all_prefixes(prefixes)?;
        return Ok(DispatchAction::End);
    }
    let assigned = execute_prefixed_command(command, prefixes, input, stores, hooks, log)?;
    if assigned {
        fire_afterassignment(input, stores);
    }
    Ok(DispatchAction::Continue)
}

pub(crate) fn execute_assignment_meaning<S, H>(
    meaning: Meaning,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut prefixes = Prefixes::default();
    let command = accumulate_prefixes(
        PrefixedCommand::Meaning(meaning),
        &mut prefixes,
        input,
        stores,
    )?;
    let assigned =
        execute_prefixed_command(command, prefixes, input, stores, hooks, &mut NoopLogSink)?;
    if assigned {
        fire_afterassignment(input, stores);
    }
    Ok(DispatchAction::Continue)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrefixedCommand {
    Primitive(UnexpandablePrimitive),
    Meaning(Meaning),
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
    mut command: PrefixedCommand,
    prefixes: &mut Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<PrefixedCommand, ExecError>
where
    S: InputSource,
{
    loop {
        let PrefixedCommand::Primitive(primitive) = command else {
            return Ok(command);
        };
        match primitive {
            UnexpandablePrimitive::Global => prefixes.global = true,
            UnexpandablePrimitive::Long => prefixes.flags = prefixes.flags | MeaningFlags::LONG,
            UnexpandablePrimitive::Outer => prefixes.flags = prefixes.flags | MeaningFlags::OUTER,
            UnexpandablePrimitive::Protected => {
                prefixes.flags = prefixes.flags | MeaningFlags::PROTECTED;
            }
            _ => return Ok(command),
        }

        let token = next_non_space_raw(input, stores)?.ok_or(ExecError::MissingPrefixedCommand)?;
        let Token::Cs(symbol) = token else {
            return Err(ExecError::PrefixWithNonAssignment { token });
        };
        command = match stores.meaning(symbol) {
            Meaning::UnexpandablePrimitive(primitive) => PrefixedCommand::Primitive(primitive),
            meaning if is_assignment_target_meaning(meaning) => PrefixedCommand::Meaning(meaning),
            _ => return Err(ExecError::PrefixWithNonAssignment { token }),
        };
    }
}

fn execute_prefixed_command<S, H, L>(
    command: PrefixedCommand,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
    log: &mut L,
) -> Result<bool, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
    L: LogSink,
{
    match command {
        PrefixedCommand::Primitive(primitive) => match primitive {
            UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef => {
                execute_def(primitive, prefixes, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Let => {
                execute_let(prefixes, input, stores)?;
                Ok(true)
            }
            UnexpandablePrimitive::FutureLet => {
                execute_futurelet(prefixes, input, stores)?;
                Ok(true)
            }
            UnexpandablePrimitive::GlobalDefs => {
                execute_globaldefs(prefixes, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::BeginGroup => {
                reject_all_prefixes(prefixes)?;
                stores.enter_group_with_kind(GroupKind::SemiSimple);
                Ok(false)
            }
            UnexpandablePrimitive::EndGroup => {
                reject_all_prefixes(prefixes)?;
                leave_group(input, stores, GroupKind::SemiSimple)?;
                Ok(false)
            }
            UnexpandablePrimitive::AfterGroup => {
                reject_all_prefixes(prefixes)?;
                execute_aftergroup(input, stores)?;
                Ok(false)
            }
            UnexpandablePrimitive::AfterAssignment => {
                reject_all_prefixes(prefixes)?;
                execute_afterassignment(input, stores)?;
                Ok(false)
            }
            UnexpandablePrimitive::Count
            | UnexpandablePrimitive::Dimen
            | UnexpandablePrimitive::Skip
            | UnexpandablePrimitive::Muskip
            | UnexpandablePrimitive::Toks => {
                execute_variable_assignment(primitive, prefixes, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::CountDef
            | UnexpandablePrimitive::DimenDef
            | UnexpandablePrimitive::SkipDef
            | UnexpandablePrimitive::MuskipDef
            | UnexpandablePrimitive::ToksDef => {
                execute_register_def(primitive, prefixes, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::CharDef | UnexpandablePrimitive::MathCharDef => {
                execute_char_def(primitive, prefixes, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Advance
            | UnexpandablePrimitive::Multiply
            | UnexpandablePrimitive::Divide => {
                execute_arithmetic(primitive, prefixes, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::CatCode
            | UnexpandablePrimitive::LcCode
            | UnexpandablePrimitive::UcCode
            | UnexpandablePrimitive::SfCode
            | UnexpandablePrimitive::MathCode
            | UnexpandablePrimitive::DelCode => {
                execute_code_table_assignment(primitive, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Read => {
                execute_read_stub(input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Show => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_show(input, stores, log)?;
                Ok(false)
            }
            UnexpandablePrimitive::ShowThe => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showthe(input, stores, hooks, log)?;
                Ok(false)
            }
            UnexpandablePrimitive::ShowTokens => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showtokens(input, stores, log)?;
                Ok(false)
            }
            UnexpandablePrimitive::Message => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_message(input, stores, hooks, log, false)?;
                Ok(false)
            }
            UnexpandablePrimitive::ErrMessage => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_message(input, stores, hooks, log, true)?;
                Ok(false)
            }
            UnexpandablePrimitive::ShowLists => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showlists(log);
                Ok(false)
            }
            UnexpandablePrimitive::Uppercase => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_change_case(input, stores, true)?;
                Ok(false)
            }
            UnexpandablePrimitive::Lowercase => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_change_case(input, stores, false)?;
                Ok(false)
            }
            UnexpandablePrimitive::IgnoreSpaces => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_ignorespaces(input, stores)?;
                Ok(false)
            }
            UnexpandablePrimitive::Global
            | UnexpandablePrimitive::Long
            | UnexpandablePrimitive::Outer
            | UnexpandablePrimitive::Protected
            | UnexpandablePrimitive::End => unreachable!("prefixes are accumulated first"),
        },
        PrefixedCommand::Meaning(meaning) => {
            reject_macro_prefixes(prefixes)?;
            let target =
                variable_from_meaning(meaning).ok_or(ExecError::UnsupportedAssignmentTarget)?;
            execute_assignment_to_target(target, prefixes, input, stores, hooks)?;
            Ok(true)
        }
    }
}
