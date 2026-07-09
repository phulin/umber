//! Expanded dimension scanning shared by conditionals and future stomach code.

use std::fmt;

use tex_lex::{InputSource, InputStack, LexError, TokenListReplayKind};
use tex_state::BoxDimension;
use tex_state::glue::Order;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::provenance::InsertedOriginKind;
use tex_state::scaled::{
    DimensionError, PhysicalUnit, Scaled, nx_plus_y, round_decimal_fraction,
    scaled_from_decimal_parts, xn_over_d,
};
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::{ExpansionState, PrepareMagDiagnostic};

use crate::{
    ExpandError, ExpandNext, ExpansionHooks, NoInputExpandNext, NoopExpansionHooks, NoopRecorder,
    ReadRecorder, scan_helpers, scan_int, semantic_token,
};
use scan_helpers::ExpandedKeywordMatch;

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

    #[must_use]
    pub const fn requiring_mu_unit(mut self) -> Self {
        self.require_mu_unit = true;
        self
    }
}

/// A successfully scanned TeX dimension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedDimen {
    value: Scaled,
    order: Order,
    diagnostics: [Option<DimensionDiagnostic>; 4],
}

impl ScannedDimen {
    #[must_use]
    pub const fn new(value: Scaled) -> Self {
        Self {
            value,
            order: Order::Normal,
            diagnostics: [None; 4],
        }
    }

    pub const fn with_diagnostic(value: Scaled, diagnostic: DimensionDiagnostic) -> Self {
        Self {
            value,
            order: Order::Normal,
            diagnostics: [Some(diagnostic), None, None, None],
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
        self.diagnostics[0]
    }

    pub fn diagnostics(self) -> impl Iterator<Item = DimensionDiagnostic> {
        self.diagnostics.into_iter().flatten()
    }

    fn with_added_diagnostic(mut self, diagnostic: DimensionDiagnostic) -> Self {
        if let Some(slot) = self.diagnostics.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(diagnostic);
        }
        self
    }

    fn with_leading_diagnostic(mut self, diagnostic: DimensionDiagnostic) -> Self {
        self.diagnostics.rotate_right(1);
        self.diagnostics[0] = Some(diagnostic);
        self
    }

    fn with_integer_diagnostic(self, diagnostic: Option<scan_int::IntegerDiagnostic>) -> Self {
        match diagnostic {
            Some(scan_int::IntegerDiagnostic::MissingNumber) => {
                self.with_leading_diagnostic(DimensionDiagnostic::MissingNumber)
            }
            Some(scan_int::IntegerDiagnostic::NumberTooBig) => {
                self.with_added_diagnostic(DimensionDiagnostic::TooLarge)
            }
            None => self,
        }
    }
}

/// Recoverable diagnostics emitted while still producing TeX's capped value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DimensionDiagnostic {
    MissingNumber,
    IllegalUnit { inserted: InsertedUnit },
    TooLarge,
    IllegalMagnification { attempted: i32 },
    IncompatibleMagnification { attempted: i32, retained: i32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertedUnit {
    Pt,
    Mu,
}

impl fmt::Display for DimensionDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingNumber => f.write_str("Missing number, treated as zero"),
            Self::IllegalUnit {
                inserted: InsertedUnit::Pt,
            } => f.write_str("Illegal unit of measure (pt inserted)"),
            Self::IllegalUnit {
                inserted: InsertedUnit::Mu,
            } => f.write_str("Illegal unit of measure (mu inserted)"),
            Self::TooLarge => f.write_str("Dimension too large"),
            Self::IllegalMagnification { .. } => {
                f.write_str("Illegal magnification has been changed to 1000")
            }
            Self::IncompatibleMagnification { attempted, .. } => write!(
                f,
                "Incompatible magnification ({attempted}); the previous value will be retained"
            ),
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

impl From<PrepareMagDiagnostic> for DimensionDiagnostic {
    fn from(value: PrepareMagDiagnostic) -> Self {
        match value {
            PrepareMagDiagnostic::IllegalMagnification { attempted } => {
                Self::IllegalMagnification { attempted }
            }
            PrepareMagDiagnostic::IncompatibleMagnification {
                attempted,
                retained,
            } => Self::IncompatibleMagnification {
                attempted,
                retained,
            },
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
    stores: &mut impl ExpansionState,
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
    stores: &mut impl ExpansionState,
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
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    scan_dimen_with_expander_and_hooks(
        input,
        stores,
        recorder,
        hooks,
        &mut NoInputExpandNext,
        options,
    )
}

/// Scans a TeX `<dimen>` using a caller-supplied recursive expansion capability.
pub fn scan_dimen_with_expander_and_hooks<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let (negative, token) = scan_signs(input, stores, recorder, hooks, expander)?;
    let Some(token) = token else {
        return Ok(ScannedDimen::with_diagnostic(
            Scaled::from_raw(0),
            DimensionDiagnostic::MissingNumber,
        ));
    };

    let scanned =
        scan_unsigned_after_first_token(input, stores, recorder, hooks, expander, token, options)?;
    consume_optional_space(input, stores, recorder, hooks, expander)?;
    Ok(apply_sign(scanned, negative))
}

fn scan_signs<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<(bool, Option<TracedTokenWord>), ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
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
) -> Result<Option<TracedTokenWord>, ScanDimenError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    Ok(expander.next_expanded_token(input, stores, recorder, hooks)?)
}

