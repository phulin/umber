//! The command lane: main control's minimal inner loop for its most frequent
//! commands. See `docs/command_lane.md`.
//!
//! The lane is not a second executor. It borrows the admitted run's command
//! processor and settles, in place, only commands whose complete TeX82
//! behavior in an eligible context is already known at delivery. It scans
//! them with the hot scanners and commits them with the hot committers that
//! the admitted run uses. Every other command is left in the episode's
//! command slot exactly as ordinary preflight delivery would leave it, so the
//! admitted run continues with no backup and no second delivery.

use super::*;

mod scalar;

/// The admitted-run facts that make a context lane-eligible. They are
/// sampled once per admitted-loop iteration, and no command the lane settles
/// can change any of them.
pub(super) struct LaneContext {
    pub(super) mode: Mode,
    pub(super) raw_main_loop_delivery: bool,
    pub(super) observing: bool,
    pub(super) tracked_region_is_active: bool,
    pub(super) host_boundary_pending: bool,
    pub(super) leader_pending: bool,
    pub(super) characters_pending: bool,
}

impl LaneContext {
    /// Whether the lane may settle commands before the admitted run's
    /// generic dispatch. Math modes keep their mode-specific scan prelude,
    /// and the character main loop owns its own raw lookahead.
    pub(super) const fn is_eligible(&self) -> bool {
        matches!(
            self.mode,
            Mode::Vertical | Mode::InternalVertical | Mode::Horizontal | Mode::RestrictedHorizontal
        ) && !self.raw_main_loop_delivery
            && !self.observing
            && !self.tracked_region_is_active
            && !self.host_boundary_pending
            && !self.leader_pending
            && !self.characters_pending
    }
}

/// How the lane hands the slot's command back to the admitted run.
pub(super) enum LaneExit<G> {
    /// Ordinary delivery status. A delivered command in the slot has not
    /// been scanned, and the admitted run dispatches it.
    Delivered(tex_command::DeliveryStatus),
    /// §1211's prefix loop consumed `\global` and delivered the slot's
    /// command, which the lane does not own. The admitted run dispatches it
    /// with the accumulated prefix, exactly as its own prefix loop would
    /// continue.
    Prefixed { global: bool },
    /// The slot's lane command has been scanned, but delivery or scanning
    /// left reports the admitted run must publish before it applies.
    Scanned(hot_apply::HotOperation<G>),
    /// The slot's lane command scanned into the admitted run's cold slot, but
    /// delivery or scanning left a report, or its application owes a report,
    /// which the admitted run's direct cold application publishes.
    ScannedCold,
    /// Scanning the slot's lane command failed.
    ScanFailed(ExecError),
    /// The slot's lane command was applied in place. It settles through the
    /// admitted run's continuation predicate, which publishes any report it
    /// left and counts the operation.
    Applied(LaneApplied),
}

/// A lane command applied in place that settles through the admitted run.
pub(super) struct LaneApplied {
    pub(super) result: Result<ReplayStep, ExecError>,
    pub(super) fires_afterassignment: bool,
    pub(super) artifact_count: usize,
    pub(super) effect_count: usize,
    pub(super) save_stack_words: usize,
}

/// The executor owners the lane reads or updates besides the processor.
pub(super) struct LaneOwners<'a, G> {
    pub(super) boxes: &'a ReplayBoxes<G>,
    /// The admitted run's cold slot, where scalar assignments scan.
    pub(super) cold: &'a mut ColdOperationSlot<G>,
    pub(super) max_save_stack: &'a mut usize,
    /// The admitted run's current rollback unit.
    pub(super) operation_mark: &'a mut DirectOperationMark<G>,
}

