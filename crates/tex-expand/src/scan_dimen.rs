//! Expanded dimension scanning shared by conditionals and future stomach code.

use std::fmt;

use tex_lex::{InputSource, InputStack, LexError, TokenListReplayKind};
use tex_state::glue::Order;
use tex_state::interner::Symbol;
use tex_state::scaled::{
    DimensionError, PhysicalUnit, Scaled, round_decimal_fraction, scaled_from_decimal_parts,
};
use tex_state::stores::Stores;
use tex_state::token::{Catcode, Token};

use crate::{
    ExpandError, ExpansionHooks, NoopExpansionHooks, NoopRecorder, ReadRecorder,
    get_x_token_with_recorder_and_hooks, scan_int,
};

const MAX_REGISTER: i32 = 32_767;

/// Dimension scanner context switches for TeX callers with special coercions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanDimenOptions {
    coerce_integer_to_sp: bool,
    allow_infinite_units: bool,
    require_mu_unit: bool,
}

impl ScanDimenOptions {
    /// Standard TeX dimension scanning: physical/internal dimensions only.
    pub const STANDARD: Self = Self {
        coerce_integer_to_sp: false,
        allow_infinite_units: false,
        require_mu_unit: false,
    };

    /// Allows a bare scanned integer to stand for raw scaled points.
    ///
    /// This is intentionally opt-in because ordinary `<dimen>` scanning must
    /// still require a unit after numeric constants.
    #[must_use]
    pub const fn with_integer_to_sp_coercion() -> Self {
        Self {
            coerce_integer_to_sp: true,
            ..Self::STANDARD
        }
    }

    pub(crate) const fn with_infinite_units(mut self) -> Self {
        self.allow_infinite_units = true;
        self
    }

    pub(crate) const fn requiring_mu_unit(mut self) -> Self {
        self.require_mu_unit = true;
        self
    }
}

/// A successfully scanned TeX dimension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedDimen {
    value: Scaled,
    order: Order,
    diagnostic: Option<DimensionDiagnostic>,
}

impl ScannedDimen {
    #[must_use]
    pub const fn new(value: Scaled) -> Self {
        Self {
            value,
            order: Order::Normal,
            diagnostic: None,
        }
    }

    pub const fn with_diagnostic(value: Scaled, diagnostic: DimensionDiagnostic) -> Self {
        Self {
            value,
            order: Order::Normal,
            diagnostic: Some(diagnostic),
        }
    }

    #[must_use]
    pub const fn value(self) -> Scaled {
        self.value
    }

    #[must_use]
    pub(crate) const fn order(self) -> Order {
        self.order
    }

    #[must_use]
    pub const fn diagnostic(self) -> Option<DimensionDiagnostic> {
        self.diagnostic
    }
}

/// Recoverable diagnostics emitted while still producing TeX's capped value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DimensionDiagnostic {
    TooLarge,
}

impl fmt::Display for DimensionDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("Dimension too large"),
        }
    }
}

impl From<DimensionError> for DimensionDiagnostic {
    fn from(value: DimensionError) -> Self {
        match value {
            DimensionError::TooLarge => Self::TooLarge,
        }
    }
}

/// Errors that prevent dimension scanning from producing a value.
#[derive(Debug)]
pub enum ScanDimenError {
    Expand(ExpandError),
    Lex(LexError),
    Integer(scan_int::ScanIntError),
    MissingNumber,
    MissingUnit,
    RegisterNumberOutOfRange(i32),
    UnsupportedFontRelativeUnit(&'static str),
    IncompatibleGlueUnits,
    UnsupportedInternalDimension(Token),
}

impl fmt::Display for ScanDimenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expand(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::Integer(err) => write!(f, "{err}"),
            Self::MissingNumber => f.write_str("Missing number"),
            Self::MissingUnit => f.write_str("Illegal unit of measure"),
            Self::RegisterNumberOutOfRange(value) => {
                write!(f, "register number {value} is out of range")
            }
            Self::UnsupportedFontRelativeUnit(unit) => {
                write!(
                    f,
                    "{unit} units require font metrics, which are not implemented yet"
                )
            }
            Self::IncompatibleGlueUnits => f.write_str("Incompatible glue units"),
            Self::UnsupportedInternalDimension(token) => {
                write!(f, "unsupported internal dimension token {token:?}")
            }
        }
    }
}

