//! Expanded glue and muglue scanning shared by future assignment consumers.

use std::fmt;

use tex_lex::{InputSource, InputStack, LexError};
use tex_state::ExpansionState;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::scan_dimen::{self, DimensionDiagnostic, ScanDimenError, ScanDimenOptions};
use crate::{
    ExpandError, ExpandNext, ExpansionContext, NoInputExpandNext, ReadBank, ReadDependency,
    scan_helpers, scan_int, semantic_token,
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
    scan_glue_with_context(
        input,
        stores,
        &mut ExpansionContext::new("texput"),
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
    scan_glue_with_context(
        input,
        stores,
        &mut ExpansionContext::new("texput"),
        true,
        context,
    )
}

pub fn scan_glue_with_context<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    mu: bool,
    context: TracedTokenWord,
) -> Result<ScannedGlue, ScanGlueError>
where
    S: InputSource,
{
    scan_glue_with_expander_and_context(
        input,
        stores,
        expansion,
        &mut NoInputExpandNext,
        mu,
        context,
    )
}

pub fn scan_glue_with_expander_and_context<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
    mu: bool,
    context: TracedTokenWord,
) -> Result<ScannedGlue, ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let (negative, first) = scan_signs(input, stores, expansion, expander)?;
    let Some(first) = first else {
        return Err(ScanGlueError::MissingNumber { context });
    };

    if let Token::Cs(symbol) = semantic_token(first) {
        let meaning = stores.meaning(symbol);
        expansion.record_meaning(symbol, meaning);
        crate::values::record_meaning_value_dependency(expansion, meaning);
        match meaning {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::GlueExpr) if !mu => {
                let scanned = scan_glue_expr(input, stores, expansion, expander, false, first)?;
                return Ok(intern_spec_with_diagnostics(
                    stores,
                    signed_spec(stores.glue(scanned.id()), negative),
                    scanned.diagnostics,
                    scanned.diagnostic_origins,
                ));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MuExpr) if mu => {
                let scanned = scan_glue_expr(input, stores, expansion, expander, true, first)?;
                return Ok(intern_spec_with_diagnostics(
                    stores,
                    signed_spec(stores.glue(scanned.id()), negative),
                    scanned.diagnostics,
                    scanned.diagnostic_origins,
                ));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::GlueToMu) if mu => {
                let scanned = scan_glue_with_expander_and_context(
                    input, stores, expansion, expander, false, first,
                )?;
                return Ok(intern_spec_with_diagnostics(
                    stores,
                    signed_spec(stores.glue(scanned.id()), negative),
                    scanned.diagnostics,
                    scanned.diagnostic_origins,
                ));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MuToGlue) if !mu => {
                let scanned = scan_glue_with_expander_and_context(
                    input, stores, expansion, expander, true, first,
                )?;
                return Ok(intern_spec_with_diagnostics(
                    stores,
                    signed_spec(stores.glue(scanned.id()), negative),
                    scanned.diagnostics,
                    scanned.diagnostic_origins,
                ));
            }
            Meaning::SkipRegister(index) if !mu => {
                consume_optional_space(input, stores, expansion, expander)?;
                let spec = stores.glue(stores.skip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::MuskipRegister(index) if mu => {
                consume_optional_space(input, stores, expansion, expander)?;
                let spec = stores.glue(stores.muskip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::GlueParam(index) if !mu => {
                consume_optional_space(input, stores, expansion, expander)?;
                let spec =
                    stores.glue(stores.glue_param(tex_state::env::banks::GlueParam::new(index)));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::MuGlueParam(index) if mu => {
                consume_optional_space(input, stores, expansion, expander)?;
                let spec =
                    stores.glue(stores.glue_param(tex_state::env::banks::GlueParam::new(index)));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) if !mu => {
                let index = scan_register_index(input, stores, expansion, expander, first)?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::Skip,
                        index: u32::from(index),
                    }
                );
                consume_optional_space(input, stores, expansion, expander)?;
                let spec = stores.glue(stores.skip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) if mu => {
                let index = scan_register_index(input, stores, expansion, expander, first)?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::Muskip,
                        index: u32::from(index),
                    }
                );
                consume_optional_space(input, stores, expansion, expander)?;
                let spec = stores.glue(stores.muskip(index));
                return Ok(intern_spec(stores, signed_spec(spec, negative)));
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastSkip) if !mu => {
                consume_optional_space(input, stores, expansion, expander)?;
                return Ok(intern_spec(
                    stores,
                    signed_spec(expansion.engine.last_skip, negative),
                ));
            }
            _ => {
                let name = stores.resolve(symbol);
                if (!mu && name == "skip") || (mu && name == "muskip") {
                    let index = scan_register_index(input, stores, expansion, expander, first)?;
                    crate::record_dependency!(
                        expansion,
                        ReadDependency::Cell {
                            bank: if mu { ReadBank::Muskip } else { ReadBank::Skip },
                            index: u32::from(index),
                        }
                    );
                    consume_optional_space(input, stores, expansion, expander)?;
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
    let width = scan_dimen::scan_dimen_with_expander_and_context(
        input,
        stores,
        expansion,
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

    if scan_keyword(input, stores, expansion, expander, "plus")? {
        let stretch = scan_dimen::scan_dimen_with_expander_and_context(
            input,
            stores,
            expansion,
            expander,
            dimen_options(mu).with_infinite_units(),
            context,
        )?;
        append_dimension_diagnostics(&mut diagnostics, &mut diagnostic_origins, stretch);
        spec.stretch = stretch.value();
        spec.stretch_order = stretch.order();
    }
    if scan_keyword(input, stores, expansion, expander, "minus")? {
        let shrink = scan_dimen::scan_dimen_with_expander_and_context(
            input,
            stores,
            expansion,
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

pub(crate) fn scan_glue_expr<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
    mu: bool,
    context: TracedTokenWord,
) -> Result<ScannedGlue, ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let (spec, bad) = parse_glue_expr(input, stores, expansion, expander, mu, false)?;
    if bad {
        Ok(intern_spec_with_diagnostics(
            stores,
            GlueSpec::ZERO,
            [
                Some(DimensionDiagnostic::TooLarge),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            [
                Some(context.origin()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
        ))
    } else {
        Ok(intern_spec(stores, spec))
    }
}

fn parse_glue_expr<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
    mu: bool,
    paren: bool,
) -> Result<(GlueSpec, bool), ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let (mut spec, mut bad) = parse_glue_term(input, stores, expansion, expander, mu)?;
    loop {
        let Some(token) = expr_next(input, stores, expansion, expander)? else {
            break;
        };
        let subtract = if is_other_char(token, '+') {
            false
        } else if is_other_char(token, '-') {
            true
        } else {
            if paren && is_other_char(token, ')') {
                break;
            }
            if !matches!(semantic_token(token), Token::Cs(s) if stores.meaning(s) == Meaning::Relax)
            {
                unread_token(input, stores, token);
            }
            break;
        };
        normalize_glue_orders(&mut spec);
        let (rhs, rhs_bad) = parse_glue_term(input, stores, expansion, expander, mu)?;
        bad |= rhs_bad;
        match add_glue(spec, rhs, subtract) {
            Some(next) => spec = next,
            None => {
                spec = GlueSpec::ZERO;
                bad = true;
            }
        }
    }
    Ok((spec, bad))
}

fn parse_glue_term<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
    mu: bool,
) -> Result<(GlueSpec, bool), ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let (mut spec, mut bad) = parse_glue_factor(input, stores, expansion, expander, mu)?;
    loop {
        let Some(op) = expr_next(input, stores, expansion, expander)? else {
            break;
        };
        if is_other_char(op, '*') {
            normalize_glue_orders(&mut spec);
            let (n, nbad) = parse_int_factor(input, stores, expansion, expander)?;
            bad |= nbad;
            let next = expr_next(input, stores, expansion, expander)?;
            if next.is_some_and(|t| is_other_char(t, '/')) {
                let (d, dbad) = parse_int_factor(input, stores, expansion, expander)?;
                bad |= dbad;
                match scale_glue(spec, n, d) {
                    Some(v) => spec = v,
                    None => {
                        spec = GlueSpec::ZERO;
                        bad = true;
                    }
                }
            } else {
                if let Some(t) = next {
                    unread_token(input, stores, t);
                }
                match scale_glue(spec, n, 1) {
                    Some(v) => spec = v,
                    None => {
                        spec = GlueSpec::ZERO;
                        bad = true;
                    }
                }
            }
        } else if is_other_char(op, '/') {
            normalize_glue_orders(&mut spec);
            let (d, dbad) = parse_int_factor(input, stores, expansion, expander)?;
            bad |= dbad;
            match scale_glue(spec, 1, d) {
                Some(v) => spec = v,
                None => {
                    spec = GlueSpec::ZERO;
                    bad = true;
                }
            }
        } else {
            unread_token(input, stores, op);
            break;
        }
    }
    Ok((spec, bad))
}

fn parse_glue_factor<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
    mu: bool,
) -> Result<(GlueSpec, bool), ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let Some(token) = expr_next(input, stores, expansion, expander)? else {
        return Ok((GlueSpec::ZERO, true));
    };
    if is_other_char(token, '(') {
        return parse_glue_expr(input, stores, expansion, expander, mu, true);
    }
    unread_token(input, stores, token);
    let scanned =
        scan_glue_with_expander_and_context(input, stores, expansion, expander, mu, token)?;
    Ok((
        stores.glue(scanned.id()),
        scanned.diagnostics().next().is_some(),
    ))
}

fn parse_int_factor<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
) -> Result<(i64, bool), ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let Some(token) = expr_next(input, stores, expansion, expander)? else {
        return Ok((0, true));
    };
    if is_other_char(token, '(') {
        return Ok(scan_int::parse_num_expression(
            input, stores, expansion, expander, true,
        )?);
    }
    unread_token(input, stores, token);
    let scanned =
        scan_int::scan_int_with_expander_and_context(input, stores, expansion, expander, token)?;
    Ok((i64::from(scanned.value()), scanned.diagnostic().is_some()))
}

fn expr_next<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
) -> Result<Option<TracedTokenWord>, ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    loop {
        let token = next_x(input, stores, expansion, expander)?;
        if token.is_none_or(|t| !is_space(t)) {
            return Ok(token);
        }
    }
}

