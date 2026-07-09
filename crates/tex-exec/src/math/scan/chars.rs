use tex_expand::{ExpansionHooks, ReadRecorder};
use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::math::{MathChar, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::Node;
use tex_state::token::Token;

use crate::{ExecError, ModeNest, push_tokens};

use super::scan_math_field;
use crate::math::support::report_math_error;

pub(crate) fn append_mathcode_char<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    ch: char,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let value = stores.mathcode(ch);
    if value == 0x8000 {
        redispatch_active_char(input, stores, ch);
        return Ok(());
    }
    let (class, math_char) = math_char_from_mathcode(ch, value, stores)?;
    append_noad(
        nest,
        NoadKind::Normal(class),
        MathField::MathChar(math_char),
    );
    Ok(())
}

pub(crate) fn append_math_char_code(
    nest: &mut ModeNest,
    stores: &Universe,
    code: u32,
) -> Result<(), ExecError> {
    let (class, math_char) = math_char_from_math_char_code(code, stores)?;
    append_noad(
        nest,
        NoadKind::Normal(class),
        MathField::MathChar(math_char),
    );
    Ok(())
}

fn math_char_from_math_char_code(
    code: u32,
    stores: &Universe,
) -> Result<(NoadClass, MathChar), ExecError> {
    if code > 0x7fff {
        return Err(ExecError::InvalidCode {
            context: "\\mathchar",
            value: code as i32,
        });
    }
    let class = ((code >> 12) & 0x7) as u8;
    let family = ((code >> 8) & 0xf) as u8;
    let ch = char::from_u32(code & 0xff).unwrap_or('\0');
    Ok(resolve_math_class_family(class, family, ch, stores))
}

pub(crate) fn math_char_from_code(code: u32, stores: &Universe) -> Result<MathChar, ExecError> {
    Ok(math_char_from_math_char_code(code, stores)?.1)
}

pub(crate) fn math_char_from_mathcode(
    original: char,
    code: u32,
    stores: &Universe,
) -> Result<(NoadClass, MathChar), ExecError> {
    if code > 0x7fff {
        return Ok((
            NoadClass::Ord,
            MathChar {
                family: 0,
                character: original,
            },
        ));
    }
    let class = ((code >> 12) & 0x7) as u8;
    let family = ((code >> 8) & 0xf) as u8;
    let ch = char::from_u32(code & 0xff).unwrap_or(original);
    Ok(resolve_math_class_family(class, family, ch, stores))
}

fn resolve_math_class_family(
    class: u8,
    code_family: u8,
    ch: char,
    stores: &Universe,
) -> (NoadClass, MathChar) {
    let mut family = code_family;
    let class = match class {
        0 => NoadClass::Ord,
        1 => NoadClass::Op,
        2 => NoadClass::Bin,
        3 => NoadClass::Rel,
        4 => NoadClass::Open,
        5 => NoadClass::Close,
        6 => NoadClass::Punct,
        7 => {
            let fam = stores.int_param(IntParam::FAM);
            if (0..=15).contains(&fam) {
                family = fam as u8;
            }
            NoadClass::Ord
        }
        _ => unreachable!("math class is three bits"),
    };
    (
        class,
        MathChar {
            family,
            character: ch,
        },
    )
}

pub(crate) fn redispatch_active_char<S>(input: &mut InputStack<S>, stores: &mut Universe, ch: char)
where
    S: InputSource,
{
    let symbol = stores.intern_active_character(ch);
    push_tokens(input, stores, [Token::Cs(symbol)]);
}

pub(crate) fn append_noad(nest: &mut ModeNest, kind: NoadKind, nucleus: MathField) {
    nest.current_list_mut()
        .push(Node::MathNoad(MathNoad::new(kind, nucleus)));
}

pub(crate) fn attach_script<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    superscript: bool,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let field = scan_math_field(nest, input, stores, recorder, hooks)?;
    let Some(mut node) = nest.current_list_mut().pop_last_node() else {
        push_scripted_empty_noad(nest, field, superscript);
        return Ok(());
    };
    let Node::MathNoad(noad) = &mut node else {
        nest.current_list_mut().push(node);
        push_scripted_empty_noad(nest, field, superscript);
        return Ok(());
    };
    let target = if superscript {
        &mut noad.superscript
    } else {
        &mut noad.subscript
    };
    if !matches!(target, MathField::Empty) {
        nest.current_list_mut().push(node);
        report_math_error(
            stores,
            if superscript {
                "Double superscript"
            } else {
                "Double subscript"
            },
        );
        push_scripted_empty_noad(nest, field, superscript);
    } else {
        *target = field;
        nest.current_list_mut().push(node);
    }
    Ok(())
}

fn push_scripted_empty_noad(nest: &mut ModeNest, field: MathField, superscript: bool) {
    let mut noad = MathNoad::new(NoadKind::Normal(NoadClass::Ord), MathField::Empty);
    if superscript {
        noad.superscript = field;
    } else {
        noad.subscript = field;
    }
    nest.current_list_mut().push(Node::MathNoad(noad));
}
