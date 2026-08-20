//! Shared cold semantic helpers for boxes, arithmetic, diagnostics, and hooks.

use super::super::*;
use super::apply::enter_group;
use super::operation::*;
use super::pdf::*;

/// TeX82 §1370's printable-sink framing for an immediate `\write`.
///
/// A closed numbered stream (including stream 16) temporarily becomes a
/// normal print selector: `print_nl(""); token_show(...); print_ln`. Going
/// through [`tex_state::print::Printer`] is essential here because the
/// process may select a `max_print_line` other than tex.web's compile-time default, and
/// because the leading `print_nl` owns the break after a preceding
/// newline-less `\message`. Real output files have neither print columns nor
/// that leading break.
pub(in crate::main_control) fn write_immediate_text(
    stores: &mut Universe,
    sink: PrintSink,
    text: &str,
) {
    let selector = match sink {
        PrintSink::Terminal => tex_state::print::Selector::TermOnly,
        PrintSink::Log => tex_state::print::Selector::LogOnly,
        PrintSink::TerminalAndLog => tex_state::print::Selector::TermAndLog,
        PrintSink::Stream(_) => {
            stores.world_mut().write_text(sink, text);
            return;
        }
    };
    let line_is_open = {
        let bufs = stores.world().stream_bufs();
        let terminal = !bufs.terminal_partial_line().is_empty();
        let log = !bufs.log_partial_line().is_empty();
        match selector {
            tex_state::print::Selector::TermOnly => terminal,
            tex_state::print::Selector::LogOnly => log,
            tex_state::print::Selector::TermAndLog => terminal || log,
            tex_state::print::Selector::NoPrint => false,
        }
    };
    let mut printer = tex_state::print::Printer::new(stores, selector);
    if line_is_open {
        printer.print_ln();
    }
    printer.print_rendered(text);
}

pub(in crate::main_control) fn print_display_content(stores: &mut Universe, content: &str) {
    stores.printer().print_nl("").print_rendered(content);
}

/// TeX82 §282's `insert_token` arm, the only way an `\aftergroup` token ever
/// re-enters the input, plus e-TeX 2.6 etex.ch [15.282]'s optimized form.
///
/// §282 is `unsave`'s `@<Clear off top level from |save_stack|@>`: it walks
/// the level downwards and, for every `insert_token` entry, runs
/// §326 `@<Insert token |p| into \TeX's input@>`. TeX82 applies one full
/// `back_input` per token. In extended mode e-TeX applies that full operation
/// only to the first token and links every remaining token directly onto the
/// resulting `backed_up` list.
///
/// Because §282 clears the level from the top down while `\aftergroup` saved
/// from the bottom up, the last-saved token is backed up first and ends up
/// deepest, so rereading restores save order. `Universe` hands the payload
/// over in save order, so backing it up in reverse reproduces both the input
/// structure and the order `unsave` observes it in.
pub(in crate::main_control) fn schedule_aftergroup(
    command: &mut CommandMachine<'_>,
    stores: &mut Universe,
    tokens: Vec<tex_state::token::RootedTracedTokenWord>,
) -> Result<(), ExecError> {
    if tokens.is_empty() {
        return Ok(());
    }
    let traced: Vec<_> = tokens
        .into_iter()
        .map(|spelling| {
            let (spelling, parent) = spelling.into_parts();
            let token = spelling.semantic_token();
            let origin = stores.inserted_origin(
                tex_state::provenance::InsertedOriginKind::AfterGroup,
                token,
                parent.id(),
            );
            tex_state::token::RootedTracedTokenWord::new(
                token,
                tex_state::provenance::OriginRef::direct(origin),
            )
        })
        .collect::<Vec<_>>();
    command
        .processor(stores)
        .back_input_aftergroup_tokens(traced)
        .map_err(command_error)
}

pub(in crate::main_control) fn warn_cross_file_group_close(
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) {
    let level = stores.group_depth() as usize;
    let Some(frame) = stores.group_frames().next_back() else {
        return;
    };
    command.processor(stores).warn_cross_file_group_close(
        level,
        frame.kind().group_text(),
        frame.entered_line(),
    );
}

/// Releases the single pending after-assignment token only after the typed
/// assignment has committed. TeX82 §1269 assigns it to `cur_tok` and invokes
/// §325 `back_input`, so it must use the ordinary canonical backup level.
pub(in crate::main_control) fn schedule_afterassignment(
    command: &mut PersistentInterpreter,
    fuel: &mut tex_command::CommandFuel,
    capabilities: &mut CommandHostCapabilities,
    observations: &mut ObservationSlot,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let Some(token) = stores.take_afterassignment() else {
        return Ok(());
    };
    let origin = stores.inserted_origin(
        tex_state::provenance::InsertedOriginKind::AfterAssignment,
        token,
        tex_state::token::OriginId::UNKNOWN,
    );
    let mut processor = command_processor(command, fuel, capabilities, observations, stores);
    let result = processor.back_input_token(tex_state::token::TracedTokenWord::pack(token, origin));
    result.map_err(command_error)
}

/// Applies TeX82 §1214's `\globaldefs` override to a prefixed assignment's
/// scope: a positive `\globaldefs` forces `global_defs`, a negative one forces
/// local scope, and zero leaves the `\global` prefix in charge.
pub(in crate::main_control) fn effective_global(global_defs: i32, explicit_global: bool) -> bool {
    match global_defs.cmp(&0) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => explicit_global,
    }
}

pub(in crate::main_control) fn checked_character_code(
    value: i32,
    context: &'static str,
) -> Result<u32, ExecError> {
    u32::try_from(value)
        .ok()
        .and_then(char::from_u32)
        .map(|character| character as u32)
        .ok_or(ExecError::InvalidCode { context, value })
}

pub(in crate::main_control) fn code_table_mutation(
    table: &str,
    character: char,
    value: i64,
    global: bool,
) -> MutationRecord {
    MutationRecord {
        target: MutationTarget::CodeTable,
        key: ObservationValue::Name(format!("{table}:{}", u32::from(character))),
        value: ObservationValue::Integer(value),
        global,
    }
}

pub(in crate::main_control) fn font_definition_mutation(
    stores: &Universe,
    target: Symbol,
    global: bool,
    observed: bool,
) -> Option<MutationRecord> {
    if observed {
        Some(MutationRecord {
            target: MutationTarget::Meaning,
            key: ObservationValue::Name(stores.resolve(target).to_owned()),
            value: ObservationValue::Name("set_font".into()),
            global,
        })
    } else {
        None
    }
}

pub(in crate::main_control) fn pdf_font_code_table(
    primitive: UnexpandablePrimitive,
) -> tex_state::PdfFontCode {
    match primitive {
        UnexpandablePrimitive::PdfLpCode => tex_state::PdfFontCode::Lp,
        UnexpandablePrimitive::PdfRpCode => tex_state::PdfFontCode::Rp,
        UnexpandablePrimitive::PdfEfCode => tex_state::PdfFontCode::Ef,
        UnexpandablePrimitive::PdfTagCode => tex_state::PdfFontCode::Tag,
        UnexpandablePrimitive::PdfKnbsCode => tex_state::PdfFontCode::Knbs,
        UnexpandablePrimitive::PdfStbsCode => tex_state::PdfFontCode::Stbs,
        UnexpandablePrimitive::PdfShbsCode => tex_state::PdfFontCode::Shbs,
        UnexpandablePrimitive::PdfKnbcCode => tex_state::PdfFontCode::Knbc,
        UnexpandablePrimitive::PdfKnacCode => tex_state::PdfFontCode::Knac,
        _ => unreachable!("caller restricts pdfTeX font-code primitives"),
    }
}

