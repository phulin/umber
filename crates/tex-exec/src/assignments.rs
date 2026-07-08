//! Assignment primitives owned by main control.

use tex_expand::scan::{scan_toks, scan_toks_expanded};
use tex_expand::{
    ExpandError, ExpansionHooks, NoopRecorder, get_x_token_with_recorder_and_hooks, scan_dimen,
    scan_glue, scan_int,
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
        ("begingroup", UnexpandablePrimitive::BeginGroup),
        ("endgroup", UnexpandablePrimitive::EndGroup),
        ("aftergroup", UnexpandablePrimitive::AfterGroup),
        ("afterassignment", UnexpandablePrimitive::AfterAssignment),
        ("long", UnexpandablePrimitive::Long),
        ("outer", UnexpandablePrimitive::Outer),
        ("protected", UnexpandablePrimitive::Protected),
        ("count", UnexpandablePrimitive::Count),
        ("dimen", UnexpandablePrimitive::Dimen),
        ("skip", UnexpandablePrimitive::Skip),
        ("muskip", UnexpandablePrimitive::Muskip),
        ("toks", UnexpandablePrimitive::Toks),
        ("countdef", UnexpandablePrimitive::CountDef),
        ("dimendef", UnexpandablePrimitive::DimenDef),
        ("skipdef", UnexpandablePrimitive::SkipDef),
        ("muskipdef", UnexpandablePrimitive::MuskipDef),
        ("toksdef", UnexpandablePrimitive::ToksDef),
        ("chardef", UnexpandablePrimitive::CharDef),
        ("mathchardef", UnexpandablePrimitive::MathCharDef),
        ("advance", UnexpandablePrimitive::Advance),
        ("multiply", UnexpandablePrimitive::Multiply),
        ("divide", UnexpandablePrimitive::Divide),
        ("catcode", UnexpandablePrimitive::CatCode),
        ("lccode", UnexpandablePrimitive::LcCode),
        ("uccode", UnexpandablePrimitive::UcCode),
        ("sfcode", UnexpandablePrimitive::SfCode),
        ("mathcode", UnexpandablePrimitive::MathCode),
        ("delcode", UnexpandablePrimitive::DelCode),
        ("read", UnexpandablePrimitive::Read),
        ("show", UnexpandablePrimitive::Show),
        ("showthe", UnexpandablePrimitive::ShowThe),
        ("showtokens", UnexpandablePrimitive::ShowTokens),
        ("message", UnexpandablePrimitive::Message),
        ("errmessage", UnexpandablePrimitive::ErrMessage),
        ("showlists", UnexpandablePrimitive::ShowLists),
        ("uppercase", UnexpandablePrimitive::Uppercase),
        ("lowercase", UnexpandablePrimitive::Lowercase),
        ("ignorespaces", UnexpandablePrimitive::IgnoreSpaces),
        ("end", UnexpandablePrimitive::End),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    install_parameter_meanings(stores);
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

fn execute_aftergroup<S>(input: &mut InputStack<S>, stores: &mut Stores) -> Result<(), ExecError>
where
    S: InputSource,
{
    let token = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\aftergroup",
    })?;
    stores.push_aftergroup(token);
    Ok(())
}

fn execute_afterassignment<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let token = input.next_token(stores)?.ok_or(ExecError::MissingToken {
        context: "\\afterassignment",
    })?;
    stores.set_afterassignment(token);
    Ok(())
}

