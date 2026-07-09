//! Assignment primitives owned by main control.

use tex_expand::scan::{
    scan_general_text_expanded_with_driver, scan_toks, scan_toks_expanded_with_driver,
};
use tex_expand::{
    DriverExpandNext, ExpandError, ExpansionHooks, NoopRecorder, ReadRecorder,
    get_x_token_with_recorder_and_hooks, scan_dimen, scan_glue, scan_int,
    scan_optional_keyword_with_hooks,
};
use tex_lex::{InputSource, InputStack, LexError, TokenListReplayKind};
use tex_state::code_tables::{DelCode, LcCode, MathCode, SfCode, UcCode};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::interner::Symbol;
use tex_state::math::MathFontSize;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{GroupKind, InteractionMode, Universe};

use crate::ModeNest;
use crate::{
    DispatchAction, ExecError, diagnostics, dispatch_delivered_token, leave_group_with_origin,
};

mod arithmetic;
mod boxes;
mod fonts;
mod hmode;
mod hyphenation;
mod macros;
mod paragraph;
mod primitives;
mod scanning;
mod shipout;
mod tokens;
mod variables;

use arithmetic::*;
pub(crate) use boxes::scan_box_group;
use boxes::*;
use fonts::*;
use hmode::*;
pub(crate) use hmode::{append_given_char, flush_pending_hchars, try_append_character};
use hyphenation::*;
use macros::*;
use paragraph::*;
pub(crate) use paragraph::{
    display_line_dimensions, end_paragraph, ensure_horizontal_for_character,
    interrupt_paragraph_for_display,
};
pub use primitives::install_unexpandable_primitives;
use scanning::*;
pub(crate) use scanning::{
    next_non_space_traced_x, next_non_space_x, scan_glue_id, scan_i32, scan_optional_keyword_x,
    scan_scaled,
};
pub(crate) use shipout::shipout_node;
use shipout::*;
use tokens::*;
pub(crate) use tokens::{active_character_symbol, is_begin_group, is_end_group, is_space};
use variables::*;

/// Executes a delivered token if it is an assignment/prefix primitive.
pub fn try_execute_assignment<S, H>(
    traced: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<bool, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let token = tex_expand::semantic_token(traced);
    let Token::Cs(symbol) = token else {
        return Ok(false);
    };
    let meaning = stores.meaning(symbol);
    if !is_assignment_meaning(meaning) {
        return Ok(false);
    }
    let mut nest = ModeNest::new();
    match dispatch_delivered_token(&mut nest, traced, input, stores, hooks)? {
        DispatchAction::Continue => Ok(true),
        DispatchAction::End => Ok(true),
        DispatchAction::Shipout(_) => Ok(true),
        DispatchAction::NotConsumed => Ok(false),
    }
}

pub(crate) fn execute_unexpandable_with_recorder<S, R, H>(
    primitive: UnexpandablePrimitive,
    traced: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut prefixes = Prefixes::default();
    let command = accumulate_prefixes(
        PrefixedCommand::Primitive(primitive),
        traced,
        &mut prefixes,
        input,
        stores,
    )?;
    if command.command == PrefixedCommand::Primitive(UnexpandablePrimitive::End) {
        reject_all_prefixes(prefixes)?;
        return Ok(DispatchAction::End);
    }
    if command.command == PrefixedCommand::Primitive(UnexpandablePrimitive::Immediate) {
        reject_all_prefixes(prefixes)?;
        let outcome = execute_immediate(input, stores, recorder, hooks)?;
        if outcome.assigned {
            fire_afterassignment(input, stores);
        }
        return Ok(outcome.action);
    }
    let outcome =
        execute_prefixed_command(command, prefixes, nest, input, stores, recorder, hooks)?;
    if outcome.assigned {
        fire_afterassignment(input, stores);
    }
    Ok(outcome.action)
}

