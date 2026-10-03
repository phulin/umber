//! Main control consumes ordinary terminal words inside the canonical reader.

use super::*;

// The consumer borrows actual driver counters. Its optional completion owns
// only a real semantic result at a reporting or failure boundary, never a word.
pub(super) struct NativeMainControl<'a> {
    pub(super) continue_native: bool,
    pub(super) remaining_operations: usize,
    pub(super) completed_operations: &'a mut usize,
    pub(super) scan_origin: &'a mut Option<OriginId>,
    pub(super) completion: Option<Result<ReplayStep, ExecError>>,
}

impl NativeMainControl<'_> {
    // Keep actual command semantics out of the per-word expansion back edge.
    // The no-op arm stays inline; this is its sibling substantive dispatcher.
    #[inline(never)]
    fn consume_substantive_terminal<'state, 'host, G>(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>,
        spelling: tex_state::token::TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        control_sequence: Option<tex_state::interner::Symbol>,
    ) -> std::ops::ControlFlow<()> {
        *self.scan_origin = Some(spelling.origin());
        let leaving_text = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
            execution.main_loop_active || execution.modes.current_list().pending_hchars().is_some()
        });
        let code = match meaning {
            tex_state::meaning::MeaningWord::Font(_) => codes::FONT,
            tex_state::meaning::MeaningWord::Static(word) => {
                Meaning::runtime_word_control_code(*word)
            }
            tex_state::meaning::MeaningWord::Macro { .. } => {
                unreachable!("native main control consumes terminals")
            }
        };
        let result = match code {
            codes::RELAX => processor.with_execution_owners(
                |context, _, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    let pending = execution.modes.current_list().pending_hchars().is_some();
                    semantic::execute_relax(
                        context,
                        pending.then_some(&mut execution.modes),
                        effects,
                        fuel,
                    )
                },
            ),
            codes::FONT => {
                let tex_state::meaning::MeaningWord::Font(font) = meaning else {
                    unreachable!("native font operand")
                };
                let global = effective_global(processor.int_param(IntParam::GLOBAL_DEFS), false);
                let result =
                    processor.with_execution_owners(|context, _, effects, _, _, _, _, _, _| {
                        semantic::execute_font_select(context, effects, *font, global)
                    });
                result.and_then(|step| {
                    processor
                        .complete_assignment()
                        .map(|()| step)
                        .map_err(command_error)
                })
            }
            codes::CHAR_BEGIN_GROUP | codes::PRIMITIVE_BEGIN_GROUP => {
                let kind = if code == codes::CHAR_BEGIN_GROUP {
                    GroupKind::Simple
                } else {
                    GroupKind::SemiSimple
                };
                processor.with_execution_owners(
                    |context, command, effects, fuel, _, execution, _, _, _| {
                        apply_groups::begin_native_group(
                            context,
                            command,
                            effects,
                            fuel,
                            &mut execution.modes,
                            kind,
                        )
                    },
                )
            }
            codes::CHAR_END_GROUP | codes::PRIMITIVE_END_GROUP => {
                let (kind, error_context) = if code == codes::CHAR_END_GROUP {
                    (GroupKind::Simple, "ordinary simple group")
                } else {
                    (GroupKind::SemiSimple, "semi simple group")
                };
                processor
                    .with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
                        if execution.modes.current_list().pending_hchars().is_some() {
                            crate::box_runtime::flush_pending_hchars_with_fuel(
                                &mut execution.modes,
                                context,
                                effects,
                                fuel,
                            )?;
                        }
                        Ok(())
                    })
                    .and_then(|()| {
                        apply_groups::finish_native_group_leave(processor, kind, error_context)
                    })
            }
            code => {
                let tex_state::meaning::MeaningWord::Static(word) = meaning else {
                    unreachable!("native assignment meaning")
                };
                native_assignments::execute_assignment(
                    processor,
                    *word,
                    code,
                    false,
                    MeaningFlags::EMPTY,
                    control_sequence,
                    *self.scan_origin,
                    &mut None,
                    None,
                )
                .and_then(|step| {
                    processor
                        .complete_assignment()
                        .map(|()| step)
                        .map_err(command_error)
                })
            }
        };
        // Unsave can restore tracing parameters and can deliver aftergroup
        // diagnostics. Re-enter main control before another word in either case.
        if leaving_text
            || *self.completed_operations + 1 >= self.remaining_operations
            || result.is_err()
            || processor.has_pending_reports()
            || (matches!(
                code,
                codes::CHAR_END_GROUP
                    | codes::PRIMITIVE_END_GROUP
                    | codes::INT_PARAM
                    | codes::PRIMITIVE_ADVANCE
                    | codes::PRIMITIVE_MULTIPLY
                    | codes::PRIMITIVE_DIVIDE
            ) && (processor.int_param(IntParam::TRACING_COMMANDS) > 0
                || processor.int_param(IntParam::TRACING_IFS) > 0))
        {
            self.completion = Some(result);
            return std::ops::ControlFlow::Break(());
        }
        *self.completed_operations += 1;
        // A consumed terminal cannot supply the origin of a later expansion failure.
        *self.scan_origin = None;
        std::ops::ControlFlow::Continue(())
    }
}