fn fire_afterassignment<S>(input: &mut InputStack<S>, stores: &mut Stores)
where
    S: InputSource,
{
    if let Some(token) = stores.take_afterassignment() {
        push_tokens(input, stores, [token]);
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Variable {
    IntRegister(u16),
    DimenRegister(u16),
    GlueRegister(u16),
    MuGlueRegister(u16),
    ToksRegister(u16),
    IntParam(u16),
    DimenParam(u16),
    GlueParam(u16),
    TokParam(u16),
}

fn execute_variable_assignment<S, H>(
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
    reject_macro_prefixes(prefixes)?;
    let index = scan_register_index(input, stores, hooks)?;
    let target = match primitive {
        UnexpandablePrimitive::Count => Variable::IntRegister(index),
        UnexpandablePrimitive::Dimen => Variable::DimenRegister(index),
        UnexpandablePrimitive::Skip => Variable::GlueRegister(index),
        UnexpandablePrimitive::Muskip => Variable::MuGlueRegister(index),
        UnexpandablePrimitive::Toks => Variable::ToksRegister(index),
        _ => unreachable!("caller restricts primitive"),
    };
    execute_assignment_to_target(target, prefixes, input, stores, hooks)
}

fn execute_assignment_to_target<S, H>(
    target: Variable,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    skip_optional_equals_x(input, stores, hooks)?;
    let global = apply_globaldefs(prefixes.global, stores);
    match target {
        Variable::IntRegister(index) => {
            let value = scan_i32(input, stores, hooks)?;
            set_int_register(stores, index, value, global);
        }
        Variable::DimenRegister(index) => {
            let value = scan_scaled(input, stores, hooks)?;
            set_dimen_register(stores, index, value, global);
        }
        Variable::GlueRegister(index) => {
            let value = scan_glue_id(input, stores, hooks, false)?;
            set_glue_register(stores, index, value, global);
        }
        Variable::MuGlueRegister(index) => {
            let value = scan_glue_id(input, stores, hooks, true)?;
            set_muglue_register(stores, index, value, global);
        }
        Variable::ToksRegister(index) => {
            let value = scan_token_list_assignment(input, stores, hooks)?;
            set_toks_register(stores, index, value, global);
        }
        Variable::IntParam(index) => {
            let value = scan_i32(input, stores, hooks)?;
            set_int_param(stores, index, value, global);
        }
        Variable::DimenParam(index) => {
            let value = scan_scaled(input, stores, hooks)?;
            set_dimen_param(stores, index, value, global);
        }
        Variable::GlueParam(index) => {
            let value = scan_glue_id(input, stores, hooks, false)?;
            set_glue_param(stores, index, value, global);
        }
        Variable::TokParam(index) => {
            let value = scan_token_list_assignment(input, stores, hooks)?;
            set_tok_param(stores, index, value, global);
        }
    }
    Ok(())
}

fn execute_register_def<S, H>(
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
    reject_macro_prefixes(prefixes)?;
    let target = scan_control_sequence(input, stores, "register definition")?;
    skip_optional_equals_x(input, stores, hooks)?;
    let index = scan_register_index(input, stores, hooks)?;
    let meaning = match primitive {
        UnexpandablePrimitive::CountDef => Meaning::CountRegister(index),
        UnexpandablePrimitive::DimenDef => Meaning::DimenRegister(index),
        UnexpandablePrimitive::SkipDef => Meaning::SkipRegister(index),
        UnexpandablePrimitive::MuskipDef => Meaning::MuskipRegister(index),
        UnexpandablePrimitive::ToksDef => Meaning::ToksRegister(index),
        _ => unreachable!("caller restricts primitive"),
    };
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    Ok(())
}

fn execute_char_def<S, H>(
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
    reject_macro_prefixes(prefixes)?;
    let target = scan_control_sequence(input, stores, "character definition")?;
    skip_optional_equals_x(input, stores, hooks)?;
    let value = scan_i32(input, stores, hooks)?;
    let meaning = match primitive {
        UnexpandablePrimitive::CharDef => {
            if !(0..=255).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\chardef",
                    value,
                });
            }
            let ch = char::from_u32(value as u32).expect("0..=255 is Unicode scalar");
            Meaning::CharGiven(ch)
        }
        UnexpandablePrimitive::MathCharDef => {
            if !(0..=32_767).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\mathchardef",
                    value,
                });
            }
            Meaning::MathCharGiven(value as u16)
        }
        _ => unreachable!("caller restricts primitive"),
    };
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, meaning);
    } else {
        stores.set_meaning(target, meaning);
    }
    Ok(())
}

