//! Expanded integer scanning shared by conditionals and future stomach code.

use std::fmt;

use tex_lex::{InputSource, InputStack, LexError};
use tex_state::ExpansionState;
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::interner::Symbol;
use tex_state::meaning::{ExpandablePrimitive, InternalInteger, Meaning};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::{
    ExpandError, ExpandNext, ExpansionHooks, NoInputExpandNext, NoopExpansionHooks, NoopRecorder,
    ReadBank, ReadCodeTable, ReadDependency, ReadRecorder, semantic_token,
};

const INT_MAX: i64 = i32::MAX as i64;
const MAX_REGISTER: i32 = 32_767;

/// A successfully scanned TeX integer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedInt {
    value: i32,
    diagnostic: Option<IntegerDiagnostic>,
    context: TracedTokenWord,
    diagnostic_context: Option<TracedTokenWord>,
    diagnostic_origin: Option<OriginId>,
}

impl ScannedInt {
    #[must_use]
    pub const fn new(value: i32, context: TracedTokenWord) -> Self {
        Self {
            value,
            diagnostic: None,
            context,
            diagnostic_context: None,
            diagnostic_origin: None,
        }
    }

    #[must_use]
    pub const fn with_diagnostic(
        value: i32,
        diagnostic: IntegerDiagnostic,
        context: TracedTokenWord,
    ) -> Self {
        Self {
            value,
            diagnostic: Some(diagnostic),
            context,
            diagnostic_context: Some(context),
            diagnostic_origin: Some(context.origin()),
        }
    }

    #[must_use]
    pub const fn with_diagnostic_origin(
        value: i32,
        diagnostic: IntegerDiagnostic,
        context: TracedTokenWord,
        origin: OriginId,
    ) -> Self {
        Self {
            value,
            diagnostic: Some(diagnostic),
            context,
            diagnostic_context: Some(context),
            diagnostic_origin: Some(origin),
        }
    }

    #[must_use]
    pub const fn value(self) -> i32 {
        self.value
    }

    #[must_use]
    pub const fn diagnostic(self) -> Option<IntegerDiagnostic> {
        self.diagnostic
    }

    #[must_use]
    pub const fn diagnostic_origin(self) -> Option<OriginId> {
        self.diagnostic_origin
    }

    #[must_use]
    pub const fn context(self) -> TracedTokenWord {
        self.context
    }

    #[must_use]
    pub const fn diagnostic_context(self) -> Option<TracedTokenWord> {
        self.diagnostic_context
    }
}

/// Recoverable diagnostics emitted while still producing TeX's capped value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegerDiagnostic {
    MissingNumber,
    NumberTooBig,
}

impl fmt::Display for IntegerDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingNumber => f.write_str("Missing number, treated as zero"),
            Self::NumberTooBig => f.write_str("Number too big"),
        }
    }
}

/// Errors that prevent integer scanning from producing a value.
#[derive(Debug)]
pub enum ScanIntError {
    Expand(ExpandError),
    Lex(LexError),
    MissingNumber {
        context: TracedTokenWord,
    },
    RegisterNumberOutOfRange {
        value: i32,
        context: TracedTokenWord,
    },
    UnsupportedInternalInteger {
        context: TracedTokenWord,
    },
}

impl fmt::Display for ScanIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expand(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::MissingNumber { .. } => f.write_str("Missing number"),
            Self::RegisterNumberOutOfRange { value, .. } => {
                write!(f, "register number {value} is out of range")
            }
            Self::UnsupportedInternalInteger { context } => {
                write!(
                    f,
                    "unsupported internal integer token {:?}",
                    semantic_token(*context)
                )
            }
        }
    }
}

impl std::error::Error for ScanIntError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Expand(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::MissingNumber { .. }
            | Self::RegisterNumberOutOfRange { .. }
            | Self::UnsupportedInternalInteger { .. } => None,
        }
    }
}

impl ScanIntError {
    #[must_use]
    pub fn primary_origin(&self) -> Option<OriginId> {
        match self {
            Self::MissingNumber { context } | Self::RegisterNumberOutOfRange { context, .. } => {
                Some(context.origin())
            }
            Self::UnsupportedInternalInteger { context } => Some(context.origin()),
            Self::Expand(err) => err.primary_origin(),
            Self::Lex(err) => err.diagnostic_site().primary_origin(),
        }
    }
}

