//! Assignment primitives owned by main control.

use tex_expand::scan::{
    scan_general_text_expanded_with_driver, scan_toks, scan_toks_expanded_with_driver,
};
use tex_expand::{
    DriverExpansionMode, ExpandError, get_command_token_with_context, get_x_token_with_context,
    scan_dimen, scan_glue, scan_int, scan_optional_keyword_with_context,
};
use tex_lex::{InputStack, LexError, TokenListReplayKind};
use tex_state::code_tables::{DelCode, LcCode, MathCode, SfCode, UcCode};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::interner::Symbol;
use tex_state::math::MathFontSize;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::node::Node;
use tex_state::provenance::InsertedOriginKind;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{GroupKind, InteractionMode, Universe};

use crate::ModeNest;
use crate::{
    DispatchAction, ExecError, diagnostics, dispatch_delivered_token, leave_group_with_origin,
    push_traced_tokens,
};

mod admissibility;
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

use admissibility::is_assignment_primitive;
pub(crate) use admissibility::math_allows_mode_independent_primitive;
use arithmetic::*;
pub(crate) use boxes::hpack_with_overfull_rule;
pub(crate) use boxes::scan_math_box;
use boxes::*;
pub(crate) use boxes::{scan_box_group, scan_pack_spec};
use fonts::*;
pub(crate) use hmode::fixed_infinite_glue;
pub(crate) use hmode::scan_rule_node;
use hmode::*;
pub(crate) use hmode::{append_given_char, flush_pending_hchars, try_append_character};
#[cfg(test)]
pub(crate) use hyphenation::test_hyphenated_word as test_hyphenated_hlist;
use hyphenation::*;
use macros::*;
#[cfg(test)]
pub(crate) use paragraph::break_hlist as test_break_hlist;
use paragraph::*;
pub(crate) use paragraph::{
    display_line_dimensions, end_paragraph, ensure_horizontal_for_character,
    interrupt_paragraph_for_display, make_indent_box, normal_paragraph,
};
pub use primitives::{install_etex_unexpandable_primitives, install_unexpandable_primitives};
use scanning::*;
pub(crate) use scanning::{
    is_assignment_target_meaning, next_non_space_traced_x, next_non_space_x, scan_glue_id,
    scan_i32, scan_optional_keyword_x, scan_scaled,
};
pub(crate) use shipout::shipout_node;
use shipout::*;
use tokens::*;
pub(crate) use tokens::{
    active_character_symbol, has_catcode_meaning, is_begin_group, is_end_group, is_space,
};
use variables::*;

