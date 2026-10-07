//! Main control's actual synchronous terminal consumer and operation count.
use super::*;

struct InterpreterTerminals<'a, const LOOKAHEAD: bool> {
    remaining_operations: usize,
    completed: usize,
    observation_receipts: &'a mut Option<Vec<MutationRecord>>,
    completion: &'a mut Option<Result<MainControlStep, ExecError>>,
    observing: bool,
    etex_extended: bool,
    initial_effect_pos: tex_state::EffectPos,
    resident_clean: bool,
}

/// Select TeX's fetch label once; the sole reader consumes the existing budget.
#[inline(always)]
pub(super) fn read<G, const LOOKAHEAD: bool>(
    processor: &mut Processor<'_, '_, G>,
    remaining_operations: usize,
    completed: &mut usize,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
    completion: &mut Option<Result<MainControlStep, ExecError>>,
    initial_effect_pos: tex_state::EffectPos,
    observing: bool,
) -> Result<tex_command::DeliveryStatus, tex_command::CommandError> {
    let mut consume = InterpreterTerminals::<LOOKAHEAD> {
        remaining_operations,
        completed: *completed,
        observation_receipts,
        completion,
        observing,
        etex_extended: processor.profile() == CommandProfile::ETEX26,
        initial_effect_pos,
        resident_clean: LOOKAHEAD && !observing && !processor.has_pending_reports(),
    };
    let result = if LOOKAHEAD {
        processor.consume_main_loop_lookahead(&mut consume)
    } else {
        processor.consume_main_control(&mut consume)
    };
    *completed = consume.completed;
    result
}

impl<'state, 'host, G, const LOOKAHEAD: bool>
    tex_command::NativeWordConsumer<Processor<'state, 'host, G>, G, &'state mut ExecutionState<G>>
    for InterpreterTerminals<'_, LOOKAHEAD>
{
    const RESIDENT_TERMINALS: bool = true;
    const RESIDENT_LITERALS: bool = true;

    #[inline(always)]
    fn resident_ready(&self) -> bool {
        self.resident_clean
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)] // Disjoint input-preserving execution owners and spelling.
    fn resident_terminal(
        &mut self,
        context: &mut tex_state::CommandContext<'_, G>,
        fuel: &mut tex_command::CommandFuel,
        effects: &mut tex_state::diagnostic::DiagnosticEffects,
        execution: &mut &'state mut ExecutionState<G>,
        entered_line: u32,
        macro_origin: Option<tex_state::token::OriginId>,
        spelling: TracedTokenWord,
        code: u64,
        operand: u64,
    ) -> ControlFlow<(), bool> {
        if LOOKAHEAD {
            if !matches!(
                code,
                codes::CHAR_LETTER | codes::CHAR_OTHER | codes::CHAR_GIVEN
            ) {
                return ControlFlow::Continue(false);
            }
            let ch = char::from_u32(operand as u32).expect("admitted native character");
            if let Err(error) = characters::append_native_character(
                context,
                effects,
                fuel,
                execution,
                ch,
                macro_origin.unwrap_or(spelling.origin()),
                self.etex_extended,
            ) {
                *self.completion = Some(Err(error.capture_command_origin(spelling.origin())));
                return ControlFlow::Break(());
            }
            self.completed += 1;
            if context.effect_pos() != self.initial_effect_pos || effects.has_first_recoverable() {
                *self.completion = Some(Ok(MainControlStep::Continue));
                return ControlFlow::Break(());
            }
            return if execution.main_loop_active && self.completed < self.remaining_operations {
                ControlFlow::Continue(true)
            } else {
                ControlFlow::Break(())
            };
        }
        if code != codes::RELAX && !resident_group(context, execution, entered_line, code) {
            return ControlFlow::Continue(false);
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
        let traced = match dispatch::enter(
            processor,
            spelling,
            meaning,
            control_sequence,
            code,
            |processor| {
                code == codes::RELAX
                    && processor.has_diagnostic_boundary_since(self.initial_effect_pos)
            },
        ) {
            Ok(ControlFlow::Continue(traced)) => traced,
            Ok(ControlFlow::Break(())) => {
                self.completed += 1;
                *self.completion = Some(Ok(MainControlStep::Continue));
                return Ok(ControlFlow::Break(()));
            }
            Err(error) => {
                *self.completion = Some(Err(error));
                return Ok(ControlFlow::Break(()));
            }
        };
        if code == codes::RELAX {
            self.completed += 1;
            if !observing && !LOOKAHEAD && self.completed < self.remaining_operations {
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
                        || execution.page_region_succession_ready()
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
        if !observing && next_lookahead == LOOKAHEAD && self.completed < self.remaining_operations {
            self.resident_clean = !processor.has_pending_reports()
                && (next_lookahead
                    || (processor.int_param(IntParam::TRACING_COMMANDS) <= 0
                        && processor
                            .host_facts()
                            .modes
                            .current_list()
                            .pending_hchars()
                            .is_none()));
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
    context: &mut tex_state::CommandContext<'_, G>,
    execution: &mut ExecutionState<G>,
    entered_line: u32,
    code: u64,
) -> bool {
    let (kind, opening) = match code {
        codes::PRIMITIVE_BEGIN_GROUP => (GroupKind::SemiSimple, true),
        codes::PRIMITIVE_END_GROUP => (GroupKind::SemiSimple, false),
        codes::CHAR_BEGIN_GROUP => (GroupKind::Simple, true),
        codes::CHAR_END_GROUP => (GroupKind::Simple, false),
        _ => return false,
    };
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
