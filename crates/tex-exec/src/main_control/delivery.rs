//! Command delivery, preflight scanning, and typed retry transitions.

use super::*;

pub(super) struct PreparedAlignmentPreamble<G> {
    pub(super) alignment: AlignmentIdentity,
    pub(super) columns: Vec<PreparedAlignmentCellTemplates<G>>,
    pub(super) tabskips: Vec<tex_state::node::GlueValue>,
    pub(super) default_tabskip: tex_state::node::GlueValue,
    pub(super) repeat_start: Option<usize>,
}

#[derive(Clone, Copy)]
pub(super) enum OffSaveCloser {
    EndGroup,
    MathShift,
    NullRight,
    RightBrace,
}

impl OffSaveCloser {
    pub(super) fn print<G>(self, report: &mut tex_state::print::ErrorReport<'_, G>) {
        match self {
            Self::EndGroup => report.print_esc("endgroup"),
            Self::MathShift => report.print_char('$'),
            Self::NullRight => report.print_esc("right."),
            Self::RightBrace => report.print_char('}'),
        };
    }
}

/// TeX82 §1069's `case cur_group of`: the group opener a stray `}` was
/// probably standing in for.
///
/// This is deliberately not [`OffSaveCloser`]. §1064 inserts a closer and says
/// what it inserted, so its `math_left_group` arm is `\right.` -- a complete
/// command. §1069 deletes the brace and only names what was forgotten, so its
/// arm is the bare `\right`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ForgottenGroupOpener {
    /// `semi_simple_group`.
    EndGroup,
    /// `math_shift_group`.
    MathShift,
    /// `math_left_group`.
    Right,
}

impl ForgottenGroupOpener {
    pub(super) fn print<G>(self, report: &mut tex_state::print::ErrorReport<'_, G>) {
        match self {
            Self::EndGroup => report.print_esc("endgroup"),
            Self::MathShift => report.print_char('$'),
            Self::Right => report.print_esc("right"),
        };
    }
}

/// Selects the one command-owned scanner that may consume input before
/// ordinary main control. Alignment preamble setup validates and backs up its
/// opening brace twice through successive command-owned backup levels; only
/// the second replay reaches TeX82's live preamble scanner.
#[allow(clippy::too_many_arguments)] // owns the replay-only command/input seam
#[derive(Clone, Copy)]
pub(super) enum AlignmentPreamblePhase {
    Opening,
    Start {
        owner: Option<tex_state::interner::Symbol>,
    },
    CellOpening,
    NextCellOpening,
    AlignPeek {
        after_noalign: bool,
    },
    NoAlignBody,
    CellDelivery,
}

pub(super) fn alignment_preamble<G>(
    active: Option<&mut ActiveReplayAlignment<G>>,
) -> Option<(AlignmentIdentity, AlignmentPreamblePhase)> {
    let active = active?;
    if active.preamble_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::Opening))
    } else if active.preamble_start_pending {
        Some((
            active.identity,
            AlignmentPreamblePhase::Start {
                owner: active.owner,
            },
        ))
    } else if active.cell_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::CellOpening))
    } else if active.next_cell_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::NextCellOpening))
    } else if active.align_peek_pending {
        let after_noalign = active.align_peek_after_noalign;
        active.align_peek_after_noalign = false;
        Some((
            active.identity,
            AlignmentPreamblePhase::AlignPeek { after_noalign },
        ))
    } else if active.noalign_open {
        Some((active.identity, AlignmentPreamblePhase::NoAlignBody))
    } else {
        Some((active.identity, AlignmentPreamblePhase::CellDelivery))
    }
}

/// TeX82 §1230's shared `def_code` operand order for every code table.
/// The hot `\catcode` path and the other cold table paths must consume the
/// character selector, optional equals, and signed value identically.
pub(super) fn scan_code_table_operands<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
) -> Result<(char, i32), ExecError> {
    let status =
        processor.scan_restricted_integer_into(RestrictedIntegerClass::CharacterCode, scalar);
    let character = take_operation_scalar!(scalar, status, take_restricted).value;
    let character =
        char::from_u32(character as u32).expect("scan_char_num returns a valid character");
    let status = processor.scan_optional_equals_into(scalar);
    let _ = take_operation_scalar!(scalar, status, take_boolean);
    let status = processor.scan_integer_into(scalar);
    let value = take_operation_scalar!(scalar, status, take_integer).value;
    Ok((character, value))
}

