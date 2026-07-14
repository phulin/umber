use tex_lex::InputStack;
use tex_state::ExpansionState;
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::provenance::InsertedOriginKind;
use tex_state::token::{Catcode, Token, TracedTokenWord};

use crate::{
    Dispatch, ExpandError, ExpandableOpcode, ExpansionContext, apply_dispatch_push,
    get_x_token_without_input_open, push_inserted_token, scan_helpers,
};

pub(crate) fn expand_after(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn crate::ExpansionMode,
    context: TracedTokenWord,
) -> Result<(), ExpandError>
where
{
    let Some(saved) = crate::get_token_with_context(input, stores, expansion)? else {
        return Err(ExpandError::MissingTokenAfterPrimitive {
            opcode: ExpandableOpcode::ExpandAfter,
            context,
        });
    };
    let Some(target) = crate::get_token_with_context(input, stores, expansion)? else {
        return Err(ExpandError::MissingTokenAfterPrimitive {
            opcode: ExpandableOpcode::ExpandAfter,
            context,
        });
    };
    mode.dispatch_raw_token_after(saved, target, input, stores, expansion)
}

pub(crate) fn scan_csname(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<String, ExpandError> {
    let mut name = String::new();

    loop {
        let Some(read) = input.next_traced_expansion_token(stores)? else {
            return Err(ExpandError::MissingEndCsName { context });
        };
        expansion.observe_read(read);
        let token = read.token();
        let traced = read.traced_token();

        if read.suppress_expansion() {
            if append_csname_token(&mut name, token) == CsNameAppend::Recover {
                push_inserted_token(input, stores, traced, InsertedOriginKind::Unread);
                return Ok(name);
            }
            continue;
        }

        let Token::Cs(symbol) = token else {
            if append_csname_token(&mut name, token) == CsNameAppend::Recover {
                push_inserted_token(input, stores, traced, InsertedOriginKind::Unread);
                return Ok(name);
            }
            continue;
        };

        let meaning = stores.meaning(symbol);
        expansion.record_meaning(symbol, meaning);

        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) {
            return Ok(name);
        }

        match crate::dispatch::dispatch_without_input_open(
            token,
            traced.origin(),
            input,
            stores,
            expansion,
            meaning,
        )? {
            Dispatch::Continue => {}
            Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => {
                if append_csname_token(&mut name, crate::semantic_token(token))
                    == CsNameAppend::Recover
                {
                    push_inserted_token(input, stores, token, InsertedOriginKind::Unread);
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
        Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => CsNameAppend::Recover,
    }
}

pub(crate) fn scan_input_name(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<String, ExpandError> {
    let Some(first) = scan_helpers::next_non_space_x_token_with_context(input, stores, expansion)?
    else {
        return Err(ExpandError::MissingInputName { context });
    };

    if is_begin_group(crate::semantic_token(first)) {
        let mut name = String::new();
        loop {
            let Some(token) = get_x_token_without_input_open(input, stores, expansion)? else {
                return Err(ExpandError::MissingInputName { context });
            };
            let semantic = crate::semantic_token(token);
            if is_end_group(semantic) {
                return if name.is_empty() {
                    Err(ExpandError::MissingInputName { context })
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
        let token = match get_x_token_without_input_open(input, stores, expansion) {
            Ok(Some(token)) => token,
            Ok(None) => break,
            Err(ExpandError::InputOpen { .. }) if !name.is_empty() => break,
            Err(error) => return Err(error),
        };
        let semantic = crate::semantic_token(token);
        if matches!(
            semantic,
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
        Err(ExpandError::MissingInputName { context })
    } else {
        Ok(name)
    }
}

fn append_input_name_token(name: &mut String, token: TracedTokenWord) -> Result<(), ExpandError> {
    match crate::semantic_token(token) {
        Token::Char { ch, .. } => {
            name.push(ch);
            Ok(())
        }
        Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {
            Err(ExpandError::NonCharacterInInputName { context: token })
        }
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