pub(in crate::main_control) fn apply_arithmetic(
    primitive: UnexpandablePrimitive,
    target: ArithmeticTarget,
    operand: ArithmeticOperand,
    global: bool,
    profile: CommandProfile,
    stores: &mut Universe,
) -> Result<crate::assignments::committer::MutationReceipt, ExecError> {
    let receipt = match (target, operand) {
        (ArithmeticTarget::IntegerRegister(index), ArithmeticOperand::Integer(rhs)) => {
            let value = arithmetic_integer(primitive, stores.count(index), rhs)?;
            AssignmentCommitter::new(stores).count(index, value, global)
        }
        (ArithmeticTarget::IntegerParameter(index), ArithmeticOperand::Integer(rhs)) => {
            let value = arithmetic_integer(primitive, stores.int_param(IntParam::new(index)), rhs)?;
            let key = parameter_mutation_key_for_dialect(
                profile.dialect(),
                ParameterClass::Integer,
                index,
            );
            AssignmentCommitter::new(stores).int_parameter(index, value, key, global)
        }
        (ArithmeticTarget::DimensionRegister(index), operand) => {
            let value = arithmetic_dimension(primitive, stores.dimen(index), operand)?;
            AssignmentCommitter::new(stores).dimension(index, value, global)
        }
        (ArithmeticTarget::DimensionParameter(index), operand) => {
            let value = arithmetic_dimension(
                primitive,
                stores.dimen_param(DimenParam::new(index)),
                operand,
            )?;
            let key = parameter_mutation_key_for_dialect(
                profile.dialect(),
                ParameterClass::Dimension,
                index,
            );
            AssignmentCommitter::new(stores).dimension_parameter(index, value, key, global)
        }
        (ArithmeticTarget::GlueRegister { index, mu }, operand) => {
            let old = stores.glue(if mu {
                stores.muskip(index)
            } else {
                stores.skip(index)
            });
            let value = arithmetic_glue(primitive, old, operand)?;
            AssignmentCommitter::new(stores).skip(index, value, global, mu, false, false)
        }
        (ArithmeticTarget::GlueParameter { index, .. }, operand) => {
            let old = stores.glue(stores.glue_param(GlueParam::new(index)));
            let value = arithmetic_glue(primitive, old, operand)?;
            let key =
                parameter_mutation_key_for_dialect(profile.dialect(), ParameterClass::Glue, index);
            AssignmentCommitter::new(stores).glue_parameter(index, value, key, global)
        }
        _ => return Err(ExecError::UnsupportedAssignmentTarget),
    };
    Ok(receipt)
}

pub(in crate::main_control) fn arithmetic_integer(
    primitive: UnexpandablePrimitive,
    old: i32,
    rhs: i32,
) -> Result<i32, ExecError> {
    match primitive {
        UnexpandablePrimitive::Advance => old.checked_add(rhs),
        UnexpandablePrimitive::Multiply => old.checked_mul(rhs),
        UnexpandablePrimitive::Divide => old.checked_div(rhs),
        _ => None,
    }
    .ok_or(ExecError::ArithmeticOverflow)
}

pub(in crate::main_control) fn arithmetic_dimension(
    primitive: UnexpandablePrimitive,
    old: Scaled,
    operand: ArithmeticOperand,
) -> Result<Scaled, ExecError> {
    if let (UnexpandablePrimitive::Advance, ArithmeticOperand::Dimension(rhs)) =
        (primitive, operand)
    {
        // TeX82 §104 deliberately does not range-check dimension addition,
        // and §1238 computes `cur_val+eqtb[l].int` without setting
        // `arith_error`. Preserve every sum representable by TeX's machine
        // integer, including `-max_dimen-1sp`.
        return old.checked_add(rhs).ok_or(ExecError::ArithmeticOverflow);
    }
    let raw = match (primitive, operand) {
        (UnexpandablePrimitive::Multiply, ArithmeticOperand::Integer(rhs)) => {
            old.raw().checked_mul(rhs)
        }
        (UnexpandablePrimitive::Divide, ArithmeticOperand::Integer(rhs)) => {
            old.raw().checked_div(rhs)
        }
        _ => None,
    };
    raw.and_then(scaled_within_arithmetic_bounds)
        .ok_or(ExecError::ArithmeticOverflow)
}

pub(in crate::main_control) fn scaled_within_arithmetic_bounds(raw: i32) -> Option<Scaled> {
    // TeX82 §1236 applies register arithmetic with `max_answer=max_dimen`
    // for dimensions and every component of glue. Its `arith_error` path
    // returns before the target definition, even when the result still fits
    // in the wider machine integer representation.
    (raw.unsigned_abs() <= Scaled::MAX_DIMEN.raw() as u32).then(|| Scaled::from_raw(raw))
}

pub(in crate::main_control) fn arithmetic_glue(
    primitive: UnexpandablePrimitive,
    old: GlueSpec,
    operand: ArithmeticOperand,
) -> Result<GlueSpec, ExecError> {
    match (primitive, operand) {
        (UnexpandablePrimitive::Advance, ArithmeticOperand::Glue(rhs)) => Ok(GlueSpec {
            width: old
                .width
                .checked_add(rhs.width)
                .ok_or(ExecError::ArithmeticOverflow)?,
            stretch: glue_component_add(
                old.stretch,
                old.stretch_order,
                rhs.stretch,
                rhs.stretch_order,
            )?
            .0,
            stretch_order: glue_component_add(
                old.stretch,
                old.stretch_order,
                rhs.stretch,
                rhs.stretch_order,
            )?
            .1,
            shrink: glue_component_add(old.shrink, old.shrink_order, rhs.shrink, rhs.shrink_order)?
                .0,
            shrink_order: glue_component_add(
                old.shrink,
                old.shrink_order,
                rhs.shrink,
                rhs.shrink_order,
            )?
            .1,
        }),
        (UnexpandablePrimitive::Multiply, ArithmeticOperand::Integer(rhs)) => {
            glue_scale(old, rhs, false)
        }
        (UnexpandablePrimitive::Divide, ArithmeticOperand::Integer(rhs)) => {
            glue_scale(old, rhs, true)
        }
        _ => Err(ExecError::UnsupportedAssignmentTarget),
    }
}

pub(in crate::main_control) fn glue_component_add(
    left: Scaled,
    mut left_order: Order,
    right: Scaled,
    mut right_order: Order,
) -> Result<(Scaled, Order), ExecError> {
    // TeX82 §1238 first normalizes a zero component on the newly scanned
    // specification before comparing its order, and only lets the stored
    // component replace it when that stored component is nonzero. Normalizing
    // both operands expresses the same value-based rule without depending on
    // which side happened to be scanned: a zero `fill` must never erase a
    // nonzero `fil` component during `\advance`.
    if left.raw() == 0 {
        left_order = Order::Normal;
    }
    if right.raw() == 0 {
        right_order = Order::Normal;
    }
    if left_order == right_order {
        return Ok((
            left.checked_add(right)
                .ok_or(ExecError::ArithmeticOverflow)?,
            left_order,
        ));
    }
    Ok(if left_order > right_order {
        (left, left_order)
    } else {
        (right, right_order)
    })
}

pub(in crate::main_control) fn glue_scale(
    spec: GlueSpec,
    factor: i32,
    divide: bool,
) -> Result<GlueSpec, ExecError> {
    let scale = |value: Scaled| {
        let raw = if divide {
            value.raw().checked_div(factor)
        } else {
            value.raw().checked_mul(factor)
        };
        raw.and_then(scaled_within_arithmetic_bounds)
            .ok_or(ExecError::ArithmeticOverflow)
    };
    Ok(GlueSpec {
        width: scale(spec.width)?,
        stretch: scale(spec.stretch)?,
        stretch_order: spec.stretch_order,
        shrink: scale(spec.shrink)?,
        shrink_order: spec.shrink_order,
    })
}

