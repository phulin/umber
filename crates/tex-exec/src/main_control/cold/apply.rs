//! Semantic application of typed cold operations.
//!
//! The handler mutates one admitted [`tex_state::CommandContext`], the
//! [`ModeNest`], and the [`PersistentInterpreter`] used by fused hot dispatch.
//! It owns no fallback executor and performs no input delivery.

use super::super::*;
use super::alignment::*;
use super::operation::*;
use super::pdf::*;
use super::support::*;

pub(in crate::main_control) fn enter_group<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut PersistentInterpreter<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    kind: GroupKind,
) {
    let entered_line = command.current_file_line_number();
    let frame = command
        .state_mut()
        .begin_group(stores, kind, entered_line)
        .expect("executor and command group stacks remain synchronized");
    let level = u32::try_from(stores.execution_group_depth()).unwrap_or(u32::MAX);
    stores.trace_group_enter(diagnostic_effects, frame.kind(), level, entered_line);
}

pub(in crate::main_control) fn leave_group_payloads<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut PersistentInterpreter<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    kind: GroupKind,
) -> Result<Vec<tex_state::token::TracedTokenWord>, tex_command::CommandGroupError> {
    let frame = stores
        .group_frames()
        .last()
        .copied()
        .ok_or(tex_command::CommandGroupError::NoOpenGroup)?;
    let level = u32::try_from(stores.execution_group_depth()).unwrap_or(u32::MAX);
    let closed = command.state_mut().end_group(stores, kind)?;
    // e-TeX [19.282--283]: each trace observes the already restored/retained
    // live word, and all of them precede §282's `\aftergroup` backups.
    crate::assignments::tracing::trace_group_restorations(
        stores,
        diagnostic_effects,
        closed.restorations(),
    );
    stores.trace_group_leave(
        diagnostic_effects,
        frame.kind(),
        level,
        frame.entered_line(),
    );
    Ok(closed.into_aftergroup())
}

