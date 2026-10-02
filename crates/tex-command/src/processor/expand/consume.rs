//! Consumer-side interpretation of the shared input reader.

use super::{
    ExpandedCommandAction, ExpandedReaderRole, ExpansionDispatch, ReadSite, classify_hot_command,
};
use crate::command::{CommandWord, CurrentCommand};
use crate::input::InputLevel;
#[cfg(feature = "profiling")]
use crate::input::ResidentTokenStorage;
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use std::ops::ControlFlow;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::token::PackedMeaningResolution;

/// How TeX82 §494's `pass_text` treats one word of skipped text.
#[derive(Clone, Copy)]
enum SkippedWord {
    /// Discarded with no effect beyond its fuel charge.
    Discarded,
    /// A literal brace, which still moves §347's `align_state`.
    Brace(i32),
    /// An `if_test` command, which nests the skip.
    NestedConditional,
    /// `\fi` closing a nested conditional.
    NestedFi,
    /// A word that needs ordinary delivery settlement.
    Boundary,
}

/// One read from the shared raw input kernel, interpreted for expansion.
/// The canonical expansion loop activates ordinary unobserved macros without
/// constructing a command.
///
/// A settled command is written straight into the caller's slot; returning it
/// by value through this status would copy the whole compact command at
/// every nested result boundary of the per-token loop.
pub(super) enum ExpansionCandidate {
    ExpandedMacro,
    Command,
    Finished(DeliveryStatus),
}

