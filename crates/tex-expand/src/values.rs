use tex_lex::{InputSource, InputStack, MacroArguments};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{FontId, TokenListId};
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::{BoxDimension, ExpansionState};

use crate::{
    Dispatch, ExpandError, ExpandNext, ExpandableOpcode, ExpansionHooks, ExpansionReplayKind,
    NoInputExpandNext, ReadRecorder, scan_helpers, scan_int,
};

#[allow(dead_code)]
pub(crate) fn expand_the<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    expand_the_with_expander_and_hooks(input, stores, recorder, hooks, &mut NoInputExpandNext)
}

pub(crate) fn expand_the_with_expander_and_hooks<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let Some(token) = scan_helpers::next_non_space_x_token_with_expander_and_hooks(
        input, stores, recorder, hooks, expander,
    )?
    else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::The,
        ));
    };
    let Token::Cs(symbol) = token else {
        return Err(ExpandError::UnsupportedTheTarget(token));
    };

    match stores.meaning(symbol) {
        Meaning::UnexpandablePrimitive(primitive) => match primitive {
            tex_state::meaning::UnexpandablePrimitive::Count => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.count(index).to_string(),
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Dimen => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(stores.dimen(index)),
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Skip => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_glue(stores.glue(stores.skip(index))),
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Muskip => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_glue(stores.glue(stores.muskip(index))),
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Toks => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander,
                )?;
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::TheOutput,
                    token_list: stores.toks(index),
                    macro_arguments: MacroArguments::new(),
                })
            }
            tex_state::meaning::UnexpandablePrimitive::Font => {
                let symbol = stores
                    .current_font_symbol()
                    .ok_or(ExpandError::UnsupportedTheTarget(token))?;
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::TheOutput,
                    token_list: stores.intern_token_list(&[Token::Cs(symbol)]),
                    macro_arguments: MacroArguments::new(),
                })
            }
            tex_state::meaning::UnexpandablePrimitive::FontDimen => {
                let number = scan_int::scan_int_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander,
                )?
                .value();
                if !(1..=32_767).contains(&number) {
                    return Err(ExpandError::UnsupportedTheTarget(token));
                }
                let font = scan_font_selector(input, stores, recorder, hooks, expander)?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(stores.font_dimen(font, number as u16)),
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::HyphenChar => {
                let font = scan_font_selector(input, stores, recorder, hooks, expander)?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.font_hyphen_char(font).to_string(),
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::SkewChar => {
                let font = scan_font_selector(input, stores, recorder, hooks, expander)?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.font_skew_char(font).to_string(),
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Wd
            | tex_state::meaning::UnexpandablePrimitive::Ht
            | tex_state::meaning::UnexpandablePrimitive::Dp => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander,
                )?;
                let dimension = match primitive {
                    tex_state::meaning::UnexpandablePrimitive::Wd => BoxDimension::Width,
                    tex_state::meaning::UnexpandablePrimitive::Ht => BoxDimension::Height,
                    tex_state::meaning::UnexpandablePrimitive::Dp => BoxDimension::Depth,
                    _ => unreachable!("outer match restricts primitive"),
                };
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(
                        stores
                            .box_dimension(index, dimension)
                            .unwrap_or_else(|| Scaled::from_raw(0)),
                    ),
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::CatCode
            | tex_state::meaning::UnexpandablePrimitive::LcCode
            | tex_state::meaning::UnexpandablePrimitive::UcCode
            | tex_state::meaning::UnexpandablePrimitive::SfCode
            | tex_state::meaning::UnexpandablePrimitive::MathCode
            | tex_state::meaning::UnexpandablePrimitive::DelCode => {
                let ch = scan_code_table_char(input, stores, recorder, hooks, expander)?;
                let value = match primitive {
                    tex_state::meaning::UnexpandablePrimitive::CatCode => stores.catcode(ch) as i32,
                    tex_state::meaning::UnexpandablePrimitive::LcCode => stores.lccode(ch) as i32,
                    tex_state::meaning::UnexpandablePrimitive::UcCode => stores.uccode(ch) as i32,
                    tex_state::meaning::UnexpandablePrimitive::SfCode => stores.sfcode(ch) as i32,
                    tex_state::meaning::UnexpandablePrimitive::MathCode => {
                        stores.mathcode(ch) as i32
                    }
                    tex_state::meaning::UnexpandablePrimitive::DelCode => stores.delcode(ch),
                    _ => unreachable!("outer match restricts primitive"),
                };
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &value.to_string(),
                ))
            }
            _ => Err(ExpandError::UnsupportedTheTarget(token)),
        },
        Meaning::CountRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.count(index).to_string(),
        )),
        Meaning::DimenRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_scaled(stores.dimen(index)),
        )),
        Meaning::SkipRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_glue(stores.glue(stores.skip(index))),
        )),
        Meaning::MuskipRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_glue(stores.glue(stores.muskip(index))),
        )),
        Meaning::ToksRegister(index) => Ok(Dispatch::Push {
            replay_kind: ExpansionReplayKind::TheOutput,
            token_list: stores.toks(index),
            macro_arguments: MacroArguments::new(),
        }),
        Meaning::IntParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.int_param(IntParam::new(index)).to_string(),
        )),
        Meaning::DimenParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_scaled(stores.dimen_param(DimenParam::new(index))),
        )),
        Meaning::GlueParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_glue(stores.glue(stores.glue_param(GlueParam::new(index)))),
        )),
        Meaning::TokParam(index) => Ok(Dispatch::Push {
            replay_kind: ExpansionReplayKind::TheOutput,
            token_list: stores.tok_param(TokParam::new(index)),
            macro_arguments: MacroArguments::new(),
        }),
        _ => match stores.resolve(symbol) {
            "count" => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.count(index).to_string(),
                ))
            }
            "dimen" => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(stores.dimen(index)),
                ))
            }
            "toks" => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander,
                )?;
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
            _ => Err(ExpandError::UnsupportedTheTarget(token)),
        },
    }
}