/// Delivers commands and settles every lane command in place until one the
/// lane does not own, a report, or the operation limit ends the lane.
///
/// Each lane command settled in place settles its own rollback unit, exactly
/// as the admitted run's roll does, so a later discard, whether for a
/// resource suspension or an error, restores only the command that ends the
/// lane.
///
/// `\relax` is TeX82 §1045's `any_mode(relax): do_nothing`. In an eligible
/// context no character run is pending, so its §1030 word boundary flushes
/// nothing, and it reaches no mode, group, or box arm.
pub(super) fn run<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    destination: &mut Option<tex_command::CurrentCommand<G>>,
    operations: &mut usize,
    max_operations: usize,
    owners: &mut LaneOwners<'_, G>,
) -> Result<LaneExit<G>, tex_command::CommandError> {
    if processor.int_param(IntParam::TRACING_COMMANDS) > 0 {
        return processor
            .preflight_command_into(destination)
            .map(LaneExit::Delivered);
    }
    loop {
        let status = processor.preflight_command_into(destination)?;
        if status != tex_command::DeliveryStatus::Command
            || *operations + 1 >= max_operations
            || processor.has_pending_reports()
        {
            return Ok(LaneExit::Delivered(status));
        }
        let command = destination
            .as_ref()
            .expect("command status initializes destination");
        let mut meaning = command.meaning();
        // Errors carry the origin of the command that began the dispatch,
        // which for a prefixed assignment is its first prefix.
        let origin = command.origin();
        let innermost_group = processor.lane_parts().0.innermost_group_kind();
        let mut family = lane_family(meaning, owners.boxes, innermost_group);
        let mut prefixed = false;
        while matches!(family, Some(LaneFamily::Global)) {
            // §1211's `prefixed_command` loop: `repeat get_x_token until
            // (cur_cmd<>spacer)and(cur_cmd<>relax)`, then fold the prefix
            // into a lane assignment or hand the command it fetched to the
            // admitted run's own loop.
            *destination = None;
            let fetched = next_non_blank_non_relax_x_token_into(processor, destination);
            match fetched {
                Ok(tex_command::DeliveryStatus::Command) => {}
                Ok(tex_command::DeliveryStatus::End) => {
                    return Ok(LaneExit::ScanFailed(
                        ExecError::MissingPrefixedCommand.capture_command_origin(origin),
                    ));
                }
                Ok(_) => unreachable!("ordinary expanded delivery returns only commands"),
                Err(error) => {
                    return Ok(LaneExit::ScanFailed(
                        command_error(error).capture_command_origin(origin),
                    ));
                }
            }
            let command = destination
                .as_ref()
                .expect("command status initializes destination");
            meaning = command.meaning();
            prefixed = true;
            family = lane_family(meaning, owners.boxes, innermost_group);
            if processor.has_pending_reports()
                || !matches!(
                    family,
                    Some(LaneFamily::Global | LaneFamily::Assignment | LaneFamily::Scalar)
                )
            {
                return Ok(LaneExit::Prefixed { global: true });
            }
        }
        match family {
            None => return Ok(LaneExit::Delivered(status)),
            Some(LaneFamily::Global) => unreachable!("the prefix loop consumes every prefix"),
            Some(LaneFamily::Relax) => {}
            Some(LaneFamily::EndSimpleGroup) => {
                let operation = hot_apply::HotOperation::end_ordinary_group();
                if let Some(exit) = apply_hot(processor, owners, &operation) {
                    return Ok(exit);
                }
            }
            Some(LaneFamily::Scalar) => {
                // §1214 resolves `\globaldefs` once, before the assignment.
                let global = effective_global(processor.int_param(IntParam::GLOBAL_DEFS), prefixed);
                if let Some(exit) = scalar::run(processor, owners, meaning, origin, global) {
                    return Ok(exit);
                }
            }
            Some(LaneFamily::Scanned | LaneFamily::Assignment) => {
                // §1214 resolves `\globaldefs` once, before the assignment.
                let global = effective_global(
                    processor.int_param(IntParam::GLOBAL_DEFS),
                    prefixed
                        || matches!(
                            meaning,
                            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                                UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
                            ))
                        ),
                );
                let mut scalar = tex_command::ScalarScanFrame::default();
                let operation = match hot_apply::scan(
                    processor,
                    meaning,
                    &mut scalar,
                    global,
                    MeaningFlags::EMPTY,
                    innermost_group,
                ) {
                    Ok(operation) => operation.expect("lane families have hot operations"),
                    Err(error) => {
                        return Ok(LaneExit::ScanFailed(error.capture_command_origin(origin)));
                    }
                };
                if processor.has_pending_reports() || !applies_in_place(&operation) {
                    return Ok(LaneExit::Scanned(operation));
                }
                if let Some(exit) = apply_hot(processor, owners, &operation) {
                    return Ok(exit);
                }
            }
        }
        destination.take();
        *operations += 1;
        settle_unit(processor, owners.operation_mark);
    }
}