impl From<ExpandError> for ScanIntError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

impl From<LexError> for ScanIntError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

/// Scans a TeX `<number>` using expanded tokens.
///
/// Supported internal integers are the state surfaces implemented so far:
/// `\count<number>`, `\dimen<number>` coerced to scaled points, `\endlinechar`,
/// and chardef-like meanings represented by [`Meaning::CharGiven`].
pub fn scan_int<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    context: TracedTokenWord,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
{
    scan_int_with_recorder_and_hooks(
        input,
        stores,
        &mut NoopRecorder,
        &mut NoopExpansionHooks,
        context,
    )
}

/// Scans a TeX `<number>` while preserving caller-supplied expansion hooks.
pub fn scan_int_with_recorder_and_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    scan_int_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut NoInputExpandNext,
        context,
    )
}

pub fn scan_int_with_expander_and_hooks<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    context: TracedTokenWord,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let (negative, token) = scan_signs(input, stores, recorder, hooks, expander)?;
    let Some(token) = token else {
        return Ok(ScannedInt::with_diagnostic(
            0,
            IntegerDiagnostic::MissingNumber,
            context,
        ));
    };

    let scanned = scan_unsigned_after_first_token(input, stores, recorder, hooks, expander, token)?;
    Ok(apply_sign(scanned, negative))
}

fn next_x<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<Option<TracedTokenWord>, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    Ok(expander.next_expanded_token(input, stores, recorder, hooks)?)
}

fn scan_signs<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<(bool, Option<TracedTokenWord>), ScanIntError>
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
        if is_char(token, '+') {
            continue;
        }
        if is_char(token, '-') {
            negative = !negative;
            continue;
        }
        return Ok((negative, Some(token)));
    }
}

fn scan_unsigned_after_first_token<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    token: TracedTokenWord,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    match semantic_token(token) {
        Token::Char {
            ch,
            cat: Catcode::Other,
        } if ch.is_ascii_digit() => scan_radix_digits(
            input,
            stores,
            recorder,
            hooks,
            expander,
            10,
            (digit_value(ch).expect("matched digit"), token),
        ),
        Token::Char {
            ch: '\'',
            cat: Catcode::Other,
        } => scan_prefixed_digits(input, stores, recorder, hooks, expander, 8, token),
        Token::Char {
            ch: '"',
            cat: Catcode::Other,
        } => scan_prefixed_digits(input, stores, recorder, hooks, expander, 16, token),
        Token::Char {
            ch: '`',
            cat: Catcode::Other,
        } => scan_backtick_constant(input, stores, recorder, hooks, expander, token),
        Token::Cs(symbol) => {
            scan_internal_integer(input, stores, recorder, hooks, expander, token, symbol)
        }
        _ => {
            unread_token(input, stores, token);
            Ok(missing_number(token))
        }
    }
}

fn scan_prefixed_digits<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    radix: i64,
    prefix: TracedTokenWord,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
        return Ok(missing_number(prefix));
    };
    let Some(digit) = token_digit_for_radix(token, radix) else {
        unread_token(input, stores, token);
        return Ok(missing_number(token));
    };
    scan_radix_digits(
        input,
        stores,
        recorder,
        hooks,
        expander,
        radix,
        (digit, token),
    )
}

fn scan_radix_digits<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    radix: i64,
    first: (i64, TracedTokenWord),
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let (first_digit, first_token) = first;
    let first_delivery = input.take_direct_source_delivery(first_token);
    let mut last_delivery = first_delivery;
    let mut value = first_digit;
    let mut overflow = value > INT_MAX;
    let mut overflow_context = None;
    let mut last_digit = first_token;
    loop {
        let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
            break;
        };
        let delivery = input.take_direct_source_delivery(token);
        let Some(digit) = token_digit_for_radix(token, radix) else {
            if !is_space(token) {
                unread_token(input, stores, token);
            }
            break;
        };
        last_digit = token;
        last_delivery = delivery;
        match value
            .checked_mul(radix)
            .and_then(|value| value.checked_add(digit))
        {
            Some(next) if next <= INT_MAX => value = next,
            _ => {
                overflow = true;
                overflow_context.get_or_insert(token);
                value = INT_MAX;
            }
        }
    }

    let joined_origin = first_delivery
        .zip(last_delivery)
        .and_then(|(first, last)| input.join_direct_source_deliveries(stores, first, last));
    Ok(scanned_unsigned(
        value,
        overflow,
        overflow_context,
        last_digit,
        joined_origin,
    ))
}