fn execute_arithmetic<S, H>(
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
    reject_macro_prefixes(prefixes)?;
    let target = scan_variable_target(input, stores, hooks)?;
    let _ = scan_optional_keyword_x(input, stores, hooks, "by")?;
    let global = apply_globaldefs(prefixes.global, stores);
    match target {
        Variable::IntRegister(index) | Variable::IntParam(index) => {
            let old = read_int_variable(stores, target);
            let rhs = scan_i32(input, stores, hooks)?;
            let value = arithmetic_i32(primitive, old, rhs)?;
            write_int_variable(stores, target, index, value, global);
        }
        Variable::DimenRegister(index) | Variable::DimenParam(index) => {
            let old = read_dimen_variable(stores, target);
            let value = match primitive {
                UnexpandablePrimitive::Advance => old
                    .checked_add(scan_scaled(input, stores, hooks)?)
                    .ok_or(ExecError::ArithmeticOverflow)?,
                UnexpandablePrimitive::Multiply => {
                    scaled_checked_mul(old, scan_i32(input, stores, hooks)?)?
                }
                UnexpandablePrimitive::Divide => {
                    scaled_checked_div(old, scan_nonzero_i32(input, stores, hooks)?)?
                }
                _ => unreachable!("caller restricts primitive"),
            };
            write_dimen_variable(stores, target, index, value, global);
        }
        Variable::GlueRegister(index) | Variable::GlueParam(index) => {
            let old = stores.glue(read_glue_variable(stores, target));
            let rhs = scan_glue_or_factor(primitive, input, stores, hooks, false)?;
            let value = arithmetic_glue(primitive, old, rhs)?;
            let id = stores.intern_glue(value);
            write_glue_variable(stores, target, index, id, global);
        }
        Variable::MuGlueRegister(index) => {
            let old = stores.glue(stores.muskip(index));
            let rhs = scan_glue_or_factor(primitive, input, stores, hooks, true)?;
            let value = arithmetic_glue(primitive, old, rhs)?;
            let id = stores.intern_glue(value);
            set_muglue_register(stores, index, id, global);
        }
        Variable::ToksRegister(_) | Variable::TokParam(_) => {
            return Err(ExecError::UnsupportedAssignmentTarget);
        }
    }
    Ok(())
}

fn execute_code_table_assignment<S, H>(
    primitive: UnexpandablePrimitive,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let code = scan_i32(input, stores, hooks)?;
    skip_optional_equals_x(input, stores, hooks)?;
    let value = scan_i32(input, stores, hooks)?;
    let ch = char_from_code(code, "code-table character")?;
    match primitive {
        UnexpandablePrimitive::CatCode => stores.set_catcode(ch, catcode_from_i32(value)?),
        UnexpandablePrimitive::LcCode => {
            stores.set_lccode(ch, checked_char_code(value, "\\lccode")? as LcCode)
        }
        UnexpandablePrimitive::UcCode => {
            stores.set_uccode(ch, checked_char_code(value, "\\uccode")? as UcCode)
        }
        UnexpandablePrimitive::SfCode => {
            if !(0..=32_767).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\sfcode",
                    value,
                });
            }
            stores.set_sfcode(ch, value as SfCode);
        }
        UnexpandablePrimitive::MathCode => {
            if !(0..=32_768).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\mathcode",
                    value,
                });
            }
            stores.set_mathcode(ch, value as MathCode);
        }
        UnexpandablePrimitive::DelCode => {
            if !(-1..=0xFF_FFFF).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\delcode",
                    value,
                });
            }
            stores.set_delcode(ch, value as DelCode);
        }
        _ => unreachable!("caller restricts primitive"),
    }
    Ok(())
}

fn execute_read_stub<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let _stream = scan_i32(input, stores, hooks)?;
    if !scan_optional_keyword_x(input, stores, hooks, "to")? {
        return Err(ExecError::ReadNeedsTo);
    }
    let _target = scan_control_sequence(input, stores, "\\read")?;
    Err(ExecError::ReadNotImplemented)
}

fn is_assignment_meaning(meaning: Meaning) -> bool {
    match meaning {
        Meaning::UnexpandablePrimitive(primitive) => matches!(
            primitive,
            UnexpandablePrimitive::Def
                | UnexpandablePrimitive::Edef
                | UnexpandablePrimitive::Gdef
                | UnexpandablePrimitive::Xdef
                | UnexpandablePrimitive::Let
                | UnexpandablePrimitive::FutureLet
                | UnexpandablePrimitive::GlobalDefs
                | UnexpandablePrimitive::Global
                | UnexpandablePrimitive::BeginGroup
                | UnexpandablePrimitive::EndGroup
                | UnexpandablePrimitive::AfterGroup
                | UnexpandablePrimitive::AfterAssignment
                | UnexpandablePrimitive::Long
                | UnexpandablePrimitive::Outer
                | UnexpandablePrimitive::Protected
                | UnexpandablePrimitive::Count
                | UnexpandablePrimitive::Dimen
                | UnexpandablePrimitive::Skip
                | UnexpandablePrimitive::Muskip
                | UnexpandablePrimitive::Toks
                | UnexpandablePrimitive::CountDef
                | UnexpandablePrimitive::DimenDef
                | UnexpandablePrimitive::SkipDef
                | UnexpandablePrimitive::MuskipDef
                | UnexpandablePrimitive::ToksDef
                | UnexpandablePrimitive::CharDef
                | UnexpandablePrimitive::MathCharDef
                | UnexpandablePrimitive::Advance
                | UnexpandablePrimitive::Multiply
                | UnexpandablePrimitive::Divide
                | UnexpandablePrimitive::CatCode
                | UnexpandablePrimitive::LcCode
                | UnexpandablePrimitive::UcCode
                | UnexpandablePrimitive::SfCode
                | UnexpandablePrimitive::MathCode
                | UnexpandablePrimitive::DelCode
                | UnexpandablePrimitive::Read
        ),
        meaning => is_assignment_target_meaning(meaning),
    }
}