/// Replays TeX82's distinct vertical and horizontal rule paths.
///
/// TeX82 §1095 routes `\hrule` through `head_for_vmode`, so an ordinary
/// horizontal paragraph must finish before its rule reaches the page builder.
/// In vertical mode the rule is a direct contribution and resets `prev_depth`;
/// `\vrule`, conversely, enters horizontal mode before it appends its node.
///
/// `\hrule` in math mode never reaches this function: `scan_command`
/// intercepts `mmode+hrule` before scanning a rule spec at all (TeX82 §1046)
/// and replays §1047's `insert_dollar_sign` instead. `\vrule` in math mode
/// (§1056's `mmode+vrule`) is an ordinary direct contribution and falls
/// through the `else` branch below like any other mode.
pub(in crate::main_control) fn begin_replay_box(
    construction: ScannedBoxConstruction,
    target: Option<SetBoxTarget>,
    ships_out: bool,
    modes: &mut ModeNest,
    stores: &mut Universe,
    boxes: &mut ReplayBoxes,
    command: &mut CommandMachine<'_>,
) -> Result<(), ExecError> {
    let kind = ReplayBoxKind::from_scanned(construction.kind);
    let packing = match construction.packing {
        ScannedPackingSpec::Natural => PackSpec::Natural,
        ScannedPackingSpec::Exactly(size) => PackSpec::Exactly(size),
        ScannedPackingSpec::Spread(size) => PackSpec::Spread(size),
    };
    // TeX82 §1083 uses `adjusted_hbox_group` only when the hbox will be
    // appended (`box_context<box_flag`) in either vertical mode
    // (`abs(mode)=vmode`). A register, shipout, or shifted construction uses
    // `hbox_group`.
    let group_kind = if kind == ReplayBoxKind::HBox
        && target.is_none()
        && !ships_out
        && matches!(
            modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
        GroupKind::AdjustedHBox
    } else {
        kind.group_kind()
    };
    enter_group(stores, command.state, group_kind);
    modes.push_at_line(
        if kind.horizontal() {
            Mode::RestrictedHorizontal
        } else {
            Mode::InternalVertical
        },
        stores.current_input_line(),
    )?;
    if !kind.horizontal() {
        commit_box_normal_paragraph(modes, stores, command);
    }
    boxes.active_boxes.push(ActiveReplayBox {
        target,
        ships_out,
        kind,
        group_kind,
        packing,
        leader_kind: None,
        shift: None,
    });
    schedule_everybox(command.state, stores, kind.horizontal());
    Ok(())
}

pub(in crate::main_control) fn commit_box_normal_paragraph(
    modes: &mut ModeNest,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) {
    let record =
        (!stores.penalty_array(PenaltyArrayKind::InterLine).is_empty()).then(|| MutationRecord {
            target: MutationTarget::Register,
            key: ObservationValue::Name("toks:256".into()),
            value: ObservationValue::Tokens(Vec::new()),
            global: false,
        });
    let receipt = AssignmentCommitter::new(stores).unscoped(record, |stores| {
        crate::paragraph_end::normal_paragraph(modes, stores);
    });
    command.retain_assignment_receipt(receipt);
}

/// Applies a scanned TeX82 §1073 box-shift prefix (`\raise`, `\lower`,
/// `\moveleft`, `\moveright`). `ScannedBoxShiftPayload::Construction` opens
/// the same `BoxEndGroup` body-closing episode as an
/// ordinary `\hbox`/`\vbox`/`\vtop` (`BeginBox`/`BeginLeaderBox`'s twin),
/// deferring the shift until `BoxEndGroup` packages the body; every other
/// payload resolves to a node immediately and is shifted and appended right
/// here, exactly like `\box<n>`, `\lastbox`, and `\vsplit` do outside a
/// shift.
pub(in crate::main_control) fn apply_box_shift(
    shift: ScannedBoxShift,
    command: &mut CommandMachine<'_>,
    modes: &mut ModeNest,
    stores: &mut Universe,
    boxes: &mut ReplayBoxes,
) -> Result<ReplayStep, ExecError> {
    match shift.payload {
        ScannedBoxShiftPayload::Missing => {
            // `scan_box`'s own "A <box> was supposed to be here" recovery
            // (tex.web §1084); the rejected command has already been backed
            // up by `scan_box_payload` for ordinary replay.
            report_missing_box(command.state, stores)?;
            Ok(ReplayStep::Continue)
        }
        ScannedBoxShiftPayload::BoxRegister { index, copy } => {
            let id = read_box_register(index, copy, stores, command);
            let node = crate::box_runtime::first_box_node(stores, id);
            append_shifted_box(modes, stores, node, shift.delta, command)?;
            Ok(ReplayStep::Continue)
        }
        ScannedBoxShiftPayload::LastBox { error_context } => {
            let node =
                crate::box_runtime::take_last_box(modes, stores, command.fuel, error_context)?;
            append_shifted_box(modes, stores, node, shift.delta, command)?;
            Ok(ReplayStep::Continue)
        }
        ScannedBoxShiftPayload::VSplit(split) => {
            if let Some(context) = &split.missing_to_context {
                report_missing_vsplit_to(context, stores)?;
            }
            let node = crate::box_runtime::split_vbox_register(
                stores,
                split.index,
                split.height,
                &split.split_context,
            )?;
            append_shifted_box(modes, stores, node, shift.delta, command)?;
            Ok(ReplayStep::Continue)
        }
        ScannedBoxShiftPayload::Construction(construction) => {
            let axis = if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                BoxShiftAxis::Horizontal
            } else {
                BoxShiftAxis::Vertical
            };
            let kind = ReplayBoxKind::from_scanned(construction.kind);
            let packing = match construction.packing {
                ScannedPackingSpec::Natural => PackSpec::Natural,
                ScannedPackingSpec::Exactly(size) => PackSpec::Exactly(size),
                ScannedPackingSpec::Spread(size) => PackSpec::Spread(size),
            };
            // TeX82 §1083 selects `adjusted_hbox_group` only for an hbox
            // whose append-like box context is being built in vertical
            // mode. A `\raise`/`\lower` hbox is necessarily reached from
            // horizontal or math mode and therefore uses `hbox_group`;
            // `\moveleft`/`\moveright` in vertical mode uses the adjusted
            // group so migrated adjustments can be appended afterward.
            let group_kind = if kind == ReplayBoxKind::HBox
                && matches!(
                    modes.current_mode(),
                    Mode::Vertical | Mode::InternalVertical
                ) {
                GroupKind::AdjustedHBox
            } else {
                kind.group_kind()
            };
            enter_group(stores, command.state, group_kind);
            modes.push_at_line(
                if kind.horizontal() {
                    Mode::RestrictedHorizontal
                } else {
                    Mode::InternalVertical
                },
                stores.current_input_line(),
            )?;
            if !kind.horizontal() {
                commit_box_normal_paragraph(modes, stores, command);
            }
            boxes.active_boxes.push(ActiveReplayBox {
                target: None,
                ships_out: false,
                kind,
                group_kind,
                packing,
                leader_kind: None,
                shift: Some(ReplayBoxShift {
                    delta: shift.delta,
                    axis,
                }),
            });
            schedule_everybox(command.state, stores, kind.horizontal());
            Ok(ReplayStep::Continue)
        }
    }
}

/// TeX82 §1071's `box_context`, for the box constructions §1079's `begin_box`
/// resolves to a `cur_box` immediately -- `box_code`, `copy_code`,
/// `last_box_code`, and `vsplit_code`, which all fall through to the shared
/// `box_end(box_context)` call at the end of `begin_box`.
///
/// tex.web encodes the context as one integer and lets `box_end` classify it
/// (`box_context<box_flag`, `<ship_out_flag`, `=ship_out_flag`, or greater).
/// Enumerating it here keeps that single classification, so no producer can
/// silently implement only part of the context space: before this existed,
/// `\box`/`\copy` and `\lastbox` recognized only the append and `\shipout`
/// contexts and dropped `\setbox`'s entirely, leaving `\setbox0\lastbox`
/// re-appending its box and voiding the destination register
/// (`umber2-johp.263`).
///
/// The leader context (`box_context>ship_out_flag`, §1078) is not represented:
/// §1078 has to scan the *following* glue command before it can build its
/// node, so the command scanner resolves leader payloads as their own
/// `ColdOperation`s with the glue already attached.
#[derive(Clone, Copy, Debug)]
pub(in crate::main_control) enum BoxContext {
    /// `box_context<box_flag`: §1076's "Append box `cur_box` to the current
    /// list, shifted by `box_context`". The plain append is a zero shift.
    Append(Scaled),
    /// `box_flag<=box_context<ship_out_flag`: §1077's "Store `cur_box` in a
    /// box register", `eq_define`/`geq_define` by `\setbox`/`\global\setbox`.
    SetBox(SetBoxTarget),
    /// `box_context=ship_out_flag`: §1075's `ship_out(cur_box)`.
    ShipOut,
}