#[derive(Clone, Copy)]
enum LaneFamily {
    Relax,
    /// §1211's `\global` prefix. `\long`, `\outer`, and `\protected`
    /// apply only to definitions and stay with the admitted run.
    Global,
    /// A prefixable assignment whose operand the hot scanner reads.
    Assignment,
    /// §1068's `simple_group` arm of `handle_right_brace`, which the
    /// admitted run's dispatch selects without a scanner.
    EndSimpleGroup,
    /// A non-prefixable command whose operand the hot scanner reads.
    Scanned,
    /// A rootless register or parameter assignment, or register arithmetic,
    /// scanned by the admitted run's cold scanners.
    Scalar,
}

/// Classifies a delivered command by the single meaning match of
/// `docs/command_lane.md`. Brace and group commands belong to the lane only
/// where §§1063--1068 reach the ordinary simple or semi-simple group arm,
/// exactly as the admitted run's dispatch selects that arm.
fn lane_family<G>(
    meaning: ResolvedMeaning<G>,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
) -> Option<LaneFamily> {
    let family = match meaning {
        ResolvedMeaning::Static(Meaning::Relax) => LaneFamily::Relax,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global)) => {
            LaneFamily::Global
        }
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Let
            | UnexpandablePrimitive::FutureLet
            | UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef
            | UnexpandablePrimitive::CatCode,
        )) => LaneFamily::Assignment,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::BeginGroup,
        )) => LaneFamily::Scanned,
        ResolvedMeaning::Static(
            Meaning::CountRegister(_)
            | Meaning::DimenRegister(_)
            | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Count
                | UnexpandablePrimitive::Dimen
                | UnexpandablePrimitive::Advance
                | UnexpandablePrimitive::Multiply
                | UnexpandablePrimitive::Divide,
            ),
        ) => LaneFamily::Scalar,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::EndGroup,
        )) if innermost_group == Some(GroupKind::SemiSimple) => LaneFamily::Scanned,
        ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        }) if !boxes.output_routine_opening_pending && !boxes.recovery_simple_group_pending => {
            LaneFamily::Scanned
        }
        ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        }) if innermost_group == Some(GroupKind::Simple) && !boxes.recovery_simple_group_open => {
            LaneFamily::EndSimpleGroup
        }
        _ => return None,
    };
    Some(family)
}

/// Whether the lane commits a scanned operation itself. An invalid category
/// code owes §1232's error and recovery, which the admitted run reports.
const fn applies_in_place<G>(operation: &hot_apply::HotOperation<G>) -> bool {
    match operation {
        hot_apply::HotOperation::CatCode { value, .. } => 0 <= *value && *value <= 15,
        _ => true,
    }
}

/// Applies one scanned hot lane operation in place. Returns `None` when it
/// settled and the lane continues.
fn apply_hot<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    owners: &mut LaneOwners<'_, G>,
    operation: &hot_apply::HotOperation<G>,
) -> Option<LaneExit<G>> {
    match apply(
        processor,
        owners,
        operation.fires_afterassignment(),
        |processor| Some(apply_operation(processor, operation)),
    ) {
        Settlement::Settled => None,
        Settlement::Declined => unreachable!("hot operations always apply"),
        Settlement::HandOff(applied) => Some(LaneExit::Applied(applied)),
    }
}

/// How one lane command's application ended.
enum Settlement {
    /// It settled silently, and the lane continues.
    Settled,
    /// It wrote nothing because its application owes a report the admitted
    /// run publishes.
    Declined,
    /// It failed, or left a report, an effect, an artifact, or a positive
    /// `\tracingcommands` behind, and settles through the admitted run.
    HandOff(LaneApplied),
}

