//! Expanded dimension scanning shared by conditionals and future stomach code.

use std::fmt;

use tex_lex::{InputStack, LexError};
use tex_state::BoxDimension;
use tex_state::env::banks::GlueParam;
use tex_state::glue::Order;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::{
    DimensionError, PhysicalUnit, Scaled, nx_plus_y, round_decimal_fraction,
    scale_true_dimension_parts, scaled_from_decimal_parts, xn_over_d,
};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{ExpansionState, PrepareMagDiagnostic};

use crate::{
    ExpandError, ExpansionContext, ExpansionMode, ReadBank, ReadDependency, ReadFontField,
    RestrictedExpansionMode, scan_helpers, scan_int, semantic_token,
};
use scan_helpers::ExpandedKeywordMatch;

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
    diagnostic_origins: [Option<OriginId>; 4],
}

impl ScannedDimen {
    #[must_use]
    pub const fn new(value: Scaled) -> Self {
        Self {
            value,
            order: Order::Normal,
            diagnostics: [None; 4],
            diagnostic_origins: [None; 4],
        }
    }

    pub const fn with_diagnostic(
        value: Scaled,
        diagnostic: DimensionDiagnostic,
        origin: OriginId,
    ) -> Self {
        Self {
            value,
            order: Order::Normal,
            diagnostics: [Some(diagnostic), None, None, None],
            diagnostic_origins: [Some(origin), None, None, None],
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

    pub fn diagnostic_origins(self) -> impl Iterator<Item = OriginId> {
        self.diagnostic_origins.into_iter().flatten()
    }

    pub fn diagnostic_records(self) -> impl Iterator<Item = (DimensionDiagnostic, OriginId)> {
        self.diagnostics
            .into_iter()
            .zip(self.diagnostic_origins)
            .filter_map(|(diagnostic, origin)| Some((diagnostic?, origin?)))
    }

    fn with_added_diagnostic(mut self, diagnostic: DimensionDiagnostic, origin: OriginId) -> Self {
        if let Some(index) = self.diagnostics.iter().position(Option::is_none) {
            self.diagnostics[index] = Some(diagnostic);
            self.diagnostic_origins[index] = Some(origin);
        }
        self
    }

    fn with_leading_diagnostic(
        mut self,
        diagnostic: DimensionDiagnostic,
        origin: OriginId,
    ) -> Self {
        self.diagnostics.rotate_right(1);
        self.diagnostic_origins.rotate_right(1);
        self.diagnostics[0] = Some(diagnostic);
        self.diagnostic_origins[0] = Some(origin);
        self
    }

    fn with_integer_diagnostic(
        self,
        diagnostic: Option<scan_int::IntegerDiagnostic>,
        origin: Option<OriginId>,
    ) -> Self {
        match diagnostic {
            Some(scan_int::IntegerDiagnostic::MissingNumber) => self.with_leading_diagnostic(
                DimensionDiagnostic::MissingNumber,
                origin.unwrap_or(OriginId::UNKNOWN),
            ),
            Some(scan_int::IntegerDiagnostic::NumberTooBig) => self.with_added_diagnostic(
                DimensionDiagnostic::TooLarge,
                origin.unwrap_or(OriginId::UNKNOWN),
            ),
            None => self,
        }
    }
}

/// Recoverable diagnostics emitted while still producing TeX's capped value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DimensionDiagnostic {
    MissingNumber,
    IllegalUnit { inserted: InsertedUnit },
    IncompatibleGlueUnits,
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
            Self::IncompatibleGlueUnits => f.write_str("Incompatible glue units"),
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
    MissingNumber {
        context: TracedTokenWord,
    },
    MissingUnit {
        context: TracedTokenWord,
    },
    RegisterNumberOutOfRange {
        value: i32,
        context: TracedTokenWord,
    },
    IncompatibleGlueUnits {
        context: TracedTokenWord,
    },
    UnsupportedInternalDimension {
        context: TracedTokenWord,
    },
}

impl fmt::Display for ScanDimenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expand(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::Integer(err) => write!(f, "{err}"),
            Self::MissingNumber { .. } => f.write_str("Missing number"),
            Self::MissingUnit { .. } => f.write_str("Illegal unit of measure"),
            Self::RegisterNumberOutOfRange { value, .. } => {
                write!(f, "register number {value} is out of range")
            }
            Self::IncompatibleGlueUnits { .. } => f.write_str("Incompatible glue units"),
            Self::UnsupportedInternalDimension { context } => {
                write!(
                    f,
                    "unsupported internal dimension token {:?}",
                    semantic_token(*context)
                )
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
            Self::MissingNumber { .. }
            | Self::MissingUnit { .. }
            | Self::RegisterNumberOutOfRange { .. }
            | Self::IncompatibleGlueUnits { .. }
            | Self::UnsupportedInternalDimension { .. } => None,
        }
    }
}