/// Executes a delivered token if it is an assignment/prefix primitive.
pub fn try_execute_assignment(
    traced: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<bool, ExecError> {
    let token = tex_expand::semantic_token(traced);
    let Token::Cs(symbol) = token else {
        return Ok(false);
    };
    let meaning = stores.meaning(symbol);
    if !is_assignment_meaning(meaning) {
        return Ok(false);
    }
    let mut nest = ModeNest::new();
    match dispatch_delivered_token(&mut nest, traced, input, stores, execution)? {
        DispatchAction::Continue => Ok(true),
        DispatchAction::End => Ok(true),
        DispatchAction::Shipout(_) => Ok(true),
        DispatchAction::NotConsumed => Ok(false),
    }
}

pub(crate) fn execute_unexpandable_with_context(
    primitive: UnexpandablePrimitive,
    traced: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    let mut prefixes = Prefixes::default();
    let command = match accumulate_prefixes(
        PrefixedCommand::Primitive(primitive),
        traced,
        &mut prefixes,
        input,
        stores,
        execution,
    ) {
        Ok(command) => command,
        Err(ExecError::PrefixWithNonAssignment { token, origin }) => {
            // TeX.web §§1218–1219 uses `back_error` here: the prefixes are
            // discarded, but the offending expanded token is put back for
            // ordinary main-control dispatch.
            push_traced_tokens(input, stores, [TracedTokenWord::pack(token, origin)]);
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! You can't use a prefix with this command.\nI'll pretend you didn't say \\long or \\outer or \\global.\n",
            );
            return Ok(DispatchAction::Continue);
        }
        Err(error) => return Err(error),
    };
    if matches!(
        command.command,
        PrefixedCommand::Primitive(UnexpandablePrimitive::End | UnexpandablePrimitive::Dump)
    ) {
        reject_all_prefixes(prefixes)?;
        return Ok(DispatchAction::End);
    }
    if command.command == PrefixedCommand::Primitive(UnexpandablePrimitive::Immediate) {
        reject_all_prefixes(prefixes)?;
        let outcome = execute_immediate(input, stores, execution)?;
        if outcome.assigned {
            fire_afterassignment(input, stores);
        }
        return Ok(outcome.action);
    }
    let outcome = match execute_prefixed_command(command, prefixes, nest, input, stores, execution)
    {
        Ok(outcome) => outcome,
        Err(ExecError::PrefixWithNonDefinition { .. }) => {
            push_traced_tokens(input, stores, [command.traced]);
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! You can't use a prefix with this command.\nI'll pretend you didn't say \\long or \\outer or \\global.\n",
            );
            return Ok(DispatchAction::Continue);
        }
        Err(ExecError::ArithmeticOverflow)
            if matches!(
                command.command,
                PrefixedCommand::Primitive(
                    UnexpandablePrimitive::Advance
                        | UnexpandablePrimitive::Multiply
                        | UnexpandablePrimitive::Divide
                )
            ) =>
        {
            // TeX.web's arithmetic commands consume their operands, report
            // overflow/division by zero, and leave the target unchanged.
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! Arithmetic overflow.\nI can't carry out that multiplication or division,\nsince the result is out of range.\n",
            );
            return Ok(DispatchAction::Continue);
        }
        Err(error) => return Err(error),
    };
    if outcome.assigned {
        fire_afterassignment(input, stores);
    }
    Ok(outcome.action)
}

fn execute_immediate(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<CommandOutcome, ExecError> {
    let traced = loop {
        let Some(traced) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
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
            execute_immediate_stream_command(primitive, traced, input, stores, execution)?;
            Ok(CommandOutcome::assigned())
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Write) => {
            execute_immediate_write(traced, input, stores, execution)?;
            Ok(CommandOutcome::continue_only())
        }
        _ => {
            // TeX.web's `do_extension` treats `\immediate` as a one-token
            // lookahead: only openout, write, and closeout are executed here.
            // Every other expanded token is put back for ordinary main-control
            // dispatch (section 1377), so `\immediate\catcode` is a deliberate
            // no-op prefix in the official TRIP input.
            push_traced_tokens(input, stores, [traced]);
            Ok(CommandOutcome::continue_only())
        }
    }
}

pub(crate) fn execute_assignment_meaning(
    meaning: Meaning,
    traced: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    let mut prefixes = Prefixes::default();
    let command = accumulate_prefixes(
        PrefixedCommand::Meaning(meaning),
        traced,
        &mut prefixes,
        input,
        stores,
        execution,
    )?;
    let mut nest = ModeNest::new();
    let outcome = execute_prefixed_command(command, prefixes, &mut nest, input, stores, execution)?;
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

#[derive(Clone, Debug, Eq, PartialEq)]
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

    fn shipout(page: crate::dispatch::PreparedDviPage) -> Self {
        Self {
            assigned: false,
            action: DispatchAction::Shipout(page),
        }
    }
}

fn head_for_vmode(command: TracedTokenWord, input: &mut InputStack, stores: &mut Universe) {
    let par = Token::Cs(stores.intern("par").symbol());
    let origin = stores.inserted_origin(InsertedOriginKind::Paragraph, par, command.origin());
    push_traced_tokens(input, stores, [TracedTokenWord::pack(par, origin), command]);
}