fn scan_backtick_constant<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    prefix: TracedTokenWord,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    // TeX's `scan_int` reads the token following a backtick directly, rather
    // than through `get_x_token`. In particular, `\{` is a valid character
    // constant here even if that control symbol has no meaning.
    let Some(token) = input.next_traced_token(stores)? else {
        return Ok(missing_number(prefix));
    };
    let value = match semantic_token(token) {
        Token::Char { ch, .. } => ch as i32,
        Token::Cs(symbol) => stores
            .resolve(symbol)
            .chars()
            .next()
            .map(|ch| ch as i32)
            .unwrap_or(0),
        Token::Param(_) | Token::Frozen(_) => return Ok(missing_number(token)),
    };
    consume_optional_space(input, stores, recorder, hooks, expander)?;
    Ok(ScannedInt::new(value, token))
}

pub(crate) fn scan_internal_integer<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    token: TracedTokenWord,
    symbol: Symbol,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let meaning = stores.meaning(symbol);
    recorder.record_meaning(symbol, meaning);
    crate::values::record_meaning_value_dependency(recorder, meaning);
    match meaning {
        Meaning::CharGiven(ch) => Ok(ScannedInt::new(ch as i32, token)),
        Meaning::MathCharGiven(value) => Ok(ScannedInt::new(i32::from(value), token)),
        Meaning::CountRegister(index) => {
            recorder.record_dependency(ReadDependency::Cell {
                bank: ReadBank::Count,
                index: u32::from(index),
            });
            Ok(ScannedInt::new(stores.count(index), token))
        }
        Meaning::DimenRegister(index) => {
            recorder.record_dependency(ReadDependency::Cell {
                bank: ReadBank::Dimen,
                index: u32::from(index),
            });
            Ok(ScannedInt::new(stores.dimen(index).raw(), token))
        }
        Meaning::IntParam(index) => {
            recorder.record_dependency(ReadDependency::Cell {
                bank: ReadBank::IntParam,
                index: u32::from(index),
            });
            Ok(ScannedInt::new(
                stores.int_param(IntParam::new(index)),
                token,
            ))
        }
        Meaning::InternalInteger(InternalInteger::Badness) => {
            recorder.record_dependency(ReadDependency::Cell {
                bank: ReadBank::LastBadness,
                index: 0,
            });
            Ok(ScannedInt::new(stores.last_badness(), token))
        }
        Meaning::InternalInteger(InternalInteger::InputLineNumber) => {
            recorder.record_dependency(ReadDependency::InputLine);
            let line = input
                .current_source_frame()
                .map_or(0, |frame| frame.line_number().min(i32::MAX as usize) as i32);
            Ok(ScannedInt::new(line, token))
        }
        Meaning::DimenParam(index) => {
            recorder.record_dependency(ReadDependency::Cell {
                bank: ReadBank::DimenParam,
                index: u32::from(index),
            });
            Ok(ScannedInt::new(
                stores.dimen_param(DimenParam::new(index)).raw(),
                token,
            ))
        }
        Meaning::SkipRegister(index) => Ok(ScannedInt::new(
            {
                recorder.record_dependency(ReadDependency::Cell {
                    bank: ReadBank::Skip,
                    index: u32::from(index),
                });
                stores.glue(stores.skip(index)).width.raw()
            },
            token,
        )),
        Meaning::MuskipRegister(index) => Ok(ScannedInt::new(
            {
                recorder.record_dependency(ReadDependency::Cell {
                    bank: ReadBank::Muskip,
                    index: u32::from(index),
                });
                stores.glue(stores.muskip(index)).width.raw()
            },
            token,
        )),
        Meaning::GlueParam(index) | Meaning::MuGlueParam(index) => {
            recorder.record_dependency(ReadDependency::Cell {
                bank: ReadBank::GlueParam,
                index: u32::from(index),
            });
            let glue = stores.glue_param(tex_state::env::banks::GlueParam::new(index));
            Ok(ScannedInt::new(stores.glue(glue).width.raw(), token))
        }
        Meaning::PageInteger(integer) => Ok(ScannedInt::new(stores.page_integer(integer), token)),
        Meaning::PageDimension(dimension) => Ok(ScannedInt::new(
            stores.page_dimension(dimension).raw(),
            token,
        )),
        Meaning::Relax => {
            unread_token(input, stores, token);
            Ok(missing_number(token))
        }
        Meaning::UnexpandablePrimitive(primitive) => scan_internal_integer_primitive(
            input, stores, recorder, hooks, expander, token, primitive,
        ),
        _ => {
            let name = stores.resolve(symbol);
            match name {
                "count" => {
                    let index =
                        scan_register_index(input, stores, recorder, hooks, expander, token)?;
                    let value = stores.count(index);
                    consume_optional_space(input, stores, recorder, hooks, expander)?;
                    Ok(ScannedInt::new(value, token))
                }
                "dimen" => {
                    let index =
                        scan_register_index(input, stores, recorder, hooks, expander, token)?;
                    let value = stores.dimen(index).raw();
                    consume_optional_space(input, stores, recorder, hooks, expander)?;
                    Ok(ScannedInt::new(value, token))
                }
                "endlinechar" => {
                    consume_optional_space(input, stores, recorder, hooks, expander)?;
                    Ok(ScannedInt::new(
                        stores.int_param(IntParam::END_LINE_CHAR),
                        token,
                    ))
                }
                _ => {
                    // TeX only enters `scan_something_internal` when the
                    // command code lies in `min_internal..=max_internal`.
                    // A control sequence whose unexpandable meaning is an
                    // ordinary command follows numeric-constant recovery:
                    // report a missing number and leave the command for the
                    // stomach. Aliased character tokens take the same path.
                    unread_token(input, stores, token);
                    Ok(missing_number(token))
                }
            }
        }
    }
}

