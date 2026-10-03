//! Value arithmetic for the canonical e-TeX expression scanner.

use super::*;

pub(super) fn validate_factor<G>(
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

pub(super) fn negate_value<G>(value: ExpressionValue<G>) -> ExpressionValue<G> {
    match value {
        ExpressionValue::Number(value) => ExpressionValue::Number(value.saturating_neg()),
        ExpressionValue::Glue(mut value) => {
            value.width = value.width.saturating_neg();
            value.stretch = value.stretch.saturating_neg();
            value.shrink = value.shrink.saturating_neg();
            value.identity = None;
            value.source_register = None;
            value.shared_zero = false;
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

pub(super) fn apply_factor<G>(
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
                glue.shared_zero = false;
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

pub(super) fn evaluate_term<G>(
    frame: &mut ExpressionFrame<G>,
    next: ExpressionOperator,
    overflow: &mut bool,
) {
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
            left.shared_zero = false;
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
            glue.shared_zero = false;
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
