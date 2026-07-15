use super::*;
use tex_state::ids::FontId;
use tex_state::page::{PageDimension, PageInteger};
mod streams;
mod variable_access;

pub(super) use streams::{
    execute_immediate_stream_command, execute_immediate_write, execute_read, execute_special,
    execute_stream_command, execute_write, openout_target,
};
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
    MuGlueParam(u16),
    TokParam(u16),
    PageDimension(PageDimension),
    PageInteger(PageInteger),
    FontDimen(FontId, u32),
    FontHyphenChar(FontId),
    FontSkewChar(FontId),
}

pub(super) fn execute_variable_assignment(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    prefixes: Prefixes,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let index = scan_register_index(input, stores, execution, context)?;
    let target = match primitive {
        UnexpandablePrimitive::Count => Variable::IntRegister(index),
        UnexpandablePrimitive::Dimen => Variable::DimenRegister(index),
        UnexpandablePrimitive::Skip => Variable::GlueRegister(index),
        UnexpandablePrimitive::Muskip => Variable::MuGlueRegister(index),
        UnexpandablePrimitive::Toks => Variable::ToksRegister(index),
        _ => unreachable!("caller restricts primitive"),
    };
    execute_assignment_to_target(target, prefixes, context, input, stores, execution)
}

pub(super) fn execute_assignment_to_target(
    target: Variable,
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    skip_optional_equals_x(input, stores, execution)?;
    let global = apply_globaldefs(prefixes.global, stores);
    match target {
        Variable::IntRegister(index) => {
            let value = scan_i32(input, stores, execution, context)?;
            set_int_register(stores, index, value, global);
        }
        Variable::DimenRegister(index) => {
            let value = scan_scaled(input, stores, execution, context)?;
            set_dimen_register(stores, index, value, global);
        }
        Variable::GlueRegister(index) => {
            let value = scan_glue_id(input, stores, execution, false, context)?;
            set_glue_register(stores, index, value, global);
        }
        Variable::MuGlueRegister(index) => {
            let value = scan_glue_id(input, stores, execution, true, context)?;
            set_muglue_register(stores, index, value, global);
        }
        Variable::ToksRegister(index) => {
            let value = scan_token_list_assignment(input, stores, execution, context)?;
            set_toks_register(stores, index, value, global);
        }
        Variable::IntParam(index) => {
            let value = scan_i32(input, stores, execution, context)?;
            set_int_param(stores, index, value, global);
        }
        Variable::PageInteger(integer) => {
            reject_macro_prefixes(prefixes)?;
            let value = scan_i32(input, stores, execution, context)?;
            stores.set_page_integer(integer, value);
        }
        Variable::DimenParam(index) => {
            let value = scan_scaled(input, stores, execution, context)?;
            set_dimen_param(stores, index, value, global);
        }
        Variable::PageDimension(dimension) => {
            reject_macro_prefixes(prefixes)?;
            let value = scan_scaled(input, stores, execution, context)?;
            stores.set_page_dimension(dimension, value);
        }
        Variable::FontDimen(font, number) => {
            let value = scan_scaled(input, stores, execution, context)?;
            set_font_dimen_recovering(stores, font, number, value)?;
        }
        Variable::GlueParam(index) => {
            let value = scan_glue_id(input, stores, execution, false, context)?;
            set_glue_param(stores, index, value, global);
        }
        Variable::MuGlueParam(index) => {
            let value = scan_glue_id(input, stores, execution, true, context)?;
            set_glue_param(stores, index, value, global);
        }
        Variable::TokParam(index) => {
            let value = scan_token_list_assignment(input, stores, execution, context)?;
            set_tok_param(stores, index, value, global);
        }
        Variable::FontHyphenChar(font) => {
            let value = scan_i32(input, stores, execution, context)?;
            stores.set_font_hyphen_char(font, value);
        }
        Variable::FontSkewChar(font) => {
            let value = scan_i32(input, stores, execution, context)?;
            stores.set_font_skew_char(font, value);
        }
    }
    Ok(())
}

pub(super) fn execute_register_def(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let target = scan_definition_target(input, stores, "register definition")?;
    // As for `\chardef`, TeX.web section 1220 installs `\relax` before
    // scanning the register number. This makes `\skipdef\s100\s` terminate
    // the number scan without expanding an undefined/old target meaning.
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, Meaning::Relax);
    } else {
        stores.set_meaning(target, Meaning::Relax);
    }
    skip_optional_equals_x(input, stores, execution)?;
    let index = scan_register_index(input, stores, execution, context)?;
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