pub(in crate::main_control) fn read_box_register(
    index: u16,
    copy: bool,
    stores: &mut Universe,
    command: &CommandMachine<'_>,
) -> Option<tex_state::node_arena::PageListId> {
    if !copy {
        return stores.take_box_to_page(index);
    }
    let root = stores.copy_box_to_page(index)?;
    stores.observe_box_copy_ref(&root, command.state.transient_dynamic_words());
    Some(root)
}

impl ReplayBoxes {
    /// Resolves the pending `box_context` for a box that reaches `box_end`
    /// immediately, consuming it exactly like tex.web's single-use integer.
    ///
    /// `\shipout` and `\setbox` cannot both be pending on well-formed input:
    /// §1084's `scan_box` accepts only a `make_box` command after `\setbox`,
    /// and `\shipout` is `leader_ship`, so `\setbox0\shipout...` never gets
    /// past the "A <box> was supposed to be here" recovery. The pending
    /// `\setbox` target is still consumed either way so a recovered input
    /// cannot leave it to capture an unrelated later box.
    pub(in crate::main_control) fn take_box_context(&mut self, ships_out: bool) -> BoxContext {
        let target = self.pending_setbox.take();
        if ships_out {
            self.pending_shipout = false;
            return BoxContext::ShipOut;
        }
        match target {
            Some(target) => BoxContext::SetBox(target),
            None => BoxContext::Append(Scaled::from_raw(0)),
        }
    }
}

/// TeX82 §1075's `box_end`: the one place a resolved `cur_box` is disposed of
/// according to its context. `\hbox`/`\vbox`/`\vtop` bodies reach the same
/// three dispositions through `BoxEndGroup`, which cannot share this entry
/// point because §1083 defers them to their group's closing brace.
pub(in crate::main_control) fn box_end(
    context: BoxContext,
    node: Option<Node>,
    modes: &mut ModeNest,
    stores: &mut Universe,
    prepared_dvi_pages: &mut PreparedDviPages,
    command: &mut CommandMachine<'_>,
) -> Result<(), ExecError> {
    match context {
        BoxContext::Append(delta) => append_shifted_box(modes, stores, node, delta, command),
        // §1077 defines the register unconditionally: a void `cur_box` makes
        // the destination void, it does not leave the old value in place.
        BoxContext::SetBox(target) => {
            let boxed = node.map(|node| stores.publish_page_nodes(std::slice::from_ref(&node)));
            commit_set_box_target(target, boxed, stores, command);
            Ok(())
        }
        // §1075 guards `ship_out` with `cur_box<>null`.
        BoxContext::ShipOut => {
            if let Some(node) = node
                && let Some(receipt) = shipout_replay_box(node, stores, command)?
                    .and_then(|publication| publication.dvi)
            {
                push_prepared_dvi_page(prepared_dvi_pages, receipt);
            }
            Ok(())
        }
    }
}

/// Commits TeX82 §1077/e-TeX 2.6 [47.1077]'s resolved `box_end` target.
///
/// Dense targets use `eq_define` and remain outside the oracle's named
/// mutation regions. Extended targets use [53a]'s `sa_def`/`gsa_def`, whose
/// committed sparse-register boundary is observed after §1085 has packaged
/// and unsaved the box group. Keeping the observation beside the write also
/// covers immediate `\box`, `\copy`, `\lastbox`, and `\vsplit` operands.
pub(in crate::main_control) fn commit_set_box_target(
    target: SetBoxTarget,
    boxed: Option<tex_state::node_arena::PageListId>,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) {
    let traced_box = boxed.clone();
    let receipt = AssignmentCommitter::new(stores).box_register(
        target.index,
        traced_box.as_ref(),
        target.global,
        |stores| match (target.global, boxed) {
            (false, Some(boxed)) => stores.assign_page_box_local(target.index, boxed),
            (true, Some(boxed)) => stores.assign_page_box_global(target.index, boxed),
            (false, None) => stores.clear_box_local(target.index),
            (true, None) => stores.clear_box_global(target.index),
        },
    );
    command.retain_assignment_receipt(receipt);
}

/// Applies TeX82 §1073's `shift_amount(cur_box):=box_context` to an already
/// scanned box, then appends it exactly like an ordinary standalone box
/// (`\box<n>`'s bare append, or `BoxEndGroup`'s final branch). A void box is
/// a no-op, matching `box_end`'s `if cur_box<>null` guard.
pub(in crate::main_control) fn append_shifted_box(
    modes: &mut ModeNest,
    stores: &mut Universe,
    node: Option<Node>,
    delta: Scaled,
    command: &mut CommandMachine<'_>,
) -> Result<(), ExecError> {
    let Some(mut node) = node else {
        return Ok(());
    };
    crate::box_runtime::apply_box_shift_delta(&mut node, delta)?;
    crate::box_runtime::append_box_node_to_current_list(modes, stores, node, command.fuel)?;
    let error_context = command.state.output_open_context(&stores.command_context());
    crate::vertical::build_page_if_outer_vertical_with_error_context(modes, stores, &error_context)
}

pub(in crate::main_control) fn apply_scanned_rule(
    command: &mut CommandMachine<'_>,
    modes: &mut ModeNest,
    stores: &mut Universe,
    width: Option<Scaled>,
    height: Option<Scaled>,
    depth: Option<Scaled>,
    horizontal: bool,
) -> Result<ReplayStep, ExecError> {
    let node = Node::Rule {
        width,
        height,
        depth,
    };
    if horizontal {
        match modes.current_mode() {
            Mode::Vertical | Mode::InternalVertical => {}
            Mode::Horizontal => {
                crate::paragraph_end::end_paragraph_with_fuel(
                    modes,
                    stores,
                    command.state,
                    command.fuel,
                )?;
            }
            Mode::RestrictedHorizontal => unreachable!(
                "TeX82 §1095 diagnoses restricted-horizontal hrule before rule scanning"
            ),
            mode => {
                return Err(ExecError::UnimplementedTypesetting {
                    mode,
                    token: Token::Cs(stores.intern("hrule").symbol()),
                    origin: tex_state::token::OriginId::UNKNOWN,
                    operation: "\\hrule",
                });
            }
        }
        crate::vertical::append_vertical_contribution(modes, stores, node);
        modes
            .current_list_mutation()
            .set_prev_depth(crate::mode::ignored_depth(stores));
        // TeX82 §1056's `append_rule` stops after `tail_append` and resetting
        // `prev_depth` in vertical mode. Unlike §1075's box append and §1103's
        // penalty append, it deliberately does not call `build_page`; the
        // next command with an explicit page-builder tail owns that visit.
    } else {
        if matches!(
            modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            start_paragraph(command.state, modes, stores, true)?;
        }
        // TeX82 §1054 reaches `append_rule` only after main_control has
        // finished the current word. Materialize Umber's pending character
        // run before appending the rule so a `\vrule` cannot split a word and
        // move its final character behind the rule node.
        crate::box_runtime::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
        modes.current_list_mutation().push(node);
        // TeX82 §1056 resets `space_factor` after a rule in either
        // horizontal mode. This matters when a zero-sfcode closer follows
        // the rule: it must inherit 1000, not sentence spacing from text
        // before the rule.
        if matches!(
            modes.current_mode(),
            Mode::Horizontal | Mode::RestrictedHorizontal
        ) {
            modes.current_list_mutation().set_space_factor(1000);
        }
    }
    Ok(ReplayStep::Continue)
}

/// TeX82 §1123's list-building tail, with §1125's kerns.
pub(in crate::main_control) struct AccentPlacement {
    pub(in crate::main_control) accent: u8,
    pub(in crate::main_control) accent_font: tex_state::ids::FontId,
    pub(in crate::main_control) accent_metrics: tex_state::font::CharMetrics,
    pub(in crate::main_control) accent_origin: tex_state::provenance::OriginRef,
    /// §1124's `q`: the base character and its origin, or `null`.
    pub(in crate::main_control) base: Option<(u8, tex_state::provenance::OriginRef)>,
}

