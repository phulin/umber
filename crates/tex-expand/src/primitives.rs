use tex_lex::{InputSource, InputStack};
use tex_state::ExpansionState;
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{Catcode, OriginId, Token};

use crate::{
    Dispatch, ExpandError, ExpandableOpcode, ExpansionHooks, ReadRecorder, apply_dispatch_push,
    dispatch_one_raw_token_with_hooks, get_x_token_without_input_open, push_dispatch_result,
    push_inserted_token, scan_helpers,
};

pub(crate) fn expand_after<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Some(saved) = input.next_token(stores)? else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::ExpandAfter,
        ));
    };
    let Some(target) = input.next_token(stores)? else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::ExpandAfter,
        ));
    };

    let target_dispatch =
        dispatch_one_raw_token_with_hooks(target, input, stores, recorder, hooks)?;
    push_dispatch_result(input, stores, target_dispatch);
    push_inserted_token(input, stores, saved);
    Ok(())
}

pub(crate) fn scan_csname<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<String, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut name = String::new();

    loop {
        let Some(read) = input.next_expansion_token(stores)? else {
            return Err(ExpandError::MissingEndCsName);
        };
        let token = read.token();

        if read.suppress_expansion() {
            if append_csname_token(&mut name, token) == CsNameAppend::Recover {
                push_inserted_token(input, stores, token);
                return Ok(name);
            }
            continue;
        }

        let Token::Cs(symbol) = token else {
            if append_csname_token(&mut name, token) == CsNameAppend::Recover {
                push_inserted_token(input, stores, token);
                return Ok(name);
            }
            continue;
        };

        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);

        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) {
            return Ok(name);
        }

        match crate::dispatch::dispatch_without_input_open(
            token,
            OriginId::UNKNOWN,
            input,
            stores,
            recorder,
            hooks,
            meaning,
        )? {
            Dispatch::Continue => {}
            Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => {
                if append_csname_token(&mut name, token) == CsNameAppend::Recover {
                    push_inserted_token(input, stores, token);
                    return Ok(name);
                }
            }
            push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CsNameAppend {
    Appended,
    Recover,
}

fn append_csname_token(name: &mut String, token: Token) -> CsNameAppend {
    match token {
        Token::Char { ch, .. } => {
            name.push(ch);
            CsNameAppend::Appended
        }
        Token::Cs(_) | Token::Param(_) => CsNameAppend::Recover,
    }
}

pub(crate) fn scan_input_name<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<String, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Some(first) =
        scan_helpers::next_non_space_x_token_with_hooks(input, stores, recorder, hooks)?
    else {
        return Err(ExpandError::MissingInputName);
    };

    if is_begin_group(first) {
        let mut name = String::new();
        loop {
            let Some(token) = get_x_token_without_input_open(input, stores, recorder, hooks)?
            else {
                return Err(ExpandError::MissingInputName);
            };
            if is_end_group(token) {
                return if name.is_empty() {
                    Err(ExpandError::MissingInputName)
                } else {
                    Ok(name)
                };
            }
            append_input_name_token(&mut name, token)?;
        }
    }

    let mut name = String::new();
    append_input_name_token(&mut name, first)?;
    loop {
        let Some(token) = get_x_token_without_input_open(input, stores, recorder, hooks)? else {
            break;
        };
        if matches!(
            token,
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            break;
        }
        append_input_name_token(&mut name, token)?;
    }

    if name.is_empty() {
        Err(ExpandError::MissingInputName)
    } else {
        Ok(name)
    }
}

fn append_input_name_token(name: &mut String, token: Token) -> Result<(), ExpandError> {
    match token {
        Token::Char { ch, .. } => {
            name.push(ch);
            Ok(())
        }
        Token::Cs(_) | Token::Param(_) => Err(ExpandError::NonCharacterInInputName(token)),
    }
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
