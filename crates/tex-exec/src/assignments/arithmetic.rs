use super::*;

pub(super) fn arithmetic_i32(
    primitive: UnexpandablePrimitive,
    old: i32,
    rhs: i32,
) -> Result<i32, ExecError> {
    match primitive {
        UnexpandablePrimitive::Advance => old.checked_add(rhs),
        UnexpandablePrimitive::Multiply => old.checked_mul(rhs),
        UnexpandablePrimitive::Divide => {
            if rhs == 0 {
                None
            } else {
                old.checked_div(rhs)
            }
        }
        _ => unreachable!("caller restricts primitive"),
    }
    .ok_or(ExecError::ArithmeticOverflow)
}

#[derive(Clone, Copy, Debug)]
pub(super) enum GlueArithmeticRhs {
    Glue(GlueSpec),
    Factor(i32),
}

pub(super) fn scan_glue_or_factor(
    primitive: UnexpandablePrimitive,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    mu: bool,
    context: TracedTokenWord,
) -> Result<GlueArithmeticRhs, ExecError> {
    match primitive {
        UnexpandablePrimitive::Advance => {
            let id = scan_glue_id(input, stores, execution, mu, context)?;
            Ok(GlueArithmeticRhs::Glue(stores.glue(id)))
        }
        UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => Ok(
            GlueArithmeticRhs::Factor(scan_i32(input, stores, execution, context)?),
        ),
        _ => unreachable!("caller restricts primitive"),
    }
}

pub(super) fn arithmetic_glue(
    primitive: UnexpandablePrimitive,
    old: GlueSpec,
    rhs: GlueArithmeticRhs,
) -> Result<GlueSpec, ExecError> {
    match (primitive, rhs) {
        (UnexpandablePrimitive::Advance, GlueArithmeticRhs::Glue(rhs)) => add_glue(old, rhs),
        (UnexpandablePrimitive::Multiply, GlueArithmeticRhs::Factor(rhs)) => {
            multiply_glue(old, rhs)
        }
        (UnexpandablePrimitive::Divide, GlueArithmeticRhs::Factor(rhs)) => divide_glue(old, rhs),
        _ => unreachable!("caller restricts primitive/rhs"),
    }
}

fn add_glue(left: GlueSpec, right: GlueSpec) -> Result<GlueSpec, ExecError> {
    Ok(GlueSpec {
        width: left
            .width
            .checked_add(right.width)
            .ok_or(ExecError::ArithmeticOverflow)?,
        stretch: add_ordered_component(
            left.stretch,
            left.stretch_order,
            right.stretch,
            right.stretch_order,
        )?
        .0,
        stretch_order: add_ordered_component(
            left.stretch,
            left.stretch_order,
            right.stretch,
            right.stretch_order,
        )?
        .1,
        shrink: add_ordered_component(
            left.shrink,
            left.shrink_order,
            right.shrink,
            right.shrink_order,
        )?
        .0,
        shrink_order: add_ordered_component(
            left.shrink,
            left.shrink_order,
            right.shrink,
            right.shrink_order,
        )?
        .1,
    })
}

fn add_ordered_component(
    left: Scaled,
    left_order: Order,
    right: Scaled,
    right_order: Order,
) -> Result<(Scaled, Order), ExecError> {
    if left_order == right_order {
        return Ok((
            left.checked_add(right)
                .ok_or(ExecError::ArithmeticOverflow)?,
            left_order,
        ));
    }
    if left_order > right_order {
        Ok((left, left_order))
    } else {
        Ok((right, right_order))
    }
}

fn multiply_glue(spec: GlueSpec, factor: i32) -> Result<GlueSpec, ExecError> {
    Ok(GlueSpec {
        width: scaled_checked_mul(spec.width, factor)?,
        stretch: scaled_checked_mul(spec.stretch, factor)?,
        stretch_order: spec.stretch_order,
        shrink: scaled_checked_mul(spec.shrink, factor)?,
        shrink_order: spec.shrink_order,
    })
}

fn divide_glue(spec: GlueSpec, divisor: i32) -> Result<GlueSpec, ExecError> {
    if divisor == 0 {
        return Err(ExecError::ArithmeticOverflow);
    }
    Ok(GlueSpec {
        width: scaled_checked_div(spec.width, divisor)?,
        stretch: scaled_checked_div(spec.stretch, divisor)?,
        stretch_order: spec.stretch_order,
        shrink: scaled_checked_div(spec.shrink, divisor)?,
        shrink_order: spec.shrink_order,
    })
}

pub(super) fn scaled_checked_mul(value: Scaled, factor: i32) -> Result<Scaled, ExecError> {
    value
        .raw()
        .checked_mul(factor)
        .map(Scaled::from_raw)
        .ok_or(ExecError::ArithmeticOverflow)
}

pub(super) fn scaled_checked_div(value: Scaled, divisor: i32) -> Result<Scaled, ExecError> {
    value
        .raw()
        .checked_div(divisor)
        .map(Scaled::from_raw)
        .ok_or(ExecError::ArithmeticOverflow)
}
