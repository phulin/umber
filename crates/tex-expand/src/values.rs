use tex_lex::{InputSource, InputStack, MacroArguments};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{FontId, TokenListId};
use tex_state::math::MathFontSize;
use tex_state::meaning::{InternalInteger, Meaning, MeaningFlags};
use tex_state::provenance::SynthesizedOriginKind;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
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
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    expand_the_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut NoInputExpandNext,
        context,
    )
}

pub(crate) fn expand_the_with_expander_and_hooks<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    cause_context: TracedTokenWord,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let cause_origin = cause_context.origin();
    let Some(token) = scan_helpers::next_non_space_x_token_with_expander_and_hooks(
        input, stores, recorder, hooks, expander,
    )?
    else {
        return Err(ExpandError::MissingTokenAfterPrimitive {
            opcode: ExpandableOpcode::The,
            context: cause_context,
        });
    };
    let semantic = crate::semantic_token(token);
    let Token::Cs(symbol) = semantic else {
        return Err(ExpandError::UnsupportedTheTarget { context: token });
    };

    match stores.meaning(symbol) {
        Meaning::UnexpandablePrimitive(primitive) => match primitive {
            tex_state::meaning::UnexpandablePrimitive::Count => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander, token,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.count(index).to_string(),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Dimen => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander, token,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(stores.dimen(index)),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Skip => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander, token,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_glue(stores.glue(stores.skip(index))),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Muskip => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander, token,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_muglue(stores.glue(stores.muskip(index))),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Toks => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander, token,
                )?;
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::TheOutput,
                    token_list: stores.toks(index),
                    origin_list: tex_state::ids::OriginListId::EMPTY,
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            tex_state::meaning::UnexpandablePrimitive::Font => {
                let symbol = stores
                    .font_identifier_symbol(stores.current_font())
                    .ok_or(ExpandError::UnsupportedTheTarget { context: token })?;
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::TheOutput,
                    token_list: stores.intern_token_list(&[Token::Cs(symbol)]),
                    origin_list: crate::synthesized_origin_list(
                        stores,
                        1,
                        cause_origin,
                        SynthesizedOriginKind::ValueRendering,
                    ),
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            primitive @ (tex_state::meaning::UnexpandablePrimitive::TextFont
            | tex_state::meaning::UnexpandablePrimitive::ScriptFont
            | tex_state::meaning::UnexpandablePrimitive::ScriptScriptFont) => {
                let family = scan_math_family(input, stores, recorder, hooks, expander, token)?;
                let font = stores.math_family_font(math_font_size(primitive), family);
                let symbol = stores
                    .font_identifier_symbol(font)
                    .ok_or(ExpandError::UnsupportedTheTarget { context: token })?;
                Ok(push_rendered_tokens(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    [Token::Cs(symbol)],
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::FontDimen => {
                let scanned = scan_int::scan_int_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander, token,
                )?;
                let number = scanned.value();
                let font = scan_font_selector(input, stores, recorder, hooks, expander, token)?;
                let available = stores.font_parameter_count(font);
                let number = u16::try_from(number)
                    .ok()
                    .filter(|number| *number > 0 && *number <= available);
                // TeX.web §578 diagnoses an unavailable parameter but
                // points at its zero-valued dummy font-info cell, so \the
                // still expands to a usable dimension.
                let value = number.map_or_else(
                    || tex_state::scaled::Scaled::from_raw(0),
                    |number| stores.font_dimen(font, number),
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(value),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::HyphenChar => {
                let font = scan_font_selector(input, stores, recorder, hooks, expander, token)?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.font_hyphen_char(font).to_string(),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::SkewChar => {
                let font = scan_font_selector(input, stores, recorder, hooks, expander, token)?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.font_skew_char(font).to_string(),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Wd
            | tex_state::meaning::UnexpandablePrimitive::Ht
            | tex_state::meaning::UnexpandablePrimitive::Dp => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander, token,
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
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::SpaceFactor => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &hooks.space_factor().to_string(),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::PrevDepth => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &format_scaled(hooks.prev_depth()),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::PrevGraf => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &hooks.prev_graf().to_string(),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::LastPenalty => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &hooks.last_penalty().to_string(),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::LastKern => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &format_scaled(hooks.last_kern()),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::LastSkip => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &format_glue(hooks.last_skip()),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::CatCode
            | tex_state::meaning::UnexpandablePrimitive::LcCode
            | tex_state::meaning::UnexpandablePrimitive::UcCode
            | tex_state::meaning::UnexpandablePrimitive::SfCode
            | tex_state::meaning::UnexpandablePrimitive::MathCode
            | tex_state::meaning::UnexpandablePrimitive::DelCode => {
                let ch = scan_code_table_char(input, stores, recorder, hooks, expander, token)?;
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
                    cause_origin,
                ))
            }
            _ => Err(ExpandError::UnsupportedTheTarget { context: token }),
        },
        Meaning::CountRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.count(index).to_string(),
            cause_origin,
        )),
        Meaning::DimenRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_scaled(stores.dimen(index)),
            cause_origin,
        )),
        Meaning::SkipRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_glue(stores.glue(stores.skip(index))),
            cause_origin,
        )),
        Meaning::MuskipRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_muglue(stores.glue(stores.muskip(index))),
            cause_origin,
        )),
        Meaning::ToksRegister(index) => Ok(Dispatch::Push {
            replay_kind: ExpansionReplayKind::TheOutput,
            token_list: stores.toks(index),
            origin_list: tex_state::ids::OriginListId::EMPTY,
            macro_arguments: MacroArguments::new(),
            macro_invocation: OriginId::UNKNOWN,
        }),
        Meaning::IntParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.int_param(IntParam::new(index)).to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::Badness) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.last_badness().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::InputLineNumber) => {
            let line = input
                .current_source_frame()
                .map_or(0, |frame| frame.line_number().min(i32::MAX as usize) as i32);
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &line.to_string(),
                cause_origin,
            ))
        }
        Meaning::DimenParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_scaled(stores.dimen_param(DimenParam::new(index))),
            cause_origin,
        )),
        Meaning::PageDimension(dimension) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_scaled(stores.page_dimension(dimension)),
            cause_origin,
        )),
        Meaning::PageInteger(integer) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.page_integer(integer).to_string(),
            cause_origin,
        )),
        Meaning::GlueParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_glue(stores.glue(stores.glue_param(GlueParam::new(index)))),
            cause_origin,
        )),
        Meaning::MuGlueParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_muglue(stores.glue(stores.glue_param(GlueParam::new(index)))),
            cause_origin,
        )),
        Meaning::TokParam(index) => Ok(Dispatch::Push {
            replay_kind: ExpansionReplayKind::TheOutput,
            token_list: stores.tok_param(TokParam::new(index)),
            origin_list: tex_state::ids::OriginListId::EMPTY,
            macro_arguments: MacroArguments::new(),
            macro_invocation: OriginId::UNKNOWN,
        }),
        _ => match stores.resolve(symbol) {
            "count" => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander, token,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.count(index).to_string(),
                    cause_origin,
                ))
            }
            "dimen" => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander, token,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(stores.dimen(index)),
                    cause_origin,
                ))
            }
            "toks" => {
                let index = scan_helpers::scan_register_index_with_expander_and_hooks(
                    input, stores, recorder, hooks, expander, token,
                )?;
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::TheOutput,
                    token_list: stores.toks(index),
                    origin_list: tex_state::ids::OriginListId::EMPTY,
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            "endlinechar" => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &stores.int_param(IntParam::END_LINE_CHAR).to_string(),
                cause_origin,
            )),
            "escapechar" => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &stores.int_param(IntParam::ESCAPE_CHAR).to_string(),
                cause_origin,
            )),
            _ => Err(ExpandError::UnsupportedTheTarget { context: token }),
        },
    }
}

