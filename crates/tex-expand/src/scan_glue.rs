//! Expanded glue and muglue scanning shared by future assignment consumers.

use std::fmt;

use tex_lex::{InputSource, InputStack, LexError, TokenListReplayKind};
use tex_state::ExpansionState;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::scan_dimen::{self, DimensionDiagnostic, ScanDimenError, ScanDimenOptions};
use crate::{
    ExpandError, ExpandNext, ExpansionHooks, NoInputExpandNext, NoopExpansionHooks, NoopRecorder,
    ReadRecorder, scan_helpers, scan_int, semantic_token,
};

/// A successfully scanned glue specification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedGlue {
    id: GlueId,
    diagnostics: [Option<DimensionDiagnostic>; 8],
    diagnostic_origins: [Option<OriginId>; 8],
}

impl ScannedGlue {
    #[must_use]
    pub const fn id(self) -> GlueId {
        self.id
    }

    pub fn diagnostics(self) -> impl Iterator<Item = DimensionDiagnostic> {
        self.diagnostics.into_iter().flatten()
    }

    pub fn diagnostic_records(self) -> impl Iterator<Item = (DimensionDiagnostic, OriginId)> {
        self.diagnostics
            .into_iter()
            .zip(self.diagnostic_origins)
            .filter_map(|(diagnostic, origin)| Some((diagnostic?, origin?)))
    }
}

#[derive(Debug)]
pub enum ScanGlueError {
    Expand(ExpandError),
    Lex(LexError),
    Dimen(ScanDimenError),
    MissingNumber {
        context: TracedTokenWord,
    },
    RegisterNumberOutOfRange {
        value: i32,
        context: TracedTokenWord,
    },
    UnsupportedInternalGlue {
        context: TracedTokenWord,
    },
}

impl fmt::Display for ScanGlueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expand(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::Dimen(err) => write!(f, "{err}"),
            Self::MissingNumber { .. } => f.write_str("Missing number"),
            Self::RegisterNumberOutOfRange { value, .. } => {
                write!(f, "register number {value} is out of range")
            }
            Self::UnsupportedInternalGlue { context } => {
                write!(
                    f,
                    "unsupported internal glue token {:?}",
                    semantic_token(*context)
                )
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
            Self::MissingNumber { .. }
            | Self::RegisterNumberOutOfRange { .. }
            | Self::UnsupportedInternalGlue { .. } => None,
        }
    }
}

impl ScanGlueError {
    #[must_use]
    pub fn primary_origin(&self) -> Option<OriginId> {
        match self {
            Self::MissingNumber { context } | Self::RegisterNumberOutOfRange { context, .. } => {
                Some(context.origin())
            }
            Self::UnsupportedInternalGlue { context } => Some(context.origin()),
            Self::Dimen(err) => err.primary_origin(),
            Self::Expand(err) => err.primary_origin(),
            Self::Lex(_) => None,
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
    stores: &mut impl ExpansionState,
    context: TracedTokenWord,
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
        context,
    )
}

pub fn scan_muglue<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    context: TracedTokenWord,
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
        context,
    )
}

pub fn scan_glue_with_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    mu: bool,
    context: TracedTokenWord,
) -> Result<ScannedGlue, ScanGlueError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    scan_glue_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut NoInputExpandNext,
        mu,
        context,
    )
}

