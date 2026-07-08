use tex_lex::{InputSource, InputStack};
use tex_state::stores::Stores;
use tex_state::token::{Catcode, Token};

use crate::{
    ExpandError, ExpansionHooks, ReadRecorder, get_x_token, get_x_token_with_recorder_and_hooks,
    scan_int,
};

pub(crate) fn next_non_space_x_token<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<Option<Token>, ExpandError>
where
    S: InputSource,
{
    loop {
        let Some(token) = get_x_token(input, stores)? else {
            return Ok(None);
        };
        if !matches!(
            token,
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            return Ok(Some(token));
        }
    }
}

pub(crate) fn next_non_space_x_token_with_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Option<Token>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
            return Ok(None);
        };
        if !matches!(
            token,
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            return Ok(Some(token));
        }
    }
}

pub(crate) fn scan_register_index<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<u16, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let value = scan_int::scan_int_with_recorder_and_hooks(input, stores, recorder, hooks)?.value();
    if !(0..=32_767).contains(&value) {
        return Err(scan_int::ScanIntError::RegisterNumberOutOfRange(value).into());
    }
    Ok(value as u16)
}