pub(crate) fn push_rendered_text(
    stores: &mut impl ExpansionState,
    replay_kind: ExpansionReplayKind,
    text: &str,
    parent: OriginId,
) -> Dispatch {
    push_rendered_tokens(stores, replay_kind, text_tokens(text), parent)
}

pub(crate) fn push_rendered_tokens<I>(
    stores: &mut impl ExpansionState,
    replay_kind: ExpansionReplayKind,
    tokens: I,
    parent: OriginId,
) -> Dispatch
where
    I: IntoIterator<Item = Token>,
{
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let token_list = freeze_output_tokens(stores, &tokens);
    Dispatch::Push {
        replay_kind,
        token_list,
        origin_list: crate::synthesized_origin_list(
            stores,
            tokens.len(),
            parent,
            SynthesizedOriginKind::ValueRendering,
        ),
        macro_arguments: MacroArguments::new(),
        macro_invocation: OriginId::UNKNOWN,
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
        Token::Frozen(_) => text_tokens("\\endtemplate"),
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
        Token::Frozen(_) => "end of alignment template".to_owned(),
        Token::Cs(symbol) => match stores.meaning(symbol) {
            Meaning::Undefined => "undefined".to_owned(),
            Meaning::Relax => "\\relax".to_owned(),
            Meaning::CharGiven(ch) => format!("the character {ch}"),
            Meaning::CharToken {
                ch,
                cat: Catcode::Letter,
            } => format!("the letter {ch}"),
            Meaning::CharToken { ch, .. } => format!("the character {ch}"),
            Meaning::MathCharGiven(value) => format!("\\mathchar\"{value:X}"),
            Meaning::CountRegister(index) => format!("\\count{index}"),
            Meaning::DimenRegister(index) => format!("\\dimen{index}"),
            Meaning::SkipRegister(index) => format!("\\skip{index}"),
            Meaning::MuskipRegister(index) => format!("\\muskip{index}"),
            Meaning::ToksRegister(index) => format!("\\toks{index}"),
            Meaning::IntParam(_)
            | Meaning::InternalInteger(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_) => {
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
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
        })
        .collect()
}

pub fn scan_the_text_with_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<String, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let dispatch = expand_the_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut NoInputExpandNext,
        context,
    )?;
    Ok(match dispatch {
        Dispatch::Push { token_list, .. } => token_list_text(stores, token_list),
        Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => {
            token_text(stores, crate::semantic_token(token))
        }
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
    let mut raw = i64::from(value.raw());
    let mut out = String::new();
    if raw < 0 {
        out.push('-');
        raw = -raw;
    }
    let unity = i64::from(Scaled::UNITY);
    out.push_str(&(raw / unity).to_string());
    out.push('.');
    let mut scaled = 10 * (raw % unity) + 5;
    let mut delta = 10;
    loop {
        if delta > unity {
            scaled += 0o100000 - 50_000;
        }
        out.push(char::from(
            b'0' + u8::try_from(scaled / unity).expect("scaled digit fits u8"),
        ));
        scaled = 10 * (scaled % unity);
        delta *= 10;
        if scaled <= delta {
            break;
        }
    }
    out.push_str("pt");
    out
}

fn format_glue(spec: GlueSpec) -> String {
    format_glue_with_unit(spec, "pt")
}

fn format_muglue(spec: GlueSpec) -> String {
    format_glue_with_unit(spec, "mu")
}

fn format_glue_with_unit(spec: GlueSpec, unit: &str) -> String {
    let mut text = format_scaled(spec.width);
    replace_unit(&mut text, unit);
    if spec.stretch.raw() != 0 {
        text.push_str(" plus ");
        text.push_str(&format_scaled_without_unit(spec.stretch, unit));
        text.push_str(component_unit(spec.stretch_order, unit));
    }
    if spec.shrink.raw() != 0 {
        text.push_str(" minus ");
        text.push_str(&format_scaled_without_unit(spec.shrink, unit));
        text.push_str(component_unit(spec.shrink_order, unit));
    }
    text
}

fn format_scaled_without_unit(value: Scaled, unit: &str) -> String {
    let mut text = format_scaled(value);
    replace_unit(&mut text, unit);
    text.trim_end_matches(unit).to_owned()
}

fn replace_unit(text: &mut String, unit: &str) {
    if unit != "pt" {
        text.truncate(text.len() - "pt".len());
        text.push_str(unit);
    }
}

fn component_unit(order: Order, normal_unit: &str) -> &'static str {
    match order {
        Order::Normal if normal_unit == "mu" => "mu",
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
    context: TracedTokenWord,
) -> Result<char, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let value = scan_int::scan_int_with_expander_and_hooks(
        input, stores, recorder, hooks, expander, context,
    )?
    .value();
    u32::try_from(value)
        .ok()
        .and_then(char::from_u32)
        .ok_or(ExpandError::UnsupportedTheTarget { context })
}