pub(crate) fn is_assignment_target_meaning(meaning: Meaning) -> bool {
    matches!(
        meaning,
        Meaning::CountRegister(_)
            | Meaning::DimenRegister(_)
            | Meaning::SkipRegister(_)
            | Meaning::MuskipRegister(_)
            | Meaning::ToksRegister(_)
            | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::TokParam(_)
    )
}

fn variable_from_meaning(meaning: Meaning) -> Option<Variable> {
    match meaning {
        Meaning::CountRegister(index) => Some(Variable::IntRegister(index)),
        Meaning::DimenRegister(index) => Some(Variable::DimenRegister(index)),
        Meaning::SkipRegister(index) => Some(Variable::GlueRegister(index)),
        Meaning::MuskipRegister(index) => Some(Variable::MuGlueRegister(index)),
        Meaning::ToksRegister(index) => Some(Variable::ToksRegister(index)),
        Meaning::IntParam(index) => Some(Variable::IntParam(index)),
        Meaning::DimenParam(index) => Some(Variable::DimenParam(index)),
        Meaning::GlueParam(index) => Some(Variable::GlueParam(index)),
        Meaning::TokParam(index) => Some(Variable::TokParam(index)),
        _ => None,
    }
}

fn scan_variable_target<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<Variable, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let token = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "arithmetic target",
    })?;
    let Token::Cs(symbol) = token else {
        return Err(ExecError::ExpectedControlSequence {
            context: "arithmetic target",
            token,
        });
    };
    let meaning = stores.meaning(symbol);
    match meaning {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => Ok(Variable::IntRegister(
            scan_register_index_with_recorder(input, stores, &mut recorder, hooks)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            Ok(Variable::DimenRegister(scan_register_index_with_recorder(
                input,
                stores,
                &mut recorder,
                hooks,
            )?))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => Ok(Variable::GlueRegister(
            scan_register_index_with_recorder(input, stores, &mut recorder, hooks)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
            Ok(Variable::MuGlueRegister(scan_register_index_with_recorder(
                input,
                stores,
                &mut recorder,
                hooks,
            )?))
        }
        meaning => variable_from_meaning(meaning).ok_or(ExecError::UnsupportedAssignmentTarget),
    }
}

fn scan_register_index<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<u16, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    scan_register_index_with_recorder(input, stores, &mut NoopRecorder, hooks)
}

fn scan_register_index_with_recorder<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<u16, ExecError>
where
    S: InputSource,
    R: tex_expand::ReadRecorder,
    H: ExpansionHooks<S>,
{
    let value = scan_int::scan_int_with_recorder_and_hooks(input, stores, recorder, hooks)
        .map_err(ExpandError::from)?
        .value();
    if !(0..=32_767).contains(&value) {
        return Err(ExecError::RegisterNumberOutOfRange(value));
    }
    Ok(value as u16)
}

fn scan_i32<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<i32, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    Ok(
        scan_int::scan_int_with_recorder_and_hooks(input, stores, &mut recorder, hooks)
            .map_err(ExpandError::from)?
            .value(),
    )
}

fn scan_nonzero_i32<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<i32, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let value = scan_i32(input, stores, hooks)?;
    if value == 0 {
        Err(ExecError::ArithmeticOverflow)
    } else {
        Ok(value)
    }
}

fn scan_scaled<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<Scaled, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    Ok(scan_dimen::scan_dimen_with_options_and_hooks(
        input,
        stores,
        &mut recorder,
        hooks,
        scan_dimen::ScanDimenOptions::STANDARD,
    )
    .map_err(ExpandError::from)?
    .value())
}

