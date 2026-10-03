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
            return self.continue_native
                && *self.completed_operations + 1 < self.remaining_operations
                && !processor.has_pending_reports();
        }
        if !self.continue_native
            || *self.completed_operations + 1 >= self.remaining_operations
            || processor.has_pending_reports()
        {
            return false;
        }
        let tex_state::meaning::MeaningWord::Static(word) = meaning else {
            return false;
        };
        match Meaning::runtime_word_control_code(*word) {
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
            _ => false,
        }
    }

    #[inline(always)]
    fn consume_native_terminal(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>,
        spelling: tex_state::token::TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        _: Option<tex_state::interner::Symbol>,
    ) -> std::ops::ControlFlow<()> {
        *self.scan_origin = Some(spelling.origin());
        let tex_state::meaning::MeaningWord::Static(word) = meaning else {
            unreachable!("native main-control selection accepts static terminals")
        };
        if *word == Meaning::RELAX_WORD {
            // TeX82 §1045: relax has no list effect after leaving main_loop.
            *self.completed_operations += 1;
            return std::ops::ControlFlow::Continue(());
        }
        let code = Meaning::runtime_word_control_code(*word);
        let result = match code {
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
            codes::CHAR_END_GROUP => apply_groups::finish_native_group_leave(
                processor,
                GroupKind::Simple,
                "ordinary simple group",
            ),
            codes::PRIMITIVE_END_GROUP => apply_groups::finish_native_group_leave(
                processor,
                GroupKind::SemiSimple,
                "semi simple group",
            ),
            _ => unreachable!("native main-control selection accepts group terminals"),
        };
        // Unsave can restore tracing parameters and can deliver aftergroup
        // diagnostics. Re-enter main control before another word in either case.
        if result.is_err()
            || processor.has_pending_reports()
            || (matches!(code, codes::CHAR_END_GROUP | codes::PRIMITIVE_END_GROUP)
                && (processor.int_param(IntParam::TRACING_COMMANDS) > 0
                    || processor.int_param(IntParam::TRACING_IFS) > 0))
        {
            self.completion = Some(result);
            return std::ops::ControlFlow::Break(());
        }
        *self.completed_operations += 1;
        std::ops::ControlFlow::Continue(())
    }
}