fn scan_unsigned_after_first_token<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    token: TracedTokenWord,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    match semantic_token(token) {
        Token::Char {
            ch,
            cat: Catcode::Other,
        } if ch.is_ascii_digit() => {
            let integer = scan_decimal_integer(
                input,
                stores,
                recorder,
                hooks,
                expander,
                digit_value(ch).expect("digit"),
            )?;
            scan_decimal_tail(input, stores, recorder, hooks, expander, integer, options)
        }
        Token::Char {
            ch: '.' | ',',
            cat: Catcode::Other,
        } => scan_fraction_and_unit(input, stores, recorder, hooks, expander, 0, options),
        Token::Char {
            ch: '\'' | '"' | '`',
            cat: Catcode::Other,
        } => scan_integer_constant_with_unit(
            input, stores, recorder, hooks, expander, token, options,
        ),
        Token::Cs(symbol) => scan_internal_or_numeric_dimension(
            input, stores, recorder, hooks, expander, token, symbol, options,
        ),
        _ => {
            unread_token(input, stores, token);
            let recovered = recover_missing_unit(0, 0, options, stores)?;
            Ok(recovered.with_leading_diagnostic(DimensionDiagnostic::MissingNumber))
        }
    }
}

fn scan_decimal_integer<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    first_digit: i32,
) -> Result<i32, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    let mut value = first_digit;
    loop {
        let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
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

fn scan_decimal_tail<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    integer: i32,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
        return coerce_or_recover_missing_unit(integer, options, stores);
    };

    if is_decimal_point(token) {
        return scan_fraction_and_unit(input, stores, recorder, hooks, expander, integer, options);
    }

    unread_token(input, stores, token);
    match scan_unit(input, stores, recorder, hooks, expander, options)? {
        Some(unit) => convert_scanned_unit(stores, integer, 0, unit),
        None if options.coerce_integer_to_sp => {
            convert_decimal(integer, 0, PhysicalUnit::Sp, false, stores.mag())
        }
        None => recover_missing_unit(integer, 0, options, stores),
    }
}

fn scan_fraction_and_unit<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    integer: i32,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    let fraction = scan_fraction(input, stores, recorder, hooks, expander)?;
    let Some(unit) = scan_unit(input, stores, recorder, hooks, expander, options)? else {
        return recover_missing_unit(integer, fraction, options, stores);
    };
    convert_scanned_unit(stores, integer, fraction, unit)
}

