use tex_lex::{InputSource, InputStack, MacroArguments};
use tex_state::env::banks::IntParam;
use tex_state::ids::TokenListId;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::scaled::Scaled;
use tex_state::stores::Stores;
use tex_state::token::{Catcode, Token};

use crate::{
    Dispatch, ExpandError, ExpandableOpcode, ExpansionHooks, ExpansionReplayKind, ReadRecorder,
    scan_helpers,
};

pub(crate) fn expand_the<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Some(token) =
        scan_helpers::next_non_space_x_token_with_hooks(input, stores, recorder, hooks)?
    else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::The,
        ));
    };
    let Token::Cs(symbol) = token else {
        return Err(ExpandError::UnsupportedTheTarget(token));
    };

    match stores.resolve(symbol) {
        "count" => {
            let index = scan_helpers::scan_register_index(input, stores, recorder, hooks)?;
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &stores.count(index).to_string(),
            ))
        }
        "dimen" => {
            let index = scan_helpers::scan_register_index(input, stores, recorder, hooks)?;
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &format_scaled(stores.dimen(index)),
            ))
        }
        "toks" => {
            let index = scan_helpers::scan_register_index(input, stores, recorder, hooks)?;
            Ok(Dispatch::Push {
                replay_kind: ExpansionReplayKind::TheOutput,
                token_list: stores.toks(index),
                macro_arguments: MacroArguments::new(),
            })
        }
        "endlinechar" => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.int_param(IntParam::END_LINE_CHAR).to_string(),
        )),
        "escapechar" => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.int_param(IntParam::ESCAPE_CHAR).to_string(),
        )),
        // TODO(umber2-5qt): support `\the` for glue, muglue, font dimensions,
        // code tables, box dimensions, page state, and time/job parameters as
        // those Env classes become semantically available to the gullet.
        _ => Err(ExpandError::UnsupportedTheTarget(token)),
    }
}

pub(crate) fn push_rendered_text(
    stores: &mut Stores,
    replay_kind: ExpansionReplayKind,
    text: &str,
) -> Dispatch {
    push_rendered_tokens(stores, replay_kind, text_tokens(text))
}

pub(crate) fn push_rendered_tokens<I>(
    stores: &mut Stores,
    replay_kind: ExpansionReplayKind,
    tokens: I,
) -> Dispatch
where
    I: IntoIterator<Item = Token>,
{
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let token_list = freeze_output_tokens(stores, &tokens);
    Dispatch::Push {
        replay_kind,
        token_list,
        macro_arguments: MacroArguments::new(),
    }
}

fn freeze_output_tokens(stores: &mut Stores, tokens: &[Token]) -> TokenListId {
    stores.intern_token_list(tokens)
}

pub(crate) fn string_tokens(stores: &Stores, token: Token) -> Vec<Token> {
    match token {
        Token::Char { ch, .. } => vec![rendered_char(ch)],
        Token::Cs(symbol) => {
            let mut out = Vec::new();
            if let Some(escape) = escapechar(stores) {
                out.push(rendered_char(escape));
            }
            out.extend(stores.resolve(symbol).chars().map(rendered_char));
            out
        }
        Token::Param(slot) => text_tokens(&format!("#{slot}")),
    }
}

pub(crate) fn meaning_text(stores: &Stores, token: Token) -> String {
    match token {
        Token::Char {
            ch,
            cat: Catcode::Letter,
        } => format!("the letter {ch}"),
        Token::Char { ch, .. } => format!("the character {ch}"),
        Token::Param(slot) => format!("macro parameter character #{slot}"),
        Token::Cs(symbol) => match stores.meaning(symbol) {
            Meaning::Undefined => "undefined".to_owned(),
            Meaning::Relax => "\\relax".to_owned(),
            Meaning::CharGiven(ch) => format!("the character {ch}"),
            Meaning::ExpandablePrimitive(_) => format!("\\{}", stores.resolve(symbol)),
            Meaning::UnexpandablePrimitive(_) => format!("\\{}", stores.resolve(symbol)),
            Meaning::Macro { flags, definition } => {
                let macro_meaning = stores.macro_definition(definition);
                let mut text = String::new();
                if flags.contains(MeaningFlags::PROTECTED) {
                    text.push_str("protected");
                }
                text.push_str("macro:");
                text.push_str(&token_list_text(stores, macro_meaning.parameter_text()));
                text.push_str("->");
                text.push_str(&token_list_text(stores, macro_meaning.replacement_text()));
                text
            }
            Meaning::Unknown(_) => "unknown".to_owned(),
        },
    }
}

fn token_list_text(stores: &Stores, token_list: TokenListId) -> String {
    stores
        .tokens(token_list)
        .iter()
        .flat_map(|&token| string_tokens(stores, token))
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(ch),
            Token::Cs(_) | Token::Param(_) => None,
        })
        .collect()
}

fn text_tokens(text: &str) -> Vec<Token> {
    text.chars().map(rendered_char).collect()
}

fn rendered_char(ch: char) -> Token {
    Token::Char {
        ch,
        cat: if ch == ' ' {
            Catcode::Space
        } else {
            Catcode::Other
        },
    }
}

fn escapechar(stores: &Stores) -> Option<char> {
    u32::try_from(stores.int_param(IntParam::ESCAPE_CHAR))
        .ok()
        .filter(|&value| value < 256)
        .and_then(char::from_u32)
}

pub(crate) fn roman_numeral(value: i32) -> String {
    if value <= 0 {
        return String::new();
    }
    let mut value = value;
    let mut out = String::new();
    for (amount, text) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while value >= amount {
            out.push_str(text);
            value -= amount;
        }
    }
    out
}

fn format_scaled(value: Scaled) -> String {
    let raw = value.raw();
    let negative = raw < 0;
    let magnitude = if negative {
        i64::from(raw).wrapping_neg()
    } else {
        i64::from(raw)
    };
    let unity = i64::from(Scaled::UNITY);
    let mut integer = magnitude / unity;
    let fraction = magnitude % unity;
    let mut decimal = ((fraction * 100_000) + (unity / 2)) / unity;
    if decimal == 100_000 {
        integer += 1;
        decimal = 0;
    }
    let mut fraction_text = format!("{decimal:05}");
    while fraction_text.len() > 1 && fraction_text.ends_with('0') {
        fraction_text.pop();
    }
    let sign = if negative { "-" } else { "" };
    format!("{sign}{integer}.{fraction_text}pt")
}
