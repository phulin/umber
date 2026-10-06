//! Fused resident-input advancement and raw/expanded command delivery.

mod collect;
mod consume;
mod input;
mod once;
mod primitive;
mod resident;
mod scalar;
mod skipping;

use tex_state::meaning::{ExpandablePrimitive, Meaning, ResolvedMeaning};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use crate::CommandError;
use crate::command::CurrentCommand;

use super::{AlignmentLookahead, CommandProcessor, DeliveryStatus};

use crate::observation::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandProvenance,
};

/// Synchronous action of the actual expansion consumer. Empty ordinary hooks
/// disappear in its static reader specialization; collection borrows its final
/// output and scanner episode for the duration of this call only.
pub(crate) trait ExpandedReaderConsumer<Processor, G, Facts> {
    /// Read each operand with get_token/get_next alignment and capacity semantics.
    const RAW_TOKEN: bool = false;
    const NATIVE_TERMINAL: bool = false;
    const RESIDENT_TERMINALS: bool = false;
    const RESIDENT_LITERALS: bool = false;

    fn native_terminal(
        &mut self,
        _processor: &mut Processor,
        _spelling: TracedTokenWord,
        _meaning: &tex_state::meaning::MeaningWord<G>,
        _code: u64,
        _control_sequence: Option<tex_state::interner::Symbol>,
    ) -> Result<std::ops::ControlFlow<()>, CommandError> {
        unreachable!("this consumer requests a delivery record")
    }

    fn resident_ready(&self) -> bool {
        false
    }

    #[allow(clippy::too_many_arguments)] // Disjoint input-preserving execution owners and spelling.
    fn resident_terminal(
        &mut self,
        _context: &mut tex_state::CommandContext<'_, G>,
        _fuel: &mut crate::CommandFuel,
        _effects: &mut tex_state::diagnostic::DiagnosticEffects,
        _facts: &mut Facts,
        _entered_line: u32,
        _macro_origin: Option<tex_state::token::OriginId>,
        _spelling: TracedTokenWord,
        _code: u64,
        _operand: u64,
    ) -> std::ops::ControlFlow<(), bool> {
        std::ops::ControlFlow::Continue(false)
    }

    fn input_boundary(&mut self) {}

    fn before_action(&mut self, _processor: &mut Processor) {}

    fn expanded(&mut self) -> std::ops::ControlFlow<()> {
        std::ops::ControlFlow::Continue(())
    }

    fn terminal(
        &mut self,
        processor: &mut Processor,
        destination: &mut Option<CurrentCommand<G>>,
        status: DeliveryStatus,
    ) -> Result<std::ops::ControlFlow<()>, CommandError>;

    fn expand(
        &mut self,
        _processor: &mut Processor,
        _command: &mut CurrentCommand<G>,
        _action: ExpandedCommandAction,
    ) -> Result<bool, CommandError> {
        Ok(false)
    }

    fn splice(
        &mut self,
        _processor: &mut Processor,
        _primitive: ExpandablePrimitive,
    ) -> Result<bool, CommandError> {
        Ok(false)
    }

    fn recover(
        &mut self,
        _processor: &mut Processor,
        failure: CommandError,
    ) -> Result<(), CommandError> {
        match failure {
            CommandError::ParagraphInMacroArgument | CommandError::OuterInMacroArgument => Ok(()),
            failure => Err(failure),
        }
    }
}

struct DeliveredCommand;

impl<Processor, G, Facts> ExpandedReaderConsumer<Processor, G, Facts> for DeliveredCommand {
    fn terminal(
        &mut self,
        _processor: &mut Processor,
        _destination: &mut Option<CurrentCommand<G>>,
        _status: DeliveryStatus,
    ) -> Result<std::ops::ControlFlow<()>, CommandError> {
        Ok(std::ops::ControlFlow::Break(()))
    }
}

/// Only the genuine terminal treatment differs between canonical reader roles.
#[derive(Clone, Copy, PartialEq)]
enum ExpandedReaderRole {
    Scanner,
    MainControl,
    MainLoopLookahead,
}

#[derive(Clone, Copy)]
enum ReadSite {
    Resident,
    Source(Option<u32>),
    Synthetic,
}

/// The one decision TeX.web §380 makes after raw delivery has resolved the
/// current meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExpandedCommandAction {
    Return,
    EndTemplate,
    Expand(ExpansionDispatch),
}

/// The exact TeX.web §366 branch selected by expanded-command
/// classification. This is call-local control flow, not a retained meaning
/// representation. A resource miss unwinds this ordinary call tree and the
/// host replays from a full checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExpansionDispatch {
    Macro,
    Primitive(ExpandablePrimitive),
    Undefined,
}

#[cfg(test)]
thread_local! {
    static EXPANDED_CLASSIFICATIONS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}
#[cfg(test)]
fn expanded_classifications() -> u64 {
    EXPANDED_CLASSIFICATIONS.with(core::cell::Cell::get)
}

#[inline(always)]
fn classify_hot_command<G>(command: &CurrentCommand<G>) -> ExpandedCommandAction {
    classify_native_command(
        command.spelling_word(),
        command.native_meaning().execution_key(),
        command.native_meaning().is_macro(),
    )
}

