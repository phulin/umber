//! Main control's actual synchronous terminal consumer and operation count.
use super::*;

pub(super) struct InterpreterTerminals<'a> {
    pub(super) remaining_operations: usize,
    pub(super) completed: usize,
    pub(super) observation_receipts: &'a mut Option<Vec<MutationRecord>>,
    pub(super) completion: &'a mut Option<Result<MainControlStep, ExecError>>,
    pub(super) observing: bool,
    pub(super) lookahead: bool,
    pub(super) initial_effect_pos: tex_state::EffectPos,
    pub(super) resident_clean: bool,
}

impl<'state, 'host, G>
    tex_command::NativeWordConsumer<Processor<'state, 'host, G>, G, &'state mut ExecutionState<G>>
    for InterpreterTerminals<'_>
{
    const RESIDENT_TERMINALS: bool = true;
    const RESIDENT_LITERALS: bool = true;

    #[inline(always)]
    fn resident_ready(&self) -> bool {
        self.resident_clean
    }

    #[inline(always)]
    fn resident_terminal(
        &mut self,
        context: &mut tex_state::ResidentGroups<'_, G>,
        execution: &mut &'state mut ExecutionState<G>,
        entered_line: u32,
        _spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
    ) -> ControlFlow<(), bool> {
        if !meaning.is_relax() {
            const BEGIN: u64 =
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BeginGroup).encode();
            const END: u64 =
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup).encode();
            let group = match meaning.scalar_word() {
                Some(BEGIN) => Some((GroupKind::SemiSimple, true)),
                Some(END) => Some((GroupKind::SemiSimple, false)),
                _ => match meaning.character() {
                    Some((_, Catcode::BeginGroup)) => Some((GroupKind::Simple, true)),
                    Some((_, Catcode::EndGroup)) => Some((GroupKind::Simple, false)),
                    _ => None,
                },
            };
            if !group.is_some_and(|(kind, opening)| {
                resident_group(context, execution, entered_line, kind, opening)
            }) {
                self.resident_clean = false;
                return ControlFlow::Continue(false);
            }
        }
        self.completed += 1;
        if self.completed == self.remaining_operations {
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
            self.completed += 1;
            if processor.has_diagnostic_boundary_since(self.initial_effect_pos) {
                *self.completion = Some(Ok(MainControlStep::Continue));
                return Ok(ControlFlow::Break(()));
            }
            if !observing && !self.lookahead && self.completed < self.remaining_operations {
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
                    self.completed += 1;
                }
                *self.completion = Some(result);
                return Ok(ControlFlow::Break(()));
            }
        }
        self.completed += 1;
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
                self.remaining_operations - self.completed,
                &mut characters,
            ) {
                Ok(appended) => appended,
                Err(error) => {
                    *self.completion = Some(Err(command_error(error)));
                    return Ok(ControlFlow::Break(()));
                }
            };
            self.completed += appended as usize;
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
            && self.completed < self.remaining_operations
        {
            self.resident_clean = !next_lookahead
                && !processor.has_pending_reports()
                && processor.int_param(IntParam::TRACING_COMMANDS) <= 0
                && processor
                    .host_facts()
                    .modes
                    .current_list()
                    .pending_hchars()
                    .is_none();
            return Ok(ControlFlow::Continue(()));
        }
        Ok(ControlFlow::Break(()))
    }
}

/// These operations use only owners disjoint from the retained input span.
/// Every decline happens before state mutation; full interpretation then owns
/// tracing, math/recovery semantics, actual restores and aftergroup insertion.
#[inline(never)]
fn resident_group<G>(
    context: &mut tex_state::ResidentGroups<'_, G>,
    execution: &mut ExecutionState<G>,
    entered_line: u32,
    kind: GroupKind,
    opening: bool,
) -> bool {
    if opening
        && kind == GroupKind::Simple
        && (matches!(
            execution.modes.current_mode(),
            Mode::Math | Mode::DisplayMath
        ) || execution.boxes.output_routine_opening_pending
            || execution.boxes.recovery_simple_group_pending)
    {
        return false;
    }
    if context.int_param(IntParam::TRACING_GROUPS) > 0 {
        return false;
    }
    if opening {
        context
            .begin_group(kind, entered_line)
            .expect("native TeX group boundary opens");
        true
    } else {
        if context.int_param(IntParam::TRACING_NESTING) > 0 || !context.end_empty_group(kind) {
            return false;
        }
        execution.boxes.recovery_simple_group_open = false;
        true
    }
}
