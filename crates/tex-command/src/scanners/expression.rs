//! Canonical e-TeX expression and glue-conversion scanners.

use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::token::Catcode;

use super::scalar::InternalValue;
use crate::observation::canonical_names::glue_order_name;
use crate::processor::CommandProcessor;
use crate::{CommandError, CommandObservation, CurrentCommand, FatalError, ScannerRecord};

const EXPRESSION_DEPTH_LIMIT: u32 = 10_000;
const INTEGER_LIMIT: i64 = i32::MAX as i64;
const DIMENSION_LIMIT: i64 = Scaled::MAX_DIMEN.raw() as i64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpressionKind {
    Integer,
    Dimension,
    Glue,
    MuGlue,
}

impl ExpressionKind {
    const fn of_primitive(primitive: UnexpandablePrimitive) -> Option<Self> {
        match primitive {
            UnexpandablePrimitive::NumExpr => Some(Self::Integer),
            UnexpandablePrimitive::DimExpr => Some(Self::Dimension),
            UnexpandablePrimitive::GlueExpr => Some(Self::Glue),
            UnexpandablePrimitive::MuExpr => Some(Self::MuGlue),
            _ => None,
        }
    }

    const fn limit(self) -> i64 {
        match self {
            Self::Integer => INTEGER_LIMIT,
            Self::Dimension | Self::Glue | Self::MuGlue => DIMENSION_LIMIT,
        }
    }

