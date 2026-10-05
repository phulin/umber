//! Consumer-side interpretation of the shared input reader.

use super::{
    ExpandedCommandAction, ExpandedReaderConsumer, ExpandedReaderRole, ExpansionDispatch, ReadSite,
    classify_hot_command, classify_native_command,
};
use crate::command::CurrentCommand;
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use std::ops::ControlFlow;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningFlags, MeaningWord};

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    #[inline(always)]
    pub(super) fn resolve_token_word(
        &self,
        word: tex_state::token::TokenWord,
    ) -> (MeaningWord<G>, Option<Symbol>) {
        let mut meaning = MeaningWord::scalar(Meaning::UNDEFINED_WORD);
        let control_sequence = self
            .state
            .write_packed_token_command_into(word, &mut meaning);
        (meaning, control_sequence)
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
    #[inline(always)]
    pub(super) fn expanded_delivery_loop<
        const OBSERVED: bool,
        const PRESERVE_UNDEFINED: bool,
        const STOP_PROTECTED: bool,
        Consumer: ExpandedReaderConsumer<Self, G, Facts>,
    >(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
        role: ExpandedReaderRole,
        terminal: &mut Consumer,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut expanded = false;
        let supplied = destination.is_some();
        let initial_status = if STOP_PROTECTED
            && destination.as_ref().is_some_and(|command| {
                Self::protected_terminal(command, classify_hot_command(command))
            }) {
            Some(DeliveryStatus::Command)
        } else if supplied {
            self.consume_supplied_command::<OBSERVED>(destination, initial_action, role)?
        } else {
            None
        };
        if let Some(status) = initial_status {
            if let ControlFlow::Break(()) = terminal.terminal(self, destination, status)? {
                return Ok(status);
            }
            destination.take();
        } else {
            expanded = supplied;
        }
        let status = self.consume_input::<_, _, _>(
            self.create_source_control_sequences,
            terminal,
            Consumer::OUTLINE_RESIDENT_SUFFIX,
            &|terminal, suppress, mode| {
                Consumer::RESIDENT_TERMINALS
                    && !OBSERVED
                    && terminal.resident_ready()
                    && !suppress
                    && !mode.requires_persistent_settlement()
            },
            &mut |terminal| terminal.input_boundary(),
            &mut {
                #[inline(always)]
                |terminal,
                 meanings,
                 state,
                 fuel,
                 alignment,
                 facts,
                 entered_line,
                 word,
                 origin,
                 _,
                 kind,
                 _mode| {
                    let mut admit = {
                        #[inline(always)]
                        |meaning: &MeaningWord<G>| {
                            let admitted = terminal.resident_terminal(
                                state,
                                facts,
                                entered_line,
                                tex_state::token::TracedTokenWord::from_parts(word, origin),
                                meaning,
                            );
                            #[cfg(feature = "profiling")]
                            if !matches!(admitted, ControlFlow::Continue(false)) {
                                fuel.record_raw_delivery(
                                    _mode.scanner_active(),
                                    word.requires_meaning_lookup(),
                                    kind,
                                );
                            }
                            #[cfg(not(feature = "profiling"))]
                            let _ = (&fuel, kind);
                            match admitted {
                                ControlFlow::Break(()) => {
                                    ControlFlow::Break(DeliveryStatus::Command)
                                }
                                ControlFlow::Continue(true) => ControlFlow::Continue(Ok(())),
                                ControlFlow::Continue(false) => {
                                    ControlFlow::Continue(Err(Some(*meaning)))
                                }
                            }
                        }
                    };
                    if let Some(symbol) = word.control_sequence() {
                        // Borrow the actual row through input-preserving group execution.
                        // Only a declined word copies its meaning into full delivery.
                        return admit(meanings.get(symbol));
                    }
                    if Consumer::RESIDENT_LITERALS
                        && let (Some(cat), Some(ch)) = (word.literal_catcode(), word.literal_char())
                        && cat != tex_state::token::Catcode::Active
                    {
                        let meaning = MeaningWord::scalar(Meaning::char_token_word(ch, cat));
                        let result = admit(&meaning);
                        if !matches!(result, ControlFlow::Continue(Err(_))) {
                            alignment.account_literal_catcode(Some(cat));
                        }
                        return result;
                    }
                    ControlFlow::Continue(Err(None))
                }
            },
            &mut {
                // Full interpretation starts after the span borrow ends.
                #[inline(always)]
                |terminal: &mut Consumer,
                 processor,
                 word,
                 origin,
                 suppress,
                 site,
                 kind,
                 resolved: Option<MeaningWord<G>>| {
                    let (meaning, control_sequence) = match resolved {
                        Some(meaning) => (meaning, word.control_sequence()),
                        None => processor.resolve_token_word(word),
                    };
                    processor.record_consumed_word(kind, word.requires_meaning_lookup());
                    let macro_parts = meaning.macro_parts();
                    let mode = processor.command.delivery_mode;
                    let native = !OBSERVED
                        && !suppress
                        && !mode.alignment_active()
                        && (!mode.scanner_active() || !meaning.is_outer());
                    if native
                        && let Some((flags, definition)) = macro_parts
                        && !(STOP_PROTECTED && flags.contains(MeaningFlags::PROTECTED))
                    {
                        let name = control_sequence.ok_or_else(CommandError::input_invariant)?;
                        terminal.before_action(processor);
                        let result = if Consumer::RAW_TOKEN {
                            processor.with_expansion_depth(|processor| {
                                processor.activate_read_macro(word, origin, flags, definition, name)
                            })
                        } else {
                            processor.activate_read_macro(word, origin, flags, definition, name)
                        };
                        if let Err(failure) = result {
                            terminal.recover(processor, failure)?;
                        }
                        expanded = true;
                        return Ok(terminal.expanded().map_break(|()| DeliveryStatus::Command));
                    }
                    let native_action;
                    if native {
                        let (action, code) =
                            classify_native_command(word, &meaning, macro_parts.is_some());
                        native_action = action;
                        if matches!(site, ReadSite::Source(_))
                            && let Some(location) = processor.pending_diagnostic_location.take()
                        {
                            processor
                                .command
                                .set_last_diagnostic_location(Some(location));
                        }
                        if let ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(
                            primitive,
                        )) = native_action
                        {
                            terminal.before_action(processor);
                            let result =
                                terminal.splice(processor, primitive).and_then(|spliced| {
                                    if spliced {
                                        Ok(())
                                    } else {
                                        processor.expand_primitive(
                                            primitive,
                                            tex_state::token::TracedTokenWord::from_parts(
                                                word, origin,
                                            ),
                                            control_sequence,
                                            true,
                                        )
                                    }
                                });
                            if let Err(failure) = result {
                                if Consumer::RAW_TOKEN {
                                    terminal.recover(processor, failure)?;
                                } else {
                                    return Err(failure);
                                }
                            }
                            expanded = true;
                            return Ok(terminal.expanded().map_break(|()| DeliveryStatus::Command));
                        }
                        let protected = STOP_PROTECTED
                            && macro_parts
                                .is_some_and(|(flags, _)| flags.contains(MeaningFlags::PROTECTED));
                        if Consumer::NATIVE_TERMINAL
                            && (protected || matches!(native_action, ExpandedCommandAction::Return))
                        {
                            processor
                                .command
                                .roots
                                .alignment
                                .account_literal_catcode(word.literal_catcode());
                            terminal.before_action(processor);
                            let result = terminal.native_terminal(
                                processor,
                                tex_state::token::TracedTokenWord::from_parts(word, origin),
                                &meaning,
                                code,
                                control_sequence,
                            )?;
                            expanded = false;
                            return Ok(result.map_break(|()| DeliveryStatus::Command));
                        }
                    } else {
                        native_action = ExpandedCommandAction::Return;
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
                        control_sequence,
                    ));
                    if native {
                        processor
                            .command
                            .roots
                            .alignment
                            .account_literal_brace(command, word.literal_catcode());
                    } else {
                        processor
                            .settle_hot_delivery_in::<OBSERVED>(command, word.literal_catcode())?;
                    }
                    if Consumer::RAW_TOKEN
                        && matches!(
                            command.alignment_adjustment(),
                            crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                        )
                    {
                        let command = destination.take().expect("alignment delimiter");
                        processor.begin_scalar_alignment_v_template(&command)?;
                        return Ok(ControlFlow::Continue(()));
                    }
                    terminal.before_action(processor);
                    let action = if native {
                        native_action
                    } else {
                        classify_hot_command(command)
                    };
                    let terminal_status = if STOP_PROTECTED
                        && Self::protected_terminal(command, action)
                    {
                        DeliveryStatus::Command
                    } else {
                        match terminal.expand(processor, command, action) {
                            Ok(true) => {
                                expanded = true;
                                return Ok(terminal
                                    .expanded()
                                    .map_break(|()| DeliveryStatus::Command));
                            }
                            Ok(false) => {}
                            Err(failure) => {
                                terminal.recover(processor, failure)?;
                                expanded = true;
                                return Ok(terminal
                                    .expanded()
                                    .map_break(|()| DeliveryStatus::Command));
                            }
                        }
                        if !(PRESERVE_UNDEFINED
                            && matches!(
                                action,
                                ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
                            ))
                            && let ExpandedCommandAction::Expand(dispatch) = action
                        {
                            processor.execute_expansion_action(command, dispatch)?;
                            expanded = true;
                            return Ok(terminal.expanded().map_break(|()| DeliveryStatus::Command));
                        }
                        if !Consumer::RAW_TOKEN
                            && role == ExpandedReaderRole::MainLoopLookahead
                            && matches!(action, ExpandedCommandAction::EndTemplate)
                            && !matches!(
                                command.alignment_adjustment(),
                                crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                            )
                        {
                            // §1038 uses x_token (§381), whose expand (§366/§376)
                            // backs frozen endv. Only get_x_token (§380) changes
                            // the command in place. This is actual TeX input.
                            processor.back_input_token(tex_state::token::TracedTokenWord::pack(
                                processor.state.frozen_endv_token(),
                                tex_state::token::OriginId::UNKNOWN,
                            ))?;
                            destination.take();
                            expanded = true;
                            return Ok(terminal.expanded().map_break(|()| DeliveryStatus::Command));
                        }
                        if Consumer::RAW_TOKEN
                            || (role == ExpandedReaderRole::MainLoopLookahead
                                && !expanded
                                && command.is_main_loop_character())
                        {
                            DeliveryStatus::Command
                        } else {
                            processor
                                .finish_terminal_expansion::<OBSERVED>(command, action, expanded)
                        }
                    };
                    match terminal.terminal(processor, destination, terminal_status)? {
                        ControlFlow::Break(()) => Ok(ControlFlow::Break(terminal_status)),
                        ControlFlow::Continue(()) => {
                            destination.take();
                            expanded = false;
                            Ok(ControlFlow::Continue(()))
                        }
                    }
                }
            },
            |status| {
                if Consumer::RAW_TOKEN && matches!(status, DeliveryStatus::ReplayCompleted(_)) {
                    Ok(ControlFlow::Continue(()))
                } else {
                    Ok(ControlFlow::Break(status))
                }
            },
        )?;
        if matches!(
            status,
            DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_)
        ) {
            destination.take();
        }
        Ok(status)
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
            ) && command.flags().contains(MeaningFlags::PROTECTED)
    }

    /// Matching's local storage belongs to macro activation, never to a
    /// recursively suspended primitive-expansion frame. Only spelling and
    /// invocation facts survive matching; diagnostics need no delivery geometry
    /// or command record, even on prefix failure.
    #[inline(always)]
    pub(super) fn activate_read_macro(
        &mut self,
        word: tex_state::token::TokenWord,
        origin: tex_state::token::OriginId,
        flags: MeaningFlags,
        definition: tex_state::DefinitionRef<G>,
        name: Symbol,
    ) -> Result<(), CommandError> {
        self.record_macro_expansion();
        self.macro_call_parts(flags, definition, name, origin, |processor| {
            processor.report_unobserved_macro_prefix_mismatch(
                tex_state::token::TracedTokenWord::from_parts(word, origin),
                flags,
                definition,
                name,
            );
        })
        .map(|_| ())
    }

    /// Existing commands have already crossed a delivery boundary. Keep their
    /// temporary owner out of the ordinary reader's recursive stack frame.
    #[inline(never)]
    fn consume_supplied_command<const OBSERVED: bool>(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        action: Option<ExpandedCommandAction>,
        role: ExpandedReaderRole,
    ) -> Result<Option<DeliveryStatus>, CommandError> {
        let mut command = destination.take().expect("supplied expansion command");
        // TeX82 §1038 also accepts the already-fetched character boundary
        // from source batching. It stays raw and emits no expanded observation.
        if role == ExpandedReaderRole::MainLoopLookahead && command.is_main_loop_character() {
            *destination = Some(command);
            return Ok(Some(DeliveryStatus::Command));
        }
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