impl ScanDimenError {
    #[must_use]
    pub fn primary_origin(&self) -> Option<OriginId> {
        match self {
            Self::MissingNumber { context } | Self::MissingUnit { context } => {
                Some(context.origin())
            }
            Self::RegisterNumberOutOfRange { context, .. }
            | Self::IncompatibleGlueUnits { context } => Some(context.origin()),
            Self::UnsupportedInternalDimension { context } => Some(context.origin()),
            Self::Integer(err) => err.primary_origin(),
            Self::Expand(err) => err.primary_origin(),
            Self::Lex(_) => None,
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
pub fn scan_dimen(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<ScannedDimen, ScanDimenError> {
    scan_dimen_with_options_and_context(
        input,
        stores,
        &mut ExpansionContext::new("texput"),
        ScanDimenOptions::STANDARD,
        context,
    )
}

/// Scans a TeX `<dimen>` using expanded tokens and caller-specific options.
pub fn scan_dimen_with_options(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    options: ScanDimenOptions,
    context: TracedTokenWord,
) -> Result<ScannedDimen, ScanDimenError> {
    scan_dimen_with_options_and_context(
        input,
        stores,
        &mut ExpansionContext::new("texput"),
        options,
        context,
    )
}

/// Scans a TeX `<dimen>` while preserving caller-supplied expansion context.
pub fn scan_dimen_with_options_and_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    options: ScanDimenOptions,
    context: TracedTokenWord,
) -> Result<ScannedDimen, ScanDimenError> {
    scan_dimen_with_mode_and_context(
        input,
        stores,
        expansion,
        &mut RestrictedExpansionMode,
        options,
        context,
    )
}

/// Scans a TeX `<dimen>` using a caller-supplied recursive expansion capability.
pub fn scan_dimen_with_mode_and_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    options: ScanDimenOptions,
    context: TracedTokenWord,
) -> Result<ScannedDimen, ScanDimenError>
where
{
    // Physical, true, and recovery-unit paths all consult the prepared job
    // magnification. Recording the key once per dimension scan keeps the
    // concrete read set complete without instrumenting arithmetic helpers.
    crate::record_dependency!(
        expansion,
        ReadDependency::Cell {
            bank: ReadBank::Magnification,
            index: 0,
        }
    );
    let (negative, token) = scan_signs(input, stores, expansion, mode)?;
    let Some(token) = token else {
        return Ok(ScannedDimen::with_diagnostic(
            Scaled::from_raw(0),
            DimensionDiagnostic::MissingNumber,
            context.origin(),
        ));
    };

    let mut consume_trailing_space = true;
    let scanned = scan_unsigned_after_first_token(
        input,
        stores,
        expansion,
        mode,
        token,
        options,
        &mut consume_trailing_space,
    )?;
    if consume_trailing_space {
        consume_optional_space(input, stores, expansion, mode)?;
    }
    Ok(apply_sign(scanned, negative))
}

fn scan_signs(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
) -> Result<(bool, Option<TracedTokenWord>), ScanDimenError>
where
{
    let mut negative = false;
    loop {
        let Some(token) = next_x(input, stores, expansion, mode)? else {
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

fn next_x(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
) -> Result<Option<TracedTokenWord>, ScanDimenError>
where
{
    Ok(mode.next_expanded_token(input, stores, expansion)?)
}

pub(crate) fn scan_dim_expr(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<ScannedDimen, ScanDimenError>
where
{
    let (value, bad) = parse_dim_expr(input, stores, expansion, mode, false)?;
    if bad || value.abs() > i64::from(Scaled::MAX_DIMEN.raw()) {
        Ok(ScannedDimen::with_diagnostic(
            Scaled::from_raw(0),
            DimensionDiagnostic::TooLarge,
            context.origin(),
        ))
    } else {
        Ok(ScannedDimen::new(Scaled::from_raw(value as i32)))
    }
}

fn parse_dim_expr(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    parenthesized: bool,
) -> Result<(i64, bool), ScanDimenError>
where
{
    let (mut value, mut bad) = parse_dim_term(input, stores, expansion, mode)?;
    loop {
        let Some(token) = expr_next(input, stores, expansion, mode)? else {
            break;
        };
        let subtract = if expr_is(token, '+') {
            false
        } else if expr_is(token, '-') {
            true
        } else {
            if parenthesized && expr_is(token, ')') {
                break;
            }
            if !matches!(semantic_token(token), Token::Cs(s) if stores.meaning(s) == Meaning::Relax)
            {
                unread_token(input, stores, token);
            }
            break;
        };
        let (rhs, rhs_bad) = parse_dim_term(input, stores, expansion, mode)?;
        bad |= rhs_bad;
        let result = if subtract {
            value.checked_sub(rhs)
        } else {
            value.checked_add(rhs)
        };
        match result {
            Some(next) if next.abs() <= i64::from(Scaled::MAX_DIMEN.raw()) => value = next,
            _ => {
                value = 0;
                bad = true;
            }
        }
    }
    Ok((value, bad))
}

fn parse_dim_term(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
) -> Result<(i64, bool), ScanDimenError>
where
{
    let (mut value, mut bad) = parse_dim_factor(input, stores, expansion, mode)?;
    loop {
        let Some(operator) = expr_next(input, stores, expansion, mode)? else {
            break;
        };
        if expr_is(operator, '*') {
            let (numerator, numerator_bad) = parse_expr_int(input, stores, expansion, mode)?;
            bad |= numerator_bad;
            let following = expr_next(input, stores, expansion, mode)?;
            if following.is_some_and(|token| expr_is(token, '/')) {
                let (denominator, denominator_bad) =
                    parse_expr_int(input, stores, expansion, mode)?;
                bad |= denominator_bad;
                match scan_int::rounded_fraction(value, numerator, denominator) {
                    Some(next) if next.abs() <= i64::from(Scaled::MAX_DIMEN.raw()) => value = next,
                    _ => {
                        value = 0;
                        bad = true;
                    }
                }
            } else {
                if let Some(token) = following {
                    unread_token(input, stores, token);
                }
                match value.checked_mul(numerator) {
                    Some(next) if next.abs() <= i64::from(Scaled::MAX_DIMEN.raw()) => value = next,
                    _ => {
                        value = 0;
                        bad = true;
                    }
                }
            }
        } else if expr_is(operator, '/') {
            let (denominator, denominator_bad) = parse_expr_int(input, stores, expansion, mode)?;
            bad |= denominator_bad;
            match scan_int::rounded_quotient(value, denominator) {
                Some(next) => value = next,
                None => {
                    value = 0;
                    bad = true;
                }
            }
        } else {
            unread_token(input, stores, operator);
            break;
        }
    }
    Ok((value, bad))
}

fn parse_dim_factor(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
) -> Result<(i64, bool), ScanDimenError>
where
{
    let Some(token) = expr_next(input, stores, expansion, mode)? else {
        return Ok((0, true));
    };
    if expr_is(token, '(') {
        return parse_dim_expr(input, stores, expansion, mode, true);
    }
    unread_token(input, stores, token);
    let scanned = scan_dimen_with_mode_and_context(
        input,
        stores,
        expansion,
        mode,
        ScanDimenOptions::STANDARD,
        token,
    )?;
    Ok((
        i64::from(scanned.value().raw()),
        scanned.diagnostic().is_some(),
    ))
}

fn parse_expr_int(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
) -> Result<(i64, bool), ScanDimenError>
where
{
    let Some(token) = expr_next(input, stores, expansion, mode)? else {
        return Ok((0, true));
    };
    if expr_is(token, '(') {
        return Ok(scan_int::parse_num_expression(
            input, stores, expansion, mode, true,
        )?);
    }
    unread_token(input, stores, token);
    let scanned = scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, token)?;
    Ok((i64::from(scanned.value()), scanned.diagnostic().is_some()))
}

fn expr_next(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
) -> Result<Option<TracedTokenWord>, ScanDimenError>
where
{
    loop {
        let token = next_x(input, stores, expansion, mode)?;
        if token.is_none_or(|token| !is_space(token)) {
            return Ok(token);
        }
    }
}

fn expr_is(token: TracedTokenWord, wanted: char) -> bool {
    matches!(semantic_token(token), Token::Char { ch, cat: Catcode::Other } if ch == wanted)
}

fn scan_unsigned_after_first_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    token: TracedTokenWord,
    options: ScanDimenOptions,
    consume_trailing_space: &mut bool,
) -> Result<ScannedDimen, ScanDimenError>
where
{
    match semantic_token(token) {
        Token::Char {
            ch,
            cat: Catcode::Other,
        } if ch.is_ascii_digit() => {
            let integer = scan_decimal_integer(
                input,
                stores,
                expansion,
                mode,
                digit_value(ch).expect("digit"),
            )?;
            scan_decimal_tail(input, stores, expansion, mode, integer, options)
        }
        Token::Char {
            ch: '.' | ',',
            cat: Catcode::Other,
        } => scan_fraction_and_unit(input, stores, expansion, mode, 0, options),
        Token::Char {
            ch: '\'' | '"' | '`',
            cat: Catcode::Other,
        } => scan_integer_constant_with_unit(input, stores, expansion, mode, token, options),
        Token::Cs(symbol) => scan_internal_or_numeric_dimension(
            input,
            stores,
            expansion,
            mode,
            token,
            symbol,
            options,
            consume_trailing_space,
        ),
        _ => {
            unread_token(input, stores, token);
            let recovered = recover_missing_unit(0, 0, options, stores, token.origin())?;
            Ok(recovered
                .with_leading_diagnostic(DimensionDiagnostic::MissingNumber, token.origin()))
        }
    }
}

fn scan_decimal_integer(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    first_digit: i32,
) -> Result<i32, ScanDimenError>
where
{
    let mut value = first_digit;
    loop {
        let Some(token) = next_x(input, stores, expansion, mode)? else {
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

fn scan_decimal_tail(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    integer: i32,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
{
    let Some(token) = next_x(input, stores, expansion, mode)? else {
        let origin = input.current_input_origin(stores);
        return coerce_or_recover_missing_unit(integer, options, stores, origin);
    };

    if is_decimal_point(token) {
        return scan_fraction_and_unit(input, stores, expansion, mode, integer, options);
    }

    unread_token(input, stores, token);
    match scan_unit(input, stores, expansion, mode, options)? {
        UnitScan::Scanned(unit) => convert_scanned_unit(stores, integer, 0, unit),
        UnitScan::Rejected(_) if options.coerce_integer_to_sp => {
            convert_decimal(integer, 0, PhysicalUnit::Sp, false, stores.mag())
        }
        UnitScan::Rejected(origin) => recover_missing_unit(integer, 0, options, stores, origin),
    }
}

fn scan_fraction_and_unit(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    integer: i32,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
{
    let fraction = scan_fraction(input, stores, expansion, mode)?;
    match scan_unit(input, stores, expansion, mode, options)? {
        UnitScan::Scanned(unit) => convert_scanned_unit(stores, integer, fraction, unit),
        UnitScan::Rejected(origin) => {
            recover_missing_unit(integer, fraction, options, stores, origin)
        }
    }
}

fn scan_fraction(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
) -> Result<i32, ScanDimenError>
where
{
    let mut digits = Vec::new();
    loop {
        let Some(token) = next_x(input, stores, expansion, mode)? else {
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
fn scan_internal_or_numeric_dimension(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    token: TracedTokenWord,
    symbol: Symbol,
    options: ScanDimenOptions,
    consume_trailing_space: &mut bool,
) -> Result<ScannedDimen, ScanDimenError>
where
{
    // TeX's internal-dimension path attaches the sign and returns without
    // expanding one token of optional-space lookahead. Numeric dimensions do
    // perform that lookahead after scanning their unit.
    *consume_trailing_space = false;
    let meaning = stores.meaning(symbol);
    expansion.record_meaning(symbol, meaning);
    crate::values::record_meaning_value_dependency(expansion, meaning);
    match meaning {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DimExpr) => {
            return scan_dim_expr(input, stores, expansion, mode, token);
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::GlueStretch | UnexpandablePrimitive::GlueShrink),
        ) => {
            let scanned = crate::scan_glue::scan_glue_with_mode_and_context(
                input, stores, expansion, mode, false, token,
            )
            .map_err(|error| ScanDimenError::Expand(error.into()))?;
            let spec = stores.glue(scanned.id());
            return Ok(ScannedDimen::new(
                if primitive == UnexpandablePrimitive::GlueStretch {
                    spec.stretch
                } else {
                    spec.shrink
                },
            ));
        }
        Meaning::DimenRegister(index) => {
            return Ok(ScannedDimen::new(stores.dimen(index)));
        }
        Meaning::DimenParam(index) => {
            return Ok(ScannedDimen::new(
                stores.dimen_param(tex_state::env::banks::DimenParam::new(index)),
            ));
        }
        Meaning::SkipRegister(index) => {
            return Ok(ScannedDimen::new(stores.glue(stores.skip(index)).width));
        }
        Meaning::GlueParam(index) => {
            let glue = stores.glue_param(GlueParam::new(index));
            return Ok(ScannedDimen::new(stores.glue(glue).width));
        }
        Meaning::MuskipRegister(index) => {
            let width = stores.glue(stores.muskip(index)).width;
            return Ok(if options.require_mu_unit {
                ScannedDimen::new(width)
            } else {
                ScannedDimen::with_diagnostic(
                    width,
                    DimensionDiagnostic::IncompatibleGlueUnits,
                    token.origin(),
                )
            });
        }
        Meaning::MuGlueParam(index) => {
            let glue = stores.glue_param(GlueParam::new(index));
            let width = stores.glue(glue).width;
            return Ok(if options.require_mu_unit {
                ScannedDimen::new(width)
            } else {
                ScannedDimen::with_diagnostic(
                    width,
                    DimensionDiagnostic::IncompatibleGlueUnits,
                    token.origin(),
                )
            });
        }
        Meaning::PageDimension(dimension) => {
            return Ok(ScannedDimen::new(stores.page_dimension(dimension)));
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            let index = scan_register_index(input, stores, expansion, mode, token)?;
            crate::record_dependency!(
                expansion,
                ReadDependency::Cell {
                    bank: ReadBank::Dimen,
                    index: u32::from(index),
                }
            );
            return Ok(ScannedDimen::new(stores.dimen(index)));
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => {
            let index = scan_register_index(input, stores, expansion, mode, token)?;
            crate::record_dependency!(
                expansion,
                ReadDependency::Cell {
                    bank: ReadBank::Skip,
                    index: u32::from(index),
                }
            );
            return Ok(ScannedDimen::new(stores.glue(stores.skip(index)).width));
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
            let index = scan_register_index(input, stores, expansion, mode, token)?;
            crate::record_dependency!(
                expansion,
                ReadDependency::Cell {
                    bank: ReadBank::Muskip,
                    index: u32::from(index),
                }
            );
            let width = stores.glue(stores.muskip(index)).width;
            return Ok(if options.require_mu_unit {
                ScannedDimen::new(width)
            } else {
                ScannedDimen::with_diagnostic(
                    width,
                    DimensionDiagnostic::IncompatibleGlueUnits,
                    token.origin(),
                )
            });
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Wd
            | UnexpandablePrimitive::Ht
            | UnexpandablePrimitive::Dp),
        ) => {
            let index = scan_register_index(input, stores, expansion, mode, token)?;
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
            return Ok(ScannedDimen::new(expansion.engine.prev_depth));
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastKern) => {
            return Ok(ScannedDimen::new(expansion.engine.last_kern));
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastSkip) => {
            return Ok(ScannedDimen::new(expansion.engine.last_skip.width));
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen) => {
            let value = crate::values::scan_font_dimen(input, stores, expansion, mode, token)
                .map_err(ScanDimenError::Expand)?;
            return Ok(ScannedDimen::new(value));
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::FontCharWd
            | UnexpandablePrimitive::FontCharHt
            | UnexpandablePrimitive::FontCharDp
            | UnexpandablePrimitive::FontCharIc),
        ) => {
            let value = crate::values::scan_font_char_dimension(
                input, stores, expansion, mode, token, primitive,
            )?;
            return Ok(ScannedDimen::new(value));
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::ParShapeLength
            | UnexpandablePrimitive::ParShapeIndent
            | UnexpandablePrimitive::ParShapeDimen),
        ) => {
            let value = crate::values::scan_parshape_dimension(
                input, stores, expansion, mode, token, primitive,
            )?;
            return Ok(ScannedDimen::new(value));
        }
        _ => {}
    }

    if stores.resolve(symbol) == "dimen" {
        let index = scan_register_index(input, stores, expansion, mode, token)?;
        return Ok(ScannedDimen::new(stores.dimen(index)));
    }

    *consume_trailing_space = true;
    unread_token(input, stores, token);
    let scanned = scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, token)?;
    if scanned.diagnostic() == Some(scan_int::IntegerDiagnostic::NumberTooBig) {
        return Ok(ScannedDimen::with_diagnostic(
            Scaled::MAX_DIMEN,
            DimensionDiagnostic::TooLarge,
            scanned.diagnostic_origin().unwrap_or(token.origin()),
        ));
    }

    let integer = scanned.value();
    let unit = match scan_unit(input, stores, expansion, mode, options)? {
        UnitScan::Scanned(unit) => unit,
        UnitScan::Rejected(_) if options.coerce_integer_to_sp => {
            return convert_decimal(integer, 0, PhysicalUnit::Sp, false, stores.mag()).map(
                |dimen| {
                    dimen.with_integer_diagnostic(scanned.diagnostic(), scanned.diagnostic_origin())
                },
            );
        }
        UnitScan::Rejected(origin) => {
            return recover_missing_unit(integer, 0, options, stores, origin).map(|dimen| {
                dimen.with_integer_diagnostic(scanned.diagnostic(), scanned.diagnostic_origin())
            });
        }
    };
    convert_scanned_unit(stores, integer, 0, unit).map(|dimen| {
        dimen.with_integer_diagnostic(scanned.diagnostic(), scanned.diagnostic_origin())
    })
}

fn scan_integer_constant_with_unit(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    token: TracedTokenWord,
    options: ScanDimenOptions,
) -> Result<ScannedDimen, ScanDimenError>
where
{
    unread_token(input, stores, token);
    let scanned = scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, token)?;
    if scanned.diagnostic() == Some(scan_int::IntegerDiagnostic::NumberTooBig) {
        return Ok(ScannedDimen::with_diagnostic(
            Scaled::MAX_DIMEN,
            DimensionDiagnostic::TooLarge,
            scanned.diagnostic_origin().unwrap_or(token.origin()),
        ));
    }
    let unit = match scan_unit(input, stores, expansion, mode, options)? {
        UnitScan::Scanned(unit) => unit,
        UnitScan::Rejected(_) if options.coerce_integer_to_sp => {
            return convert_decimal(scanned.value(), 0, PhysicalUnit::Sp, false, stores.mag()).map(
                |dimen| {
                    dimen.with_integer_diagnostic(scanned.diagnostic(), scanned.diagnostic_origin())
                },
            );
        }
        UnitScan::Rejected(origin) => {
            return recover_missing_unit(scanned.value(), 0, options, stores, origin).map(
                |dimen| {
                    dimen.with_integer_diagnostic(scanned.diagnostic(), scanned.diagnostic_origin())
                },
            );
        }
    };
    convert_scanned_unit(stores, scanned.value(), 0, unit).map(|dimen| {
        dimen.with_integer_diagnostic(scanned.diagnostic(), scanned.diagnostic_origin())
    })
}

fn scan_register_index(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<u16, ScanDimenError>
where
{
    let scanned =
        scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?;
    let value = scanned.value();
    let maximum = crate::scan_helpers::maximum_register_index(stores);
    if !(0..=i32::from(maximum)).contains(&value) {
        stores.report_bad_register_code(value, maximum);
        return Ok(0);
    }
    Ok(value as u16)
}

fn scan_unit(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    options: ScanDimenOptions,
) -> Result<UnitScan, ScanDimenError>
where
{
    skip_spaces(input, stores, expansion, mode)?;
    let first = match next_x(input, stores, expansion, mode)? {
        Some(token) => token,
        None => return Ok(UnitScan::Rejected(input.current_input_origin(stores))),
    };

    if let Token::Cs(symbol) = semantic_token(first) {
        let meaning = stores.meaning(symbol);
        expansion.record_meaning(symbol, meaning);
        crate::values::record_meaning_value_dependency(expansion, meaning);
        let internal = match meaning {
            Meaning::DimenRegister(index) => Some(stores.dimen(index)),
            Meaning::DimenParam(index) => {
                Some(stores.dimen_param(tex_state::env::banks::DimenParam::new(index)))
            }
            Meaning::SkipRegister(index) => Some(stores.glue(stores.skip(index)).width),
            Meaning::GlueParam(index) => {
                Some(stores.glue(stores.glue_param(GlueParam::new(index))).width)
            }
            Meaning::PageDimension(dimension) => Some(stores.page_dimension(dimension)),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
                let index = scan_register_index(input, stores, expansion, mode, first)?;
                Some(stores.dimen(index))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => {
                let index = scan_register_index(input, stores, expansion, mode, first)?;
                Some(stores.glue(stores.skip(index)).width)
            }
            Meaning::MuskipRegister(index) => Some(stores.glue(stores.muskip(index)).width),
            Meaning::MuGlueParam(index) => {
                Some(stores.glue(stores.glue_param(GlueParam::new(index))).width)
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
                let index = scan_register_index(input, stores, expansion, mode, first)?;
                Some(stores.glue(stores.muskip(index)).width)
            }
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::Wd
                | UnexpandablePrimitive::Ht
                | UnexpandablePrimitive::Dp),
            ) => {
                let index = scan_register_index(input, stores, expansion, mode, first)?;
                let dimension = match primitive {
                    UnexpandablePrimitive::Wd => BoxDimension::Width,
                    UnexpandablePrimitive::Ht => BoxDimension::Height,
                    UnexpandablePrimitive::Dp => BoxDimension::Depth,
                    _ => unreachable!("outer match restricts primitive"),
                };
                Some(
                    stores
                        .box_dimension(index, dimension)
                        .unwrap_or_else(|| Scaled::from_raw(0)),
                )
            }
            _ => None,
        };
        if let Some(unit) = internal {
            return Ok(UnitScan::Scanned(ScannedUnit::Internal(unit)));
        }
        if matches!(
            meaning,
            Meaning::CharGiven(_)
                | Meaning::MathCharGiven(_)
                | Meaning::CountRegister(_)
                | Meaning::IntParam(_)
                | Meaning::InternalInteger(_)
                | Meaning::PageInteger(_)
                | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count)
        ) {
            let integer =
                scan_int::scan_internal_integer(input, stores, expansion, mode, first, symbol)?;
            return Ok(UnitScan::Scanned(ScannedUnit::Internal(Scaled::from_raw(
                integer.value(),
            ))));
        }
    }

    if options.allow_infinite_units {
        match keyword_matches(input, stores, expansion, mode, first, "fil")? {
            ExpandedKeywordMatch::Matched => {
                let mut order = Order::Fil;
                while keyword(input, stores, expansion, mode, "l")? {
                    if order != Order::Filll {
                        order = match order {
                            Order::Normal => Order::Fil,
                            Order::Fil => Order::Fill,
                            Order::Fill => Order::Filll,
                            Order::Filll => Order::Filll,
                        };
                    }
                }
                return Ok(UnitScan::Scanned(ScannedUnit::Infinite(order)));
            }
            ExpandedKeywordMatch::PartialMismatch => {
                return Ok(UnitScan::Rejected(first.origin()));
            }
            ExpandedKeywordMatch::FirstTokenMismatch => {}
        }
    }

    if options.require_mu_unit {
        match keyword_matches(input, stores, expansion, mode, first, "mu")? {
            ExpandedKeywordMatch::Matched => {
                return Ok(UnitScan::Scanned(physical_unit(PhysicalUnit::Pt)));
            }
            ExpandedKeywordMatch::PartialMismatch => {
                return Ok(UnitScan::Rejected(first.origin()));
            }
            ExpandedKeywordMatch::FirstTokenMismatch => {}
        }
        unread_token(input, stores, first);
        return Ok(UnitScan::Rejected(first.origin()));
    }

    match keyword_matches(input, stores, expansion, mode, first, "true")? {
        ExpandedKeywordMatch::Matched => {
            skip_spaces(input, stores, expansion, mode)?;
            let Some(token) = next_x(input, stores, expansion, mode)? else {
                return Ok(UnitScan::Rejected(input.current_input_origin(stores)));
            };
            return scan_unit_keyword(input, stores, expansion, mode, token, true);
        }
        ExpandedKeywordMatch::PartialMismatch => {
            return Ok(UnitScan::Rejected(first.origin()));
        }
        ExpandedKeywordMatch::FirstTokenMismatch => {}
    }

    scan_unit_keyword(input, stores, expansion, mode, first, false)
}

fn scan_unit_keyword(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    first: TracedTokenWord,
    true_unit: bool,
) -> Result<UnitScan, ScanDimenError>
where
{
    let Some(second) = next_x(input, stores, expansion, mode)? else {
        unread_token(input, stores, first);
        return Ok(UnitScan::Rejected(first.origin()));
    };

    match unit_from_tokens(semantic_token(first), semantic_token(second)) {
        Some(ScannedUnit::Physical { unit, .. }) => {
            Ok(UnitScan::Scanned(ScannedUnit::Physical { unit, true_unit }))
        }
        Some(ScannedUnit::Em) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Cell {
                    bank: ReadBank::CurrentFont,
                    index: 0,
                }
            );
            let font = stores.current_font();
            crate::record_dependency!(
                expansion,
                ReadDependency::Font {
                    field: ReadFontField::Parameter,
                    font: font.raw(),
                    index: 6,
                }
            );
            Ok(UnitScan::Scanned(ScannedUnit::FontRelative {
                unit: stores.font_parameter(font, 6),
            }))
        }
        Some(ScannedUnit::Ex) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Cell {
                    bank: ReadBank::CurrentFont,
                    index: 0,
                }
            );
            let font = stores.current_font();
            crate::record_dependency!(
                expansion,
                ReadDependency::Font {
                    field: ReadFontField::Parameter,
                    font: font.raw(),
                    index: 5,
                }
            );
            Ok(UnitScan::Scanned(ScannedUnit::FontRelative {
                unit: stores.font_parameter(font, 5),
            }))
        }
        Some(ScannedUnit::Infinite(_) | ScannedUnit::Internal(_)) => {
            unreachable!("unit keywords never return non-keyword units")
        }
        Some(ScannedUnit::FontRelative { .. }) => {
            unreachable!("unit keywords never return resolved font-relative units")
        }
        None => {
            unread_tokens(input, stores, [first, second]);
            Ok(UnitScan::Rejected(first.origin()))
        }
    }
}

