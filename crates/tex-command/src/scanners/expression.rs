//! Canonical e-TeX expression and glue-conversion scanners.

use tex_state::GlueId;
use tex_state::glue::{GlueSpec, Order};
use tex_state::meaning::{Meaning, ResolvedMeaning, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::token::Catcode;

use super::scalar::InternalValue;
use crate::observation::canonical_names::glue_order_name;
use crate::processor::{CommandProcessor, DeliveryStatus};
use crate::{
    CommandError, CommandObservation, CurrentCommand, FatalError, ObservationValue, ScannerRecord,
};

const EXPRESSION_DEPTH_LIMIT: u32 = 10_000;
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
    };

    fn from_spec(
        spec: GlueSpec,
        identity: Option<GlueId<G>>,
        source_register: Option<(bool, u16)>,
    ) -> Self {
        Self {
            width: i64::from(spec.width.raw()),
            stretch: i64::from(spec.stretch.raw()),
            stretch_order: spec.stretch_order,
            shrink: i64::from(spec.shrink.raw()),
            shrink_order: spec.shrink_order,
            identity,
            source_register,
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
pub(crate) enum PendingExpressionPhase<G> {
    FactorLeading,
    FactorScalar,
    Operator { factor: ExpressionValue<G> },
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingExpressionScan<G> {
    primitive: UnexpandablePrimitive,
    stack_mark: usize,
    frame: ExpressionFrame<G>,
    overflow: bool,
    phase: PendingExpressionPhase<G>,
}

impl<G> PendingExpressionScan<G> {
    pub(crate) fn stack_mark(&self) -> usize {
        self.stack_mark
    }
}

impl<G> CommandProcessor<'_, '_, G> {
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

        let mut call = crate::scanners::scalar::ScalarCallFrame::default();
        let status = self.scan_expression(primitive, kind, &mut call);
        self.expression_depth -= 1;
        let (mut value, overflow) = match status {
            crate::scanners::scalar::ScalarCallStatus::Complete => call.take_complete(),
            crate::scanners::scalar::ScalarCallStatus::Suspended
            | crate::scanners::scalar::ScalarCallStatus::Failed => {
                return Err(call.take_error());
            }
        };
        if overflow {
            self.expression_arithmetic_error()?;
            value = kind.zero();
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
        let scan = self.scan_glue_retained(scan_mu);
        let value = self.restore_retained_expression_child(scan)?.value;
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
        primitive: UnexpandablePrimitive,
        kind: ExpressionKind,
        call: &mut crate::scanners::scalar::ScalarCallFrame<(ExpressionValue<G>, bool)>,
    ) -> crate::scanners::scalar::ScalarCallStatus {
        use crate::scanners::scalar::ScalarCallStatus;

        macro_rules! fail {
            ($error:expr) => {{
                call.put_error($error);
                return ScalarCallStatus::Failed;
            }};
        }

        let mut pending = None;
        if self.take_pending_scalar_frame_into(&mut pending, call) != ScalarCallStatus::Complete {
            return ScalarCallStatus::Failed;
        }
        let (stack_mark, mut frame, mut overflow, mut phase, mut child) = if pending.is_none() {
            (
                self.command.scratch.expression_stack_len(),
                ExpressionFrame::new(kind),
                false,
                PendingExpressionPhase::FactorLeading,
                None,
            )
        } else {
            match pending.take().expect("present expression continuation") {
                crate::scanners::PendingScalarFrame::Expression { progress, child }
                    if progress.primitive == primitive =>
                {
                    (
                        progress.stack_mark,
                        progress.frame,
                        progress.overflow,
                        progress.phase,
                        child,
                    )
                }
                mut pending => {
                    if let Some(child) = pending.take_child()
                        && let Err(error) = self.abort_continuation(child)
                    {
                        fail!(error);
                    }
                    fail!(CommandError::input_invariant());
                }
            }
        };
        if let Err(error) = self.restore_scalar_child(
            &mut child,
            crate::scanners::ScalarChildDestination::Expression,
        ) {
            fail!(error);
        }

        loop {
            let factor = match phase {
                PendingExpressionPhase::FactorLeading => {
                    let first = loop {
                        let mut first = None;
                        let delivery = match self.get_x_token_into(&mut first) {
                            Ok(delivery) => delivery,
                            Err(error) => {
                                return self.suspend_expression(
                                    error,
                                    PendingExpressionScan {
                                        primitive,
                                        stack_mark,
                                        frame,
                                        overflow,
                                        phase: PendingExpressionPhase::FactorLeading,
                                    },
                                    call,
                                );
                            }
                        };
                        match delivery {
                            DeliveryStatus::End => break None,
                            DeliveryStatus::Command => {
                                let first =
                                    first.expect("command delivery initializes destination");
                                if !matches!(
                                    static_meaning(first.meaning()),
                                    Meaning::CharToken {
                                        cat: Catcode::Space,
                                        ..
                                    }
                                ) {
                                    break Some(first);
                                }
                            }
                            _ => {
                                unreachable!("ordinary expanded delivery returns only commands")
                            }
                        }
                    };
                    if first
                        .as_ref()
                        .is_some_and(|command| is_other_character(command, '('))
                    {
                        let factor_kind = frame.factor_kind();
                        if let Err(error) = self.command.scratch.push_expression_frame(frame) {
                            fail!(crate::scan_toks::scratch_command_error(error));
                        }
                        frame = ExpressionFrame::new(factor_kind);
                        phase = PendingExpressionPhase::FactorLeading;
                        continue;
                    }
                    if let Some(command) = first
                        && let Err(error) = self.back_input(command)
                    {
                        fail!(error);
                    }
                    phase = PendingExpressionPhase::FactorScalar;
                    continue;
                }
                PendingExpressionPhase::FactorScalar => {
                    let factor_kind = frame.factor_kind();
                    let value = match factor_kind {
                        ExpressionKind::Integer => {
                            let result = self.scan_integer_retained();
                            self.restore_retained_expression_child(result)
                                .map(|value| ExpressionValue::Number(i64::from(value.value)))
                        }
                        ExpressionKind::Dimension => {
                            let result = self.scan_dimension_retained();
                            self.restore_retained_expression_child(result)
                                .map(|value| ExpressionValue::Number(i64::from(value.value.raw())))
                        }
                        ExpressionKind::Glue => {
                            let result = self.scan_glue_retained(false);
                            self.restore_retained_expression_child(result).map(|value| {
                                ExpressionValue::Glue(ExpressionGlue::from_spec(
                                    value.value,
                                    self.scanned_glue_identity,
                                    self.scanned_glue_register,
                                ))
                            })
                        }
                        ExpressionKind::MuGlue => {
                            let result = self.scan_glue_retained(true);
                            self.restore_retained_expression_child(result).map(|value| {
                                ExpressionValue::Glue(ExpressionGlue::from_spec(
                                    value.value,
                                    self.scanned_glue_identity,
                                    self.scanned_glue_register,
                                ))
                            })
                        }
                    };
                    match value {
                        Ok(value) => value,
                        Err(error) => {
                            return self.suspend_expression(
                                error,
                                PendingExpressionScan {
                                    primitive,
                                    stack_mark,
                                    frame,
                                    overflow,
                                    phase: PendingExpressionPhase::FactorScalar,
                                },
                                call,
                            );
                        }
                    }
                }
                PendingExpressionPhase::Operator { factor } => factor,
            };

            let parenthesized = self.command.scratch.expression_stack_len() > stack_mark;
            let mut operator = match self.scan_expression_operator(parenthesized) {
                Ok(operator) => operator,
                Err(error) => {
                    return self.suspend_expression(
                        error,
                        PendingExpressionScan {
                            primitive,
                            stack_mark,
                            frame,
                            overflow,
                            phase: PendingExpressionPhase::Operator { factor },
                        },
                        call,
                    );
                }
            };
            let factor = validate_factor(factor, frame.kind, frame.term_operator, &mut overflow);
            operator = apply_factor(&mut frame, factor, operator, &mut overflow);

            if operator.continues_term() {
                frame.term_operator = operator;
                phase = PendingExpressionPhase::FactorLeading;
                continue;
            }
            evaluate_term(&mut frame, operator, &mut overflow);
            if !matches!(operator, ExpressionOperator::None) {
                phase = PendingExpressionPhase::FactorLeading;
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
                call.put_complete((completed, overflow));
                return ScalarCallStatus::Complete;
            };
            frame = parent;
            phase = PendingExpressionPhase::Operator { factor: completed };
        }
    }

    fn restore_retained_expression_child<T>(
        &mut self,
        result: crate::RetainedScalarScan<G, T>,
    ) -> Result<T, CommandError> {
        match result {
            crate::RetainedScalarScan::Complete(value) => Ok(value),
            crate::RetainedScalarScan::Failed(error) => Err(error),
            crate::RetainedScalarScan::Suspended { error, child } => {
                self.install_scanner_resume(Some(child));
                Err(error)
            }
        }
    }

    fn suspend_expression(
        &mut self,
        mut error: CommandError,
        progress: PendingExpressionScan<G>,
        call: &mut crate::scanners::scalar::ScalarCallFrame<(ExpressionValue<G>, bool)>,
    ) -> crate::scanners::scalar::ScalarCallStatus {
        use crate::scanners::scalar::ScalarCallStatus;

        let status = if error.is_resource_suspension() {
            match self.retain_scalar_frame(crate::scanners::PendingScalarFrame::Expression {
                progress,
                child: None,
            }) {
                Ok(()) => ScalarCallStatus::Suspended,
                Err(retain_error) => {
                    error = retain_error;
                    ScalarCallStatus::Failed
                }
            }
        } else {
            if let Err(truncate_error) = self
                .command
                .scratch
                .truncate_expression_stack(progress.stack_mark)
            {
                error = crate::scan_toks::scratch_command_error(truncate_error);
            }
            ScalarCallStatus::Failed
        };
        call.put_error(error);
        status
    }

    fn scan_expression_operator(
        &mut self,
        parenthesized: bool,
    ) -> Result<ExpressionOperator, CommandError> {
        let command = loop {
            let mut command = None;
            let delivery = self.get_x_token_into(&mut command)?;
            let command = match delivery {
                DeliveryStatus::End => return Ok(ExpressionOperator::None),
                DeliveryStatus::Command => {
                    command.expect("command delivery initializes destination")
                }
                _ => unreachable!("ordinary expanded delivery returns only commands"),
            };
            if !matches!(
                static_meaning(command.meaning()),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                break command;
            }
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
        } else if !matches!(static_meaning(command.meaning()), Meaning::Relax) {
            self.back_input(command)?;
        }
        Ok(ExpressionOperator::None)
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

fn is_other_character<G>(command: &CurrentCommand<G>, expected: char) -> bool {
    matches!(
        static_meaning(command.meaning()),
        Meaning::CharToken {
            ch,
            cat: Catcode::Other,
        } if ch == expected
    )
}

fn static_meaning<G>(meaning: ResolvedMeaning<G>) -> Meaning {
    match meaning {
        ResolvedMeaning::Static(meaning) => meaning,
        ResolvedMeaning::Macro { .. } => Meaning::Undefined,
    }
}

fn validate_factor<G>(
    factor: ExpressionValue<G>,
    kind: ExpressionKind,
    term_operator: ExpressionOperator,
    overflow: &mut bool,
) -> ExpressionValue<G> {
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

fn apply_factor<G>(
    frame: &mut ExpressionFrame<G>,
    factor: ExpressionValue<G>,
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
                glue.source_register = None;
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

fn evaluate_term<G>(frame: &mut ExpressionFrame<G>, next: ExpressionOperator, overflow: &mut bool) {
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

fn add_value<G>(
    left: ExpressionValue<G>,
    right: ExpressionValue<G>,
    subtract: bool,
    kind: ExpressionKind,
    overflow: &mut bool,
) -> ExpressionValue<G> {
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
            left.source_register = None;
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

fn multiply_value<G>(
    value: ExpressionValue<G>,
    factor: i64,
    kind: ExpressionKind,
    overflow: &mut bool,
) -> ExpressionValue<G> {
    map_components(value, kind.limit(), overflow, |component, limit| {
        bounded_i128(i128::from(component) * i128::from(factor), limit)
    })
}

fn divide_value<G>(
    value: ExpressionValue<G>,
    divisor: i64,
    kind: ExpressionKind,
    overflow: &mut bool,
) -> ExpressionValue<G> {
    map_components(value, kind.limit(), overflow, |component, limit| {
        rounded_fraction(component, 1, divisor, limit)
    })
}

fn scale_value<G>(
    value: ExpressionValue<G>,
    numerator: i64,
    denominator: i64,
    kind: ExpressionKind,
    overflow: &mut bool,
) -> ExpressionValue<G> {
    map_components(value, kind.limit(), overflow, |component, limit| {
        rounded_fraction(component, numerator, denominator, limit)
    })
}

fn map_components<G>(
    value: ExpressionValue<G>,
    number_limit: i64,
    overflow: &mut bool,
    mut map: impl FnMut(i64, i64) -> Option<i64>,
) -> ExpressionValue<G> {
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
            glue.source_register = None;
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