pub fn scan_glue_with_expander_and_hooks<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    mu: bool,
    context: TracedTokenWord,
) -> Result<ScannedGlue, ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let (negative, first) = scan_signs(input, stores, recorder, hooks, expander)?;
    let Some(first) = first else {
        return Err(ScanGlueError::MissingNumber { context });
    };

    if let Token::Cs(symbol) = semantic_token(first) {
        match stores.meaning(symbol) {
            Meaning::SkipRegister(index) if !mu => {
                consume_optional_space(input, stores, recorder, hooks, expander)?;
                let spec = stores.glue(stores.skip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::MuskipRegister(index) if mu => {
                consume_optional_space(input, stores, recorder, hooks, expander)?;
                let spec = stores.glue(stores.muskip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::GlueParam(index) if !mu => {
                consume_optional_space(input, stores, recorder, hooks, expander)?;
                let spec =
                    stores.glue(stores.glue_param(tex_state::env::banks::GlueParam::new(index)));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::MuGlueParam(index) if mu => {
                consume_optional_space(input, stores, recorder, hooks, expander)?;
                let spec =
                    stores.glue(stores.glue_param(tex_state::env::banks::GlueParam::new(index)));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) if !mu => {
                let index = scan_register_index(input, stores, recorder, hooks, expander, first)?;
                consume_optional_space(input, stores, recorder, hooks, expander)?;
                let spec = stores.glue(stores.skip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) if mu => {
                let index = scan_register_index(input, stores, recorder, hooks, expander, first)?;
                consume_optional_space(input, stores, recorder, hooks, expander)?;
                let spec = stores.glue(stores.muskip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastSkip) if !mu => {
                consume_optional_space(input, stores, recorder, hooks, expander)?;
                return Ok(intern_spec(
                    stores,
                    signed_spec(hooks.last_skip(), negative),
                ));
            }
            _ => {
                let name = stores.resolve(symbol);
                if (!mu && name == "skip") || (mu && name == "muskip") {
                    let index =
                        scan_register_index(input, stores, recorder, hooks, expander, first)?;
                    consume_optional_space(input, stores, recorder, hooks, expander)?;
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
    let width = scan_dimen::scan_dimen_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        expander,
        dimen_options(mu),
        context,
    )?;
    let mut diagnostics = [None; 8];
    let mut diagnostic_origins = [None; 8];
    append_dimension_diagnostics(&mut diagnostics, &mut diagnostic_origins, width);
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

    if scan_keyword(input, stores, recorder, hooks, expander, "plus")? {
        let stretch = scan_dimen::scan_dimen_with_expander_and_hooks(
            input,
            stores,
            recorder,
            hooks,
            expander,
            dimen_options(mu).with_infinite_units(),
            context,
        )?;
        append_dimension_diagnostics(&mut diagnostics, &mut diagnostic_origins, stretch);
        spec.stretch = stretch.value();
        spec.stretch_order = stretch.order();
    }
    if scan_keyword(input, stores, recorder, hooks, expander, "minus")? {
        let shrink = scan_dimen::scan_dimen_with_expander_and_hooks(
            input,
            stores,
            recorder,
            hooks,
            expander,
            dimen_options(mu).with_infinite_units(),
            context,
        )?;
        append_dimension_diagnostics(&mut diagnostics, &mut diagnostic_origins, shrink);
        spec.shrink = shrink.value();
        spec.shrink_order = shrink.order();
    }

    Ok(intern_spec_with_diagnostics(
        stores,
        spec,
        diagnostics,
        diagnostic_origins,
    ))
}

fn dimen_options(mu: bool) -> ScanDimenOptions {
    if mu {
        ScanDimenOptions::STANDARD.requiring_mu_unit()
    } else {
        ScanDimenOptions::STANDARD
    }
}

fn intern_spec(stores: &mut impl ExpansionState, spec: GlueSpec) -> ScannedGlue {
    intern_spec_with_diagnostics(stores, spec, [None; 8], [None; 8])
}

fn intern_spec_with_diagnostics(
    stores: &mut impl ExpansionState,
    spec: GlueSpec,
    diagnostics: [Option<DimensionDiagnostic>; 8],
    diagnostic_origins: [Option<OriginId>; 8],
) -> ScannedGlue {
    ScannedGlue {
        id: stores.intern_glue(spec),
        diagnostics,
        diagnostic_origins,
    }
}

fn append_dimension_diagnostics(
    diagnostics: &mut [Option<DimensionDiagnostic>; 8],
    diagnostic_origins: &mut [Option<OriginId>; 8],
    dimen: scan_dimen::ScannedDimen,
) {
    for (diagnostic, origin) in dimen.diagnostic_records() {
        if let Some(index) = diagnostics.iter().position(Option::is_none) {
            diagnostics[index] = Some(diagnostic);
            diagnostic_origins[index] = Some(origin);
        }
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

fn scan_signs<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<(bool, Option<TracedTokenWord>), ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let mut negative = false;
    loop {
        let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
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

fn next_x<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<Option<TracedTokenWord>, ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    Ok(expander.next_expanded_token(input, stores, recorder, hooks)?)
}

fn scan_register_index<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    context: TracedTokenWord,
) -> Result<u16, ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let scanned = crate::scan_int::scan_int_with_expander_and_hooks(
        input, stores, recorder, hooks, expander, context,
    )?;
    let value = scanned.value();
    if !(0..=32_767).contains(&value) {
        return Err(ScanGlueError::RegisterNumberOutOfRange {
            value,
            context: scanned.context(),
        });
    }
    Ok(value as u16)
}

fn scan_keyword<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    keyword: &str,
) -> Result<bool, ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    Ok(scan_helpers::scan_optional_keyword_with_expander_and_hooks(
        input, stores, recorder, hooks, expander, keyword,
    )?)
}

fn consume_optional_space<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<(), ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
        return Ok(());
    };
    if !is_space(token) {
        unread_token(input, stores, token);
    }
    Ok(())
}

fn unread_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    token: TracedTokenWord,
) where
    S: InputSource,
{
    unread_tokens(input, stores, [token]);
}

fn unread_tokens<S, I>(input: &mut InputStack<S>, stores: &mut impl ExpansionState, tokens: I)
where
    S: InputSource,
    I: IntoIterator<Item = TracedTokenWord>,
{
    let traced_tokens = tokens.into_iter().collect::<Vec<_>>();
    let tokens = traced_tokens
        .iter()
        .copied()
        .map(semantic_token)
        .collect::<Vec<_>>();
    let token_list = stores.intern_token_list(&tokens);
    let mut origins = stores.origin_list_builder();
    for token in traced_tokens {
        origins.push(token.origin());
    }
    let origin_list = stores.finish_origin_list(&mut origins);
    input.push_token_list_with_origins(token_list, origin_list, TokenListReplayKind::Inserted);
}

fn is_space(token: TracedTokenWord) -> bool {
    matches!(
        semantic_token(token),
        Token::Char {
            cat: Catcode::Space,
            ..
        }
    )
}

fn is_other_char(token: TracedTokenWord, expected: char) -> bool {
    matches!(
        semantic_token(token),
        Token::Char {
            ch,
            cat: Catcode::Other
        } if ch == expected
    )
}

#[cfg(test)]
mod tests;