fn scan_fraction<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<i32, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    let mut digits = Vec::new();
    loop {
        let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
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

#[allow(clippy::too_many_arguments)]
fn scan_internal_or_numeric_dimension<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    token: TracedTokenWord,
    symbol: Symbol,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    match stores.meaning(symbol) {
        Meaning::DimenRegister(index) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            return Ok(ScannedDimen::new(stores.dimen(index)));
        }
        Meaning::DimenParam(index) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            return Ok(ScannedDimen::new(
                stores.dimen_param(tex_state::env::banks::DimenParam::new(index)),
            ));
        }
        Meaning::PageDimension(dimension) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            return Ok(ScannedDimen::new(stores.page_dimension(dimension)));
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            let index = scan_register_index(input, stores, recorder, hooks, expander)?;
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            return Ok(ScannedDimen::new(stores.dimen(index)));
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Wd
            | UnexpandablePrimitive::Ht
            | UnexpandablePrimitive::Dp),
        ) => {
            let index = scan_register_index(input, stores, recorder, hooks, expander)?;
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            let dimension = match primitive {
                UnexpandablePrimitive::Wd => BoxDimension::Width,
                UnexpandablePrimitive::Ht => BoxDimension::Height,
                UnexpandablePrimitive::Dp => BoxDimension::Depth,
                _ => unreachable!("outer match restricts primitive"),
            };
            return Ok(ScannedDimen::new(
                stores
                    .box_dimension(index, dimension)
                    .unwrap_or_else(|| Scaled::from_raw(0)),
            ));
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevDepth) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            return Ok(ScannedDimen::new(hooks.prev_depth()));
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastKern) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            return Ok(ScannedDimen::new(hooks.last_kern()));
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastSkip) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            return Ok(ScannedDimen::new(hooks.last_skip().width));
        }
        _ => {}
    }

    if stores.resolve(symbol) == "dimen" {
        let index = scan_register_index(input, stores, recorder, hooks, expander)?;
        consume_optional_space(input, stores, recorder, hooks, expander)?;
        return Ok(ScannedDimen::new(stores.dimen(index)));
    }

    unread_token(input, stores, token);
    let scanned =
        scan_int::scan_int_with_expander_and_hooks(input, stores, recorder, hooks, expander)?;
    if scanned.diagnostic() == Some(scan_int::IntegerDiagnostic::NumberTooBig) {
        return Ok(ScannedDimen::with_diagnostic(
            Scaled::MAX_DIMEN,
            DimensionDiagnostic::TooLarge,
        ));
    }

    let integer = scanned.value();
    let Some(unit) = scan_unit(input, stores, recorder, hooks, expander, options)? else {
        if options.coerce_integer_to_sp {
            return convert_decimal(integer, 0, PhysicalUnit::Sp, false, stores.mag())
                .map(|dimen| dimen.with_integer_diagnostic(scanned.diagnostic()));
        }
        return recover_missing_unit(integer, 0, options, stores)
            .map(|dimen| dimen.with_integer_diagnostic(scanned.diagnostic()));
    };
    convert_scanned_unit(stores, integer, 0, unit)
        .map(|dimen| dimen.with_integer_diagnostic(scanned.diagnostic()))
}

fn scan_integer_constant_with_unit<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    token: TracedTokenWord,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    unread_token(input, stores, token);
    let scanned =
        scan_int::scan_int_with_expander_and_hooks(input, stores, recorder, hooks, expander)?;
    if scanned.diagnostic() == Some(scan_int::IntegerDiagnostic::NumberTooBig) {
        return Ok(ScannedDimen::with_diagnostic(
            Scaled::MAX_DIMEN,
            DimensionDiagnostic::TooLarge,
        ));
    }
    let Some(unit) = scan_unit(input, stores, recorder, hooks, expander, options)? else {
        if options.coerce_integer_to_sp {
            return convert_decimal(scanned.value(), 0, PhysicalUnit::Sp, false, stores.mag())
                .map(|dimen| dimen.with_integer_diagnostic(scanned.diagnostic()));
        }
        return recover_missing_unit(scanned.value(), 0, options, stores)
            .map(|dimen| dimen.with_integer_diagnostic(scanned.diagnostic()));
    };
    convert_scanned_unit(stores, scanned.value(), 0, unit)
        .map(|dimen| dimen.with_integer_diagnostic(scanned.diagnostic()))
}