impl std::error::Error for ScanDimenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Expand(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::Integer(err) => Some(err),
            Self::MissingNumber
            | Self::MissingUnit
            | Self::RegisterNumberOutOfRange(_)
            | Self::UnsupportedFontRelativeUnit(_)
            | Self::IncompatibleGlueUnits
            | Self::UnsupportedInternalDimension(_) => None,
        }
    }
}

impl From<ExpandError> for ScanDimenError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

impl From<LexError> for ScanDimenError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<scan_int::ScanIntError> for ScanDimenError {
    fn from(value: scan_int::ScanIntError) -> Self {
        Self::Integer(value)
    }
}

/// Scans a TeX `<dimen>` using expanded tokens.
pub fn scan_dimen<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
{
    scan_dimen_with_options_and_hooks(
        input,
        stores,
        &mut NoopRecorder,
        &mut NoopExpansionHooks,
        ScanDimenOptions::STANDARD,
    )
}

/// Scans a TeX `<dimen>` using expanded tokens and caller-specific options.
pub fn scan_dimen_with_options<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
{
    scan_dimen_with_options_and_hooks(
        input,
        stores,
        &mut NoopRecorder,
        &mut NoopExpansionHooks,
        options,
    )
}

/// Scans a TeX `<dimen>` while preserving caller-supplied expansion hooks.
pub fn scan_dimen_with_options_and_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let (negative, token) = scan_signs(input, stores, recorder, hooks)?;
    let Some(token) = token else {
        return Err(ScanDimenError::MissingNumber);
    };

    let scanned = scan_unsigned_after_first_token(input, stores, recorder, hooks, token, options)?;
    consume_optional_space(input, stores, recorder, hooks)?;
    Ok(apply_sign(scanned, negative))
}

fn scan_signs<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(bool, Option<Token>), ScanDimenError>
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

fn scan_unsigned_after_first_token<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    token: Token,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    match token {
        Token::Char {
            ch,
            cat: Catcode::Other,
        } if ch.is_ascii_digit() => {
            let integer = scan_decimal_integer(
                input,
                stores,
                recorder,
                hooks,
                digit_value(ch).expect("digit"),
            )?;
            scan_decimal_tail(input, stores, recorder, hooks, integer, options)
        }
        Token::Char {
            ch: '.' | ',',
            cat: Catcode::Other,
        } => scan_fraction_and_unit(input, stores, recorder, hooks, 0, options),
        Token::Char {
            ch: '\'' | '"' | '`',
            cat: Catcode::Other,
        } => scan_integer_constant_with_unit(input, stores, recorder, hooks, token, options),
        Token::Cs(symbol) => scan_internal_or_numeric_dimension(
            input, stores, recorder, hooks, token, symbol, options,
        ),
        _ => Err(ScanDimenError::MissingNumber),
    }
}

fn scan_decimal_integer<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    first_digit: i32,
) -> Result<i32, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut value = first_digit;
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
            break;
        };
        let Some(digit) = decimal_digit(token) else {
            unread_token(input, stores, token);
            break;
        };
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(digit))
            .unwrap_or(Scaled::MAX_DIMEN.raw() + 1);
    }
    Ok(value)
}

fn scan_decimal_tail<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    integer: i32,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)? else {
        return coerce_or_missing_unit(integer, options);
    };

    if is_decimal_point(token) {
        return scan_fraction_and_unit(input, stores, recorder, hooks, integer, options);
    }

    unread_token(input, stores, token);
    match scan_unit(input, stores, recorder, hooks, options)? {
        Some(unit) => convert_scanned_unit(integer, 0, unit),
        None if options.coerce_integer_to_sp => convert_decimal(integer, 0, PhysicalUnit::Sp),
        None => Err(ScanDimenError::MissingUnit),
    }
}

