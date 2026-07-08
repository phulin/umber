//! Expanded glue and muglue scanning shared by future assignment consumers.

use std::fmt;

use tex_lex::{InputSource, InputStack, LexError, TokenListReplayKind};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::stores::Stores;
use tex_state::token::{Catcode, Token};

use crate::scan_dimen::{self, ScanDimenError, ScanDimenOptions};
use crate::{
    ExpandError, ExpansionHooks, NoopExpansionHooks, NoopRecorder, ReadRecorder,
    get_x_token_with_recorder_and_hooks, scan_int,
};

/// A successfully scanned glue specification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedGlue {
    id: GlueId,
}

impl ScannedGlue {
    #[must_use]
    pub const fn id(self) -> GlueId {
        self.id
    }
}

#[derive(Debug)]
pub enum ScanGlueError {
    Expand(ExpandError),
    Lex(LexError),
    Dimen(ScanDimenError),
    MissingNumber,
    RegisterNumberOutOfRange(i32),
    UnsupportedInternalGlue(Token),
}

impl fmt::Display for ScanGlueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expand(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::Dimen(err) => write!(f, "{err}"),
            Self::MissingNumber => f.write_str("Missing number"),
            Self::RegisterNumberOutOfRange(value) => {
                write!(f, "register number {value} is out of range")
            }
            Self::UnsupportedInternalGlue(token) => {
                write!(f, "unsupported internal glue token {token:?}")
            }
        }
    }
}

impl std::error::Error for ScanGlueError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Expand(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::Dimen(err) => Some(err),
            Self::MissingNumber
            | Self::RegisterNumberOutOfRange(_)
            | Self::UnsupportedInternalGlue(_) => None,
        }
    }
}

impl From<ExpandError> for ScanGlueError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

impl From<LexError> for ScanGlueError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<ScanDimenError> for ScanGlueError {
    fn from(value: ScanDimenError) -> Self {
        Self::Dimen(value)
    }
}

impl From<scan_int::ScanIntError> for ScanGlueError {
    fn from(value: scan_int::ScanIntError) -> Self {
        Self::Expand(value.into())
    }
}

pub fn scan_glue<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<ScannedGlue, ScanGlueError>
where
    S: InputSource,
{
    scan_glue_with_hooks(
        input,
        stores,
        &mut NoopRecorder,
        &mut NoopExpansionHooks,
        false,
    )
}

pub fn scan_muglue<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<ScannedGlue, ScanGlueError>
where
    S: InputSource,
{
    scan_glue_with_hooks(
        input,
        stores,
        &mut NoopRecorder,
        &mut NoopExpansionHooks,
        true,
    )
}