fn scan_register_index<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<u16, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    let value =
        scan_int::scan_int_with_expander_and_hooks(input, stores, recorder, hooks, expander)?
            .value();
    if !(0..=MAX_REGISTER).contains(&value) {
        return Err(ScanDimenError::RegisterNumberOutOfRange(value));
    }
    Ok(value as u16)
}

fn scan_unit<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    options: ScanDimenOptions,
) -> Result<Option<ScannedUnit>, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    skip_spaces(input, stores, recorder, hooks, expander)?;
    let first = match next_x(input, stores, recorder, hooks, expander)? {
        Some(token) => token,
        None => return Ok(None),
    };

    if options.allow_infinite_units {
        match keyword_matches(input, stores, recorder, hooks, expander, first, "fil")? {
            ExpandedKeywordMatch::Matched => {
                let mut order = Order::Fil;
                while keyword(input, stores, recorder, hooks, expander, "l")? {
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
            ExpandedKeywordMatch::PartialMismatch => return Ok(None),
            ExpandedKeywordMatch::FirstTokenMismatch => {}
        }
    }

    if options.require_mu_unit {
        match keyword_matches(input, stores, recorder, hooks, expander, first, "mu")? {
            ExpandedKeywordMatch::Matched => return Ok(Some(physical_unit(PhysicalUnit::Pt))),
            ExpandedKeywordMatch::PartialMismatch => return Ok(None),
            ExpandedKeywordMatch::FirstTokenMismatch => {}
        }
        unread_token(input, stores, first);
        return Ok(None);
    }

    match keyword_matches(input, stores, recorder, hooks, expander, first, "true")? {
        ExpandedKeywordMatch::Matched => {
            skip_spaces(input, stores, recorder, hooks, expander)?;
            let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
                return Ok(None);
            };
            return scan_unit_keyword(input, stores, recorder, hooks, expander, token, true);
        }
        ExpandedKeywordMatch::PartialMismatch => return Ok(None),
        ExpandedKeywordMatch::FirstTokenMismatch => {}
    }

    scan_unit_keyword(input, stores, recorder, hooks, expander, first, false)
}

fn scan_unit_keyword<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    first: TracedTokenWord,
    true_unit: bool,
) -> Result<Option<ScannedUnit>, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    let Some(second) = next_x(input, stores, recorder, hooks, expander)? else {
        unread_token(input, stores, first);
        return Ok(None);
    };

    match unit_from_tokens(semantic_token(first), semantic_token(second)) {
        Some(ScannedUnit::Physical { unit, .. }) => {
            Ok(Some(ScannedUnit::Physical { unit, true_unit }))
        }
        Some(ScannedUnit::Em) => Ok(Some(ScannedUnit::FontRelative {
            unit: stores.font_parameter(stores.current_font(), 6),
        })),
        Some(ScannedUnit::Ex) => Ok(Some(ScannedUnit::FontRelative {
            unit: stores.font_parameter(stores.current_font(), 5),
        })),
        Some(ScannedUnit::Infinite(_)) => unreachable!("unit keywords never return infinity"),
        Some(ScannedUnit::FontRelative { .. }) => {
            unreachable!("unit keywords never return resolved font-relative units")
        }
        None => {
            unread_tokens(input, stores, [first, second]);
            Ok(None)
        }
    }
}

fn keyword_matches<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    first: TracedTokenWord,
    keyword: &str,
) -> Result<ExpandedKeywordMatch, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    Ok(
        scan_helpers::scan_keyword_after_first_with_expander_and_hooks(
            input, stores, recorder, hooks, expander, first, keyword,
        )?,
    )
}