fn scan_fraction_and_unit<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    integer: i32,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let fraction = scan_fraction(input, stores, recorder, hooks)?;
    let unit =
        scan_unit(input, stores, recorder, hooks, options)?.ok_or(ScanDimenError::MissingUnit)?;
    convert_scanned_unit(integer, fraction, unit)
}

fn scan_fraction<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<i32, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut digits = Vec::new();
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
            break;
        };
        let Some(digit) = decimal_digit(token) else {
            unread_token(input, stores, token);
            break;
        };
        if digits.len() < 17 {
            digits.push(u8::try_from(digit).expect("decimal digit fits u8"));
        }
    }
    Ok(round_decimal_fraction(&digits))
}

fn scan_internal_or_numeric_dimension<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    token: Token,
    symbol: Symbol,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    if stores.resolve(symbol) == "dimen" {
        let index = scan_register_index(input, stores, recorder, hooks)?;
        consume_optional_space(input, stores, recorder, hooks)?;
        return Ok(ScannedDimen::new(stores.dimen(index)));
    }

    unread_token(input, stores, token);
    let scanned = scan_int::scan_int_with_recorder_and_hooks(input, stores, recorder, hooks)?;
    if scanned.diagnostic().is_some() {
        return Ok(ScannedDimen::with_diagnostic(
            Scaled::MAX_DIMEN,
            DimensionDiagnostic::TooLarge,
        ));
    }

    let integer = scanned.value();
    let Some(unit) = scan_unit(input, stores, recorder, hooks, options)? else {
        if options.coerce_integer_to_sp {
            return convert_decimal(integer, 0, PhysicalUnit::Sp);
        }
        return Err(ScanDimenError::MissingUnit);
    };
    convert_scanned_unit(integer, 0, unit)
}

fn scan_integer_constant_with_unit<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    token: Token,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    unread_token(input, stores, token);
    let scanned = scan_int::scan_int_with_recorder_and_hooks(input, stores, recorder, hooks)?;
    if scanned.diagnostic().is_some() {
        return Ok(ScannedDimen::with_diagnostic(
            Scaled::MAX_DIMEN,
            DimensionDiagnostic::TooLarge,
        ));
    }
    let Some(unit) = scan_unit(input, stores, recorder, hooks, options)? else {
        if options.coerce_integer_to_sp {
            return convert_decimal(scanned.value(), 0, PhysicalUnit::Sp);
        }
        return Err(ScanDimenError::MissingUnit);
    };
    convert_scanned_unit(scanned.value(), 0, unit)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeywordMatch {
    Matched,
    FirstTokenMismatch,
    PartialMismatch,
}

fn scan_register_index<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<u16, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let value = scan_int::scan_int_with_recorder_and_hooks(input, stores, recorder, hooks)?.value();
    if !(0..=MAX_REGISTER).contains(&value) {
        return Err(ScanDimenError::RegisterNumberOutOfRange(value));
    }
    Ok(value as u16)
}