pub(super) fn execute_char_def(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let target = scan_definition_target(input, stores, "character definition")?;
    // TeX.web temporarily makes the target `\relax` before scanning the
    // numeric value, so a self-terminating definition cannot expand the old
    // meaning (section 1220).
    if apply_globaldefs(prefixes.global, stores) {
        stores.set_meaning_global(target, Meaning::Relax);
    } else {
        stores.set_meaning(target, Meaning::Relax);
    }
    skip_optional_equals_x(input, stores, execution)?;
    let value = scan_i32(input, stores, execution, context)?;
    let meaning = match primitive {
        UnexpandablePrimitive::CharDef => {
            let value = recover_restricted_code(
                stores,
                value,
                255,
                "Bad character code",
                "A character number must be between 0 and 255.",
            );
            let ch = char::from_u32(value as u32).expect("0..=255 is Unicode scalar");
            Meaning::CharGiven(ch)
        }
        UnexpandablePrimitive::MathCharDef => {
            let value = recover_restricted_code(
                stores,
                value,
                32_767,
                "Bad mathchar",
                "A mathchar number must be between 0 and 32767.",
            );
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

fn recover_restricted_code(
    stores: &mut Universe,
    value: i32,
    maximum: i32,
    message: &str,
    help: &str,
) -> i32 {
    if (0..=maximum).contains(&value) {
        return value;
    }
    stores.world_mut().write_text(
        tex_state::PrintSink::TerminalAndLog,
        &format!("\n! {message} ({value}).\n{help}\nI changed this one to zero.\n"),
    );
    0
}

pub(super) fn execute_arithmetic(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let target = scan_variable_target(input, stores, execution)?;
    let _ = scan_optional_keyword_x(input, stores, execution, "by")?;
    let global = apply_globaldefs(prefixes.global, stores);
    match target {
        Variable::IntRegister(index) | Variable::IntParam(index) => {
            let old = read_int_variable(stores, target);
            let rhs = scan_i32(input, stores, execution, context)?;
            let value = arithmetic_i32(primitive, old, rhs)?;
            write_int_variable(stores, target, index, value, global);
        }
        Variable::PageInteger(integer) => {
            let old = stores.page_integer(integer);
            let rhs = scan_i32(input, stores, execution, context)?;
            let value = arithmetic_i32(primitive, old, rhs)?;
            stores.set_page_integer(integer, value);
        }
        Variable::FontHyphenChar(font) | Variable::FontSkewChar(font) => {
            let old = read_int_variable(stores, target);
            let rhs = scan_i32(input, stores, execution, context)?;
            let value = arithmetic_i32(primitive, old, rhs)?;
            write_font_int_variable(stores, target, font, value);
        }
        Variable::DimenRegister(index) | Variable::DimenParam(index) => {
            let old = read_dimen_variable(stores, target);
            let value = match primitive {
                UnexpandablePrimitive::Advance => old
                    .checked_add(scan_scaled(input, stores, execution, context)?)
                    .ok_or(ExecError::ArithmeticOverflow)?,
                UnexpandablePrimitive::Multiply => {
                    scaled_checked_mul(old, scan_i32(input, stores, execution, context)?)?
                }
                UnexpandablePrimitive::Divide => {
                    scaled_checked_div(old, scan_nonzero_i32(input, stores, execution, context)?)?
                }
                _ => unreachable!("caller restricts primitive"),
            };
            write_dimen_variable(stores, target, index, value, global);
        }
        Variable::PageDimension(dimension) => {
            let old = stores.page_dimension(dimension);
            let value = match primitive {
                UnexpandablePrimitive::Advance => old
                    .checked_add(scan_scaled(input, stores, execution, context)?)
                    .ok_or(ExecError::ArithmeticOverflow)?,
                UnexpandablePrimitive::Multiply => {
                    scaled_checked_mul(old, scan_i32(input, stores, execution, context)?)?
                }
                UnexpandablePrimitive::Divide => {
                    scaled_checked_div(old, scan_nonzero_i32(input, stores, execution, context)?)?
                }
                _ => unreachable!("caller restricts primitive"),
            };
            stores.set_page_dimension(dimension, value);
        }
        Variable::FontDimen(font, number) => {
            let old = stores.font_dimen(font, number);
            let value = match primitive {
                UnexpandablePrimitive::Advance => old
                    .checked_add(scan_scaled(input, stores, execution, context)?)
                    .ok_or(ExecError::ArithmeticOverflow)?,
                UnexpandablePrimitive::Multiply => {
                    scaled_checked_mul(old, scan_i32(input, stores, execution, context)?)?
                }
                UnexpandablePrimitive::Divide => {
                    scaled_checked_div(old, scan_nonzero_i32(input, stores, execution, context)?)?
                }
                _ => unreachable!("caller restricts primitive"),
            };
            set_font_dimen_recovering(stores, font, number, value)?;
        }
        Variable::GlueRegister(index) | Variable::GlueParam(index) => {
            let old = stores.glue(read_glue_variable(stores, target));
            let rhs = scan_glue_or_factor(primitive, input, stores, execution, false, context)?;
            let value = arithmetic_glue(primitive, old, rhs)?;
            let id = stores.intern_glue(value);
            write_glue_variable(stores, target, index, id, global);
        }
        Variable::MuGlueParam(index) => {
            let old = stores.glue(stores.glue_param(GlueParam::new(index)));
            let rhs = scan_glue_or_factor(primitive, input, stores, execution, true, context)?;
            let value = arithmetic_glue(primitive, old, rhs)?;
            let id = stores.intern_glue(value);
            set_glue_param(stores, index, id, global);
        }
        Variable::MuGlueRegister(index) => {
            let old = stores.glue(stores.muskip(index));
            let rhs = scan_glue_or_factor(primitive, input, stores, execution, true, context)?;
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

fn set_font_dimen_recovering(
    stores: &mut Universe,
    font: tex_state::ids::FontId,
    number: u32,
    value: Scaled,
) -> Result<(), ExecError> {
    match stores.set_font_dimen(font, number, value) {
        Ok(()) => Ok(()),
        Err(tex_state::FontParameterError::CannotGrow { current_len, .. }) => {
            let name = stores.font_name(font);
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                &format!(
                    "\n! Font {name} has only {current_len} fontdimen parameters.\nTo increase the number of font parameters, you must\nuse \\fontdimen immediately after the \\font is loaded.\n"
                ),
            );
            Ok(())
        }
        Err(tex_state::FontParameterError::NumberOutOfRange { number, maximum }) => {
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                &format!(
                    "\n! Bad fontdimen number ({number}).\nThe largest representable fontdimen number is {maximum};\nI ignored this assignment.\n"
                ),
            );
            Ok(())
        }
        Err(tex_state::FontParameterError::FontOutOfRange { font, maximum }) => {
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                &format!(
                    "\n! Font id {} is outside the fontdimen cell range.\nThe largest representable font id is {maximum};\nI ignored this assignment.\n",
                    font.raw()
                ),
            );
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn execute_code_table_assignment(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let code = scan_i32(input, stores, execution, context)?;
    skip_optional_equals_x(input, stores, execution)?;
    let value = scan_i32(input, stores, execution, context)?;
    let ch = char_from_code(code, "code-table character")?;
    let global = apply_globaldefs(prefixes.global, stores);
    match primitive {
        UnexpandablePrimitive::CatCode => {
            let value = catcode_from_i32(value)?;
            if global {
                stores.set_catcode_global(ch, value);
            } else {
                stores.set_catcode(ch, value);
            }
        }
        UnexpandablePrimitive::LcCode => {
            let value = checked_char_code(value, "\\lccode")? as LcCode;
            if global {
                stores.set_lccode_global(ch, value)
            } else {
                stores.set_lccode(ch, value)
            }
        }
        UnexpandablePrimitive::UcCode => {
            let value = checked_char_code(value, "\\uccode")? as UcCode;
            if global {
                stores.set_uccode_global(ch, value)
            } else {
                stores.set_uccode(ch, value)
            }
        }
        UnexpandablePrimitive::SfCode => {
            if !(0..=32_767).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\sfcode",
                    value,
                });
            }
            if global {
                stores.set_sfcode_global(ch, value as SfCode)
            } else {
                stores.set_sfcode(ch, value as SfCode)
            }
        }
        UnexpandablePrimitive::MathCode => {
            if !(0..=32_768).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\mathcode",
                    value,
                });
            }
            if global {
                stores.set_mathcode_global(ch, value as MathCode)
            } else {
                stores.set_mathcode(ch, value as MathCode)
            }
        }
        UnexpandablePrimitive::DelCode => {
            if !(-1..=0xFF_FFFF).contains(&value) {
                return Err(ExecError::InvalidCode {
                    context: "\\delcode",
                    value,
                });
            }
            if global {
                stores.set_delcode_global(ch, value as DelCode)
            } else {
                stores.set_delcode(ch, value as DelCode)
            }
        }
        _ => unreachable!("caller restricts primitive"),
    }
    Ok(())
}