fn scan_glue_id<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
    mu: bool,
) -> Result<GlueId, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    Ok(
        scan_glue::scan_glue_with_hooks(input, stores, &mut recorder, hooks, mu)
            .map_err(ExecError::ScanGlue)?
            .id(),
    )
}

fn scan_token_list_assignment<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<tex_state::ids::TokenListId, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let token = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "token-list assignment",
    })?;
    if let Token::Cs(symbol) = token {
        match stores.meaning(symbol) {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
                let index = scan_register_index(input, stores, hooks)?;
                return Ok(stores.toks(index));
            }
            Meaning::ToksRegister(index) => return Ok(stores.toks(index)),
            Meaning::TokParam(index) => return Ok(stores.tok_param(TokParam::new(index))),
            _ => {}
        }
    }
    if !is_begin_group(token) {
        return Err(ExecError::MissingToken {
            context: "token-list assignment group",
        });
    }
    scan_balanced_text_after_open_group(input, stores)
}

fn scan_balanced_text_after_open_group<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<tex_state::ids::TokenListId, ExecError>
where
    S: InputSource,
{
    let mut depth = 1usize;
    let mut builder = stores.token_list_builder();
    while let Some(token) = input.next_token(stores)? {
        match token {
            token if is_begin_group(token) => {
                depth += 1;
                builder.push(token);
            }
            token if is_end_group(token) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(stores.finish_token_list(&mut builder));
                }
                builder.push(token);
            }
            token => builder.push(token),
        }
    }
    Err(ExecError::MissingToken {
        context: "token-list closing brace",
    })
}

fn next_non_space_x<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<Option<Token>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
        else {
            return Ok(None);
        };
        if !is_space(token) {
            return Ok(Some(token));
        }
    }
}

fn scan_optional_keyword_x<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
    keyword: &str,
) -> Result<bool, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let mut consumed = Vec::new();
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
        else {
            return Ok(false);
        };
        if !is_space(token) {
            consumed.push(token);
            break;
        }
    }
    for (offset, expected) in keyword.bytes().enumerate() {
        let token = if offset == 0 {
            consumed[0]
        } else {
            let Some(token) =
                get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
            else {
                push_tokens(input, stores, consumed);
                return Ok(false);
            };
            consumed.push(token);
            token
        };
        if !token_matches_keyword_byte(token, expected) {
            push_tokens(input, stores, consumed);
            return Ok(false);
        }
    }
    Ok(true)
}

fn arithmetic_i32(primitive: UnexpandablePrimitive, old: i32, rhs: i32) -> Result<i32, ExecError> {
    match primitive {
        UnexpandablePrimitive::Advance => old.checked_add(rhs),
        UnexpandablePrimitive::Multiply => old.checked_mul(rhs),
        UnexpandablePrimitive::Divide => {
            if rhs == 0 {
                None
            } else {
                old.checked_div(rhs)
            }
        }
        _ => unreachable!("caller restricts primitive"),
    }
    .ok_or(ExecError::ArithmeticOverflow)
}

#[derive(Clone, Copy, Debug)]
enum GlueArithmeticRhs {
    Glue(GlueSpec),
    Factor(i32),
}

fn scan_glue_or_factor<S, H>(
    primitive: UnexpandablePrimitive,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
    mu: bool,
) -> Result<GlueArithmeticRhs, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    match primitive {
        UnexpandablePrimitive::Advance => {
            let id = scan_glue_id(input, stores, hooks, mu)?;
            Ok(GlueArithmeticRhs::Glue(stores.glue(id)))
        }
        UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
            Ok(GlueArithmeticRhs::Factor(scan_i32(input, stores, hooks)?))
        }
        _ => unreachable!("caller restricts primitive"),
    }
}

fn arithmetic_glue(
    primitive: UnexpandablePrimitive,
    old: GlueSpec,
    rhs: GlueArithmeticRhs,
) -> Result<GlueSpec, ExecError> {
    match (primitive, rhs) {
        (UnexpandablePrimitive::Advance, GlueArithmeticRhs::Glue(rhs)) => add_glue(old, rhs),
        (UnexpandablePrimitive::Multiply, GlueArithmeticRhs::Factor(rhs)) => {
            multiply_glue(old, rhs)
        }
        (UnexpandablePrimitive::Divide, GlueArithmeticRhs::Factor(rhs)) => divide_glue(old, rhs),
        _ => unreachable!("caller restricts primitive/rhs"),
    }
}