/// Appends §1123's `link(tail):=p; tail:=p; space_factor:=1000`, preceded by
/// §1125's accent kerns when §1124 produced a base character.
pub(in crate::main_control) fn apply_accent_nodes(
    modes: &mut ModeNest,
    stores: &mut Universe,
    etex_extended: bool,
    placement: AccentPlacement,
) -> Result<ReplayStep, ExecError> {
    let AccentPlacement {
        accent,
        accent_font,
        accent_metrics,
        accent_origin,
        base,
    } = placement;
    let accent_node = Node::Char {
        font: accent_font,
        ch: char::from(accent),
        origin: accent_origin,
    };
    // §1124's `f:=cur_font` is re-read *after* `do_assignments`, so the base
    // character is set in whatever font those assignments left selected.
    let base_font = stores.current_font();
    let base = base.and_then(|(character, origin)| {
        let Some(metrics) = stores.font_char_metrics(base_font, character) else {
            crate::diagnostics::report_missing_character_warning(
                stores,
                base_font,
                char::from(character),
                etex_extended,
            );
            return None;
        };
        Some((character, origin, metrics))
    });
    let Some((character, base_origin, base_metrics)) = base else {
        modes.current_list_mutation().push(accent_node);
        modes.current_list_mutation().set_space_factor(1000);
        return Ok(ReplayStep::Continue);
    };
    let accent_x_height = stores.font_parameter(accent_font, 5);
    let accent_slant = stores.font_parameter(accent_font, 1);
    let base_slant = stores.font_parameter(base_font, 1);
    let delta = tex_state::scaled::text_accent_delta(
        base_metrics.width,
        accent_metrics.width,
        base_metrics.height,
        base_slant,
        accent_x_height,
        accent_slant,
    );
    modes.current_list_mutation().push(Node::Kern {
        amount: delta,
        kind: KernKind::Accent,
    });
    if base_metrics.height == accent_x_height {
        modes.current_list_mutation().push(accent_node);
    } else {
        let children = stores.publish_page_nodes(&[accent_node]);
        let mut boxed =
            crate::box_runtime::hpack_with_overfull_rule(stores, children, PackSpec::Natural);
        boxed.shift = accent_x_height
            .checked_sub(base_metrics.height)
            .ok_or(ExecError::ArithmeticOverflow)?;
        modes.current_list_mutation().push(Node::HList(boxed));
    }
    modes.current_list_mutation().push(Node::Kern {
        amount: Scaled::from_raw(-accent_metrics.width.raw() - delta.raw()),
        kind: KernKind::Accent,
    });
    modes.current_list_mutation().push(Node::Char {
        font: base_font,
        ch: char::from(character),
        origin: base_origin,
    });
    modes.current_list_mutation().set_space_factor(1000);
    Ok(ReplayStep::Continue)
}

pub(in crate::main_control) fn assign_math_family_font(
    stores: &mut Universe,
    size: MathFontSize,
    family: u8,
    font: FontId,
    global: bool,
) -> Result<(), ExecError> {
    // The typed scanner has resolved the selector, but §1234's assignment is
    // not committed until the font is known to supply classic or OpenType
    // MATH metrics. Keep this check before the environment mutation so a
    // captured error can restore/retry the command without changing its
    // checkpoint identity.
    if !stores.font(font).supports_math() {
        return Err(ExecError::OpenTypeMathUnsupported);
    }
    stores.set_math_family_font(size, family, font, global);
    Ok(())
}

pub(in crate::main_control) fn load_font(
    request: &FontLoadRequest,
    resource: FontResource,
) -> Result<tex_fonts::LoadedFont, ExecError> {
    let display_name = request.name.strip_suffix(".tfm").unwrap_or(&request.name);
    let from_tfm = |metrics: tex_state::world::FileContent,
                    opentype: Option<tex_fonts::OpenTypeFont>,
                    mapped: Option<(tex_fonts::OpenTypeFont, tex_fonts::LegacyEncodingMap)>|
     -> Result<tex_fonts::LoadedFont, ExecError> {
        let tfm = tex_fonts::TfmFont::parse_with_size(metrics.bytes(), request.size)?;
        let mut font = tfm.into_loaded_font(
            display_name,
            metrics.path().to_owned(),
            metrics.hash().bytes(),
        );
        if let Some((selection, encoding_map)) = mapped {
            font = font.with_mapped_opentype(selection, encoding_map);
        } else if let Some(selection) = opentype {
            font = font.with_opentype(selection);
        }
        Ok(font)
    };
    match resource {
        FontResource::Unavailable => unreachable!("unavailable resources recover before parsing"),
        FontResource::Tfm { metrics, opentype } => from_tfm(metrics, opentype, None),
        FontResource::MappedTfm {
            metrics,
            opentype,
            encoding_map,
        } => from_tfm(metrics, None, Some((opentype, encoding_map))),
        FontResource::ClassicTfmFallback { metrics } => {
            Ok(from_tfm(metrics, None, None)?.with_classic_mapping_fallback())
        }
        FontResource::OpenType(selection) => {
            let design_size = Scaled::from_raw(10 * Scaled::UNITY);
            let size = tex_state::scaled::tfm_font_size(design_size, request.size)
                .map_err(|_| ExecError::ArithmeticOverflow)?;
            Ok(tex_fonts::LoadedFont::new_opentype(
                request
                    .name
                    .strip_prefix("opentype:")
                    .unwrap_or(&request.name),
                request
                    .name
                    .strip_prefix("opentype:")
                    .unwrap_or(&request.name),
                design_size,
                size,
                selection,
            ))
        }
    }
}

/// TeX82 §1095 `new_graf`: command control has already made any required
/// backup, then this typed transition installs the indent and schedules the
/// immutable `\everypar` payload through the same command state.
pub(in crate::main_control) fn start_paragraph(
    command: &mut CommandState,
    modes: &mut ModeNest,
    stores: &mut Universe,
    indent: bool,
) -> Result<(), ExecError> {
    let error_context = command.output_open_context(&stores.command_context());
    crate::paragraph_end::start_paragraph(modes, stores, indent, &error_context)?;
    let everypar = stores.tok_param(TokParam::EVERY_PAR);
    if !stores.tokens(everypar).is_empty() {
        let origin = stores.bootstrap_origin();
        let traced: Vec<_> = stores
            .tokens(everypar)
            .iter()
            .copied()
            .map(|token| tex_state::token::TracedTokenWord::pack(token, origin))
            .collect();
        let tokens = stores.finish_traced_token_list(&traced);
        command.push_everypar(&stores.command_context(), tokens);
    }
    Ok(())
}