/// Applies one lane command through `commit`, which returns `None` when it
/// declines to write anything.
fn apply<'p, 'q, G>(
    processor: &mut CommandProcessor<'p, 'q, G>,
    owners: &mut LaneOwners<'_, G>,
    fires_afterassignment: bool,
    commit: impl FnOnce(&mut CommandProcessor<'p, 'q, G>) -> Option<Result<ReplayStep, ExecError>>,
) -> Settlement {
    let (artifact_count, effect_count) = {
        let (stores, _, _) = processor.lane_parts();
        (stores.artifact_commit_count(), stores.effect_record_count())
    };
    let Some(result) = commit(processor) else {
        return Settlement::Declined;
    };
    let profile = processor.profile();
    let (stores, command, _) = processor.lane_parts();
    let save_stack_words = MainControl::save_stack_words(stores, owners.boxes, command, profile);
    let settled = result.is_ok()
        && stores.artifact_commit_count() == artifact_count
        && stores.effect_record_count() == effect_count
        && stores.int_param(IntParam::TRACING_COMMANDS) <= 0
        && !processor.has_pending_reports();
    if settled {
        *owners.max_save_stack = (*owners.max_save_stack).max(save_stack_words);
        return Settlement::Settled;
    }
    Settlement::HandOff(LaneApplied {
        result,
        fires_afterassignment,
        artifact_count,
        effect_count,
        save_stack_words,
    })
}

/// The hot appliers' commits, performed through the processor's own
/// borrows. §1269's `\afterassignment` token and §282's `\aftergroup`
/// tokens are backed up through this same processor, so the next lane
/// delivery reads them.
fn apply_operation<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    operation: &hot_apply::HotOperation<G>,
) -> Result<ReplayStep, ExecError> {
    match *operation {
        hot_apply::HotOperation::MacroDefinition {
            target,
            definition,
            flags,
            global,
        } => {
            let (stores, _, diagnostic_effects) = processor.lane_parts();
            hot_apply::commit_macro_definition(
                target,
                definition,
                flags,
                global,
                stores,
                diagnostic_effects,
            );
            schedule_afterassignment(processor)?;
        }
        hot_apply::HotOperation::Let {
            target,
            meaning,
            global,
        } => {
            let (stores, _, diagnostic_effects) = processor.lane_parts();
            hot_apply::commit_let(target, meaning, global, stores, diagnostic_effects)?;
            schedule_afterassignment(processor)?;
        }
        hot_apply::HotOperation::CatCode {
            character,
            value,
            global,
        } => {
            let catcode = hot_apply::catcode_from_value(value)?;
            let (stores, _, diagnostic_effects) = processor.lane_parts();
            hot_apply::commit_catcode(character, catcode, global, stores, diagnostic_effects);
            schedule_afterassignment(processor)?;
        }
        hot_apply::HotOperation::EnterGroup(kind) => {
            let (stores, command, diagnostic_effects) = processor.lane_parts();
            enter_group(stores, command, diagnostic_effects, kind);
        }
        hot_apply::HotOperation::LeaveGroup { kind, context } => {
            let (level, frame) = {
                let frames = processor.lane_parts().0.group_frames();
                (frames.len(), frames.last().copied())
            };
            if let Some(frame) = frame {
                processor.warn_cross_file_group_close(
                    level,
                    frame.kind().group_text(),
                    frame.entered_line(),
                );
            }
            let aftergroup = {
                let (stores, command, diagnostic_effects) = processor.lane_parts();
                leave_group_payloads(stores, command, diagnostic_effects, kind)
                    .map_err(|_| ExecError::MissingToken { context })?
            };
            if !aftergroup.is_empty() {
                processor
                    .back_input_aftergroup_tokens(aftergroup)
                    .map_err(command_error)?;
            }
        }
    }
    Ok(ReplayStep::Continue)
}

/// Settles the lane's rollback unit and opens its successor in place, as the
/// admitted run's roll does. Lane commands never write the mode nest, and a
/// lane run leaves the page list and active boxes untouched, so the state and
/// command-attempt journals are the only ones with a suffix to settle.
fn settle_unit<G>(processor: &mut CommandProcessor<'_, '_, G>, mark: &mut DirectOperationMark<G>) {
    let (stores, command, _) = processor.lane_parts();
    stores.roll_state_operation(&mut mark.state);
    command
        .roll_attempt_operation(&mark.attempt)
        .expect("the lane's unit owns a valid command-attempt scope");
}

pub(super) fn schedule_afterassignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
) -> Result<(), ExecError> {
    let token = {
        let (stores, command, _) = processor.lane_parts();
        command
            .take_afterassignment(stores)
            .expect("afterassignment uses the synchronized command generation")
    };
    if let Some(token) = token {
        processor.back_input_token(token).map_err(command_error)?;
    }
    Ok(())
}