#[inline(always)]
fn classify_native_command(
    spelling: TokenWord,
    code: u64,
    is_macro: bool,
) -> ExpandedCommandAction {
    #[cfg(test)]
    EXPANDED_CLASSIFICATIONS.with(|counter| counter.set(counter.get().saturating_add(1)));
    if is_macro {
        return ExpandedCommandAction::Expand(ExpansionDispatch::Macro);
    }
    match ExpandablePrimitive::from_execution_key(code) {
        Some(ExpandablePrimitive::EndTemplate) => ExpandedCommandAction::EndTemplate,
        Some(ExpandablePrimitive::EndCsName) => ExpandedCommandAction::Return,
        Some(primitive) => ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive)),
        None if code == Meaning::runtime_word_control_code(Meaning::UNDEFINED_WORD)
            && spelling.out_parameter_slot().is_none() =>
        {
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
        }
        _ => ExpandedCommandAction::Return,
    }
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn write_delivered_word(
        &mut self,
        word: TokenWord,
        origin: OriginId,
        suppress: bool,
        site: ReadSite,
        kind: crate::fuel::RawDeliveryKind,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Option<Catcode> {
        let line = match site {
            ReadSite::Source(line) => line,
            _ => None,
        };
        CurrentCommand::write_delivery_into(
            destination,
            word,
            origin,
            matches!(site, ReadSite::Source(_)),
            line,
            suppress,
            self.state,
        );
        self.record_consumed_word(kind, word.requires_meaning_lookup());
        word.literal_catcode()
    }

    #[inline(always)]
    fn settle_hot_delivery_in<const OBSERVED: bool>(
        &mut self,
        command: &mut CurrentCommand<G>,
        literal_catcode: Option<Catcode>,
    ) -> Result<(), CommandError> {
        let suppress = command.suppresses_expandable_control_sequence();
        let outer = command.is_outer();
        if OBSERVED
            || self
                .command
                .delivery_mode
                .requires_semantic_settlement(suppress, outer)
        {
            return self.settle_exceptional_delivery::<OBSERVED>(
                command,
                literal_catcode,
                suppress,
                outer,
            );
        }
        self.update_delivered_source_location(command);
        self.command
            .roots
            .alignment
            .account_literal_brace(command, literal_catcode);
        Ok(())
    }

    #[inline(always)]
    fn update_delivered_source_location(&mut self, command: &CurrentCommand<G>) {
        if command.is_direct_source_delivery()
            && let Some(location) = self.pending_diagnostic_location.take()
        {
            self.command.set_last_diagnostic_location(Some(location));
        }
    }

    /// The concrete TeX82 §341 raw-token loop. It owns one fuel charge for
    /// each semantic raw token and retries only through cold input transitions.
    #[inline(always)]
    pub(super) fn raw_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        destination.take();
        self.raw_next_hot(destination)
    }

    #[inline(always)]
    pub(crate) fn raw_next_hot(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let result = if self.is_observed() {
            self.fetch_hot::<true>(destination)
        } else {
            self.fetch_hot::<false>(destination)
        };
        match result {
            Ok(status) => Ok(status),
            Err(failure) => self.fail_hot_expanded_delivery(
                destination,
                self.command.transient.active_expansion_depth,
                failure,
            ),
        }
    }

    /// Raw callers request a command from the shared reader and settlement authority.
    /// Input boundaries retry under the original charge. Every successful read
    /// borrows the currently exposed semantic frame, including after recovery.
    #[inline(always)]
    fn fetch_hot<const OBSERVED: bool>(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let status = self.consume_input::<_, _, _>(
            self.create_source_control_sequences,
            &mut (),
            false,
            &|_, _, _| false,
            &mut |_| {},
            &mut |_, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _| {
                std::ops::ControlFlow::Continue(Err(()))
            },
            &mut |_, processor, word, origin, suppress, site, kind, _| {
                processor
                    .finish_delivered_word::<OBSERVED>(
                        word,
                        origin,
                        suppress,
                        site,
                        kind,
                        destination,
                    )
                    .map(std::ops::ControlFlow::Break)
            },
            |status| Ok(std::ops::ControlFlow::Break(status)),
        )?;
        if matches!(
            status,
            DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_)
        ) {
            destination.take();
        }
        Ok(status)
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn finish_delivered_word<const OBSERVED: bool>(
        &mut self,
        word: TokenWord,
        origin: OriginId,
        suppress: bool,
        site: ReadSite,
        kind: crate::fuel::RawDeliveryKind,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let literal = self.write_delivered_word(word, origin, suppress, site, kind, destination);
        self.settle_hot_delivery_in::<OBSERVED>(
            destination.as_mut().expect("initialized delivery"),
            literal,
        )?;
        Ok(DeliveryStatus::Command)
    }

    /// Charges one semantic raw token, selects each exposed native span once,
    /// and invokes the actual consumer with word scalars. Cold transitions
    /// retry under that same charge; no token result is constructed between
    /// storage, resolution and consumption.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)] // Actual consumer, admission and input-transition actions.
    fn consume_input<R, Consumer, Admission: Default>(
        &mut self,
        create_control_sequences: bool,
        consumer: &mut Consumer,
        glyph_origins: bool,
        retain: &impl Fn(&Consumer, bool, super::delivery_mode::DeliveryMode) -> bool,
        interrupt: &mut impl FnMut(&mut Consumer),
        admit: &mut impl FnMut(
            &mut Consumer,
            &mut tex_state::CommandContext<'_, G>,
            &mut crate::fuel::CommandFuel,
            &mut usize,
            usize,
            bool,
            &mut tex_state::diagnostic::DiagnosticEffects,
            &mut super::alignment::AlignmentDeliveryState<G>,
            &mut Facts,
            u32,
            Option<OriginId>,
            TokenWord,
            OriginId,
            bool,
            crate::fuel::RawDeliveryKind,
            super::delivery_mode::DeliveryMode,
        ) -> std::ops::ControlFlow<R, Result<(), Admission>>,
        finish: &mut impl FnMut(
            &mut Consumer,
            &mut Self,
            TokenWord,
            OriginId,
            bool,
            ReadSite,
            crate::fuel::RawDeliveryKind,
            Admission,
        ) -> Result<std::ops::ControlFlow<R>, CommandError>,
        finished: impl Fn(DeliveryStatus) -> Result<std::ops::ControlFlow<R>, CommandError>,
    ) -> Result<R, CommandError> {
        loop {
            self.pending_diagnostic_location = None;
            self.charge_command_action()?;
            let consumed = loop {
                let resident = self.consume_resident_input(
                    create_control_sequences,
                    consumer,
                    glyph_origins,
                    retain,
                    interrupt,
                    &mut {
                        #[inline(always)]
                        |consumer,
                         state,
                         fuel,
                         input_usage,
                         input_depth,
                         frame_live,
                         effects,
                         alignment,
                         facts,
                         entered_line,
                         macro_origin,
                         word,
                         origin,
                         suppress,
                         kind,
                         mode| {
                            let result = admit(
                                consumer,
                                state,
                                fuel,
                                input_usage,
                                input_depth,
                                frame_live,
                                effects,
                                alignment,
                                facts,
                                entered_line,
                                macro_origin,
                                word,
                                origin,
                                suppress,
                                kind,
                                mode,
                            );
                            result.map_break(std::ops::ControlFlow::Break)
                        }
                    },
                    finish,
                    &finished,
                )?;
                if let Some(result) = resident {
                    break result;
                }
                interrupt(consumer);
                let boundary = self.finish_input_boundary(
                    crate::input::ResidentBoundary::Empty,
                    &mut |processor, word, origin, suppress, site, kind| {
                        finish(
                            consumer,
                            processor,
                            word,
                            origin,
                            suppress,
                            site,
                            kind,
                            Admission::default(),
                        )
                    },
                )?;
                if let Some(result) = boundary {
                    match result {
                        std::ops::ControlFlow::Continue(value) => break value,
                        std::ops::ControlFlow::Break(status) => break finished(status)?,
                    }
                }
            };
            if let std::ops::ControlFlow::Break(result) = consumed {
                return Ok(result);
            }
        }
    }

    /// Delivers one expanded command through the shared native slot, clearing
    /// that actual destination at completion and failure boundaries.
    pub(super) fn expanded_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next_with_action(destination, None)
    }

    fn expanded_next_with_action(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next_hot(destination, initial_action)
    }

    fn expanded_next_with_preserved_undefined(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next_hot_preserving_undefined(destination)
    }

    /// TeX82 §380's synchronous expanded delivery. Observation is fixed for
    /// this call, while scanner and alignment semantics remain live. This
    /// wrapper owns depth and failure cleanup outside the expansion back edge.
    fn expanded_next_hot(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next_hot_with_mode::<false, false>(
            destination,
            initial_action,
            ExpandedReaderRole::Scanner,
        )
    }

    #[cold]
    #[inline(never)]
    fn expanded_next_hot_preserving_undefined(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next_hot_with_mode::<true, false>(
            destination,
            None,
            ExpandedReaderRole::Scanner,
        )
    }

    /// Delivers e-TeX's `get_x_or_protected`: each raw fetch is expanded only
    /// when it is neither unexpandable nor a protected macro.  The protected
    /// terminal remains a raw delivery for the caller to back up, so the
    /// ordinary `get_x_token` expanded observation is never emitted for it.
    #[cold]
    #[inline(never)]
    fn expanded_next_protected(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next_hot_with_mode::<false, true>(
            destination,
            None,
            ExpandedReaderRole::Scanner,
        )
    }

    fn expanded_next_hot_with_mode<const PRESERVE_UNDEFINED: bool, const STOP_PROTECTED: bool>(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
        role: ExpandedReaderRole,
    ) -> Result<DeliveryStatus, CommandError> {
        self.consume_expanded_with_mode::<PRESERVE_UNDEFINED, STOP_PROTECTED>(
            destination,
            initial_action,
            role,
            &mut DeliveredCommand,
        )
    }

    /// Raw-token expansion consumers share the canonical reader. Their actual
    /// expansion actions retain native capacity and completion semantics.
    pub(crate) fn consume_raw_expansion<const STOP_PROTECTED: bool>(
        &mut self,
        consumer: &mut impl ExpandedReaderConsumer<Self, G, Facts>,
    ) -> Result<(), CommandError> {
        let mut destination = None;
        let result = if self.is_observed() {
            self.expanded_delivery_loop::<true, false, STOP_PROTECTED, _>(
                &mut destination,
                None,
                ExpandedReaderRole::Scanner,
                consumer,
            )
        } else {
            self.expanded_delivery_loop::<false, false, STOP_PROTECTED, _>(
                &mut destination,
                None,
                ExpandedReaderRole::Scanner,
                consumer,
            )
        };
        match result {
            Ok(DeliveryStatus::End) => Err(CommandError::input_invariant()),
            Ok(_) => Ok(()),
            Err(failure) => self.fail_hot_expanded_delivery(
                &mut destination,
                self.command.transient.active_expansion_depth,
                failure,
            ),
        }
    }

    #[inline(always)]
    fn consume_expanded_with_mode<const PRESERVE_UNDEFINED: bool, const STOP_PROTECTED: bool>(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
        role: ExpandedReaderRole,
        terminal: &mut impl ExpandedReaderConsumer<Self, G, Facts>,
    ) -> Result<DeliveryStatus, CommandError> {
        if self.is_observed() {
            return self.consume_observed_expanded_words::<PRESERVE_UNDEFINED, STOP_PROTECTED>(
                destination,
                initial_action,
                role,
                terminal,
            );
        }
        self.with_expanded_delivery(destination, |processor, destination| {
            processor.expanded_delivery_loop::<false, PRESERVE_UNDEFINED, STOP_PROTECTED, _>(
                destination,
                initial_action,
                role,
                terminal,
            )
        })
    }

    /// One real reader invocation records checkpoint quiescence, not per-word state.
    #[inline(always)]
    fn with_expanded_delivery(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        consume: impl FnOnce(
            &mut Self,
            &mut Option<CurrentCommand<G>>,
        ) -> Result<DeliveryStatus, CommandError>,
    ) -> Result<DeliveryStatus, CommandError> {
        let depth = self.command.transient.active_expansion_depth;
        let Some(active_depth) = depth.checked_add(1) else {
            return self.fail_hot_expanded_delivery(
                destination,
                depth,
                CommandError::input_invariant(),
            );
        };
        self.command.transient.active_expansion_depth = active_depth;
        let result = consume(self, destination);
        debug_assert_eq!(self.command.transient.active_expansion_depth, active_depth);
        self.command.transient.active_expansion_depth = depth;
        match result {
            Ok(status) => Ok(status),
            Err(failure) => self.fail_hot_expanded_delivery(destination, depth, failure),
        }
    }

    /// Primitive and supplied-command execution is separate from input reading.
    /// Its recursive scanner frames need no resident word or meaning temporaries.
    #[inline(never)]
    fn execute_expansion_action(
        &mut self,
        command: &mut CurrentCommand<G>,
        action: ExpansionDispatch,
    ) -> Result<(), CommandError> {
        match action {
            ExpansionDispatch::Macro | ExpansionDispatch::Undefined => {
                self.expand_hot_definition_action(command, action)
            }
            ExpansionDispatch::Primitive(primitive) => self.expand_primitive(
                primitive,
                command.spelling(),
                command.control_sequence(),
                true,
            ),
        }
    }

    /// Keeps macro-definition and undefined-control-sequence handling out of
    /// the ordinary conditional recursion frame. These branches share the
    /// command ownership cleanup but are not part of the scalar hot path, so
    /// their error storage lives in this out-of-line helper.
    #[inline(never)]
    fn expand_hot_definition_action(
        &mut self,
        hot_command: &mut CurrentCommand<G>,
        action: ExpansionDispatch,
    ) -> Result<(), CommandError> {
        match action {
            ExpansionDispatch::Macro => {
                self.record_macro_expansion();
                if let Err(failure) = self.macro_call_hot(hot_command) {
                    match failure {
                        CommandError::ParagraphInMacroArgument
                        | CommandError::OuterInMacroArgument => {}
                        failure => return Err(failure),
                    }
                }
            }
            ExpansionDispatch::Undefined => {
                self.expand_undefined_hot(hot_command, true)?;
            }
            ExpansionDispatch::Primitive(_) => {
                return Err(CommandError::input_invariant());
            }
        }
        Ok(())
    }

    #[inline(always)]
    fn record_macro_expansion(&mut self) {
        #[cfg(feature = "profiling")]
        {
            tex_state::measurement::record_hot_core_macro_expansion();
            if self.write_expansion_depth != 0 {
                self.record_write_expansion();
            }
        }
    }

    /// Replay-aware raw delivery is a cold entry for the same raw owner. The
    /// public ordinary wrapper consumes completion statuses; replay callers
    /// keep them visible.
    #[cold]
    #[inline(never)]
    pub(super) fn raw_next_with_replay_completion(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.raw_next(destination)
    }

    /// Replay-aware ordinary expansion enters the canonical expanded loop and
    /// leaves its completion status visible to the caller.
    #[cold]
    #[inline(never)]
    pub(super) fn expanded_next_with_replay_completion(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next(destination)
    }

    /// The protected entry is intentionally out of line. Its full protected
    /// classifier is installed below once a raw command has been settled.
    #[cold]
    #[inline(never)]
    pub(super) fn protected_expanded_next_with_replay_completion(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next_protected(destination)
    }

    #[cold]
    #[inline(never)]
    fn expanded_until_undefined(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let status = if destination.is_some() {
                DeliveryStatus::Command
            } else {
                self.raw_next(destination)?
            };
            match status {
                DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_) => return Ok(status),
                DeliveryStatus::Command => {}
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                    continue;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
            }

            let command = destination
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            let stop = matches!(
                &command.meaning(),
                ResolvedMeaning::Static(Meaning::Undefined)
            );
            if stop {
                return Ok(DeliveryStatus::Command);
            }
            let result = self.expanded_next_with_preserved_undefined(destination)?;
            match result {
                status @ (DeliveryStatus::End | DeliveryStatus::ReplayCompleted(_)) => {
                    return Ok(status);
                }
                DeliveryStatus::Command => return Ok(DeliveryStatus::Command),
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
            }
        }
    }

    /// Diagnostic callers keep the undefined command instead of entering its
    /// recovery branch. The exceptional wrapper is cold and owns that one
    /// classifier choice.
    #[cold]
    #[inline(never)]
    pub(super) fn expanded_next_preserving_undefined(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_until_undefined(destination)
    }

    fn x_token_next_with_action(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut initial_action = initial_action;
        if destination.as_ref().is_some_and(|command| {
            matches!(
                &command.meaning(),
                ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                    ExpandablePrimitive::EndTemplate
                ))
            )
        }) {
            let alignment_delimiter = destination.as_ref().is_some_and(|command| {
                matches!(
                    command.alignment_adjustment(),
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                )
            });
            if alignment_delimiter {
                return Ok(DeliveryStatus::AlignmentEndTemplate);
            }
            destination.take();
            self.insert_frozen_endv()?;
            initial_action = None;
        }
        self.expanded_next_with_action(destination, initial_action)
    }

    #[cold]
    #[inline(never)]
    pub(super) fn alignment_expanded_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        match self.expanded_next(destination)? {
            DeliveryStatus::PendingExpanded => Ok(DeliveryStatus::Command),
            result => Ok(result),
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn alignment_main_loop_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        match self.consume_expanded_with_mode::<false, false>(
            destination,
            None,
            ExpandedReaderRole::MainLoopLookahead,
            &mut DeliveredCommand,
        )? {
            DeliveryStatus::PendingExpanded => Ok(DeliveryStatus::Command),
            result => Ok(result),
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn tex_alignment_lookahead_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.expanded_next(destination)? {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::AlignmentClosingBrace => return Ok(DeliveryStatus::Command),
                result => return Ok(result),
            }
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn etex_alignment_lookahead_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.protected_expanded_next_with_replay_completion(destination)? {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::AlignmentClosingBrace => return Ok(DeliveryStatus::Command),
                result => return Ok(result),
            }
        }
    }

    /// Delivers one expanded command directly into caller-provided storage.
    pub fn get_x_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        loop {
            let result = self.expanded_next_with_action(destination, None)?;
            match result {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::End | DeliveryStatus::Command => return Ok(result),
            }
        }
    }

    /// Requests one expanded token from the generation-scoped delivery
    /// driver.  Scanner and primitive code uses this typed status boundary;
    /// it never reaches into the driver's loop or recursively calls a
    /// delivery implementation by name.
    pub(crate) fn request_expanded_token(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        // Scanner calls are ordinary synchronous Rust calls.  A resource
        // error returns through this boundary and the processor drop hook
        // unwinds all call-local expansion state for checkpoint replay.
        self.get_x_token_into(destination)
    }

    /// Requests one already-delivered command's expansion from the same
    /// driver.  This is the only nested expansion request used by structural
    /// scanners; suspension and completion remain represented by the typed
    /// `Result` status returned here.
    pub(crate) fn request_expansion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        let mut command = destination
            .take()
            .ok_or_else(CommandError::input_invariant)?;
        let action = classify_hot_command(&command);
        let result = self.expand_delivered_action(&mut command, action, report_trace);
        if result.is_err() {
            // Retain the actual delivered owner for recovery by
            // callers that inspect or clear the opener after a failed
            // expansion. Successful expansion consumes it below.
            *destination = Some(command);
            return result;
        }
        // The expanded opener is consumed by the nested call and cannot
        // become the scanner's next operand.
        Ok(())
    }

    /// One expansion action for consumers that already own a raw command.
    pub(crate) fn expand_delivered_action(
        &mut self,
        command: &mut CurrentCommand<G>,
        action: ExpandedCommandAction,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        match action {
            ExpandedCommandAction::Return => Err(CommandError::input_invariant()),
            ExpandedCommandAction::EndTemplate => self.expand_primitive(
                ExpandablePrimitive::EndTemplate,
                command.spelling(),
                command.control_sequence(),
                report_trace,
            ),
            ExpandedCommandAction::Expand(ExpansionDispatch::Macro) => {
                #[cfg(feature = "profiling")]
                {
                    tex_state::measurement::record_hot_core_macro_expansion();
                    if self.write_expansion_depth != 0 {
                        self.record_write_expansion();
                    }
                }
                self.with_expansion_depth(|processor| processor.macro_call_hot(command))
            }
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined) => {
                self.expand_undefined_hot(command, report_trace)
            }
            ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive)) => self
                .expand_primitive(
                    primitive,
                    command.spelling(),
                    command.control_sequence(),
                    report_trace,
                ),
        }
    }

    /// Delivers protected replay-aware expansion into caller-provided storage.
    pub(crate) fn get_x_or_protected_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let preserve = self.command.profile().capabilities().supports_etex();
        let result = if preserve {
            self.protected_expanded_next_with_replay_completion(destination)?
        } else {
            self.expanded_next_with_replay_completion(destination)?
        };
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Delivers one expanded command to a diagnostic host while preserving
    /// TeX82 §370's undefined command instead of consuming it after recovery.
    pub fn get_x_token_preserving_undefined_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.expanded_next_preserving_undefined(destination)
    }

    /// TeX.web §381's `x_token` entered with `cur_cmd`/`cur_chr` already set.
    ///
    /// §381 does not begin with `get_next`: it expands whatever the caller
    /// left in the current command and only then reads on. Ordinary delivery
    /// leaves nothing, which is [`Self::get_x_token`]; §1152 loads an active
    /// character's meaning directly and passes it here, so that meaning is
    /// expanded without ever having been delivered raw.
    fn x_token_from_into(
        &mut self,
        pending: Option<CurrentCommand<G>>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.x_token_from_into_with_action(pending, destination, None)
    }

    fn x_token_from_into_with_action(
        &mut self,
        pending: Option<CurrentCommand<G>>,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        *destination = pending;
        let result = self.x_token_next_with_action(destination, initial_action)?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End
                | DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentEndTemplate
                | DeliveryStatus::AlignmentClosingBrace
                | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Completes TeX82 §1152's active-character `x_token` handoff.
    ///
    /// The ordinary destination-directed expanded entry exposes
    /// `PendingExpanded` and `AlignmentClosingBrace` only as internal
    /// observer transport markers; both already leave the settled command in
    /// `destination`. Active-character treatment has the same settled-command
    /// ownership, so it must normalize those statuses without constructing or
    /// redelivering another command. An intercepted alignment end-template is
    /// the one exceptional boundary: its command is consumed to begin the
    /// scalar v-template, after which `x_token` retries with no pending
    /// command above the newly installed input frame.
    #[cold]
    #[inline(never)]
    fn active_x_token_into(
        &mut self,
        pending: CurrentCommand<G>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut pending = Some(pending);
        loop {
            match self.x_token_from_into(pending.take(), destination)? {
                DeliveryStatus::End => return Ok(DeliveryStatus::End),
                DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentClosingBrace => return Ok(DeliveryStatus::Command),
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                    // The intercepted delimiter has been consumed by the
                    // alignment transition. The next x-token starts with a
                    // fresh input fetch, rather than redelivering it.
                    pending = None;
                }
                DeliveryStatus::ReplayCompleted(_) => {
                    // Stored replay retirement is an input-boundary event,
                    // not a settled active-character command. Continue the
                    // same x-token operation after the continuation retires.
                    pending = None;
                }
            }
        }
    }

    /// Demanded synthetic-command evidence for observation and exceptional x_token.
    #[cold]
    #[inline(never)]
    fn treat_observed_or_exceptional_active_character(
        &mut self,
        spelling: TracedTokenWord,
        meaning: tex_state::meaning::MeaningWord<G>,
        control_sequence: Option<tex_state::interner::Symbol>,
    ) -> Result<(), CommandError> {
        self.advance_delivery_sequence();
        let command = CurrentCommand::from_delivered_parts(
            spelling.token_word(),
            spelling.origin(),
            false,
            None,
            false,
            meaning,
            control_sequence,
        );
        let mut destination = None;
        let status = self.active_x_token_into(command, &mut destination)?;
        let settled = match status {
            DeliveryStatus::End => return Ok(()),
            DeliveryStatus::Command => destination
                .take()
                .expect("command status initializes destination"),
            _ => unreachable!("active-character delivery normalizes to commands"),
        };
        self.back_input(settled)
    }

    /// TeX82 §§785/791's shared alignment lookahead fetch.
    ///
    /// TeX82's `get_x_token` commits the terminal expanded command before
    /// `init_col` backs an ordinary command up. The backup is later read
    /// again above its u-template, producing a second raw/expanded delivery.
    /// Spacers skipped by §406 are complete deliveries and are committed here
    /// normally.
    ///
    /// e-TeX 2.6 change sections [37.785] and [37.791] replace that helper
    /// with `get_x_or_protected`. Its terminal unexpandable command comes
    /// straight from `get_token`, so neither skipped spacers nor a consumed
    /// `\noalign`, `\crcr`, `\omit`, or closing brace has an expanded
    /// delivery. A protected macro is likewise terminal and is backed up as
    /// the first command of the next cell.
    pub fn next_alignment_lookahead(
        &mut self,
    ) -> Result<Option<AlignmentLookahead<G>>, CommandError> {
        loop {
            let etex_protected_fetch = self.command.profile().capabilities().supports_etex();
            let mut destination = None;
            let result = if etex_protected_fetch {
                self.etex_alignment_lookahead_next(&mut destination)
            } else {
                self.tex_alignment_lookahead_next(&mut destination)
            };
            let lookahead = match result? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => AlignmentLookahead::Committed(
                    destination.expect("command status initializes destination"),
                ),
                DeliveryStatus::PendingExpanded => AlignmentLookahead::PendingExpanded(
                    destination.expect("pending status initializes destination"),
                ),
                _ => unreachable!("alignment lookahead consumes replay completions"),
            };
            if matches!(
                lookahead.command().meaning(),
                ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                let _ = self.commit_alignment_lookahead_delivery(lookahead);
                continue;
            }
            return Ok(Some(lookahead));
        }
    }

    /// Commits a terminal TeX82 lookahead delivery that alignment control
    /// consumes instead of passing to an ordinary `back_input` branch.
    pub fn commit_alignment_lookahead_delivery(
        &mut self,
        lookahead: AlignmentLookahead<G>,
    ) -> CurrentCommand<G> {
        match lookahead {
            AlignmentLookahead::Committed(command) => command,
            AlignmentLookahead::PendingExpanded(command) => {
                self.observe_expanded_delivery(&command);
                command
            }
        }
    }

    /// Completes TeX82 §§785/791's ordinary `align_peek`/`init_col` branch.
    ///
    /// A command reached through §380's expansion loop is still pending only
    /// in Umber's observer transport. TeX has already completed
    /// `get_x_token`, so its expanded delivery precedes §789's `back_input`;
    /// the later replay above the u-template is a distinct delivery.
    pub fn back_alignment_lookahead(
        &mut self,
        lookahead: AlignmentLookahead<G>,
    ) -> Result<(), CommandError> {
        let command = self.commit_alignment_lookahead_delivery(lookahead);
        self.back_input(command)
    }

    /// Delivers replay-aware expanded input into caller-provided storage.
    pub fn get_x_token_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let result = self.expanded_next_with_replay_completion(destination)?;
            match result {
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::End
                | DeliveryStatus::Command
                | DeliveryStatus::ReplayCompleted(_) => return Ok(result),
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn fail_hot_expanded_delivery<R>(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        depth: u32,
        failure: CommandError,
    ) -> Result<R, CommandError> {
        destination.take();
        self.command.transient.active_expansion_depth = depth;
        Err(failure)
    }
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// Settles the persistent semantic conditions and token-local facts
    /// represented by one delivered command without widening the ordinary hot
    /// loops.
    #[cold]
    #[inline(never)]
    fn settle_exceptional_delivery<const OBSERVED: bool>(
        &mut self,
        command: &mut CurrentCommand<G>,
        literal_catcode: Option<Catcode>,
        suppresses_expandable_control_sequence: bool,
        outer: bool,
    ) -> Result<(), CommandError> {
        // TeX82 §§342/789 publishes a source position at get_next exit;
        // intercepted delimiters keep the prior exited command's position.
        let previous_location = self.command.last_diagnostic_location;
        self.update_delivered_source_location(command);
        self.command
            .roots
            .alignment
            .account_literal_brace(command, literal_catcode);
        if OBSERVED {
            self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
        }
        let mode = self.command.delivery_mode;
        if suppresses_expandable_control_sequence {
            command.suppress_expandable();
        }
        let settlement = if mode.scanner_active() && outer {
            self.check_outer_validity_entry(command)
        } else {
            if mode.alignment_active()
                && matches!(
                    command.alignment_adjustment(),
                    super::AlignmentDeliveryAdjustment::None
                )
            {
                self.command.roots.alignment.classify_delimiter(command);
            }
            Ok(())
        };
        if settlement.is_ok() && OBSERVED {
            self.observe_resident_hot_command(command);
        }
        if matches!(
            command.alignment_adjustment(),
            super::AlignmentDeliveryAdjustment::Delimiter(_)
        ) {
            self.command.set_last_diagnostic_location(previous_location);
        }
        settlement
    }

    #[inline(always)]
    fn finish_expanded_command<const OBSERVED: bool>(
        &mut self,
        command: &CurrentCommand<G>,
        delivery_expanded: bool,
    ) -> DeliveryStatus {
        #[cfg(feature = "profiling")]
        self.record_expanded_delivery();
        if OBSERVED {
            self.observe_expanded_delivery(command);
        }
        if self
            .command
            .alignment
            .needs_hot_closing_brace_recovery(command)
        {
            DeliveryStatus::AlignmentClosingBrace
        } else if delivery_expanded {
            DeliveryStatus::PendingExpanded
        } else {
            DeliveryStatus::Command
        }
    }

    /// Compact observation counterpart for the scanner-owned expanded
    /// delivery.  The terminal command remains in the hot slot while its
    /// canonical identity, spelling, and provenance are projected into the
    /// observer record.
    #[doc(hidden)]
    pub fn observe_expanded_delivery(&mut self, command: &CurrentCommand<G>) {
        observe!(self, {
            #[cfg(test)]
            {}
            let meaning = command.meaning();
            let (command_name, command_operand) =
                crate::observation::canonical_delivery_identity_for_profile(
                    self.command.profile(),
                    command.identity(),
                    meaning,
                );
            let spelling = self.observed_command_spelling(command);
            let semantic_operand = crate::observation::canonical_sparse_register_operand(
                self.command.profile(),
                meaning,
            );
            CommandObservation::Command(CommandDeliveryRecord {
                boundary: CommandDeliveryBoundary::Expanded,
                spelling,
                command: command_name,
                command_operand,
                semantic_operand,
                provenance: CommandProvenance::new(
                    self.observed_delivery_coordinate(),
                    self.current_delivery_sequence(),
                    command.origin(),
                    self.direct_source_provenance(command),
                ),
            })
        });
    }

    /// TeX82 §375's ``@<Insert a token containing |frozen_endv|@>``:
    ///
    /// ```text
    /// begin cur_tok:=cs_token_flag+frozen_endv; back_input;
    /// end
    /// ```
    ///
    /// This is §366 `expand`'s entire `end_template` case, and the reason
    /// §780 installs *two* frozen `\endtemplate` control sequences: the one
    /// stored in a template (`frozen_end_template`, command code
    /// `end_template`) is `>outer_call`, so §336's `check_outer_validity`
    /// still catches a template that ends inside an unfinished scan, and only
    /// once it has been delivered is it replaced by `frozen_endv`, whose
    /// command code is the ordinary unexpandable `endv`.
    ///
    /// §325's stack-conservation loop stops at a `v_template` level, so the
    /// exhausted template stays on the stack underneath this backup and
    /// retires only after `endv` has been acted on.
    pub(crate) fn insert_frozen_endv(&mut self) -> Result<(), CommandError> {
        let frozen_endv = self.state.frozen_endv_token();
        self.back_input_token(TracedTokenWord::pack(frozen_endv, OriginId::UNKNOWN))
    }

    /// Executes one already-classified conditional while the expanded loop's
    /// caller-owned hot slot is empty. The conditional stack retains the
    /// semantic opener state; no token, provenance, or rich command crosses
    /// the recursive operand scan.
    #[inline(never)]
    fn expand_conditional_occupied(
        &mut self,
        _primitive: ExpandablePrimitive,
        kind: crate::conditionals::ConditionalKind,
    ) -> Result<(), CommandError> {
        #[cfg(feature = "profiling")]
        {
            tex_state::measurement::record_hot_core_expandable_opcode(
                usize::try_from(_primitive.operand())
                    .expect("expandable primitive operand fits usize"),
            );
            if self.write_expansion_depth != 0 {
                self.record_write_expansion();
            }
        }
        let result = self.expand_conditional_primitive(kind, false);
        if result
            .as_ref()
            .is_err_and(CommandError::is_resource_suspension)
        {
            let error = result.expect_err("matched resource need");
            self.command
                .scratch
                .unwind_resource_failure()
                .map_err(crate::scan_toks::scratch_command_error)?;
            return Err(error);
        }
        result
    }

    /// Handles the one expandable branch that is not represented by an
    /// `ExpandablePrimitive`. Undefined recovery keeps its compact spelling
    /// until the diagnostic site is captured; it is otherwise just another
    /// continue case of the ordinary expanded loop.
    #[inline(always)]
    fn expand_undefined_hot(
        &mut self,
        command: &CurrentCommand<G>,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        self.with_expansion_depth(|processor| {
            processor.expand_undefined_hot_in(command, report_trace)
        })
    }

    #[inline(never)]
    fn expand_undefined_hot_in(
        &mut self,
        command: &CurrentCommand<G>,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        if report_trace
            && (self
                .state
                .int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS)
                > 1)
        {
            self.print_command_trace(crate::PrintCommand::from_current(command));
        }
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_undefined_expansion();
        let context = self.command.output_open_context(self.state);
        let site = Some(self.complete_diagnostic_site(self.capture_hot_diagnostic_site(command)));
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::UndefinedControlSequence { context, site });
        if !self.command.profile().capabilities().supports_etex() {
            self.observe_command_diagnostic("undefined_control_sequence", command);
        }
        Ok(())
    }
}

/// TeX82 §1038's raw-accepted set: `letter`, `other_char`, and `char_given`.
///
/// These are exactly the three commands §1034's inner loop can continue on
/// without expanding, so they are the only ones the lookahead delivers
/// straight out of `get_next`.
/// TeX82 §366's `cur_cmd>max_command` test for Umber's resolved command.
///
/// `Meaning::Undefined` normally represents §207's `undefined_cs` command,
/// which is expanded solely to perform §370's diagnostic recovery. A compact
/// out-parameter token also carries that meaning as its invalid-slot recovery,
/// but its command remains `out_param<max_command`; its token spelling keeps
/// the two command identities distinct here.
pub(crate) fn is_expandable_command<G>(command: &CurrentCommand<G>) -> bool {
    let meaning = &command.meaning();
    matches!(meaning, ResolvedMeaning::Macro { .. })
        || matches!(meaning, ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive)) if *primitive != ExpandablePrimitive::EndCsName)
        || (matches!(meaning, ResolvedMeaning::Static(Meaning::Undefined))
            && !matches!(command.spelling().semantic_token(), Token::Param(_)))
}

#[cfg(test)]
mod tests;