fn add_glue(left: GlueSpec, right: GlueSpec) -> Result<GlueSpec, ExecError> {
    Ok(GlueSpec {
        width: left
            .width
            .checked_add(right.width)
            .ok_or(ExecError::ArithmeticOverflow)?,
        stretch: add_ordered_component(
            left.stretch,
            left.stretch_order,
            right.stretch,
            right.stretch_order,
        )?
        .0,
        stretch_order: add_ordered_component(
            left.stretch,
            left.stretch_order,
            right.stretch,
            right.stretch_order,
        )?
        .1,
        shrink: add_ordered_component(
            left.shrink,
            left.shrink_order,
            right.shrink,
            right.shrink_order,
        )?
        .0,
        shrink_order: add_ordered_component(
            left.shrink,
            left.shrink_order,
            right.shrink,
            right.shrink_order,
        )?
        .1,
    })
}

fn add_ordered_component(
    left: Scaled,
    left_order: Order,
    right: Scaled,
    right_order: Order,
) -> Result<(Scaled, Order), ExecError> {
    if left_order == right_order {
        return Ok((
            left.checked_add(right)
                .ok_or(ExecError::ArithmeticOverflow)?,
            left_order,
        ));
    }
    if left_order > right_order {
        Ok((left, left_order))
    } else {
        Ok((right, right_order))
    }
}

fn multiply_glue(spec: GlueSpec, factor: i32) -> Result<GlueSpec, ExecError> {
    Ok(GlueSpec {
        width: scaled_checked_mul(spec.width, factor)?,
        stretch: scaled_checked_mul(spec.stretch, factor)?,
        stretch_order: spec.stretch_order,
        shrink: scaled_checked_mul(spec.shrink, factor)?,
        shrink_order: spec.shrink_order,
    })
}

fn divide_glue(spec: GlueSpec, divisor: i32) -> Result<GlueSpec, ExecError> {
    if divisor == 0 {
        return Err(ExecError::ArithmeticOverflow);
    }
    Ok(GlueSpec {
        width: scaled_checked_div(spec.width, divisor)?,
        stretch: scaled_checked_div(spec.stretch, divisor)?,
        stretch_order: spec.stretch_order,
        shrink: scaled_checked_div(spec.shrink, divisor)?,
        shrink_order: spec.shrink_order,
    })
}

fn scaled_checked_mul(value: Scaled, factor: i32) -> Result<Scaled, ExecError> {
    value
        .raw()
        .checked_mul(factor)
        .map(Scaled::from_raw)
        .ok_or(ExecError::ArithmeticOverflow)
}

fn scaled_checked_div(value: Scaled, divisor: i32) -> Result<Scaled, ExecError> {
    value
        .raw()
        .checked_div(divisor)
        .map(Scaled::from_raw)
        .ok_or(ExecError::ArithmeticOverflow)
}

fn read_int_variable(stores: &Stores, target: Variable) -> i32 {
    match target {
        Variable::IntRegister(index) => stores.count(index),
        Variable::IntParam(index) => stores.int_param(IntParam::new(index)),
        _ => unreachable!("caller restricts target"),
    }
}

fn write_int_variable(stores: &mut Stores, target: Variable, index: u16, value: i32, global: bool) {
    match target {
        Variable::IntRegister(_) => set_int_register(stores, index, value, global),
        Variable::IntParam(_) => set_int_param(stores, index, value, global),
        _ => unreachable!("caller restricts target"),
    }
}

fn read_dimen_variable(stores: &Stores, target: Variable) -> Scaled {
    match target {
        Variable::DimenRegister(index) => stores.dimen(index),
        Variable::DimenParam(index) => stores.dimen_param(DimenParam::new(index)),
        _ => unreachable!("caller restricts target"),
    }
}

fn write_dimen_variable(
    stores: &mut Stores,
    target: Variable,
    index: u16,
    value: Scaled,
    global: bool,
) {
    match target {
        Variable::DimenRegister(_) => set_dimen_register(stores, index, value, global),
        Variable::DimenParam(_) => set_dimen_param(stores, index, value, global),
        _ => unreachable!("caller restricts target"),
    }
}

fn read_glue_variable(stores: &Stores, target: Variable) -> GlueId {
    match target {
        Variable::GlueRegister(index) => stores.skip(index),
        Variable::GlueParam(index) => stores.glue_param(GlueParam::new(index)),
        _ => unreachable!("caller restricts target"),
    }
}