fn scan_unit<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    options: ScanDimenOptions,
) -> Result<Option<ScannedUnit>, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    skip_spaces(input, stores, recorder, hooks)?;
    let first = match get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)? {
        Some(token) => token,
        None => return Ok(None),
    };

    if options.allow_infinite_units {
        match keyword_matches(input, stores, recorder, hooks, first, "fil")? {
            KeywordMatch::Matched => {
                let mut order = Order::Fil;
                while keyword(input, stores, recorder, hooks, "l")? {
                    if order != Order::Filll {
                        order = match order {
                            Order::Normal => Order::Fil,
                            Order::Fil => Order::Fill,
                            Order::Fill => Order::Filll,
                            Order::Filll => Order::Filll,
                        };
                    }
                }
                return Ok(Some(ScannedUnit::Infinite(order)));
            }
            KeywordMatch::PartialMismatch => return Ok(None),
            KeywordMatch::FirstTokenMismatch => {}
        }
    }

    if options.require_mu_unit {
        match keyword_matches(input, stores, recorder, hooks, first, "mu")? {
            KeywordMatch::Matched => return Ok(Some(ScannedUnit::Physical(PhysicalUnit::Pt))),
            KeywordMatch::PartialMismatch => return Err(ScanDimenError::IncompatibleGlueUnits),
            KeywordMatch::FirstTokenMismatch => {}
        }
        unread_token(input, stores, first);
        return Err(ScanDimenError::IncompatibleGlueUnits);
    }

    match keyword_matches(input, stores, recorder, hooks, first, "true")? {
        KeywordMatch::Matched => {
            // TODO(umber2-teq/umber2-flt): apply \mag once the magnification
            // parameter/state surface exists. Until then true units are parsed
            // and intentionally left unscaled.
            skip_spaces(input, stores, recorder, hooks)?;
            let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
            else {
                return Ok(None);
            };
            return scan_unit_keyword(input, stores, recorder, hooks, token);
        }
        KeywordMatch::PartialMismatch => return Ok(None),
        KeywordMatch::FirstTokenMismatch => {}
    }

    scan_unit_keyword(input, stores, recorder, hooks, first)
}

fn scan_unit_keyword<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    first: Token,
) -> Result<Option<ScannedUnit>, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Some(second) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)? else {
        unread_token(input, stores, first);
        return Ok(None);
    };

    match unit_from_tokens(first, second) {
        Some(ScannedUnit::Physical(unit)) => Ok(Some(ScannedUnit::Physical(unit))),
        Some(ScannedUnit::Em) => {
            // TODO(fonts): resolve em against the current font's quad.
            Err(ScanDimenError::UnsupportedFontRelativeUnit("em"))
        }
        Some(ScannedUnit::Ex) => {
            // TODO(fonts): resolve ex against the current font's x-height.
            Err(ScanDimenError::UnsupportedFontRelativeUnit("ex"))
        }
        Some(ScannedUnit::Infinite(_)) => unreachable!("unit keywords never return infinity"),
        None => {
            unread_tokens(input, stores, [first, second]);
            Ok(None)
        }
    }
}

fn keyword_matches<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    first: Token,
    keyword: &str,
) -> Result<KeywordMatch, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut consumed = Vec::new();
    consumed.push(first);

    if !token_matches_keyword_byte(first, keyword.as_bytes()[0]) {
        return Ok(KeywordMatch::FirstTokenMismatch);
    }

    for &expected in &keyword.as_bytes()[1..] {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
            unread_tokens(input, stores, consumed);
            return Ok(KeywordMatch::PartialMismatch);
        };
        consumed.push(token);
        if !token_matches_keyword_byte(token, expected) {
            unread_tokens(input, stores, consumed);
            return Ok(KeywordMatch::PartialMismatch);
        }
    }

    Ok(KeywordMatch::Matched)
}

