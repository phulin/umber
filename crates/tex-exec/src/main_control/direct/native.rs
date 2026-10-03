//! Main control consumes ordinary terminal words inside the canonical reader.

use super::*;

// This consumer borrows the driver's actual counters. The processor retains
// the authoritative input, execution and context owners throughout consumption.
pub(super) struct NativeMainControl<'a> {
    pub(super) retain_commands: bool,
    pub(super) remaining_operations: usize,
    pub(super) completed_operations: &'a mut usize,
    pub(super) current_origin: &'a mut Option<OriginId>,
    pub(super) scan_origin: &'a mut Option<OriginId>,
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
        self.retain_commands
            && matches!(
                meaning,
                tex_state::meaning::MeaningWord::Static(Meaning::RELAX_WORD)
            )
            && *self.completed_operations + 1 < self.remaining_operations
            && !processor.has_pending_reports()
            && !processor.command_trace_printed()
            && processor.int_param(IntParam::TRACING_COMMANDS) <= 0
            && processor.int_param(IntParam::TRACING_IFS) <= 0
            && processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.boxes.pending_leader.is_none()
                    && execution.modes.current_list().pending_hchars().is_none()
            })
    }

    #[inline(always)]
    fn consume_native_terminal(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>,
        spelling: tex_state::token::TracedTokenWord,
        _: &tex_state::meaning::MeaningWord<G>,
        _: Option<tex_state::interner::Symbol>,
    ) -> std::ops::ControlFlow<()> {
        // TeX82 §1045: relax has no list effect after leaving main_loop.
        processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
        });
        *self.completed_operations += 1;
        *self.current_origin = None;
        *self.scan_origin = Some(spelling.origin());
        std::ops::ControlFlow::Continue(())
    }
}