fn write_glue_variable(
    stores: &mut Stores,
    target: Variable,
    index: u16,
    value: GlueId,
    global: bool,
) {
    match target {
        Variable::GlueRegister(_) => set_glue_register(stores, index, value, global),
        Variable::GlueParam(_) => set_glue_param(stores, index, value, global),
        _ => unreachable!("caller restricts target"),
    }
}

fn set_int_register(stores: &mut Stores, index: u16, value: i32, global: bool) {
    if global {
        stores.set_count_global(index, value);
    } else {
        stores.set_count(index, value);
    }
}

fn set_dimen_register(stores: &mut Stores, index: u16, value: Scaled, global: bool) {
    if global {
        stores.set_dimen_global(index, value);
    } else {
        stores.set_dimen(index, value);
    }
}

fn set_glue_register(stores: &mut Stores, index: u16, value: GlueId, global: bool) {
    if global {
        stores.set_skip_global(index, value);
    } else {
        stores.set_skip(index, value);
    }
}

fn set_muglue_register(stores: &mut Stores, index: u16, value: GlueId, global: bool) {
    if global {
        stores.set_muskip_global(index, value);
    } else {
        stores.set_muskip(index, value);
    }
}

fn set_toks_register(
    stores: &mut Stores,
    index: u16,
    value: tex_state::ids::TokenListId,
    global: bool,
) {
    if global {
        stores.set_toks_global(index, value);
    } else {
        stores.set_toks(index, value);
    }
}

fn set_int_param(stores: &mut Stores, index: u16, value: i32, global: bool) {
    let param = IntParam::new(index);
    if global {
        stores.set_int_param_global(param, value);
    } else {
        stores.set_int_param(param, value);
    }
}

fn set_dimen_param(stores: &mut Stores, index: u16, value: Scaled, global: bool) {
    let param = DimenParam::new(index);
    if global {
        stores.set_dimen_param_global(param, value);
    } else {
        stores.set_dimen_param(param, value);
    }
}

fn set_glue_param(stores: &mut Stores, index: u16, value: GlueId, global: bool) {
    let param = GlueParam::new(index);
    if global {
        stores.set_glue_param_global(param, value);
    } else {
        stores.set_glue_param(param, value);
    }
}

fn set_tok_param(
    stores: &mut Stores,
    index: u16,
    value: tex_state::ids::TokenListId,
    global: bool,
) {
    let param = TokParam::new(index);
    if global {
        stores.set_tok_param_global(param, value);
    } else {
        stores.set_tok_param(param, value);
    }
}

fn char_from_code(value: i32, context: &'static str) -> Result<char, ExecError> {
    u32::try_from(value)
        .ok()
        .and_then(char::from_u32)
        .ok_or(ExecError::InvalidCode { context, value })
}

fn checked_char_code(value: i32, context: &'static str) -> Result<u32, ExecError> {
    let _ = char_from_code(value, context)?;
    Ok(value as u32)
}

fn catcode_from_i32(value: i32) -> Result<Catcode, ExecError> {
    match value {
        0 => Ok(Catcode::Escape),
        1 => Ok(Catcode::BeginGroup),
        2 => Ok(Catcode::EndGroup),
        3 => Ok(Catcode::MathShift),
        4 => Ok(Catcode::AlignmentTab),
        5 => Ok(Catcode::EndLine),
        6 => Ok(Catcode::Parameter),
        7 => Ok(Catcode::Superscript),
        8 => Ok(Catcode::Subscript),
        9 => Ok(Catcode::Ignored),
        10 => Ok(Catcode::Space),
        11 => Ok(Catcode::Letter),
        12 => Ok(Catcode::Other),
        13 => Ok(Catcode::Active),
        14 => Ok(Catcode::Comment),
        15 => Ok(Catcode::Invalid),
        _ => Err(ExecError::InvalidCode {
            context: "\\catcode",
            value,
        }),
    }
}

fn install_parameter_meanings(stores: &mut Stores) {
    for &(name, index) in INT_PARAMS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::IntParam(index));
    }
    for &(name, index) in DIMEN_PARAMS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::DimenParam(index));
    }
    for &(name, index) in GLUE_PARAMS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::GlueParam(index));
    }
    for &(name, index) in TOK_PARAMS {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::TokParam(index));
    }
}

