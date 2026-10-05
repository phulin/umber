//! Canonical e-TeX expression and glue-conversion scanners.

use tex_state::GlueId;
use tex_state::glue::{GlueSpec, Order};
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::token::Catcode;

use super::scalar::InternalValue;
use crate::observation::canonical_names::glue_order_name;
use crate::processor::CommandProcessor;
use crate::{CommandError, CommandObservation, ObservationValue, ScannerRecord};

mod arithmetic;
use arithmetic::{apply_factor, evaluate_term, negate_value, validate_factor};

const INTEGER_LIMIT: i64 = i32::MAX as i64;
const DIMENSION_LIMIT: i64 = Scaled::MAX_DIMEN.raw() as i64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExpressionKind {
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

    fn zero<G>(self) -> ExpressionValue<G> {
        match self {
            Self::Integer | Self::Dimension => ExpressionValue::Number(0),
            Self::Glue | Self::MuGlue => ExpressionValue::Glue(ExpressionGlue::ZERO),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExpressionGlue<G> {
    width: i64,
    stretch: i64,
    stretch_order: Order,
    shrink: i64,
    shrink_order: Order,
    identity: Option<GlueId<G>>,
    source_register: Option<(bool, u16)>,
    shared_zero: bool,
}

impl<G> Clone for ExpressionGlue<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for ExpressionGlue<G> {}

impl<G> ExpressionGlue<G> {
    const ZERO: Self = Self {
        width: 0,
        stretch: 0,
        stretch_order: Order::Normal,
        shrink: 0,
        shrink_order: Order::Normal,
        identity: None,
        source_register: None,
        shared_zero: false,
    };

    fn from_spec(
        spec: GlueSpec,
        identity: Option<GlueId<G>>,
        source_register: Option<(bool, u16)>,
        shared_zero: bool,
    ) -> Self {
        Self {
            width: i64::from(spec.width.raw()),
            stretch: i64::from(spec.stretch.raw()),
            stretch_order: spec.stretch_order,
            shrink: i64::from(spec.shrink.raw()),
            shrink_order: spec.shrink_order,
            identity,
            source_register,
            shared_zero,
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

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ExpressionValue<G> {
    Number(i64),
    Glue(ExpressionGlue<G>),
}

impl<G> Clone for ExpressionValue<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for ExpressionValue<G> {}

impl<G> ExpressionValue<G> {
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

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExpressionFrame<G> {
    kind: ExpressionKind,
    expression_operator: ExpressionOperator,
    expression: ExpressionValue<G>,
    term_operator: ExpressionOperator,
    term: ExpressionValue<G>,
    scale_numerator: i64,
    factor_negative: bool,
}

impl<G> Clone for ExpressionFrame<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for ExpressionFrame<G> {}

impl<G> ExpressionFrame<G> {
    fn new(kind: ExpressionKind) -> Self {
        Self {
            kind,
            expression_operator: ExpressionOperator::None,
            expression: kind.zero(),
            term_operator: ExpressionOperator::None,
            term: kind.zero(),
            scale_numerator: 0,
            factor_negative: false,
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

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ExpressionPhase<G> {
    FactorLeading,
    FactorScalar,
    Operator { factor: ExpressionValue<G> },
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// e-TeX 2.6 `scan_expr`, from the already-delivered expression primitive
    /// through its committed typed result (etex.ch [53a.4945--5360]).
    pub(crate) fn scan_expression_primitive(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<InternalValue, CommandError> {
        let kind = ExpressionKind::of_primitive(primitive)
            .expect("only e-TeX expression primitives reach scan_expr");
        let (mut value, overflow) =
            self.with_expansion_depth(|processor| processor.scan_expression(kind))?;
        if overflow {
            self.expression_arithmetic_error()?;
            value = kind.zero();
            // pdftex.web scan_expr returns the shared zero_glue pointer after
            // a glue-expression arithmetic error, not a fresh zero spec.
            if let ExpressionValue::Glue(glue) = &mut value {
                glue.shared_zero = true;
            }
        }
        self.observe_expression(kind, value);
        self.scanned_glue_identity = match value {
            ExpressionValue::Glue(value) => value.identity,
            ExpressionValue::Number(_) => None,
        };
        self.scanned_glue_register = match value {
            ExpressionValue::Glue(value) => value.source_register,
            ExpressionValue::Number(_) => None,
        };
        self.scanned_glue_shared_zero = match value {
            ExpressionValue::Glue(value) => value.shared_zero,
            ExpressionValue::Number(_) => false,
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
        let scan = self.scan_glue(scan_mu);
        let value = scan?;
        self.observe(CommandObservation::Scanner(ScannerRecord {
            kind: scanner,
            value: glue_value(value),
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
    ) -> Result<(ExpressionValue<G>, bool), CommandError> {
        macro_rules! fail {
            ($error:expr) => {{
                return Err($error);
            }};
        }

        let stack_mark = self.command.scratch.expression_stack_len();
        let mut frame = ExpressionFrame::new(kind);
        let mut overflow = false;
        let mut phase = ExpressionPhase::FactorLeading;

        loop {
            let factor = match phase {
                ExpressionPhase::FactorLeading => {
                    let mut parenthesis = false;
                    let probe =
                        self.consume_expanded_words(&mut crate::terminal_consumer::<Self, G, _>(
                            |processor, spelling, meaning, _code, _, exceptional| {
                                let character = meaning
                                    .scalar_word()
                                    .and_then(Meaning::runtime_word_character);
                                if character.is_some_and(|(_, cat)| cat == Catcode::Space) {
                                    return Ok(std::ops::ControlFlow::Continue(()));
                                }
                                parenthesis = character == Some(('(', Catcode::Other));
                                if !parenthesis {
                                    processor.back_consumed_word(spelling, exceptional)?;
                                }
                                Ok(std::ops::ControlFlow::Break(()))
                            },
                        ));
                    if let Err(error) = probe {
                        return Err(self.expression_scan_error(error, stack_mark));
                    }
                    if parenthesis {
                        let factor_kind = frame.factor_kind();
                        if let Err(error) = self.command.scratch.push_expression_frame(frame) {
                            fail!(crate::scan_toks::scratch_command_error(error));
                        }
                        frame = ExpressionFrame::new(factor_kind);
                        phase = ExpressionPhase::FactorLeading;
                        continue;
                    }
                    phase = ExpressionPhase::FactorScalar;
                    continue;
                }
                ExpressionPhase::FactorScalar => {
                    let factor_kind = frame.factor_kind();
                    let value = match factor_kind {
                        ExpressionKind::Integer => {
                            let result = self.scan_integer();
                            result.map(|value| ExpressionValue::Number(i64::from(value.value)))
                        }
                        ExpressionKind::Dimension => {
                            let result = self.scan_dimension();
                            result.map(|value| ExpressionValue::Number(i64::from(value.raw())))
                        }
                        ExpressionKind::Glue => {
                            let result = self.scan_glue(false);
                            result.map(|value| {
                                ExpressionValue::Glue(ExpressionGlue::from_spec(
                                    value,
                                    self.scanned_glue_identity,
                                    self.scanned_glue_register,
                                    self.scanned_glue_shared_zero,
                                ))
                            })
                        }
                        ExpressionKind::MuGlue => {
                            let result = self.scan_glue(true);
                            result.map(|value| {
                                ExpressionValue::Glue(ExpressionGlue::from_spec(
                                    value,
                                    self.scanned_glue_identity,
                                    self.scanned_glue_register,
                                    self.scanned_glue_shared_zero,
                                ))
                            })
                        }
                    };
                    match value {
                        Ok(value) => value,
                        Err(error) => {
                            return Err(self.expression_scan_error(error, stack_mark));
                        }
                    }
                }
                ExpressionPhase::Operator { factor } => factor,
            };

            let parenthesized = self.command.scratch.expression_stack_len() > stack_mark;
            let mut operator = match self.scan_expression_operator(parenthesized) {
                Ok(operator) => operator,
                Err(error) => {
                    return Err(self.expression_scan_error(error, stack_mark));
                }
            };
            let factor = if frame.factor_negative {
                frame.factor_negative = false;
                negate_value(factor)
            } else {
                factor
            };
            let factor = validate_factor(factor, frame.kind, frame.term_operator, &mut overflow);
            operator = apply_factor(&mut frame, factor, operator, &mut overflow);

            if operator.continues_term() {
                frame.term_operator = operator;
                phase = ExpressionPhase::FactorLeading;
                continue;
            }
            evaluate_term(&mut frame, operator, &mut overflow);
            if !matches!(operator, ExpressionOperator::None) {
                phase = ExpressionPhase::FactorLeading;
                continue;
            }

            let completed = frame.expression;
            let parent = match self.command.scratch.pop_expression_frame(stack_mark) {
                Ok(parent) => parent,
                Err(error) => fail!(crate::scan_toks::scratch_command_error(error)),
            };
            let Some(parent) = parent else {
                if let Err(error) = self.command.scratch.truncate_expression_stack(stack_mark) {
                    fail!(crate::scan_toks::scratch_command_error(error));
                }
                return Ok((completed, overflow));
            };
            frame = parent;
            phase = ExpressionPhase::Operator { factor: completed };
        }
    }

    fn expression_scan_error(
        &mut self,
        mut error: CommandError,
        stack_mark: usize,
    ) -> CommandError {
        if let Err(truncate_error) = self.command.scratch.truncate_expression_stack(stack_mark) {
            error = crate::scan_toks::scratch_command_error(truncate_error);
        }
        error
    }

    fn scan_expression_operator(
        &mut self,
        parenthesized: bool,
    ) -> Result<ExpressionOperator, CommandError> {
        let mut operator = ExpressionOperator::None;
        self.consume_expanded_words(&mut crate::terminal_consumer::<Self, G, _>(
            |processor, spelling, meaning, _code, _, exceptional| {
                let character = meaning
                    .scalar_word()
                    .and_then(Meaning::runtime_word_character);
                if character.is_some_and(|(_, cat)| cat == Catcode::Space) {
                    return Ok(std::ops::ControlFlow::Continue(()));
                }
                operator = match character {
                    Some(('+', Catcode::Other)) => ExpressionOperator::Add,
                    Some(('-', Catcode::Other)) => ExpressionOperator::Subtract,
                    Some(('*', Catcode::Other)) => ExpressionOperator::Multiply,
                    Some(('/', Catcode::Other)) => ExpressionOperator::Divide,
                    _ => ExpressionOperator::None,
                };
                if matches!(operator, ExpressionOperator::None) {
                    if parenthesized {
                        if character != Some((')', Catcode::Other)) {
                            processor.back_consumed_word(spelling, exceptional)?;
                            processor.missing_expression_parenthesis_error()?;
                        }
                    } else if meaning.scalar_word() != Some(Meaning::RELAX_WORD) {
                        processor.back_consumed_word(spelling, exceptional)?;
                    }
                }
                Ok(std::ops::ControlFlow::Break(()))
            },
        ))?;
        Ok(operator)
    }

    fn missing_expression_parenthesis_error(&mut self) -> Result<(), CommandError> {
        // e-TeX \[26.1576] reaches `back_error`, so the caller has already
        // restored the rejected token for §314 to name.
        let context = self.command.output_open_context(self.state);
        let mut report = self.state.print_err("Missing ) inserted for expression");
        report
            .help(&["I was expecting to see `+', `-', `*', `/', or `)'. Didn't."])
            .context(context);
        let outcome = report.error();
        self.finish_error_outcome(outcome)?;
        Ok(())
    }

    fn expression_arithmetic_error(&mut self) -> Result<(), CommandError> {
        let context = self.command.output_open_context(self.state);
        let mut report = self.state.print_err("Arithmetic overflow");
        report
            .help(&[
                "I can't evaluate this expression,",
                "since the result is out of range.",
            ])
            .context(context);
        let outcome = report.error();
        self.finish_error_outcome(outcome)?;
        Ok(())
    }

    fn observe_expression(&mut self, kind: ExpressionKind, value: ExpressionValue<G>) {
        let value = match value {
            ExpressionValue::Number(value) => match kind {
                ExpressionKind::Integer => ObservationValue::Integer(value),
                ExpressionKind::Dimension => ObservationValue::Scaled(value),
                _ => unreachable!("numeric expressions are integer or dimension values"),
            },
            ExpressionValue::Glue(value) => glue_value(value.into_spec()),
        };
        self.observe(CommandObservation::Scanner(ScannerRecord {
            kind: kind.scanner_name(),
            value,
        }));
    }
}

fn expression_internal_value<G>(kind: ExpressionKind, value: ExpressionValue<G>) -> InternalValue {
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

fn glue_value(value: GlueSpec) -> ObservationValue {
    ObservationValue::Glue {
        width: i64::from(value.width.raw()),
        stretch: i64::from(value.stretch.raw()),
        stretch_order: glue_order_name(value.stretch_order),
        shrink: i64::from(value.shrink.raw()),
        shrink_order: glue_order_name(value.shrink_order),
    }
}

#[cfg(test)]
mod tests;