/// Dispatches one already-fetched command through TeX82 §1030's `reswitch:`
/// label and the big case below it.
///
/// This is the shared tail of *every* main-control step, whatever fetched the
/// command: §1030's own `get_x_token`, §1038's `main_loop_lookahead`, or an
/// alignment cell's template-aware delivery. tex.web has no second dispatcher
/// for alignment bodies -- §785's `align_peek` and §1130's `endv` case only
/// bound a cell, and everything between those bounds runs through the same
/// `main_control` big case -- so a caller that reaches `scan_command` without
/// passing through here is dispatching a *narrowed* main control that silently
/// drops whatever this function handles (`umber2-johp.208`).
///
/// Two things are handled here rather than in `scan_command` because tex.web
/// handles them before its big case reaches an assignment:
///
/// - §1211 `prefixed_command`'s `while cur_cmd=prefix` loop. §1210 routes
///   `any_mode(prefix)` -- so `\global`/`\long`/`\outer` (and e-TeX's
///   `\protected`) are prefixes in every mode, never mode-dispatched
///   primitives, and the accumulated `a` is what the assignment cases below
///   consult. Hoisting the loop above `scan_command` keeps that single
///   accumulation point, but only if every dispatch path runs it.
/// - §1045's `any_mode(ignore_spaces): begin <Get the next non-blank non-call
///   token>; goto reswitch; end`.
#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
#[cfg(feature = "profiling")]
pub(super) fn hot_core_command_family<G>(
    meaning: &ResolvedMeaning<G>,
) -> tex_state::measurement::HotCoreCommandFamily {
    use tex_state::measurement::HotCoreCommandFamily as Family;

    match meaning {
        ResolvedMeaning::Static(
            Meaning::CharGiven(_) | Meaning::CharToken { .. } | Meaning::MathCharGiven(_),
        ) => Family::Character,
        ResolvedMeaning::Static(Meaning::Relax) => Family::Relax,
        ResolvedMeaning::Static(Meaning::Undefined) => Family::Undefined,
        ResolvedMeaning::Macro { .. } => Family::Macro,
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(_)) => Family::ExpandablePrimitive,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(_)) => Family::UnexpandablePrimitive,
        ResolvedMeaning::Static(
            Meaning::CountRegister(_)
            | Meaning::DimenRegister(_)
            | Meaning::SkipRegister(_)
            | Meaning::MuskipRegister(_)
            | Meaning::ToksRegister(_)
            | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_),
        ) => Family::RegisterOrParameter,
        ResolvedMeaning::Static(Meaning::Font(_)) => Family::Font,
        ResolvedMeaning::Static(Meaning::InternalInteger(_)) => Family::InternalQuantity,
        ResolvedMeaning::Static(Meaning::EndV) => Family::EndTemplate,
        ResolvedMeaning::Static(Meaning::Unknown(_)) => Family::Unknown,
    }
}

#[cfg(feature = "profiling")]
pub(super) fn hot_core_meaning_family<G>(
    meaning: &ResolvedMeaning<G>,
) -> tex_state::measurement::HotCoreMeaningFamily {
    use tex_state::measurement::HotCoreMeaningFamily as Family;

    match meaning {
        ResolvedMeaning::Static(Meaning::Undefined) => Family::Undefined,
        ResolvedMeaning::Static(Meaning::Relax) => Family::Relax,
        ResolvedMeaning::Macro { .. } => Family::Macro,
        ResolvedMeaning::Static(Meaning::CharGiven(_)) => Family::CharGiven,
        ResolvedMeaning::Static(Meaning::CharToken { .. }) => Family::CharToken,
        ResolvedMeaning::Static(Meaning::MathCharGiven(_)) => Family::MathCharGiven,
        ResolvedMeaning::Static(Meaning::CountRegister(_)) => Family::CountRegister,
        ResolvedMeaning::Static(Meaning::DimenRegister(_)) => Family::DimenRegister,
        ResolvedMeaning::Static(Meaning::SkipRegister(_)) => Family::SkipRegister,
        ResolvedMeaning::Static(Meaning::MuskipRegister(_)) => Family::MuskipRegister,
        ResolvedMeaning::Static(Meaning::ToksRegister(_)) => Family::ToksRegister,
        ResolvedMeaning::Static(Meaning::IntParam(_)) => Family::IntParam,
        ResolvedMeaning::Static(Meaning::DimenParam(_)) => Family::DimenParam,
        ResolvedMeaning::Static(Meaning::GlueParam(_)) => Family::GlueParam,
        ResolvedMeaning::Static(Meaning::MuGlueParam(_)) => Family::MuGlueParam,
        ResolvedMeaning::Static(Meaning::TokParam(_)) => Family::TokParam,
        ResolvedMeaning::Static(Meaning::PageDimension(_)) => Family::PageDimension,
        ResolvedMeaning::Static(Meaning::PageInteger(_)) => Family::PageInteger,
        ResolvedMeaning::Static(Meaning::InternalInteger(_)) => Family::InternalInteger,
        ResolvedMeaning::Static(Meaning::Font(_)) => Family::Font,
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(_)) => Family::ExpandablePrimitive,
        ResolvedMeaning::Static(Meaning::EndV) => Family::EndV,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(_)) => Family::UnexpandablePrimitive,
        ResolvedMeaning::Static(Meaning::Unknown(_)) => Family::Unknown,
    }
}