fn add_glue(mut left: GlueSpec, mut right: GlueSpec, subtract: bool) -> Option<GlueSpec> {
    if subtract {
        right = signed_spec(right, true);
    }
    left.width = checked_component(i64::from(left.width.raw()) + i64::from(right.width.raw()))?;
    (left.stretch, left.stretch_order) = add_ordered(
        left.stretch,
        left.stretch_order,
        right.stretch,
        right.stretch_order,
    )?;
    (left.shrink, left.shrink_order) = add_ordered(
        left.shrink,
        left.shrink_order,
        right.shrink,
        right.shrink_order,
    )?;
    Some(left)
}

fn add_ordered(a: Scaled, ao: Order, b: Scaled, bo: Order) -> Option<(Scaled, Order)> {
    if ao == bo {
        let v = checked_component(i64::from(a.raw()) + i64::from(b.raw()))?;
        Some((v, if v.raw() == 0 { Order::Normal } else { ao }))
    } else if order_rank(bo) > order_rank(ao) && b.raw() != 0 {
        Some((b, bo))
    } else {
        Some((a, ao))
    }
}

fn normalize_glue_orders(spec: &mut GlueSpec) {
    if spec.stretch.raw() == 0 {
        spec.stretch_order = Order::Normal;
    }
    if spec.shrink.raw() == 0 {
        spec.shrink_order = Order::Normal;
    }
}