fn off_save_alignment(
    command: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    // TeX.web's `off_save` chooses a recovery token that can actually close
    // the current group.  In particular, a semisimple group must be closed by
    // the inaccessible equivalent of `\endgroup`, not by a right brace.
    if stores.innermost_group_kind() == Some(GroupKind::SemiSimple) {
        push_traced_tokens(input, stores, [command]);
        leave_group_with_origin(input, stores, GroupKind::SemiSimple, command.origin())?;
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Missing \\endgroup inserted.\n",
        );
        return Ok(());
    }

    let closing_group = if stores.innermost_group_kind() == Some(GroupKind::MathShift) {
        Token::Char {
            ch: '$',
            cat: Catcode::MathShift,
        }
    } else {
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        }
    };
    let origin = stores.inserted_origin(
        InsertedOriginKind::ErrorRecovery,
        closing_group,
        command.origin(),
    );
    input.back_input_alignment_token(command);
    crate::insert_traced_tokens(
        input,
        stores,
        [TracedTokenWord::pack(closing_group, origin), command],
    );
    stores.world_mut().write_text(
        tex_state::PrintSink::TerminalAndLog,
        if matches!(
            closing_group,
            Token::Char {
                cat: Catcode::MathShift,
                ..
            }
        ) {
            "\n! Missing $ inserted.\n"
        } else {
            "\n! Missing } inserted.\n"
        },
    );
    Ok(())
}