/// Closes a `\insert<class>{...}` or `\vadjust{...}` body: TeX82 §1099/§1100's
/// shared `insert_group` case of `handle_right_brace`.
///
/// `end_graf` first finishes any paragraph left open inside the body (§1100:
/// `end_graf` runs before anything else, exactly like
/// `vbox_group`/`vtop_group`). `\splittopskip`, `\splitmaxdepth`, and
/// `\floatingpenalty` are read at their current (still-local) values before
/// `unsave` -- assignments to those parameters made inside the body govern
/// its own splitting, exactly as tex.web's `q:=split_top_skip;
/// d:=split_max_depth; f:=floating_penalty; unsave` orders it. The body is
/// then packed with TeX82's `vpack` macro (`vpackage(p,h,m,max_dimen)`):
/// unconstrained depth, but the *current* `\vbadness`/`\vfuzz` -- unlike an
/// ordinary `\vbox`, neither `\insert` nor `\vadjust` ever suppresses those
/// parameters.
///
/// §1100 then branches on `saved(0)` (`class`, here): `class<255` builds an
/// `ins_node` whose `height` field is the packed natural height+depth
/// (TeX82's `size`, consumed only by the page builder's splitting
/// arithmetic, `crate::page_builder`); `class=255` (`\vadjust`) instead
/// builds an `adjust_node` carrying only the packed content -- `q`/`d`/`f`
/// are still read above (mirroring tex.web's unconditional `q:=...; d:=...;
/// f:=...` before the branch) but never stored, matching
/// `delete_glue_ref(q)`'s discard. Either node is appended to whatever list
/// was open when `\insert`/`\vadjust` began -- the enclosing mode's list, not
/// a side channel -- exactly like `\mark` and `\penalty` above. `nest_ptr=0`
/// (`is_outer_vertical`) then invokes `build_page`, matching §1099's `if
/// nest_ptr=0 then build_page` (`\vadjust` never actually reaches this since
/// it is forbidden in outer vertical mode).
pub(in crate::main_control) fn finish_insert_or_adjust_group(
    class: u16,
    pre: bool,
    modes: &mut ModeNest,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) -> Result<ReplayStep, ExecError> {
    // TeX82 §§993/1100: an outer-vertical insertion invokes `build_page`
    // before main control fetches another command. Preserve this closing
    // brace's still-live input stack for `ensure_vbox` -> `box_error` -> §82.
    let page_error_context = command.state.output_open_context(&stores.command_context());
    crate::paragraph_end::end_paragraph_with_fuel(modes, stores, command.state, command.fuel)?;
    let split_top_skip = *stores.glue(stores.glue_param(GlueParam::SPLIT_TOP_SKIP));
    let split_max_depth = stores.dimen_param(DimenParam::SPLIT_MAX_DEPTH);
    let floating_penalty = stores.int_param(IntParam::FLOATING_PENALTY);
    let aftergroup = stores
        .leave_group_with_kind(GroupKind::Insert)
        .map_err(|_| ExecError::MissingToken {
            context: "insert group",
        })?;
    schedule_aftergroup(command, stores, aftergroup)?;
    let level = crate::box_runtime::commit_current_list(modes, stores, command.fuel)?;
    let content = stores.publish_page_nodes(level.list().nodes());
    let params = tex_typeset::VpackParams {
        box_max_depth: Scaled::MAX_DIMEN,
        ..crate::packing_params::vpack_params(stores)
    };
    let packed = crate::packing_params::vpack(stores, content.clone(), PackSpec::Natural, params);
    crate::box_runtime::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
    let node = if class == 255 {
        Node::Adjust(tex_state::node::AdjustNode { content, pre })
    } else {
        let size = packed
            .node
            .height
            .checked_add(packed.node.depth)
            .ok_or(ExecError::ArithmeticOverflow)?;
        Node::Ins {
            class,
            size,
            split_top_skip,
            split_max_depth,
            floating_penalty,
            content,
        }
    };
    crate::vertical::append_vertical_contribution(modes, stores, node);
    crate::vertical::build_page_if_outer_vertical_with_error_context(
        modes,
        stores,
        &page_error_context,
    )?;
    Ok(ReplayStep::Continue)
}

/// Schedules an every-box list after replay has entered its scoped group and
/// mode.  The immutable traced list is owned by command state, preserving the
/// ordinary macro, recovery, retirement, and provenance path for hook tokens.
pub(in crate::main_control) fn schedule_everybox(
    command: &mut CommandState,
    stores: &mut Universe,
    horizontal: bool,
) {
    let parameter = if horizontal {
        TokParam::EVERY_HBOX
    } else {
        TokParam::EVERY_VBOX
    };
    let tokens = stores.tok_param(parameter);
    if stores.tokens(tokens).is_empty() {
        return;
    }
    let tokens: Vec<_> = stores.tokens(tokens).to_vec();
    let mut traced = tex_state::token::RootedTracedTokenBuffer::default();
    for token in tokens {
        let origin = stores.inserted_origin(
            tex_state::provenance::InsertedOriginKind::TokenListReplay(if horizontal {
                tex_state::TokenListReplayKind::EveryHBox
            } else {
                tex_state::TokenListReplayKind::EveryVBox
            }),
            token,
            tex_state::token::OriginId::UNKNOWN,
        );
        traced.extend_archived([tex_state::token::TracedTokenWord::pack(token, origin)]);
    }
    let tokens = stores.finish_rooted_traced_token_list(&traced);
    command.push_everybox(&stores.command_context(), tokens, horizontal);
}

/// Runs TeX82 §774 `init_align`'s and §799 `fin_row`'s shared
/// `if every_cr<>null then begin_token_list(every_cr,every_cr_text)`.
///
/// Both sections push `\everycr` immediately before `align_peek`, so the hook
/// supplies the tokens that lookahead classifies -- typically plain.tex's
/// `\noalign{...}`. §785's `align_peek` itself never pushes it, and neither
/// does §1133's `no_align_group` case of `handle_right_brace`, which reaches
/// `align_peek` a second time after a `\noalign` body.
pub(in crate::main_control) fn schedule_everycr(command: &mut CommandState, stores: &mut Universe) {
    let tokens = stores.tok_param(TokParam::EVERY_CR);
    if stores.tokens(tokens).is_empty() {
        return;
    }
    let tokens: Vec<_> = stores.tokens(tokens).to_vec();
    let mut traced = tex_state::token::RootedTracedTokenBuffer::default();
    for token in tokens {
        let origin = stores.inserted_origin(
            tex_state::provenance::InsertedOriginKind::TokenListReplay(
                tex_state::TokenListReplayKind::EveryCr,
            ),
            token,
            tex_state::token::OriginId::UNKNOWN,
        );
        traced.extend_archived([tex_state::token::TracedTokenWord::pack(token, origin)]);
    }
    let tokens = stores.finish_rooted_traced_token_list(&traced);
    command.push_everycr(&stores.command_context(), tokens);
}

/// Runs TeX82 §1030 `main_control`'s prologue,
/// `if every_job<>null then begin_token_list(every_job,every_job_text)`.
///
/// `\everyjob` is read once, before `big_switch` fetches anything, so the hook
/// is owned by the entry into `main_control` rather than by any command.
/// `Universe::take_pending_every_job` is the one-shot that distinguishes a job
/// started from a format image (where the parameter the format dumped is live
/// at entry) from the INITEX job that built it and from a resumed timeline
/// that already passed this point.
pub(in crate::main_control) fn schedule_everyjob(
    command: &mut CommandState,
    stores: &mut Universe,
) {
    let tokens = stores.take_pending_every_job();
    if stores.tokens(tokens).is_empty() {
        return;
    }
    let tokens: Vec<_> = stores.tokens(tokens).to_vec();
    let mut traced = tex_state::token::RootedTracedTokenBuffer::default();
    for token in tokens {
        let origin = stores.inserted_origin(
            tex_state::provenance::InsertedOriginKind::TokenListReplay(
                tex_state::TokenListReplayKind::EveryJob,
            ),
            token,
            tex_state::token::OriginId::UNKNOWN,
        );
        traced.extend_archived([tex_state::token::TracedTokenWord::pack(token, origin)]);
    }
    let tokens = stores.finish_rooted_traced_token_list(&traced);
    command.push_everyjob(&stores.command_context(), tokens);
}

pub(in crate::main_control) fn schedule_everymath(
    command: &mut CommandState,
    stores: &mut Universe,
    display: bool,
) {
    let parameter = if display {
        TokParam::EVERY_DISPLAY
    } else {
        TokParam::EVERY_MATH
    };
    let tokens = stores.tok_param(parameter);
    if stores.tokens(tokens).is_empty() {
        return;
    }
    let origin = stores.bootstrap_origin();
    let traced: Vec<_> = stores
        .tokens(tokens)
        .iter()
        .copied()
        .map(|token| tex_state::token::TracedTokenWord::pack(token, origin))
        .collect();
    let tokens = stores.finish_traced_token_list(&traced);
    command.push_everymath(&stores.command_context(), tokens, display);
}