fn scan_internal_integer_primitive<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    token: TracedTokenWord,
    primitive: tex_state::meaning::UnexpandablePrimitive,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    match primitive {
        tex_state::meaning::UnexpandablePrimitive::Count => {
            let index = scan_register_index(input, stores, recorder, hooks, expander, token)?;
            recorder.record_dependency(ReadDependency::Cell {
                bank: ReadBank::Count,
                index: u32::from(index),
            });
            let value = stores.count(index);
            Ok(ScannedInt::new(value, token))
        }
        tex_state::meaning::UnexpandablePrimitive::Dimen => {
            let index = scan_register_index(input, stores, recorder, hooks, expander, token)?;
            recorder.record_dependency(ReadDependency::Cell {
                bank: ReadBank::Dimen,
                index: u32::from(index),
            });
            let value = stores.dimen(index).raw();
            Ok(ScannedInt::new(value, token))
        }
        tex_state::meaning::UnexpandablePrimitive::SpaceFactor => {
            Ok(ScannedInt::new(hooks.space_factor(), token))
        }
        tex_state::meaning::UnexpandablePrimitive::PrevDepth => {
            Ok(ScannedInt::new(hooks.prev_depth().raw(), token))
        }
        tex_state::meaning::UnexpandablePrimitive::PrevGraf => {
            Ok(ScannedInt::new(hooks.prev_graf(), token))
        }
        tex_state::meaning::UnexpandablePrimitive::ParShape => {
            Ok(ScannedInt::new(hooks.par_shape_len(), token))
        }
        tex_state::meaning::UnexpandablePrimitive::LastPenalty => {
            Ok(ScannedInt::new(hooks.last_penalty(), token))
        }
        tex_state::meaning::UnexpandablePrimitive::LastKern => {
            Ok(ScannedInt::new(hooks.last_kern().raw(), token))
        }
        tex_state::meaning::UnexpandablePrimitive::LastSkip => {
            Ok(ScannedInt::new(hooks.last_skip().width.raw(), token))
        }
        tex_state::meaning::UnexpandablePrimitive::CatCode
        | tex_state::meaning::UnexpandablePrimitive::LcCode
        | tex_state::meaning::UnexpandablePrimitive::UcCode
        | tex_state::meaning::UnexpandablePrimitive::SfCode
        | tex_state::meaning::UnexpandablePrimitive::MathCode
        | tex_state::meaning::UnexpandablePrimitive::DelCode => {
            let scanned =
                scan_int_with_expander_and_hooks(input, stores, recorder, hooks, expander, token)?;
            let code = scanned.value();
            let ch = u32::try_from(code).ok().and_then(char::from_u32).ok_or(
                ScanIntError::RegisterNumberOutOfRange {
                    value: code,
                    context: scanned.context(),
                },
            )?;
            let value = match primitive {
                tex_state::meaning::UnexpandablePrimitive::CatCode => {
                    recorder
                        .record_dependency(ReadDependency::CodeGeneration(ReadCodeTable::Catcode));
                    recorder.record_dependency(ReadDependency::Code {
                        table: ReadCodeTable::Catcode,
                        scalar: ch as u32,
                    });
                    stores.catcode(ch) as i32
                }
                tex_state::meaning::UnexpandablePrimitive::LcCode => {
                    recorder
                        .record_dependency(ReadDependency::CodeGeneration(ReadCodeTable::Lccode));
                    recorder.record_dependency(ReadDependency::Code {
                        table: ReadCodeTable::Lccode,
                        scalar: ch as u32,
                    });
                    stores.lccode(ch) as i32
                }
                tex_state::meaning::UnexpandablePrimitive::UcCode => {
                    recorder
                        .record_dependency(ReadDependency::CodeGeneration(ReadCodeTable::Uccode));
                    recorder.record_dependency(ReadDependency::Code {
                        table: ReadCodeTable::Uccode,
                        scalar: ch as u32,
                    });
                    stores.uccode(ch) as i32
                }
                tex_state::meaning::UnexpandablePrimitive::SfCode => {
                    recorder
                        .record_dependency(ReadDependency::CodeGeneration(ReadCodeTable::Sfcode));
                    recorder.record_dependency(ReadDependency::Code {
                        table: ReadCodeTable::Sfcode,
                        scalar: ch as u32,
                    });
                    stores.sfcode(ch) as i32
                }
                tex_state::meaning::UnexpandablePrimitive::MathCode => {
                    recorder
                        .record_dependency(ReadDependency::CodeGeneration(ReadCodeTable::Mathcode));
                    recorder.record_dependency(ReadDependency::Code {
                        table: ReadCodeTable::Mathcode,
                        scalar: ch as u32,
                    });
                    stores.mathcode(ch) as i32
                }
                tex_state::meaning::UnexpandablePrimitive::DelCode => {
                    recorder
                        .record_dependency(ReadDependency::CodeGeneration(ReadCodeTable::Delcode));
                    recorder.record_dependency(ReadDependency::Code {
                        table: ReadCodeTable::Delcode,
                        scalar: ch as u32,
                    });
                    stores.delcode(ch)
                }
                _ => unreachable!("outer match restricts primitive"),
            };
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            Ok(ScannedInt::new(value, token))
        }
        primitive if is_internal_numeric_primitive(primitive) => {
            Err(ScanIntError::UnsupportedInternalInteger { context: token })
        }
        _ => {
            unread_token(input, stores, token);
            Ok(missing_number(token))
        }
    }
}