impl<G> CommandProcessor<'_, '_, G> {
    /// TeX82 §494 admits skipped words directly from the selected native
    /// span. Its first exceptional word goes straight to raw settlement.
    pub(crate) fn get_next_skipping_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        nested_conditions: &mut u32,
    ) -> Result<DeliveryStatus, CommandError> {
        if self.is_observed()
            || self.command.delivery_mode.tracing()
            || self.command.delivery_mode.requires_persistent_settlement()
        {
            return self.get_next_into(destination);
        }
        let result = (|| {
            loop {
                let Some(InputLevel::Resident(row)) = self.command.roots.input.levels.rows.last()
                else {
                    return self.get_next_into(destination);
                };
                if self.fuel.remaining() == 0 {
                    return self.get_next_into(destination);
                }
                let classify = !row.header.frame.flags().contains(
                    tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
                );
                #[cfg(feature = "profiling")]
                let kind = if matches!(row.storage, ResidentTokenStorage::MacroArgument { .. }) {
                    crate::fuel::RawDeliveryKind::MacroArgument
                } else {
                    crate::fuel::RawDeliveryKind::StoredToken
                };
                #[cfg(feature = "profiling")]
                let scanner = self.command.delivery_mode.scanner_active();
                self.pending_diagnostic_location = None;
                let selected = self.consume_resident_input(
                    &mut |state, fuel, _, alignment, word, _| {
                        let effect = match word.literal_catcode() {
                            Some(tex_state::token::Catcode::BeginGroup) => SkippedWord::Brace(1),
                            Some(tex_state::token::Catcode::EndGroup) => SkippedWord::Brace(-1),
                            Some(tex_state::token::Catcode::Active) if classify => {
                                Self::skipped_command(state, word, *nested_conditions)
                            }
                            Some(tex_state::token::Catcode::Active) => SkippedWord::Boundary,
                            Some(_) => SkippedWord::Discarded,
                            None if classify && word.is_control_sequence() => {
                                Self::skipped_command(state, word, *nested_conditions)
                            }
                            None => SkippedWord::Boundary,
                        };
                        if matches!(effect, SkippedWord::Boundary) || !fuel.try_charge() {
                            return ControlFlow::Continue(false);
                        }
                        match effect {
                            SkippedWord::Brace(delta) => alignment.align_state += delta,
                            SkippedWord::NestedConditional => *nested_conditions += 1,
                            SkippedWord::NestedFi => *nested_conditions -= 1,
                            SkippedWord::Discarded | SkippedWord::Boundary => {}
                        }
                        #[cfg(feature = "profiling")]
                        fuel.record_raw_run(scanner, kind, 1);
                        if fuel.remaining() == 0 {
                            ControlFlow::Break(None)
                        } else {
                            ControlFlow::Continue(true)
                        }
                    },
                    &mut |processor, word, origin, suppress, site, kind| {
                        processor.charge_command_action()?;
                        processor
                            .finish_delivered_word::<false>(
                                word,
                                origin,
                                suppress,
                                site,
                                kind,
                                destination,
                            )
                            .map(Some)
                    },
                )?;
                let status = match selected {
                    Some(status) => status,
                    None => match self.advance_input_boundary(
                        self.create_source_control_sequences,
                        &mut |processor, word, origin, suppress, site, kind| {
                            processor.charge_command_action()?;
                            processor
                                .finish_delivered_word::<false>(
                                    word,
                                    origin,
                                    suppress,
                                    site,
                                    kind,
                                    destination,
                                )
                                .map(Some)
                        },
                    )? {
                        None => continue,
                        Some(ControlFlow::Continue(status)) => status,
                        Some(ControlFlow::Break(status)) => {
                            destination.take();
                            return Ok(status);
                        }
                    },
                };
                let Some(status) = status else {
                    return self.get_next_into(destination);
                };
                if let Some(status) = self.settle_raw_step(status, destination)? {
                    return Ok(status);
                }
            }
        })();
        match result {
            Ok(status) => Ok(status),
            Err(failure) => self.fail_hot_expanded_delivery(
                destination,
                self.command.transient.active_expansion_depth,
                failure,
            ),
        }
    }

    /// TeX82 §494 `pass_text` inspects only `cur_cmd`: a conditional
    /// (`if_test`) nests the skip, a delimiter (`fi_or_else`) ends it or
    /// closes a nested conditional, and §336's `check_outer_validity` owns
    /// an outer macro or `\endtemplate`. Every other skipped command is
    /// discarded without settlement, so its word needs one dense meaning
    /// lookup and no delivery record.
    #[inline(always)]
    fn skipped_command(
        state: &tex_state::CommandContext<'_, G>,
        word: tex_state::token::TokenWord,
        nested_conditions: u32,
    ) -> SkippedWord {
        let mut meaning = CommandWord::from_static_word(Meaning::UNDEFINED_WORD);
        state.write_packed_token_command_into(word, &mut meaning);
        if meaning.is_outer() {
            return SkippedWord::Boundary;
        }
        let CommandWord::Static { word, .. } = meaning else {
            return SkippedWord::Discarded;
        };
        if !matches!(
            Meaning::runtime_word_class(word),
            tex_state::meaning::StaticCommandClass::Expandable
        ) {
            return SkippedWord::Discarded;
        }
        let Some(primitive) = tex_state::meaning::ExpandablePrimitive::from_operand(
            Meaning::runtime_word_operand(word),
        ) else {
            return SkippedWord::Discarded;
        };
        match primitive {
            _ if crate::conditionals::ConditionalKind::from_primitive(primitive).is_some() => {
                SkippedWord::NestedConditional
            }
            tex_state::meaning::ExpandablePrimitive::Fi if nested_conditions != 0 => {
                SkippedWord::NestedFi
            }
            tex_state::meaning::ExpandablePrimitive::Else
            | tex_state::meaning::ExpandablePrimitive::Or
                if nested_conditions != 0 =>
            {
                SkippedWord::Discarded
            }
            tex_state::meaning::ExpandablePrimitive::Fi
            | tex_state::meaning::ExpandablePrimitive::Else
            | tex_state::meaning::ExpandablePrimitive::Or => SkippedWord::Boundary,
            _ => SkippedWord::Discarded,
        }
    }

    #[inline(always)]
    pub(super) fn read_expansion_candidate<const OBSERVED: bool, const STOP_PROTECTED: bool>(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<ExpansionCandidate, CommandError> {
        self.consume_input(
            self.create_source_control_sequences,
            &mut |processor, word, origin, suppress, site, kind| {
                let (meaning, resolution) = processor.resolve_token_word(word);
                processor.record_consumed_word(kind, resolution.meaning_lookup());
                if !OBSERVED
                    && !processor
                        .command
                        .delivery_mode
                        .requires_semantic_settlement(suppress, meaning.is_outer())
                    && let CommandWord::Macro { flags, definition } = meaning
                    && !(STOP_PROTECTED && flags.contains(MeaningFlags::PROTECTED))
                {
                    let name = resolution
                        .control_sequence()
                        .ok_or_else(CommandError::input_invariant)?;
                    processor.activate_read_macro(word, origin, flags, definition, name)?;
                    return Ok(ExpansionCandidate::ExpandedMacro);
                }
                let line = match site {
                    ReadSite::Source(line) => line,
                    _ => None,
                };
                let command = destination.insert(CurrentCommand::from_delivered_parts(
                    word,
                    origin,
                    matches!(site, ReadSite::Source(_)),
                    line,
                    suppress,
                    meaning,
                    resolution.control_sequence(),
                ));
                processor
                    .settle_hot_delivery_in::<OBSERVED>(command, resolution.literal_catcode())?;
                Ok(ExpansionCandidate::Command)
            },
            ExpansionCandidate::Finished,
        )
    }

    #[inline(always)]
    pub(super) fn resolve_token_word(
        &self,
        word: tex_state::token::TokenWord,
    ) -> (CommandWord<G>, PackedMeaningResolution) {
        let mut meaning = CommandWord::from_static_word(Meaning::UNDEFINED_WORD);
        let resolution = self
            .state
            .write_packed_token_command_into(word, &mut meaning);
        (meaning, resolution)
    }

    #[inline(always)]
    pub(super) fn record_consumed_word(
        &mut self,
        kind: crate::fuel::RawDeliveryKind,
        lookup: bool,
    ) {
        #[cfg(feature = "profiling")]
        self.fuel
            .record_raw_delivery(self.command.delivery_mode.scanner_active(), lookup, kind);
        #[cfg(not(feature = "profiling"))]
        let _ = (kind, lookup);
    }

    /// The ordinary expansion consumer owns one read/interpret back edge.
    /// A supplied command is a distinct entry boundary, never an optional-slot
    /// choice repeated for each newly read word.
    pub(super) fn expanded_delivery_loop<
        const OBSERVED: bool,
        const PRESERVE_UNDEFINED: bool,
        const STOP_PROTECTED: bool,
    >(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
        role: ExpandedReaderRole,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut expanded = false;
        if STOP_PROTECTED
            && destination.as_ref().is_some_and(|command| {
                Self::protected_terminal(command, classify_hot_command(command))
            })
        {
            return Ok(DeliveryStatus::Command);
        }
        if destination.is_some() {
            if let Some(status) =
                self.consume_supplied_command::<OBSERVED>(destination, initial_action)?
            {
                return Ok(status);
            }
            expanded = true;
        }
        loop {
            match self.read_expansion_candidate::<OBSERVED, STOP_PROTECTED>(destination)? {
                ExpansionCandidate::ExpandedMacro => {
                    expanded = true;
                    continue;
                }
                ExpansionCandidate::Finished(status) => {
                    destination.take();
                    return Ok(status);
                }
                ExpansionCandidate::Command => {}
            }
            let command = destination
                .as_mut()
                .expect("command candidate initializes destination");
            let action = classify_hot_command(command);
            if STOP_PROTECTED && Self::protected_terminal(command, action) {
                return Ok(DeliveryStatus::Command);
            }
            if PRESERVE_UNDEFINED
                && matches!(
                    action,
                    ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
                )
            {
                return Ok(self.finish_terminal_expansion::<OBSERVED>(command, action, expanded));
            }
            if let ExpandedCommandAction::Expand(dispatch) = action {
                self.execute_expansion_action(command, dispatch)?;
                expanded = true;
                continue;
            }
            if role == ExpandedReaderRole::MainControl
                && !expanded
                && matches!(action, ExpandedCommandAction::EndTemplate)
                && !matches!(
                    command.alignment_adjustment(),
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                )
            {
                destination.take();
                self.insert_frozen_endv()?;
                continue;
            }
            return Ok(self.finish_terminal_expansion::<OBSERVED>(command, action, expanded));
        }
    }

    /// e-TeX's `get_x_or_protected` procedure stops on the first
    /// unexpandable command or protected macro after every expansion restart.
    /// The command has already crossed its raw boundary; returning it here
    /// deliberately skips TeX82's terminal expanded observation.
    #[inline(always)]
    fn protected_terminal(command: &CurrentCommand<G>, action: ExpandedCommandAction) -> bool {
        matches!(action, ExpandedCommandAction::Return)
            || matches!(
                action,
                ExpandedCommandAction::Expand(ExpansionDispatch::Macro)
            ) && command
                .command_word()
                .flags()
                .contains(MeaningFlags::PROTECTED)
    }

    /// Matching's local storage belongs to macro activation, never to a
    /// recursively suspended primitive-expansion frame. Only spelling and
    /// invocation facts survive matching; diagnostics need no delivery geometry
    /// or command record, even on prefix failure.
    #[inline(never)]
    fn activate_read_macro(
        &mut self,
        word: tex_state::token::TokenWord,
        origin: tex_state::token::OriginId,
        flags: MeaningFlags,
        definition: tex_state::DefinitionRef<G>,
        name: Symbol,
    ) -> Result<(), CommandError> {
        self.record_macro_expansion();
        match self.macro_call_parts(flags, definition, name, origin, |processor| {
            processor.report_unobserved_macro_prefix_mismatch(
                tex_state::token::TracedTokenWord::from_parts(word, origin),
                flags,
                definition,
                name,
            );
        }) {
            Ok(_)
            | Err(CommandError::ParagraphInMacroArgument | CommandError::OuterInMacroArgument) => {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Existing commands have already crossed a delivery boundary. Keep their
    /// temporary owner out of the ordinary reader's recursive stack frame.
    #[inline(never)]
    fn consume_supplied_command<const OBSERVED: bool>(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        action: Option<ExpandedCommandAction>,
    ) -> Result<Option<DeliveryStatus>, CommandError> {
        let mut command = destination.take().expect("supplied expansion command");
        let action = action.unwrap_or_else(|| classify_hot_command(&command));
        if let ExpandedCommandAction::Expand(dispatch) = action {
            self.execute_expansion_action(&mut command, dispatch)?;
            return Ok(None);
        }
        let status = self.finish_terminal_expansion::<OBSERVED>(&mut command, action, false);
        *destination = Some(command);
        Ok(Some(status))
    }

    #[inline(always)]
    fn finish_terminal_expansion<const OBSERVED: bool>(
        &mut self,
        command: &mut CurrentCommand<G>,
        action: ExpandedCommandAction,
        expanded: bool,
    ) -> DeliveryStatus {
        if matches!(action, ExpandedCommandAction::EndTemplate) {
            if matches!(
                command.alignment_adjustment(),
                crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
            ) {
                return DeliveryStatus::AlignmentEndTemplate;
            }
            command.convert_end_template_to_endv(self.state.frozen_endv_token());
        }
        self.finish_expanded_command::<OBSERVED>(command, expanded)
    }
}