/// A tex.web recoverable-error report that scanning detects but only the
/// stomach can print.
///
/// The remaining reports here are ones whose semantic transition completes
/// before the World-facing executor sees them. A scan that can print at the
/// point of detection does -- §§433-437's range recovery moved into
/// `scan_restricted_integer` for exactly that reason -- because a queued
/// report lands after everything the rest of the step emits, including
/// §362's `)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum PendingDiagnostic {
    /// A command-owned diagnostic whose semantic transition completed before
    /// the World-facing executor could render it.
    Command(tex_command::CommandSemanticDiagnostic),
    /// tex.web §1212's `<Discard erroneous prefixes and return>`.
    ///
    /// The `bool` is `eTeX_ex`: etex.ch rewrites `help_line[0]` to name
    /// `\protected` alongside `\long`, `\outer`, and `\global`.
    PrefixOnNonPrefixedCommand(tex_command::PrintCommand, String, bool),
    /// tex.web §1213's `<Discard the prefixes \long and \outer if they are
    /// irrelevant>`.
    ///
    /// The `bool` is `eTeX_ex`, which here rewrites the *message* as well as
    /// the help: etex.ch prints `' or `\protected'` before `' with `'.
    IrrelevantLongOuterPrefix(tex_command::PrintCommand, String, bool),
}

impl PendingDiagnostic {
    pub(in crate::main_control) fn causal_kind(&self) -> Option<&'static str> {
        match self {
            Self::Command(tex_command::CommandSemanticDiagnostic::Trace { .. })
            | Self::Command(tex_command::CommandSemanticDiagnostic::PdfExpansionMessage {
                ..
            }) => None,
            Self::Command(tex_command::CommandSemanticDiagnostic::UndefinedControlSequence {
                ..
            }) => Some("undefined-control-sequence"),
            Self::Command(tex_command::CommandSemanticDiagnostic::MacroPrefixMismatch {
                ..
            }) => Some("macro-prefix-mismatch"),
            Self::Command(tex_command::CommandSemanticDiagnostic::MissingNumber { .. }) => {
                Some("missing-number")
            }
            Self::Command(tex_command::CommandSemanticDiagnostic::FontDimenUnavailable {
                ..
            }) => Some("font-dimen-unavailable"),
            Self::Command(tex_command::CommandSemanticDiagnostic::Recoverable { .. }) => {
                Some("command-recoverable")
            }
            Self::PrefixOnNonPrefixedCommand(..) => Some("prefix-on-non-prefixed-command"),
            Self::IrrelevantLongOuterPrefix(..) => Some("irrelevant-long-outer-prefix"),
        }
    }
}

/// Prints each report a completed scan owes, in detection order.
pub(in crate::main_control) fn report_pending_diagnostics(
    stores: &mut Universe,
    diagnostics: Vec<PendingDiagnostic>,
) -> Result<(), ExecError> {
    for diagnostic in diagnostics {
        match diagnostic {
            PendingDiagnostic::Command(tex_command::CommandSemanticDiagnostic::Trace {
                text,
                force_newline,
            }) => {
                let mut output = stores.begin_diagnostic();
                if force_newline {
                    output.print_ln().print(&text);
                } else {
                    output.print_nl(&text);
                }
                output.end(false);
            }
            PendingDiagnostic::Command(
                tex_command::CommandSemanticDiagnostic::PdfExpansionMessage { text },
            ) => {
                let mut output = stores.printer();
                output.print_nl(&text).print_ln();
            }
            PendingDiagnostic::Command(
                tex_command::CommandSemanticDiagnostic::UndefinedControlSequence { context },
            ) => crate::diagnostics::report_undefined_control_sequence(stores, Some(context))?,
            PendingDiagnostic::Command(
                tex_command::CommandSemanticDiagnostic::MacroPrefixMismatch {
                    macro_name: symbol,
                    context,
                },
            ) => {
                let name = stores.resolve(symbol).to_owned();
                let kind = stores.control_sequence_kind(symbol);
                let mut report = stores.print_err("Use of ");
                report
                    .sprint_cs(kind, &name)
                    .print(" doesn't match its definition");
                report
                    .help(&[
                        "If you say, e.g., `\\def\\a1{...}', then you must always",
                        "put `1' after `\\a', since control sequence names are",
                        "made up of letters only. The macro here has not been",
                        "followed by the required stuff, so I'm ignoring it.",
                    ])
                    .context(context);
                report.error().jump_out()?;
            }
            PendingDiagnostic::Command(tex_command::CommandSemanticDiagnostic::Recoverable {
                runaway,
                message,
                help,
                context,
                ..
            }) => {
                if let Some(runaway) = runaway {
                    let mut output = stores.printer();
                    output.print_nl(runaway.heading).print_ln();
                    output.print_rendered(&runaway.partial);
                }
                let mut report = stores.print_err(&message);
                report.help(help).context(context);
                report.error().jump_out()?;
            }
            PendingDiagnostic::Command(tex_command::CommandSemanticDiagnostic::MissingNumber {
                context,
            }) => {
                let mut report = stores.print_err("Missing number, treated as zero");
                report
                    .help(&[
                        "A number should have been here; I inserted `0'.",
                        "(If you can't figure out why I needed to see a number,",
                        "look up `weird error' in the index to The TeXbook.)",
                    ])
                    .context(context);
                report.error().jump_out()?;
            }
            PendingDiagnostic::Command(
                tex_command::CommandSemanticDiagnostic::FontDimenUnavailable { font, context },
            ) => report_font_parameter_recovery(stores, font, context)?,
            PendingDiagnostic::PrefixOnNonPrefixedCommand(command, context, etex) => {
                let command = tex_command::print_cmd_chr_text(&stores.command_context(), command);
                let mut report = stores.print_err("You can't use a prefix with `");
                report.print(&command).print_char('\'');
                report.help(if etex {
                    &["I'll pretend you didn't say \\long or \\outer or \\global or \\protected."]
                } else {
                    &["I'll pretend you didn't say \\long or \\outer or \\global."]
                });
                report.context(context);
                report.error().jump_out()?;
            }
            PendingDiagnostic::IrrelevantLongOuterPrefix(command, context, etex) => {
                let command = tex_command::print_cmd_chr_text(&stores.command_context(), command);
                let mut report = stores.print_err("You can't use `");
                report.print_esc("long").print("' or `").print_esc("outer");
                if etex {
                    report.print("' or `").print_esc("protected");
                }
                report.print("' with `").print(&command).print_char('\'');
                report.help(if etex {
                    &["I'll pretend you didn't say \\long or \\outer or \\protected here."]
                } else {
                    &["I'll pretend you didn't say \\long or \\outer here."]
                });
                report.context(context);
                report.error().jump_out()?;
            }
        }
    }
    Ok(())
}

pub(in crate::main_control) fn mode_text_for_command_trace(mode: Mode) -> &'static str {
    match mode {
        Mode::Vertical => "vertical mode",
        Mode::InternalVertical => "internal vertical mode",
        Mode::Horizontal => "horizontal mode",
        Mode::RestrictedHorizontal => "restricted horizontal mode",
        Mode::Math => "math mode",
        Mode::DisplayMath => "display math mode",
    }
}

/// Reports TeX82 §1258's and §1259's illegal font-size recoveries.
pub(in crate::main_control) fn report_font_size_recovery(
    stores: &mut Universe,
    recovery: &tex_command::FontSizeRecovery,
) -> Result<(), ExecError> {
    match recovery {
        tex_command::FontSizeRecovery::ImproperAtSize { size, context } => {
            let mut report = stores.print_err("Improper `at' size (");
            report.print_scaled(*size).print("pt), replaced by 10pt");
            report
                .help(&[
                    "I can only handle fonts at positive sizes that are",
                    "less than 2048pt, so I've changed what you said to 10pt.",
                ])
                .context(context.clone());
            report.error().jump_out()?;
        }
        tex_command::FontSizeRecovery::IllegalMagnification { value, context } => {
            let mut report = stores.print_err("Illegal magnification has been changed to 1000");
            report
                .help(&["The magnification ratio must be between 1 and 32768."])
                .context(context.clone());
            report.int_error(*value).jump_out()?;
        }
    }
    Ok(())
}

/// TeX82 §1279's `token_show(def_ref)` into `new_string`.
pub(in crate::main_control) fn message_text(
    stores: &Universe,
    tokens: tex_state::ids::TokenListId,
) -> String {
    message_tokens_text(stores, tokens)
}