const fn is_internal_numeric_primitive(
    primitive: tex_state::meaning::UnexpandablePrimitive,
) -> bool {
    use tex_state::meaning::UnexpandablePrimitive as Primitive;

    matches!(
        primitive,
        Primitive::Count
            | Primitive::Dimen
            | Primitive::Skip
            | Primitive::Muskip
            | Primitive::Toks
            | Primitive::CatCode
            | Primitive::LcCode
            | Primitive::UcCode
            | Primitive::SfCode
            | Primitive::MathCode
            | Primitive::DelCode
            | Primitive::FontDimen
            | Primitive::HyphenChar
            | Primitive::SkewChar
            | Primitive::ParShape
            | Primitive::PrevDepth
            | Primitive::PrevGraf
            | Primitive::Wd
            | Primitive::Ht
            | Primitive::Dp
            | Primitive::SpaceFactor
            | Primitive::LastPenalty
            | Primitive::LastKern
            | Primitive::LastSkip
    )
}

fn scan_register_index<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    context: TracedTokenWord,
) -> Result<u16, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let scanned =
        scan_int_with_expander_and_hooks(input, stores, recorder, hooks, expander, context)?;
    let value = scanned.value();
    if !(0..=MAX_REGISTER).contains(&value) {
        return Err(ScanIntError::RegisterNumberOutOfRange {
            value,
            context: scanned.context(),
        });
    }
    Ok(value as u16)
}