impl<'state, 'host, G>
    tex_command::MainControlConsumer<
        CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>,
        G,
    > for NativeMainControl<'_>
{
    #[inline(always)]
    fn accepts_native_terminal(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>,
        meaning: &tex_state::meaning::MeaningWord<G>,
    ) -> bool {
        if matches!(
            meaning,
            tex_state::meaning::MeaningWord::Static(Meaning::RELAX_WORD)
        ) {
            return self.continue_native && !processor.has_pending_reports();
        }
        if !self.continue_native || processor.has_pending_reports() {
            return false;
        }
        let word = match meaning {
            tex_state::meaning::MeaningWord::Font(_) => return true,
            tex_state::meaning::MeaningWord::Static(word) => *word,
            tex_state::meaning::MeaningWord::Macro { .. } => return false,
        };
        match Meaning::runtime_word_control_code(word) {
            codes::PRIMITIVE_BEGIN_GROUP => true,
            codes::CHAR_BEGIN_GROUP => {
                processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                    !matches!(
                        execution.modes.current_mode(),
                        Mode::Math | Mode::DisplayMath
                    ) && !execution.boxes.output_routine_opening_pending
                        && !execution.boxes.recovery_simple_group_pending
                })
            }
            codes::CHAR_END_GROUP => {
                processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
                    !execution.boxes.recovery_simple_group_open
                        && context.innermost_group_kind() == Some(GroupKind::Simple)
                })
            }
            codes::PRIMITIVE_END_GROUP => {
                processor.with_execution_owners(|context, _, _, _, _, _, _, _, _| {
                    context.innermost_group_kind() == Some(GroupKind::SemiSimple)
                })
            }
            code => native_assignments::accepts_assignment(code),
        }
    }

    #[inline(always)]
    fn consume_native_terminal(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>,
        spelling: tex_state::token::TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        control_sequence: Option<tex_state::interner::Symbol>,
    ) -> std::ops::ControlFlow<()> {
        if matches!(
            meaning,
            tex_state::meaning::MeaningWord::Static(Meaning::RELAX_WORD)
        ) {
            if processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.main_loop_active
                    || execution.modes.current_list().pending_hchars().is_some()
            }) {
                return self.consume_substantive_terminal(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                );
            }
            // TeX82 §1045: relax has no list effect after leaving main_loop.
            if *self.completed_operations + 1 >= self.remaining_operations {
                self.completion = Some(Ok(ReplayStep::Continue));
                return std::ops::ControlFlow::Break(());
            }
            *self.completed_operations += 1;
            return std::ops::ControlFlow::Continue(());
        }
        self.consume_substantive_terminal(processor, spelling, meaning, control_sequence)
    }
}
