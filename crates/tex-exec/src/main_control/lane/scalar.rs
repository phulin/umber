//! The command lane's scalar family: §1224--§1238's rootless register and
//! parameter assignments and §1235's register arithmetic.
//!
//! These scan through the admitted run's own cold scanners into its cold
//! slot, so the lane shares every operand rule with the admitted dispatch.
//! The lane then commits the scanned value through the scalar committer the
//! admitted run's cold arms use, instead of the generic cold application.

use super::*;

/// Scans and applies one scalar lane command. Returns `None` when it settled
/// and the lane continues.
pub(super) fn run<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    owners: &mut LaneOwners<'_, G>,
    meaning: ResolvedMeaning<G>,
    origin: tex_state::token::OriginId,
    global: bool,
) -> Option<LaneExit<G>> {
    let mut scalar = tex_command::ScalarScanFrame::default();
    if let Err(error) = scan(owners.cold, processor, &mut scalar, meaning, origin, global) {
        return Some(LaneExit::ScanFailed(error.capture_command_origin(origin)));
    }
    let operation = owners
        .cold
        .operation
        .take()
        .expect("a completed scalar scan fills the cold slot");
    // §1236's invalid arithmetic target owes its error, as does a report
    // left by delivery or scanning.
    if processor.has_pending_reports()
        || matches!(operation, ColdOperation::InvalidArithmeticTarget { .. })
    {
        owners.cold.operation = Some(operation);
        return Some(LaneExit::ScannedCold);
    }
    match apply(processor, owners, true, |processor| {
        commit(processor, &operation)
    }) {
        Settlement::Settled => None,
        Settlement::Declined => {
            owners.cold.operation = Some(operation);
            Some(LaneExit::ScannedCold)
        }
        Settlement::HandOff(applied) => Some(LaneExit::Applied(applied)),
    }
}

/// The admitted dispatcher's scan arms for the scalar lane family.
fn scan<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    meaning: ResolvedMeaning<G>,
    origin: tex_state::token::OriginId,
    global: bool,
) -> Result<(), ExecError> {
    match static_meaning(meaning) {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => {
            scan_count_register_assignment(cold, processor, scalar, None, global)
        }
        Meaning::CountRegister(index) => {
            scan_count_register_assignment(cold, processor, scalar, Some(index), global)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            scan_dimension_register_assignment(cold, processor, scalar, None, global)
        }
        Meaning::DimenRegister(index) => {
            scan_dimension_register_assignment(cold, processor, scalar, Some(index), global)
        }
        meaning @ (Meaning::IntParam(_) | Meaning::DimenParam(_)) => {
            scan_unary_scalar_operation(cold, processor, scalar, meaning, global, origin)
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Advance
            | UnexpandablePrimitive::Multiply
            | UnexpandablePrimitive::Divide),
        ) => scan_arithmetic_assignment(cold, processor, scalar, primitive, global),
        _ => unreachable!("the scalar lane family restricts command meanings"),
    }
}

/// Commits a scanned scalar assignment and schedules §1269's
/// `\afterassignment` token. Declines an arithmetic overflow, which §1236
/// reports before `word_define`, so the target is never written.
fn commit<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    operation: &ColdOperation<G>,
) -> Option<Result<ReplayStep, ExecError>> {
    let profile = processor.profile();
    let (stores, _, diagnostic_effects) = processor.lane_parts();
    if let ColdOperation::Arithmetic {
        primitive,
        target,
        operand,
        global,
    } = *operation
    {
        match apply_arithmetic(
            primitive,
            target,
            operand,
            global,
            profile,
            stores,
            diagnostic_effects,
        ) {
            Ok(_) => {}
            Err(ExecError::ArithmeticOverflow) => return None,
            Err(error) => return Some(Err(error)),
        }
    } else {
        let _ = commit_scalar_assignment(operation, profile, stores, diagnostic_effects)
            .expect("the scalar lane family scans only scalar assignments");
    }
    Some(schedule_afterassignment(processor).map(|()| ReplayStep::Continue))
}