    const fn scanner_name(self) -> &'static str {
        match self {
            Self::Integer => "expression_integer",
            Self::Dimension => "expression_dimension",
            Self::Glue => "expression_glue",
            Self::MuGlue => "expression_muglue",
        }
    }

    fn zero(self) -> ExpressionValue {
        match self {
            Self::Integer | Self::Dimension => ExpressionValue::Number(0),
            Self::Glue | Self::MuGlue => ExpressionValue::Glue(ExpressionGlue::ZERO),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExpressionGlue {
    width: i64,
    stretch: i64,
    stretch_order: Order,
    shrink: i64,
    shrink_order: Order,
    identity: Option<GlueId>,
    skip_index: Option<u16>,
}

impl ExpressionGlue {
    const ZERO: Self = Self {
        width: 0,
        stretch: 0,
        stretch_order: Order::Normal,
        shrink: 0,
        shrink_order: Order::Normal,
        identity: None,
        skip_index: None,
    };

    fn from_spec(spec: GlueSpec, identity: Option<GlueId>, skip_index: Option<u16>) -> Self {
        Self {
            width: i64::from(spec.width.raw()),
            stretch: i64::from(spec.stretch.raw()),
            stretch_order: spec.stretch_order,
            shrink: i64::from(spec.shrink.raw()),
            shrink_order: spec.shrink_order,
            identity,
            skip_index,
        }
    }

    fn into_spec(self) -> GlueSpec {
        GlueSpec {
            width: Scaled::from_raw(i32::try_from(self.width).expect("checked width fits i32")),
            stretch: Scaled::from_raw(
                i32::try_from(self.stretch).expect("checked stretch fits i32"),
            ),
            stretch_order: self.stretch_order,
            shrink: Scaled::from_raw(i32::try_from(self.shrink).expect("checked shrink fits i32")),
            shrink_order: self.shrink_order,
        }
    }

    fn normalize(&mut self) {
        if self.stretch == 0 {
            self.stretch_order = Order::Normal;
        }
        if self.shrink == 0 {
            self.shrink_order = Order::Normal;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpressionValue {
    Number(i64),
    Glue(ExpressionGlue),
}

impl ExpressionValue {
    fn integer(self) -> i64 {
        let Self::Number(value) = self else {
            unreachable!("e-TeX expression multipliers are integer-valued")
        };
        value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpressionOperator {
    None,
    Add,
    Subtract,
    Multiply,
    Divide,
    Scale,
}

impl ExpressionOperator {
    const fn continues_term(self) -> bool {
        matches!(self, Self::Multiply | Self::Divide | Self::Scale)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExpressionFrame {
    kind: ExpressionKind,
    expression_operator: ExpressionOperator,
    expression: ExpressionValue,
    term_operator: ExpressionOperator,
    term: ExpressionValue,
    scale_numerator: i64,
}

impl ExpressionFrame {
    fn new(kind: ExpressionKind) -> Self {
        Self {
            kind,
            expression_operator: ExpressionOperator::None,
            expression: kind.zero(),
            term_operator: ExpressionOperator::None,
            term: kind.zero(),
            scale_numerator: 0,
        }
    }

    const fn factor_kind(self) -> ExpressionKind {
        if matches!(self.term_operator, ExpressionOperator::None) {
            self.kind
        } else {
            ExpressionKind::Integer
        }
    }
}

enum ScannedFactor {
    Value(ExpressionValue),
    OpenParenthesis,
}

impl CommandProcessor<'_> {
    /// e-TeX 2.6 `scan_expr`, from the already-delivered expression primitive
    /// through its committed typed result (etex.ch [53a.4945--5360]).
    pub(super) fn scan_expression_primitive(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<InternalValue, CommandError> {
        let kind = ExpressionKind::of_primitive(primitive)
            .expect("only e-TeX expression primitives reach scan_expr");
        // Web2C's `expand_depth_count` increments once per expression
        // primitive. Parentheses use the explicit stack below and do not.
        self.expression_depth = self.expression_depth.saturating_add(1);
        if self.expression_depth >= EXPRESSION_DEPTH_LIMIT {
            let fatal = FatalError::overflow(
                "expansion depth",
                i32::try_from(EXPRESSION_DEPTH_LIMIT).expect("depth limit fits i32"),
            );
            self.observe(CommandObservation::Diagnostic(fatal.record()));
            return Err(CommandError::Fatal(fatal));
        }

        let result = self.scan_expression(kind);
        self.expression_depth -= 1;
        let (mut value, overflow) = result?;
        if overflow {
            self.expression_arithmetic_error()?;
            value = kind.zero();
        }
        self.observe_expression(kind, value);
        self.scanned_glue_identity = match value {
            ExpressionValue::Glue(value) => value.identity,
            ExpressionValue::Number(_) => None,
        };
        self.scanned_glue_skip_index = match value {
            ExpressionValue::Glue(value) => value.skip_index,
            ExpressionValue::Number(_) => None,
        };
        Ok(expression_internal_value(kind, value))
    }

    /// etex.ch [53a.5404--5425]: scan at the source glue level and return the
    /// identical components and orders at the destination level.
    pub(super) fn scan_glue_conversion_primitive(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<InternalValue, CommandError> {
        let (scan_mu, scanner, to_mu) = match primitive {
            UnexpandablePrimitive::MuToGlue => (true, "mu_to_glue", false),
            UnexpandablePrimitive::GlueToMu => (false, "glue_to_mu", true),
            _ => unreachable!("only e-TeX glue conversions reach this scanner"),
        };
        let value = self.scan_glue(scan_mu)?.value;
        self.observe(CommandObservation::Scanner(ScannerRecord {
            kind: scanner,
            value: glue_value(value),
            tokens: None,
        }));
        Ok(if to_mu {
            InternalValue::MuGlue(value)
        } else {
            InternalValue::Glue(value)
        })
    }

    /// The iterative state machine from e-TeX's `scan_expr`. Its explicit
    /// stack is important: parenthesized subexpressions do not consume
    /// expansion depth and do not recurse on the Rust stack.
    fn scan_expression(
        &mut self,
        kind: ExpressionKind,
    ) -> Result<(ExpressionValue, bool), CommandError> {
        let mut stack = Vec::new();
        let mut frame = ExpressionFrame::new(kind);
        let mut overflow = false;

        'scan_factor: loop {
            let factor_kind = frame.factor_kind();
            let mut factor = match self.scan_expression_factor(factor_kind)? {
                ScannedFactor::Value(value) => value,
                ScannedFactor::OpenParenthesis => {
                    stack.push(frame);
                    frame = ExpressionFrame::new(factor_kind);
                    continue 'scan_factor;
                }
            };

            // `found:` in etex.ch. A completed parenthesized expression
            // returns here as the current factor of the restored frame.
            loop {
                let mut operator = self.scan_expression_operator(!stack.is_empty())?;
                factor = validate_factor(factor, frame.kind, frame.term_operator, &mut overflow);
                operator = apply_factor(&mut frame, factor, operator, &mut overflow);

                if operator.continues_term() {
                    frame.term_operator = operator;
                    continue 'scan_factor;
                }
                evaluate_term(&mut frame, operator, &mut overflow);
                if !matches!(operator, ExpressionOperator::None) {
                    continue 'scan_factor;
                }

                let completed = frame.expression;
                let Some(parent) = stack.pop() else {
                    return Ok((completed, overflow));
                };
                factor = completed;
                frame = parent;
            }
        }
    }

    fn scan_expression_factor(
        &mut self,
        kind: ExpressionKind,
    ) -> Result<ScannedFactor, CommandError> {
        let first = self.next_non_blank_x_token()?;
        if first
            .as_ref()
            .is_some_and(|command| is_other_character(command, '('))
        {
            return Ok(ScannedFactor::OpenParenthesis);
        }
        if let Some(command) = first {
            self.back_input(command)?;
        }
        let value = match kind {
            ExpressionKind::Integer => {
                ExpressionValue::Number(i64::from(self.scan_integer()?.value))
            }
            ExpressionKind::Dimension => {
                ExpressionValue::Number(i64::from(self.scan_dimension()?.value.raw()))
            }
            ExpressionKind::Glue => {
                let value = self.scan_glue(false)?.value;
                ExpressionValue::Glue(ExpressionGlue::from_spec(
                    value,
                    self.scanned_glue_identity,
                    self.scanned_glue_skip_index,
                ))
            }
            ExpressionKind::MuGlue => {
                let value = self.scan_glue(true)?.value;
                ExpressionValue::Glue(ExpressionGlue::from_spec(
                    value,
                    self.scanned_glue_identity,
                    self.scanned_glue_skip_index,
                ))
            }
        };
        Ok(ScannedFactor::Value(value))
    }

    fn scan_expression_operator(
        &mut self,
        parenthesized: bool,
    ) -> Result<ExpressionOperator, CommandError> {
        let Some(command) = self.next_non_blank_x_token()? else {
            return Ok(ExpressionOperator::None);
        };
        for (character, operator) in [
            ('+', ExpressionOperator::Add),
            ('-', ExpressionOperator::Subtract),
            ('*', ExpressionOperator::Multiply),
            ('/', ExpressionOperator::Divide),
        ] {
            if is_other_character(&command, character) {
                return Ok(operator);
            }
        }

        if parenthesized {
            if !is_other_character(&command, ')') {
                self.back_input(command)?;
                self.missing_expression_parenthesis_error()?;
            }
        } else if !matches!(command.meaning(), Meaning::Relax) {
            self.back_input(command)?;
        }
        Ok(ExpressionOperator::None)
    }

    fn missing_expression_parenthesis_error(&mut self) -> Result<(), CommandError> {
        // e-TeX \[26.1576] reaches `back_error`, so the caller has already
        // restored the rejected token for §314 to name.
        let context = self.command.output_open_context(&self.state);
        let mut report = self.state.print_err("Missing ) inserted for expression");
        report
            .help(&["I was expecting to see `+', `-', `*', `/', or `)'. Didn't."])
            .context(context);
        report.error().jump_out()?;
        Ok(())
    }

    fn expression_arithmetic_error(&mut self) -> Result<(), CommandError> {
        let context = self.command.output_open_context(&self.state);
        let mut report = self.state.print_err("Arithmetic overflow");
        report
            .help(&[
                "I can't evaluate this expression,",
                "since the result is out of range.",
            ])
            .context(context);
        report.error().jump_out()?;
        Ok(())
    }

    fn observe_expression(&mut self, kind: ExpressionKind, value: ExpressionValue) {
        let value = match value {
            ExpressionValue::Number(value) => value.to_string(),
            ExpressionValue::Glue(value) => glue_value(value.into_spec()),
        };
        self.observe(CommandObservation::Scanner(ScannerRecord {
            kind: kind.scanner_name(),
            value,
            tokens: None,
        }));
    }
}

fn is_other_character(command: &CurrentCommand, expected: char) -> bool {
    matches!(
        command.meaning(),
        Meaning::CharToken {
            ch,
            cat: Catcode::Other,
        } if ch == expected
    )
}

fn validate_factor(
    factor: ExpressionValue,
    kind: ExpressionKind,
    term_operator: ExpressionOperator,
    overflow: &mut bool,
) -> ExpressionValue {
    let limit = if term_operator.continues_term() {
        INTEGER_LIMIT
    } else {
        kind.limit()
    };
    match factor {
        ExpressionValue::Number(value) => {
            ExpressionValue::Number(bounded_number(value, limit, overflow))
        }
        ExpressionValue::Glue(mut value) => {
            value.width = bounded_number(value.width, DIMENSION_LIMIT, overflow);
            value.stretch = bounded_number(value.stretch, DIMENSION_LIMIT, overflow);
            value.shrink = bounded_number(value.shrink, DIMENSION_LIMIT, overflow);
            ExpressionValue::Glue(value)
        }
    }
}

fn bounded_number(value: i64, limit: i64, overflow: &mut bool) -> i64 {
    if (-limit..=limit).contains(&value) {
        value
    } else {
        *overflow = true;
        0
    }
}

fn apply_factor(
    frame: &mut ExpressionFrame,
    factor: ExpressionValue,
    mut operator: ExpressionOperator,
    overflow: &mut bool,
) -> ExpressionOperator {
    match frame.term_operator {
        ExpressionOperator::None => {
            frame.term = factor;
            if matches!(frame.term, ExpressionValue::Glue(_))
                && !matches!(operator, ExpressionOperator::None)
            {
                let ExpressionValue::Glue(mut glue) = frame.term else {
                    unreachable!()
                };
                glue.normalize();
                glue.identity = None;
                glue.skip_index = None;
                frame.term = ExpressionValue::Glue(glue);
            }
        }
        ExpressionOperator::Multiply if matches!(operator, ExpressionOperator::Divide) => {
            frame.scale_numerator = factor.integer();
            operator = ExpressionOperator::Scale;
        }
        ExpressionOperator::Multiply => {
            frame.term = multiply_value(frame.term, factor.integer(), frame.kind, overflow);
        }
        ExpressionOperator::Divide => {
            frame.term = divide_value(frame.term, factor.integer(), frame.kind, overflow);
        }
        ExpressionOperator::Scale => {
            frame.term = scale_value(
                frame.term,
                frame.scale_numerator,
                factor.integer(),
                frame.kind,
                overflow,
            );
        }
        ExpressionOperator::Add | ExpressionOperator::Subtract => {
            unreachable!("add/subtract is expression state, never term state")
        }
    }
    operator
}

fn evaluate_term(frame: &mut ExpressionFrame, next: ExpressionOperator, overflow: &mut bool) {
    frame.expression = match frame.expression_operator {
        ExpressionOperator::None => frame.term,
        ExpressionOperator::Add => {
            add_value(frame.expression, frame.term, false, frame.kind, overflow)
        }
        ExpressionOperator::Subtract => {
            add_value(frame.expression, frame.term, true, frame.kind, overflow)
        }
        ExpressionOperator::Multiply | ExpressionOperator::Divide | ExpressionOperator::Scale => {
            unreachable!("multiply/divide is term state, never expression state")
        }
    };
    frame.term_operator = ExpressionOperator::None;
    frame.expression_operator = next;
}

fn add_value(
    left: ExpressionValue,
    right: ExpressionValue,
    subtract: bool,
    kind: ExpressionKind,
    overflow: &mut bool,
) -> ExpressionValue {
    match (left, right) {
        (ExpressionValue::Number(left), ExpressionValue::Number(right)) => {
            let right = if subtract { -right } else { right };
            ExpressionValue::Number(bounded_number(left + right, kind.limit(), overflow))
        }
        (ExpressionValue::Glue(mut left), ExpressionValue::Glue(right)) => {
            left.width = add_component(left.width, right.width, subtract, overflow);
            add_ordered_component(
                &mut left.stretch,
                &mut left.stretch_order,
                right.stretch,
                right.stretch_order,
                subtract,
                overflow,
            );
            add_ordered_component(
                &mut left.shrink,
                &mut left.shrink_order,
                right.shrink,
                right.shrink_order,
                subtract,
                overflow,
            );
            left.normalize();
            left.identity = None;
            left.skip_index = None;
            ExpressionValue::Glue(left)
        }
        _ => unreachable!("one expression frame has one value type"),
    }
}

fn add_component(left: i64, right: i64, subtract: bool, overflow: &mut bool) -> i64 {
    bounded_number(
        left + if subtract { -right } else { right },
        DIMENSION_LIMIT,
        overflow,
    )
}

fn add_ordered_component(
    left: &mut i64,
    left_order: &mut Order,
    right: i64,
    right_order: Order,
    subtract: bool,
    overflow: &mut bool,
) {
    if *left_order == right_order {
        *left = add_component(*left, right, subtract, overflow);
    } else if *left_order < right_order && right != 0 {
        // etex.ch's exact dominant-order branch copies the higher-order
        // component without applying the expression subtraction flag.
        *left = right;
        *left_order = right_order;
    }
}

fn multiply_value(
    value: ExpressionValue,
    factor: i64,
    kind: ExpressionKind,
    overflow: &mut bool,
) -> ExpressionValue {
    map_components(value, kind.limit(), overflow, |component, limit| {
        bounded_i128(i128::from(component) * i128::from(factor), limit)
    })
}

fn divide_value(
    value: ExpressionValue,
    divisor: i64,
    kind: ExpressionKind,
    overflow: &mut bool,
) -> ExpressionValue {
    map_components(value, kind.limit(), overflow, |component, limit| {
        rounded_fraction(component, 1, divisor, limit)
    })
}

fn scale_value(
    value: ExpressionValue,
    numerator: i64,
    denominator: i64,
    kind: ExpressionKind,
    overflow: &mut bool,
) -> ExpressionValue {
    map_components(value, kind.limit(), overflow, |component, limit| {
        rounded_fraction(component, numerator, denominator, limit)
    })
}

fn map_components(
    value: ExpressionValue,
    number_limit: i64,
    overflow: &mut bool,
    mut map: impl FnMut(i64, i64) -> Option<i64>,
) -> ExpressionValue {
    let mut apply = |value, limit| match map(value, limit) {
        Some(value) => value,
        None => {
            *overflow = true;
            0
        }
    };
    match value {
        ExpressionValue::Number(value) => ExpressionValue::Number(apply(value, number_limit)),
        ExpressionValue::Glue(mut glue) => {
            glue.width = apply(glue.width, DIMENSION_LIMIT);
            glue.stretch = apply(glue.stretch, DIMENSION_LIMIT);
            glue.shrink = apply(glue.shrink, DIMENSION_LIMIT);
            glue.identity = None;
            glue.skip_index = None;
            ExpressionValue::Glue(glue)
        }
    }
}

fn bounded_i128(value: i128, limit: i64) -> Option<i64> {
    let limit = i128::from(limit);
    (-limit..=limit)
        .contains(&value)
        .then(|| i64::try_from(value).expect("bounded expression result fits i64"))
}

/// e-TeX's `fract`/`quotient`: nearest-integer division, with exact halves
/// rounded away from zero and the result bounded before publication.
fn rounded_fraction(value: i64, numerator: i64, denominator: i64, limit: i64) -> Option<i64> {
    if denominator == 0 {
        return None;
    }
    let product = i128::from(value) * i128::from(numerator);
    let divisor = i128::from(denominator);
    let negative = (product < 0) ^ (divisor < 0);
    let product = product.abs();
    let divisor = divisor.abs();
    let mut quotient = product / divisor;
    if (product % divisor) * 2 >= divisor {
        quotient += 1;
    }
    let quotient = if negative { -quotient } else { quotient };
    bounded_i128(quotient, limit)
}

fn expression_internal_value(kind: ExpressionKind, value: ExpressionValue) -> InternalValue {
    match (kind, value) {
        (ExpressionKind::Integer, ExpressionValue::Number(value)) => {
            InternalValue::Integer(i32::try_from(value).expect("checked integer fits i32"))
        }
        (ExpressionKind::Dimension, ExpressionValue::Number(value)) => InternalValue::Dimension(
            Scaled::from_raw(i32::try_from(value).expect("checked dimension fits i32")),
        ),
        (ExpressionKind::Glue, ExpressionValue::Glue(value)) => {
            InternalValue::Glue(value.into_spec())
        }
        (ExpressionKind::MuGlue, ExpressionValue::Glue(value)) => {
            InternalValue::MuGlue(value.into_spec())
        }
        _ => unreachable!("expression kind fixes its result type"),
    }
}

fn glue_value(value: GlueSpec) -> String {
    format!(
        "width={};stretch={};stretch_order={};shrink={};shrink_order={}",
        value.width.raw(),
        value.stretch.raw(),
        glue_order_name(value.stretch_order),
        value.shrink.raw(),
        glue_order_name(value.shrink_order),
    )
}

#[cfg(test)]
mod tests;
