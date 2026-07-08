use super::*;
use tex_state::ids::FontId;
mod streams;
mod variable_access;

pub(super) use streams::{execute_read, execute_special, execute_stream_command, execute_write};
use variable_access::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Variable {
    IntRegister(u16),
    DimenRegister(u16),
    GlueRegister(u16),
    MuGlueRegister(u16),
    ToksRegister(u16),
    IntParam(u16),
    DimenParam(u16),
    GlueParam(u16),
    TokParam(u16),
    FontDimen(FontId, u16),
    FontHyphenChar(FontId),
    FontSkewChar(FontId),
}

pub(super) fn execute_variable_assignment<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
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

pub(super) fn execute_assignment_to_target<S, H>(
    target: Variable,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
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
        Variable::FontDimen(font, number) => {
            let value = scan_scaled(input, stores, hooks)?;
            stores.set_font_dimen(font, number, value, global)?;
        }
        Variable::GlueParam(index) => {
            let value = scan_glue_id(input, stores, hooks, false)?;
            set_glue_param(stores, index, value, global);
        }
        Variable::TokParam(index) => {
            let value = scan_token_list_assignment(input, stores, hooks)?;
            set_tok_param(stores, index, value, global);
        }
        Variable::FontHyphenChar(font) => {
            let value = scan_i32(input, stores, hooks)?;
            stores.set_font_hyphen_char(font, value, global);
        }
        Variable::FontSkewChar(font) => {
            let value = scan_i32(input, stores, hooks)?;
            stores.set_font_skew_char(font, value, global);
        }
    }
    Ok(())
}

pub(super) fn execute_register_def<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
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

pub(super) fn execute_char_def<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
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

pub(super) fn execute_arithmetic<S, H>(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    input: &mut InputStack<S>,
    stores: &mut Universe,
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
        Variable::FontHyphenChar(font) | Variable::FontSkewChar(font) => {
            let old = read_int_variable(stores, target);
            let rhs = scan_i32(input, stores, hooks)?;
            let value = arithmetic_i32(primitive, old, rhs)?;
            write_font_int_variable(stores, target, font, value, global);
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
        Variable::FontDimen(font, number) => {
            let old = stores.font_dimen(font, number);
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
            stores.set_font_dimen(font, number, value, global)?;
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

pub(super) fn execute_code_table_assignment<S, H>(
    primitive: UnexpandablePrimitive,
    input: &mut InputStack<S>,
    stores: &mut Universe,
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