pub fn scan_glue_with_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    mu: bool,
) -> Result<ScannedGlue, ScanGlueError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let (negative, first) = scan_signs(input, stores, recorder, hooks)?;
    let Some(first) = first else {
        return Err(ScanGlueError::MissingNumber);
    };

    if let Token::Cs(symbol) = first {
        match stores.meaning(symbol) {
            Meaning::SkipRegister(index) if !mu => {
                consume_optional_space(input, stores, recorder, hooks)?;
                let spec = stores.glue(stores.skip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::MuskipRegister(index) if mu => {
                consume_optional_space(input, stores, recorder, hooks)?;
                let spec = stores.glue(stores.muskip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::GlueParam(index) if !mu => {
                consume_optional_space(input, stores, recorder, hooks)?;
                let spec =
                    stores.glue(stores.glue_param(tex_state::env::banks::GlueParam::new(index)));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) if !mu => {
                let index = scan_register_index(input, stores, recorder, hooks)?;
                consume_optional_space(input, stores, recorder, hooks)?;
                let spec = stores.glue(stores.skip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) if mu => {
                let index = scan_register_index(input, stores, recorder, hooks)?;
                consume_optional_space(input, stores, recorder, hooks)?;
                let spec = stores.glue(stores.muskip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            _ => {
                let name = stores.resolve(symbol);
                if (!mu && name == "skip") || (mu && name == "muskip") {
                    let index = scan_register_index(input, stores, recorder, hooks)?;
                    consume_optional_space(input, stores, recorder, hooks)?;
                    let id = if mu {
                        stores.muskip(index)
                    } else {
                        stores.skip(index)
                    };
                    let spec = stores.glue(id);
                    return Ok(intern_spec(stores, signed_spec(spec, negative)));
                }
            }
        }
    }

    unread_token(input, stores, first);
    let width = scan_dimen::scan_dimen_with_options_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        dimen_options(mu),
    )?;
    let mut spec = GlueSpec {
        width: width.value(),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    };
    if negative {
        spec.width = -spec.width;
    }

    if scan_keyword(input, stores, recorder, hooks, "plus")? {
        let stretch = scan_dimen::scan_dimen_with_options_and_hooks(
            input,
            stores,
            recorder,
            hooks,
            dimen_options(mu).with_infinite_units(),
        )?;
        spec.stretch = stretch.value();
        spec.stretch_order = stretch.order();
    }
    if scan_keyword(input, stores, recorder, hooks, "minus")? {
        let shrink = scan_dimen::scan_dimen_with_options_and_hooks(
            input,
            stores,
            recorder,
            hooks,
            dimen_options(mu).with_infinite_units(),
        )?;
        spec.shrink = shrink.value();
        spec.shrink_order = shrink.order();
    }

    Ok(intern_spec(stores, spec))
}

fn dimen_options(mu: bool) -> ScanDimenOptions {
    if mu {
        ScanDimenOptions::STANDARD.requiring_mu_unit()
    } else {
        ScanDimenOptions::STANDARD
    }
}

fn intern_spec(stores: &mut Stores, spec: GlueSpec) -> ScannedGlue {
    ScannedGlue {
        id: stores.intern_glue(spec),
    }
}

fn signed_spec(mut spec: GlueSpec, negative: bool) -> GlueSpec {
    if negative {
        spec.width = -spec.width;
        spec.stretch = -spec.stretch;
        spec.shrink = -spec.shrink;
    }
    spec
}

fn scan_signs<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(bool, Option<Token>), ScanGlueError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut negative = false;
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
            return Ok((negative, None));
        };
        if is_space(token) {
            continue;
        }
        if is_other_char(token, '+') {
            continue;
        }
        if is_other_char(token, '-') {
            negative = !negative;
            continue;
        }
        return Ok((negative, Some(token)));
    }
}

fn scan_register_index<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<u16, ScanGlueError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let value =
        crate::scan_int::scan_int_with_recorder_and_hooks(input, stores, recorder, hooks)?.value();
    if !(0..=32_767).contains(&value) {
        return Err(ScanGlueError::RegisterNumberOutOfRange(value));
    }
    Ok(value as u16)
}

fn scan_keyword<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    keyword: &str,
) -> Result<bool, ScanGlueError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut consumed = Vec::with_capacity(keyword.len());
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
            return Ok(false);
        };
        if !is_space(token) {
            consumed.push(token);
            break;
        }
    }

    for (index, &expected) in keyword.as_bytes().iter().enumerate() {
        let token = if index == 0 {
            consumed[0]
        } else {
            let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
            else {
                unread_tokens(input, stores, consumed);
                return Ok(false);
            };
            consumed.push(token);
            token
        };
        if !token_matches_keyword_byte(token, expected) {
            unread_tokens(input, stores, consumed);
            return Ok(false);
        }
    }

    Ok(true)
}

fn consume_optional_space<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ScanGlueError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)? else {
        return Ok(());
    };
    if !is_space(token) {
        unread_token(input, stores, token);
    }
    Ok(())
}

fn unread_token<S>(input: &mut InputStack<S>, stores: &mut Stores, token: Token)
where
    S: InputSource,
{
    unread_tokens(input, stores, [token]);
}

fn unread_tokens<S, I>(input: &mut InputStack<S>, stores: &mut Stores, tokens: I)
where
    S: InputSource,
    I: IntoIterator<Item = Token>,
{
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let token_list = stores.intern_token_list(&tokens);
    input.push_token_list(token_list, TokenListReplayKind::Inserted);
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

fn is_space(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::Space,
            ..
        }
    )
}

fn is_other_char(token: Token, expected: char) -> bool {
    matches!(
        token,
        Token::Char {
            ch,
            cat: Catcode::Other
        } if ch == expected
    )
}

#[cfg(test)]
mod tests {
    use tex_lex::{InputStack, MemoryInput};
    use tex_state::glue::{GlueSpec, Order};
    use tex_state::scaled::Scaled;
    use tex_state::stores::Stores;
    use tex_state::token::{Catcode, Token};

    use crate::scan_glue::{scan_glue, scan_muglue};