const INT_PARAMS: &[(&str, u16)] = &[
    ("pretolerance", 0),
    ("tolerance", 1),
    ("linepenalty", 2),
    ("hyphenpenalty", 3),
    ("exhyphenpenalty", 4),
    ("clubpenalty", 5),
    ("widowpenalty", 6),
    ("displaywidowpenalty", 7),
    ("brokenpenalty", 8),
    ("binoppenalty", 9),
    ("relpenalty", 10),
    ("predisplaypenalty", 11),
    ("postdisplaypenalty", 12),
    ("interlinepenalty", 13),
    ("doublehyphendemerits", 14),
    ("finalhyphendemerits", 15),
    ("adjdemerits", 16),
    ("mag", IntParam::MAG.raw()),
    ("delimiterfactor", 18),
    ("looseness", 19),
    ("time", 20),
    ("day", 21),
    ("month", 22),
    ("year", 23),
    ("showboxbreadth", 24),
    ("showboxdepth", 25),
    ("hbadness", 26),
    ("vbadness", 27),
    ("pausing", 28),
    ("tracingonline", 29),
    ("tracingmacros", 30),
    ("tracingstats", 31),
    ("globaldefs", IntParam::GLOBAL_DEFS.raw()),
    ("tracingparagraphs", 33),
    ("tracingpages", 34),
    ("tracingoutput", 35),
    ("tracinglostchars", 36),
    ("tracingcommands", 37),
    ("tracingrestores", 38),
    ("uchyph", 39),
    ("escapechar", IntParam::ESCAPE_CHAR.raw()),
    ("defaulthyphenchar", 41),
    ("defaultskewchar", 42),
    ("endlinechar", IntParam::END_LINE_CHAR.raw()),
    ("newlinechar", 49),
    ("language", 50),
    ("lefthyphenmin", 51),
    ("righthyphenmin", 52),
    ("holdinginserts", 53),
    ("errorcontextlines", 54),
    ("outputpenalty", 55),
    ("maxdeadcycles", 56),
    ("hangafter", 57),
    ("floatingpenalty", 58),
];

const DIMEN_PARAMS: &[(&str, u16)] = &[
    ("parindent", 0),
    ("mathsurround", 1),
    ("lineskiplimit", 2),
    ("hsize", 3),
    ("vsize", 4),
    ("maxdepth", 5),
    ("splitmaxdepth", 6),
    ("boxmaxdepth", 7),
    ("hfuzz", 8),
    ("vfuzz", 9),
    ("delimitershortfall", 10),
    ("nulldelimiterspace", 11),
    ("scriptspace", 12),
    ("predisplaysize", 13),
    ("displaywidth", 14),
    ("displayindent", 15),
    ("overfullrule", 16),
    ("hangindent", 17),
    ("hoffset", 18),
    ("voffset", 19),
    ("emergencystretch", 20),
];

const GLUE_PARAMS: &[(&str, u16)] = &[
    ("lineskip", 0),
    ("baselineskip", 1),
    ("parskip", 2),
    ("abovedisplayskip", 3),
    ("belowdisplayskip", 4),
    ("abovedisplayshortskip", 5),
    ("belowdisplayshortskip", 6),
    ("leftskip", 7),
    ("rightskip", 8),
    ("topskip", 9),
    ("splittopskip", 10),
    ("tabskip", 11),
    ("spaceskip", 12),
    ("xspaceskip", 13),
    ("parfillskip", 14),
    ("thinmuskip", 15),
    ("medmuskip", 16),
    ("thickmuskip", 17),
];

const TOK_PARAMS: &[(&str, u16)] = &[
    ("output", 0),
    ("everypar", 1),
    ("everymath", 2),
    ("everydisplay", 3),
    ("everyhbox", 4),
    ("everyvbox", 5),
    ("everyjob", 6),
    ("everycr", 7),
    ("errhelp", 8),
];

fn reject_macro_prefixes(prefixes: Prefixes) -> Result<(), ExecError> {
    if prefixes.flags != MeaningFlags::EMPTY {
        return Err(ExecError::PrefixWithNonDefinition);
    }
    Ok(())
}

fn reject_all_prefixes(prefixes: Prefixes) -> Result<(), ExecError> {
    if prefixes.global || prefixes.flags != MeaningFlags::EMPTY {
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

fn is_begin_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    )
}

fn is_end_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
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

fn token_matches_keyword_byte(token: Token, expected: u8) -> bool {
    let Token::Char {
        ch,
        cat: Catcode::Letter | Catcode::Other,
    } = token
    else {
        return false;
    };
    ch.to_ascii_lowercase() == char::from(expected)
}