fn consume_optional_space<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    _recorder: &mut R,
    _hooks: &mut H,
    _expander: &mut E,
) -> Result<(), ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    // TeX consumes at most one following *raw* space after an internal
    // integer. Expanding here would execute the next command while an
    // assignment is still scanning its operand (plain.tex's
    // `\escapechar\m@ne\expandafter...` is a real instance).
    let Some(token) = input.next_traced_token(stores)? else {
        return Ok(());
    };
    if is_space(token) {
        return Ok(());
    }

    let is_conditional_delimiter = matches!(
        semantic_token(token),
        Token::Cs(symbol)
            if matches!(
                stores.meaning(symbol),
                Meaning::ExpandablePrimitive(
                    ExpandablePrimitive::Else | ExpandablePrimitive::Or | ExpandablePrimitive::Fi
                )
            )
    );
    unread_token(input, stores, token);
    if is_conditional_delimiter
        && let Some(inserted) = next_x(input, stores, _recorder, _hooks, _expander)?
    {
        unread_token(input, stores, inserted);
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
    crate::back_input(input, stores, [token]);
}

fn apply_sign(scanned: ScannedInt, negative: bool) -> ScannedInt {
    let value = if negative {
        -scanned.value()
    } else {
        scanned.value()
    };
    ScannedInt {
        value,
        diagnostic: scanned.diagnostic(),
        context: scanned.context(),
        diagnostic_context: scanned.diagnostic_context(),
        diagnostic_origin: scanned.diagnostic_origin(),
    }
}

fn scanned_unsigned(
    value: i64,
    overflow: bool,
    overflow_context: Option<TracedTokenWord>,
    context: TracedTokenWord,
    joined_origin: Option<OriginId>,
) -> ScannedInt {
    if overflow {
        let diagnostic_context = overflow_context.unwrap_or(context);
        joined_origin.map_or_else(
            || {
                ScannedInt::with_diagnostic(
                    i32::MAX,
                    IntegerDiagnostic::NumberTooBig,
                    diagnostic_context,
                )
            },
            |origin| {
                ScannedInt::with_diagnostic_origin(
                    i32::MAX,
                    IntegerDiagnostic::NumberTooBig,
                    diagnostic_context,
                    origin,
                )
            },
        )
    } else {
        ScannedInt::new(value as i32, context)
    }
}

const fn missing_number(context: TracedTokenWord) -> ScannedInt {
    ScannedInt::with_diagnostic(0, IntegerDiagnostic::MissingNumber, context)
}

fn token_digit_for_radix(token: TracedTokenWord, radix: i64) -> Option<i64> {
    let Token::Char { ch, .. } = semantic_token(token) else {
        return None;
    };
    let digit = digit_value(ch)?;
    (digit < radix).then_some(digit)
}

fn digit_value(ch: char) -> Option<i64> {
    match ch {
        '0'..='9' => Some(i64::from(ch as u8 - b'0')),
        'a'..='f' => Some(i64::from(ch as u8 - b'a' + 10)),
        'A'..='F' => Some(i64::from(ch as u8 - b'A' + 10)),
        _ => None,
    }
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

fn is_char(token: TracedTokenWord, expected: char) -> bool {
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