    fn char_token(ch: char, cat: Catcode) -> Token {
        Token::Char { ch, cat }
    }

    fn scan(input_text: &str) -> (GlueSpec, Option<Token>) {
        let mut stores = Stores::new();
        let mut input = InputStack::new(MemoryInput::new(input_text));
        let scanned = scan_glue(&mut input, &mut stores).expect("glue scan should succeed");
        let spec = stores.glue(scanned.id());
        let next = input
            .next_token(&mut stores)
            .expect("remaining token should lex");
        (spec, next)
    }

    #[test]
    fn scans_width_plus_and_minus_components() {
        let (spec, next) = scan("1pt plus 2pt minus .5pt x");

        assert_eq!(spec.width.raw(), 65_536);
        assert_eq!(spec.stretch.raw(), 131_072);
        assert_eq!(spec.stretch_order, Order::Normal);
        assert_eq!(spec.shrink.raw(), 32_768);
        assert_eq!(spec.shrink_order, Order::Normal);
        assert_eq!(next, Some(char_token('x', Catcode::Letter)));
    }

    #[test]
    fn scans_infinite_orders_case_insensitively() {
        let (spec, _next) = scan("0pt PlUs 1fil minus 2FiLlL x");

        assert_eq!(spec.stretch.raw(), 65_536);
        assert_eq!(spec.stretch_order, Order::Fil);
        assert_eq!(spec.shrink.raw(), 131_072);
        assert_eq!(spec.shrink_order, Order::Filll);
    }

    #[test]
    fn keeps_mixed_component_orders_independent() {
        let (spec, _next) = scan("0pt plus 3fill minus 4fil x");

        assert_eq!(spec.stretch.raw(), 196_608);
        assert_eq!(spec.stretch_order, Order::Fill);
        assert_eq!(spec.shrink.raw(), 262_144);
        assert_eq!(spec.shrink_order, Order::Fil);
    }

    #[test]
    fn restores_partially_matched_component_keyword_tokens() {
        let (spec, next) = scan("1pt plux 2pt");

        assert_eq!(spec.width.raw(), 65_536);
        assert_eq!(spec.stretch.raw(), 0);
        assert_eq!(next, Some(char_token('p', Catcode::Letter)));
    }

    #[test]
    fn omitted_components_stay_zero() {
        let (spec, next) = scan("3pt x");

        assert_eq!(spec.width.raw(), 196_608);
        assert_eq!(spec.stretch.raw(), 0);
        assert_eq!(spec.shrink.raw(), 0);
        assert_eq!(next, Some(char_token('x', Catcode::Letter)));
    }

    #[test]
    fn scans_internal_skip_values() {
        let mut stores = Stores::new();
        stores.intern("skip");
        let id = stores.intern_glue(GlueSpec {
            width: Scaled::from_raw(10),
            stretch: Scaled::from_raw(20),
            stretch_order: Order::Fill,
            shrink: Scaled::from_raw(30),
            shrink_order: Order::Fil,
        });
        stores.set_skip(7, id);
        let mut input = InputStack::new(MemoryInput::new("\\skip7 x"));

        let scanned = scan_glue(&mut input, &mut stores).expect("skip should scan");

        assert_eq!(stores.glue(scanned.id()), stores.glue(id));
    }

    #[test]
    fn scans_muglue_with_mu_units() {
        let mut stores = Stores::new();
        let mut input = InputStack::new(MemoryInput::new("1mu plus 2fil x"));

        let scanned = scan_muglue(&mut input, &mut stores).expect("muglue should scan");
        let spec = stores.glue(scanned.id());

        assert_eq!(spec.width.raw(), 65_536);
        assert_eq!(spec.stretch.raw(), 131_072);
        assert_eq!(spec.stretch_order, Order::Fil);
    }

    #[test]
    fn scans_internal_muskip_values() {
        let mut stores = Stores::new();
        stores.intern("muskip");
        let id = stores.intern_glue(GlueSpec {
            width: Scaled::from_raw(10),
            stretch: Scaled::from_raw(20),
            stretch_order: Order::Fill,
            shrink: Scaled::from_raw(30),
            shrink_order: Order::Fil,
        });
        stores.set_muskip(7, id);
        let mut input = InputStack::new(MemoryInput::new("\\muskip7 x"));

        let scanned = scan_muglue(&mut input, &mut stores).expect("muskip should scan");

        assert_eq!(stores.glue(scanned.id()), stores.glue(id));
    }
}
