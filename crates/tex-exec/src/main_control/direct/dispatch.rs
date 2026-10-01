//! Native dispatch in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn run_command(
        &mut self,
        settled: Option<tex_command::CurrentCommand<G>>,
        alignment: Option<AlignmentIdentity>,
        set_box_allowed: bool,
        mut initial_prefix: Option<(bool, MeaningFlags)>,
    ) -> Result<ReplayStep, ExecError> {
        let mode = self.control.modes.current_mode();
        let boxes = &self.control.boxes;
        let display_eq_no = self.control.modes.current_list().display_eq_no().is_some();
        let main_loop_active = self.control.main_loop_active;
        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");
        let innermost_group = admitted_context.innermost_group_kind();
        let job_is_all_over = crate::page_output::job_is_all_over(&admitted_context);
        let mut source_command = None;
        if settled.is_none()
            && main_loop_active
            && matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
            && self.control.active_alignment.is_none()
            && self.control.operation_observations.is_none()
            && !admitted_context.tracked_region_is_active()
            && !self.control.page_region_succession_pending
        {
            let etex_extended = self.control.command_profile() == CommandProfile::ETEX26;
            let mut forbidden_facts = CommandMachineHostFacts::Forbidden;
            let mut source_processor = command_processor(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut forbidden_facts,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            );
            let mut run_error = None;
            let mut borrowed_applied = false;
            let mut count = 0;
            let mut continues = true;
            let status = {
                let mut consume = characters::MainSourceAdmission {
                    modes: &mut self.control.modes,
                    etex_extended,
                    run_error: &mut run_error,
                    borrowed_applied: &mut borrowed_applied,
                    count: &mut count,
                    continues: &mut continues,
                };
                source_processor.main_loop_source_step_into(&mut source_command, &mut consume)
            };
            self.cursor = Some(source_processor.delivery_cursor());
            self.diagnostics.extend(
                source_processor
                    .take_semantic_diagnostics()
                    .into_iter()
                    .map(PendingDiagnostic::Command),
            );
            drop(source_processor);
            if let Some(error) = run_error {
                return Err(error);
            }
            match status.map_err(command_error)? {
                tex_command::DeliveryStatus::CharacterRun if borrowed_applied || count != 0 => {
                    self.control.main_loop_active = continues;
                    drop(admitted_context);
                    self.report_scan()?;
                    return Ok(ReplayStep::Continue);
                }
                tex_command::DeliveryStatus::CharacterRunBoundary
                | tex_command::DeliveryStatus::CharacterRun => {}
                _ => unreachable!("shared character consumer returns a run or its scalar boundary"),
            }
        }
        let mut host_facts = ExecutorHostFacts {
            modes: &self.control.modes,
            pdf_ignore_depth: self.control.pdf_ignore_depth,
            telemetry: &mut self.control.episode_telemetry,
        };
        let mut owned_processor = if let Some(provider) = self.provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts,
                provider,
                &mut self.control.declined_resource_attempt,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        } else {
            command_processor(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        };
        owned_processor.set_output_routine_active(boxes.output_routine_active);
        if let Some(cursor) = self.cursor {
            owned_processor.resume_delivery_cursor(cursor);
        }
        let processor: &mut CommandProcessor<'_, '_, G> = &mut owned_processor;
        // Finish this actual processor/context loan before semantic application.
        macro_rules! finish_scan {
            ($call:expr) => {
                direct_finish_scan!(self, owned_processor, admitted_context, host_facts, $call)
            };
        }

        let mut scalar_storage = tex_command::ScalarScanFrame::default();
        let scalar = &mut scalar_storage;
        let shown_mode = &mut self.control.shown_mode;
        let diagnostics = &mut self.diagnostics;
        let mut scan_origin = self.current_origin;
        let mut command = if let Some(command) = settled {
            command
        } else {
            // TeX82 §1030 has two fetch labels, not one. `big_switch` uses
            // `get_x_token`; §1034's inner character loop instead re-enters at
            // §1038's `main_loop_lookahead`, whose bare `get_next` is what keeps a
            // run of adjacent characters from being delivered through expansion.
            prepare_command_trace(processor, mode, *shown_mode);
            let mut destination = source_command;
            let delivery = if main_loop_active {
                processor.main_loop_lookahead_into(&mut destination)
            } else {
                processor.preflight_command_into(&mut destination)
            };
            match (delivery.map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?
            {
                tex_command::DeliveryStatus::End => {
                    return finish_scan!(self.execute_end_of_input());
                }
                tex_command::DeliveryStatus::ReplayCompleted(episode) => {
                    return {
                        let (direct_arg0,) = (episode,);
                        finish_scan!(self.execute_replay_completed(direct_arg0))
                    };
                }
                tex_command::DeliveryStatus::Command => {}
                _ => unreachable!("main-control delivery has no alignment event"),
            };
            let command = destination.expect("command status initializes destination");
            // TeX82 §§1034/1038 keeps a fetched character inside `main_loop`;
            // it reaches neither `reswitch` nor §1030's command trace. A
            // non-character fetched by the same lookahead does go to `reswitch` and
            // must retain the ordinary trace boundary.
            let continues_main_loop = main_loop_active
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(
                        Meaning::CharToken {
                            cat: Catcode::Letter | Catcode::Other,
                            ..
                        } | Meaning::CharGiven(_)
                            | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                    )
                );
            if !continues_main_loop {
                report_main_control_command_trace(processor, mode, &command, boxes, shown_mode);
            }
            if main_loop_active
                && matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::NoBoundary
                    ))
                )
            {
                return {
                    let (direct_arg0,) = (true,);
                    finish_scan!(self.execute_no_boundary(direct_arg0))
                };
            }

            command
        };
        scan_origin = Some(command.origin());
        self.current_origin = scan_origin;

        // TeX82 §1078 uses §404's non-blank, non-relax fetch after every leader
        // payload. Constructed boxes close in a separate replay step, so the first
        // token after the box has already reached this dispatcher. Finish §404
        // here without exposing its filler to main control or command tracing.
        if boxes.pending_leader.is_some()
            && matches!(
                command.meaning(),
                ResolvedMeaning::Static(
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } | Meaning::Relax
                )
            )
        {
            let mut destination = None;
            if (next_non_blank_non_relax_x_token_into(processor, &mut destination)
                .map_err(command_error))
            .map_err(|error| scan_error(error, scan_origin))?
                != tex_command::DeliveryStatus::Command
            {
                return Err(scan_error(
                    ExecError::MissingToken {
                        context: "leader glue",
                    },
                    scan_origin,
                ));
            }
            command = destination
                .take()
                .expect("command status initializes destination");
        }

        // TeX82 §1078 fetches the command following a completed leader payload
        // inside `box_end`, before control returns to §1030's `big_switch` or
        // §1211's prefix loop. Split replay finishes the box in one step and
        // delivers that command in the next, so classify it at this same outer
        // boundary. In particular, a non-glue `\global` is the command that
        // `back_error` must restore; allowing it into the prefix loop first would
        // consume and restore the following assignment instead.
        if let Some((kind, payload)) = boxes.pending_leader.as_ref() {
            let result = LeaderGlueResult::Payload {
                kind: *kind,
                payload: *payload,
            };
            return {
                let (direct_arg0, direct_arg1) = (Some(command), result);
                finish_scan!(self.scan_leader_glue_command(direct_arg0, direct_arg1))
            };
        }
        // §1030's `reswitch:` label sits *above* the big case, not at the fetch:
        // a case that has already fetched its own replacement command dispatches
        // that command in place. `goto reswitch` is therefore not `back_input`,
        // and a case using it pushes no input level and delivers nothing twice.
        // This loop is that label.
        let mut suppress_left_boundary = false;
        loop {
            // A prefix the caller's own §1211 loop consumed has already fetched
            // the current command, which is checked exactly as a command this
            // loop fetches.
            let mut prefixed = initial_prefix.is_some();
            let (mut global, mut flags) = initial_prefix
                .take()
                .unwrap_or((false, MeaningFlags::EMPTY));
            loop {
                if prefixed {
                    // §1211's `if cur_cmd<=max_non_prefixed_command then <Discard
                    // erroneous prefixes and return>`: §209's partition, not a
                    // hand-listed set of assignment families.
                    if !tex_command::exceeds_max_non_prefixed_command(static_meaning(
                        command.meaning(),
                    )) {
                        let printed = tex_command::PrintCommand::from_current(&command);
                        // §1212's `back_error`: the substantive command is retained
                        // and re-delivered without the discarded prefixes.
                        let site = processor.capture_diagnostic_site(Some(&command));
                        (processor.back_input(command).map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                        // `back_error` is `back_input` *then* `error`, so §82 renders
                        // the context with the backed-up level already on the stack.
                        let etex = processor.profile().capabilities().supports_etex();
                        let site = Some(processor.complete_diagnostic_site(site));
                        diagnostics.push(PendingDiagnostic::PrefixOnNonPrefixedCommand(
                            printed,
                            processor.error_context(),
                            etex,
                            site,
                        ));
                        return finish_scan!(self.execute_continue());
                    }
                }
                #[cfg(feature = "profiling")]
                {
                    let meaning = (&command.meaning());
                    tex_state::measurement::record_hot_core_command_family(
                        hot_core_command_family(meaning),
                    );
                    tex_state::measurement::record_hot_core_main_control_meaning(
                        hot_core_meaning_family(meaning),
                    );
                    if let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
                        meaning
                    {
                        tex_state::measurement::record_hot_core_unexpandable_opcode(
                            usize::try_from(primitive.operand())
                                .expect("unexpandable primitive operand fits usize"),
                        );
                    }
                }
                match command.meaning() {
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Global,
                    )) => global = true,
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Long,
                    )) => flags = flags | MeaningFlags::LONG,
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Outer,
                    )) => flags = flags | MeaningFlags::OUTER,
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Protected,
                    )) => flags = flags | MeaningFlags::PROTECTED,
                    _ => break,
                }
                let mut destination = None;
                let next = match next_non_blank_non_relax_x_token_into(processor, &mut destination)
                {
                    Ok(tex_command::DeliveryStatus::Command) => destination
                        .take()
                        .expect("command status initializes destination"),
                    Ok(tex_command::DeliveryStatus::End) => {
                        return Err(scan_error(ExecError::MissingPrefixedCommand, scan_origin));
                    }
                    Ok(_) => unreachable!("ordinary expanded delivery returns only commands"),
                    Err(error) => return Err(scan_error(command_error(error), scan_origin)),
                };
                command = next;
                prefixed = true;
            }
            // §1213's `<Discard the prefixes \long and \outer if they are
            // irrelevant>`. §1214 deliberately leaves `a` unadjusted, so the
            // command still runs; only the report is owed. eTeX's `\protected`
            // is prefix code 8, which §1213's `a mod 4<>0` excludes.
            if flags.bits() & (MeaningFlags::LONG | MeaningFlags::OUTER).bits() != 0
                && !matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Def
                            | UnexpandablePrimitive::Edef
                            | UnexpandablePrimitive::Gdef
                            | UnexpandablePrimitive::Xdef
                    ))
                )
            {
                let etex = processor.profile().capabilities().supports_etex();
                let site = Some(processor.current_diagnostic_site(Some(&command)));
                diagnostics.push(PendingDiagnostic::IrrelevantLongOuterPrefix(
                    tex_command::PrintCommand::from_current(&command),
                    processor.error_context(),
                    etex,
                    site,
                ));
            }
            // §406's helper is `repeat get_x_token until cur_cmd<>spacer` --
            // exactly `next_non_space` -- and the command it leaves in `cur_cmd`
            // is then dispatched by the case itself. Backing it up instead would
            // push a backup level, emit a recovery record, and deliver that
            // command a second time, none of which TeX82 does
            // (`umber2-johp.196`).
            if matches!(
                command.meaning(),
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::IgnoreSpaces
                ))
            ) {
                let next =
                    if let Some(alignment) = alignment {
                        let mut destination = None;
                        loop {
                            match (processor
                                .get_x_alignment_delivery_into(false, &mut destination)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?
                            {
                                tex_command::DeliveryStatus::End => {
                                    return finish_scan!(self.execute_end_of_input());
                                }
                                tex_command::DeliveryStatus::ReplayCompleted(episode) => {
                                    return {
                                        let (direct_arg0,) = (episode,);
                                        finish_scan!(self.execute_replay_completed(direct_arg0))
                                    };
                                }
                                tex_command::DeliveryStatus::AlignmentEndTemplate => {
                                    let event = tex_command::AlignmentDeliveryEvent::EndTemplate(
                                        destination
                                            .take()
                                            .expect("alignment status initializes destination"),
                                    );
                                    return {
                                        let (direct_arg0, direct_arg1) = (alignment, event);
                                        finish_scan!(self.scan_alignment_delivery_event(
                                            direct_arg0,
                                            direct_arg1
                                        ))
                                    };
                                }
                                tex_command::DeliveryStatus::AlignmentClosingBrace => {
                                    let event = tex_command::AlignmentDeliveryEvent::ClosingBrace(
                                        destination
                                            .take()
                                            .expect("alignment status initializes destination"),
                                    );
                                    return {
                                        let (direct_arg0, direct_arg1) = (alignment, event);
                                        finish_scan!(self.scan_alignment_delivery_event(
                                            direct_arg0,
                                            direct_arg1
                                        ))
                                    };
                                }
                                tex_command::DeliveryStatus::Command
                                    if matches!(
                                        destination
                                            .as_ref()
                                            .expect("command status initializes destination")
                                            .meaning(),
                                        ResolvedMeaning::Static(Meaning::CharToken {
                                            cat: Catcode::Space,
                                            ..
                                        })
                                    ) =>
                                {
                                    destination = None
                                }
                                tex_command::DeliveryStatus::Command => {
                                    break destination
                                        .take()
                                        .expect("command status initializes destination");
                                }
                                tex_command::DeliveryStatus::PendingExpanded => {
                                    unreachable!(
                                        "alignment delivery commits terminal observations"
                                    );
                                }
                                tex_command::DeliveryStatus::CharacterRun
                                | tex_command::DeliveryStatus::CharacterRunBoundary => {
                                    unreachable!(
                                        "alignment delivery does not admit main-loop character runs"
                                    );
                                }
                            }
                        }
                    } else {
                        let mut destination = None;
                        if (next_non_blank_x_token_into(processor, &mut destination)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?
                            != tex_command::DeliveryStatus::Command
                        {
                            return finish_scan!(self.execute_end_of_input());
                        }
                        destination
                            .take()
                            .expect("command status initializes destination")
                    };
                command = next;
                report_command_trace(processor, mode, &command, shown_mode);
                continue;
            }
            if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::NoBoundary
                    ))
                )
            {
                let mut destination = None;
                if (processor
                    .get_x_token_into(&mut destination)
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?
                    != tex_command::DeliveryStatus::Command
                {
                    return finish_scan!(self.execute_continue());
                }
                let next = destination
                    .take()
                    .expect("command status initializes destination");
                suppress_left_boundary = matches!(
                    next.meaning(),
                    ResolvedMeaning::Static(
                        Meaning::CharToken {
                            cat: Catcode::Letter | Catcode::Other,
                            ..
                        } | Meaning::CharGiven(_)
                            | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                    )
                );
                command = next;
                report_command_trace(processor, mode, &command, shown_mode);
                continue;
            }
            // TeX82 §1214 resolves `\globaldefs` exactly once, before entering
            // §1211's assignment case. Every scanner-time provisional
            // definition, committed application, and mutation observation below
            // therefore receives the same effective value rather than
            // independently consulting live state at a later seam.
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                global
                    || matches!(
                        command.meaning(),
                        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
                        ))
                    ),
            );

            self.suppress_left_boundary = suppress_left_boundary;

            if let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::TextFont
                | UnexpandablePrimitive::ScriptFont
                | UnexpandablePrimitive::ScriptScriptFont),
            )) = command.meaning()
            {
                let size = tex_command::MathFamilySize::of_primitive(primitive)
                    .expect("the outer match restricts this to `def_family`");
                return {
                    let (direct_arg0, direct_arg1) = (size, global);
                    finish_scan!(self.scan_math_family_assignment(direct_arg0, direct_arg1))
                };
            }
            // Math operands are scanned exclusively by `tex-command`.  The replay
            // driver receives a typed scalar request and schedules any opaque field
            // episode only after this processor borrow has ended.
            if matches!(mode, Mode::Math | Mode::DisplayMath)
                && let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    primitive @ (UnexpandablePrimitive::Left
                    | UnexpandablePrimitive::Right
                    | UnexpandablePrimitive::Middle),
                )) = command.meaning()
            {
                let kind = match primitive {
                    UnexpandablePrimitive::Left => MathDelimiterBoundaryKind::Left,
                    UnexpandablePrimitive::Right => MathDelimiterBoundaryKind::Right,
                    UnexpandablePrimitive::Middle => MathDelimiterBoundaryKind::Middle,
                    _ => unreachable!(),
                };
                let boundary = match processor.scan_math_delimiter_boundary(kind) {
                    Ok(boundary) => boundary,
                    Err(error) => {
                        let error = command_error(error);
                        return Err(scan_error(error, scan_origin));
                    }
                };
                return {
                    let (direct_arg0,) = (boundary,);
                    finish_scan!(self.execute_math_delimiter(direct_arg0))
                };
            }
            if matches!(mode, Mode::Math | Mode::DisplayMath)
                && let Some(request) =
                    (processor.scan_math_request(&command).map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?
            {
                return {
                    let (direct_arg0,) = (request,);
                    finish_scan!(self.execute_math(direct_arg0))
                };
            }
            if matches!(mode, Mode::Math | Mode::DisplayMath)
                && let ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::Superscript,
                    ..
                }) = command.meaning()
            {
                return {
                    let (direct_arg0,) = (MathRequest::Script(tex_command::ScannedMathScript {
                        kind: MathScriptKind::Superscript,
                        provenance: tex_command::StructuredProvenance {
                            primary: command.origin(),
                        },
                    }),);
                    finish_scan!(self.execute_math(direct_arg0))
                };
            }
            if matches!(mode, Mode::Math | Mode::DisplayMath)
                && let ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::Subscript,
                    ..
                }) = command.meaning()
            {
                return {
                    let (direct_arg0,) = (MathRequest::Script(tex_command::ScannedMathScript {
                        kind: MathScriptKind::Subscript,
                        provenance: tex_command::StructuredProvenance {
                            primary: command.origin(),
                        },
                    }),);
                    finish_scan!(self.execute_math(direct_arg0))
                };
            }

            if boxes.output_routine_opening_pending
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::BeginGroup,
                        ..
                    })
                )
            {
                return finish_scan!(self.execute_output_routine_opening_brace());
            }
            // `align_error`'s inserted brace is an actual execution group, even when
            // it appears inside a replayed box body.  It must therefore win over the
            // box body's brace-depth bookkeeping so §1131 can observe it at end-v.
            if boxes.recovery_simple_group_pending
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::BeginGroup,
                        ..
                    })
                )
            {
                return finish_scan!(self.execute_begin_simple_group());
            }
            if boxes.recovery_simple_group_open
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::EndGroup,
                        ..
                    })
                )
            {
                return finish_scan!(self.execute_end_simple_group());
            }
            // TeX82 §1068 dispatches a right brace from the current `cur_group`.
            // An ancestor simple group must not make a nested box's body closer look
            // like an ordinary group closer.
            if innermost_group == Some(GroupKind::Simple)
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::EndGroup,
                        ..
                    })
                )
            {
                return {
                    let (direct_arg0, direct_arg1) = (GroupKind::Simple, "ordinary simple group");
                    finish_scan!(self.execute_leave_group(direct_arg0, direct_arg1))
                };
            }
            // TeX82 §1186's `math_group` arm of `handle_right_brace` (the brace that
            // closes a subformula scanned by §1151's `scan_math`) and §1174's
            // `math_choice_group` arm (the brace that closes one `\mathchoice`
            // branch). §1153 and §1172/§1174 opened these levels with `push_math`,
            // so each pair really does bracket a save-stack level and its closer must
            // not fall through to the ordinary or box brace arms below.
            if let Some(kind @ (GroupKind::Math | GroupKind::MathChoice)) = innermost_group
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::EndGroup,
                        ..
                    })
                )
            {
                return {
                    let (direct_arg0,) = (kind,);
                    finish_scan!(self.execute_end_math_group(direct_arg0))
                };
            }
            if innermost_group == Some(GroupKind::Disc)
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::EndGroup,
                        ..
                    })
                )
            {
                return finish_scan!(self.execute_discretionary_part_end());
            }
            // TeX82 §1150's `mmode+left_brace`: a bare explicit brace encountered
            // directly in math mode starts a subformula that becomes the nucleus of a
            // freshly appended noad -- `tail_append(new_noad); back_input;
            // scan_math(nucleus(tail))` -- rather than an ordinary `simple_group`
            // scope. This must be checked before the general brace arms below: a math
            // formula nested inside an active box body (for example plain.tex's
            // `\maketable` macro, which replays its whole `\halign` argument inside
            // `\setbox1=\vbox{#2}`) otherwise had its bare `{`/`}` swallowed with no
            // noad ever appended, so a following `^`/`_` incorrectly saw the
            // *enclosing* list's last node (an ordinary character, from *outside*
            // the formula) as its attachment target instead of a fresh empty
            // nucleus. Reusing the existing `TextField(Ord)` request (the same
            // completed-field plumbing `\mathord{...}` already drives) is exact:
            // `scan_math`'s brace case and `\mathord`'s explicit field scan both
            // bottom out in one §1153 `math_group`/`fin_mlist` cycle, and an
            // Ord-classified noad is what an unornamented brace group produces. A
            // box's own mandatory opening brace never reaches this dispatch at all:
            // `scan_left_brace` (TeX82 §403) consumed it while the construction was
            // still being scanned.
            if matches!(mode, Mode::Math | Mode::DisplayMath)
                && let ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                }) = command.meaning()
            {
                (processor.back_input(command).map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                return {
                    let (direct_arg0,) = (MathRequest::TextField(MathTextFieldKind::Ord),);
                    finish_scan!(self.execute_math(direct_arg0))
                };
            }
            // TeX82 §1068's `handle_right_brace` dispatches purely on `cur_group`, so
            // a box body's own closing brace is exactly the one delivered while the
            // innermost group is still the group `scan_spec`/`begin_insert_or_adjust`
            // opened for that body. Braces nested inside the body opened ordinary
            // `simple_group` levels of their own (§1063), and §1069's `simple_group:
            // unsave` -- reached through the `EndOrdinaryGroup` arm above -- closes
            // those. No separate brace-depth count is kept: the save stack already
            // holds every open level, and counting braces instead silently skipped
            // `unsave`, losing both the nested group's local restores and the
            // `\aftergroup` tokens §282 backs up when it pops.
            if let Some(box_state) = boxes.active_boxes.last()
                && innermost_group == Some(box_state.group_kind)
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::EndGroup,
                        ..
                    })
                )
            {
                let threshold = match box_state.kind {
                    ReplayBoxKind::VBox | ReplayBoxKind::VTop | ReplayBoxKind::VCenter => 1,
                    ReplayBoxKind::Insert(..) => 2,
                    ReplayBoxKind::HBox => 0,
                };
                if threshold != 0 && partoken_context_replays(processor, mode, threshold) {
                    (processor
                        .insert_partoken_before(command)
                        .map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                    return finish_scan!(self.execute_continue());
                }
                return {
                    let (direct_arg0, direct_arg1) = (
                        box_state.shipout_region.is_some(),
                        i32::try_from(processor.current_file_line_number()).unwrap_or(i32::MAX),
                    );
                    finish_scan!(self.execute_box_end_group(direct_arg0, direct_arg1))
                };
            }
            // TeX82 §1016 opens `output_group` before replaying the braced output
            // token list. A box body nested in that list owns its closing brace first;
            // only the live output group can close the enclosing output routine.
            if boxes.output_routine_active
                && innermost_group == Some(GroupKind::Output)
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::EndGroup,
                        ..
                    })
                )
            {
                if partoken_context_replays(processor, mode, 2) {
                    (processor
                        .insert_partoken_before(command)
                        .map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                    return finish_scan!(self.execute_continue());
                }
                return finish_scan!(self.execute_end_output_routine());
            }
            // TeX82 §1090's `@<Cases of |main_control| that build boxes and lists@>`
            // opens with one shared vertical-mode case, not thirteen separate ones:
            //
            //     vmode+letter,vmode+other_char,vmode+char_num,vmode+char_given,
            //     vmode+math_shift,vmode+un_hbox,vmode+vrule,
            //     vmode+accent,vmode+discretionary,vmode+hskip,vmode+valign,
            //     vmode+ex_space,vmode+no_boundary:
            //       begin back_input; new_graf(true); end;
            //
            // Every member takes the same two actions in the same order, *before* any
            // operand of its own is looked at: the triggering command is pushed back
            // (§325 `back_input`, which opens a `backed_up` input level), and §1091
            // `new_graf` then opens the paragraph and pushes `\everypar`. The backed-up
            // command is redelivered afterwards and dispatched again, now in horizontal
            // mode, where it scans its operand.
            //
            // Scanning the operand here instead -- `\char`'s character number,
            // `\hskip`'s glue, `\vrule`'s rule spec, `\accent`'s accent number and base
            // character, `\discretionary`'s three lists -- reads it in vertical mode,
            // before `\everypar` has run and before the paragraph's horizontal list
            // exists, and skips the backup level and redelivery entirely.
            if matches!(mode, Mode::Vertical | Mode::InternalVertical)
                && starts_paragraph_in_vertical_mode(command.meaning())
            {
                (processor.back_input(command).map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                return finish_scan!(self.execute_paragraph_start());
            }

            return {
                let command_origin = command.origin();
                match (
                    mode,
                    command
                        .control_code()
                        .expect("expanded delivery selected a static or font command"),
                ) {
                    (_, codes::CHAR_BEGIN_GROUP) => {
                        return {
                            let (direct_arg0,) = (GroupKind::Simple,);
                            finish_scan!(self.execute_enter_group(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_BEGIN_GROUP) => {
                        return {
                            let (direct_arg0,) = (GroupKind::SemiSimple,);
                            finish_scan!(self.execute_enter_group(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_END_GROUP)
                        if innermost_group == Some(GroupKind::SemiSimple) =>
                    {
                        return {
                            let (direct_arg0, direct_arg1) =
                                (GroupKind::SemiSimple, "semi simple group");
                            finish_scan!(self.execute_leave_group(direct_arg0, direct_arg1))
                        };
                    }
                    (_, codes::PRIMITIVE_CAT_CODE) => {
                        let (character, value) = (scan_code_table_operands(processor, scalar))
                            .map_err(|error| scan_error(error, scan_origin))?;
                        return {
                            let (direct_arg0, direct_arg1, direct_arg2) =
                                (character, value, global);
                            finish_scan!(self.execute_catcode(
                                direct_arg0,
                                direct_arg1,
                                direct_arg2
                            ))
                        };
                    }
                    (
                        _,
                        codes::PRIMITIVE_DEF
                        | codes::PRIMITIVE_EDEF
                        | codes::PRIMITIVE_GDEF
                        | codes::PRIMITIVE_XDEF,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::Def
                            | UnexpandablePrimitive::Edef
                            | UnexpandablePrimitive::Gdef
                            | UnexpandablePrimitive::Xdef),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let definition = (processor
                                .scan_macro_definition(matches!(
                                    primitive,
                                    UnexpandablePrimitive::Edef | UnexpandablePrimitive::Xdef
                                ))
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                                    (definition.target, definition.definition, flags, global);
                                finish_scan!(self.execute_macro_definition(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2,
                                    direct_arg3
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_LET | codes::PRIMITIVE_FUTURE_LET) => {
                        let _meaning = static_meaning(command.meaning());
                        {
                            let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                                primitive @ (UnexpandablePrimitive::Let
                                | UnexpandablePrimitive::FutureLet),
                            )) = command.meaning()
                            else {
                                unreachable!("packed command selected its native operand family")
                            };
                            {
                                let (target, meaning) = (processor
                                    .scan_let_assignment(
                                        primitive == UnexpandablePrimitive::FutureLet,
                                    )
                                    .map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?;
                                return {
                                    let (direct_arg0, direct_arg1, direct_arg2) =
                                        (target, meaning, global);
                                    finish_scan!(self.execute_let(
                                        direct_arg0,
                                        direct_arg1,
                                        direct_arg2
                                    ))
                                };
                            }
                        }
                    }
                    (_, codes::RELAX) => finish_scan!(self.execute_relax()),
                    (_, codes::CHAR_END_GROUP) => {
                        let (direct_arg0,) = (match innermost_group {
                            Some(GroupKind::SemiSimple) => Some(ForgottenGroupOpener::EndGroup),
                            Some(GroupKind::MathShift) => Some(ForgottenGroupOpener::MathShift),
                            Some(GroupKind::MathLeft) => Some(ForgottenGroupOpener::Right),
                            _ => None,
                        },);
                        finish_scan!(self.execute_extra_right_brace(direct_arg0))
                    }
                    (_, codes::PRIMITIVE_END_GROUP) => {
                        // TeX82 §1215's `\let` copies `cur_cmd`/`cur_chr`, so every
                        // user control sequence with `end_group` meaning takes §1063's
                        // dispatch irrespective of its spelling. Inaccessible alignment
                        // sentinels have the distinct `EndV`/`EndTemplate` meanings and
                        // remain owned by their dedicated paths.
                        return {
                            let (direct_arg0,) = (command,);
                            finish_scan!(self.scan_off_save(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_END | codes::PRIMITIVE_DUMP)
                        if mode == Mode::Horizontal =>
                    {
                        (processor
                            .recover_stop_for_vertical_mode(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_continue());
                    }
                    (_, codes::PRIMITIVE_END | codes::PRIMITIVE_DUMP)
                        if mode == Mode::RestrictedHorizontal =>
                    {
                        return {
                            let (direct_arg0,) = (command,);
                            finish_scan!(self.scan_off_save(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_END | codes::PRIMITIVE_DUMP)
                        if matches!(mode, Mode::Math | Mode::DisplayMath) =>
                    {
                        (processor
                            .recover_missing_math_shift(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_missing_math_shift());
                    }
                    (_, codes::PRIMITIVE_END | codes::PRIMITIVE_DUMP) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            // §1051's `privileged`: `mode>0` only. Internal vertical mode
                            // (inside a `\\vbox`, an `\\insert`, or `\\output` itself) reports
                            // an illegal case and leaves the job running.
                            if mode != Mode::Vertical {
                                return {
                                    let (direct_arg0,) = (command.spelling().semantic_token(),);
                                    finish_scan!(self.execute_illegal_stop(direct_arg0))
                                };
                            }
                            if job_is_all_over {
                                // §1335's `final_cleanup` unwinds the input stack that
                                // `main_control`'s return has abandoned.
                                let incomplete_conditions = processor.final_cleanup();
                                return {
                                    let (direct_arg0, direct_arg1) = (
                                        matches!(
                                            meaning,
                                            Meaning::UnexpandablePrimitive(
                                                UnexpandablePrimitive::Dump
                                            )
                                        ),
                                        incomplete_conditions,
                                    );
                                    finish_scan!(self.execute_end(direct_arg0, direct_arg1))
                                };
                            }
                            (processor.back_input(command).map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?;
                            return finish_scan!(self.execute_eject_residual_page());
                        }
                    }
                    (_, codes::PRIMITIVE_COUNT) => {
                        return {
                            let (direct_arg0, direct_arg1) = (None, global);
                            finish_scan!(
                                self.scan_count_register_assignment(direct_arg0, direct_arg1)
                            )
                        };
                    }
                    (_, codes::COUNT_REGISTER) => {
                        let ResolvedMeaning::Static(Meaning::CountRegister(index)) =
                            command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            return {
                                let (direct_arg0, direct_arg1) = (Some(index), global);
                                finish_scan!(
                                    self.scan_count_register_assignment(direct_arg0, direct_arg1)
                                )
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_DIMEN) => {
                        return {
                            let (direct_arg0, direct_arg1) = (None, global);
                            finish_scan!(
                                self.scan_dimension_register_assignment(direct_arg0, direct_arg1)
                            )
                        };
                    }
                    (_, codes::PRIMITIVE_WD | codes::PRIMITIVE_HT | codes::PRIMITIVE_DP) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::Wd
                            | UnexpandablePrimitive::Ht
                            | UnexpandablePrimitive::Dp),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let dimension = match primitive {
                                UnexpandablePrimitive::Wd => tex_state::BoxDimension::Width,
                                UnexpandablePrimitive::Ht => tex_state::BoxDimension::Height,
                                UnexpandablePrimitive::Dp => tex_state::BoxDimension::Depth,
                                _ => unreachable!(),
                            };
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (None, dimension, global);
                                finish_scan!(self.scan_box_dimension_assignment(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::DIMEN_REGISTER) => {
                        let ResolvedMeaning::Static(Meaning::DimenRegister(index)) =
                            command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            return {
                                let (direct_arg0, direct_arg1) = (Some(index), global);
                                finish_scan!(
                                    self.scan_dimension_register_assignment(
                                        direct_arg0,
                                        direct_arg1
                                    )
                                )
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_SKIP) => {
                        return {
                            let (direct_arg0, direct_arg1, direct_arg2) = (None, global, false);
                            finish_scan!(self.scan_glue_register_assignment(
                                direct_arg0,
                                direct_arg1,
                                direct_arg2
                            ))
                        };
                    }
                    (_, codes::SKIP_REGISTER) => {
                        let ResolvedMeaning::Static(Meaning::SkipRegister(index)) =
                            command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (Some(index), global, false);
                                finish_scan!(self.scan_glue_register_assignment(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_MUSKIP) => {
                        return {
                            let (direct_arg0, direct_arg1, direct_arg2) = (None, global, true);
                            finish_scan!(self.scan_glue_register_assignment(
                                direct_arg0,
                                direct_arg1,
                                direct_arg2
                            ))
                        };
                    }
                    (_, codes::MUSKIP_REGISTER) => {
                        let ResolvedMeaning::Static(Meaning::MuskipRegister(index)) =
                            command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (Some(index), global, true);
                                finish_scan!(self.scan_glue_register_assignment(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_H_SKIP) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (meaning, global, command_origin);
                                finish_scan!(self.scan_unary_scalar_operation(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_KERN) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (meaning, global, command_origin);
                                finish_scan!(self.scan_unary_scalar_operation(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_PENALTY) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (meaning, global, command_origin);
                                finish_scan!(self.scan_unary_scalar_operation(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_CONTROL_SPACE) => {
                        return finish_scan!(self.execute_control_space());
                    }
                    (_, codes::PRIMITIVE_PREV_DEPTH) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                                return {
                                    let (direct_arg0, direct_arg1, direct_arg2) =
                                        (meaning, global, command_origin);
                                    finish_scan!(self.scan_unary_scalar_operation(
                                        direct_arg0,
                                        direct_arg1,
                                        direct_arg2
                                    ))
                                };
                            } else {
                                return {
                                    let (direct_arg0,) = (command.spelling().semantic_token(),);
                                    finish_scan!(self.execute_illegal_prev_depth(direct_arg0))
                                };
                            }
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_BATCH_MODE
                        | codes::PRIMITIVE_NONSTOP_MODE
                        | codes::PRIMITIVE_SCROLL_MODE
                        | codes::PRIMITIVE_ERROR_STOP_MODE,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::BatchMode
                            | UnexpandablePrimitive::NonstopMode
                            | UnexpandablePrimitive::ScrollMode
                            | UnexpandablePrimitive::ErrorStopMode),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0,) = (primitive,);
                            finish_scan!(self.execute_set_interaction_mode(direct_arg0))
                        }
                    }
                    (_, codes::PRIMITIVE_INTERACTION_MODE) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (meaning, global, command_origin);
                                finish_scan!(self.scan_unary_scalar_operation(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_SPACE_FACTOR) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
                                return {
                                    let (direct_arg0, direct_arg1, direct_arg2) =
                                        (meaning, global, command_origin);
                                    finish_scan!(self.scan_unary_scalar_operation(
                                        direct_arg0,
                                        direct_arg1,
                                        direct_arg2
                                    ))
                                };
                            } else {
                                return {
                                    let (direct_arg0,) = (command.spelling().semantic_token(),);
                                    finish_scan!(self.execute_illegal_space_factor(direct_arg0))
                                };
                            }
                        }
                    }
                    (_, codes::PRIMITIVE_PREV_GRAF) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (meaning, global, command_origin);
                                finish_scan!(self.scan_unary_scalar_operation(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_CHAR) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            let origin = processor
                                .active_macro_origin()
                                .unwrap_or_else(|| command.origin());
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (meaning, global, origin);
                                finish_scan!(self.scan_unary_scalar_operation(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_ACCENT) => {
                        let (direct_arg0,) = ((processor.scan_accent().map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?,);
                        finish_scan!(self.execute_accent(direct_arg0))
                    }
                    (_, codes::PRIMITIVE_DISCRETIONARY) => {
                        return {
                            let (direct_arg0,) = ((processor
                                .scan_discretionary_opening()
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?,);
                            finish_scan!(self.execute_discretionary_opening(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_DISCRETIONARY_HYPHEN) => {
                        return {
                            let (direct_arg0,) = (processor
                                .active_macro_origin()
                                .unwrap_or_else(|| command.origin()),);
                            finish_scan!(self.execute_discretionary_hyphen(direct_arg0))
                        };
                    }
                    (
                        _,
                        codes::PRIMITIVE_H_FIL
                        | codes::PRIMITIVE_H_FILL
                        | codes::PRIMITIVE_H_SS
                        | codes::PRIMITIVE_H_FIL_NEG,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::HFil
                            | UnexpandablePrimitive::HFill
                            | UnexpandablePrimitive::HSs
                            | UnexpandablePrimitive::HFilNeg),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0,) = (primitive,);
                            finish_scan!(self.execute_fixed_horizontal_glue(direct_arg0))
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_V_SKIP
                        | codes::PRIMITIVE_V_FIL
                        | codes::PRIMITIVE_V_FILL
                        | codes::PRIMITIVE_V_SS
                        | codes::PRIMITIVE_V_FIL_NEG,
                    ) if matches!(mode, Mode::Math | Mode::DisplayMath) => {
                        (processor
                            .recover_missing_math_shift(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_missing_math_shift());
                    }
                    (
                        _,
                        codes::PRIMITIVE_V_SKIP
                        | codes::PRIMITIVE_V_FIL
                        | codes::PRIMITIVE_V_FILL
                        | codes::PRIMITIVE_V_SS
                        | codes::PRIMITIVE_V_FIL_NEG,
                    ) if mode == Mode::Horizontal => {
                        (processor
                            .recover_stop_for_vertical_mode(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_continue());
                    }
                    (
                        _,
                        codes::PRIMITIVE_V_SKIP
                        | codes::PRIMITIVE_V_FIL
                        | codes::PRIMITIVE_V_FILL
                        | codes::PRIMITIVE_V_SS
                        | codes::PRIMITIVE_V_FIL_NEG,
                    ) if mode == Mode::RestrictedHorizontal => {
                        return {
                            let (direct_arg0,) = (command,);
                            finish_scan!(self.scan_off_save(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_V_SKIP)
                        if matches!(mode, Mode::Vertical | Mode::InternalVertical) =>
                    {
                        let meaning = static_meaning(command.meaning());
                        {
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (meaning, global, command_origin);
                                finish_scan!(self.scan_unary_scalar_operation(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_V_FIL
                        | codes::PRIMITIVE_V_FILL
                        | codes::PRIMITIVE_V_SS
                        | codes::PRIMITIVE_V_FIL_NEG,
                    ) if matches!(mode, Mode::Vertical | Mode::InternalVertical) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::VFil
                            | UnexpandablePrimitive::VFill
                            | UnexpandablePrimitive::VSs
                            | UnexpandablePrimitive::VFilNeg),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            return {
                                let (direct_arg0,) = (primitive,);
                                finish_scan!(self.execute_fixed_vertical_glue(direct_arg0))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_INDENT) => {
                        return {
                            let (direct_arg0,) = (true,);
                            finish_scan!(self.execute_paragraph_indent(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_NO_INDENT) => {
                        return {
                            let (direct_arg0,) = (false,);
                            finish_scan!(self.execute_paragraph_indent(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_QUIT_V_MODE) => {
                        if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                            return finish_scan!(self.execute_paragraph_start());
                        } else {
                            return finish_scan!(self.execute_continue());
                        }
                    }
                    (_, codes::PRIMITIVE_PAR_SHAPE) => {
                        return {
                            let (direct_arg0,) = (global,);
                            finish_scan!(self.scan_paragraph_shape_assignment(direct_arg0))
                        };
                    }
                    (
                        _,
                        codes::PRIMITIVE_INTER_LINE_PENALTIES
                        | codes::PRIMITIVE_CLUB_PENALTIES
                        | codes::PRIMITIVE_WIDOW_PENALTIES
                        | codes::PRIMITIVE_DISPLAY_WIDOW_PENALTIES,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::InterLinePenalties
                            | UnexpandablePrimitive::ClubPenalties
                            | UnexpandablePrimitive::WidowPenalties
                            | UnexpandablePrimitive::DisplayWidowPenalties),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let kind = match primitive {
                                UnexpandablePrimitive::InterLinePenalties => {
                                    PenaltyArrayKind::InterLine
                                }
                                UnexpandablePrimitive::ClubPenalties => PenaltyArrayKind::Club,
                                UnexpandablePrimitive::WidowPenalties => PenaltyArrayKind::Widow,
                                UnexpandablePrimitive::DisplayWidowPenalties => {
                                    PenaltyArrayKind::DisplayWidow
                                }
                                _ => unreachable!(
                                    "outer match restricts primitive to e-TeX penalty arrays"
                                ),
                            };
                            return {
                                let (direct_arg0, direct_arg1) = (kind, global);
                                finish_scan!(
                                    self.scan_penalty_array_assignment(direct_arg0, direct_arg1)
                                )
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_TOKS) => {
                        let owner = (command.control_sequence().ok_or(ExecError::MissingToken {
                            context: "token-list assignment owner",
                        }))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        let assignment = (processor
                            .scan_token_register_assignment(owner)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return {
                            let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) = (
                                assignment.index,
                                assignment.tokens,
                                assignment.source,
                                global,
                            );
                            finish_scan!(self.execute_toks(
                                direct_arg0,
                                direct_arg1,
                                direct_arg2,
                                direct_arg3
                            ))
                        };
                    }
                    (_, codes::TOKS_REGISTER) => {
                        let ResolvedMeaning::Static(Meaning::ToksRegister(index)) =
                            command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let owner =
                                (command.control_sequence().ok_or(ExecError::MissingToken {
                                    context: "token-list assignment owner",
                                }))
                                .map_err(|error| scan_error(error, scan_origin))?;
                            let (tokens, source) = (processor
                                .scan_token_register_value(owner)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                                    (index, tokens, source, global);
                                finish_scan!(self.execute_toks(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2,
                                    direct_arg3
                                ))
                            };
                        }
                    }
                    (_, codes::INT_PARAM) => {
                        let ResolvedMeaning::Static(Meaning::IntParam(index)) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0, direct_arg1, direct_arg2) =
                                (Meaning::IntParam(index), global, command_origin);
                            finish_scan!(self.scan_unary_scalar_operation(
                                direct_arg0,
                                direct_arg1,
                                direct_arg2
                            ))
                        }
                    }
                    (_, codes::DIMEN_PARAM) => {
                        let ResolvedMeaning::Static(Meaning::DimenParam(index)) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0, direct_arg1, direct_arg2) =
                                (Meaning::DimenParam(index), global, command_origin);
                            finish_scan!(self.scan_unary_scalar_operation(
                                direct_arg0,
                                direct_arg1,
                                direct_arg2
                            ))
                        }
                    }
                    (_, codes::PAGE_DIMENSION) => {
                        let ResolvedMeaning::Static(Meaning::PageDimension(dimension)) =
                            command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0, direct_arg1, direct_arg2) =
                                (Meaning::PageDimension(dimension), global, command_origin);
                            finish_scan!(self.scan_unary_scalar_operation(
                                direct_arg0,
                                direct_arg1,
                                direct_arg2
                            ))
                        }
                    }
                    (_, codes::PAGE_INTEGER) => {
                        let ResolvedMeaning::Static(Meaning::PageInteger(integer)) =
                            command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0, direct_arg1, direct_arg2) =
                                (Meaning::PageInteger(integer), global, command_origin);
                            finish_scan!(self.scan_unary_scalar_operation(
                                direct_arg0,
                                direct_arg1,
                                direct_arg2
                            ))
                        }
                    }
                    (_, codes::TOK_PARAM) => {
                        let ResolvedMeaning::Static(Meaning::TokParam(index)) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let owner =
                                (command.control_sequence().ok_or(ExecError::MissingToken {
                                    context: "token-list assignment owner",
                                }))
                                .map_err(|error| scan_error(error, scan_origin))?;
                            let tokens = (processor
                                .scan_token_parameter_assignment(TokParam::new(index), owner)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                                    (index, tokens.tokens, tokens.source, global);
                                finish_scan!(self.execute_tok_param(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2,
                                    direct_arg3
                                ))
                            };
                        }
                    }
                    (_, codes::GLUE_PARAM) => {
                        let ResolvedMeaning::Static(Meaning::GlueParam(index)) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let assignment = (processor
                                .scan_glue_parameter_assignment(index, false)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (assignment.index, assignment.value, global);
                                finish_scan!(self.execute_glue_param(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::MU_GLUE_PARAM) => {
                        let ResolvedMeaning::Static(Meaning::MuGlueParam(index)) =
                            command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let assignment = (processor
                                .scan_glue_parameter_assignment(index, true)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (assignment.index, assignment.value, global);
                                finish_scan!(self.execute_glue_param(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_OPEN_IN
                        | codes::PRIMITIVE_CLOSE_IN
                        | codes::PRIMITIVE_READ
                        | codes::PRIMITIVE_READ_LINE,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::OpenIn
                            | UnexpandablePrimitive::CloseIn
                            | UnexpandablePrimitive::Read
                            | UnexpandablePrimitive::ReadLine),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            // §1214 fixes the effective scope before §1225 calls
                            // `read_toks`; carry that scope across the typed apply seam.
                            return {
                                let (direct_arg0, direct_arg1) = (
                                    (processor
                                        .scan_input_stream_request(primitive, global)
                                        .map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?,
                                    None,
                                );
                                finish_scan!(self.execute_input_stream(direct_arg0, direct_arg1))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_FONT) => {
                        // TeX82 §1257's `define(u,set_font,null_font)` precedes the file
                        // name scan, so like §1224's provisional `\relax` it takes the
                        // scope the eventual definition would take, `\globaldefs`
                        // included.
                        let request = (processor
                            .scan_font_definition(global)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return {
                            let (direct_arg0, direct_arg1) = (request, global);
                            finish_scan!(self.execute_font_definition(direct_arg0, direct_arg1))
                        };
                    }
                    (_, codes::PRIMITIVE_PDF_COPY_FONT | codes::PRIMITIVE_LETTERSPACE_FONT) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::PdfCopyFont
                            | UnexpandablePrimitive::LetterspaceFont),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let kind = match primitive {
                                UnexpandablePrimitive::PdfCopyFont => GeneratedFontKind::Copy,
                                UnexpandablePrimitive::LetterspaceFont => {
                                    GeneratedFontKind::Letterspace
                                }
                                _ => unreachable!(),
                            };
                            return {
                                let (direct_arg0, direct_arg1) = (
                                    (processor
                                        .scan_generated_font_definition(kind, global)
                                        .map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?,
                                    global,
                                );
                                finish_scan!(
                                    self.execute_generated_font_definition(
                                        direct_arg0,
                                        direct_arg1
                                    )
                                )
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_PDF_X_IMAGE | codes::PRIMITIVE_PDF_REF_X_IMAGE) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                                primitive @ (UnexpandablePrimitive::PdfXImage
                                | UnexpandablePrimitive::PdfRefXImage),
                            )) = command.meaning()
                            else {
                                unreachable!("packed command selected its native operand family")
                            };
                            {
                                // pdftex.web §§1551–1552 begin both image cases with
                                // `check_pdfoutput`, before version checking, image-object
                                // allocation, every rule/attr/named/page/colorspace/page-box/file
                                // scan, host image lookup, reference validation, whatsit
                                // allocation, or list mutation.
                                if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                                    let name = match primitive {
                                        UnexpandablePrimitive::PdfXImage => "pdfximage",
                                        UnexpandablePrimitive::PdfRefXImage => "pdfrefximage",
                                        _ => unreachable!(),
                                    };
                                    // pdftex.web §§1551--1552 report this before scanning any
                                    // image operand. Retain the exact command on input so the
                                    // ErrorStop retry can re-enter the complete command.
                                    (processor.back_input(command).map_err(command_error))
                                        .map_err(|error| scan_error(error, scan_origin))?;
                                    return Err(scan_error(
                                        ExecError::PdfExtensionInDviMode(name),
                                        scan_origin,
                                    ));
                                }
                                if primitive == UnexpandablePrimitive::PdfRefXImage {
                                    return {
                                        let (direct_arg0, direct_arg1, direct_arg2) =
                                            (meaning, global, command_origin);
                                        finish_scan!(self.scan_unary_scalar_operation(
                                            direct_arg0,
                                            direct_arg1,
                                            direct_arg2
                                        ))
                                    };
                                }
                                return {
                                    let (direct_arg0, direct_arg1) = (
                                        (processor.scan_pdf_image_request().map_err(command_error))
                                            .map_err(|error| scan_error(error, scan_origin))?,
                                        PdfImageResource::Unavailable,
                                    );
                                    finish_scan!(self.execute_pdf_x_image(direct_arg0, direct_arg1))
                                };
                            }
                        }
                    }
                    (_, codes::PRIMITIVE_PDF_SET_RANDOM_SEED) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) =
                                    (meaning, global, command_origin);
                                finish_scan!(self.scan_unary_scalar_operation(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_PDF_RESET_TIMER) => {
                        return finish_scan!(self.execute_pdf_reset_timer());
                    }
                    (_, codes::PRIMITIVE_PDF_INTERWORD_SPACE_ON) => {
                        return {
                            let (direct_arg0,) =
                                (tex_state::node::PdfAccessibilityControl::InterwordSpaceOn,);
                            finish_scan!(self.execute_pdf_interword_space(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PDF_INTERWORD_SPACE_OFF) => {
                        return {
                            let (direct_arg0,) =
                                (tex_state::node::PdfAccessibilityControl::InterwordSpaceOff,);
                            finish_scan!(self.execute_pdf_interword_space(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PDF_FAKE_SPACE) => {
                        return {
                            let (direct_arg0,) =
                                (tex_state::node::PdfAccessibilityControl::FakeSpace,);
                            finish_scan!(self.execute_pdf_interword_space(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PDF_RUNNING_LINK_ON) => {
                        return {
                            let (direct_arg0,) = (true,);
                            finish_scan!(self.execute_pdf_running_link(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PDF_RUNNING_LINK_OFF) => {
                        return {
                            let (direct_arg0,) = (false,);
                            finish_scan!(self.execute_pdf_running_link(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PDF_SPACE_FONT) => {
                        return {
                            let (direct_arg0,) =
                                ((processor.scan_balanced_text(true).map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?
                                    .tokens,);
                            finish_scan!(self.execute_pdf_space_font(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PDF_OBJECT) => {
                        // pdftex.web §§1535 and 1542 call `check_pdfoutput` before
                        // `reserveobjnum`, `useobjnum`, the integer, stream/attr/file
                        // options, body scan, or allocation.
                        if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                            (processor.back_input(command).map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?;
                            return Err(scan_error(
                                ExecError::PdfExtensionInDviMode("pdfobj"),
                                scan_origin,
                            ));
                        }
                        return {
                            let (direct_arg0,) =
                                ((processor.scan_pdf_object_request().map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?,);
                            finish_scan!(self.execute_pdf_object(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PDF_REFERENCE_OBJECT) => {
                        // pdftex.web §1544 calls `check_pdfoutput` before `scan_int`,
                        // object validation, whatsit allocation, or list mutation.
                        if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                            (processor.back_input(command).map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?;
                            return Err(scan_error(
                                ExecError::PdfExtensionInDviMode("pdfrefobj"),
                                scan_origin,
                            ));
                        }
                        return {
                            let (direct_arg0,) = ((processor
                                .scan_pdf_reference_object_request()
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?,);
                            finish_scan!(self.execute_pdf_reference_object(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PDF_X_FORM | codes::PRIMITIVE_PDF_REF_X_FORM) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::PdfXForm
                            | UnexpandablePrimitive::PdfRefXForm),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            // pdftex.web §§1548–1549 call `check_pdfoutput` before form-object
                            // allocation, either option scan, the box-register/integer scan,
                            // object validation, whatsit allocation, or list mutation.
                            if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                                let name = match primitive {
                                    UnexpandablePrimitive::PdfXForm => "pdfxform",
                                    UnexpandablePrimitive::PdfRefXForm => "pdfrefxform",
                                    _ => unreachable!(),
                                };
                                (processor.back_input(command).map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?;
                                return Err(scan_error(
                                    ExecError::PdfExtensionInDviMode(name),
                                    scan_origin,
                                ));
                            }
                            return {
                                let (direct_arg0,) = ((processor
                                    .scan_pdf_form_request(primitive)
                                    .map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?,);
                                finish_scan!(self.execute_pdf_form(direct_arg0))
                            };
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_PDF_INFO
                        | codes::PRIMITIVE_PDF_CATALOG
                        | codes::PRIMITIVE_PDF_NAMES
                        | codes::PRIMITIVE_PDF_TRAILER
                        | codes::PRIMITIVE_PDF_TRAILER_ID,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::PdfInfo
                            | UnexpandablePrimitive::PdfCatalog
                            | UnexpandablePrimitive::PdfNames
                            | UnexpandablePrimitive::PdfTrailer
                            | UnexpandablePrimitive::PdfTrailerId),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0,) = ((processor
                                .scan_pdf_document_fragment_request(primitive)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?,);
                            finish_scan!(self.execute_pdf_document_fragment(direct_arg0))
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_PDF_LITERAL
                        | codes::PRIMITIVE_PDF_SET_MATRIX
                        | codes::PRIMITIVE_PDF_SAVE
                        | codes::PRIMITIVE_PDF_RESTORE
                        | codes::PRIMITIVE_PDF_COLOR_STACK
                        | codes::PRIMITIVE_PDF_SAVE_POS
                        | codes::PRIMITIVE_PDF_SNAP_REF_POINT
                        | codes::PRIMITIVE_PDF_SNAP_Y
                        | codes::PRIMITIVE_PDF_SNAP_Y_COMP,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::PdfLiteral
                            | UnexpandablePrimitive::PdfSetMatrix
                            | UnexpandablePrimitive::PdfSave
                            | UnexpandablePrimitive::PdfRestore
                            | UnexpandablePrimitive::PdfColorStack
                            | UnexpandablePrimitive::PdfSavePos
                            | UnexpandablePrimitive::PdfSnapRefPoint
                            | UnexpandablePrimitive::PdfSnapY
                            | UnexpandablePrimitive::PdfSnapYComp),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            // pdftex.web §§1524 and 1563 run `check_pdfoutput` before each
                            // extension's operand scanner. `\pdfsavepos` is the sole member
                            // of this graphics family that remains available in DVI mode.
                            if primitive != UnexpandablePrimitive::PdfSavePos
                                && processor.int_param(IntParam::PDF_OUTPUT) <= 0
                            {
                                let name = match primitive {
                                    UnexpandablePrimitive::PdfLiteral => "pdfliteral",
                                    UnexpandablePrimitive::PdfSetMatrix => "pdfsetmatrix",
                                    UnexpandablePrimitive::PdfSave => "pdfsave",
                                    UnexpandablePrimitive::PdfRestore => "pdfrestore",
                                    UnexpandablePrimitive::PdfColorStack => "pdfcolorstack",
                                    UnexpandablePrimitive::PdfSnapRefPoint => "pdfsnaprefpoint",
                                    UnexpandablePrimitive::PdfSnapY => "pdfsnapy",
                                    UnexpandablePrimitive::PdfSnapYComp => "pdfsnapycomp",
                                    UnexpandablePrimitive::PdfSavePos => unreachable!(),
                                    _ => unreachable!(),
                                };
                                (processor.back_input(command).map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?;
                                return Err(scan_error(
                                    ExecError::PdfExtensionInDviMode(name),
                                    scan_origin,
                                ));
                            }
                            return {
                                let (direct_arg0,) = ((processor
                                    .scan_pdf_graphics_request(primitive)
                                    .map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?
                                .expect("graphics primitive has a typed request"),);
                                finish_scan!(self.execute_pdf_graphics(direct_arg0))
                            };
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_PDF_ANNOT
                        | codes::PRIMITIVE_PDF_START_LINK
                        | codes::PRIMITIVE_PDF_END_LINK
                        | codes::PRIMITIVE_PDF_OUTLINE
                        | codes::PRIMITIVE_PDF_DEST
                        | codes::PRIMITIVE_PDF_THREAD
                        | codes::PRIMITIVE_PDF_START_THREAD
                        | codes::PRIMITIVE_PDF_END_THREAD,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::PdfAnnot
                            | UnexpandablePrimitive::PdfStartLink
                            | UnexpandablePrimitive::PdfEndLink
                            | UnexpandablePrimitive::PdfOutline
                            | UnexpandablePrimitive::PdfDest
                            | UnexpandablePrimitive::PdfThread
                            | UnexpandablePrimitive::PdfStartThread
                            | UnexpandablePrimitive::PdfEndThread),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            if matches!(
                                primitive,
                                UnexpandablePrimitive::PdfAnnot
                                    | UnexpandablePrimitive::PdfStartLink
                                    | UnexpandablePrimitive::PdfEndLink
                                    | UnexpandablePrimitive::PdfOutline
                                    | UnexpandablePrimitive::PdfDest
                                    | UnexpandablePrimitive::PdfThread
                                    | UnexpandablePrimitive::PdfStartThread
                                    | UnexpandablePrimitive::PdfEndThread
                            ) && processor.int_param(IntParam::PDF_OUTPUT) <= 0
                            {
                                let name = match primitive {
                                    UnexpandablePrimitive::PdfAnnot => "pdfannot",
                                    UnexpandablePrimitive::PdfStartLink => "pdfstartlink",
                                    UnexpandablePrimitive::PdfEndLink => "pdfendlink",
                                    UnexpandablePrimitive::PdfOutline => "pdfoutline",
                                    UnexpandablePrimitive::PdfDest => "pdfdest",
                                    UnexpandablePrimitive::PdfThread => "pdfthread",
                                    UnexpandablePrimitive::PdfStartThread => "pdfstartthread",
                                    UnexpandablePrimitive::PdfEndThread => "pdfendthread",
                                    _ => unreachable!(),
                                };
                                (processor.back_input(command).map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?;
                                return Err(scan_error(
                                    ExecError::PdfExtensionInDviMode(name),
                                    scan_origin,
                                ));
                            }
                            // pdftex.web §1561 rejects both link boundary commands in
                            // vertical mode immediately after `check_pdfoutput`, before
                            // `\pdfstartlink` scans its rule, attributes, or action.  Keep
                            // this restriction on the scanning side of the typed-request
                            // boundary so malformed operands cannot mask the mode error.
                            if matches!(
                                primitive,
                                UnexpandablePrimitive::PdfStartLink
                                    | UnexpandablePrimitive::PdfEndLink
                            ) && matches!(mode, Mode::Vertical | Mode::InternalVertical)
                            {
                                (processor.back_input(command).map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?;
                                return Err(scan_error(
                                    ExecError::PdfLinkInVerticalMode(match primitive {
                                        UnexpandablePrimitive::PdfStartLink => "pdfstartlink",
                                        UnexpandablePrimitive::PdfEndLink => "pdfendlink",
                                        _ => unreachable!(),
                                    }),
                                    scan_origin,
                                ));
                            }
                            return {
                                let (direct_arg0,) = ((processor
                                    .scan_pdf_navigation_request(primitive)
                                    .map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?,);
                                finish_scan!(self.execute_pdf_navigation(direct_arg0))
                            };
                        }
                    }
                    (_, codes::FONT) => {
                        let ResolvedMeaning::Static(Meaning::Font(font)) = command.meaning() else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0, direct_arg1, direct_arg2) =
                                (font, command.control_sequence(), global);
                            finish_scan!(self.execute_font_select(
                                direct_arg0,
                                direct_arg1,
                                direct_arg2
                            ))
                        }
                    }
                    (_, codes::PRIMITIVE_FONT_DIMEN) => {
                        return finish_scan!(self.scan_font_dimen_assignment());
                    }
                    (_, codes::PRIMITIVE_HYPHEN_CHAR | codes::PRIMITIVE_SKEW_CHAR) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::HyphenChar
                            | UnexpandablePrimitive::SkewChar),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0,) = (primitive,);
                            finish_scan!(self.scan_font_integer_assignment(direct_arg0))
                        }
                    }
                    (_, codes::PRIMITIVE_OPEN_OUT) => {
                        return finish_scan!(self.scan_open_out_operation());
                    }
                    (_, codes::PRIMITIVE_CLOSE_OUT) => {
                        return {
                            let (direct_arg0,) =
                                ((processor.scan_write_stream().map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?,);
                            finish_scan!(self.execute_deferred_close_out(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PDF_FONT_EXPAND) => {
                        // pdftex.web §§1680--1682 configures font metrics independently
                        // of the selected output backend; generated fonts are valid in
                        // both DVI and PDF mode.
                        return finish_scan!(self.scan_pdf_font_expand_assignment());
                    }
                    (
                        _,
                        codes::PRIMITIVE_PDF_FONT_ATTR
                        | codes::PRIMITIVE_PDF_INCLUDE_CHARS
                        | codes::PRIMITIVE_PDF_MAP_FILE
                        | codes::PRIMITIVE_PDF_MAP_LINE
                        | codes::PRIMITIVE_PDF_GLYPH_TO_UNICODE
                        | codes::PRIMITIVE_PDF_NO_BUILTIN_TO_UNICODE,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::PdfFontAttr
                            | UnexpandablePrimitive::PdfIncludeChars
                            | UnexpandablePrimitive::PdfMapFile
                            | UnexpandablePrimitive::PdfMapLine
                            | UnexpandablePrimitive::PdfGlyphToUnicode
                            | UnexpandablePrimitive::PdfNoBuiltinToUnicode),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let dvi_name = match primitive {
                                UnexpandablePrimitive::PdfFontAttr => Some("pdffontattr"),
                                UnexpandablePrimitive::PdfIncludeChars => Some("pdfincludechars"),
                                UnexpandablePrimitive::PdfMapFile => Some("pdfmapfile"),
                                UnexpandablePrimitive::PdfMapLine => Some("pdfmapline"),
                                _ => None,
                            };
                            if processor.int_param(IntParam::PDF_OUTPUT) <= 0
                                && let Some(name) = dvi_name
                            {
                                (processor.back_input(command).map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?;
                                return Err(scan_error(
                                    ExecError::PdfExtensionInDviMode(name),
                                    scan_origin,
                                ));
                            }
                            let scanned = (processor
                                .scan_pdf_font_action(primitive)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                                    (primitive, scanned.font, scanned.first, scanned.second);
                                finish_scan!(self.execute_pdf_font_action(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2,
                                    direct_arg3
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_WRITE) => {
                        // TeX82 §1350's `new_write_whatsit` normalizes the stream number
                        // before storing it in `write_stream(tail)`, for the deferred
                        // whatsit exactly as for the `\immediate` one.
                        let stream = (processor.scan_write_stream().map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                        let tokens = (processor.scan_balanced_text(false).map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?
                            .tokens;
                        return {
                            let (direct_arg0, direct_arg1) = (stream, tokens);
                            finish_scan!(self.execute_deferred_write(direct_arg0, direct_arg1))
                        };
                    }
                    (_, codes::PRIMITIVE_SPECIAL) => {
                        let (deferred, text) = (processor.scan_special().map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                        return {
                            let (direct_arg0, direct_arg1) = (deferred, text.tokens);
                            finish_scan!(self.execute_deferred_special(direct_arg0, direct_arg1))
                        };
                    }
                    (_, codes::PRIMITIVE_SET_LANGUAGE) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
                                return {
                                    let (direct_arg0, direct_arg1, direct_arg2) =
                                        (meaning, global, command_origin);
                                    finish_scan!(self.scan_unary_scalar_operation(
                                        direct_arg0,
                                        direct_arg1,
                                        direct_arg2
                                    ))
                                };
                            } else {
                                return {
                                    let (direct_arg0,) = (command.spelling().semantic_token(),);
                                    finish_scan!(self.execute_illegal_set_language(direct_arg0))
                                };
                            }
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_LC_CODE
                        | codes::PRIMITIVE_UC_CODE
                        | codes::PRIMITIVE_SF_CODE
                        | codes::PRIMITIVE_MATH_CODE
                        | codes::PRIMITIVE_DEL_CODE,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::LcCode
                            | UnexpandablePrimitive::UcCode
                            | UnexpandablePrimitive::SfCode
                            | UnexpandablePrimitive::MathCode
                            | UnexpandablePrimitive::DelCode),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            // TeX82 §1230 selects the table entry with §434's
                            // `scan_char_num`, including its out-of-range recovery to
                            // character zero. The assigned value has the table-specific
                            // bound below; it is a distinct operand and must not inherit the
                            // selector's recovery.
                            return {
                                let (direct_arg0, direct_arg1) = (primitive, global);
                                finish_scan!(
                                    self.scan_code_table_assignment(direct_arg0, direct_arg1)
                                )
                            };
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_PDF_LP_CODE
                        | codes::PRIMITIVE_PDF_RP_CODE
                        | codes::PRIMITIVE_PDF_EF_CODE
                        | codes::PRIMITIVE_PDF_TAG_CODE
                        | codes::PRIMITIVE_PDF_KNBS_CODE
                        | codes::PRIMITIVE_PDF_STBS_CODE
                        | codes::PRIMITIVE_PDF_SHBS_CODE
                        | codes::PRIMITIVE_PDF_KNBC_CODE
                        | codes::PRIMITIVE_PDF_KNAC_CODE,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::PdfLpCode
                            | UnexpandablePrimitive::PdfRpCode
                            | UnexpandablePrimitive::PdfEfCode
                            | UnexpandablePrimitive::PdfTagCode
                            | UnexpandablePrimitive::PdfKnbsCode
                            | UnexpandablePrimitive::PdfStbsCode
                            | UnexpandablePrimitive::PdfShbsCode
                            | UnexpandablePrimitive::PdfKnbcCode
                            | UnexpandablePrimitive::PdfKnacCode),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0,) = (primitive,);
                            finish_scan!(self.scan_pdf_font_code_assignment(direct_arg0))
                        }
                    }
                    (_, codes::PRIMITIVE_PDF_NO_LIGATURES) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            return {
                                let (direct_arg0,) = (meaning,);
                                finish_scan!(self.scan_font_only_operation(direct_arg0))
                            };
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_ADVANCE
                        | codes::PRIMITIVE_MULTIPLY
                        | codes::PRIMITIVE_DIVIDE,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::Advance
                            | UnexpandablePrimitive::Multiply
                            | UnexpandablePrimitive::Divide),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0, direct_arg1) = (primitive, global);
                            finish_scan!(self.scan_arithmetic_assignment(direct_arg0, direct_arg1))
                        }
                    }
                    (_, codes::PRIMITIVE_CHAR_DEF | codes::PRIMITIVE_MATH_CHAR_DEF) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::CharDef
                            | UnexpandablePrimitive::MathCharDef),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            // TeX82 §1224 installs the scanner-time `\relax` through
                            // `define`, so it has the same effective scope as the eventual
                            // definition, including `\globaldefs`. This remains main-control
                            // scope policy; the command processor only receives the selected
                            // provisional scope while it owns raw operand delivery.
                            // §1224's case: `char_def_code` scans §434's `scan_char_num` and
                            // `math_char_def_code` scans §436's `scan_fifteen_bit_int`.
                            let class = match primitive {
                                UnexpandablePrimitive::CharDef => {
                                    RestrictedIntegerClass::CharacterCode
                                }
                                UnexpandablePrimitive::MathCharDef => {
                                    RestrictedIntegerClass::FifteenBit
                                }
                                _ => {
                                    unreachable!(
                                        "outer match restricts primitive to §1224's character shorthands"
                                    )
                                }
                            };
                            let definition = (processor
                                .scan_character_definition(
                                    class,
                                    global,
                                    assignment_tracing::trace_provisional_meaning_write,
                                )
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                                    (primitive, definition.target, definition.value, global);
                                finish_scan!(self.execute_character_definition(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2,
                                    direct_arg3
                                ))
                            };
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_COUNT_DEF
                        | codes::PRIMITIVE_DIMEN_DEF
                        | codes::PRIMITIVE_SKIP_DEF
                        | codes::PRIMITIVE_MUSKIP_DEF
                        | codes::PRIMITIVE_TOKS_DEF,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::CountDef
                            | UnexpandablePrimitive::DimenDef
                            | UnexpandablePrimitive::SkipDef
                            | UnexpandablePrimitive::MuskipDef
                            | UnexpandablePrimitive::ToksDef),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let definition = (processor
                                .scan_register_definition(
                                    global,
                                    assignment_tracing::trace_provisional_meaning_write,
                                )
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                                    (primitive, definition.target, definition.index, global);
                                finish_scan!(self.execute_register_definition(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2,
                                    direct_arg3
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_UPPERCASE | codes::PRIMITIVE_LOWERCASE) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::Uppercase
                            | UnexpandablePrimitive::Lowercase),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            (processor
                                .shift_case(primitive == UnexpandablePrimitive::Uppercase)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return finish_scan!(self.execute_continue());
                        }
                    }
                    (_, codes::PRIMITIVE_AFTER_GROUP) => {
                        let mut destination = None;
                        (processor
                            .get_token_into(&mut destination)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        let command = (destination.ok_or(ExecError::MissingToken {
                            context: "\\aftergroup",
                        }))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return {
                            let (direct_arg0,) = (command.spelling(),);
                            finish_scan!(self.execute_after_group(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_AFTER_ASSIGNMENT) => {
                        let mut destination = None;
                        (processor
                            .get_token_into(&mut destination)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        let command = (destination.ok_or(ExecError::MissingToken {
                            context: "\\afterassignment",
                        }))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return {
                            let (direct_arg0,) = (command.spelling(),);
                            finish_scan!(self.execute_after_assignment(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_MESSAGE | codes::PRIMITIVE_ERR_MESSAGE) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::Message
                            | UnexpandablePrimitive::ErrMessage),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let tokens =
                                (processor.scan_balanced_text(true).map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1) = (
                                    tokens.tokens,
                                    primitive == UnexpandablePrimitive::ErrMessage,
                                );
                                finish_scan!(self.execute_message(direct_arg0, direct_arg1))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_SHOW) => {
                        let (direct_arg0,) = ((processor.scan_show().map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?,);
                        finish_scan!(self.execute_display_diagnostic(direct_arg0))
                    }
                    (_, codes::PRIMITIVE_SHOW_THE) => {
                        let (direct_arg0,) = ((processor.scan_showthe().map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?,);
                        finish_scan!(self.execute_display_diagnostic(direct_arg0))
                    }
                    (_, codes::PRIMITIVE_SHOW_TOKENS) => {
                        let text = (processor.scan_showtokens().map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                        return {
                            let (direct_arg0,) = (text.tokens,);
                            finish_scan!(self.execute_show_tokens(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_SHOW_IFS) => {
                        return {
                            let (direct_arg0,) = (processor.active_conditions(),);
                            finish_scan!(self.execute_show_ifs(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_SHOW_LISTS) => {
                        return finish_scan!(self.execute_show_lists());
                    }
                    (_, codes::PRIMITIVE_SHOW_GROUPS) => {
                        return {
                            let (direct_arg0,) = (None,);
                            finish_scan!(self.execute_show_groups(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_SHOW_BOX) => {
                        let (index, _) = (processor.scan_showbox().map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                        return {
                            let (direct_arg0,) = (index,);
                            finish_scan!(self.execute_show_box(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_IMMEDIATE) => {
                        // TeX82 §§299/367/1370: `write_out` temporarily sets `mode:=0`.
                        // The `\immediate` command itself has already consumed this
                        // processor episode's main-control prefix, so install `no mode`
                        // for an expandable command inside the write text. If one is
                        // traced, §367 also leaves `shown_mode=0` after `write_out`
                        // restores the real mode.
                        let trace_count = processor.command_trace_count();
                        processor.set_command_trace_mode_prefix(Some("no mode".into()));
                        let extension = (processor
                            .scan_immediate_extension(processor.int_param(IntParam::PDF_OUTPUT) > 0)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        if matches!(extension, ImmediateExtension::PdfExtensionInDviMode(_)) {
                            // The inner PDF command is already backed up by the command
                            // scanner. Back up `\immediate` after it so the outer command
                            // is delivered first on the next retry.
                            (processor.back_input(command).map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?;
                        }
                        if processor.command_trace_count() != trace_count {
                            *shown_mode = None;
                        }
                        if let ImmediateExtension::PdfImage(request) = extension {
                            return {
                                let (direct_arg0, direct_arg1) =
                                    (request, PdfImageResource::Unavailable);
                                finish_scan!(self.execute_pdf_x_image(direct_arg0, direct_arg1))
                            };
                        } else {
                            return {
                                let (direct_arg0,) = (extension,);
                                finish_scan!(self.execute_immediate_extension(direct_arg0))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_H_RULE)
                        if matches!(mode, Mode::Math | Mode::DisplayMath) =>
                    {
                        // TeX82 §1046 lists `mmode+hrule` among the "math-only cases in
                        // non-math modes, or vice versa"; unlike `mmode+vrule` (§1056,
                        // handled below), `\hrule` never reaches `scan_rule_spec` while
                        // in math mode. §1047's `insert_dollar_sign` closes math with an
                        // inserted `$` and replays `\hrule` in the resulting mode.
                        (processor
                            .recover_missing_math_shift(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_missing_math_shift());
                    }
                    (_, codes::PRIMITIVE_H_RULE) if mode == Mode::Horizontal => {
                        // TeX82 §1095's `head_for_vmode` backs up an `\hrule`, inserts
                        // `\par`, and retries the rule in vertical mode. This must happen
                        // before §463 scans optional width/height/depth keywords: their
                        // lookahead may cross a line boundary, while §804's paragraph
                        // diagnostic must retain the line on which the rule was read.
                        (processor
                            .recover_stop_for_vertical_mode(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_continue());
                    }
                    (_, codes::PRIMITIVE_H_RULE) if mode == Mode::RestrictedHorizontal => {
                        // TeX82 §1095's negative-mode `head_for_vmode` diagnoses this
                        // command immediately. In particular, §463 must not scan a rule
                        // specification first: its keyword lookahead would replace the
                        // source-line error context with a backed-up token level.
                        return finish_scan!(self.execute_h_rule_here_except_leaders());
                    }
                    (_, codes::PRIMITIVE_H_RULE | codes::PRIMITIVE_V_RULE) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::HRule
                            | UnexpandablePrimitive::VRule),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let spec = (processor.scan_rule_spec(primitive).map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) = (
                                    spec.width,
                                    spec.height,
                                    spec.depth,
                                    primitive == UnexpandablePrimitive::HRule,
                                );
                                finish_scan!(self.execute_rule(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2,
                                    direct_arg3
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_SET_BOX) => {
                        let assignment = (processor
                            .scan_setbox_assignment(set_box_allowed)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return {
                            let (direct_arg0, direct_arg1) = (
                                SetBoxTarget {
                                    index: assignment.index,
                                    global,
                                },
                                assignment.path,
                            );
                            finish_scan!(self.execute_set_box(direct_arg0, direct_arg1))
                        };
                    }
                    (_, codes::PRIMITIVE_V_SPLIT) => {
                        let (direct_arg0,) = ((processor.scan_vsplit().map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?,);
                        finish_scan!(self.execute_v_split(direct_arg0))
                    }
                    (_, codes::PRIMITIVE_BOX | codes::PRIMITIVE_COPY) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::Box | UnexpandablePrimitive::Copy),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let register = (processor.scan_box_register().map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) = (
                                    register.index,
                                    primitive == UnexpandablePrimitive::Copy,
                                    boxes.pending_shipout.is_some(),
                                );
                                finish_scan!(self.execute_box_register(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_UN_V_BOX
                        | codes::PRIMITIVE_UN_V_COPY
                        | codes::PRIMITIVE_PAGE_DISCARDS
                        | codes::PRIMITIVE_SPLIT_DISCARDS,
                    ) if mode == Mode::Horizontal => {
                        (processor
                            .recover_stop_for_vertical_mode(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_continue());
                    }
                    (
                        _,
                        codes::PRIMITIVE_UN_V_BOX
                        | codes::PRIMITIVE_UN_V_COPY
                        | codes::PRIMITIVE_PAGE_DISCARDS
                        | codes::PRIMITIVE_SPLIT_DISCARDS,
                    ) if mode == Mode::RestrictedHorizontal => {
                        return {
                            let (direct_arg0,) = (command,);
                            finish_scan!(self.scan_off_save(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PAGE_DISCARDS | codes::PRIMITIVE_SPLIT_DISCARDS)
                        if matches!(mode, Mode::Math | Mode::DisplayMath) =>
                    {
                        (processor
                            .recover_missing_math_shift(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_missing_math_shift());
                    }
                    (_, codes::PRIMITIVE_PAGE_DISCARDS | codes::PRIMITIVE_SPLIT_DISCARDS) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::PageDiscards
                            | UnexpandablePrimitive::SplitDiscards),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0,) = (primitive,);
                            finish_scan!(self.execute_saved_vertical_discards(direct_arg0))
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_UN_H_BOX
                        | codes::PRIMITIVE_UN_H_COPY
                        | codes::PRIMITIVE_UN_V_BOX
                        | codes::PRIMITIVE_UN_V_COPY,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::UnHBox
                            | UnexpandablePrimitive::UnHCopy
                            | UnexpandablePrimitive::UnVBox
                            | UnexpandablePrimitive::UnVCopy),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let register = (processor.scan_box_register().map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0, direct_arg1, direct_arg2) = (
                                    primitive,
                                    register.index,
                                    processor.diagnostic_context_coordinate(),
                                );
                                finish_scan!(self.execute_unbox(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2
                                ))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_LAST_BOX) => {
                        return {
                            let (direct_arg0,) = (processor.diagnostic_context_coordinate(),);
                            finish_scan!(self.execute_last_box(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_RAISE | codes::PRIMITIVE_LOWER) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::Raise
                            | UnexpandablePrimitive::Lower),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                                return {
                                    let (direct_arg0,) = (command.spelling().semantic_token(),);
                                    finish_scan!(self.execute_illegal_box_shift(direct_arg0))
                                };
                            } else {
                                return {
                                    let (direct_arg0,) = ((processor
                                        .scan_box_shift(primitive)
                                        .map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?,);
                                    finish_scan!(self.execute_box_shift(direct_arg0))
                                };
                            }
                        }
                    }
                    (_, codes::PRIMITIVE_MOVE_LEFT | codes::PRIMITIVE_MOVE_RIGHT) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::MoveLeft
                            | UnexpandablePrimitive::MoveRight),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                                return {
                                    let (direct_arg0,) = ((processor
                                        .scan_box_shift(primitive)
                                        .map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?,);
                                    finish_scan!(self.execute_box_shift(direct_arg0))
                                };
                            } else {
                                return {
                                    let (direct_arg0,) = (command.spelling().semantic_token(),);
                                    finish_scan!(self.execute_illegal_box_shift(direct_arg0))
                                };
                            }
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_LEADERS
                        | codes::PRIMITIVE_C_LEADERS
                        | codes::PRIMITIVE_X_LEADERS,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::Leaders
                            | UnexpandablePrimitive::CLeaders
                            | UnexpandablePrimitive::XLeaders),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0,) = (primitive,);
                            finish_scan!(self.scan_leaders_step(direct_arg0))
                        }
                    }
                    (_, codes::PRIMITIVE_SHIPOUT) => {
                        return finish_scan!(self.execute_begin_shipout());
                    }
                    (
                        _,
                        codes::PRIMITIVE_H_BOX | codes::PRIMITIVE_V_BOX | codes::PRIMITIVE_V_TOP,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::HBox
                            | UnexpandablePrimitive::VBox
                            | UnexpandablePrimitive::VTop),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0,) = ((processor
                                .scan_box_construction(primitive)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?,);
                            finish_scan!(self.execute_begin_box(direct_arg0))
                        }
                    }
                    (_, codes::PRIMITIVE_V_CENTER)
                        if matches!(mode, Mode::Math | Mode::DisplayMath) =>
                    {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ UnexpandablePrimitive::VCenter,
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            return {
                                let (direct_arg0,) = ((processor
                                    .scan_box_construction(primitive)
                                    .map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?,);
                                finish_scan!(self.execute_begin_box(direct_arg0))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_INSERT) => {
                        return {
                            let (direct_arg0,) = ((processor
                                .scan_insert_construction(false)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?,);
                            finish_scan!(self.execute_begin_insert(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_V_ADJUST) => {
                        if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                            return {
                                let (direct_arg0,) = (command.spelling().semantic_token(),);
                                finish_scan!(self.execute_illegal_insert_or_adjust(direct_arg0))
                            };
                        } else {
                            return {
                                let (direct_arg0,) = ((processor
                                    .scan_insert_construction(true)
                                    .map_err(command_error))
                                .map_err(|error| scan_error(error, scan_origin))?,);
                                finish_scan!(self.execute_begin_insert(direct_arg0))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_MARK) => {
                        return {
                            let (direct_arg0, direct_arg1) = (
                                0,
                                (processor.scan_balanced_text(true).map_err(command_error))
                                    .map_err(|error| scan_error(error, scan_origin))?
                                    .tokens,
                            );
                            finish_scan!(self.execute_mark(direct_arg0, direct_arg1))
                        };
                    }
                    (_, codes::PRIMITIVE_MARKS) => {
                        return finish_scan!(self.scan_marks_operation());
                    }
                    (_, codes::PRIMITIVE_H_ALIGN) if mode == Mode::Horizontal => {
                        (processor
                            .recover_stop_for_vertical_mode(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_continue());
                    }
                    (_, codes::PRIMITIVE_H_ALIGN) if mode == Mode::RestrictedHorizontal => {
                        return {
                            let (direct_arg0,) = (command,);
                            finish_scan!(self.scan_off_save(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_H_ALIGN) => {
                        if mode == Mode::Math {
                            // TeX82 §1130 tests `privileged` before inspecting the
                            // current group. Inline math is negative `mmode`, so §1051
                            // ignores only the already-delivered command and leaves the
                            // following token untouched.
                            return {
                                let (direct_arg0,) = (command.spelling().semantic_token(),);
                                finish_scan!(self.execute_illegal_h_align(direct_arg0))
                            };
                        } else if mode == Mode::DisplayMath {
                            if innermost_group != Some(GroupKind::MathShift) {
                                return {
                                    let (direct_arg0,) = (command,);
                                    finish_scan!(self.scan_off_save(direct_arg0))
                                };
                            } else {
                                // TeX82 §774's `init_align` admits a display alignment at
                                // the display's own math-shift save level. The execute
                                // phase below diagnoses and flushes any preceding formula
                                // before it opens the alignment list.
                                return {
                                    let (direct_arg0, direct_arg1) =
                                        (false, command.control_sequence());
                                    finish_scan!(
                                        self.execute_begin_alignment(direct_arg0, direct_arg1)
                                    )
                                };
                            }
                        } else {
                            return {
                                let (direct_arg0, direct_arg1) =
                                    (false, command.control_sequence());
                                finish_scan!(self.execute_begin_alignment(direct_arg0, direct_arg1))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_V_ALIGN) => {
                        if matches!(mode, Mode::Math | Mode::DisplayMath)
                            && innermost_group != Some(GroupKind::MathShift)
                        {
                            return {
                                let (direct_arg0,) = (command,);
                                finish_scan!(self.scan_off_save(direct_arg0))
                            };
                        } else {
                            return {
                                let (direct_arg0, direct_arg1) = (true, command.control_sequence());
                                finish_scan!(self.execute_begin_alignment(direct_arg0, direct_arg1))
                            };
                        }
                    }
                    (_, codes::PRIMITIVE_PAR)
                        if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
                            && processor.paragraph_end_needs_alignment_recovery() =>
                    {
                        return {
                            let (direct_arg0,) = (command,);
                            finish_scan!(self.scan_off_save(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_PAR) if matches!(mode, Mode::Math | Mode::DisplayMath) => {
                        (processor
                            .recover_missing_math_shift(command)
                            .map_err(command_error))
                        .map_err(|error| scan_error(error, scan_origin))?;
                        return finish_scan!(self.execute_missing_math_shift());
                    }
                    (_, codes::PRIMITIVE_PAR) => {
                        return {
                            let (direct_arg0,) =
                                (i32::try_from(processor.current_file_line_number())
                                    .unwrap_or(i32::MAX),);
                            finish_scan!(self.execute_paragraph(direct_arg0))
                        };
                    }
                    (_, codes::CHAR_MATH_SHIFT)
                        if matches!(mode, Mode::Math | Mode::DisplayMath)
                            && innermost_group != Some(GroupKind::MathShift) =>
                    {
                        return {
                            let (direct_arg0,) = (command,);
                            finish_scan!(self.scan_off_save(direct_arg0))
                        };
                    }
                    (_, codes::CHAR_MATH_SHIFT) => match mode {
                        // §1090's shared backup already retried this exact shift after
                        // `new_graf`; probing it in vertical mode would run before
                        // `\everypar`.
                        Mode::Vertical | Mode::InternalVertical => {
                            unreachable!("§1090 backs a vertical-mode math shift up first")
                        }
                        // §1138 `init_math`: `hmode+math_shift`, for either sign of
                        // `hmode`. The probe is `get_token`, and only `mode>0` -- the
                        // unrestricted horizontal mode -- may consume the second `$`.
                        Mode::Horizontal | Mode::RestrictedHorizontal => {
                            let paired = (processor
                                .scan_init_math_display_pair(mode == Mode::Horizontal)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (direct_arg0,) = (if paired {
                                    MathShiftPairing::Paired
                                } else {
                                    MathShiftPairing::Unpaired
                                },);
                                finish_scan!(self.execute_math_shift(direct_arg0))
                            };
                        }
                        // §1194 `after_math` reaches §1197's `get_x_token` probe twice
                        // over: once for a closing display (`m>=0` with `a=null`) and
                        // once for a closing equation number (`mode=-m`).
                        Mode::DisplayMath => {
                            let (direct_arg0,) = (MathShiftPairing::ProbeDisplayEnd,);
                            finish_scan!(self.execute_math_shift(direct_arg0))
                        }
                        Mode::Math if display_eq_no => {
                            let (direct_arg0,) = (MathShiftPairing::ProbeDisplayEnd,);
                            finish_scan!(self.execute_math_shift(direct_arg0))
                        }
                        // §1194's `m<0` closes inline math through `@<Finish math in
                        // text@>`, which probes nothing at all.
                        Mode::Math => {
                            let (direct_arg0,) = (MathShiftPairing::Unpaired,);
                            finish_scan!(self.execute_math_shift(direct_arg0))
                        }
                    },
                    (_, codes::CHAR_LETTER | codes::CHAR_OTHER | codes::CHAR_SPACE) => {
                        let ResolvedMeaning::Static(Meaning::CharToken {
                            ch,
                            cat: cat @ (Catcode::Letter | Catcode::Other | Catcode::Space),
                        }) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) = (
                                ch,
                                cat,
                                processor
                                    .active_macro_origin()
                                    .unwrap_or_else(|| command.origin()),
                                false,
                            );
                            finish_scan!(self.execute_character(
                                direct_arg0,
                                direct_arg1,
                                direct_arg2,
                                direct_arg3
                            ))
                        }
                    }
                    (
                        _,
                        codes::PRIMITIVE_UN_PENALTY
                        | codes::PRIMITIVE_UN_KERN
                        | codes::PRIMITIVE_UN_SKIP,
                    ) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::UnPenalty
                            | UnexpandablePrimitive::UnKern
                            | UnexpandablePrimitive::UnSkip),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            let (direct_arg0, direct_arg1) =
                                (primitive, processor.diagnostic_context_coordinate());
                            finish_scan!(self.execute_delete_last(direct_arg0, direct_arg1))
                        }
                    }
                    (_, codes::PRIMITIVE_ITALIC_CORRECTION) => {
                        if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                            return {
                                let (direct_arg0,) = (command.spelling().semantic_token(),);
                                finish_scan!(self.execute_illegal_italic_correction(direct_arg0))
                            };
                        } else {
                            return finish_scan!(self.execute_italic_correction());
                        }
                    }
                    (_, codes::PRIMITIVE_NO_BOUNDARY) => {
                        return {
                            let (direct_arg0,) = (false,);
                            finish_scan!(self.execute_no_boundary(direct_arg0))
                        };
                    }
                    (_, codes::PRIMITIVE_NON_SCRIPT) => {
                        if matches!(mode, Mode::Math | Mode::DisplayMath) {
                            return finish_scan!(self.execute_non_script());
                        } else {
                            (processor
                                .recover_missing_math_shift(command)
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return finish_scan!(self.execute_missing_math_shift());
                        }
                    }
                    (_, codes::PRIMITIVE_PATTERNS | codes::PRIMITIVE_HYPHENATION) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            primitive @ (UnexpandablePrimitive::Patterns
                            | UnexpandablePrimitive::Hyphenation),
                        )) = command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            // TeX82 §960's `new_patterns` (`\patterns`) and §934's
                            // `new_hyph_exceptions` (`\hyphenation`) each require §403's
                            // `scan_left_brace` and then classify a `get_x_token` loop's
                            // deliveries (§961, §935) as word characters, word boundaries, or
                            // the closing brace. Neither absorbs a balanced text, so neither
                            // enters §473's `absorbing` scanner status.
                            //
                            // §1252's INITEX-only guard on `\patterns` is applied at the
                            // apply seam, not here: its production branch flushes the same
                            // braced group (`repeat get_token until cur_cmd=right_brace`)
                            // that the scan already consumes, and only the session -- not the
                            // command core -- knows which binary tex.web's `init`/`tini`
                            // split would have produced.
                            let patterns = primitive == UnexpandablePrimitive::Patterns;
                            // Captured for both seams before anything of the group is read:
                            // the two rejections §1252 can raise each report at this cursor.
                            let rejection_context = processor.error_context();
                            let trie_built = patterns && !processor.hyphenation_patterns_open();
                            if trie_built {
                                // TeX82 §960's `trie_not_ready=false` branch diagnoses and
                                // discards with `scan_toks(false,false)`. Unlike §961's
                                // pattern-word loop, §473 therefore enters `absorbing`
                                // before §403 reads the opening brace.
                                let _ =
                                    (processor.scan_balanced_text(false).map_err(command_error))
                                        .map_err(|error| scan_error(error, scan_origin))?;
                                return {
                                    let (
                                        direct_arg0,
                                        direct_arg1,
                                        direct_arg2,
                                        direct_arg3,
                                        direct_arg4,
                                    ) = (
                                        Vec::new(),
                                        Vec::new(),
                                        true,
                                        rejection_context,
                                        trie_built,
                                    );
                                    finish_scan!(self.execute_hyphenation_data(
                                        direct_arg0,
                                        direct_arg1,
                                        direct_arg2,
                                        direct_arg3,
                                        direct_arg4
                                    ))
                                };
                            }
                            let scanned = (processor
                                .scan_hyphenation_data(if patterns {
                                    HyphenationDataKind::Patterns
                                } else {
                                    HyphenationDataKind::Exceptions
                                })
                                .map_err(command_error))
                            .map_err(|error| scan_error(error, scan_origin))?;
                            return {
                                let (
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2,
                                    direct_arg3,
                                    direct_arg4,
                                ) = (
                                    scanned.words,
                                    scanned.patterns,
                                    patterns,
                                    rejection_context,
                                    trie_built,
                                );
                                finish_scan!(self.execute_hyphenation_data(
                                    direct_arg0,
                                    direct_arg1,
                                    direct_arg2,
                                    direct_arg3,
                                    direct_arg4
                                ))
                            };
                        }
                    }
                    (_, codes::UNEXPANDABLE_MIN..=codes::UNEXPANDABLE_MAX) => {
                        let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
                            command.meaning()
                        else {
                            unreachable!("packed command selected its native operand family")
                        };
                        {
                            return {
                                let (direct_arg0, direct_arg1) = (command, primitive);
                                finish_scan!(
                                    self.scan_unclassified_primitive(direct_arg0, direct_arg1)
                                )
                            };
                        }
                    }
                    (_, _) => {
                        let meaning = static_meaning(command.meaning());
                        {
                            let (direct_arg0, direct_arg1) = (command, meaning);
                            finish_scan!(self.scan_unclassified_meaning(direct_arg0, direct_arg1))
                        }
                    }
                }
            };
        }
    }
}