fn keyword<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    keyword: &str,
) -> Result<bool, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    skip_spaces(input, stores, recorder, hooks)?;
    let Some(first) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)? else {
        return Ok(false);
    };
    match keyword_matches(input, stores, recorder, hooks, first, keyword)? {
        KeywordMatch::Matched => Ok(true),
        KeywordMatch::FirstTokenMismatch => {
            unread_token(input, stores, first);
            Ok(false)
        }
        KeywordMatch::PartialMismatch => Ok(false),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScannedUnit {
    Physical(PhysicalUnit),
    Infinite(Order),
    Em,
    Ex,
}

fn unit_from_tokens(first: Token, second: Token) -> Option<ScannedUnit> {
    let first = keyword_char(first)?;
    let second = keyword_char(second)?;
    match (first, second) {
        ('p', 't') => Some(ScannedUnit::Physical(PhysicalUnit::Pt)),
        ('p', 'c') => Some(ScannedUnit::Physical(PhysicalUnit::Pc)),
        ('i', 'n') => Some(ScannedUnit::Physical(PhysicalUnit::In)),
        ('b', 'p') => Some(ScannedUnit::Physical(PhysicalUnit::Bp)),
        ('c', 'm') => Some(ScannedUnit::Physical(PhysicalUnit::Cm)),
        ('m', 'm') => Some(ScannedUnit::Physical(PhysicalUnit::Mm)),
        ('d', 'd') => Some(ScannedUnit::Physical(PhysicalUnit::Dd)),
        ('c', 'c') => Some(ScannedUnit::Physical(PhysicalUnit::Cc)),
        ('s', 'p') => Some(ScannedUnit::Physical(PhysicalUnit::Sp)),
        ('e', 'm') => Some(ScannedUnit::Em),
        ('e', 'x') => Some(ScannedUnit::Ex),
        _ => None,
    }
}

fn convert_decimal(
    integer: i32,
    fraction: i32,
    unit: PhysicalUnit,
) -> Result<ScannedDimen, ScanDimenError> {
    let negative = integer < 0;
    let magnitude = if negative {
        integer.checked_neg().unwrap_or(Scaled::MAX_DIMEN.raw() + 1)
    } else {
        integer
    };
    match scaled_from_decimal_parts(magnitude, fraction, unit) {
        Ok(value) if negative => Ok(ScannedDimen::new(-value)),
        Ok(value) => Ok(ScannedDimen::new(value)),
        Err(error) => Ok(ScannedDimen::with_diagnostic(
            Scaled::MAX_DIMEN,
            DimensionDiagnostic::from(error),
        )),
    }
}

fn convert_scanned_unit(
    integer: i32,
    fraction: i32,
    unit: ScannedUnit,
) -> Result<ScannedDimen, ScanDimenError> {
    match unit {
        ScannedUnit::Physical(unit) => convert_decimal(integer, fraction, unit),
        ScannedUnit::Infinite(order) => {
            let mut scanned = convert_decimal(integer, fraction, PhysicalUnit::Pt)?;
            scanned.order = order;
            Ok(scanned)
        }
        ScannedUnit::Em | ScannedUnit::Ex => unreachable!("font units are handled while scanning"),
    }
}

fn coerce_or_missing_unit(
    integer: i32,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError> {
    if options.coerce_integer_to_sp {
        convert_decimal(integer, 0, PhysicalUnit::Sp)
    } else {
        Err(ScanDimenError::MissingUnit)
    }
}

fn apply_sign(scanned: ScannedDimen, negative: bool) -> ScannedDimen {
    let value = if negative {
        -scanned.value()
    } else {
        scanned.value()
    };
    ScannedDimen {
        value,
        order: scanned.order(),
        diagnostic: scanned.diagnostic(),
    }
}

fn consume_optional_space<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ScanDimenError>
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

fn skip_spaces<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
            return Ok(());
        };
        if !is_space(token) {
            unread_token(input, stores, token);
            return Ok(());
        }
    }
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

fn decimal_digit(token: Token) -> Option<i32> {
    let Token::Char {
        ch,
        cat: Catcode::Other,
    } = token
    else {
        return None;
    };
    digit_value(ch)
}