fn execute_immediate<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<CommandOutcome, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut noop = NoopRecorder;
    let traced = loop {
        let Some(traced) = get_x_token_with_recorder_and_hooks(input, stores, &mut noop, hooks)?
        else {
            return Err(ExecError::MissingPrefixedCommand);
        };
        if !is_space(tex_expand::semantic_token(traced)) {
            break traced;
        }
    };
    let token = tex_expand::semantic_token(traced);
    let origin = traced.origin();
    let Token::Cs(symbol) = token else {
        return Err(ExecError::PrefixWithNonAssignment { token, origin });
    };
    match stores.meaning(symbol) {
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::OpenOut | UnexpandablePrimitive::CloseOut),
        ) => {
            execute_immediate_stream_command(primitive, traced, input, stores, hooks)?;
            Ok(CommandOutcome::assigned())
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Write) => {
            execute_immediate_write(traced, input, stores, recorder, hooks)?;
            Ok(CommandOutcome::continue_only())
        }
        _ => Err(ExecError::PrefixWithNonAssignment { token, origin }),
    }
}

pub(crate) fn execute_assignment_meaning<S, H>(
    meaning: Meaning,
    traced: TracedTokenWord,
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
        traced,
        &mut prefixes,
        input,
        stores,
    )?;
    let mut nest = ModeNest::new();
    let mut recorder = NoopRecorder;
    let outcome = execute_prefixed_command(
        command,
        prefixes,
        &mut nest,
        input,
        stores,
        &mut recorder,
        hooks,
    )?;
    if outcome.assigned {
        fire_afterassignment(input, stores);
    }
    Ok(outcome.action)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrefixedCommand {
    Primitive(UnexpandablePrimitive),
    Meaning(Meaning),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TracedPrefixedCommand {
    command: PrefixedCommand,
    traced: TracedTokenWord,
    token: Token,
    origin: OriginId,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommandOutcome {
    assigned: bool,
    action: DispatchAction,
}

impl CommandOutcome {
    const fn assigned() -> Self {
        Self {
            assigned: true,
            action: DispatchAction::Continue,
        }
    }

    const fn assigned_if(assigned: bool) -> Self {
        Self {
            assigned,
            action: DispatchAction::Continue,
        }
    }

    const fn continue_only() -> Self {
        Self {
            assigned: false,
            action: DispatchAction::Continue,
        }
    }

    const fn shipout(hash: tex_state::ContentHash) -> Self {
        Self {
            assigned: false,
            action: DispatchAction::Shipout(hash),
        }
    }
}

fn accumulate_prefixes<S>(
    mut command: PrefixedCommand,
    traced: TracedTokenWord,
    prefixes: &mut Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<TracedPrefixedCommand, ExecError>
where
    S: InputSource,
{
    let mut token = tex_expand::semantic_token(traced);
    let mut origin = traced.origin();
    loop {
        let PrefixedCommand::Primitive(primitive) = command else {
            return Ok(TracedPrefixedCommand {
                command,
                traced,
                token,
                origin,
            });
        };
        match primitive {
            UnexpandablePrimitive::Global => prefixes.global = true,
            UnexpandablePrimitive::Long => prefixes.flags = prefixes.flags | MeaningFlags::LONG,
            UnexpandablePrimitive::Outer => prefixes.flags = prefixes.flags | MeaningFlags::OUTER,
            UnexpandablePrimitive::Protected => {
                prefixes.flags = prefixes.flags | MeaningFlags::PROTECTED;
            }
            _ => {
                return Ok(TracedPrefixedCommand {
                    command,
                    traced,
                    token,
                    origin,
                });
            }
        }

        let traced =
            next_non_space_traced_raw(input, stores)?.ok_or(ExecError::MissingPrefixedCommand)?;
        token = tex_expand::semantic_token(traced);
        origin = traced.origin();
        let Token::Cs(symbol) = token else {
            return Err(ExecError::PrefixWithNonAssignment { token, origin });
        };
        command = match stores.meaning(symbol) {
            Meaning::UnexpandablePrimitive(primitive) => PrefixedCommand::Primitive(primitive),
            meaning if is_assignment_target_meaning(meaning) => PrefixedCommand::Meaning(meaning),
            _ => return Err(ExecError::PrefixWithNonAssignment { token, origin }),
        };
    }
}

fn execute_prefixed_command<S, H>(
    command: TracedPrefixedCommand,
    prefixes: Prefixes,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut impl ReadRecorder,
    hooks: &mut H,
) -> Result<CommandOutcome, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    match command.command {
        PrefixedCommand::Primitive(primitive) => match primitive {
            UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef => {
                execute_def(primitive, prefixes, input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Let => {
                execute_let(prefixes, input, stores)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::FutureLet => {
                execute_futurelet(prefixes, input, stores)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::GlobalDefs => {
                execute_globaldefs(prefixes, command.traced, input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::BeginGroup => {
                reject_all_prefixes(prefixes)?;
                stores.enter_group_with_kind(GroupKind::SemiSimple);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::EndGroup => {
                reject_all_prefixes(prefixes)?;
                leave_group_with_origin(
                    input,
                    stores,
                    GroupKind::SemiSimple,
                    command.traced.origin(),
                )?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::AfterGroup => {
                reject_all_prefixes(prefixes)?;
                execute_aftergroup(input, stores)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::AfterAssignment => {
                reject_all_prefixes(prefixes)?;
                execute_afterassignment(input, stores)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Count
            | UnexpandablePrimitive::Dimen
            | UnexpandablePrimitive::Skip
            | UnexpandablePrimitive::Muskip
            | UnexpandablePrimitive::Toks => {
                execute_variable_assignment(
                    primitive,
                    command.traced,
                    prefixes,
                    input,
                    stores,
                    hooks,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::CountDef
            | UnexpandablePrimitive::DimenDef
            | UnexpandablePrimitive::SkipDef
            | UnexpandablePrimitive::MuskipDef
            | UnexpandablePrimitive::ToksDef => {
                execute_register_def(primitive, prefixes, command.traced, input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::CharDef | UnexpandablePrimitive::MathCharDef => {
                execute_char_def(primitive, prefixes, command.traced, input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Advance
            | UnexpandablePrimitive::Multiply
            | UnexpandablePrimitive::Divide => {
                execute_arithmetic(primitive, prefixes, command.traced, input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::CatCode
            | UnexpandablePrimitive::LcCode
            | UnexpandablePrimitive::UcCode
            | UnexpandablePrimitive::SfCode
            | UnexpandablePrimitive::MathCode
            | UnexpandablePrimitive::DelCode => {
                execute_code_table_assignment(primitive, command.traced, input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Font => {
                execute_font_definition(prefixes, command.traced, input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::TextFont
            | UnexpandablePrimitive::ScriptFont
            | UnexpandablePrimitive::ScriptScriptFont => {
                execute_math_family_font_assignment(
                    primitive,
                    prefixes,
                    command.traced,
                    input,
                    stores,
                    hooks,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::FontDimen
            | UnexpandablePrimitive::HyphenChar
            | UnexpandablePrimitive::SkewChar => {
                let target =
                    scan_font_variable_target(primitive, command.traced, input, stores, hooks)?;
                execute_assignment_to_target(
                    target,
                    prefixes,
                    command.traced,
                    input,
                    stores,
                    hooks,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Patterns => {
                reject_all_prefixes(prefixes)?;
                execute_patterns(input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Hyphenation => {
                reject_all_prefixes(prefixes)?;
                execute_hyphenation(input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Par
            | UnexpandablePrimitive::EndGraf
            | UnexpandablePrimitive::Indent
            | UnexpandablePrimitive::NoIndent
            | UnexpandablePrimitive::ParShape
            | UnexpandablePrimitive::PrevDepth
            | UnexpandablePrimitive::PrevGraf
            | UnexpandablePrimitive::NoInterlineSkip => {
                reject_all_prefixes(prefixes)?;
                execute_paragraph_command(primitive, command.traced, nest, input, stores, hooks)?;
                Ok(CommandOutcome::assigned_if(
                    primitive == UnexpandablePrimitive::ParShape
                        || primitive == UnexpandablePrimitive::PrevDepth
                        || primitive == UnexpandablePrimitive::PrevGraf,
                ))
            }
            UnexpandablePrimitive::HAlign | UnexpandablePrimitive::VAlign => {
                reject_macro_prefixes(prefixes)?;
                crate::align::execute_alignment(
                    primitive,
                    command.traced,
                    nest,
                    input,
                    stores,
                    recorder,
                    hooks,
                )?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::HBox
            | UnexpandablePrimitive::VBox
            | UnexpandablePrimitive::VTop
            | UnexpandablePrimitive::VSplit => {
                reject_macro_prefixes(prefixes)?;
                execute_make_box(
                    primitive,
                    command.traced,
                    nest,
                    prefixes.global,
                    input,
                    stores,
                    hooks,
                )?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::SetBox => {
                reject_macro_prefixes(prefixes)?;
                execute_setbox(prefixes.global, command.traced, input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Box
            | UnexpandablePrimitive::Copy
            | UnexpandablePrimitive::UnHBox
            | UnexpandablePrimitive::UnHCopy
            | UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::UnVCopy
            | UnexpandablePrimitive::LastBox
            | UnexpandablePrimitive::Raise
            | UnexpandablePrimitive::Lower
            | UnexpandablePrimitive::MoveLeft
            | UnexpandablePrimitive::MoveRight => {
                reject_all_prefixes(prefixes)?;
                execute_box_list_command(primitive, command.traced, nest, input, stores, hooks)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Kern
            | UnexpandablePrimitive::HSkip
            | UnexpandablePrimitive::VSkip
            | UnexpandablePrimitive::VFil
            | UnexpandablePrimitive::VFill
            | UnexpandablePrimitive::VSs
            | UnexpandablePrimitive::VFilNeg => {
                reject_all_prefixes(prefixes)?;
                execute_kern_or_skip(primitive, command.traced, nest, input, stores, hooks)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Leaders
            | UnexpandablePrimitive::CLeaders
            | UnexpandablePrimitive::XLeaders => {
                reject_all_prefixes(prefixes)?;
                execute_leaders(primitive, command.traced, nest, input, stores, hooks)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::HRule => {
                reject_all_prefixes(prefixes)?;
                execute_hrule(command.traced, nest, input, stores, hooks)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::UnPenalty
            | UnexpandablePrimitive::UnKern
            | UnexpandablePrimitive::UnSkip => {
                reject_all_prefixes(prefixes)?;
                execute_delete_last(primitive, nest, stores)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::LastPenalty
            | UnexpandablePrimitive::LastKern
            | UnexpandablePrimitive::LastSkip => {
                reject_all_prefixes(prefixes)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Char
            | UnexpandablePrimitive::HFil
            | UnexpandablePrimitive::HFill
            | UnexpandablePrimitive::HSs
            | UnexpandablePrimitive::HFilNeg
            | UnexpandablePrimitive::Penalty
            | UnexpandablePrimitive::VRule
            | UnexpandablePrimitive::ControlSpace
            | UnexpandablePrimitive::ItalicCorrection
            | UnexpandablePrimitive::Discretionary
            | UnexpandablePrimitive::DiscretionaryHyphen
            | UnexpandablePrimitive::NoBoundary
            | UnexpandablePrimitive::SpaceFactor
            | UnexpandablePrimitive::Accent
            | UnexpandablePrimitive::Mark
            | UnexpandablePrimitive::VAdjust
            | UnexpandablePrimitive::Insert => {
                reject_all_prefixes(prefixes)?;
                execute_hmode_material(command.traced, primitive, nest, input, stores, hooks)?;
                Ok(CommandOutcome::assigned_if(
                    primitive == UnexpandablePrimitive::SpaceFactor,
                ))
            }
            UnexpandablePrimitive::Wd | UnexpandablePrimitive::Ht | UnexpandablePrimitive::Dp => {
                reject_macro_prefixes(prefixes)?;
                execute_box_dimension_assignment(
                    primitive,
                    prefixes.global,
                    command.traced,
                    input,
                    stores,
                    hooks,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Read => {
                execute_read(command.traced, input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Write => {
                execute_write(command.traced, nest, input, stores, hooks)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Special => {
                reject_all_prefixes(prefixes)?;
                execute_special(command.traced, nest, input, stores, hooks)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Shipout => {
                reject_all_prefixes(prefixes)?;
                let hash = execute_shipout(command.traced, input, stores, recorder, hooks)?;
                Ok(CommandOutcome::shipout(hash))
            }
            UnexpandablePrimitive::OpenIn
            | UnexpandablePrimitive::CloseIn
            | UnexpandablePrimitive::OpenOut
            | UnexpandablePrimitive::CloseOut => {
                execute_stream_command(primitive, command.traced, nest, input, stores, hooks)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Show => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_show(input, stores)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowBox => {
                reject_all_prefixes(prefixes)?;
                let index = scan_register_index(input, stores, hooks, command.traced)?;
                diagnostics::execute_showbox(stores, index);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowThe => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showthe(command.traced, input, stores, hooks)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowTokens => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showtokens(input, stores)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Message => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_message(input, stores, hooks, false)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ErrMessage => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_message(input, stores, hooks, true)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowLists => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showlists(stores, nest);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowHyphens => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showhyphens(input, stores, hooks)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Uppercase => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_change_case(input, stores, true)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Lowercase => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_change_case(input, stores, false)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::IgnoreSpaces => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_ignorespaces(input, stores)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::MathChar
            | UnexpandablePrimitive::Delimiter
            | UnexpandablePrimitive::MathOrd
            | UnexpandablePrimitive::MathOp
            | UnexpandablePrimitive::MathBin
            | UnexpandablePrimitive::MathRel
            | UnexpandablePrimitive::MathOpen
            | UnexpandablePrimitive::MathClose
            | UnexpandablePrimitive::MathPunct
            | UnexpandablePrimitive::MathInner
            | UnexpandablePrimitive::Underline
            | UnexpandablePrimitive::Overline
            | UnexpandablePrimitive::Limits
            | UnexpandablePrimitive::NoLimits
            | UnexpandablePrimitive::DisplayLimits
            | UnexpandablePrimitive::Over
            | UnexpandablePrimitive::Atop
            | UnexpandablePrimitive::Above
            | UnexpandablePrimitive::OverWithDelims
            | UnexpandablePrimitive::AtopWithDelims
            | UnexpandablePrimitive::AboveWithDelims
            | UnexpandablePrimitive::Radical
            | UnexpandablePrimitive::MathAccent
            | UnexpandablePrimitive::VCenter
            | UnexpandablePrimitive::MSkip
            | UnexpandablePrimitive::MKern
            | UnexpandablePrimitive::NonScript
            | UnexpandablePrimitive::MathChoice
            | UnexpandablePrimitive::Left
            | UnexpandablePrimitive::Right
            | UnexpandablePrimitive::EqNo
            | UnexpandablePrimitive::LeftEqNo
            | UnexpandablePrimitive::DisplayStyle
            | UnexpandablePrimitive::TextStyle
            | UnexpandablePrimitive::ScriptStyle
            | UnexpandablePrimitive::ScriptScriptStyle => {
                Err(ExecError::UnimplementedTypesetting {
                    mode: nest.current_mode(),
                    token: command.token,
                    origin: command.origin,
                    operation: "math primitive",
                })
            }
            UnexpandablePrimitive::NoAlign => Err(ExecError::MisplacedNoAlign),
            UnexpandablePrimitive::Omit => Err(ExecError::MisplacedOmit),
            UnexpandablePrimitive::Cr
            | UnexpandablePrimitive::CrCr
            | UnexpandablePrimitive::Span => Err(ExecError::UnimplementedTypesetting {
                mode: nest.current_mode(),
                token: command.token,
                origin: command.origin,
                operation: "alignment primitive",
            }),
            UnexpandablePrimitive::Global
            | UnexpandablePrimitive::Long
            | UnexpandablePrimitive::Outer
            | UnexpandablePrimitive::Protected
            | UnexpandablePrimitive::Immediate
            | UnexpandablePrimitive::End => unreachable!("prefixes are accumulated first"),
        },
        PrefixedCommand::Meaning(meaning) => {
            reject_macro_prefixes(prefixes)?;
            let target =
                variable_from_meaning(meaning).ok_or(ExecError::UnsupportedAssignmentTarget)?;
            execute_assignment_to_target(target, prefixes, command.traced, input, stores, hooks)?;
            Ok(CommandOutcome::assigned())
        }
    }
}
