//! Main control's actual synchronous terminal consumer and operation count.
use super::*;

pub(super) struct InterpreterTerminals<'a> {
    pub(super) remaining_operations: usize,
    pub(super) completed: &'a mut usize,
    pub(super) observation_receipts: &'a mut Option<Vec<MutationRecord>>,
    pub(super) completion: &'a mut Option<Result<MainControlStep, ExecError>>,
    pub(super) observing: bool,
    pub(super) lookahead: bool,
    pub(super) initial_effect_pos: tex_state::EffectPos,
    pub(super) resident_clean: bool,
}

impl<'state, 'host, G> tex_command::NativeWordConsumer<Processor<'state, 'host, G>, G>
    for InterpreterTerminals<'_>
{
    const RESIDENT_TERMINALS: bool = true;

    #[inline(always)]
    fn resident_ready(&self) -> bool {
        self.resident_clean
    }

    #[inline(always)]
    fn resident_terminal(
        &mut self,
        _spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
    ) -> ControlFlow<(), bool> {
        if !meaning.is_relax() {
            self.resident_clean = false;
            return ControlFlow::Continue(false);
        }
        *self.completed += 1;
        if *self.completed == self.remaining_operations {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(true)
        }
    }

    #[inline(always)]
    fn input_boundary(&mut self) {
        self.resident_clean = false;
    }

    #[inline(always)]
    fn consume(
        &mut self,
        processor: &mut Processor<'state, 'host, G>,
        spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        code: u64,
        control_sequence: Option<Symbol>,
        exceptional: Option<&tex_command::CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, tex_command::CommandError> {
        // Native delivery never lends an observation record.
        // Fold that actual ownership fact into its continuation.
        let observing = exceptional.is_some() && self.observing;
        let traced = match dispatch::enter(processor, spelling, meaning, control_sequence, code) {
            Ok(traced) => traced,
            Err(error) => {
                *self.completion = Some(Err(error));
                return Ok(ControlFlow::Break(()));
            }
        };
        if code == codes::RELAX {
            *self.completed += 1;
            if processor.has_diagnostic_boundary_since(self.initial_effect_pos) {
                *self.completion = Some(Ok(MainControlStep::Continue));
                return Ok(ControlFlow::Break(()));
            }
            if !observing && !self.lookahead && *self.completed < self.remaining_operations {
                self.resident_clean = !traced;
                return Ok(ControlFlow::Continue(()));
            }
            return Ok(ControlFlow::Break(()));
        }
        match dispatch::reswitch(
            processor,
            spelling,
            meaning,
            control_sequence,
            exceptional,
            self.observation_receipts,
            0,
            code,
        ) {
            Ok(MainControlStep::Continue) => {}
            result => {
                if result.is_ok() {
                    *self.completed += 1;
                }
                *self.completion = Some(result);
                return Ok(ControlFlow::Break(()));
            }
        }
        *self.completed += 1;
        let (mut next_lookahead, host_boundary) =
            processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
                let diagnostic_boundary = context.effect_pos() != self.initial_effect_pos
                    || effects.has_first_recoverable();
                (
                    execution.main_loop_active,
                    diagnostic_boundary
                        || execution.paragraph_checkpoint_cut
                        || execution.prepared_shipout.is_some()
                        || execution.prepared_pdf_form.is_some()
                        || (execution.page_region_succession_pending
                            && !execution.modes.retains_page_node_handles())
                        || (!execution.boxes.output_routine_active
                            && (context.page_fire_up().is_some()
                                || context.page_builder_resume_after_output_pending())),
                )
            });
        if host_boundary {
            *self.completion = Some(Ok(MainControlStep::Continue));
            return Ok(ControlFlow::Break(()));
        }
        if next_lookahead && !observing {
            let mut characters = characters::MainSourceAdmission {
                etex_extended: processor.profile() == CommandProfile::ETEX26,
                error: None,
            };
            let appended = match processor.consume_main_source_characters(
                self.remaining_operations - *self.completed,
                &mut characters,
            ) {
                Ok(appended) => appended,
                Err(error) => {
                    *self.completion = Some(Err(command_error(error)));
                    return Ok(ControlFlow::Break(()));
                }
            };
            *self.completed += appended as usize;
            if appended != 0 {
                next_lookahead = processor.host_facts().main_loop_active;
            }
            if let Some(error) = characters.error {
                *self.completion = Some(Err(error));
                return Ok(ControlFlow::Break(()));
            }
        }
        if !observing
            && next_lookahead == self.lookahead
            && *self.completed < self.remaining_operations
        {
            self.resident_clean = false;
            return Ok(ControlFlow::Continue(()));
        }
        Ok(ControlFlow::Break(()))
    }
}