pub(crate) fn push_rendered_text(
    stores: &mut impl ExpansionState,
    replay_kind: ExpansionReplayKind,
    text: &str,
) -> Dispatch {
    push_rendered_tokens(stores, replay_kind, text_tokens(text))
}

pub(crate) fn push_rendered_tokens<I>(
    stores: &mut impl ExpansionState,
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

fn freeze_output_tokens(stores: &mut impl ExpansionState, tokens: &[Token]) -> TokenListId {
    stores.intern_token_list(tokens)
}

pub(crate) fn string_tokens(stores: &impl ExpansionState, token: Token) -> Vec<Token> {
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

pub fn meaning_text(stores: &impl ExpansionState, token: Token) -> String {
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
            Meaning::MathCharGiven(value) => format!("\\mathchar\"{value:X}"),
            Meaning::CountRegister(index) => format!("\\count{index}"),
            Meaning::DimenRegister(index) => format!("\\dimen{index}"),
            Meaning::SkipRegister(index) => format!("\\skip{index}"),
            Meaning::MuskipRegister(index) => format!("\\muskip{index}"),
            Meaning::ToksRegister(index) => format!("\\toks{index}"),
            Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::TokParam(_) => {
                format!("\\{}", stores.resolve(symbol))
            }
            Meaning::Font(_) => format!("select font {}", token_text(stores, token)),
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

fn token_list_text(stores: &impl ExpansionState, token_list: TokenListId) -> String {
    let mut text = String::new();
    for &token in stores.tokens(token_list) {
        text.push_str(&token_text(stores, token));
        if let Token::Cs(symbol) = token {
            let name = stores.resolve(symbol);
            if name.chars().all(|ch| ch.is_ascii_alphabetic()) {
                text.push(' ');
            }
        }
    }
    text
}

pub fn token_text(stores: &impl ExpansionState, token: Token) -> String {
    string_tokens(stores, token)
        .into_iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(ch),
            Token::Cs(_) | Token::Param(_) => None,
        })
        .collect()
}

pub fn scan_the_text_with_hooks<S, R, H>(
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
    let dispatch =
        expand_the_with_expander_and_hooks(input, stores, recorder, hooks, &mut NoInputExpandNext)?;
    Ok(match dispatch {
        Dispatch::Push { token_list, .. } => token_list_text(stores, token_list),
        Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => token_text(stores, token),
        Dispatch::Continue => String::new(),
    })
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

fn escapechar(stores: &impl ExpansionState) -> Option<char> {
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

fn format_glue(spec: GlueSpec) -> String {
    let mut text = format_scaled(spec.width);
    if spec.stretch.raw() != 0 {
        text.push_str(" plus ");
        text.push_str(&format_scaled_without_unit(spec.stretch));
        text.push_str(order_unit(spec.stretch_order));
    }
    if spec.shrink.raw() != 0 {
        text.push_str(" minus ");
        text.push_str(&format_scaled_without_unit(spec.shrink));
        text.push_str(order_unit(spec.shrink_order));
    }
    text
}

fn format_scaled_without_unit(value: Scaled) -> String {
    format_scaled(value).trim_end_matches("pt").to_owned()
}

fn order_unit(order: Order) -> &'static str {
    match order {
        Order::Normal => "pt",
        Order::Fil => "fil",
        Order::Fill => "fill",
        Order::Filll => "filll",
    }
}

fn scan_code_table_char<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<char, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let value =
        scan_int::scan_int_with_expander_and_hooks(input, stores, recorder, hooks, expander)?
            .value();
    u32::try_from(value)
        .ok()
        .and_then(char::from_u32)
        .ok_or(ExpandError::UnsupportedTheTarget(Token::Char {
            ch: '?',
            cat: Catcode::Other,
        }))
}

pub(crate) fn scan_font_selector<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<FontId, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let Some(token) = scan_helpers::next_non_space_x_token_with_expander_and_hooks(
        input, stores, recorder, hooks, expander,
    )?
    else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::FontName,
        ));
    };
    let Token::Cs(symbol) = token else {
        return Err(ExpandError::UnsupportedTheTarget(token));
    };
    match stores.meaning(symbol) {
        Meaning::Font(id) => Ok(id),
        _ => Err(ExpandError::UnsupportedTheTarget(token)),
    }
}