fn digit_value(ch: char) -> Option<i32> {
    match ch {
        '0'..='9' => Some(i32::from(ch as u8 - b'0')),
        _ => None,
    }
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

fn is_decimal_point(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            ch: '.' | ',',
            cat: Catcode::Other
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

fn token_matches_keyword_byte(token: Token, expected: u8) -> bool {
    let Some(ch) = keyword_char(token) else {
        return false;
    };
    ch == char::from(expected).to_ascii_lowercase()
}

fn keyword_char(token: Token) -> Option<char> {
    let Token::Char {
        ch,
        cat: Catcode::Letter | Catcode::Other,
    } = token
    else {
        return None;
    };
    Some(ch.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use tex_lex::{InputStack, MemoryInput};
    use tex_state::macro_store::MacroMeaning;
    use tex_state::meaning::MeaningFlags;
    use tex_state::scaled::{
        PhysicalUnit, Scaled, round_decimal_fraction, scaled_from_decimal_parts,
    };
    use tex_state::stores::Stores;
    use tex_state::token::{Catcode, Token};

    use crate::scan_dimen::{
        DimensionDiagnostic, ScanDimenError, ScanDimenOptions, scan_dimen, scan_dimen_with_options,
    };

    fn scan(input_text: &str) -> (i32, Option<DimensionDiagnostic>, Option<Token>) {
        let mut stores = Stores::new();
        scan_with_stores(input_text, &mut stores)
    }

    fn scan_with_stores(
        input_text: &str,
        stores: &mut Stores,
    ) -> (i32, Option<DimensionDiagnostic>, Option<Token>) {
        let mut input = InputStack::new(MemoryInput::new(input_text));
        let scanned = scan_dimen(&mut input, stores).expect("dimension scan should succeed");
        let next = input
            .next_token(stores)
            .expect("remaining token should lex");
        (scanned.value().raw(), scanned.diagnostic(), next)
    }

    fn scan_coerced(input_text: &str) -> (i32, Option<DimensionDiagnostic>, Option<Token>) {
        let mut stores = Stores::new();
        let mut input = InputStack::new(MemoryInput::new(input_text));
        let scanned = scan_dimen_with_options(
            &mut input,
            &mut stores,
            ScanDimenOptions::with_integer_to_sp_coercion(),
        )
        .expect("dimension scan should succeed");
        let next = input
            .next_token(&mut stores)
            .expect("remaining token should lex");
        (scanned.value().raw(), scanned.diagnostic(), next)
    }

    fn char_token(ch: char, cat: Catcode) -> Token {
        Token::Char { ch, cat }
    }

    #[test]
    fn scans_fractional_decimal_constants_with_dot_and_comma() {
        assert_eq!(scan("1.5pt x").0, 98_304);
        assert_eq!(scan("1,25pt x").0, 81_920);
        assert_eq!(scan(".5pt x").0, 32_768);
        assert_eq!(scan("-.5pt x").0, -32_768);
    }

    #[test]
    fn scans_all_physical_units() {
        for (unit, text) in [
            (PhysicalUnit::Pt, "1pt x"),
            (PhysicalUnit::Pc, "1pc x"),
            (PhysicalUnit::In, "1in x"),
            (PhysicalUnit::Bp, "1bp x"),
            (PhysicalUnit::Cm, "1cm x"),
            (PhysicalUnit::Mm, "1mm x"),
            (PhysicalUnit::Dd, "1dd x"),
            (PhysicalUnit::Cc, "1cc x"),
            (PhysicalUnit::Sp, "1sp x"),
        ] {
            let expected = scaled_from_decimal_parts(1, 0, unit)
                .expect("unit conversion should fit")
                .raw();
            assert_eq!(scan(text).0, expected);
        }
    }

    #[test]
    fn scans_true_units_without_magnification_scaling_yet() {
        assert_eq!(scan("1truept x").0, 65_536);
        assert_eq!(scan("1 true in x").0, 4_736_286);
    }

    #[test]
    fn supports_integer_to_sp_coercion_when_requested() {
        let (value, diagnostic, next) = scan_coerced("123 x");

        assert_eq!(value, 123);
        assert_eq!(diagnostic, None);
        assert_eq!(next, Some(char_token('x', Catcode::Letter)));
    }

    #[test]
    fn rejects_bare_integer_without_coercion() {
        let mut stores = Stores::new();
        let mut input = InputStack::new(MemoryInput::new("123 x"));
        let err = scan_dimen(&mut input, &mut stores).expect_err("unit is required");

        assert!(matches!(err, ScanDimenError::MissingUnit));
    }

    #[test]
    fn scans_supported_internal_dimensions() {
        let mut stores = Stores::new();
        stores.intern("dimen");
        stores.set_dimen(3, Scaled::from_raw(42_000));

        let (value, diagnostic, next) = scan_with_stores("\\dimen3 x", &mut stores);

        assert_eq!(value, 42_000);
        assert_eq!(diagnostic, None);
        assert_eq!(next, Some(char_token('x', Catcode::Letter)));
    }

    #[test]
    fn scans_integer_like_internal_values_with_units() {
        let mut stores = Stores::new();
        stores.intern("count");
        stores.set_count(4, 2);

        assert_eq!(scan_with_stores("\\count4pt x", &mut stores).0, 131_072);
    }

    #[test]
    fn scans_hex_integer_constants_with_units() {
        assert_eq!(scan("\"7Fpt x").0, 127 * Scaled::UNITY);
    }

    #[test]
    fn restores_partially_matched_true_keyword_tokens() {
        let mut stores = Stores::new();
        let mut input = InputStack::new(MemoryInput::new("1truxpt"));
        let err = scan_dimen(&mut input, &mut stores).expect_err("bad true keyword lacks unit");

        assert!(matches!(err, ScanDimenError::MissingUnit));
        assert_eq!(
            input.next_token(&mut stores).expect("token should replay"),
            Some(char_token('t', Catcode::Letter))
        );
        assert_eq!(
            input.next_token(&mut stores).expect("token should replay"),
            Some(char_token('r', Catcode::Letter))
        );
        assert_eq!(
            input.next_token(&mut stores).expect("token should replay"),
            Some(char_token('u', Catcode::Letter))
        );
        assert_eq!(
            input.next_token(&mut stores).expect("token should replay"),
            Some(char_token('x', Catcode::Letter))
        );
    }

    #[test]
    fn reports_font_relative_units_as_clear_stubs() {
        let mut stores = Stores::new();
        let mut input = InputStack::new(MemoryInput::new("1em"));
        let err = scan_dimen(&mut input, &mut stores).expect_err("em is not implemented");
        assert!(matches!(
            err,
            ScanDimenError::UnsupportedFontRelativeUnit("em")
        ));

        let mut stores = Stores::new();
        let mut input = InputStack::new(MemoryInput::new("1ex"));
        let err = scan_dimen(&mut input, &mut stores).expect_err("ex is not implemented");
        assert!(matches!(
            err,
            ScanDimenError::UnsupportedFontRelativeUnit("ex")
        ));
    }

    #[test]
    fn reports_dimension_too_large_and_caps_value() {
        let (value, diagnostic, _next) = scan("16384pt x");

        assert_eq!(value, Scaled::MAX_DIMEN.raw());
        assert_eq!(diagnostic, Some(DimensionDiagnostic::TooLarge));
        assert_eq!(
            diagnostic.expect("overflow diagnostic").to_string(),
            "Dimension too large"
        );
    }

    #[test]
    fn scans_values_through_macro_expansion() {
        let mut stores = Stores::new();
        let number = stores.intern("number");
        let replacement = stores.intern_token_list(&[
            char_token('1', Catcode::Other),
            char_token('.', Catcode::Other),
            char_token('5', Catcode::Other),
            char_token('p', Catcode::Letter),
            char_token('t', Catcode::Letter),
        ]);
        let params = stores.intern_token_list(&[]);
        stores.set_macro_meaning(
            number,
            MacroMeaning::new(MeaningFlags::EMPTY, params, replacement),
        );

        assert_eq!(scan_with_stores("\\number x", &mut stores).0, 98_304);
    }

    #[test]
    fn fractional_sp_truncates_to_integer_scaled_points() {
        let expected = scaled_from_decimal_parts(1, round_decimal_fraction(&[5]), PhysicalUnit::Sp)
            .expect("fractional sp conversion fits")
            .raw();

        assert_eq!(scan("1.5sp x").0, expected);
    }
}