pub(in crate::main_control) fn message_tokens_text(
    stores: &Universe,
    tokens: tex_state::ids::TokenListId,
) -> String {
    let mut text = String::new();
    for &token in stores.tokens(tokens).iter() {
        tex_state::token_show::append_token_string_text(stores, token, &mut text);
    }
    text
}

/// TeX82 §1297's `token_show(temp_head)` through the active selector.
#[cfg(test)]
pub(in crate::main_control) fn show_tokens_text(
    stores: &Universe,
    tokens: tex_state::ids::TokenListId,
) -> String {
    show_tokens_tokens_text(stores, tokens)
}

pub(in crate::main_control) fn show_tokens_tokens_text(
    stores: &Universe,
    tokens: tex_state::ids::TokenListId,
) -> String {
    let newlinechar = u32::try_from(stores.int_param(IntParam::NEWLINE_CHAR))
        .ok()
        .filter(|&code| code <= u8::MAX.into())
        .and_then(char::from_u32);
    let mut text = String::new();
    for &token in stores.tokens(tokens).iter() {
        tex_state::token_show::append_token_selector_text(stores, token, newlinechar, &mut text);
    }
    text
}

/// e-TeX 2.6 `etex.ch` [17.3715--3732]'s exact `show_ifs` traversal.
/// The `### level N: ...` body only, joined by `\n` -- not etex.ch
/// [17.3720]'s leading `print_nl(""); print_ln`, which needs live column
/// state this pure builder does not have. The canonical `ColdOperation::ShowIfs`
/// site prints those two calls itself, through the open diagnostic, before
/// printing this body.
pub(in crate::main_control) fn render_showifs(
    conditions: &[tex_command::ActiveCondition],
) -> String {
    if conditions.is_empty() {
        return "### no active conditionals".to_owned();
    }
    let mut text = String::new();
    let mut level = conditions.len();
    for (index, condition) in conditions.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str("### level ");
        text.push_str(&level.to_string());
        text.push_str(": \\");
        if condition.inverted() {
            text.push_str("unless\\");
        }
        text.push_str(condition.kind_name());
        if condition.else_branch() {
            text.push_str("\\else");
        }
        if condition.source_line() != 0 {
            text.push_str(" entered on line ");
            text.push_str(&condition.source_line().to_string());
        }
        level -= 1;
    }
    text
}

/// TeX82 §1280's `<Print string s on the terminal>`.
pub(in crate::main_control) fn issue_terminal_message(stores: &mut Universe, text: &str) {
    let mut printer = stores.printer();
    if printer.terminal_offset() + text.chars().count() > printer.max_print_line().saturating_sub(2)
    {
        printer.print_ln();
    } else if printer.terminal_offset() > 0 || printer.log_offset() > 0 {
        printer.print_char(' ');
    }
    printer.print(text);
}

/// TeX82 §1283's `<Print string s as an error message>`.
pub(in crate::main_control) fn issue_error_message(
    stores: &mut Universe,
    text: &str,
    context: String,
) -> Result<(), ExecError> {
    let err_help = stores.tok_param(TokParam::ERR_HELP);
    let rendered = (!stores.tokens(err_help).is_empty()).then(|| message_text(stores, err_help));
    let interactive = stores.interaction_mode() == tex_state::InteractionMode::ErrorStop;
    let long_help_seen = stores
        .world_mut()
        .error_channel_mut()
        .take_long_help_seen(rendered.is_none() && !interactive);
    let mut report = stores.print_err("");
    report.print(text);
    match rendered {
        Some(rendered) => {
            report.use_err_help(rendered);
        }
        None if long_help_seen => {
            report.help(&["(That was another \\errmessage.)"]);
        }
        None => {
            report.help(&[
                "This error message was generated by an \\errmessage",
                "command, so I can't give any explicit help.",
                "Pretend that you're Hercule Poirot: Examine all clues,",
                "and deduce the truth by order and method.",
            ]);
        }
    }
    report.context(context);
    report.error().jump_out()?;
    Ok(())
}

/// Reports TeX82 §579's `<Issue an error message if cur_val=fmem_ptr>`.
///
/// Every `FontParameterError` is a way of landing on §578's `fmem_ptr`
/// fallback -- a number at or below zero, a number past the font's table when
/// the font is not the last one loaded, or a capacity bound -- so all of them
/// report the same §579 message and leave the font untouched.
pub(in crate::main_control) fn report_font_parameter_recovery(
    stores: &mut Universe,
    font: tex_state::ids::FontId,
    context: String,
) -> Result<(), ExecError> {
    // TeX82 §579 prints `font_id_text(f)`, which §1257 replaces whenever
    // a font definition (even a failed one naming `null_font`) reaches
    // `common_ending`. The physical TFM name is not the diagnostic identity.
    let name = stores.font_identifier_symbol(font).map_or_else(
        || stores.font_name(font),
        |symbol| stores.resolve(symbol).to_owned(),
    );
    let count = i32::try_from(stores.font_parameter_count(font)).unwrap_or(i32::MAX);
    let mut report = stores.print_err("Font ");
    report
        .print_esc(&name)
        .print(" has only ")
        .print_int(count)
        .print(" fontdimen parameters");
    report.help(&[
        "To increase the number of font parameters, you must",
        "use \\fontdimen immediately after the \\font is loaded.",
    ]);
    report.context(context);
    report.error().jump_out()?;
    Ok(())
}

/// Returns TeX82 §1257's string `t` for a new font definition.
pub(in crate::main_control) fn font_identifier_for_definition(
    stores: &mut Universe,
    target: Symbol,
) -> SymbolId {
    let (text, always_retained) = match stores.control_sequence_kind(target) {
        ControlSequenceKind::ActiveCharacter => (format!("FONT{}", stores.resolve(target)), true),
        ControlSequenceKind::Null => ("FONT".to_owned(), true),
        ControlSequenceKind::SingleCharacter
        | ControlSequenceKind::Named
        | ControlSequenceKind::Internal => (stores.resolve(target).to_owned(), false),
    };
    if always_retained {
        // TeX82 §1252 constructs a fresh `FONT<char>`/`FONT` string for each
        // active or null target; semantic-name interning must not deduplicate
        // that physical pool allocation.
        stores.intern_retained_pool_string(&text)
    } else {
        stores.intern(&text)
    }
}

/// Reports TeX82 §433-§437's `print_err`/`help2`/`int_error` recovery text.
///
/// The recovery itself belongs to the restricted scan (`tex_command`'s
/// `RestrictedIntegerClass`); only the terminal report is a stomach-side
/// effect, because the command core owns no `World` text sink.
/// Converts a command-core failure into its `ExecError` counterpart,
/// preserving the originating `CommandError` variant and message. Only
/// `MissingInput` and `PdfNavigation` map onto dedicated `ExecError` variants
/// shared with other producers; every other variant is carried through
/// verbatim via `ExecError::Command` so it names itself instead of collapsing
/// into a generic `MissingToken`. This match is written one arm per variant
/// (no wildcard) so adding a new `CommandError` variant fails to compile here
/// until it is explicitly handled.
pub(in crate::main_control) fn command_error(error: CommandError) -> ExecError {
    match error {
        CommandError::AtOrigin { error, origin } => {
            command_error(*error).capture_command_origin(origin)
        }
        CommandError::MissingInput {
            name,
            original_name,
        } => ExecError::MissingInput {
            name,
            original_name,
        },
        CommandError::MissingInputProbe(request) => ExecError::MissingInputProbe { request },
        CommandError::PdfNavigation(message) => ExecError::PdfNavigation(message),
        // §93 `succumb` is not a command failure to be re-described; it keeps
        // its own identity all the way up to the driver.
        CommandError::Fatal(fatal) => ExecError::Fatal(fatal),
        CommandError::FuelExhausted { .. }
        | CommandError::InputInvariant(_)
        | CommandError::StaleDelivery
        | CommandError::MacroPrefixMismatch
        | CommandError::ParagraphInMacroArgument
        | CommandError::OuterInMacroArgument
        | CommandError::UnsupportedExpandablePrimitive(_) => ExecError::Command(error),
    }
}