pub(crate) fn scan_font_selector<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    context: TracedTokenWord,
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
        return Err(ExpandError::MissingTokenAfterPrimitive {
            opcode: ExpandableOpcode::FontName,
            context,
        });
    };
    let semantic = crate::semantic_token(token);
    let Token::Cs(symbol) = semantic else {
        return Err(ExpandError::UnsupportedTheTarget { context: token });
    };
    match stores.meaning(symbol) {
        Meaning::Font(id) => Ok(id),
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Font) => {
            Ok(stores.current_font())
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (tex_state::meaning::UnexpandablePrimitive::TextFont
            | tex_state::meaning::UnexpandablePrimitive::ScriptFont
            | tex_state::meaning::UnexpandablePrimitive::ScriptScriptFont),
        ) => {
            let family = scan_math_family(input, stores, recorder, hooks, expander, token)?;
            Ok(stores.math_family_font(math_font_size(primitive), family))
        }
        _ => Err(ExpandError::MissingFontIdentifier { context: token }),
    }
}

fn scan_math_family<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    context: TracedTokenWord,
) -> Result<u8, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let scanned = scan_int::scan_int_with_expander_and_hooks(
        input, stores, recorder, hooks, expander, context,
    )?;
    Ok(u8::try_from(scanned.value())
        .ok()
        .filter(|family| *family < 16)
        .unwrap_or(0))
}

fn math_font_size(primitive: tex_state::meaning::UnexpandablePrimitive) -> MathFontSize {
    match primitive {
        tex_state::meaning::UnexpandablePrimitive::TextFont => MathFontSize::Text,
        tex_state::meaning::UnexpandablePrimitive::ScriptFont => MathFontSize::Script,
        tex_state::meaning::UnexpandablePrimitive::ScriptScriptFont => MathFontSize::ScriptScript,
        _ => unreachable!("caller restricts math font primitive"),
    }
}