#[allow(clippy::too_many_arguments)] // applies the complete canonical replay state atomically
pub(in crate::main_control) fn apply<G>(
    scanned: &mut PreparedColdCommand<G>,
    stores: tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    next_alignment_identity: &mut u64,
    active_alignment: &mut Option<ActiveReplayAlignment<G>>,
    command: &mut CommandMachine<'_, G>,
    boxes: &mut ReplayBoxes<G>,
    active_discretionaries: &[ActiveDiscretionary],
    active_math_choices: &[usize],
    active_math_fields: &mut Vec<ActiveMathFieldTarget>,
    active_math_left_boundaries: &[bool],
    active_math_shifts: &[MathShiftContext],
    prepared_dvi_pages: &mut PreparedDviPages,
) -> Result<ReplayStep, ExecError> {
    let mut stores = LinearCommandContext::new(stores);
    let stores = &mut stores;
    match scanned {
        ColdOperation::Continue => Ok(ReplayStep::Continue),
        ColdOperation::AlignmentTemplateEntered => {
            // TeX82 §§1034--1038's character lookahead calls `get_next`,
            // whose alignment interception inserts the v-template and then
            // resumes that same lookahead. Characters at the end of a cell
            // body can therefore ligate or kern with the v-template prefix.
            // The run ends at §1131's `endv` below, before `fin_col` advances
            // across a tab or span delimiter.
            let _active = active_alignment.as_mut().ok_or(ExecError::MissingToken {
                context: "active replay alignment",
            })?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Relax => {
            // TeX82 §1030 reaches §1045's do-nothing arm only after leaving
            // the ligature loop. The command itself has no list effect, but
            // it is still a word boundary: `?\\relax\\char96` must not form
            // the `?`` ligature across the relax.
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::TextDirection { direction, enabled } => {
            if *enabled {
                crate::box_runtime::flush_pending_hchars_with_fuel(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                )?;
                modes
                    .current_list_mutation()
                    .push(stores, Node::Direction(*direction));
            } else {
                let name = match direction {
                    tex_state::node::Direction::BeginM => "beginM",
                    tex_state::node::Direction::EndM => "endM",
                    tex_state::node::Direction::BeginL => "beginL",
                    tex_state::node::Direction::EndL => "endL",
                    tex_state::node::Direction::BeginR => "beginR",
                    tex_state::node::Direction::EndR => "endR",
                };
                // etex.ch's `eTeX_enabled`: one report for every optional
                // feature, so the help names the disabled feature generally
                // rather than the primitive the message already named.
                let context = command.state.output_open_context(&**stores);
                report_escaped_error(
                    stores,
                    command.diagnostic_effects,
                    "Improper ",
                    name,
                    "",
                    &["Sorry, this optional e-TeX feature has been disabled."],
                    context,
                )?;
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::AlignPeekRestart { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == *alignment)
                .ok_or(ExecError::MissingToken {
                    context: "alignment restart lookahead",
                })?;
            active.align_peek_pending = true;
            active.align_peek_after_noalign = true;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::MissingMathShift => {
            // TeX82 §1047's `insert_dollar_sign` diagnostic; the matching
            // input recovery (backing up the offending command behind an
            // inserted `$`) already ran in `recover_missing_math_shift`.
            let context = command.state.output_open_context(&**stores);
            crate::error_report::report_error(
                stores,
                command.diagnostic_effects,
                "Missing $ inserted",
                &[
                    "I've inserted a begin-math/end-math symbol since I think",
                    "you left one out. Proceed, with fingers crossed.",
                ],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::MissingAlignmentCr => {
            // TeX82 §1132 reaches §82's `ins_error` only after §325 has
            // backed up the brace and frozen `\cr` has been inserted. The
            // open context therefore labels the repair `<inserted text>` and
            // leaves the original brace below it as `<to be read again>`.
            let context = command.state.output_open_context(&**stores);
            crate::error_report::report_error(
                stores,
                command.diagnostic_effects,
                "Missing \\cr inserted",
                &["I'm guessing that you meant to end an alignment here."],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        // These are intercepted by `MainControl::apply_operation`, where
        // the owning opaque episode and mutable replay driver are available.
        ColdOperation::ReplayCompleted(_)
        | ColdOperation::Math(_)
        | ColdOperation::DisplayAlignmentRecovery
        | ColdOperation::MathDelimiter(_) => Ok(ReplayStep::Continue),
        ColdOperation::MathFamily {
            family,
            font,
            global,
        } => {
            AssignmentCommitter::new(stores, command.diagnostic_effects).try_unscoped(
                None,
                |stores| {
                    assign_math_family_font(
                        stores,
                        MathFontSize::from(family.size),
                        family.family,
                        *font,
                        *global,
                    )
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::EndOfInput => Ok(ReplayStep::EndOfInput),
        ColdOperation::End {
            dump,
            incomplete_conditions: _,
        } => {
            // TeX82 §1054 has accepted `its_all_over`; the page-builder-owned
            // replay fence must not enter terminal or INITEX format state.
            stores.finish_end_job();
            // §1335's final_cleanup tail -- closing every still-open paren,
            // reporting `incomplete_conditions`, the "(see the transcript
            // file..." note, and this same `dump` flag's
            // `(\dump is performed only by INITEX)` note, in that exact
            // order -- runs in `MainControl::end_of_job_final_cleanup`
            // once this returns: the paren close needs `self`'s job-framing
            // state, which this free function does not have, and tex.web
            // orders it first. `incomplete_conditions` is discarded here
            // rather than used, because the caller re-derives it from the
            // `ColdOperation::End` it matched before moving `scanned` into this
            // call (see `apply_operation`).
            //
            // TeX82 §1335's INITEX tail releases `last_glue` before
            // `store_fmt_file`; e-TeX 2.6's [45.999] change may meanwhile
            // have retained top-of-page glue, kerns, and penalties in
            // `page_discards`, which are deliberately absent from the
            // format. `its_all_over` (§1054) already proved that the page and
            // contribution lists contain no live material. Normalize the
            // remaining page-builder scalars while preserving both e-TeX
            // discard lists, so the host-side format encoder still rejects
            // genuine page material instead of mistaking `last_penalty` or
            // `last_node_type` for it.
            if *dump && command.initex && crate::page_output::job_is_all_over(stores) {
                stores.start_new_page();
            }
            // TeX82 §1378 closes every still-open numbered output file after
            // `final_cleanup`. The two normalized fallback selectors are not
            // file slots (§1342), so the state boundary exposes only 0..15
            // here. `close_out` preserves §1378's `if write_open[k]` guard:
            // never-opened and already-closed slots produce no close effect.
            // This runs here, synchronously with `\end`/`\dump` applying,
            // rather than in the driver-facing §1333 `finish_job` that prints
            // §642's DVI report: it is a `World` state effect, not a print,
            // and it has already happened by the time a driver can call
            // `finish_job` (which only runs after this step has already
            // returned), so its position here can't reorder anything
            // `finish_job` prints. Leaving it here also keeps it exactly
            // where `effects::tests::
            // output_stream_final_cleanup_closes_only_live_numbered_files`
            // already observes it, synchronous with the terminating step.
            for raw in 0..tex_state::world::STREAM_SLOT_COUNT as u8 {
                stores.close_output_stream(StreamSlot::new(raw));
            }
            Ok(ReplayStep::End)
        }
        ColdOperation::Count {
            index,
            value,
            global,
        } => {
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                .count(*index, *value, *global);
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Dimen {
            index,
            value,
            global,
        } => {
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                .dimension(*index, *value, *global);
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BoxDimensionAssignment {
            index,
            dimension,
            value,
            global,
        } => {
            // `Universe::set_box_dimension{,_global}` share one body: TeX82
            // §1055's `alter_box_dimen` mutates the visible box node
            // directly rather than through the save stack, so the assignment
            // prefix does not change which binding level is affected.
            let _ = global;
            AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(None, |stores| {
                assign_box_dimension(stores, *index, *dimension, *value)
            });
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Skip {
            index,
            value,
            global,
            redundant,
            reassigning,
            ..
        } => {
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).skip(
                *index,
                *value,
                *global,
                false,
                *redundant,
                *reassigning,
            );
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Muskip {
            index,
            value,
            global,
            redundant,
            reassigning,
            ..
        } => {
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).skip(
                *index,
                *value,
                *global,
                true,
                *redundant,
                *reassigning,
            );
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::HorizontalSkip { value } => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_paragraph(
                    command.state,
                    modes,
                    stores,
                    command.diagnostic_effects,
                    true,
                )?;
            }
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            modes.current_list_mutation().push(
                stores,
                Node::Glue {
                    spec: *value,
                    kind: GlueKind::Normal,
                    leader: None,
                },
            );
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Kern { amount } => {
            // TeX82 §1057's `any_mode(kern),mmode+mkern: append_kern`
            // (§1061: `tail_append(new_kern(cur_val)); subtype(tail):=s`).
            // Unlike `\hskip` (§1090's `head_for_vmode`, which is genuinely
            // `vmode+hskip`-listed), `\kern` has no mode-specific dispatch
            // entry at all -- it is legal in every mode and appends directly,
            // with no paragraph start and no page-builder call. The outer
            // vertical list is represented by the page contribution queue,
            // so it still uses the shared contribution splice (contrast
            // `\penalty`, §1103, which also calls `build_page` there).
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            crate::vertical::append_vertical_contribution(
                modes,
                stores,
                Node::Kern {
                    amount: *amount,
                    kind: KernKind::Explicit,
                },
            );
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Penalty { amount } => {
            // TeX82 §1103's `append_penalty`: `tail_append(new_penalty(cur_val))`
            // in whichever list is current, then `if mode=vmode then
            // build_page` -- i.e. only in *outer* vertical mode, not internal
            // vertical mode, matching `append_vertical_contribution`'s own
            // `is_outer_vertical` gate and (unlike `\vskip`'s `append_glue`,
            // §1057) always followed by a page-builder call in that case.
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            crate::vertical::append_vertical_contribution(modes, stores, Node::Penalty(*amount));
            crate::vertical::build_page_if_outer_vertical(
                modes,
                stores,
                command.diagnostic_effects,
                command.state.state(),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::DeleteLast { primitive, context } => {
            let state = &*command.state;
            crate::box_runtime::execute_delete_last(
                *primitive,
                |stores| {
                    state
                        .render_diagnostic_context(*context, stores)
                        .map_err(|_| ExecError::Command(tex_command::CommandError::StaleDelivery))
                },
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::SetInteractionMode(primitive) => {
            let mode = match primitive {
                UnexpandablePrimitive::BatchMode => tex_state::InteractionMode::Batch,
                UnexpandablePrimitive::NonstopMode => tex_state::InteractionMode::Nonstop,
                UnexpandablePrimitive::ScrollMode => tex_state::InteractionMode::Scroll,
                UnexpandablePrimitive::ErrorStopMode => tex_state::InteractionMode::ErrorStop,
                _ => unreachable!("only the four interaction-mode primitives are scanned"),
            };
            // TeX82 §1264's `new_interaction`: `print_ln` under the *old*
            // interaction mode's selector, unconditionally, before
            // `interaction:=cur_chr` takes effect. Skipping it left whichever
            // channel's column tracking stale until something else happened
            // to force a newline later -- invisible while every diagnostic
            // wrote both channels in lockstep, but a real divergence once
            // `\tracingonline<=0` redirects one channel alone (`umber2-
            // alfh.9`): the terminal's stale column then forces an extra,
            // unwanted newline into the log too, the first time anything
            // prints through the restored `term_and_log` selector.
            AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(None, |stores| {
                stores.printer().print_ln();
                stores.set_interaction_mode(mode);
            });
            Ok(ReplayStep::Continue)
        }
        ColdOperation::SetInteractionModeValue { value, context } => {
            let mode = match value {
                0 => tex_state::InteractionMode::Batch,
                1 => tex_state::InteractionMode::Nonstop,
                2 => tex_state::InteractionMode::Scroll,
                3 => tex_state::InteractionMode::ErrorStop,
                value => {
                    let context = render_diagnostic_coordinate(command, stores, *context)?;
                    crate::diagnostics::report_bad_interaction_mode_with_context(
                        stores,
                        command.diagnostic_effects,
                        *value,
                        context,
                    )?;
                    return Ok(ReplayStep::Continue);
                }
            };
            // See the sibling `SetInteractionMode` arm's comment.
            AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(None, |stores| {
                stores.printer().print_ln();
                stores.set_interaction_mode(mode);
            });
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ItalicCorrection => {
            match modes.current_mode() {
                Mode::Horizontal | Mode::RestrictedHorizontal => {
                    crate::box_runtime::append_italic_correction_with_fuel(
                        modes,
                        stores,
                        command.diagnostic_effects,
                        command.fuel,
                    )?;
                }
                Mode::Math | Mode::DisplayMath => {
                    // TeX82 §1112: `mmode+ital_corr: tail_append(new_kern(0));`
                    // -- `new_kern`'s default subtype (`normal`) is never
                    // overridden here (unlike hmode's italic-correction kern,
                    // or an explicit `\kern`), so it must not become a legal
                    // kern-then-glue line-break point.
                    modes.current_list_mutation().push(
                        stores,
                        Node::Kern {
                            amount: Scaled::from_raw(0),
                            kind: KernKind::Font,
                        },
                    );
                }
                Mode::Vertical | Mode::InternalVertical => {
                    unreachable!("vertical \\/ is scanned as IllegalItalicCorrection")
                }
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IllegalItalicCorrection { token } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IllegalMacroParameter { token } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ExtraEndCsName => {
            // TeX82 §1135's `cs_error`.
            let context = command.state.output_open_context(&**stores);
            report_escaped_error(
                stores,
                command.diagnostic_effects,
                "Extra ",
                "endcsname",
                "",
                &["I'm ignoring this, since I wasn't doing a \\csname."],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::NoBoundary { suppress_right } => {
            if *suppress_right {
                crate::box_runtime::flush_pending_hchars_without_right_boundary(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                )?;
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::NonScript => {
            // TeX82 §1171: a zero glue with the `cond_math_glue` subtype.
            modes.current_list_mutation().push(
                stores,
                Node::Glue {
                    spec: GlueSpec::ZERO,
                    kind: GlueKind::NonScript,
                    leader: None,
                },
            );
            Ok(ReplayStep::Continue)
        }
        ColdOperation::CharacterCode {
            value,
            origin,
            suppress_left_boundary,
        } => {
            let ch = u32::try_from(*value).ok().and_then(char::from_u32).ok_or(
                ExecError::InvalidCode {
                    context: "\\char",
                    value: *value,
                },
            )?;
            if matches!(modes.current_mode(), Mode::Math | Mode::DisplayMath) {
                // TeX82 `main_control`'s `mmode+char_num` (§1154) scans the
                // character number and then calls `set_math_char` (§1155)
                // with its `math_code`, exactly like the sibling
                // `mmode+letter`/`mmode+other_char`/`mmode+char_given` cases:
                // it appends a math-char noad and never begins or continues
                // a horizontal list from math mode.
                set_math_char(ch, *origin, stores, modes, command)?;
                return Ok(ReplayStep::Continue);
            }
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_paragraph(
                    command.state,
                    modes,
                    stores,
                    command.diagnostic_effects,
                    true,
                )?;
            }
            modes
                .current_list_mutation()
                .set_no_boundary(*suppress_left_boundary);
            crate::box_runtime::append_character_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                ch,
                *origin,
                command.state.profile() == CommandProfile::ETEX26,
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ControlSpace => {
            match modes.current_mode() {
                Mode::Math | Mode::DisplayMath => {
                    // TeX82 §1030's `mmode+ex_space: goto append_normal_space`
                    // (§1041) appends real interword glue in math mode, unlike
                    // an ordinary `mmode+spacer`, which §1045 makes a no-op.
                    let spec = crate::box_runtime::control_space_glue_spec(stores);
                    modes.current_list_mutation().push(
                        stores,
                        Node::Glue {
                            spec,
                            kind: GlueKind::Normal,
                            leader: None,
                        },
                    );
                }
                Mode::Vertical | Mode::InternalVertical => {
                    start_paragraph(
                        command.state,
                        modes,
                        stores,
                        command.diagnostic_effects,
                        true,
                    )?;
                    crate::box_runtime::append_control_space_with_fuel(
                        modes,
                        stores,
                        command.diagnostic_effects,
                        command.fuel,
                    )?;
                }
                _ => {
                    crate::box_runtime::append_control_space_with_fuel(
                        modes,
                        stores,
                        command.diagnostic_effects,
                        command.fuel,
                    )?;
                }
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PrevDepth { value } => {
            debug_assert!(matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ));
            AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(None, |_| {
                modes.current_list_mutation().set_prev_depth(*value)
            });
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IllegalPrevDepth { token } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::SpaceFactor { value } => {
            debug_assert!(matches!(
                modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ));
            // TeX82 §1243's `alter_aux`: `if (cur_val<=0)or(cur_val>32767)
            // then int_error(cur_val) else space_factor:=cur_val` -- an
            // out-of-range value is diagnosed and left unchanged rather than
            // clamped.
            if (1..=32767).contains(value) {
                AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(None, |_| {
                    modes.current_list_mutation().set_space_factor(*value);
                });
            } else {
                // §91's `int_error` appends ` (value)` to the message before
                // §82 completes the report, so the value is not part of the
                // `print_err` text.
                let context = command.state.output_open_context(&**stores);
                let mut report = stores.print_err("Bad space factor");
                report
                    .help(&["I allow only values in the range 1..32767 here."])
                    .context(context);
                report
                    .int_error(*value)
                    .defer_recovery(command.diagnostic_effects)?;
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IllegalSpaceFactor { token } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PrevGraf { value } => {
            // TeX82 §1244's `alter_prev_graf`: `\prevgraf` is `any_mode` (it
            // walks the mode nest up to its nearest enclosing vertical level
            // rather than checking the current mode), unlike `\spacefactor`/
            // `\prevdepth`'s §1243 `report_illegal_case`.
            if *value < 0 {
                let context = command.state.output_open_context(&**stores);
                let mut report = stores.print_err("Bad ");
                report
                    .print_esc("prevgraf")
                    .help(&["I allow only nonnegative values here."])
                    .context(context);
                report
                    .int_error(*value)
                    .defer_recovery(command.diagnostic_effects)?;
            } else {
                AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(None, |_| {
                    modes.set_enclosing_vertical_prev_graf(*value);
                });
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PageDimension { dimension, value } => {
            // TeX82 §1245's `alter_page_so_far`: a direct
            // `page_so_far[c]:=cur_val` store with no mode check, no
            // diagnostic, and no save-stack entry (§1242: "these definitions
            // are always global"). The page builder reads the same slots.
            AssignmentCommitter::new(stores, command.diagnostic_effects)
                .unscoped(None, |stores| stores.set_page_dimension(*dimension, *value));
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PageInteger { integer, value } => {
            // TeX82 §1246's `alter_integer`, scoped exactly like
            // `alter_page_so_far` above. `\deadcycles` in particular is what
            // §1024's output-routine loop guard compares against
            // `\maxdeadcycles`, so a wrong value here is only visible once a
            // page ships.
            AssignmentCommitter::new(stores, command.diagnostic_effects)
                .unscoped(None, |stores| stores.set_page_integer(*integer, *value));
            Ok(ReplayStep::Continue)
        }
        ColdOperation::FixedHorizontalGlue { primitive } => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_paragraph(
                    command.state,
                    modes,
                    stores,
                    command.diagnostic_effects,
                    true,
                )?;
            }
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            modes.current_list_mutation().push(
                stores,
                Node::Glue {
                    spec: crate::box_runtime::fixed_infinite_glue(*primitive),
                    kind: GlueKind::Normal,
                    leader: None,
                },
            );
            Ok(ReplayStep::Continue)
        }
        ColdOperation::VerticalSkip { value } => {
            // TeX82 §1054's `vmode+vskip: append_glue` (§1057): unlike
            // `\hskip` in vertical mode, `\vskip` never starts a paragraph --
            // the scan side (`scan_command`) only produces this step when the
            // mode is already `Vertical` or `InternalVertical`. §1057 also
            // notes `append_glue` deliberately never calls `build_page`
            // itself ("it is used in at least one place where that would be
            // a mistake"), unlike `append_penalty` (§1103); no page build
            // follows here.
            crate::box_runtime::append_node_to_current_list(
                modes,
                stores,
                command.diagnostic_effects,
                Node::Glue {
                    spec: *value,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::FixedVerticalGlue { primitive } => {
            // See `ColdOperation::VerticalSkip` above: same §1054/§1057
            // `append_glue`, no paragraph start, no page build.
            let spec = crate::box_runtime::fixed_infinite_glue(*primitive);
            crate::box_runtime::append_node_to_current_list(
                modes,
                stores,
                command.diagnostic_effects,
                Node::Glue {
                    spec,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ParagraphIndent { indent } => {
            // TeX82 §1090 routes only `vmode+start_par` to §1091 `new_graf`,
            // the single site that pushes `\everypar`. §1092 routes both
            // `hmode+start_par` and `mmode+start_par` to §1093
            // `indent_in_hmode`, which appends the paragraph-indent box (as
            // an ordinary `sub_box` noad in math mode) without beginning a
            // paragraph -- so an `\indent` inside a paragraph already under
            // way must not replay `\everypar` a second time.
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_paragraph(
                    command.state,
                    modes,
                    stores,
                    command.diagnostic_effects,
                    *indent,
                )?;
            } else {
                crate::box_runtime::indent_in_hmode(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    *indent,
                    command.fuel,
                )?;
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ParagraphShape { lines, global } => {
            // TeX82 §1214's "Adjust for the setting of `\globaldefs`" runs
            // unconditionally before `prefixed_command`'s `case cur_cmd of
            // @<Assignments@> endcases`, so it applies uniformly to all
            // thirty assignment forms §1210 dispatches -- `set_shape`
            // (§1248's `define(par_shape_loc,shape_ref,p)`) among them,
            // since `\parshape` is an ordinary `eqtb` entry that `define`
            // scopes through the save stack. This was the third canonical
            // apply arm (after `\def`/`\edef`/`\gdef`/`\xdef` and
            // `\let`/`\futurelet`) that passed the raw `\global` prefix bit
            // straight through and silently ignored a nonzero
            // `\globaldefs`; it was missed by both earlier sweeps because
            // `set_shape` belongs to neither definition family.
            AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(None, |stores| {
                stores
                    .assign_paragraph_shape(lines, assignment_scope(*global))
                    .expect("paragraph shape fits admitted durable storage")
            });
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PenaltyArray {
            kind,
            values,
            global,
        } => {
            // e-TeX 2.6 change [49.1248] commits every selector through
            // `define(q, shape_ref, p)`, so this uses the same save-stack and
            // `\globaldefs`-adjusted scope bit as TeX82 §1214/§1248.
            let record = MutationRecord {
                target: MutationTarget::Register,
                key: ObservationValue::Name(format!(
                    "toks:{}",
                    match kind {
                        PenaltyArrayKind::InterLine => 256,
                        PenaltyArrayKind::Club => 257,
                        PenaltyArrayKind::Widow => 258,
                        PenaltyArrayKind::DisplayWidow => 259,
                    }
                )),
                value: ObservationValue::Tokens(Vec::new()),
                global: *global,
            };
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                .unscoped_with_effects(Some(record), |stores, diagnostic_effects| {
                    let old = stores.penalty_array(*kind);
                    stores
                        .assign_penalty_array(*kind, values, assignment_scope(*global))
                        .expect("penalty array fits admitted durable storage");
                    assignment_tracing::trace_penalty_array(
                        stores,
                        diagnostic_effects,
                        *kind,
                        *global,
                        &old,
                        values,
                    );
                });
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Toks {
            index,
            tokens,
            global,
            ..
        } => {
            let new = tokens.as_ref().map(|tokens| tokens.prepared());
            let observed = ObservationValue::Tokens(
                new.map(|new| {
                    stores
                        .token_list(new.clone())
                        .iter()
                        .map(|word| observed_macro_token(word.semantic_token(), stores))
                        .collect()
                })
                .unwrap_or_default(),
            );
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).toks(
                *index,
                tokens.as_mut().map(OperationTokenRoot::take_prepared),
                observed,
                *global,
            );
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IntParam {
            index,
            value,
            global,
        } => {
            let key = parameter_mutation_key_for_dialect(
                command.state.profile().dialect(),
                ParameterClass::Integer,
                *index,
            );
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                .int_parameter(*index, *value, key, *global);
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::DimenParam {
            index,
            value,
            global,
        } => {
            let key = parameter_mutation_key_for_dialect(
                command.state.profile().dialect(),
                ParameterClass::Dimension,
                *index,
            );
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                .dimension_parameter(*index, *value, key, *global);
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::TokParam {
            index,
            tokens,
            global,
            ..
        } => {
            let new = tokens.as_ref().map(|tokens| tokens.prepared());
            let observed = ObservationValue::Tokens(
                new.map(|tokens| {
                    stores
                        .token_list(tokens.clone())
                        .iter()
                        .map(|word| observed_macro_token(word.semantic_token(), stores))
                        .collect()
                })
                .unwrap_or_default(),
            );
            let key = parameter_mutation_key_for_dialect(
                command.state.profile().dialect(),
                ParameterClass::Token,
                *index,
            );
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                .token_parameter(
                    *index,
                    tokens.as_mut().map(OperationTokenRoot::take_prepared),
                    observed,
                    key,
                    *global,
                );
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::GlueParam {
            index,
            value,
            global,
        } => {
            let key = parameter_mutation_key_for_dialect(
                command.state.profile().dialect(),
                ParameterClass::Glue,
                *index,
            );
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                .glue_parameter(*index, *value, key, *global);
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfFontCode {
            table,
            font,
            character,
            value,
        } => {
            let record = MutationRecord {
                target: MutationTarget::Register,
                key: ObservationValue::Name(format!("{table:?}:{}:{character}", font.raw())),
                value: ObservationValue::Integer(i64::from(*value)),
                global: true,
            };
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                .unscoped(Some(record), |stores| {
                    stores.set_pdf_font_code(*table, *font, *character, *value)
                });
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfNoLigatures { font } => {
            let record = MutationRecord {
                target: MutationTarget::Register,
                key: ObservationValue::Name("pdf_no_ligatures".into()),
                value: ObservationValue::Name(font.raw().to_string()),
                global: true,
            };
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                .unscoped(Some(record), |stores| {
                    stores.disable_pdf_font_ligatures(*font)
                });
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::CodeTable {
            primitive,
            character,
            value,
            global,
        } => {
            let mut value = *value;
            // TeX82 §§1230--1234's `def_code` reports an invalid table
            // value after scanning it, substitutes zero, and commits the
            // assignment. In particular, this is not an operation failure:
            // rolling the command snapshot back would reread the operand and
            // lose ownership of the following input.
            let maximum = match primitive {
                UnexpandablePrimitive::CatCode => 15,
                UnexpandablePrimitive::LcCode | UnexpandablePrimitive::UcCode => 255,
                UnexpandablePrimitive::SfCode => 32_767,
                UnexpandablePrimitive::MathCode => 32_768,
                UnexpandablePrimitive::DelCode => 0xFF_FFFF,
                _ => unreachable!("only code-table primitives are scanned"),
            };
            let valid = (0..=maximum).contains(&value)
                || (*primitive == UnexpandablePrimitive::DelCode && value == -1);
            if !valid {
                let context = command.state.output_open_context(&**stores);
                let mut report = stores.print_err("Invalid code (");
                report
                    .print_int(value)
                    .print("), should be in the range 0..")
                    .print_int(maximum)
                    .help(&["I changed this one to zero."])
                    .context(context);
                report.error().defer_recovery(command.diagnostic_effects)?;
                value = 0;
            }
            match primitive {
                UnexpandablePrimitive::CatCode => {
                    unreachable!("measured catcode assignments are owned by hot_apply")
                }
                UnexpandablePrimitive::LcCode => {
                    let value = u32::try_from(value).map_err(|_| ExecError::InvalidCode {
                        context: "\\lccode",
                        value,
                    })? as LcCode;
                    let old = stores.lccode(*character);
                    let record = code_table_mutation("lccode", *character, value as i64, *global);
                    let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                        .scoped_word(
                            old,
                            value,
                            *global,
                            record,
                            |stores, global| {
                                stores
                                    .assign_code(
                                        tex_state::env::CodeTableKind::Lccode,
                                        *character,
                                        i64::from(value),
                                        assignment_scope(global),
                                    )
                                    .expect("lccode target belongs to admitted state")
                            },
                            |stores, diagnostic_effects, _| {
                                assignment_tracing::trace_code(
                                    stores,
                                    diagnostic_effects,
                                    "lccode",
                                    *character,
                                    *global,
                                    old as i32,
                                    value as i32,
                                )
                            },
                        );
                    command.retain_assignment_receipt(receipt);
                }
                UnexpandablePrimitive::UcCode => {
                    let value = checked_character_code(value, "\\uccode")? as UcCode;
                    let old = stores.uccode(*character);
                    let record = code_table_mutation("uccode", *character, value as i64, *global);
                    let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                        .scoped_word(
                            old,
                            value,
                            *global,
                            record,
                            |stores, global| {
                                stores
                                    .assign_code(
                                        tex_state::env::CodeTableKind::Uccode,
                                        *character,
                                        i64::from(value),
                                        assignment_scope(global),
                                    )
                                    .expect("uccode target belongs to admitted state")
                            },
                            |stores, diagnostic_effects, _| {
                                assignment_tracing::trace_code(
                                    stores,
                                    diagnostic_effects,
                                    "uccode",
                                    *character,
                                    *global,
                                    old as i32,
                                    value as i32,
                                )
                            },
                        );
                    command.retain_assignment_receipt(receipt);
                }
                UnexpandablePrimitive::SfCode => {
                    let value = u16::try_from(value)
                        .ok()
                        .filter(|value| *value <= 32_767)
                        .ok_or(ExecError::InvalidCode {
                            context: "\\sfcode",
                            value,
                        })? as SfCode;
                    let old = stores.sfcode(*character);
                    let record =
                        code_table_mutation("sfcode", *character, i64::from(value), *global);
                    let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                        .scoped_word(
                            old,
                            value,
                            *global,
                            record,
                            |stores, global| {
                                stores
                                    .assign_code(
                                        tex_state::env::CodeTableKind::Sfcode,
                                        *character,
                                        i64::from(value),
                                        assignment_scope(global),
                                    )
                                    .expect("sfcode target belongs to admitted state")
                            },
                            |stores, diagnostic_effects, _| {
                                assignment_tracing::trace_code(
                                    stores,
                                    diagnostic_effects,
                                    "sfcode",
                                    *character,
                                    *global,
                                    i32::from(old),
                                    i32::from(value),
                                )
                            },
                        );
                    command.retain_assignment_receipt(receipt);
                }
                UnexpandablePrimitive::MathCode => {
                    let value = u32::try_from(value)
                        .ok()
                        .filter(|value| *value <= 32_768)
                        .ok_or(ExecError::InvalidCode {
                            context: "\\mathcode",
                            value,
                        })? as MathCode;
                    let old = stores.mathcode(*character);
                    let record = code_table_mutation("mathcode", *character, value as i64, *global);
                    let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                        .scoped_word(
                            old,
                            value,
                            *global,
                            record,
                            |stores, global| {
                                stores
                                    .assign_code(
                                        tex_state::env::CodeTableKind::Mathcode,
                                        *character,
                                        i64::from(value),
                                        assignment_scope(global),
                                    )
                                    .expect("mathcode target belongs to admitted state")
                            },
                            |stores, diagnostic_effects, _| {
                                assignment_tracing::trace_code(
                                    stores,
                                    diagnostic_effects,
                                    "mathcode",
                                    *character,
                                    *global,
                                    old as i32,
                                    value as i32,
                                )
                            },
                        );
                    command.retain_assignment_receipt(receipt);
                }
                UnexpandablePrimitive::DelCode => {
                    let value = (-1..=0xFF_FFFF)
                        .contains(&value)
                        .then_some(value as DelCode)
                        .ok_or(ExecError::InvalidCode {
                            context: "\\delcode",
                            value,
                        })?;
                    let old = stores.delcode(*character);
                    let record =
                        code_table_mutation("delcode", *character, i64::from(value), *global);
                    let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                        .scoped_word(
                            old,
                            value,
                            *global,
                            record,
                            |stores, global| {
                                stores
                                    .assign_code(
                                        tex_state::env::CodeTableKind::Delcode,
                                        *character,
                                        i64::from(value),
                                        assignment_scope(global),
                                    )
                                    .expect("delcode target belongs to admitted state")
                            },
                            |stores, diagnostic_effects, _| {
                                assignment_tracing::trace_code(
                                    stores,
                                    diagnostic_effects,
                                    "delcode",
                                    *character,
                                    *global,
                                    old,
                                    value,
                                )
                            },
                        );
                    command.retain_assignment_receipt(receipt);
                }
                _ => unreachable!("only code-table primitives are scanned"),
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::FontSelect {
            font,
            _selector: _,
            global,
        } => {
            AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(None, |stores| {
                stores
                    .assign_current_font(*font, assignment_scope(*global))
                    .expect("current font belongs to admitted state")
            });
            Ok(ReplayStep::Continue)
        }
        ColdOperation::FontDefinition { request, global } => {
            // TeX82 §1257 records `font_id_text(f):=t` at `common_ending`,
            // including the `f=null_font` recovery path. Active targets use
            // the synthesized string `FONT<char>` rather than their bare
            // control-sequence text.
            let identifier = font_identifier_for_definition(stores, request.target);
            let observe_font_definition = command.state.profile().capabilities().supports_etex();
            // TeX82 §1258/§1259 report an illegal `at`/`scaled` size and
            // continue with the replaced value; §1257 then loads the font
            // normally. The replacement is the scanner's, the report this
            // seam's.
            if let Some(recovery) = &request.size_recovery {
                report_font_size_recovery(stores, command.diagnostic_effects, recovery)?;
            }
            let bind_null_font =
                |stores: &mut tex_state::CommandContext<'_, G>,
                 diagnostic_effects: &mut DiagnosticEffects| {
                    let record = font_definition_mutation(
                        stores,
                        request.target,
                        *global,
                        observe_font_definition,
                    );
                    AssignmentCommitter::new(stores, diagnostic_effects).unscoped(
                        record,
                        |stores| {
                            stores
                                .assign_resolved_meaning(
                                    request.target,
                                    tex_state::meaning::ResolvedMeaning::Static(Meaning::Font(
                                        tex_state::font::NULL_FONT,
                                    )),
                                    assignment_scope(*global),
                                )
                                .expect("font selector belongs to admitted state");
                            stores
                                .set_font_identifier_symbol(tex_state::font::NULL_FONT, identifier);
                        },
                    )
                };
            let path = crate::canonical_font_resource_path(&request.name);
            let resource = command
                .capabilities
                .font(&path)
                .expect("font resource is resolved after the processor borrow");
            if matches!(resource, FontResource::Unavailable) {
                // TeX.web §§1257/561 diagnose the failed TFM open before
                // continuing with the selector bound to `null_font`. The
                // scanner has already backed up the delimiter token, so the
                // live input context also shows the command that follows the
                // font specification.
                let selector = stores.resolve(request.target).to_owned();
                let selector_kind = stores.control_sequence_kind(request.target);
                report_font_not_loadable_with_context(
                    stores,
                    command.diagnostic_effects,
                    selector_kind,
                    &selector,
                    &request.name,
                    request.size,
                    if request.name.starts_with("opentype:") {
                        FontLoadFailure::MissingOpenType
                    } else {
                        FontLoadFailure::MissingTfm
                    },
                    request.error_context.clone(),
                )?;
                let receipt = bind_null_font(stores, command.diagnostic_effects);
                command.retain_assignment_receipt(receipt);
                return Ok(ReplayStep::Continue);
            }
            let loaded = match load_font(request, resource) {
                Ok(loaded) => loaded,
                Err(ExecError::FontParse(_)) => {
                    // TeX.web §564 treats malformed metrics exactly like the
                    // recoverable not-loadable path. The fulfilled resource
                    // remains retained by the host; only this definition is
                    // replaced by nullfont with its requested assignment scope.
                    let selector = stores.resolve(request.target).to_owned();
                    let selector_kind = stores.control_sequence_kind(request.target);
                    report_font_not_loadable_with_context(
                        stores,
                        command.diagnostic_effects,
                        selector_kind,
                        &selector,
                        &request.name,
                        request.size,
                        FontLoadFailure::MalformedTfm,
                        request.error_context.clone(),
                    )?;
                    let receipt = bind_null_font(stores, command.diagnostic_effects);
                    command.retain_assignment_receipt(receipt);
                    return Ok(ReplayStep::Continue);
                }
                Err(error) => return Err(error),
            };
            let mut loaded = Some(loaded);
            let id = match stores.try_intern_font_with_identifier(&mut loaded, identifier) {
                Ok(id) => id,
                Err(_) => {
                    let selector = stores.resolve(request.target).to_owned();
                    let selector_kind = stores.control_sequence_kind(request.target);
                    report_font_capacity(
                        stores,
                        command.diagnostic_effects,
                        selector_kind,
                        &selector,
                        &request.name,
                        request.size,
                        request.error_context.clone(),
                    )?;
                    let receipt = bind_null_font(stores, command.diagnostic_effects);
                    command.retain_assignment_receipt(receipt);
                    return Ok(ReplayStep::Continue);
                }
            };
            // Web2C tex.ch [49.1260] removes TeX82 §1254's `flush_string`:
            // [29.517]'s `slow_make_string` may have returned an older pool
            // string, so flushing it would retire an unrelated allocation.
            let record =
                font_definition_mutation(stores, request.target, *global, observe_font_definition);
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(
                record,
                |stores| {
                    stores
                        .assign_resolved_meaning(
                            request.target,
                            tex_state::meaning::ResolvedMeaning::Static(Meaning::Font(id)),
                            assignment_scope(*global),
                        )
                        .expect("font selector belongs to admitted state")
                },
            );
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::GeneratedFontDefinition { definition, global } => {
            if definition.kind == GeneratedFontKind::Copy
                && matches!(
                    stores.font_construction(definition.source),
                    tex_fonts::FontConstruction::Letterspaced { .. }
                        | tex_fonts::FontConstruction::Expanded { .. }
                )
            {
                let reason = match stores.font_construction(definition.source) {
                    tex_fonts::FontConstruction::Expanded { .. } => "cannot copy an expanded font",
                    _ => "cannot copy a letterspaced font",
                };
                return Err(ExecError::CannotCopyFont(reason));
            }
            let id = match definition.kind {
                GeneratedFontKind::Copy => {
                    stores.try_copy_font_with_identifier(definition.source, definition.target)?
                }
                GeneratedFontKind::Letterspace => {
                    let zero_em = stores.font_parameter(definition.source, 6).raw() == 0;
                    let id = stores.try_letterspace_font_with_identifier(
                        definition.source,
                        definition.target,
                        definition.amount,
                        definition.no_ligatures,
                    )?;
                    if zero_em {
                        stores.printer().print_rendered(
                            "\npdfTeX warning (\\letterspacefont): font has zero em size (\\fontdimen6)\n",
                        );
                    }
                    id
                }
            };
            let record = font_definition_mutation(stores, definition.target, *global, true);
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(
                record,
                |stores| {
                    stores
                        .assign_resolved_meaning(
                            definition.target,
                            tex_state::meaning::ResolvedMeaning::Static(Meaning::Font(id)),
                            assignment_scope(*global),
                        )
                        .expect("generated-font selector belongs to admitted state")
                },
            );
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::InputStream {
            request,
            resource: _,
        } => {
            match request {
                RootedInputStreamRequest::Open { .. } | RootedInputStreamRequest::Close { .. } => {
                    unreachable!("input open/close executes at the MainControl outer barrier")
                }
                // TeX82 §482 has already collected the list inside the
                // command core, which also reported §1225's missing-`to`
                // recovery at the point tex.web reports it; the definition is
                // all that is left.
                RootedInputStreamRequest::Read {
                    target,
                    global,
                    definition,
                    ..
                } => {
                    // TeX82 §1225 installs `read_toks`'s freshly allocated
                    // macro through `define(p,call,cur_val)`, so e-TeX
                    // [17.687-750] traces the same pre/post eqtb write as a
                    // `\def`, immediately after collection and before the
                    // next command is fetched.
                    let observed =
                        ObservationValue::Tokens(observed_read_body(definition.prepared(), stores));
                    let record = MutationRecord {
                        target: MutationTarget::Meaning,
                        key: ObservationValue::Name(stores.resolve(*target).to_owned()),
                        value: observed,
                        global: *global,
                    };
                    let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects)
                        .unscoped_with_effects(Some(record), |stores, diagnostic_effects| {
                            assignment_tracing::trace_meaning_write(
                                stores,
                                diagnostic_effects,
                                Token::Cs(*target),
                                true,
                                *global,
                                |stores| {
                                    stores
                                        .assign_resolved_meaning(
                                            *target,
                                            tex_state::meaning::ResolvedMeaning::Macro {
                                                flags: MeaningFlags::EMPTY,
                                                definition: definition.take_prepared(),
                                            },
                                            assignment_scope(*global),
                                        )
                                        .expect(
                                            "prepared read definition belongs to admitted state",
                                        )
                                },
                            );
                        });
                    command.retain_assignment_receipt(receipt);
                }
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfXImage { request, resource } => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfximage"));
            }
            let source = match std::mem::replace(resource, PdfImageResource::Unavailable) {
                PdfImageResource::Available(source) => source,
                PdfImageResource::Unavailable => {
                    return Err(ExecError::PdfImageOpen {
                        name: std::mem::take(&mut request.name),
                        message: "image is unavailable".to_owned(),
                    });
                }
                PdfImageResource::Invalid(message) => {
                    return Err(ExecError::PdfImageOpen {
                        name: std::mem::take(&mut request.name),
                        message,
                    });
                }
            };
            let dimensions =
                pdf_image_dimensions(&source, request.width, request.height, request.depth);
            stores
                .allocate_pdf_external_image(source, dimensions, request.color_space_object)
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfRefXImage { object } => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfrefximage"));
            }
            let image = u32::try_from(*object)
                .ok()
                .and_then(|raw| tex_state::PdfExternalImageId::new(raw).ok())
                .and_then(|id| stores.pdf_external_image_record(id))
                .ok_or(ExecError::PdfReferencedObjectNotFound)?;
            let dimensions = image.dimensions();
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::PdfRefXImage {
                    object: image.id().raw(),
                    width: dimensions.width,
                    height: dimensions.height,
                    depth: dimensions.depth,
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfSetRandomSeed { .. } | ColdOperation::PdfResetTimer => {
            unreachable!("PDF clock and random operations execute at the MainControl outer barrier")
        }
        ColdOperation::PdfInterwordSpace(control) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                let name = match control {
                    tex_state::node::PdfAccessibilityControl::InterwordSpaceOn => {
                        "pdfinterwordspaceon"
                    }
                    tex_state::node::PdfAccessibilityControl::InterwordSpaceOff => {
                        "pdfinterwordspaceoff"
                    }
                    tex_state::node::PdfAccessibilityControl::FakeSpace => "pdffakespace",
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::PdfAccessibility(*control),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfRunningLink(enabled) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode(if *enabled {
                    "pdfrunninglinkon"
                } else {
                    "pdfrunninglinkoff"
                }));
            }
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::PdfRunningLink(*enabled),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfSpaceFont(tokens) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfspacefont"));
            }
            let name = pdf_graphics_text(tokens.prepared(), stores);
            stores.set_pdf_space_font_name(name);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfGraphics(request) => apply_pdf_graphics_request(
            request,
            stores,
            modes,
            command.state,
            command.diagnostic_effects,
            command.fuel,
        ),
        ColdOperation::PdfNavigation(request) => apply_pdf_navigation_request(
            request,
            stores,
            modes,
            command.diagnostic_effects,
            command.fuel,
        ),
        ColdOperation::PdfObject(request) => apply_pdf_object_request(request, stores, false),
        ColdOperation::PdfReferenceObject(request) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfrefobj"));
            }
            let object = u32::try_from(request.object)
                .ok()
                .filter(|object| stores.pdf_raw_object(*object).is_some())
                .ok_or(ExecError::PdfReferencedObjectNotFound)?;
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::PdfReferenceObject { object },
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfForm(request) => {
            apply_pdf_form_request(request, stores, modes, command, false).map(|pending| {
                debug_assert!(pending.is_none());
                ReplayStep::Continue
            })
        }
        ColdOperation::PdfDocumentFragment(request) => {
            let dvi_only_error = matches!(request.kind, tex_state::PdfDocumentFragmentKind::Names);
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                if dvi_only_error {
                    return Err(ExecError::PdfExtensionInDviMode("pdfnames"));
                }
                let name = match request.kind {
                    tex_state::PdfDocumentFragmentKind::Info => "pdfinfo",
                    tex_state::PdfDocumentFragmentKind::Catalog => "pdfcatalog",
                    tex_state::PdfDocumentFragmentKind::Trailer => "pdftrailer",
                    tex_state::PdfDocumentFragmentKind::TrailerId => "pdftrailerid",
                    tex_state::PdfDocumentFragmentKind::Names => {
                        unreachable!("pdfnames is rejected before the ignored-fragment warning")
                    }
                };
                stores.printer().print_rendered(&format!(
                        "\npdfTeX warning (\\{name}): not allowed in DVI mode (\\pdfoutput <= 0); ignoring it\n"
                    ));
                return Ok(ReplayStep::Continue);
            }
            stores.append_pdf_document_fragment(request.kind, request.text.tokens.take_prepared());
            if let Some(action) = &mut request.open_action {
                let action = take_admitted_pdf_action(action);
                if stores.pdf_catalog_open_action().is_some() {
                    return Err(ExecError::PdfDuplicateOpenAction);
                }
                let (destination, structure, thread) =
                    pdf_action_target_identities(stores, &action);
                stores
                    .set_pdf_catalog_open_action_with_targets(
                        action,
                        destination,
                        structure,
                        thread,
                    )
                    .map_err(|_| ExecError::PdfObjectCapacity)?;
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfFontExpand { font, spec } => {
            stores.configure_font_expansion(
                *font,
                tex_state::font::FontExpansion {
                    stretch: spec.stretch() as u16,
                    shrink: spec.shrink() as u16,
                    step: spec.step() as u8,
                    auto_expand: spec.auto_expand(),
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::PdfFontAction {
            primitive,
            font,
            first,
            second,
        } => {
            let first = first
                .as_ref()
                .map(|tokens| pdf_graphics_text(tokens.prepared(), stores));
            match primitive {
                UnexpandablePrimitive::PdfFontAttr => stores.set_pdf_font_attribute(
                    font.expect("font attribute scanned a font"),
                    first.expect("font attribute scanned text"),
                ),
                UnexpandablePrimitive::PdfIncludeChars => stores.include_pdf_font_chars(
                    font.expect("include chars scanned a font"),
                    first.expect("include chars scanned text"),
                ),
                UnexpandablePrimitive::PdfNoBuiltinToUnicode => stores
                    .disable_pdf_builtin_to_unicode(
                        font.expect("no builtin ToUnicode scanned a font"),
                    ),
                UnexpandablePrimitive::PdfGlyphToUnicode => {
                    let glyph = first.expect("glyph mapping scanned a glyph");
                    let unicode = pdf_graphics_text(
                        second
                            .as_ref()
                            .expect("glyph mapping scanned a Unicode value")
                            .prepared(),
                        stores,
                    );
                    match parse_glyph_to_unicode(&glyph, &unicode) {
                        GlyphToUnicodeParse::Mapping(mapping) => {
                            stores.set_pdf_glyph_to_unicode(mapping)
                        }
                        GlyphToUnicodeParse::Warning(message) => {
                            stores.printer().print_rendered(&format!(
                                "\npdfTeX warning: pdftex: ToUnicode: {message}\n"
                            ));
                        }
                    }
                }
                UnexpandablePrimitive::PdfMapFile => {
                    let bytes = first.expect("map file scanned text");
                    if bytes.iter().all(u8::is_ascii_whitespace) {
                        stores.push_pdf_font_map(tex_state::PdfFontMapOperation::BlockDefault);
                    } else {
                        stores.push_pdf_font_map(tex_state::PdfFontMapOperation::File(
                            tex_fonts::PdfFontMapFile::parse(&bytes)?,
                        ));
                    }
                }
                UnexpandablePrimitive::PdfMapLine => {
                    let bytes = first.expect("map line scanned text");
                    if bytes.iter().all(u8::is_ascii_whitespace) {
                        stores.push_pdf_font_map(tex_state::PdfFontMapOperation::BlockDefault);
                    } else {
                        let duplicate_count = stores.pdf_font_map_duplicate_names().len();
                        stores.push_pdf_font_map(tex_state::PdfFontMapOperation::Line(
                            tex_fonts::PdfFontMapEntry::parse(&bytes)?,
                        ));
                        let duplicates = stores.pdf_font_map_duplicate_names();
                        if duplicates.len() > duplicate_count
                            && stores.int_param(IntParam::PDF_SUPPRESS_WARNING_DUP_MAP) <= 0
                        {
                            let name = String::from_utf8_lossy(
                                duplicates.last().expect("new duplicate has a name"),
                            );
                            stores.printer().print_rendered(&format!(
                                "\npdfTeX warning: pdftex: fontmap entry for `{name}' already exists, duplicates ignored\n"
                            ));
                        }
                    }
                }
                _ => unreachable!("scanner restricts PDF font actions"),
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::FontDimen {
            font,
            number,
            value,
            recovery_context,
        } => {
            // tex.web §578's `find_font_dimen` resolves an unusable parameter
            // number to the scratch location `fmem_ptr`; §579 then reports
            // "Font x has only n fontdimen parameters" and §1253 still runs
            // `scan_optional_equals; scan_normal_dimen; font_info[k].sc:=
            // cur_val` into that scratch cell, so the font is unchanged and
            // the job continues. Only §580's grow path, which
            // `set_font_dimen` implements, can add a parameter.
            //
            // The scan already made §578's decision and captured §579's
            // context there, so this only writes or reports.
            match recovery_context {
                Some(context) => report_font_parameter_recovery(
                    stores,
                    command.diagnostic_effects,
                    *font,
                    std::mem::take(context),
                )?,
                None => {
                    let number = u32::try_from(*number)
                        .expect("a writable parameter number is a positive u32");
                    match AssignmentCommitter::new(stores, command.diagnostic_effects)
                        .try_unscoped(None, |stores| stores.set_font_dimen(*font, number, *value))
                    {
                        Ok(_) => {}
                        Err(capacity) => {
                            return Err(ExecError::Fatal(tex_command::FatalError::overflow(
                                "font memory",
                                i32::try_from(capacity).expect("font capacity fits TeX integer"),
                            )));
                        }
                    }
                }
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::FontInteger { font, skew, value } => {
            AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(None, |stores| {
                if *skew {
                    stores.set_font_skew_char(*font, *value)
                } else {
                    stores.set_font_hyphen_char(*font, *value)
                }
            });
            Ok(ReplayStep::Continue)
        }
        ColdOperation::DeferredOpenOut { stream, file_name } => {
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::OpenOut {
                    slot: StreamSlot::new(*stream),
                    path: std::mem::take(file_name),
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::DeferredCloseOut { stream } => {
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::CloseOut {
                    slot: stream.stream_slot(),
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::DeferredWrite { stream, tokens } => {
            let tokens = stores.node_token_list(tokens.prepared());
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::DeferredWrite {
                    sink: replay_write_sink(*stream),
                    tokens,
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::DeferredSpecial {
            deferred: true,
            tokens,
        } => {
            let tokens = stores.node_token_list(tokens.prepared());
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::DeferredSpecial {
                    class: "dvi".to_owned(),
                    tokens,
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::DeferredSpecial {
            deferred: false,
            tokens,
        } => {
            let mut text = String::new();
            for word in stores.token_list(tokens.prepared().clone()) {
                tex_state::token_show::append_token_string_text(
                    &**stores,
                    word.semantic_token(),
                    &mut text,
                );
            }
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::Special {
                    class: "dvi".to_owned(),
                    payload: tex_byte_text(&text),
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::SetLanguage { language } => {
            // TeX82 §1377, verbatim:
            //
            //   new_whatsit(language_node,small_node_size);
            //   scan_int;
            //   if cur_val<=0 then clang:=0
            //   else if cur_val>255 then clang:=0
            //   else clang:=cur_val;
            //   what_lang(tail):=clang;
            //   what_lhm(tail):=norm_min(left_hyphen_min);
            //   what_rhm(tail):=norm_min(right_hyphen_min);
            //
            // Both out-of-range directions recover to language zero; only
            // `1..=255` survives. The pending character run is flushed
            // first, before `clang` moves, so it hyphenates under the
            // language that was current while it was being built.
            let clang = u8::try_from(*language).unwrap_or(0);
            let left_hyphen_min =
                crate::box_runtime::norm_min(stores.int_param(IntParam::LEFT_HYPHEN_MIN));
            let right_hyphen_min =
                crate::box_runtime::norm_min(stores.int_param(IntParam::RIGHT_HYPHEN_MIN));
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                Whatsit::Language {
                    language: clang,
                    left_hyphen_min,
                    right_hyphen_min,
                },
            )?;
            modes.current_list_mutation().set_hyphen_context(
                clang,
                left_hyphen_min,
                right_hyphen_min,
            );
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IllegalSetLanguage { token } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Arithmetic {
            primitive,
            target,
            operand,
            global,
        } => {
            // TeX82 §1236 sets `arith_error` and, when it is set, reports
            // "Arithmetic overflow" and `return`s *before* `word_define`, so
            // the target keeps its old value and the job continues. Every
            // arm of `apply_arithmetic` computes its value before writing it,
            // so the target is provably unwritten on this path.
            match apply_arithmetic(
                *primitive,
                *target,
                *operand,
                *global,
                command.state.profile(),
                stores,
                command.diagnostic_effects,
            ) {
                Err(ExecError::ArithmeticOverflow) => {
                    let context = command.state.output_open_context(&**stores);
                    let mut report = stores.print_err("Arithmetic overflow");
                    report.help(&[
                        "I can't carry out that multiplication or division,",
                        "since the result is out of range.",
                    ]);
                    report.context(context);
                    report.error().defer_recovery(command.diagnostic_effects)?;
                }
                Ok(receipt) => {
                    command.retain_assignment_receipt(receipt);
                }
                Err(error) => return Err(error),
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::InvalidArithmeticTarget { primitive, target } => {
            // TeX82 §1236 prints this error and returns from
            // `do_register_command`; §1269's common `done` path still gets
            // to replay a pending `\afterassignment` token.
            let target = tex_command::print_cmd_chr_text(&**stores, target.clone());
            let primitive = stores
                .primitive_name(Meaning::UnexpandablePrimitive(*primitive))
                .expect("installed arithmetic primitive has a canonical name")
                .to_owned();
            let context = command.state.output_open_context(&**stores);
            let mut report = stores.print_err("You can't use `");
            report
                .print(&target)
                .print("' after ")
                .print_esc(&primitive);
            report.help(&["I'm forgetting what you said and not changing anything."]);
            report.context(context);
            report.error().defer_recovery(command.diagnostic_effects)?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::CharacterDefinition {
            primitive,
            target,
            provisional_old,
            value,
            global,
            ..
        } => {
            let meaning = match primitive {
                UnexpandablePrimitive::CharDef => Meaning::CharGiven(
                    char::from_u32(*value as u32)
                        .expect("§434 recovers a character code to a character"),
                ),
                UnexpandablePrimitive::MathCharDef => Meaning::MathCharGiven(*value as u16),
                _ => unreachable!("character-definition step carries only §1224 primitives"),
            };
            assignment_tracing::trace_completed_provisional_meaning_write(
                stores,
                command.diagnostic_effects,
                Token::Cs(*target),
                provisional_old.clone(),
                Meaning::Relax,
                *global,
            );
            let observed = match primitive {
                UnexpandablePrimitive::CharDef => ObservationValue::Character(*value as u32),
                UnexpandablePrimitive::MathCharDef => ObservationValue::Integer(i64::from(*value)),
                _ => unreachable!("character-definition step carries only §1224 primitives"),
            };
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).meaning(
                *target,
                Token::Cs(*target),
                meaning,
                observed,
                *global,
                |stores| {
                    stores
                        .assign_resolved_meaning(
                            *target,
                            tex_state::meaning::ResolvedMeaning::Static(meaning),
                            assignment_scope(*global),
                        )
                        .expect("character-definition target belongs to admitted state")
                },
            );
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::RegisterDefinition {
            primitive,
            target,
            provisional_old,
            index,
            global,
        } => {
            let meaning = match primitive {
                UnexpandablePrimitive::CountDef => Meaning::CountRegister(*index),
                UnexpandablePrimitive::DimenDef => Meaning::DimenRegister(*index),
                UnexpandablePrimitive::SkipDef => Meaning::SkipRegister(*index),
                UnexpandablePrimitive::MuskipDef => Meaning::MuskipRegister(*index),
                UnexpandablePrimitive::ToksDef => Meaning::ToksRegister(*index),
                _ => unreachable!("register-definition step carries only §1224 primitives"),
            };
            assignment_tracing::trace_completed_provisional_meaning_write(
                stores,
                command.diagnostic_effects,
                Token::Cs(*target),
                provisional_old.clone(),
                Meaning::Relax,
                *global,
            );
            let observed_name =
                if command.state.profile().capabilities().supports_etex() && *index > 255 {
                    if *primitive == UnexpandablePrimitive::ToksDef {
                        "toks_register"
                    } else {
                        "register"
                    }
                } else {
                    match primitive {
                        UnexpandablePrimitive::CountDef => "assign_int",
                        UnexpandablePrimitive::DimenDef => "assign_dimen",
                        UnexpandablePrimitive::SkipDef => "assign_glue",
                        UnexpandablePrimitive::MuskipDef => "assign_mu_glue",
                        UnexpandablePrimitive::ToksDef => "assign_toks",
                        _ => unreachable!("register-definition step carries only §1224 primitives"),
                    }
                };
            let receipt = AssignmentCommitter::new(stores, command.diagnostic_effects).meaning(
                *target,
                Token::Cs(*target),
                meaning,
                ObservationValue::Name(observed_name.into()),
                *global,
                |stores| {
                    stores
                        .assign_resolved_meaning(
                            *target,
                            tex_state::meaning::ResolvedMeaning::Static(meaning),
                            assignment_scope(*global),
                        )
                        .expect("register-definition target belongs to admitted state")
                },
            );
            command.retain_assignment_receipt(receipt);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::HyphenationData {
            words,
            pattern_specs,
            patterns,
            rejection_context,
            trie_built,
        } => {
            // TeX82 §1252 rejects `\patterns` for two different reasons, and
            // the two do not share a message. The `init`/`tini` split comes
            // first: a production binary has no `new_patterns` to call, so it
            // reports "Patterns can be loaded only by INITEX" with `help0`
            // and flushes the braced group. Only INITEX reaches §960, whose
            // own `trie_not_ready=false` guard is the "Too late" one, and it
            // does carry help. `\hyphenation` is legal in both binaries.
            if *patterns && !command.initex {
                let mut report = stores.print_err("Patterns can be loaded only by INITEX");
                report.context(std::mem::take(rejection_context));
                report.error().defer_recovery(command.diagnostic_effects)?;
                return Ok(ReplayStep::Continue);
            }
            if *trie_built {
                let mut report = stores.print_err("Too late for \\patterns");
                report.help(&["All patterns must be given before typesetting begins."]);
                report.context(std::mem::take(rejection_context));
                report.error().defer_recovery(command.diagnostic_effects)?;
                return Ok(ReplayStep::Continue);
            }
            // Both halves of §§935/963's diagnostics were already reported by
            // the live scan, where §82 could still show the character that
            // caused them; installing is all that is left here.
            if *patterns {
                AssignmentCommitter::new(stores, command.diagnostic_effects).try_unscoped(
                    None,
                    |stores| {
                        crate::paragraph_end::apply_scanned_patterns(
                            stores,
                            std::mem::take(pattern_specs),
                        )
                    },
                )?;
            } else {
                AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(
                    None,
                    |stores| {
                        crate::paragraph_end::apply_scanned_hyphenation_exceptions(
                            stores,
                            std::mem::take(words),
                        );
                    },
                );
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::AfterGroup(token) => {
            match command.state.state_mut().save_aftergroup(&**stores, *token) {
                // TeX82 §280's `save_for_after` consumes the operand at every
                // level, but creates an `insert_token` save word only when
                // `cur_level > level_one`. `NoOpenGroup` is therefore the
                // canonical outer-level no-op; every synchronization failure
                // remains an invariant violation.
                Ok(()) | Err(tex_command::CommandGroupError::NoOpenGroup) => {}
                Err(error) => panic!(
                    "aftergroup is admitted only for the synchronized group stack: {error:?}"
                ),
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::AfterAssignment(token) => {
            command
                .state
                .state_mut()
                .set_afterassignment(&**stores, *token)
                .expect("afterassignment uses the synchronized command generation");
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Rule {
            width,
            height,
            depth,
            horizontal,
        } => apply_scanned_rule(command, modes, stores, *width, *height, *depth, *horizontal),
        ColdOperation::HRuleHereExceptLeaders => {
            let context = command.state.output_open_context(&**stores);
            report_escaped_error(
                stores,
                command.diagnostic_effects,
                "You can't use `",
                "hrule",
                "' here except with leaders",
                &[
                    "To put a horizontal rule in an hbox or an alignment,",
                    "you should use \\leaders or \\hrulefill (see The TeXbook).",
                ],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Message { tokens, error } => {
            // TeX82 §1279's `issue_message` renders the scanned list through
            // `token_show` into one string and then hands it to §1280 or
            // §1283; neither branch formats or routes its own output.
            let text = message_tokens_text(stores, tokens.prepared().clone());
            if *error {
                let context = command.state.output_open_context(&**stores);
                issue_error_message(stores, command.diagnostic_effects, &text, context)?;
            } else {
                issue_terminal_message(stores, &text);
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::DisplayDiagnostic(diagnostic) => {
            // TeX82 §§62/1294/1297 begin the display with `print_nl(">␣")`,
            // which closes a partial selected line but does not add a blank
            // line when both selected sinks are already at column zero.
            // The scanned value carries exactly that line's content; replay
            // owns the selector-sensitive transition and decodes no textual
            // envelope.
            let context = command.state.output_open_context(&**stores);
            print_display_content(stores, command.diagnostic_effects, &diagnostic.content);
            command.defer_show_completion(false, context);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ShowBox { index } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::execute_showbox(
                stores,
                command.diagnostic_effects,
                *index,
                command.state.profile(),
            );
            command.defer_show_completion(true, context);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ShowLists => {
            // TeX82 §218 observes the synchronous linked list built by
            // main_control. Materialize Umber's batched character tail before
            // traversing its diagnostic physical projection.
            crate::box_runtime::flush_pending_hchars(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::execute_showlists(
                stores,
                command.diagnostic_effects,
                modes,
                command.state.profile(),
            )?;
            command.defer_show_completion(true, context);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ShowTokens { tokens } => {
            // e-TeX's odd xray modifier reaches `the_toks`, then TeX82
            // §1297 prints `token_show(temp_head)` and takes the common
            // `\show` completion path.
            let context = command.state.output_open_context(&**stores);
            let text = show_tokens_tokens_text(stores, tokens.prepared().clone());
            // §1297 opens with §62's `print_nl(">␣")`, whose break is
            // conditional on a selected sink already having an open column.
            // An unconditional newline here left a blank line above the
            // display whenever the file's own `(` had just closed one.
            let mut diagnostic = stores.begin_online_diagnostic(command.diagnostic_effects);
            diagnostic.print_nl("> ").print_rendered(&text);
            diagnostic.end_open();
            command.defer_show_completion(false, context);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ShowIfs { conditions } => {
            // etex.ch [17.3720]'s `show_ifs` is a `begin_diagnostic` form
            // like `\showbox`/`\showlists`/`\showgroups`, not a direct
            // print: see `tex-exec::diagnostics`'s module doc for why the
            // dump must be routed through §245's redirection rather than
            // written straight to both channels.
            let context = command.state.output_open_context(&**stores);
            let mut diagnostic = stores.begin_diagnostic(command.diagnostic_effects);
            diagnostic.print_nl("").print_ln();
            diagnostic.print_rendered(&render_showifs(conditions));
            diagnostic.end(true);
            command.defer_show_completion(true, context);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ShowGroups {
            diagnostic: Some(diagnostic),
        } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::execute_showgroups(stores, command.diagnostic_effects, diagnostic);
            command.defer_show_completion(true, context);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ShowGroups { diagnostic: None } => {
            let diagnostic = detached_showgroups(
                stores,
                active_alignment,
                boxes,
                active_discretionaries,
                active_math_choices,
                active_math_left_boundaries,
                active_math_shifts,
            );
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::execute_showgroups(stores, command.diagnostic_effects, &diagnostic);
            command.defer_show_completion(true, context);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ImmediateExtension(extension) => {
            match extension {
                RootedImmediateExtension::Continue => {}
                RootedImmediateExtension::PdfExtensionInDviMode(primitive) => {
                    let name = match primitive {
                        UnexpandablePrimitive::PdfObject => "pdfobj",
                        UnexpandablePrimitive::PdfXForm => "pdfxform",
                        UnexpandablePrimitive::PdfXImage => "pdfximage",
                        _ => unreachable!("only immediate PDF extensions reach this result"),
                    };
                    return Err(ExecError::PdfExtensionInDviMode(name));
                }
                RootedImmediateExtension::OpenOut { stream, file_name } => {
                    let target = replay_openout_target(file_name.packed());
                    stores.open_output_stream(StreamSlot::new(*stream), target.clone().into());
                    command
                        .capabilities
                        .invalidate_input_unavailability_for_output(&target);
                    if command.state.engine_semantics().supports_pdftex() {
                        let tracing_online = stores.int_param(IntParam::TRACING_ONLINE);
                        let (terminal_line_is_open, log_line_is_open) = {
                            let printer = stores.printer();
                            (printer.terminal_offset() > 0, printer.log_offset() > 0)
                        };
                        let (sink, text) = crate::diagnostics::report_openout(
                            tracing_online,
                            terminal_line_is_open,
                            log_line_is_open,
                            *stream,
                            &target,
                        );
                        super::support::write_preframed_immediate_text(stores, command, sink, text);
                    }
                }
                RootedImmediateExtension::Write { stream, tokens } => {
                    let text = write_text(tokens.prepared(), stores);
                    if let Some(sink) = immediate_write_sink(*stream, stores) {
                        write_immediate_text(stores, command, sink, &text);
                    }
                }
                RootedImmediateExtension::CloseOut { stream } => {
                    if let Some(stream) = stream.stream_slot() {
                        stores.close_output_stream(stream);
                    }
                }
                RootedImmediateExtension::PdfObject(request) => {
                    if matches!(request, RootedPdfObjectRequest::Reserve) {
                        return Err(ExecError::PdfImmediateReservedObject);
                    }
                    apply_pdf_object_request(request, stores, true)?;
                }
                RootedImmediateExtension::PdfForm(request) => {
                    let pending = apply_pdf_form_request(request, stores, modes, command, true)?;
                    assert!(
                        pending.is_none(),
                        "immediate form publication is intercepted before generic cold apply"
                    );
                }
                RootedImmediateExtension::PdfImage(_) => {
                    unreachable!("immediate image requests are normalized before resolution")
                }
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::SetBox { target, path } => {
            // §1214's `<Adjust for the setting of \globaldefs>` runs inside
            // `prefixed_command`, before §1241 scans the box, so `global` in
            // §1241's `if global then n:=256+cur_val` is the *effective*
            // scope. Resolving it at `box_end` instead would read
            // `\globaldefs` as the box body left it.
            // TeX82 §§1074 and 1077 remove `\lastbox` from the current
            // list before storing that box in the destination register. Keep
            // the same ownership boundary: the source-list rewrite must
            // settle before the destination construction suffix opens, or
            // sealing that suffix would transfer the live source descriptor
            // into durable box ownership along with the selected box.
            if let ScannedSetBoxPath::Payload(ScannedBoxShiftPayload::LastBox { error_context }) =
                path
            {
                let node = crate::box_runtime::take_last_box(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                    |_| Ok(std::mem::take(error_context)),
                )?;
                boxes.pending_setbox = Some(PendingSetBox {
                    target: *target,
                    region: stores.begin_page_node_region(),
                });
                let context = boxes.take_box_context(false);
                box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
                return Ok(ReplayStep::Continue);
            }

            boxes.pending_setbox = Some(PendingSetBox {
                target: *target,
                region: stores.begin_page_node_region(),
            });
            match path {
                ScannedSetBoxPath::Forbidden { error_context } => {
                    if let BoxContext::SetBox(pending) = boxes.take_box_context(false) {
                        stores
                            .release_page_node_region(pending.region)
                            .expect("rejected setbox releases its empty page region");
                    }
                    report_improper_setbox(
                        std::mem::take(error_context),
                        stores,
                        command.diagnostic_effects,
                    )?;
                }
                ScannedSetBoxPath::Payload(payload) => match payload {
                    ScannedBoxShiftPayload::Missing => {
                        if let BoxContext::SetBox(pending) = boxes.take_box_context(false) {
                            stores
                                .release_page_node_region(pending.region)
                                .expect("missing setbox payload releases its page region");
                        }
                        let context = command.state.output_open_context(&**stores);
                        report_improper_setbox(context, stores, command.diagnostic_effects)?;
                    }
                    ScannedBoxShiftPayload::BoxRegister { index, copy } => {
                        let id = read_box_register(*index, *copy, stores, command);
                        let node = crate::box_runtime::first_box_node(stores, id);
                        let context = boxes.take_box_context(false);
                        box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
                    }
                    ScannedBoxShiftPayload::LastBox { .. } => {
                        unreachable!("setbox lastbox is ordered before its destination suffix")
                    }
                    ScannedBoxShiftPayload::VSplit(split) => {
                        if let Some(context) = &split.missing_to_context {
                            report_missing_vsplit_to(context, command.diagnostic_effects, stores)?;
                        }
                        let diagnostic_context = command_diagnostic_context(command, stores);
                        let mut geometry = pack_geometry_sink(command.state, command.observations);
                        let node = crate::box_runtime::split_vbox_register(
                            stores,
                            command.diagnostic_effects,
                            &mut geometry,
                            &diagnostic_context,
                            split.index,
                            split.height,
                            &split.split_context,
                        )?;
                        let context = boxes.take_box_context(false);
                        box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
                    }
                    ScannedBoxShiftPayload::Construction(construction) => begin_replay_box(
                        *construction,
                        boxes.pending_setbox.take(),
                        None,
                        modes,
                        stores,
                        boxes,
                        command,
                    )?,
                },
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::VSplit(split) => {
            if let Some(context) = &split.missing_to_context {
                report_missing_vsplit_to(context, command.diagnostic_effects, stores)?;
            }
            let diagnostic_context = command_diagnostic_context(command, stores);
            let mut geometry = pack_geometry_sink(command.state, command.observations);
            let node = crate::box_runtime::split_vbox_register(
                stores,
                command.diagnostic_effects,
                &mut geometry,
                &diagnostic_context,
                split.index,
                split.height,
                &split.split_context,
            )?;
            let context = boxes.take_box_context(false);
            box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BoxRegister {
            index,
            copy,
            ships_out,
        } => {
            let id = read_box_register(*index, *copy, stores, command);
            let node = crate::box_runtime::first_box_node(stores, id);
            let context = boxes.take_box_context(*ships_out);
            box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Unbox {
            primitive,
            index,
            error_context,
        } => {
            let state = &*command.state;
            crate::box_runtime::execute_scanned_unbox_with_error_context(
                *primitive,
                *index,
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                |stores| {
                    state
                        .render_diagnostic_context(*error_context, stores)
                        .map_err(|_| ExecError::Command(tex_command::CommandError::StaleDelivery))
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::SavedVerticalDiscards(primitive) => {
            crate::box_runtime::execute_scanned_saved_vertical_discards(
                *primitive,
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::LastBox { error_context } => {
            let state = &*command.state;
            let node = crate::box_runtime::take_last_box(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
                |stores| {
                    state
                        .render_diagnostic_context(*error_context, stores)
                        .map_err(|_| ExecError::Command(tex_command::CommandError::StaleDelivery))
                },
            )?;
            let context = boxes.take_box_context(false);
            box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Leaders {
            kind,
            payload,
            glue,
        } => {
            boxes.pending_leader = None;
            crate::box_runtime::append_leader_contribution(
                modes,
                stores,
                command.diagnostic_effects,
                *kind,
                *payload,
                *glue,
                command.fuel,
            )?;
            crate::vertical::build_page_if_outer_vertical(
                modes,
                stores,
                command.diagnostic_effects,
                command.state.state(),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::LeaderRegister {
            kind,
            index,
            copy,
            glue,
        } => {
            if let Some(payload) = crate::box_runtime::take_register_payload(stores, *index, *copy)
            {
                crate::box_runtime::append_leader_contribution(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    *kind,
                    payload,
                    *glue,
                    command.fuel,
                )?;
                crate::vertical::build_page_if_outer_vertical(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.state.state(),
                )?;
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::MissingLeaderPayload => {
            // A leader payload is scanned by §1084's `scan_box` like any
            // other box context, so a non-box command there gets §1084's own
            // report, not a leader-specific one.
            report_missing_box(command.state, command.diagnostic_effects, stores)?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::LeadersNotFollowedByGlue => {
            boxes.pending_leader = None;
            // TeX82 §1078's `back_error`; `scan_leader_glue_command` has
            // already put the command that was not glue back.
            let context = command.state.output_open_context(&**stores);
            crate::error_report::report_error(
                stores,
                command.diagnostic_effects,
                "Leaders not followed by proper glue",
                &[
                    "You should say `\\leaders <box or rule><hskip or vskip>'.",
                    "I found the <box or rule>, but there's no suitable",
                    "<hskip or vskip>, so I'm ignoring these leaders.",
                ],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BeginShipout => {
            boxes.pending_shipout = Some(stores.begin_page_node_region());
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BeginBox(construction) => {
            let target = boxes.pending_setbox.take();
            let shipout_region = boxes.pending_shipout.take();
            begin_replay_box(
                *construction,
                target,
                shipout_region,
                modes,
                stores,
                boxes,
                command,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BeginInsert(construction) => {
            // TeX82 §1099's `begin_insert_or_adjust`: `scan_eight_bit_int`
            // has already applied its range clamp and queued the canonical
            // "Bad register code" report. The additional `\insert255`
            // rejection ("box 255 is special") runs here. `\vadjust` set
            // `class:=255` directly
            // (`is_vadjust`), without ever calling `scan_eight_bit_int`, so
            // neither diagnostic applies to it -- 255 is its correct,
            // already-valid sentinel class, not a user-typed `\insert255`.
            let mut class = construction.class;
            if !construction.is_vadjust && class == 255 {
                let mut report = stores.print_err("You can't ");
                report
                    .print_esc("insert")
                    .print_int(255)
                    .help(&["I'm changing to \\insert0; box 255 is special."]);
                if let Some(context) = construction.reserved_class_context.take() {
                    report.context(context);
                }
                report.error().defer_recovery(command.diagnostic_effects)?;
                class = 0;
            }
            let class = class as u16;
            enter_group(
                stores,
                command.state,
                command.diagnostic_effects,
                GroupKind::Insert,
            );
            modes.push_at_line(
                Mode::InternalVertical,
                i32::try_from(command.state.current_file_line_number()).unwrap_or(i32::MAX),
            )?;
            // §1099: `normal_paragraph` resets \parshape/\looseness/\hangindent/
            // \hangafter local to the just-opened insert group, exactly like
            // `begin_box` does for `\vbox`/`\vtop` (§1051-2).
            crate::paragraph_end::normal_paragraph(modes, stores, command.diagnostic_effects);
            boxes.active_boxes.push(ActiveReplayBox {
                target: None,
                shipout_region: None,
                kind: ReplayBoxKind::Insert(class, construction.pre),
                group_kind: GroupKind::Insert,
                packing: PackSpec::Natural,
                leader_kind: None,
                shift: None,
            });
            // Unlike `\hbox`/`\vbox`/`\vtop`, §1099 never begins the
            // `\everyhbox`/`\everyvbox` token list for an insertion body.
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IllegalInsertOrAdjust { token } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IllegalEqNo { token } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IllegalHAlign { token } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IllegalLastItem { token, context } => {
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(std::mem::take(context)),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::MisplacedAlignmentDelimiter { token, context } => {
            let context = render_diagnostic_coordinate(command, stores, *context)?;
            crate::diagnostics::report_misplaced_alignment_delimiter(
                stores,
                command.diagnostic_effects,
                *token,
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::MisplacedAlignmentCommand { omit } => {
            let context = command.state.output_open_context(&**stores);
            let (name, help) = if *omit {
                (
                    "omit",
                    [
                        "I expect to see \\omit only after tab marks or the \\cr of",
                        "an alignment. Proceed, and I'll ignore this case.",
                    ],
                )
            } else {
                (
                    "noalign",
                    [
                        "I expect to see \\noalign only after the \\cr of",
                        "an alignment. Proceed, and I'll ignore this case.",
                    ],
                )
            };
            crate::diagnostics::report_misplaced_alignment_command(
                stores,
                command.diagnostic_effects,
                name,
                &help,
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Mark { class, tokens } => {
            // No `build_page` call afterward (unlike `\penalty`/`\insert`):
            // TeX82 §1101 and e-TeX 2.6 [26.424]'s `make_mark` append the
            // node in every mode and leave page building to a later trigger.
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            let tokens = stores.node_token_list(tokens.prepared());
            crate::vertical::append_vertical_contribution(
                modes,
                stores,
                Node::Mark {
                    class: *class,
                    tokens,
                },
            );
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BeginLeaderBox {
            construction,
            kind: leader_kind,
        } => {
            let kind = ReplayBoxKind::from_scanned(construction.kind);
            let packing = match construction.packing {
                ScannedPackingSpec::Natural => PackSpec::Natural,
                ScannedPackingSpec::Exactly(size) => PackSpec::Exactly(size),
                ScannedPackingSpec::Spread(size) => PackSpec::Spread(size),
            };
            enter_group(
                stores,
                command.state,
                command.diagnostic_effects,
                kind.group_kind(),
            );
            modes.push_at_line(
                if kind.horizontal() {
                    Mode::RestrictedHorizontal
                } else {
                    Mode::InternalVertical
                },
                i32::try_from(command.state.current_file_line_number()).unwrap_or(i32::MAX),
            )?;
            if !kind.horizontal() {
                commit_box_normal_paragraph(modes, stores, command);
            }
            boxes.active_boxes.push(ActiveReplayBox {
                target: None,
                shipout_region: None,
                kind,
                group_kind: kind.group_kind(),
                packing,
                leader_kind: Some(*leader_kind),
                shift: None,
            });
            schedule_everybox(command.state, stores, kind.horizontal());
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BoxShift(shift) => apply_box_shift(shift, command, modes, stores, boxes),
        ColdOperation::IllegalBoxShift { token } => {
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BeginSimpleGroup => {
            enter_group(
                stores,
                command.state,
                command.diagnostic_effects,
                GroupKind::Simple,
            );
            boxes.recovery_simple_group_pending = false;
            boxes.recovery_simple_group_open = true;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::EndSimpleGroup => {
            let aftergroup = leave_group_payloads(
                stores,
                command.state,
                command.diagnostic_effects,
                GroupKind::Simple,
            )
            .map_err(|_| ExecError::MissingToken {
                context: "simple recovery group",
            })?;
            schedule_aftergroup(command, stores, aftergroup)?;
            boxes.recovery_simple_group_open = false;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::OutputRoutineOpeningBrace => {
            // TeX82 §1026 runs `normal_paragraph` after the output token list
            // and output save level have both been opened, as part of
            // `scan_left_brace`. Keep the reset at this consumed-opening-brace
            // boundary: doing it in the deferred page-fire tail runs before
            // command control has established the routine's body boundary.
            crate::paragraph_end::normal_paragraph(modes, stores, command.diagnostic_effects);
            boxes.output_routine_opening_pending = false;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::EndOutputRoutine => {
            let output_context = command.state.output_open_context(&**stores);
            let unbalanced = {
                let mut processor = command_processor(
                    command.state,
                    command.fuel,
                    command.capabilities,
                    command.observations,
                    command.diagnostic_effects,
                    stores,
                );
                let result = processor
                    .finish_selected_output_routine()
                    .map_err(command_error);
                processor.retire();
                result?
            };
            if unbalanced {
                crate::error_report::report_error(
                    stores,
                    command.diagnostic_effects,
                    "Unbalanced output routine",
                    &[
                        "Your sneaky output routine has problematic {'s and/or }'s.",
                        "I can't handle that very well; good luck.",
                    ],
                    output_context,
                )?;
            }
            // TeX82 §1026 has now semantically ended the output token list.
            // Section 1028's subsequent error therefore sees the source
            // level below every retained depleted `<output>` replay.
            let context = command.state.output_close_context(&**stores);
            // TeX82 §1026 retires the output token list, then runs §1096's
            // `end_graf` before it unsaves the output group. A non-null
            // paragraph left open by \output must be line-broken into this
            // internal vertical list; merely popping it discards the paragraph.
            // `end_paragraph` is the shared spelling of §1096: it ignores
            // non-horizontal modes and pops a null paragraph without a line.
            let diagnostic_context = command_diagnostic_context(command, stores);
            let mut geometry = pack_geometry_sink(command.state, command.observations);
            crate::paragraph_end::end_paragraph_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                &mut geometry,
                diagnostic_context,
                command.fuel,
            )?;
            let mut output_level = crate::box_runtime::commit_current_list(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            let aftergroup = leave_group_payloads(
                stores,
                command.state,
                command.diagnostic_effects,
                GroupKind::Output,
            )
            .map_err(|_| ExecError::MissingToken {
                context: "output routine group",
            })?;
            // TeX82 §1026 closes `output_group` with §282's `unsave`
            // before resuming `build_page`. `unsave` backs every saved
            // `insert_token` into input, including `\aftergroup` material.
            schedule_aftergroup(command, stores, aftergroup)?;
            boxes.output_routine_active = false;
            crate::page_output::resume_page_builder_after_output(
                stores,
                command.diagnostic_effects,
                output_level.list_mutation().take_nodes(),
                crate::diagnostics::ExecutionDiagnosticContext::source_free(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::EjectResidualPage => {
            // TeX82 §1054's `its_all_over` false branch. The stop is already
            // backed up; appending the end-job trio and running §994's
            // `build_page` is all the ejection this step performs. §1005's
            // `@<Check if node p is a new champion breakpoint...@>` decides
            // whether the `-'10000000000` penalty fires §1012's `fire_up`,
            // and §1025 alone ever starts `\output`.
            // Each canonical retry appends a fresh trio. The page builder owns
            // the retry fence because only its journaled transition position
            // can distinguish a legitimate retry after default output, an
            // explicit routine, insertion heldovers, or a dead cycle from an
            // identical-state replay. The old executor boolean stayed set
            // after a prior trio had been consumed and could therefore leave
            // a terminal kern alone forever.
            let progress = stores
                .begin_end_job_ejection()
                .map_err(|_| ExecError::Fatal(FatalError::confusion("end job page progress")))?;
            crate::page_output::append_end_job_contributions(stores);
            crate::page_builder::build_page(
                stores,
                command.diagnostic_effects,
                command.state.state(),
            )?;
            if !stores.complete_end_job_ejection(progress) {
                return Err(ExecError::Fatal(FatalError::confusion(
                    "end job page progress",
                )));
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::IllegalStop { token } => {
            // TeX82 §1051's `privileged`: `\end`/`\dump` below outer
            // vertical mode reports and is discarded, exactly like the other
            // Forbidden cases.
            let context = command.state.output_open_context(&**stores);
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                command.diagnostic_effects,
                *token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ExtraRightBrace { forgotten: None } => {
            // TeX82 §1068's `bottom_level` arm of `handle_right_brace`.
            let context = command.state.output_open_context(&**stores);
            crate::error_report::report_error(
                stores,
                command.diagnostic_effects,
                "Too many }'s",
                &[
                    "You've closed more groups than you opened.",
                    "Such booboos are generally harmless, so keep going.",
                ],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::ExtraRightBrace {
            forgotten: Some(forgotten),
        } => {
            // TeX82 §1069's `extra_right_brace` reports and discards the
            // mismatched brace. It does not `unsave` the group it names.
            let context = command.state.output_open_context(&**stores);
            let mut report = stores.print_err("Extra }, or forgotten ");
            forgotten.print(&mut report);
            report.help(&[
                "I've deleted a group-closing symbol because it seems to be",
                "spurious, as in `$x}$'. But perhaps the } is legitimate and",
                "you forgot something else, as in `\\hbox{$x}'. In such cases",
                "the way to recover is to insert both the forgotten and the",
                "deleted material, e.g., by typing `I$}'.",
            ]);
            report.context(context);
            report.error().defer_recovery(command.diagnostic_effects)?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::OffSave(closer) => {
            // `scan_off_save` already ran the input recovery (backing up the
            // command behind its chosen closer); this only prints TeX82
            // §1064's report naming what §1065 inserted.
            let context = command.state.output_open_context(&**stores);
            let mut report = stores.print_err("Missing ");
            closer.print(&mut report);
            report
                .print(" inserted")
                .help(&OFF_SAVE_HELP)
                .context(context);
            report.error().defer_recovery(command.diagnostic_effects)?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::OffSaveBottomDrop { token } => {
            // TeX82 §1066: "print_err("Extra "); print_cmd_chr(cur_cmd,
            // cur_chr)". `scan_off_save` already dropped the command itself
            // (no backup, nothing to replay); this only names it.
            let name = tex_command::command_token_text(&mut **stores, *token);
            let context = command.state.output_open_context(&**stores);
            crate::error_report::report_error(
                stores,
                command.diagnostic_effects,
                &format!("Extra {name}"),
                &["Things are pretty mixed up, but I think the worst is over."],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::EndMathGroup(kind) => {
            // TeX82 §1186 and §1174's `build_choices` both open with
            // `unsave`. Everything after it -- popping `saved`, `fin_mlist`,
            // and storing the result in the field or branch -- belongs to the
            // scanner that opened the group, so `execute_live_math_group`
            // performs it once its level is gone.
            let aftergroup =
                leave_group_payloads(stores, command.state, command.diagnostic_effects, *kind)
                    .map_err(|_| ExecError::MissingToken {
                        context: "math group",
                    })?;
            schedule_aftergroup(command, stores, aftergroup)?;
            if *kind == GroupKind::Math {
                // TeX82 §1186 runs `fin_mlist` in the right-brace command
                // itself, then writes the result through §1153's saved field
                // pointer. This is the ordinary main-control closer for the
                // live group, not a return into a nested executor loop.
                let target = *active_math_fields.last().ok_or(ExecError::MissingToken {
                    context: "active math field",
                })?;
                let mut level = crate::box_runtime::commit_current_list(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                )?;
                let (nodes, incomplete) = {
                    let mut list = level.list_mutation();
                    (list.take_nodes(), list.take_incomplete_fraction())
                };
                let list = finish_math_list(nodes, incomplete, stores)?;
                let field = collapse_singleton_math_group(stores, list);
                fill_math_field_target(modes, stores, target, field);
                active_math_fields.pop();
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::AlignmentRecovery { brace } => {
            let (message, opens_simple_group) = match brace {
                Catcode::BeginGroup => ("Missing { inserted", true),
                Catcode::EndGroup => ("Missing } inserted", false),
                _ => {
                    return Err(ExecError::MissingToken {
                        context: "align_error balancing brace",
                    });
                }
            };
            let context = command.state.output_open_context(&**stores);
            crate::error_report::report_error(
                stores,
                command.diagnostic_effects,
                message,
                &[
                    "I've put in what seems to be necessary to fix",
                    "the current column of the current alignment.",
                    "Try to go on, since this might almost work.",
                ],
                context,
            )?;
            boxes.recovery_simple_group_pending = opens_simple_group;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BoxEndGroup {
            ships_out,
            current_line,
        } => {
            let box_state = boxes.active_boxes.pop().ok_or(ExecError::MissingToken {
                context: "box group",
            })?;
            if let ReplayBoxKind::Insert(class, pre) = box_state.kind {
                return finish_insert_or_adjust_group(class, pre, modes, stores, command);
            }
            // TeX82 §1085's `handle_right_brace` runs `end_graf` (§1096) for
            // `vbox_group` and `vtop_group` -- and only for those two -- before
            // `package`: `hbox_group` and `adjusted_hbox_group` package
            // immediately. A vertical box whose body still has a paragraph open
            // when its closing brace arrives must therefore line-break that
            // paragraph into the box's own vertical list first. Without this,
            // `modes.pop()` below took the still-open *horizontal* level for the
            // box body and packaged its hlist material directly, so
            // `\vbox{\noindent A}` produced `\vbox(0.0+0.0)x0.0` holding a bare
            // char node -- and left the box's real internal-vertical level open
            // on the mode nest (`umber2-johp.232`).
            if !box_state.kind.horizontal() {
                let mut diagnostic_context = command_diagnostic_context(command, stores);
                // The closing brace can exhaust and retire its source before
                // this cold operation is applied. TeX82 §661 nevertheless
                // uses the brace's live `line` as the ending line of every
                // paragraph pack report, so retain that delivered scalar in
                // `BoxEndGroup` instead of re-reading a now-empty input stack.
                diagnostic_context.current_line = *current_line;
                let mut geometry = pack_geometry_sink(command.state, command.observations);
                crate::paragraph_end::end_paragraph_with_fuel(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    &mut geometry,
                    diagnostic_context,
                    command.fuel,
                )?;
            }
            // TeX82's main-control loop appends every character (and its
            // resolved ligature/kern chain) to the current list synchronously
            // as it is scanned, so by the time `handle_right_brace` reaches
            // `package` (§1086) to `hpack`/`vpack` a finished box, the list
            // is already complete. Umber batches a run of pending horizontal
            // characters (for ligature/kerning/shaping) in
            // `ModeList::pending_hchars` rather than materializing nodes
            // immediately, so any box-body list a `}` is about to freeze must
            // first flush that batch -- exactly like every other site that
            // treats a list as finished (`execute_discretionary_part`,
            // `capture_replay_alignment_cell`, `finish_replay_alignment_row`).
            // Without this, a box whose body ends in a bare character run
            // with no trailing glue/kern/space to force an earlier flush
            // (e.g. `\hbox{c}`, or plain.tex's `\setbox\z@\hbox{#1}` inside
            // `\c`) silently packages an empty list: the pending characters
            // are dropped along with the popped mode level instead of ever
            // becoming node.

            let mut level = crate::box_runtime::commit_current_list(
                modes,
                stores,
                command.diagnostic_effects,
                command.fuel,
            )?;
            let children = level.list_mutation().take_nodes();
            // TeX82 §1086 snapshots `d:=box_max_depth` before `unsave`.
            // The box body may assign a local, signed `\boxmaxdepth`; that
            // value governs this package operation even though the assignment
            // is restored before `vpackage` runs.
            let box_max_depth = stores.dimen_param(DimenParam::BOX_MAX_DEPTH);
            // e-TeX 2.6 [23.328]'s `group_warning` runs immediately before
            // every `unsave`, including §1086's hbox/vbox packaging path.
            // Keeping the hook here preserves save-stack order when one
            // nested source closes both a box group and a conditional.
            warn_cross_file_group_close(stores, command);
            let aftergroup = leave_group_payloads(
                stores,
                command.state,
                command.diagnostic_effects,
                box_state.group_kind,
            )
            .map_err(|_| ExecError::MissingToken {
                context: "box group",
            })?;
            schedule_aftergroup(command, stores, aftergroup)?;
            // TeX82 §1086 restores the box group before it calls `hpack` or
            // `vpack`. Besides putting §283's tracing-restores lines ahead of
            // §660/§674 diagnostics, this makes the enclosing h/v badness,
            // fuzz, and overfull-rule parameters authoritative. Max depth is
            // the exception: `package` saved it above before `unsave`.
            let node = if box_state.kind.horizontal() {
                let diagnostic_context = command_diagnostic_context(command, stores);
                let mut geometry = pack_geometry_sink(command.state, command.observations);
                Node::HList(crate::box_runtime::hpack_with_overfull_rule(
                    stores,
                    command.diagnostic_effects,
                    &mut geometry,
                    &diagnostic_context,
                    children,
                    box_state.packing,
                ))
            } else {
                Node::VList(match box_state.kind {
                    ReplayBoxKind::VBox | ReplayBoxKind::VCenter => {
                        let diagnostic_context = command_diagnostic_context(command, stores);
                        let mut params = crate::packing_params::vpack_params(stores);
                        params.box_max_depth = box_max_depth;
                        let mut geometry = pack_geometry_sink(command.state, command.observations);
                        crate::packing_params::vpack(
                            stores,
                            command.diagnostic_effects,
                            &mut geometry,
                            &diagnostic_context,
                            children,
                            box_state.packing,
                            params,
                        )
                        .node
                    }
                    ReplayBoxKind::VTop => {
                        let diagnostic_context = command_diagnostic_context(command, stores);
                        let mut params = crate::packing_params::vpack_params(stores);
                        params.box_max_depth = box_max_depth;
                        let mut geometry = pack_geometry_sink(command.state, command.observations);
                        crate::packing_params::vtop(
                            stores,
                            command.diagnostic_effects,
                            &mut geometry,
                            &diagnostic_context,
                            children,
                            box_state.packing,
                            params,
                        )
                        .node
                    }
                    ReplayBoxKind::HBox => unreachable!("horizontal box was handled above"),
                    ReplayBoxKind::Insert(_, _) => unreachable!(
                        "insert/adjust bodies return through finish_insert_or_adjust_group above"
                    ),
                })
            };
            // TeX82 §1168's `vcenter_group` case of `handle_right_brace`:
            //
            //     vcenter_group: begin end_graf; unsave; save_ptr:=save_ptr-2;
            //       p:=vpack(link(head),saved(1),saved(0)); pop_nest;
            //       tail_append(new_noad); type(tail):=vcenter_noad;
            //       math_type(nucleus(tail)):=sub_box; info(nucleus(tail)):=p;
            //       end;
            //
            // The packaged box becomes a `vcenter_noad` nucleus on the
            // enclosing mlist. It never reaches §1075's `box_end`: §1073's
            // `scan_box` admits only `cur_cmd=make_box`, so a `\vcenter` can
            // be neither a `\setbox` target, a `\shipout` operand, a leader
            // payload, nor a `\raise`/`\lower` operand, and the whole box
            // context every other branch below classifies is inapplicable.
            if box_state.kind == ReplayBoxKind::VCenter {
                let boxed = stores.publish_page_nodes(vec![node]);
                modes.current_list_mutation().push(
                    stores,
                    Node::MathNoad(MathNoad::new(NoadKind::VCenter, MathField::SubBox(boxed))),
                );
                return Ok(ReplayStep::Continue);
            }
            if let Some(kind) = box_state.leader_kind {
                let payload =
                    crate::box_runtime::payload_from_node(node).ok_or(ExecError::MissingToken {
                        context: "leader box payload",
                    })?;
                boxes.pending_leader = Some((kind, payload));
            } else if *ships_out {
                let region = box_state
                    .shipout_region
                    .expect("constructed shipout retains its page region");
                debug_assert!(command.prepared_shipout.is_none());
                *command.prepared_shipout = Some(PreparedShipout {
                    source: PreparedShipoutSource::Page(node),
                    region: Some(region),
                });
            } else if let Some(target) = box_state.target {
                let boxed = stores.publish_page_nodes(vec![node]);
                commit_set_box_target(target, Some(boxed), stores, command);
            } else {
                // TeX82 §1076's `box_end` branch for an ordinary
                // (non-register, non-shipout, non-leader) box appends the
                // freshly built box to whatever list is currently open,
                // exactly like `\box<n>` (`box_end`'s `BoxContext::Append`
                // above): baseline-skip insertion, migration extraction, and
                // (in outer vertical mode) page-builder contribution all
                // apply. A bare `modes.current_list_mutation().push(node)` here
                // bypassed all of that, silently dropping every standalone
                // `\hbox`/`\vbox`/`\vtop` (and macros built on them, such as
                // plain.tex's `\centerline`) appended directly in vertical
                // mode: the node landed in the mode-nest list rather than the
                // page contribution list the page builder actually drains.
                //
                // TeX82 §1073's box-shift prefixes (`\raise`/`\lower`/
                // `\moveleft`/`\moveright`) reach exactly this branch: their
                // wrapped `\hbox`/`\vbox`/`\vtop` can never itself be a
                // `\setbox` target, a `\shipout` operand, or a leader payload
                // (`scan_box`'s `cur_cmd=make_box` requirement excludes
                // `vmove`/`hmove`), so `box_state.shift` is only ever set
                // here.
                let mut node = node;
                if let Some(shift) = box_state.shift {
                    crate::box_runtime::apply_box_shift_delta(&mut node, shift.delta)?;
                }
                crate::box_runtime::append_box_node_to_current_list(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    node,
                    command.fuel,
                )?;
                crate::vertical::build_page_if_outer_vertical(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.state.state(),
                )?;
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BeginAlignment { vertical, owner } => {
            // TeX82 §774's display-math entry accepts an alignment only when
            // the current formula is empty. `flush_math` owns both the
            // material and an incomplete fraction before `push_nest` opens
            // the alignment list; retaining either here makes §812's display
            // alignment handoff collide with pre-alignment math material.
            if modes.current_mode() == Mode::DisplayMath {
                let has_formula = !modes.current_list().nodes(stores).is_empty()
                    || modes.current_list().incomplete_fraction().is_some();
                if has_formula {
                    let primitive = if *vertical { "\\valign" } else { "\\halign" };
                    let context = command.state.output_open_context(&**stores);
                    let mut report = stores.print_err(&format!("Improper {primitive} inside $$'s"));
                    report.help(&[
                        "Displays can use special alignments (like \\eqalignno)",
                        "only if nothing but the alignment itself is between $$'s.",
                        "So I've deleted the formulas that preceded this alignment.",
                    ]);
                    report.context(context);
                    report.error().defer_recovery(command.diagnostic_effects)?;
                    let mut list = modes.current_list_mutation();
                    list.take_nodes();
                    list.take_incomplete_fraction();
                }
            }
            if let Some(outer) = active_alignment.take() {
                command
                    .state
                    .apply_alignment_request(&**stores, AlignmentRequest::Suspend(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment suspension",
                    })?;
                boxes.suspended_alignments.push(outer);
            }
            let identity = AlignmentIdentity::new(*next_alignment_identity);
            *next_alignment_identity = next_alignment_identity.wrapping_add(1);
            command
                .state
                .apply_alignment_request(&**stores, AlignmentRequest::Begin(identity))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment lifecycle",
                })?;
            let default_tabskip = stores
                .glue_param(GlueParam::TAB_SKIP)
                .map(|id| stores.glue(id))
                .unwrap_or(GlueSpec::ZERO);
            *active_alignment = Some(ActiveReplayAlignment {
                identity,
                kind: if *vertical {
                    AlignmentKind::VAlign
                } else {
                    AlignmentKind::HAlign
                },
                owner: *owner,
                packing: AlignmentPackSpec::Natural,
                columns: Vec::new(),
                repeat_start: None,
                column: 0,
                preamble_opening_pending: true,
                preamble_start_pending: false,
                cell_opening_pending: false,
                next_cell_opening_pending: false,
                align_peek_pending: false,
                align_peek_after_noalign: false,
                noalign_open: false,
                captured_row_count: 0,
                captured_cell_count: 0,
                tabskips: vec![default_tabskip],
                default_tabskip,
                row_migrations: tex_state::page_node_arena::PageListSpan::empty(),
                cell_span: 1,
                row_open: false,
                cell_open: false,
            });
            // TeX82 §774's `init_align` runs `push_nest` and then only
            // *negates* an ordinary vertical mode, so the alignment's own
            // list inherits that list's `aux` (`prev_depth`). Display math is
            // the deliberate exception: its `aux` is `incompleat_noad`, so
            // §774 reaches through it to `nest[nest_ptr-2].aux_field.sc`, the
            // enclosing vertical list's `prev_depth`. Umber's independent
            // mode levels do not copy either value on push, so select the
            // canonical source explicitly before opening the alignment.
            let enclosing_prev_depth = if modes.current_mode() == Mode::DisplayMath {
                modes.enclosing_vertical_prev_depth()
            } else {
                modes.current_list().prev_depth()
            };
            modes.push_at_line(
                replay_alignment_mode(if *vertical {
                    AlignmentKind::VAlign
                } else {
                    AlignmentKind::HAlign
                }),
                i32::try_from(command.state.current_file_line_number()).unwrap_or(i32::MAX),
            )?;
            if let Some(prev_depth) = enclosing_prev_depth {
                modes.current_list_mutation().set_prev_depth(prev_depth);
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::AlignmentPreambleOpening { alignment, packing } => {
            command
                .state
                .apply_alignment_request(&**stores, AlignmentRequest::Preamble(*alignment))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment preamble lifecycle",
                })?;
            if let Some(active) = active_alignment.as_mut()
                && active.identity == *alignment
            {
                active.packing = alignment_pack_spec(*packing);
                active.preamble_opening_pending = false;
                active.preamble_start_pending = true;
            }
            // TeX82 §774's `init_align` reaches the preamble through §645's
            // `scan_spec(align_group,false)`, whose `new_save_level(c)` opens
            // the save level that brackets the alignment as a whole. §800's
            // `fin_align` removes it with the second of its two `unsave`s.
            enter_group(
                stores,
                command.state,
                command.diagnostic_effects,
                GroupKind::Align,
            );
            Ok(ReplayStep::Continue)
        }
        ColdOperation::AlignmentPreambleStart { alignment } => {
            // `init_row` reaches `align_peek` before `init_col` selects the
            // first cell. Keep the first pair validated here, but defer
            // `BeginCell` until that lookahead has classified the next token.
            // In particular, a recovered preamble may be followed directly
            // by `}`, which `align_peek` passes to `fin_align`.
            debug_assert!(
                active_alignment.as_ref().is_some_and(
                    |active| active.identity == *alignment && !active.columns.is_empty()
                )
            );
            // TeX82 §774 closes `init_align` with a second
            // `new_save_level(align_group)`, the level that brackets one
            // alignment *entry*. §791's `fin_col` replaces it at every `&`,
            // `\\span`-free column end, and `\\cr`, so an assignment made in a
            // cell -- `\\bf`, `\\tt`, a `\\fam`, any local register -- is
            // restored before the next entry begins. §800's first `unsave`
            // removes the last one.
            enter_group(
                stores,
                command.state,
                command.diagnostic_effects,
                GroupKind::Align,
            );
            // §774 then runs
            // `if every_cr<>null then begin_token_list(every_cr,every_cr_text)`
            // before its own `align_peek`, exactly as §799 does at every later
            // row boundary. The push follows the entry save level, so the hook
            // is scoped to the entry it opens.
            schedule_everycr(command.state, stores);
            Ok(ReplayStep::Continue)
        }
        ColdOperation::BeginNoAlign { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == *alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            active.align_peek_pending = false;
            active.noalign_open = true;
            enter_group(
                stores,
                command.state,
                command.diagnostic_effects,
                GroupKind::NoAlign,
            );
            // TeX82 §785 leaves the alignment's own mode level in place when
            // `\noalign` opens. It calls `normal_paragraph` only for an
            // h-alignment's internal-vertical mode; a v-alignment is already
            // in restricted horizontal mode, but that level is the alignment
            // list itself, not a paragraph to pop.
            if modes.current_mode() == Mode::InternalVertical {
                crate::paragraph_end::normal_paragraph(modes, stores, command.diagnostic_effects);
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::AlignmentPeekCell { alignment, omit } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == *alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            let templates =
                active
                    .columns
                    .get(active.column)
                    .cloned()
                    .ok_or(ExecError::MissingToken {
                        context: "next alignment preamble column",
                    })?;
            command
                .state
                .begin_prepared_alignment_cell(*alignment, templates)
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment next-row lifecycle",
                })?;
            begin_replay_alignment_cell(active, modes, stores, command.diagnostic_effects)?;
            active.align_peek_pending = false;
            if *omit {
                command
                    .state
                    .apply_alignment_request(
                        &**stores,
                        AlignmentRequest::PrepareCellLookahead(*alignment),
                    )
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment omit lookahead lifecycle",
                    })?;
                command
                    .state
                    .apply_alignment_request(
                        &**stores,
                        AlignmentRequest::InstallOmitCellTemplate(*alignment),
                    )
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment omit-cell lifecycle",
                    })?;
            } else {
                // TeX82 §37 now calls `init_col`, which immediately pushes
                // the selected u-template above the command backed up by
                // `align_peek`. A second lookahead would re-deliver that
                // command before the template is installed.
                command
                    .state
                    .apply_alignment_request(
                        &**stores,
                        AlignmentRequest::InstallCellTemplate(*alignment),
                    )
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment next-row cell-template lifecycle",
                    })?;
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::NoAlignEndGroup { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == *alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            if !active.noalign_open {
                return Err(ExecError::MissingToken {
                    context: "noalign group",
                });
            }
            active.noalign_open = false;
            active.align_peek_pending = true;
            active.align_peek_after_noalign = true;
            // TeX82 §1133's whole `no_align_group` case of `handle_right_brace`
            // is `end_graf; unsave; align_peek`. A `\noalign` body is ordinary
            // internal vertical material, so anything horizontal in it (a
            // character, an `\hskip`, an `\indent`) starts a paragraph through
            // §1090 exactly as it would anywhere else in vertical mode, and the
            // closing brace is what line-breaks it back onto the alignment's own
            // vertical list. Without `end_graf` the paragraph stayed open across
            // the brace, so the following rows were built on the horizontal
            // level and `fin_align` popped that level instead of the alignment
            // (`umber2-usol`).
            let diagnostic_context = command_diagnostic_context(command, stores);
            let mut geometry = pack_geometry_sink(command.state, command.observations);
            crate::paragraph_end::end_paragraph_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                &mut geometry,
                diagnostic_context,
                command.fuel,
            )?;
            leave_group_payloads(
                stores,
                command.state,
                command.diagnostic_effects,
                GroupKind::NoAlign,
            )
            .map_err(|_| ExecError::MissingToken {
                context: "noalign group",
            })?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::AlignmentCellOpening { alignment, opening } => {
            command
                .state
                .apply_alignment_request(
                    &**stores,
                    match opening {
                        AlignmentCellOpening::Template => {
                            AlignmentRequest::InstallCellTemplate(*alignment)
                        }
                        AlignmentCellOpening::Omit => {
                            AlignmentRequest::InstallOmitCellTemplate(*alignment)
                        }
                    },
                )
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment cell-template lifecycle",
                })?;
            if let Some(active) = active_alignment.as_mut()
                && active.identity == *alignment
            {
                active.cell_opening_pending = false;
                active.next_cell_opening_pending = false;
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::AlignmentCellFinish { alignment } => {
            if matches!(
                modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ) {
                crate::box_runtime::flush_pending_hchars_with_fuel(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                )?;
            }
            let mut processor = command.processor(stores);
            let finished = processor
                .finish_alignment_cell(*alignment)
                .map_err(command_error);
            processor.retire();
            let finished = finished?;
            begin_next_replay_alignment_cell(
                *alignment,
                finished.delimiter,
                finished.delimiter_line,
                command,
                active_alignment,
                modes,
                stores,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::AlignmentFinish {
            alignment,
            current_line,
        } => {
            if active_alignment.as_ref().map(|active| active.identity) != Some(*alignment) {
                return Err(ExecError::MissingToken {
                    context: "active replay alignment",
                });
            }
            // TeX82 §800's `fin_align` opens with two `unsave`s -- "that
            // |align_group| was for individual entries", then "that
            // |align_group| was for the whole alignment" -- before it
            // determines the column widths and packages the prototype box.
            let entry_aftergroup = leave_fin_align_save_level(
                command.state,
                command.diagnostic_effects,
                stores,
                "align1",
            )?;
            let alignment_aftergroup = leave_fin_align_save_level(
                command.state,
                command.diagnostic_effects,
                stores,
                "align0",
            )?;
            let active = active_alignment
                .as_mut()
                .expect("active replay alignment was checked");
            let error_context = command.state.output_open_context(&**stores);
            let mut geometry = pack_geometry_sink(command.state, command.observations);
            finish_replay_alignment(
                active,
                modes,
                stores,
                command.diagnostic_effects,
                &mut geometry,
                command.fuel,
                AlignmentFinishSite::new(&error_context, *current_line),
            )?;
            schedule_aftergroup(command, stores, entry_aftergroup)?;
            schedule_aftergroup(command, stores, alignment_aftergroup)?;
            command
                .state
                .apply_alignment_request(&**stores, AlignmentRequest::Finish(*alignment))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment finish lifecycle",
                })?;
            *active_alignment = None;
            if let Some(outer) = boxes.suspended_alignments.pop() {
                command
                    .state
                    .apply_alignment_request(&**stores, AlignmentRequest::Resume(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment resumption",
                    })?;
                *active_alignment = Some(outer);
            }
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Paragraph { current_line } => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                crate::paragraph_end::normal_paragraph(modes, stores, command.diagnostic_effects);
                crate::vertical::build_page_if_outer_vertical(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.state.state(),
                )?;
            } else {
                let mut diagnostic_context = command_diagnostic_context(command, stores);
                diagnostic_context.current_line = *current_line;
                let mut geometry = pack_geometry_sink(command.state, command.observations);
                crate::paragraph_end::end_paragraph_with_fuel(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    &mut geometry,
                    diagnostic_context,
                    command.fuel,
                )?;
            }
            Ok(ReplayStep::Continue)
        }
        // TeX82 §1137 and §1193 need the mode nest, the save stack, and the
        // command processor's token-list scheduling together, so
        // `MainControl::apply_host_owned_step` applies this step for
        // every delivery entry point before `apply_cold_operation` runs.
        ColdOperation::MathShift { .. } => {
            unreachable!("apply_host_owned_step applies canonical math shifts")
        }
        ColdOperation::ParagraphStart => {
            start_paragraph(
                command.state,
                modes,
                stores,
                command.diagnostic_effects,
                true,
            )?;
            Ok(ReplayStep::Continue)
        }
        ColdOperation::Character {
            ch,
            cat,
            origin,
            suppress_left_boundary,
        } => {
            if matches!(modes.current_mode(), Mode::Math | Mode::DisplayMath) {
                if !matches!(cat, Catcode::Space) {
                    // TeX82 §1154's `mmode+letter,mmode+other_char:
                    // set_math_char(ho(math_code(cur_chr)))`.
                    set_math_char(*ch, *origin, stores, modes, command)?;
                }
                return Ok(ReplayStep::Continue);
            }
            match cat {
                // TeX82 §1045's `any_mode(relax),vmode+spacer,mmode+spacer,
                // mmode+no_boundary:do_nothing` leaves vertical mode
                // untouched by an ordinary space; only `start_par`, a
                // letter/other/char_num/char_given, or an explicit
                // box/rule/etc. triggers `new_graf` via §1090's
                // `back_input; new_graf(true)`. A space therefore never
                // itself opens a paragraph here.
                Catcode::Space => {
                    if matches!(
                        modes.current_mode(),
                        Mode::Horizontal | Mode::RestrictedHorizontal
                    ) {
                        crate::box_runtime::append_space_with_fuel(
                            modes,
                            stores,
                            command.diagnostic_effects,
                            command.fuel,
                        )?;
                    }
                }
                Catcode::Letter | Catcode::Other => {
                    if matches!(
                        modes.current_mode(),
                        Mode::Vertical | Mode::InternalVertical
                    ) {
                        start_paragraph(
                            command.state,
                            modes,
                            stores,
                            command.diagnostic_effects,
                            true,
                        )?;
                    }
                    modes
                        .current_list_mutation()
                        .set_no_boundary(*suppress_left_boundary);
                    crate::box_runtime::append_character_with_fuel(
                        modes,
                        stores,
                        command.diagnostic_effects,
                        *ch,
                        *origin,
                        command.state.profile() == CommandProfile::ETEX26,
                        command.fuel,
                    )?;
                }
                _ => unreachable!("canonical character scan restricts catcodes"),
            }
            Ok(ReplayStep::Continue)
        }
        // `apply_operation` consumes the command-owned episodes inside the
        // direct operation. Observed replay is not an alternate production
        // execution path, so reaching these arms is an invariant.
        ColdOperation::DiscretionaryOpening(_) | ColdOperation::DiscretionaryPartEnd => {
            unreachable!("discretionary is applied by MainControl")
        }
        ColdOperation::DiscretionaryHyphen { .. } => {
            unreachable!("discretionary hyphen is applied by MainControl")
        }
        ColdOperation::Accent(_) => {
            unreachable!("accent is applied by MainControl")
        }
    }
}