fn keyword_matches(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    first: TracedTokenWord,
    keyword: &str,
) -> Result<ExpandedKeywordMatch, ScanDimenError>
where
{
    Ok(
        scan_helpers::scan_keyword_after_first_with_mode_and_context(
            input, stores, expansion, mode, first, keyword,
        )?,
    )
}

fn keyword(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    keyword: &str,
) -> Result<bool, ScanDimenError>
where
{
    skip_spaces(input, stores, expansion, mode)?;
    let Some(first) = next_x(input, stores, expansion, mode)? else {
        return Ok(false);
    };
    match scan_helpers::scan_keyword_after_first_with_mode_and_context(
        input, stores, expansion, mode, first, keyword,
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
    Internal(Scaled),
    FontRelative { unit: Scaled },
    Em,
    Ex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnitScan {
    Scanned(ScannedUnit),
    Rejected(OriginId),
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
        match scale_true_dimension_parts(magnitude, fraction, mag) {
            Ok(parts) => parts,
            Err(error) => {
                return Ok(ScannedDimen::with_diagnostic(
                    Scaled::MAX_DIMEN,
                    DimensionDiagnostic::from(error),
                    OriginId::UNKNOWN,
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
            OriginId::UNKNOWN,
        )),
    }
}

fn convert_scanned_unit(
    stores: &mut tex_state::ExpansionContext<'_>,
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
        ScannedUnit::Internal(unit) | ScannedUnit::FontRelative { unit } => {
            convert_font_relative_unit(integer, fraction, unit)
        }
        ScannedUnit::Em | ScannedUnit::Ex => unreachable!("font units are handled while scanning"),
    }
}

fn convert_font_relative_unit(
    integer: i32,
    fraction: i32,
    unit: Scaled,
) -> Result<ScannedDimen, ScanDimenError> {
    assert!(
        (0..=Scaled::UNITY).contains(&fraction),
        "dimension fraction out of range"
    );
    let negative = integer < 0;
    let magnitude = if negative {
        integer.checked_neg().unwrap_or(Scaled::MAX_DIMEN.raw() + 1)
    } else {
        integer
    };

    let fractional = match xn_over_d(unit, fraction, Scaled::UNITY) {
        Ok(value) => value.quotient,
        Err(error) => {
            return Ok(ScannedDimen::with_diagnostic(
                Scaled::MAX_DIMEN,
                DimensionDiagnostic::from(error),
                OriginId::UNKNOWN,
            ));
        }
    };
    match nx_plus_y(magnitude, unit, fractional).and_then(|value| {
        value
            .check_dimension()
            .map_err(|_| tex_state::scaled::ArithmeticError::Overflow)
    }) {
        Ok(value) if negative => Ok(ScannedDimen::new(-value)),
        Ok(value) => Ok(ScannedDimen::new(value)),
        Err(_) => Ok(ScannedDimen::with_diagnostic(
            Scaled::MAX_DIMEN,
            DimensionDiagnostic::TooLarge,
            OriginId::UNKNOWN,
        )),
    }
}

fn convert_physical_unit(
    stores: &mut tex_state::ExpansionContext<'_>,
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
        scanned =
            scanned.with_added_diagnostic(DimensionDiagnostic::from(diagnostic), OriginId::UNKNOWN);
    }
    Ok(scanned)
}

fn coerce_or_recover_missing_unit(
    integer: i32,
    options: ScanDimenOptions,
    stores: &impl ExpansionState,
    origin: OriginId,
) -> Result<ScannedDimen, ScanDimenError> {
    if options.coerce_integer_to_sp {
        convert_decimal(integer, 0, PhysicalUnit::Sp, false, stores.mag())
    } else {
        recover_missing_unit(integer, 0, options, stores, origin)
    }
}

fn recover_missing_unit(
    integer: i32,
    fraction: i32,
    options: ScanDimenOptions,
    stores: &impl ExpansionState,
    origin: OriginId,
) -> Result<ScannedDimen, ScanDimenError> {
    recover_missing_unit_with_mag(integer, fraction, options, stores.mag(), origin)
}

fn recover_missing_unit_with_mag(
    integer: i32,
    fraction: i32,
    options: ScanDimenOptions,
    mag: i32,
    origin: OriginId,
) -> Result<ScannedDimen, ScanDimenError> {
    let inserted = if options.require_mu_unit {
        InsertedUnit::Mu
    } else {
        InsertedUnit::Pt
    };
    convert_decimal(integer, fraction, PhysicalUnit::Pt, false, mag).map(|dimen| {
        dimen.with_added_diagnostic(DimensionDiagnostic::IllegalUnit { inserted }, origin)
    })
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
        diagnostic_origins: scanned.diagnostic_origins,
    }
}

fn consume_optional_space(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
) -> Result<(), ScanDimenError>
where
{
    let Some(token) = next_x(input, stores, expansion, mode)? else {
        return Ok(());
    };
    if !is_space(token) {
        unread_token(input, stores, token);
    }
    Ok(())
}

fn skip_spaces(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
) -> Result<(), ScanDimenError>
where
{
    loop {
        let Some(token) = next_x(input, stores, expansion, mode)? else {
            return Ok(());
        };
        if !is_space(token) {
            unread_token(input, stores, token);
            return Ok(());
        }
    }
}

fn unread_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    token: TracedTokenWord,
) {
    unread_tokens(input, stores, [token]);
}

fn unread_tokens<I>(input: &mut InputStack, stores: &mut tex_state::ExpansionContext<'_>, tokens: I)
where
    I: IntoIterator<Item = TracedTokenWord>,
{
    crate::back_input(input, stores, tokens);
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