fn scale_glue(spec: GlueSpec, n: i64, d: i64) -> Option<GlueSpec> {
    Some(GlueSpec {
        width: scale_component(spec.width, n, d)?,
        stretch: scale_component(spec.stretch, n, d)?,
        stretch_order: spec.stretch_order,
        shrink: scale_component(spec.shrink, n, d)?,
        shrink_order: spec.shrink_order,
    })
}
fn scale_component(v: Scaled, n: i64, d: i64) -> Option<Scaled> {
    checked_component(scan_int::rounded_fraction(i64::from(v.raw()), n, d)?)
}
fn checked_component(v: i64) -> Option<Scaled> {
    (v.abs() <= i64::from(Scaled::MAX_DIMEN.raw())).then(|| Scaled::from_raw(v as i32))
}
fn order_rank(order: Order) -> u8 {
    match order {
        Order::Normal => 0,
        Order::Fil => 1,
        Order::Fill => 2,
        Order::Filll => 3,
    }
}

fn scan_signs<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
) -> Result<(bool, Option<TracedTokenWord>), ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let mut negative = false;
    loop {
        let Some(token) = next_x(input, stores, expansion, expander)? else {
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

fn next_x<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
) -> Result<Option<TracedTokenWord>, ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    Ok(expander.next_expanded_token(input, stores, expansion)?)
}

fn scan_register_index<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
    context: TracedTokenWord,
) -> Result<u16, ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let scanned = crate::scan_int::scan_int_with_expander_and_context(
        input, stores, expansion, expander, context,
    )?;
    let value = scanned.value();
    let maximum = crate::scan_helpers::maximum_register_index(stores);
    if !(0..=i32::from(maximum)).contains(&value) {
        stores.report_bad_register_code(value, maximum);
        return Ok(0);
    }
    Ok(value as u16)
}

fn scan_keyword<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
    keyword: &str,
) -> Result<bool, ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    Ok(
        scan_helpers::scan_optional_keyword_with_expander_and_context(
            input, stores, expansion, expander, keyword,
        )?,
    )
}

fn consume_optional_space<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
) -> Result<(), ScanGlueError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let Some(token) = next_x(input, stores, expansion, expander)? else {
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
    crate::back_input(input, stores, tokens);
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