/// TeX82 §1030's `if tracing_commands>0 then show_cur_cmd_chr` at `reswitch`,
/// reached after `big_switch` and after cases such as §1045 `ignore_spaces`
/// fetch a replacement command. §1211's prefix loop does not return to that
/// label, so its internal fetches remain untraced.
pub(super) fn report_command_trace<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    command: &tex_command::CurrentCommand<G>,
    shown_mode: &mut Option<Mode>,
) {
    if processor.int_param(IntParam::TRACING_COMMANDS) > 0 {
        *shown_mode = Some(mode);
        processor.print_command_trace(tex_command::PrintCommand::from_current(command));
    }
}

/// Applies TeX82's §1030 main-control trace boundary to a fetched command.
///
/// A constructed leader payload is one exception: after its box closes,
/// §1078's `box_end` fetches the following glue inside the leader case and
/// never returns to `big_switch`. The split replay lifecycle leaves that
/// internal fetch to the next processor episode, so `pending_leader` retains
/// the canonical boundary distinction and suppresses only §1030's settled
/// unexpandable-command trace. Expansion tracing performed by `get_x_token`
/// remains unchanged.
///
/// The opening brace of an output routine is the other exception. TeX82
/// §1025 consumes it with `scan_left_brace` before entering §1030, whereas
/// split replay delivers it as an explicit step. Suppressing that delivery
/// also leaves the mode prefix pending for the first command in the routine.
///
/// A `\shipout` box constructor is likewise scanner-owned: §§1075/1084 call
/// `scan_box` from the already-traced `leader_ship` case, so its `\hbox`,
/// `\vbox`, or `\vtop` never returns to §1030's `reswitch`. Split replay
/// retains `pending_shipout` across that internal fetch.
pub(super) fn report_main_control_command_trace<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    command: &tex_command::CurrentCommand<G>,
    boxes: &ReplayBoxes<G>,
    shown_mode: &mut Option<Mode>,
) {
    // Expansion can invoke §299 itself for e-TeX's `\tracingifs`. Its mode
    // prefix and `shown_mode` transition precede this settled command.
    if processor.command_trace_printed() {
        *shown_mode = Some(mode);
    }
    let output_routine_opening = boxes.output_routine_opening_pending
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            })
        );
    let shipout_box_constructor = boxes.pending_shipout.is_some()
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::HBox
                    | UnexpandablePrimitive::VBox
                    | UnexpandablePrimitive::VTop
            ))
        );
    if boxes.pending_leader.is_none() && !output_routine_opening && !shipout_box_constructor {
        report_command_trace(processor, mode, command, shown_mode);
    }
}

pub(super) fn prepare_command_trace<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    shown_mode: Option<Mode>,
) {
    // The mode text is owned because expansion may retain it until a later
    // command in this processor episode.  Do not allocate that ownership when
    // neither command nor conditional tracing can consume the prefix: this is
    // the ordinary production path and this boundary is crossed once per
    // main-control operation.
    let tracing_can_consume_prefix = processor.int_param(IntParam::TRACING_COMMANDS) > 0
        || processor.int_param(IntParam::TRACING_IFS) > 0;
    let mode_prefix = (tracing_can_consume_prefix && shown_mode != Some(mode))
        .then(|| mode_text_for_command_trace(mode).into());
    processor.set_command_trace_mode_prefix(mode_prefix);
}

pub(super) fn retain_operation_child<G, T>(
    _processor: &mut CommandProcessor<'_, '_, G>,
    scan: tex_command::RetainedScalarScan<T>,
) -> Result<T, ExecError> {
    match scan {
        tex_command::RetainedScalarScan::Complete(value) => Ok(value),
        tex_command::RetainedScalarScan::Failed(error) => Err(command_error(error)),
    }
}
