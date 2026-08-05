//! Exhaustive primitive observation identity generated from shared metadata.

use crate::CommandDialect;
use tex_state::meaning::{ExpandablePrimitive, UnexpandablePrimitive};

pub(crate) fn unexpandable_primitive_identity(
    dialect: CommandDialect,
    primitive: UnexpandablePrimitive,
) -> (String, Option<i64>) {
    let (command, operand) = crate::primitives::metadata::unexpandable_identity(dialect, primitive);
    (command.to_owned(), operand)
}

pub(crate) fn expandable_primitive_identity(
    dialect: CommandDialect,
    primitive: ExpandablePrimitive,
) -> (String, Option<i64>) {
    let (command, operand) = crate::primitives::metadata::expandable_identity(dialect, primitive);
    (command.to_owned(), operand)
}