fn keyword<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    keyword: &str,
) -> Result<bool, ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    skip_spaces(input, stores, recorder, hooks, expander)?;
    let Some(first) = next_x(input, stores, recorder, hooks, expander)? else {
        return Ok(false);
    };
    match scan_helpers::scan_keyword_after_first_with_expander_and_hooks(
        input, stores, recorder, hooks, expander, first, keyword,
    )? {
        ExpandedKeywordMatch::Matched => Ok(true),
        ExpandedKeywordMatch::FirstTokenMismatch => {
            unread_token(input, stores, first);
            Ok(false)
        }
        ExpandedKeywordMatch::PartialMismatch => Ok(false),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScannedUnit {
    Physical { unit: PhysicalUnit, true_unit: bool },
    Infinite(Order),
    FontRelative { unit: Scaled },
    Em,
    Ex,
}

fn unit_from_tokens(first: Token, second: Token) -> Option<ScannedUnit> {
    let first = keyword_char(first)?;
    let second = keyword_char(second)?;
    match (first, second) {
        ('p', 't') => Some(physical_unit(PhysicalUnit::Pt)),
        ('p', 'c') => Some(physical_unit(PhysicalUnit::Pc)),
        ('i', 'n') => Some(physical_unit(PhysicalUnit::In)),
        ('b', 'p') => Some(physical_unit(PhysicalUnit::Bp)),
        ('c', 'm') => Some(physical_unit(PhysicalUnit::Cm)),
        ('m', 'm') => Some(physical_unit(PhysicalUnit::Mm)),
        ('d', 'd') => Some(physical_unit(PhysicalUnit::Dd)),
        ('c', 'c') => Some(physical_unit(PhysicalUnit::Cc)),
        ('s', 'p') => Some(physical_unit(PhysicalUnit::Sp)),
        ('e', 'm') => Some(ScannedUnit::Em),
        ('e', 'x') => Some(ScannedUnit::Ex),
        _ => None,
    }
}

fn physical_unit(unit: PhysicalUnit) -> ScannedUnit {
    ScannedUnit::Physical {
        unit,
        true_unit: false,
    }
}

fn convert_decimal(
    integer: i32,
    fraction: i32,
    unit: PhysicalUnit,
    true_unit: bool,
    mag: i32,
) -> Result<ScannedDimen, ScanDimenError> {
    let negative = integer < 0;
    let magnitude = if negative {
        integer.checked_neg().unwrap_or(Scaled::MAX_DIMEN.raw() + 1)
    } else {
        integer
    };
    let (magnitude, fraction) = if true_unit {
        match true_scaled_decimal_parts(magnitude, fraction, mag) {
            Ok(parts) => parts,
            Err(error) => {
                return Ok(ScannedDimen::with_diagnostic(
                    Scaled::MAX_DIMEN,
                    DimensionDiagnostic::from(error),
                ));
            }
        }
    } else {
        (magnitude, fraction)
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
    stores: &mut impl ExpansionState,
    integer: i32,
    fraction: i32,
    unit: ScannedUnit,
) -> Result<ScannedDimen, ScanDimenError> {
    match unit {
        ScannedUnit::Physical { unit, true_unit } => {
            convert_physical_unit(stores, integer, fraction, unit, true_unit)
        }
        ScannedUnit::Infinite(order) => {
            let mut scanned =
                convert_decimal(integer, fraction, PhysicalUnit::Pt, false, stores.mag())?;
            scanned.order = order;
            Ok(scanned)
        }
        ScannedUnit::FontRelative { unit } => convert_font_relative_unit(integer, fraction, unit),
        ScannedUnit::Em | ScannedUnit::Ex => unreachable!("font units are handled while scanning"),
    }
}

fn convert_font_relative_unit(
    integer: i32,
    fraction: i32,
    unit: Scaled,
) -> Result<ScannedDimen, ScanDimenError> {
    assert!(integer >= 0, "dimension integer part must be nonnegative");
    assert!(
        (0..=Scaled::UNITY).contains(&fraction),
        "dimension fraction out of range"
    );

    let fractional = match xn_over_d(unit, fraction, Scaled::UNITY) {
        Ok(value) => value.quotient,
        Err(error) => {
            return Ok(ScannedDimen::with_diagnostic(
                Scaled::MAX_DIMEN,
                DimensionDiagnostic::from(error),
            ));
        }
    };
    match nx_plus_y(integer, unit, fractional).and_then(|value| {
        value
            .check_dimension()
            .map_err(|_| tex_state::scaled::ArithmeticError::Overflow)
    }) {
        Ok(value) => Ok(ScannedDimen::new(value)),
        Err(_) => Ok(ScannedDimen::with_diagnostic(
            Scaled::MAX_DIMEN,
            DimensionDiagnostic::TooLarge,
        )),
    }
}

fn convert_physical_unit(
    stores: &mut impl ExpansionState,
    integer: i32,
    fraction: i32,
    unit: PhysicalUnit,
    true_unit: bool,
) -> Result<ScannedDimen, ScanDimenError> {
    let (mag, mag_diagnostic) = if true_unit {
        stores.prepare_mag()
    } else {
        (stores.mag(), None)
    };
    let mut scanned = convert_decimal(integer, fraction, unit, true_unit, mag)?;
    if let Some(diagnostic) = mag_diagnostic {
        scanned = scanned.with_added_diagnostic(DimensionDiagnostic::from(diagnostic));
    }
    Ok(scanned)
}

fn true_scaled_decimal_parts(
    integer: i32,
    fraction: i32,
    mag: i32,
) -> Result<(i32, i32), DimensionError> {
    if mag == 1000 {
        return Ok((integer, fraction));
    }

    let converted = xn_over_d(Scaled::from_raw(integer), 1000, mag)?;
    let mut fraction = (1000 * fraction + Scaled::UNITY * converted.remainder) / mag;
    let integer = converted.quotient.raw() + fraction / Scaled::UNITY;
    fraction %= Scaled::UNITY;
    Ok((integer, fraction))
}

fn coerce_or_recover_missing_unit(
    integer: i32,
    options: ScanDimenOptions,
    stores: &impl ExpansionState,
) -> Result<ScannedDimen, ScanDimenError> {
    if options.coerce_integer_to_sp {
        convert_decimal(integer, 0, PhysicalUnit::Sp, false, stores.mag())
    } else {
        recover_missing_unit(integer, 0, options, stores)
    }
}

fn recover_missing_unit(
    integer: i32,
    fraction: i32,
    options: ScanDimenOptions,
    stores: &impl ExpansionState,
) -> Result<ScannedDimen, ScanDimenError> {
    recover_missing_unit_with_mag(integer, fraction, options, stores.mag())
}

fn recover_missing_unit_with_mag(
    integer: i32,
    fraction: i32,
    options: ScanDimenOptions,
    mag: i32,
) -> Result<ScannedDimen, ScanDimenError> {
    let inserted = if options.require_mu_unit {
        InsertedUnit::Mu
    } else {
        InsertedUnit::Pt
    };
    convert_decimal(integer, fraction, PhysicalUnit::Pt, false, mag)
        .map(|dimen| dimen.with_added_diagnostic(DimensionDiagnostic::IllegalUnit { inserted }))
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
        diagnostics: scanned.diagnostics,
    }
}

fn consume_optional_space<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<(), ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
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

fn skip_spaces<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<(), ScanDimenError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    St: ExpansionState,
    E: ExpandNext<S, St, R, H>,
{
    loop {
        let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
            return Ok(());
        };
        if !is_space(token) {
            unread_token(input, stores, token);
            return Ok(());
        }
    }
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
        origins.push(stores.inserted_origin(
            InsertedOriginKind::Unread,
            semantic_token(token),
            token.origin(),
        ));
    }
    let origin_list = stores.finish_origin_list(&mut origins);
    input.push_token_list_with_origins(token_list, origin_list, TokenListReplayKind::Inserted);
}

fn decimal_digit(token: TracedTokenWord) -> Option<i32> {
    let Token::Char {
        ch,
        cat: Catcode::Other,
    } = semantic_token(token)
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

fn is_space(token: TracedTokenWord) -> bool {
    matches!(
        semantic_token(token),
        Token::Char {
            cat: Catcode::Space,
            ..
        }
    )
}

fn is_decimal_point(token: TracedTokenWord) -> bool {
    matches!(
        semantic_token(token),
        Token::Char {
            ch: '.' | ',',
            cat: Catcode::Other
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
mod tests;