pub(super) fn execute_pdf_font_code_assignment(
    primitive: UnexpandablePrimitive,
    prefixes: Prefixes,
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    reject_macro_prefixes(prefixes)?;
    let font = super::scan_font_selector(input, stores, execution)?;
    let code = scan_i32(input, stores, execution, context)?;
    let code = u8::try_from(code).map_err(|_| ExecError::InvalidCode {
        context: "pdfTeX font-code character",
        value: code,
    })?;
    skip_optional_equals_x(input, stores, execution)?;
    let value = scan_i32(input, stores, execution, context)?;
    let table = match primitive {
        UnexpandablePrimitive::PdfLpCode => tex_state::PdfFontCode::Lp,
        UnexpandablePrimitive::PdfRpCode => tex_state::PdfFontCode::Rp,
        UnexpandablePrimitive::PdfEfCode => tex_state::PdfFontCode::Ef,
        UnexpandablePrimitive::PdfTagCode => tex_state::PdfFontCode::Tag,
        UnexpandablePrimitive::PdfKnbsCode => tex_state::PdfFontCode::Knbs,
        UnexpandablePrimitive::PdfStbsCode => tex_state::PdfFontCode::Stbs,
        UnexpandablePrimitive::PdfShbsCode => tex_state::PdfFontCode::Shbs,
        UnexpandablePrimitive::PdfKnbcCode => tex_state::PdfFontCode::Knbc,
        UnexpandablePrimitive::PdfKnacCode => tex_state::PdfFontCode::Knac,
        _ => unreachable!("caller restricts pdfTeX font-code primitives"),
    };
    stores.set_pdf_font_code(table, font, code, value);
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
