//! Assignment primitives owned by main control.

use tex_expand::scan::{scan_toks, scan_toks_expanded_with_driver};
use tex_expand::{
    DriverExpandNext, ExpandError, ExpansionHooks, NoopRecorder,
    get_x_token_with_recorder_and_hooks, scan_dimen, scan_glue, scan_int,
    scan_optional_keyword_with_hooks,
};
use tex_lex::{InputSource, InputStack, LexError, TokenListReplayKind};
use tex_state::code_tables::{DelCode, LcCode, MathCode, SfCode, UcCode};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::{GroupKind, InteractionMode, Universe};

use crate::ModeNest;
use crate::{DispatchAction, ExecError, diagnostics, dispatch_delivered_token, leave_group};

mod arithmetic;
mod boxes;
mod fonts;
mod hmode;
mod hyphenation;
mod macros;
mod paragraph;
mod primitives;
mod scanning;
mod tokens;
mod variables;

use arithmetic::*;
use boxes::*;
use fonts::*;
use hmode::*;
pub(crate) use hmode::{append_given_char, flush_pending_hchars, try_append_character};
use hyphenation::*;
use macros::*;
use paragraph::*;
pub use primitives::install_unexpandable_primitives;
use scanning::*;
use tokens::*;
use variables::*;

/// Executes a delivered token if it is an assignment/prefix primitive.
pub fn try_execute_assignment<S, H>(
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Universe,
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
    let mut nest = ModeNest::new();
    match dispatch_delivered_token(&mut nest, token, input, stores, hooks)? {
        DispatchAction::Continue => Ok(true),
        DispatchAction::End => Ok(true),
        DispatchAction::NotConsumed => Ok(false),
    }
}

pub(crate) fn execute_unexpandable<S, H>(
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
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
    let assigned = execute_prefixed_command(command, prefixes, nest, input, stores, hooks)?;
    if assigned {
        fire_afterassignment(input, stores);
    }
    Ok(DispatchAction::Continue)
}

pub(crate) fn execute_assignment_meaning<S, H>(
    meaning: Meaning,
    input: &mut InputStack<S>,
    stores: &mut Universe,
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
    let mut nest = ModeNest::new();
    let assigned = execute_prefixed_command(command, prefixes, &mut nest, input, stores, hooks)?;
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
    stores: &mut Universe,
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

fn execute_prefixed_command<S, H>(
    command: PrefixedCommand,
    prefixes: Prefixes,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<bool, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
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
            UnexpandablePrimitive::Font => {
                execute_font_definition(prefixes, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::FontDimen
            | UnexpandablePrimitive::HyphenChar
            | UnexpandablePrimitive::SkewChar => {
                let target = scan_font_variable_target(primitive, input, stores, hooks)?;
                execute_assignment_to_target(target, prefixes, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Patterns => {
                reject_all_prefixes(prefixes)?;
                execute_patterns(input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Hyphenation => {
                reject_all_prefixes(prefixes)?;
                execute_hyphenation(input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Par
            | UnexpandablePrimitive::EndGraf
            | UnexpandablePrimitive::Indent
            | UnexpandablePrimitive::NoIndent
            | UnexpandablePrimitive::ParShape
            | UnexpandablePrimitive::PrevDepth
            | UnexpandablePrimitive::NoInterlineSkip => {
                reject_all_prefixes(prefixes)?;
                execute_paragraph_command(primitive, nest, input, stores, hooks)?;
                Ok(primitive == UnexpandablePrimitive::ParShape
                    || primitive == UnexpandablePrimitive::PrevDepth)
            }
            UnexpandablePrimitive::HBox
            | UnexpandablePrimitive::VBox
            | UnexpandablePrimitive::VTop => {
                reject_macro_prefixes(prefixes)?;
                execute_make_box(primitive, nest, prefixes.global, input, stores, hooks)?;
                Ok(false)
            }
            UnexpandablePrimitive::SetBox => {
                reject_macro_prefixes(prefixes)?;
                execute_setbox(prefixes.global, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Box
            | UnexpandablePrimitive::Copy
            | UnexpandablePrimitive::UnHBox
            | UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::LastBox
            | UnexpandablePrimitive::Raise
            | UnexpandablePrimitive::Lower
            | UnexpandablePrimitive::MoveLeft
            | UnexpandablePrimitive::MoveRight => {
                reject_all_prefixes(prefixes)?;
                execute_box_list_command(primitive, nest, input, stores, hooks)?;
                Ok(false)
            }
            UnexpandablePrimitive::Kern
            | UnexpandablePrimitive::HSkip
            | UnexpandablePrimitive::VSkip => {
                reject_all_prefixes(prefixes)?;
                execute_kern_or_skip(primitive, nest, input, stores, hooks)?;
                Ok(false)
            }
            UnexpandablePrimitive::Char
            | UnexpandablePrimitive::HFil
            | UnexpandablePrimitive::HFill
            | UnexpandablePrimitive::HSs
            | UnexpandablePrimitive::HFilNeg
            | UnexpandablePrimitive::Penalty
            | UnexpandablePrimitive::VRule
            | UnexpandablePrimitive::ItalicCorrection
            | UnexpandablePrimitive::Discretionary
            | UnexpandablePrimitive::DiscretionaryHyphen
            | UnexpandablePrimitive::NoBoundary
            | UnexpandablePrimitive::SpaceFactor
            | UnexpandablePrimitive::Accent => {
                reject_all_prefixes(prefixes)?;
                execute_hmode_material(primitive, nest, input, stores, hooks)?;
                Ok(primitive == UnexpandablePrimitive::SpaceFactor)
            }
            UnexpandablePrimitive::Wd | UnexpandablePrimitive::Ht | UnexpandablePrimitive::Dp => {
                reject_macro_prefixes(prefixes)?;
                execute_box_dimension_assignment(primitive, prefixes.global, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Read => {
                execute_read(input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Write => {
                execute_write(input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::OpenIn
            | UnexpandablePrimitive::CloseIn
            | UnexpandablePrimitive::OpenOut
            | UnexpandablePrimitive::CloseOut => {
                execute_stream_command(primitive, input, stores, hooks)?;
                Ok(true)
            }
            UnexpandablePrimitive::Show => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_show(input, stores)?;
                Ok(false)
            }
            UnexpandablePrimitive::ShowBox => {
                reject_all_prefixes(prefixes)?;
                let index = scan_register_index(input, stores, hooks)?;
                diagnostics::execute_showbox(stores, index);
                Ok(false)
            }
            UnexpandablePrimitive::ShowThe => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showthe(input, stores, hooks)?;
                Ok(false)
            }
            UnexpandablePrimitive::ShowTokens => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showtokens(input, stores)?;
                Ok(false)
            }
            UnexpandablePrimitive::Message => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_message(input, stores, hooks, false)?;
                Ok(false)
            }
            UnexpandablePrimitive::ErrMessage => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_message(input, stores, hooks, true)?;
                Ok(false)
            }
            UnexpandablePrimitive::ShowLists => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showlists(stores);
                Ok(false)
            }
            UnexpandablePrimitive::ShowHyphens => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showhyphens(input, stores, hooks)?;
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