fn accumulate_prefixes(
    mut command: PrefixedCommand,
    traced: TracedTokenWord,
    prefixes: &mut Prefixes,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<TracedPrefixedCommand, ExecError> {
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

        let traced = loop {
            let traced = get_command_token_with_context(
                input,
                &mut tex_state::ExpansionContext::new(stores),
                execution,
            )?
            .ok_or(ExecError::MissingPrefixedCommand)?;
            let token = tex_expand::semantic_token(traced);
            if is_space(token) {
                continue;
            }
            if let Token::Cs(symbol) = token
                && stores.meaning(symbol) == Meaning::Relax
            {
                continue;
            }
            break traced;
        };
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

fn execute_prefixed_command(
    command: TracedPrefixedCommand,
    mut prefixes: Prefixes,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<CommandOutcome, ExecError> {
    let accepts_macro_flags = matches!(
        command.command,
        PrefixedCommand::Primitive(
            UnexpandablePrimitive::Def
                | UnexpandablePrimitive::Edef
                | UnexpandablePrimitive::Gdef
                | UnexpandablePrimitive::Xdef
        )
    );
    if !accepts_macro_flags && prefixes.flags != MeaningFlags::EMPTY {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! You can't use `\\long' or `\\outer' with this command.\nI'll pretend you didn't say \\long or \\outer.\n",
        );
        prefixes.flags = MeaningFlags::EMPTY;
    }
    match command.command {
        PrefixedCommand::Primitive(primitive) => match primitive {
            UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef => {
                execute_def(primitive, prefixes, input, stores, execution)?;
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
                execute_globaldefs(prefixes, command.traced, input, stores, execution)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::BeginGroup => {
                reject_all_prefixes(prefixes)?;
                stores.enter_group_with_kind(GroupKind::SemiSimple);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::EndGroup => {
                reject_all_prefixes(prefixes)?;
                if let Err(error) = leave_group_with_origin(
                    input,
                    stores,
                    GroupKind::SemiSimple,
                    command.traced.origin(),
                ) {
                    if matches!(error, ExecError::ExtraEndGroup { .. }) {
                        stores.world_mut().write_text(
                            tex_state::PrintSink::TerminalAndLog,
                            "\n! Extra \\endgroup.\nThings are pretty mixed up, but I think the worst is over.\n",
                        );
                    } else {
                        return Err(error);
                    }
                }
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
                    execution,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::CountDef
            | UnexpandablePrimitive::DimenDef
            | UnexpandablePrimitive::SkipDef
            | UnexpandablePrimitive::MuskipDef
            | UnexpandablePrimitive::ToksDef => {
                execute_register_def(
                    primitive,
                    prefixes,
                    command.traced,
                    input,
                    stores,
                    execution,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::CharDef | UnexpandablePrimitive::MathCharDef => {
                execute_char_def(
                    primitive,
                    prefixes,
                    command.traced,
                    input,
                    stores,
                    execution,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Advance
            | UnexpandablePrimitive::Multiply
            | UnexpandablePrimitive::Divide => {
                execute_arithmetic(
                    primitive,
                    prefixes,
                    command.traced,
                    input,
                    stores,
                    execution,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::CatCode
            | UnexpandablePrimitive::LcCode
            | UnexpandablePrimitive::UcCode
            | UnexpandablePrimitive::SfCode
            | UnexpandablePrimitive::MathCode
            | UnexpandablePrimitive::DelCode => {
                execute_code_table_assignment(
                    primitive,
                    prefixes,
                    command.traced,
                    input,
                    stores,
                    execution,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Font => {
                execute_font_definition(prefixes, command.traced, input, stores, execution)?;
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
                    execution,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::FontDimen
            | UnexpandablePrimitive::HyphenChar
            | UnexpandablePrimitive::SkewChar => {
                let target =
                    scan_font_variable_target(primitive, command.traced, input, stores, execution)?;
                execute_assignment_to_target(
                    target,
                    prefixes,
                    command.traced,
                    input,
                    stores,
                    execution,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Patterns => {
                reject_all_prefixes(prefixes)?;
                execute_patterns(input, stores, execution)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Hyphenation => {
                reject_all_prefixes(prefixes)?;
                execute_hyphenation(input, stores, execution)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Par
            | UnexpandablePrimitive::EndGraf
            | UnexpandablePrimitive::Indent
            | UnexpandablePrimitive::NoIndent
            | UnexpandablePrimitive::ParShape
            | UnexpandablePrimitive::InterLinePenalties
            | UnexpandablePrimitive::ClubPenalties
            | UnexpandablePrimitive::WidowPenalties
            | UnexpandablePrimitive::DisplayWidowPenalties
            | UnexpandablePrimitive::PrevDepth
            | UnexpandablePrimitive::PrevGraf
            | UnexpandablePrimitive::NoInterlineSkip => {
                reject_macro_prefixes(prefixes)?;
                if prefixes.global
                    && matches!(
                        primitive,
                        UnexpandablePrimitive::PrevDepth
                            | UnexpandablePrimitive::PrevGraf
                            | UnexpandablePrimitive::NoInterlineSkip
                    )
                {
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        "\n! You can't use a prefix with this command.\nI'll pretend you didn't say \\global.\n",
                    );
                }
                execute_paragraph_command(
                    primitive,
                    command.traced,
                    nest,
                    input,
                    stores,
                    execution,
                    prefixes.global,
                )?;
                Ok(CommandOutcome::assigned_if(
                    primitive == UnexpandablePrimitive::ParShape
                        || matches!(
                            primitive,
                            UnexpandablePrimitive::InterLinePenalties
                                | UnexpandablePrimitive::ClubPenalties
                                | UnexpandablePrimitive::WidowPenalties
                                | UnexpandablePrimitive::DisplayWidowPenalties
                        )
                        || primitive == UnexpandablePrimitive::PrevDepth
                        || primitive == UnexpandablePrimitive::PrevGraf,
                ))
            }
            UnexpandablePrimitive::HAlign | UnexpandablePrimitive::VAlign => {
                reject_macro_prefixes(prefixes)?;
                if primitive == UnexpandablePrimitive::HAlign {
                    match nest.current_mode() {
                        crate::Mode::Horizontal => {
                            head_for_vmode(command.traced, input, stores);
                            return Ok(CommandOutcome::continue_only());
                        }
                        crate::Mode::RestrictedHorizontal => {
                            off_save_alignment(command.traced, input, stores)?;
                            return Ok(CommandOutcome::continue_only());
                        }
                        _ => {}
                    }
                } else if matches!(
                    nest.current_mode(),
                    crate::Mode::Vertical | crate::Mode::InternalVertical
                ) {
                    // TeX's main_control handles \valign through the hmode
                    // entry path: in vertical mode it starts an indented
                    // paragraph and retries the alignment there.
                    push_traced_tokens(input, stores, [command.traced]);
                    ensure_horizontal_for_character(nest, input, stores)?;
                    return Ok(CommandOutcome::continue_only());
                }
                crate::align::execute_alignment(
                    primitive,
                    command.traced,
                    nest,
                    input,
                    stores,
                    execution,
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
                    execution,
                )?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::SetBox => {
                reject_macro_prefixes(prefixes)?;
                execute_setbox(
                    prefixes.global,
                    command.traced,
                    nest,
                    input,
                    stores,
                    execution,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Box
            | UnexpandablePrimitive::Copy
            | UnexpandablePrimitive::UnHBox
            | UnexpandablePrimitive::UnHCopy
            | UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::UnVCopy
            | UnexpandablePrimitive::PageDiscards
            | UnexpandablePrimitive::SplitDiscards
            | UnexpandablePrimitive::LastBox
            | UnexpandablePrimitive::Raise
            | UnexpandablePrimitive::Lower
            | UnexpandablePrimitive::MoveLeft
            | UnexpandablePrimitive::MoveRight => {
                reject_all_prefixes(prefixes)?;
                if matches!(
                    primitive,
                    UnexpandablePrimitive::UnHBox | UnexpandablePrimitive::UnHCopy
                ) && matches!(
                    nest.current_mode(),
                    crate::Mode::Vertical | crate::Mode::InternalVertical
                ) {
                    // TeX82 enters an indented paragraph before it scans the
                    // box register. This remains observable for a void box:
                    // Plain's `\leavevmode` relies on the indent box to keep
                    // the otherwise-empty paragraph alive through `end_graf`.
                    push_traced_tokens(input, stores, [command.traced]);
                    ensure_horizontal_for_character(nest, input, stores)?;
                    return Ok(CommandOutcome::continue_only());
                }
                execute_box_list_command(
                    primitive,
                    command.traced,
                    nest,
                    input,
                    stores,
                    execution,
                )?;
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
                if matches!(
                    primitive,
                    UnexpandablePrimitive::VSkip
                        | UnexpandablePrimitive::VFil
                        | UnexpandablePrimitive::VFill
                        | UnexpandablePrimitive::VSs
                        | UnexpandablePrimitive::VFilNeg
                ) {
                    match nest.current_mode() {
                        crate::Mode::RestrictedHorizontal => {
                            // TeX.web §1091 `head_for_vmode` invokes
                            // off_save, which closes the hbox and retries the
                            // vertical command in the enclosing mode.
                            off_save_alignment(command.traced, input, stores)?;
                            return Ok(CommandOutcome::continue_only());
                        }
                        crate::Mode::Math | crate::Mode::DisplayMath => {
                            diagnostics::report_illegal_case(
                                stores,
                                command.token,
                                nest.current_mode(),
                            );
                            return Ok(CommandOutcome::continue_only());
                        }
                        _ => {}
                    }
                }
                if primitive == UnexpandablePrimitive::HSkip
                    && matches!(
                        nest.current_mode(),
                        crate::Mode::Vertical | crate::Mode::InternalVertical
                    )
                {
                    // TeX82's vertical-mode main-control case backs up the
                    // triggering command before `new_graf`. In particular,
                    // `every_par` must run before an `\hskip` scans its glue.
                    push_traced_tokens(input, stores, [command.traced]);
                    ensure_horizontal_for_character(nest, input, stores)?;
                    return Ok(CommandOutcome::continue_only());
                }
                execute_kern_or_skip(primitive, command.traced, nest, input, stores, execution)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Leaders
            | UnexpandablePrimitive::CLeaders
            | UnexpandablePrimitive::XLeaders => {
                reject_all_prefixes(prefixes)?;
                execute_leaders(primitive, command.traced, nest, input, stores, execution)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::HRule => {
                reject_all_prefixes(prefixes)?;
                if nest.current_mode() == crate::Mode::Horizontal {
                    // TeX.web's head_for_vmode ends the paragraph as a
                    // separate main-control step, then retries the rule. A
                    // page break found while ending the paragraph must fire
                    // before the rule is added to the contribution list.
                    head_for_vmode(command.traced, input, stores);
                    return Ok(CommandOutcome::continue_only());
                }
                execute_hrule(command.traced, nest, input, stores, execution)?;
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
            | UnexpandablePrimitive::Marks
            | UnexpandablePrimitive::VAdjust
            | UnexpandablePrimitive::Insert => {
                reject_all_prefixes(prefixes)?;
                if primitive == UnexpandablePrimitive::Accent
                    && matches!(
                        nest.current_mode(),
                        crate::Mode::Vertical | crate::Mode::InternalVertical
                    )
                {
                    // TeX82 backs up a vertical-mode accent before `new_graf`,
                    // so `every_par` runs before the accent scans its number
                    // and base character in horizontal mode.
                    push_traced_tokens(input, stores, [command.traced]);
                    ensure_horizontal_for_character(nest, input, stores)?;
                    return Ok(CommandOutcome::continue_only());
                }
                if matches!(
                    primitive,
                    UnexpandablePrimitive::Discretionary
                        | UnexpandablePrimitive::DiscretionaryHyphen
                ) && matches!(
                    nest.current_mode(),
                    crate::Mode::Vertical | crate::Mode::InternalVertical
                ) {
                    push_traced_tokens(input, stores, [command.traced]);
                    ensure_horizontal_for_character(nest, input, stores)?;
                    return Ok(CommandOutcome::continue_only());
                }
                execute_hmode_material(command.traced, primitive, nest, input, stores, execution)?;
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
                    execution,
                )?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Read | UnexpandablePrimitive::ReadLine => {
                execute_read(primitive, command.traced, input, stores, execution)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Write => {
                execute_write(command.traced, nest, input, stores, execution)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Special => {
                reject_all_prefixes(prefixes)?;
                execute_special(command.traced, nest, input, stores, execution)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::SetLanguage => {
                reject_all_prefixes(prefixes)?;
                if !matches!(
                    nest.current_mode(),
                    crate::Mode::Horizontal | crate::Mode::RestrictedHorizontal
                ) {
                    return Err(ExecError::UnimplementedTypesetting {
                        mode: nest.current_mode(),
                        token: command.token,
                        origin: command.origin,
                        operation: "setlanguage outside horizontal mode",
                    });
                }
                let language = scan_i32(input, stores, execution, command.traced)?;
                let language = u8::try_from(language).unwrap_or(0);
                hmode::flush_pending_hchars(nest, stores)?;
                let normalize_min = |value: i32| u8::try_from(value.clamp(1, 63)).unwrap_or(1);
                let left_hyphen_min = normalize_min(stores.int_param(IntParam::LEFT_HYPHEN_MIN));
                let right_hyphen_min = normalize_min(stores.int_param(IntParam::RIGHT_HYPHEN_MIN));
                crate::vertical::append_node_to_current_list(
                    nest,
                    stores,
                    tex_state::node::Node::Whatsit(tex_state::node::Whatsit::Language {
                        language,
                        left_hyphen_min,
                        right_hyphen_min,
                    }),
                )?;
                nest.current_list_mut().set_hyphen_language(language);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Shipout => {
                reject_all_prefixes(prefixes)?;
                match execute_shipout(command.traced, input, stores, execution)? {
                    Some(page) => Ok(CommandOutcome::shipout(page)),
                    None => Ok(CommandOutcome::continue_only()),
                }
            }
            UnexpandablePrimitive::OpenIn
            | UnexpandablePrimitive::CloseIn
            | UnexpandablePrimitive::OpenOut
            | UnexpandablePrimitive::CloseOut => {
                execute_stream_command(primitive, command.traced, nest, input, stores, execution)?;
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::Show => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_show(input, stores)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowBox => {
                reject_all_prefixes(prefixes)?;
                let index = scan_register_index(input, stores, execution, command.traced)?;
                diagnostics::execute_showbox(stores, index);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowThe => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showthe(command.traced, input, stores, execution)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowTokens => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showtokens(command.traced, input, stores, execution)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowGroups => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showgroups(stores);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowIfs => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showifs(input, stores);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Message => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_message(input, stores, execution, false)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ErrMessage => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_message(input, stores, execution, true)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowLists => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showlists(stores, nest);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::ShowHyphens => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_showhyphens(input, stores, execution)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Uppercase => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_change_case(input, stores, execution, true)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Lowercase => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_change_case(input, stores, execution, false)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::IgnoreSpaces => {
                reject_all_prefixes(prefixes)?;
                diagnostics::execute_ignorespaces(input, stores)?;
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::InteractionMode => {
                reject_macro_prefixes(prefixes)?;
                skip_optional_equals_x(input, stores, execution)?;
                let value = scan_i32(input, stores, execution, command.traced)?;
                let mode = match value {
                    0 => InteractionMode::Batch,
                    1 => InteractionMode::Nonstop,
                    2 => InteractionMode::Scroll,
                    3 => InteractionMode::ErrorStop,
                    value => {
                        diagnostics::report_bad_interaction_mode(stores, value);
                        return Ok(CommandOutcome::continue_only());
                    }
                };
                stores.set_interaction_mode(mode);
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::BeginL
            | UnexpandablePrimitive::EndL
            | UnexpandablePrimitive::BeginR
            | UnexpandablePrimitive::EndR => {
                reject_all_prefixes(prefixes)?;
                if stores.int_param(IntParam::TEX_XET_STATE) <= 0 {
                    let name = match primitive {
                        UnexpandablePrimitive::BeginL => "beginL",
                        UnexpandablePrimitive::EndL => "endL",
                        UnexpandablePrimitive::BeginR => "beginR",
                        UnexpandablePrimitive::EndR => "endR",
                        _ => unreachable!(),
                    };
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        &format!("\n! Improper \\{name}.\nSorry, this \\{name} will be ignored.\n"),
                    );
                    return Ok(CommandOutcome::continue_only());
                }
                if !matches!(
                    nest.current_mode(),
                    crate::Mode::Horizontal | crate::Mode::RestrictedHorizontal
                ) {
                    crate::diagnostics::report_illegal_case(
                        stores,
                        command.token,
                        nest.current_mode(),
                    );
                    return Ok(CommandOutcome::continue_only());
                }
                flush_pending_hchars(nest, stores)?;
                let direction = match primitive {
                    UnexpandablePrimitive::BeginL => tex_state::node::Direction::BeginL,
                    UnexpandablePrimitive::EndL => tex_state::node::Direction::EndL,
                    UnexpandablePrimitive::BeginR => tex_state::node::Direction::BeginR,
                    UnexpandablePrimitive::EndR => tex_state::node::Direction::EndR,
                    _ => unreachable!(),
                };
                nest.current_list_mut().push(Node::Direction(direction));
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::BatchMode
            | UnexpandablePrimitive::NonstopMode
            | UnexpandablePrimitive::ScrollMode
            | UnexpandablePrimitive::ErrorStopMode => {
                reject_macro_prefixes(prefixes)?;
                let mode = match primitive {
                    UnexpandablePrimitive::BatchMode => InteractionMode::Batch,
                    UnexpandablePrimitive::NonstopMode => InteractionMode::Nonstop,
                    UnexpandablePrimitive::ScrollMode => InteractionMode::Scroll,
                    UnexpandablePrimitive::ErrorStopMode => InteractionMode::ErrorStop,
                    _ => unreachable!("interaction primitive matched above"),
                };
                stores.set_interaction_mode(mode);
                Ok(CommandOutcome::assigned())
            }
            UnexpandablePrimitive::MathChar
            | UnexpandablePrimitive::Delimiter
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
            | UnexpandablePrimitive::Middle
            | UnexpandablePrimitive::DisplayStyle
            | UnexpandablePrimitive::TextStyle
            | UnexpandablePrimitive::ScriptStyle
            | UnexpandablePrimitive::ScriptScriptStyle => {
                reject_all_prefixes(prefixes)?;
                // These are the `non_math` cases in tex.web §1043 and
                // §1147: insert `$`, then reconsider the original command in
                // math mode instead of consuming it or aborting execution.
                crate::math::insert_dollar_sign(command.traced, input, stores);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::EqNo
            | UnexpandablePrimitive::LeftEqNo
            | UnexpandablePrimitive::FontCharWd
            | UnexpandablePrimitive::FontCharHt
            | UnexpandablePrimitive::FontCharDp
            | UnexpandablePrimitive::FontCharIc
            | UnexpandablePrimitive::ParShapeLength
            | UnexpandablePrimitive::ParShapeIndent
            | UnexpandablePrimitive::ParShapeDimen
            | UnexpandablePrimitive::NumExpr
            | UnexpandablePrimitive::DimExpr
            | UnexpandablePrimitive::GlueExpr
            | UnexpandablePrimitive::MuExpr
            | UnexpandablePrimitive::GlueStretch
            | UnexpandablePrimitive::GlueShrink
            | UnexpandablePrimitive::GlueStretchOrder
            | UnexpandablePrimitive::GlueShrinkOrder
            | UnexpandablePrimitive::GlueToMu
            | UnexpandablePrimitive::MuToGlue => {
                reject_all_prefixes(prefixes)?;
                crate::diagnostics::report_illegal_case(stores, command.token, nest.current_mode());
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::MathOrd
            | UnexpandablePrimitive::MathOp
            | UnexpandablePrimitive::MathBin
            | UnexpandablePrimitive::MathRel
            | UnexpandablePrimitive::MathOpen
            | UnexpandablePrimitive::MathClose
            | UnexpandablePrimitive::MathPunct
            | UnexpandablePrimitive::MathInner
            | UnexpandablePrimitive::Underline
            | UnexpandablePrimitive::Overline => {
                reject_all_prefixes(prefixes)?;
                crate::math::insert_dollar_sign(command.traced, input, stores);
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::NoAlign | UnexpandablePrimitive::Omit => {
                let name = if primitive == UnexpandablePrimitive::NoAlign {
                    "noalign"
                } else {
                    "omit"
                };
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    &format!("\n! Misplaced \\{name}.\nI expect to see \\{name} only after the \\cr of an alignment.\n"),
                );
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Cr
            | UnexpandablePrimitive::CrCr
            | UnexpandablePrimitive::Span => {
                let name = match primitive {
                    UnexpandablePrimitive::Cr => "cr",
                    UnexpandablePrimitive::CrCr => "crcr",
                    UnexpandablePrimitive::Span => "span",
                    _ => unreachable!(),
                };
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    &format!("\n! Misplaced \\{name}.\nI can't figure out why you would want to use this alignment command here.\n"),
                );
                Ok(CommandOutcome::continue_only())
            }
            UnexpandablePrimitive::Global
            | UnexpandablePrimitive::Long
            | UnexpandablePrimitive::Outer
            | UnexpandablePrimitive::Protected
            | UnexpandablePrimitive::Immediate
            | UnexpandablePrimitive::End
            | UnexpandablePrimitive::Dump => unreachable!("prefixes are accumulated first"),
        },
        PrefixedCommand::Meaning(meaning) => {
            reject_macro_prefixes(prefixes)?;
            let target =
                variable_from_meaning(meaning).ok_or(ExecError::UnsupportedAssignmentTarget)?;
            execute_assignment_to_target(
                target,
                prefixes,
                command.traced,
                input,
                stores,
                execution,
            )?;
            Ok(CommandOutcome::assigned())
        }
    }
}
