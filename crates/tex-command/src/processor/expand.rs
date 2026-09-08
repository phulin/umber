//! Fused resident-input advancement and raw/expanded command delivery.

mod collect;
mod consume;
mod input;
mod resident;

use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, ResolvedMeaning};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use crate::command::{CommandClass, DeliveryStamp, HotCommand};
use crate::execution_scratch::ArgumentSetId;
use crate::input::{InputLevelId, ResidentBoundary};
use crate::{CommandError, CommandReplayDelivery, CurrentCommand};

use super::expand_render::format_pdf_date;
use super::{
    AlignmentLookahead, CommandProcessor, DeliveryStatus, MainCharacterConsumer, MainCharacterInput,
};

use crate::observation::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandProvenance,
};

enum ResidentColdOutcome {
    Retry,
    Finished(DeliveryStatus),
    Word(ResidentWord),
}

#[derive(Clone, Copy)]
enum ExpandedUntilMode {
    Protected,
    PreserveUndefined,
}
#[cfg(test)]
#[derive(Clone, Copy)]
enum ResidentStorageKind {
    Stored,
    MacroBody,
    MacroArgument,
    Source,
    Synthetic,
}
enum InputFrameTransition<G> {
    Boundary(ResidentBoundary),
    ResidentExhausted {
        resident_index: usize,
        identity: InputLevelId,
    },
    Parameter {
        slot: u8,
        arguments: Option<ArgumentSetId<G>>,
        active_source: Option<tex_state::packed_input::SourceContext>,
    },
}

/// One packed word selected from a resident token row. The selection keeps
/// the row's already-admitted coordinates beside the word so the delivery
/// loops can either resolve it into the hot command or consume an ordinary
/// character directly in the main-control run.
enum ResidentWordRead<G> {
    NoResident,
    Source {
        resident_index: usize,
    },
    Parameter {
        slot: u8,
        arguments: Option<ArgumentSetId<G>>,
        active_source: Option<tex_state::packed_input::SourceContext>,
    },
    Exhausted {
        resident_index: usize,
        identity: InputLevelId,
    },
    Word(ResidentWord),
}

/// Successfully loaded token and its frame-local delivery facts. The writer
/// accepts only this occupied read, so resolution has no input-status branch.
#[derive(Clone, Copy)]
struct ResidentWord {
    word: TokenWord,
    origin: OriginId,
    identity: u64,
    position: u64,
    active_source: Option<tex_state::packed_input::SourceContext>,
    suppress_expandable: bool,
    site: ReadSite,
    #[cfg(test)]
    storage_kind: ResidentStorageKind,
    #[cfg(feature = "profiling")]
    raw_kind: crate::fuel::RawDeliveryKind,
}

#[derive(Clone, Copy)]
enum ReadSite {
    Resident,
    Source(Option<u32>),
    Synthetic,
}

fn static_meaning<G>(meaning: &ResolvedMeaning<G>) -> Option<Meaning> {
    match meaning {
        ResolvedMeaning::Static(meaning) => Some(*meaning),
        ResolvedMeaning::Macro { .. } => None,
    }
}

/// The one decision TeX.web §380 makes after raw delivery has resolved the
/// current meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpandedCommandAction {
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
fn classify_hot_command<G>(command: &HotCommand<G>) -> ExpandedCommandAction {
    #[cfg(test)]
    EXPANDED_CLASSIFICATIONS.with(|counter| counter.set(counter.get().saturating_add(1)));

    let word = command.command_word();
    match word.class() {
        CommandClass::Macro => ExpandedCommandAction::Expand(ExpansionDispatch::Macro),
        CommandClass::Expandable => match word.expandable_primitive() {
            Some(ExpandablePrimitive::EndTemplate) => ExpandedCommandAction::EndTemplate,
            Some(ExpandablePrimitive::EndCsName) => ExpandedCommandAction::Return,
            Some(primitive) => {
                ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive))
            }
            None => ExpandedCommandAction::Return,
        },
        CommandClass::Undefined if command.spelling_word().out_parameter_slot().is_none() => {
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
        }
        _ => ExpandedCommandAction::Return,
    }
}

#[inline(always)]
fn hot_decimal_digit<G>(command: &HotCommand<G>) -> Option<u8> {
    match command.command_word().static_meaning() {
        Some(Meaning::CharToken {
            ch: ch @ '0'..='9',
            cat: Catcode::Other,
        }) => Some(ch as u8 - b'0'),
        _ => None,
    }
}

#[inline(always)]
fn hot_is_space<G>(command: &HotCommand<G>) -> bool {
    matches!(
        command.command_word().static_meaning(),
        Some(Meaning::CharToken {
            cat: Catcode::Space,
            ..
        })
    )
}

impl<G> CommandProcessor<'_, '_, G> {
    #[inline(always)]
    fn write_resident_word(
        &mut self,
        selected: ResidentWord,
        destination: &mut Option<HotCommand<G>>,
    ) -> Option<Catcode> {
        let ResidentWord {
            word,
            origin,
            identity,
            position,
            active_source,
            suppress_expandable,
            site,
            #[cfg(test)]
            storage_kind,
            #[cfg(feature = "profiling")]
            raw_kind,
        } = selected;
        #[cfg(test)]
        match storage_kind {
            ResidentStorageKind::Stored => {
                self.command.stored_token_advance_counters.command_writes = self
                    .command
                    .stored_token_advance_counters
                    .command_writes
                    .saturating_add(1);
                self.command.raw_delivery_path_counters.stored_direct = self
                    .command
                    .raw_delivery_path_counters
                    .stored_direct
                    .saturating_add(1);
            }
            ResidentStorageKind::MacroBody => {
                self.command.macro_kernel_counters.body_command_writes = self
                    .command
                    .macro_kernel_counters
                    .body_command_writes
                    .saturating_add(1);
            }
            ResidentStorageKind::MacroArgument => {
                self.command.macro_kernel_counters.argument_command_writes = self
                    .command
                    .macro_kernel_counters
                    .argument_command_writes
                    .saturating_add(1);
                self.command
                    .raw_delivery_path_counters
                    .macro_argument_direct = self
                    .command
                    .raw_delivery_path_counters
                    .macro_argument_direct
                    .saturating_add(1);
            }
            ResidentStorageKind::Source | ResidentStorageKind::Synthetic => {}
        }
        let direct_source_line = match site {
            ReadSite::Source(line) => line,
            _ => None,
        };
        let resolution = HotCommand::write_delivery_into(
            destination,
            word,
            origin,
            identity,
            position,
            active_source,
            matches!(site, ReadSite::Source(_)),
            direct_source_line,
            suppress_expandable,
            self.state,
        );
        #[cfg(test)]
        if matches!(storage_kind, ResidentStorageKind::Stored) {
            self.command.stored_token_advance_counters.meaning_lookups = self
                .command
                .stored_token_advance_counters
                .meaning_lookups
                .saturating_add(u64::from(resolution.meaning_lookup()));
        }
        #[cfg(feature = "profiling")]
        self.fuel.record_raw_delivery(
            self.command.delivery_mode.scanner_active(),
            resolution.meaning_lookup(),
            raw_kind,
        );
        match site {
            ReadSite::Resident => self.enter_resident_delivery(),
            _ => self.readmit_delivery_stamp(
                destination
                    .as_ref()
                    .expect("initialized delivery")
                    .delivery_stamp(),
            ),
        }
        resolution.literal_catcode()
    }

    #[inline(always)]
    fn settle_hot_delivery(
        &mut self,
        command: &mut HotCommand<G>,
        literal_catcode: Option<Catcode>,
    ) -> Result<(), CommandError> {
        if self.is_observed() {
            self.settle_hot_delivery_in::<true>(command, literal_catcode)
        } else {
            self.settle_hot_delivery_in::<false>(command, literal_catcode)
        }
    }

    #[inline(always)]
    fn settle_hot_delivery_in<const OBSERVED: bool>(
        &mut self,
        command: &mut HotCommand<G>,
        literal_catcode: Option<Catcode>,
    ) -> Result<(), CommandError> {
        // These are token-local facts from the already-decoded compact
        // command. Keep them at the same point where the old token bits were
        // installed; exceptional settlement may rewrite the command.
        let suppresses_expandable_control_sequence =
            command.suppresses_expandable_control_sequence();
        let outer = command.is_outer();
        self.command.roots.alignment.account_literal_brace(
            &mut self.command.timeline,
            command,
            literal_catcode,
        );
        if OBSERVED {
            self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
        }
        if OBSERVED
            || self
                .command
                .delivery_mode
                .requires_semantic_settlement(suppresses_expandable_control_sequence, outer)
        {
            self.settle_exceptional_delivery::<OBSERVED>(
                command,
                suppresses_expandable_control_sequence,
                outer,
            )?;
        }
        Ok(())
    }

    #[inline(always)]
    fn transition_resident_word(
        &mut self,
        selected: ResidentWordRead<G>,
        create_control_sequences: bool,
    ) -> Result<ResidentColdOutcome, CommandError> {
        let transition = match selected {
            ResidentWordRead::NoResident => InputFrameTransition::Boundary(ResidentBoundary::Empty),
            ResidentWordRead::Source { resident_index } => {
                self.invalidate_delivery_freshness();
                return self.advance_source_token(resident_index, create_control_sequences);
            }
            ResidentWordRead::Parameter {
                slot,
                arguments,
                active_source,
            } => {
                self.transition_input_frame(InputFrameTransition::Parameter {
                    slot,
                    arguments,
                    active_source,
                })?;
                return Ok(ResidentColdOutcome::Retry);
            }
            ResidentWordRead::Exhausted {
                resident_index,
                identity,
            } => InputFrameTransition::ResidentExhausted {
                resident_index,
                identity,
            },
            ResidentWordRead::Word(_) => {
                return Err(CommandError::input_invariant());
            }
        };
        let outcome = self.transition_input_frame(transition)?;
        Ok(outcome)
    }

    /// The concrete TeX82 §341 raw-token loop. It owns one fuel charge for
    /// each semantic raw token and retries only through cold input transitions.
    #[inline(always)]
    pub(super) fn raw_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut hot_destination = None;
        let result = self.raw_next_hot(&mut hot_destination);
        self.finish_hot_delivery(destination, &mut hot_destination, result)
    }

    #[inline(always)]
    pub(crate) fn raw_next_hot(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
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
        destination: &mut Option<HotCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let literal_catcode = match self.read_raw_word(self.create_source_control_sequences)? {
            ResidentColdOutcome::Word(word) => self.write_resident_word(word, destination),
            ResidentColdOutcome::Finished(status) => {
                destination.take();
                return Ok(status);
            }
            ResidentColdOutcome::Retry => unreachable!("raw reading settles input transitions"),
        };
        self.settle_hot_delivery_in::<OBSERVED>(
            destination.as_mut().expect("initialized delivery"),
            literal_catcode,
        )?;
        Ok(DeliveryStatus::Command)
    }

    /// Reads one semantic raw token. All consumers share this charge and
    /// retry boundary; reading alone never constructs or resolves a command.
    #[inline(always)]
    fn read_raw_word(
        &mut self,
        create_control_sequences: bool,
    ) -> Result<ResidentColdOutcome, CommandError> {
        self.charge_command_action()?;
        loop {
            let selected = self.read_resident_word();
            if let ResidentWordRead::Word(word) = selected {
                return Ok(ResidentColdOutcome::Word(word));
            }
            match self.transition_resident_word(selected, create_control_sequences)? {
                ResidentColdOutcome::Retry => {}
                outcome => return Ok(outcome),
            }
        }
    }

    /// Delivers one expanded command through the compact loop and materializes
    /// only at the caller's rich-command boundary. The scanner-owned callers
    /// use the hot entry directly, so a terminal delimiter operand never
    /// crosses this boundary merely to be classified.
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
        let mut hot_destination = destination.take().map(HotCommand::from_current);
        let result = self.expanded_next_hot(&mut hot_destination, initial_action);
        self.finish_hot_delivery(destination, &mut hot_destination, result)
    }

    fn finish_hot_delivery(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        hot_destination: &mut Option<HotCommand<G>>,
        result: Result<DeliveryStatus, CommandError>,
    ) -> Result<DeliveryStatus, CommandError> {
        match result {
            Ok(status) => {
                if matches!(
                    status,
                    DeliveryStatus::End
                        | DeliveryStatus::ReplayCompleted(_)
                        | DeliveryStatus::CharacterRun
                ) {
                    hot_destination.take();
                } else {
                    *destination = hot_destination.take().map(|command| command.materialize());
                }
                Ok(status)
            }
            Err(error) => {
                hot_destination.take();
                destination.take();
                Err(error)
            }
        }
    }

    /// TeX82 §380's synchronous expanded delivery. Observation is fixed for
    /// this call, while scanner and alignment semantics remain live. This
    /// wrapper owns depth and failure cleanup outside the expansion back edge.
    fn expanded_next_hot(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
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
        let result = if self.is_observed() {
            self.expanded_delivery_loop::<true>(destination, initial_action)
        } else {
            self.expanded_delivery_loop::<false>(destination, initial_action)
        };
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
        command: &mut HotCommand<G>,
        action: ExpansionDispatch,
    ) -> Result<(), CommandError> {
        match action {
            ExpansionDispatch::Macro | ExpansionDispatch::Undefined => {
                self.expand_hot_definition_action(command, action)
            }
            ExpansionDispatch::Primitive(primitive) => {
                if let Some(kind) = crate::conditionals::ConditionalKind::from_primitive(primitive)
                {
                    self.with_expansion_depth(|processor| {
                        processor.trace_hot_conditional(command);
                        processor.expand_conditional_occupied(primitive, kind)
                    })
                } else {
                    self.expand_compact_occupied(command, primitive, true)
                }
            }
        }
    }

    /// Keeps macro-definition and undefined-control-sequence handling out of
    /// the ordinary conditional recursion frame. These branches share the
    /// command ownership cleanup but are not part of the scalar hot path, so
    /// their error storage lives in this out-of-line helper.
    #[inline(never)]
    fn expand_hot_definition_action(
        &mut self,
        hot_command: &mut HotCommand<G>,
        action: ExpansionDispatch,
    ) -> Result<(), CommandError> {
        match action {
            ExpansionDispatch::Macro => {
                self.record_macro_expansion();
                if let Err(failure) = self.macro_call_hot(hot_command).map(|_| ()) {
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

    /// Completes a source or synthetic `endv` command after the main-loop
    /// reader has crossed a cold input boundary. Such a command still needs
    /// the ordinary delivery settlement, but it never belongs to the warm
    /// character-run body.
    #[cold]
    #[inline(never)]
    fn finish_main_loop_synthetic(
        &mut self,
        command: &mut Option<HotCommand<G>>,
        literal_catcode: Option<Catcode>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        if let Err(failure) = self.fuel.charge() {
            self.invalidate_delivery_freshness();
            return Err(failure);
        }
        let mut command = command.take().ok_or_else(CommandError::input_invariant)?;
        if let Err(failure) = self.settle_hot_delivery(&mut command, literal_catcode) {
            self.invalidate_delivery_freshness();
            return Err(failure);
        }
        *destination = Some(command.materialize());
        Ok(DeliveryStatus::CharacterRunBoundary)
    }

    #[cold]
    #[inline(never)]
    fn finish_main_cold_transition(
        &mut self,
        cold: ResidentColdOutcome,
        command: &mut Option<HotCommand<G>>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<Option<DeliveryStatus>, CommandError> {
        match cold {
            ResidentColdOutcome::Retry => Ok(None),
            ResidentColdOutcome::Finished(status) => Ok(Some(status)),
            ResidentColdOutcome::Word(word) => {
                let literal_catcode = self.write_resident_word(word, command);
                self.finish_main_loop_synthetic(command, literal_catcode, destination)
                    .map(Some)
            }
        }
    }

    /// Consumes the direct ordinary-character prefix owned by main control.
    /// The consumer is mandatory here; no other delivery loop carries it.
    #[inline(always)]
    pub(super) fn main_character_run(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        consume: &mut impl MainCharacterConsumer<G>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        self.invalidate_delivery_freshness();
        let mut command = None;

        let mut consumed_characters = false;
        #[cfg(feature = "profiling")]
        let mut character_run_count = 0_u32;
        #[cfg(feature = "profiling")]
        let mut character_run_kind = None;

        loop {
            let allows_characters = self.command.delivery_mode.allows_character_run();
            #[cfg(feature = "profiling")]
            let raw_kind = match self
                .command
                .roots
                .input
                .levels
                .rows
                .get(self.command.roots.input.levels.top.wrapping_sub(1))
            {
                Some(crate::input::InputLevel::Resident(row))
                    if matches!(
                        row.storage,
                        crate::input::ResidentTokenStorage::MacroArgument(_)
                    ) =>
                {
                    crate::fuel::RawDeliveryKind::MacroArgument
                }
                _ => crate::fuel::RawDeliveryKind::StoredToken,
            };
            let selected = Self::read_resident_run(self.command, |word, origin| {
                use resident::ResidentAdmission;
                let Token::Char {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                } = word.semantic_token()
                else {
                    return Ok(ResidentAdmission::Boundary);
                };
                if !allows_characters {
                    return Ok(ResidentAdmission::Boundary);
                }
                self.fuel.charge()?;
                consumed_characters = true;
                #[cfg(feature = "profiling")]
                {
                    character_run_kind = Some(raw_kind);
                    character_run_count = character_run_count.saturating_add(1);
                }
                let admission = consume.admit(
                    self.state,
                    self.fuel,
                    self.diagnostic_effects,
                    MainCharacterInput::Scalar { ch, origin },
                );
                Ok(if admission.continue_run() {
                    ResidentAdmission::Continue
                } else {
                    ResidentAdmission::Stop
                })
            });
            let selected = match selected {
                Ok(std::ops::ControlFlow::Continue(selected)) => selected,
                Ok(std::ops::ControlFlow::Break(())) => {
                    self.invalidate_delivery_freshness();
                    #[cfg(feature = "profiling")]
                    if let Some(kind) = character_run_kind.take() {
                        self.fuel.record_raw_run(false, kind, character_run_count);
                    }
                    return Ok(DeliveryStatus::CharacterRun);
                }
                Err(failure) => {
                    self.invalidate_delivery_freshness();
                    return Err(failure);
                }
            };
            if matches!(selected, ResidentWordRead::NoResident) {
                if consumed_characters {
                    self.invalidate_delivery_freshness();
                    #[cfg(feature = "profiling")]
                    if let Some(kind) = character_run_kind.take() {
                        self.fuel.record_raw_run(false, kind, character_run_count);
                    }
                    return Ok(DeliveryStatus::CharacterRun);
                }
                let cold = self.transition_resident_word(
                    ResidentWordRead::NoResident,
                    self.create_source_control_sequences,
                )?;
                if let Some(status) =
                    self.finish_main_cold_transition(cold, &mut command, destination)?
                {
                    return Ok(status);
                }

                continue;
            }

            if let ResidentWordRead::Source { resident_index } = selected {
                #[cfg(test)]
                {
                    self.command
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .source_branch_entries = self
                        .command
                        .roots
                        .input
                        .levels
                        .cursor_mutations
                        .source_branch_entries
                        .saturating_add(1);
                }
                if self
                    .advance_source_character_step(resident_index, consume)?
                    .is_some()
                {
                    // The source cursor and its provenance moved in place;
                    // no resident command can retain the previous
                    // freshness proof across this direct admission.
                    self.invalidate_delivery_freshness();
                    return Ok(DeliveryStatus::CharacterRun);
                }
                let cold = self.transition_resident_word(
                    ResidentWordRead::Source { resident_index },
                    self.create_source_control_sequences,
                )?;
                if let Some(status) =
                    self.finish_main_cold_transition(cold, &mut command, destination)?
                {
                    return Ok(status);
                }

                continue;
            }
            if !matches!(selected, ResidentWordRead::Word(_)) {
                if consumed_characters && matches!(selected, ResidentWordRead::Exhausted { .. }) {
                    self.invalidate_delivery_freshness();
                    #[cfg(feature = "profiling")]
                    if let Some(kind) = character_run_kind.take() {
                        self.fuel.record_raw_run(false, kind, character_run_count);
                    }
                    return Ok(DeliveryStatus::CharacterRun);
                }
                let cold =
                    self.transition_resident_word(selected, self.create_source_control_sequences)?;
                if let Some(status) =
                    self.finish_main_cold_transition(cold, &mut command, destination)?
                {
                    return Ok(status);
                }

                continue;
            }
            let ResidentWordRead::Word(selected) = selected else {
                unreachable!("input transitions were handled above")
            };
            if let Err(failure) = self.fuel.charge() {
                self.invalidate_delivery_freshness();
                return Err(failure);
            }
            #[cfg(feature = "profiling")]
            if let Some(kind) = character_run_kind.take() {
                self.fuel.record_raw_run(false, kind, character_run_count);
            }
            let literal_catcode = self.write_resident_word(selected, &mut command);
            let mut command = command
                .take()
                .expect("boundary word initialized its command");
            if let Err(failure) = self.settle_hot_delivery(&mut command, literal_catcode) {
                self.invalidate_delivery_freshness();
                return Err(failure);
            }
            *destination = Some(command.materialize());
            return Ok(DeliveryStatus::CharacterRunBoundary);
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
        self.expanded_until(destination, ExpandedUntilMode::Protected)
    }

    #[cold]
    #[inline(never)]
    fn expanded_until(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        mode: ExpandedUntilMode,
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
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("protected delivery has no character consumer")
                }
            }

            let command = destination
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            let stop = match mode {
                ExpandedUntilMode::Protected => {
                    matches!(
                        command.meaning_ref(),
                        ResolvedMeaning::Macro { flags, .. }
                            if flags.contains(MeaningFlags::PROTECTED)
                    ) || !is_expandable_command(command)
                }
                ExpandedUntilMode::PreserveUndefined => matches!(
                    command.meaning_ref(),
                    ResolvedMeaning::Static(Meaning::Undefined)
                ),
            };
            if stop {
                return Ok(DeliveryStatus::Command);
            }
            match self.expanded_next(destination)? {
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
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("protected delivery has no character consumer")
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
        self.expanded_until(destination, ExpandedUntilMode::PreserveUndefined)
    }

    fn x_token_next_with_action(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut initial_action = initial_action;
        if destination.as_ref().is_some_and(|command| {
            matches!(
                command.meaning_ref(),
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

    /// Main-control lookahead first returns a raw character without expansion;
    /// non-character commands continue through the x-token entry.
    #[cold]
    #[inline(never)]
    pub(super) fn main_loop_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.main_action_lookahead(destination, false)
    }

    #[cold]
    #[inline(never)]
    fn main_action_lookahead(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        preflight: bool,
    ) -> Result<DeliveryStatus, CommandError> {
        let existing = destination.is_some();
        let mut hot_destination = if existing {
            destination.as_ref().map(HotCommand::from_current_ref)
        } else {
            None
        };
        if hot_destination.is_none() {
            match self.raw_next_hot(&mut hot_destination)? {
                DeliveryStatus::Command => {}
                status => return Ok(status),
            }
        }
        let hot = hot_destination
            .as_ref()
            .ok_or_else(CommandError::input_invariant)?;
        let is_character = hot.command_word().is_main_loop_character();
        if is_character && !preflight {
            if !existing {
                *destination = hot_destination.take().map(|command| command.materialize());
            }
            return Ok(DeliveryStatus::Command);
        }
        let action = classify_hot_command(hot);
        if matches!(action, ExpandedCommandAction::EndTemplate) {
            if matches!(
                hot.alignment_adjustment(),
                crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
            ) {
                if !existing {
                    *destination = hot_destination.take().map(|command| command.materialize());
                }
                return Ok(DeliveryStatus::AlignmentEndTemplate);
            }
            hot_destination.take();
            destination.take();
            self.insert_frozen_endv()?;
            let result = self.expanded_next_hot(&mut hot_destination, None);
            return self.finish_hot_delivery(destination, &mut hot_destination, result);
        }
        if is_character || matches!(action, ExpandedCommandAction::Return) {
            if existing {
                self.observe_expanded_delivery(
                    destination
                        .as_ref()
                        .ok_or_else(CommandError::input_invariant)?,
                );
            } else {
                let command = hot_destination
                    .take()
                    .ok_or_else(CommandError::input_invariant)?
                    .materialize();
                self.observe_expanded_delivery(&command);
                *destination = Some(command);
            }
            return Ok(DeliveryStatus::Command);
        }
        let result = self.expanded_next_hot(&mut hot_destination, Some(action));
        self.finish_hot_delivery(destination, &mut hot_destination, result)
    }

    /// Main-control preflight owns its first raw fetch and then continues from
    /// that resident command through ordinary expansion.
    #[cold]
    #[inline(never)]
    pub(super) fn preflight_next(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.main_action_lookahead(destination, true)? {
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                result => return Ok(result),
            }
        }
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
        match self.main_loop_next(destination)? {
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

    /// Delivers one ordinary expanded command through TeX.web's `get_x_token`.
    ///
    /// This thin canonical entry point enters the ordinary expanded loop.
    /// Expansion mutates canonical command state and restarts in that loop;
    /// it never returns a push-bearing dispatch result or enters a second
    /// interpreter.
    pub fn get_x_token(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.get_x_token_into(&mut destination)? {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary expanded delivery returns only commands"),
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
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("expanded delivery does not own a character consumer")
                }
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

    /// Requests one expanded token while retaining the delivery loop's
    /// compact owner. Synchronous collectors use this entry when their
    /// ordinary body only needs the packed command class and spelling; the
    /// rich command remains reserved for an actual scanner or recovery
    /// boundary. Alignment template admission is such a boundary and keeps
    /// the established rich handoff.
    pub(crate) fn request_expanded_hot_token(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            match self.expanded_next_hot(destination, None)? {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::AlignmentEndTemplate => {
                    self.finish_alignment_end_template_hot(destination)?;
                }
                DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                    return Ok(DeliveryStatus::Command);
                }
                status @ (DeliveryStatus::End | DeliveryStatus::Command) => return Ok(status),
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    return Err(CommandError::input_invariant());
                }
            }
        }
    }

    /// Finishes the alignment-only rich boundary of
    /// [`Self::request_expanded_hot_token`]. Ordinary scalar requests keep
    /// their compact destination through the shared expanded loop; only the
    /// active-cell delimiter needs a `CurrentCommand` for the existing
    /// freshness and delimiter-line checks.
    #[cold]
    #[inline(never)]
    fn finish_alignment_end_template_hot(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
    ) -> Result<(), CommandError> {
        let command = destination
            .take()
            .ok_or_else(CommandError::input_invariant)?
            .materialize();
        self.begin_scalar_alignment_v_template(&command)
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
            .map(HotCommand::from_current)
            .ok_or_else(CommandError::input_invariant)?;
        let action = classify_hot_command(&command);
        let result = match action {
            ExpandedCommandAction::Return => Err(CommandError::input_invariant()),
            ExpandedCommandAction::EndTemplate => self.expand_compact_occupied(
                &mut command,
                ExpandablePrimitive::EndTemplate,
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
                self.with_expansion_depth(|processor| {
                    processor.macro_call_hot(&mut command).map(|_| ())
                })
            }
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined) => {
                self.expand_undefined_hot(&command, report_trace)
            }
            ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive)) => {
                self.expand_compact_occupied(&mut command, primitive, report_trace)
            }
        };
        if result.is_err() {
            // Keep the ordinary rich boundary's recovery contract for
            // callers that inspect or clear the opener after a failed
            // expansion. Successful expansion consumes it below.
            *destination = Some(command.materialize());
            return result;
        }
        // The expanded opener is consumed by the nested call and cannot
        // become the scanner's next operand.
        Ok(())
    }

    /// Expands one already-delivered hot command in place. This is the
    /// collector counterpart to [`Self::request_expansion_into`]: successful
    /// expansion consumes the opener, while an error leaves the compact
    /// owner available to the caller's existing recovery/unwind path.
    pub(crate) fn request_expansion_hot(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        let mut command = destination
            .take()
            .ok_or_else(CommandError::input_invariant)?;
        let action = classify_hot_command(&command);
        let result = match action {
            ExpandedCommandAction::Return => Err(CommandError::input_invariant()),
            ExpandedCommandAction::EndTemplate => self.expand_compact_occupied(
                &mut command,
                ExpandablePrimitive::EndTemplate,
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
                self.with_expansion_depth(|processor| {
                    processor.macro_call_hot(&mut command).map(|_| ())
                })
            }
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined) => {
                self.expand_undefined_hot(&command, report_trace)
            }
            ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive)) => {
                self.expand_compact_occupied(&mut command, primitive, report_trace)
            }
        };
        if result.is_err() {
            *destination = Some(command);
            return result;
        }
        Ok(())
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
    pub fn get_x_token_preserving_undefined(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        let result = self.expanded_next_preserving_undefined(&mut destination)?;
        match result {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary expanded delivery returns only commands"),
        }
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
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("active-character delivery has no character consumer")
                }
            }
        }
    }

    /// TeX82 §1152's `@<Treat |cur_chr| as an active character@>`:
    ///
    /// ```text
    /// begin cur_cs:=cur_chr+active_base;
    /// cur_cmd:=eq_type(cur_cs); cur_chr:=equiv(cur_cs);
    /// x_token; back_input;
    /// end
    /// ```
    ///
    /// This is the whole of TeX's `\mathcode` escape hatch. §1155's
    /// `set_math_char` and §1151's `scan_math` both branch here when a
    /// character's `math_code` is `@'100000`, which is what makes plain
    /// TeX's ``\mathcode`\'="8000`` route `'` through the active `'` macro
    /// that builds `\prime` lists.
    ///
    /// The character is not backed up and reread. §1152 loads the
    /// `active_base + c` cell's meaning straight into `cur_cmd`/`cur_chr`,
    /// so there is no raw delivery for it at all: `x_token` expands that
    /// meaning in place -- observing a macro push, not a backup -- and only
    /// the unexpandable token expansion settles on is backed up, from where
    /// the caller rereads it. An active character bound to an unexpandable
    /// meaning still reaches §381's tail, so it is still observed as one
    /// expanded delivery and backed up unchanged.
    pub fn treat_as_active_character(
        &mut self,
        ch: char,
        origin: OriginId,
    ) -> Result<(), CommandError> {
        let spelling = TracedTokenWord::pack(
            Token::Char {
                ch,
                cat: Catcode::Active,
            },
            origin,
        );
        let stamp = DeliveryStamp::new(0, 0);
        self.advance_delivery_sequence();
        let command = CurrentCommand::<G>::resolve(spelling, stamp, None, false, None, self.state);
        let mut destination = None;
        let status = self.active_x_token_into(command, &mut destination)?;
        let settled = match status {
            DeliveryStatus::End => return Ok(()),
            DeliveryStatus::Command => destination
                .take()
                .expect("command status initializes destination"),
            _ => unreachable!("active-character delivery normalizes to commands"),
        };
        // §325 needs only `cur_tok`; the settled token is `x_token`'s result
        // rather than a delivery this call is undoing, exactly as in §326.
        self.back_input_saved(settled)
    }

    /// TeX82 §404's `<Get the next non-blank non-relax non-call token>`:
    /// `repeat get_x_token until (cur_cmd<>spacer)and(cur_cmd<>relax)`.
    ///
    /// This is the shared spelling of that module, used by §403's
    /// `scan_left_brace`, §1078, §1084, §1151's `scan_math`, §1160's
    /// non-radical `scan_delimiter`, §1211's `prefixed_command`, §1226 and
    /// §1270's `scan_optional_equals`. It differs from §406's
    /// `<Get the next non-blank non-call token>` only by also skipping
    /// `\relax`, and the two are not interchangeable: §1160 classifies the
    /// token it stops on, so a `\relax` that reached it as a command rather
    /// than as a skipped filler would scan as an invalid delimiter.
    pub fn next_non_blank_non_relax_x_token(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        loop {
            match self.get_x_token_into(&mut destination)? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary expanded delivery returns only commands"),
            }
            let command = destination
                .as_ref()
                .expect("command status initializes destination");
            if !matches!(
                static_meaning(command.meaning_ref()),
                Some(
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } | Meaning::Relax
                )
            ) {
                return Ok(destination);
            }
            destination = None;
        }
    }

    /// TeX82 §404's expanded nonblank/non-relax fetch for scanners that can
    /// classify the terminal command directly from the compact delivery.
    ///
    /// The hot command is the sole result owner: it is overwritten on each
    /// fetch, and exactly the command that stops the loop remains in the
    /// caller's slot. In particular, no rich command is made merely to hand
    /// one delimiter operand from expansion to `scan_delimiter`.
    pub(crate) fn next_non_blank_non_relax_x_token_hot(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        loop {
            match self.expanded_next_hot(destination, None)? {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::End => return Ok(DeliveryStatus::End),
                DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentClosingBrace => {}
                DeliveryStatus::AlignmentEndTemplate => {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?
                        .materialize();
                    self.begin_scalar_alignment_v_template(&command)?;
                    continue;
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    return Err(CommandError::input_invariant());
                }
            }
            let command = destination
                .as_ref()
                .ok_or_else(CommandError::input_invariant)?;
            if !matches!(
                command.command_word().static_meaning(),
                Some(
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } | Meaning::Relax
                )
            ) {
                return Ok(DeliveryStatus::Command);
            }
            destination.take();
        }
    }

    /// TeX82 §406's `<Get the next non-blank non-call token>`:
    /// `repeat get_x_token until cur_cmd<>spacer`.
    ///
    /// Unlike §404's similarly named helper, this preserves `\relax`. The
    /// returned command is the exact expanded delivery that stopped the
    /// loop: callers such as §1045's `\ignorespaces` dispatch it in place
    /// without backing it up or rebuilding its provenance.
    pub fn next_non_blank_x_token(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        loop {
            match self.get_x_token_into(&mut destination)? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary expanded delivery returns only commands"),
            }
            let command = destination
                .as_ref()
                .expect("command status initializes destination");
            if !matches!(
                static_meaning(command.meaning_ref()),
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                return Ok(destination);
            }
            destination = None;
        }
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

    /// Delivers one expanded command or the completion of an executor-owned
    /// stored replay episode.
    ///
    /// Completion is published after the command machine has retired and
    /// observed the exact stored level, but before it resumes the enclosing
    /// source.  Callers must finish the corresponding isolated execution
    /// lifecycle before requesting another delivery.
    pub fn get_x_token_with_replay_completion(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let result = self.get_x_token_with_replay_completion_into(&mut destination)?;
        Ok(match result {
            DeliveryStatus::End => None,
            DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("ordinary replay-aware delivery has no alignment event"),
        })
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
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("expanded delivery does not own a character consumer")
                }
            }
        }
    }

    /// Delivers main-control preflight through one raw-fetch/classification
    /// loop. An ordinary unexpandable command publishes its canonical expanded
    /// observation directly, without completing a second expanded-driver
    /// episode; a macro, expandable primitive, or undefined command continues
    /// in place through the canonical expanded loop.
    pub fn preflight_command_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let result = self.preflight_next(destination)?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Delivers one command through TeX82 §1038's `main_loop_lookahead`.
    ///
    /// `main_control`'s inner character loop (§1034) never returns to
    /// `big_switch`'s `get_x_token` between adjacent characters. §1038 fetches
    /// the next command with a bare `get_next` -- "set only `cur_cmd` and
    /// `cur_chr`, for speed" -- and jumps straight back into the loop when
    /// that raw command is `letter`, `other_char`, or `char_given`. Only a
    /// raw command outside that set reaches `x_token`, which is the sole
    /// reason a run of ordinary characters produces one raw delivery each and
    /// no expanded delivery at all.
    ///
    /// `char_num` is deliberately *not* in the raw set: §1038 accepts it only
    /// after `x_token`, because `\char` can be reached by expansion.
    pub fn main_loop_lookahead(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let result = self.main_loop_lookahead_into(&mut destination)?;
        Ok(match result {
            DeliveryStatus::End => None,
            DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("main-loop lookahead has no alignment event"),
        })
    }

    /// Delivers main-loop lookahead into caller-provided command storage.
    pub fn main_loop_lookahead_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let result = self.main_loop_next(destination)?;
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
                | DeliveryStatus::ReplayCompleted(_) => {
                    return Ok(result);
                }
                DeliveryStatus::CharacterRun | DeliveryStatus::CharacterRunBoundary => {
                    unreachable!("main-loop lookahead has no character consumer")
                }
            }
        }
    }

    /// Lends one main-control source step to the direct list admission, then
    /// settles scalar input through the same owner when the borrowed prefix is
    /// unavailable.  The source row is selected once by `main_character_run`;
    /// this entry never probes it and re-enters a second delivery loop.
    pub fn main_loop_source_step_into<C: MainCharacterConsumer<G>>(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        consume: &mut C,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        debug_assert!(!self.is_observed());
        self.main_character_run(destination, consume)
    }

    #[cold]
    #[inline(never)]
    fn fail_hot_expanded_delivery(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
        depth: u32,
        failure: CommandError,
    ) -> Result<DeliveryStatus, CommandError> {
        destination.take();
        self.command.transient.active_expansion_depth = depth;
        self.invalidate_delivery_freshness();
        Err(failure)
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Settles the persistent semantic conditions and token-local facts
    /// represented by one delivered command without widening the ordinary hot
    /// loops.
    #[cold]
    #[inline(never)]
    fn settle_exceptional_delivery<const OBSERVED: bool>(
        &mut self,
        command: &mut HotCommand<G>,
        suppresses_expandable_control_sequence: bool,
        outer: bool,
    ) -> Result<(), CommandError> {
        let mode = self.command.delivery_mode;
        if suppresses_expandable_control_sequence {
            command.suppress_expandable();
        }
        if mode.scanner_active() && outer {
            let mut rich = command.materialize();
            self.check_outer_validity_entry(&mut rich)?;
            *command = HotCommand::from_current(rich);
        } else if mode.alignment_active()
            && matches!(
                command.alignment_adjustment(),
                crate::processor::AlignmentDeliveryAdjustment::None
            )
        {
            self.command.roots.alignment.classify_delimiter(command);
        }
        if OBSERVED {
            self.observe_resident_hot_command(command);
        }
        Ok(())
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    #[inline(always)]
    fn finish_expanded_command<const OBSERVED: bool>(
        &mut self,
        command: &HotCommand<G>,
        delivery_expanded: bool,
    ) -> DeliveryStatus {
        #[cfg(feature = "profiling")]
        self.record_expanded_delivery();
        if OBSERVED {
            self.observe_expanded_hot_delivery(command);
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

    #[doc(hidden)]
    pub fn observe_expanded_delivery(&mut self, command: &CurrentCommand<G>) {
        observe!(self, {
            #[cfg(test)]
            {}
            let (command_name, command_operand) =
                crate::observation::canonical_current_command_identity_for_profile(
                    self.command.profile(),
                    command,
                );
            let spelling = self.observed_command_spelling(command);
            let semantic_operand = crate::observation::canonical_sparse_register_operand(
                self.command.profile(),
                command.meaning(),
            );
            CommandObservation::Command(CommandDeliveryRecord {
                boundary: CommandDeliveryBoundary::Expanded,
                spelling,
                command: command_name,
                command_operand,
                semantic_operand,
                provenance: CommandProvenance::from_stamp(
                    command.delivery_stamp(),
                    self.current_delivery_sequence(),
                    command.origin(),
                    self.direct_source_provenance(command),
                ),
            })
        });
    }

    /// Compact observation counterpart for the scanner-owned expanded
    /// delivery.  The terminal command remains in the hot slot while its
    /// canonical identity, spelling, and provenance are projected into the
    /// observer record.
    pub(crate) fn observe_expanded_hot_delivery(&mut self, command: &HotCommand<G>) {
        observe!(self, {
            #[cfg(test)]
            {}
            let meaning = command.resolved_meaning();
            let (command_name, command_operand) =
                crate::observation::canonical_delivery_identity_for_profile(
                    self.command.profile(),
                    command.identity(),
                    meaning,
                );
            let spelling = self.observed_hot_command_spelling(command);
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
                provenance: CommandProvenance::from_stamp(
                    command.delivery_stamp(),
                    self.current_delivery_sequence(),
                    command.origin(),
                    self.direct_source_provenance_hot(command),
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

    /// Emits the conditional command trace while the opener remains in the
    /// caller-owned hot slot. This boundary is cold because tracing is an
    /// observer-only path outside ordinary scanner recursion.
    #[cold]
    #[inline(never)]
    fn trace_hot_conditional(&mut self, command: &HotCommand<G>) {
        if self.command.delivery_mode.tracing() {
            self.print_hot_command_trace(command);
        }
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

    #[inline(never)]
    fn expand_compact_occupied(
        &mut self,
        command: &mut HotCommand<G>,
        primitive: ExpandablePrimitive,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        self.with_expansion_depth(|processor| {
            processor.expand_compact_occupied_in(command, primitive, report_trace)
        })
    }

    #[inline(never)]
    fn expand_compact_occupied_in(
        &mut self,
        command: &mut HotCommand<G>,
        primitive: ExpandablePrimitive,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        #[cfg(feature = "profiling")]
        {
            tex_state::measurement::record_hot_core_expandable_opcode(
                usize::try_from(primitive.operand())
                    .expect("expandable primitive operand fits usize"),
            );
            if self.write_expansion_depth != 0 {
                self.record_write_expansion();
            }
        }
        // TeX82 §367 traces a primitive before its scanner consumes an
        // operand. `EndTemplate` is handled by the delivery loop's sentinel
        // branch and has no primitive trace of its own.
        if report_trace
            && primitive != ExpandablePrimitive::EndTemplate
            && self.command.delivery_mode.tracing()
        {
            self.print_hot_command_trace(command);
        }

        let origin = command.origin();
        let result =
            if let Some(kind) = crate::conditionals::ConditionalKind::from_primitive(primitive) {
                self.expand_conditional_primitive(kind, false)
            } else {
                match primitive {
                    ExpandablePrimitive::Unless => self.expand_unless_compact(),
                    primitive @ (ExpandablePrimitive::Else
                    | ExpandablePrimitive::Or
                    | ExpandablePrimitive::Fi) => {
                        self.expand_conditional_delimiter_hot(command, primitive)
                    }
                    _ => self.expand_compact_occupied_nonconditional(command, primitive, origin),
                }
            };
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

    /// Dispatches the non-conditional expandable families.  Conditional
    /// operands are the one deeply recursive ordinary path, so keeping this
    /// large closed match out of [`Self::expand_compact_occupied`] avoids
    /// reserving its diagnostic and conversion temporaries in every waiting
    /// conditional frame.
    #[inline(never)]
    fn expand_compact_occupied_nonconditional(
        &mut self,
        command: &mut HotCommand<G>,
        primitive: ExpandablePrimitive,
        origin: OriginId,
    ) -> Result<(), CommandError> {
        match primitive {
            ExpandablePrimitive::EndTemplate => self.insert_frozen_endv(),
            ExpandablePrimitive::NoExpand => self.expand_noexpand(),
            ExpandablePrimitive::ExpandAfter => self.expand_expandafter(),
            ExpandablePrimitive::CsName => self.expand_csname(origin),
            ExpandablePrimitive::String => self.expand_string(origin),
            ExpandablePrimitive::Meaning => self.expand_meaning(origin),
            ExpandablePrimitive::Number => self.expand_number_compact(origin, false),
            ExpandablePrimitive::RomanNumeral => self.expand_number_compact(origin, true),
            ExpandablePrimitive::The => {
                let mut target = None;
                match self.request_expanded_token(&mut target)? {
                    DeliveryStatus::Command => {}
                    _ => return Err(CommandError::input_invariant()),
                }
                let target = target.take().ok_or(CommandError::input_invariant())?;
                let scanned = self.scan_internal_value_or_zero_from_target(&target)?;
                self.expand_the_value(origin, scanned.value)
            }
            ExpandablePrimitive::Unexpanded => self.expand_unexpanded(),
            ExpandablePrimitive::Expanded => self.expand_expanded(),
            ExpandablePrimitive::Detokenize => self.expand_detokenize(origin),
            ExpandablePrimitive::Scantokens => self.expand_scantokens(),
            ExpandablePrimitive::FontName => self.expand_fontname(origin),
            ExpandablePrimitive::PdfFontName => self.expand_pdf_font_name(origin),
            ExpandablePrimitive::PdfFontObjectNumber => self.expand_pdf_font_object_number(origin),
            ExpandablePrimitive::PdfFontSize => self.expand_pdf_font_size(origin),
            ExpandablePrimitive::LeftMarginKern | ExpandablePrimitive::RightMarginKern => {
                self.expand_margin_kern(origin, primitive)
            }
            ExpandablePrimitive::Input => self.expand_input_hot(command),
            ExpandablePrimitive::EndInput => self.expand_endinput(),
            ExpandablePrimitive::JobName => {
                self.state.unsupported_host_capability();
                let job_name = self.host.job_name().to_owned();
                self.push_rendered_text(&job_name, origin);
                Ok(())
            }
            ExpandablePrimitive::ETeXRevision => {
                self.push_rendered_text(".6", origin);
                Ok(())
            }
            ExpandablePrimitive::PdfTeXRevision => {
                self.push_rendered_text("27", origin);
                Ok(())
            }
            ExpandablePrimitive::PdfTeXBanner => {
                self.push_rendered_text(
                    "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) kpathsea version 6.4.2",
                    origin,
                );
                Ok(())
            }
            ExpandablePrimitive::PdfUniformDeviate => self.expand_pdf_uniform_deviate(origin),
            ExpandablePrimitive::PdfNormalDeviate => {
                let value = self.state.pdf_normal_deviate();
                self.push_rendered_text(&value.to_string(), origin);
                Ok(())
            }
            ExpandablePrimitive::CreationDate => {
                let clock = self.state.job_clock();
                self.push_rendered_text(&format_pdf_date(clock, 0), origin);
                Ok(())
            }
            ExpandablePrimitive::ShellEscape => {
                let status = self
                    .state
                    .internal_integer(tex_state::meaning::InternalInteger::PdfShellEscape)
                    .expect("the shell-escape status is an integer enquiry");
                self.push_rendered_text(&status.to_string(), origin);
                Ok(())
            }
            ExpandablePrimitive::StringCompare => self.expand_string_compare(origin),
            ExpandablePrimitive::PdfEscapeString => self.expand_pdf_escape_string(origin),
            ExpandablePrimitive::PdfEscapeHex => self.expand_pdf_escape_hex(origin),
            ExpandablePrimitive::PdfUnescapeHex => self.expand_pdf_unescape_hex(origin),
            ExpandablePrimitive::PdfColorStackInit => self.expand_pdf_color_stack_init(origin),
            ExpandablePrimitive::PdfMatch => self.expand_pdf_match(origin),
            ExpandablePrimitive::PdfLastMatch => self.expand_pdf_last_match(origin),
            ExpandablePrimitive::PdfFileDump => self.expand_pdf_file_dump(origin),
            ExpandablePrimitive::FileSize => self.expand_pdf_file_size(origin),
            ExpandablePrimitive::PdfFileModificationDate => {
                self.expand_pdf_file_modification_date(origin)
            }
            ExpandablePrimitive::PdfMdFiveSum => self.expand_pdf_md_five_sum(origin),
            ExpandablePrimitive::PdfInsertHeight => self.expand_pdf_insert_height(origin),
            ExpandablePrimitive::PdfXImageBBox => self.expand_pdf_ximage_bbox(origin),
            ExpandablePrimitive::PdfXFormName => self.expand_pdf_xform_name(origin),
            ExpandablePrimitive::PdfPageRef => self.expand_pdf_page_ref(origin),
            ExpandablePrimitive::PdfPrimitive => {
                let mut destination = None;
                match self.get_next_into(&mut destination)? {
                    DeliveryStatus::End => return Err(CommandError::input_invariant()),
                    DeliveryStatus::Command => {}
                    _ => unreachable!("ordinary raw delivery returns only commands"),
                }
                let target = destination
                    .take()
                    .expect("command status initializes destination");
                let Some(symbol) = target.control_sequence() else {
                    return Ok(());
                };
                let name = self.state.resolve(symbol);
                let Some(frozen) = self.state.primitive_token(name) else {
                    return Ok(());
                };
                self.back_input_token(TracedTokenWord::pack(frozen, target.origin()))
            }
            ExpandablePrimitive::TopMark
            | ExpandablePrimitive::FirstMark
            | ExpandablePrimitive::BotMark
            | ExpandablePrimitive::SplitFirstMark
            | ExpandablePrimitive::SplitBotMark => self.expand_mark(primitive),
            ExpandablePrimitive::TopMarks
            | ExpandablePrimitive::FirstMarks
            | ExpandablePrimitive::BotMarks
            | ExpandablePrimitive::SplitFirstMarks
            | ExpandablePrimitive::SplitBotMarks => self.expand_mark_class(primitive),
            ExpandablePrimitive::ETeXVersion
            | ExpandablePrimitive::IfPdfPrimitive
            | ExpandablePrimitive::PdfEscapeName => {
                Err(CommandError::UnsupportedExpandablePrimitive(primitive))
            }
            ExpandablePrimitive::EndCsName => Err(CommandError::input_invariant()),
            ExpandablePrimitive::Unless
            | ExpandablePrimitive::Else
            | ExpandablePrimitive::Or
            | ExpandablePrimitive::Fi => Err(CommandError::input_invariant()),
            _ => Err(CommandError::UnsupportedExpandablePrimitive(primitive)),
        }
    }

    /// Handles the one expandable branch that is not represented by an
    /// `ExpandablePrimitive`. Undefined recovery keeps its compact spelling
    /// until the diagnostic site is captured; it is otherwise just another
    /// continue case of the ordinary expanded loop.
    #[inline(always)]
    fn expand_undefined_hot(
        &mut self,
        command: &HotCommand<G>,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        self.with_expansion_depth(|processor| {
            processor.expand_undefined_hot_in(command, report_trace)
        })
    }

    #[inline(never)]
    fn expand_undefined_hot_in(
        &mut self,
        command: &HotCommand<G>,
        report_trace: bool,
    ) -> Result<(), CommandError> {
        if report_trace && self.command.delivery_mode.tracing() {
            self.print_hot_command_trace(command);
        }
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_undefined_expansion();
        let context = self.command.output_open_context(self.state);
        let site = Some(self.complete_diagnostic_site(self.capture_hot_diagnostic_site(command)));
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::UndefinedControlSequence { context, site });
        if !self.command.profile().capabilities().supports_etex() {
            self.observe_hot_command_diagnostic("undefined_control_sequence", command);
        }
        Ok(())
    }

    /// Fast path for the common decimal conversion opener.  The first digit
    /// is obtained from raw delivery so a non-decimal prefix can be handed
    /// back unchanged to the complete scalar scanner.  Subsequent digits are
    /// expanded synchronously and a non-space terminator is backed up from
    /// the same hot owner; no rich command is needed for the usual
    /// `\number 123`/`\romannumeral 123` conversion.
    #[inline(never)]
    fn expand_number_compact(&mut self, origin: OriginId, roman: bool) -> Result<(), CommandError> {
        // Alignment template termination is a semantic boundary owned by
        // the ordinary rich scanner entry. Keep that exceptional transition
        // on its established path; ordinary conversions stay entirely in
        // the occupied hot command below.
        if self.command.delivery_mode.alignment_active() {
            return self.expand_number(origin, roman);
        }
        let mut first = None;
        match self.expanded_next_hot(&mut first, None)? {
            DeliveryStatus::End => {
                self.observe_integer_value(0);
                let text = if roman {
                    super::expand_render::roman_numeral(0)
                } else {
                    "0".to_owned()
                };
                self.push_rendered_text(&text, origin);
                return Ok(());
            }
            DeliveryStatus::Command | DeliveryStatus::PendingExpanded => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let first = first.take().ok_or(CommandError::input_invariant())?;
        let Some(digit) = hot_decimal_digit(&first) else {
            let scan = self.scan_integer_from_leading_hot(first)?;
            let text = if roman {
                super::expand_render::roman_numeral(scan.value)
            } else {
                scan.value.to_string()
            };
            self.push_rendered_text(&text, origin);
            return Ok(());
        };
        let mut value = i32::from(digit);
        let mut overflowed = false;
        loop {
            let mut next = None;
            match self.expanded_next_hot(&mut next, None)? {
                DeliveryStatus::End => break,
                DeliveryStatus::Command | DeliveryStatus::PendingExpanded => {}
                _ => return Err(CommandError::input_invariant()),
            }
            let next = next.take().ok_or(CommandError::input_invariant())?;
            if let Some(digit) = hot_decimal_digit(&next) {
                match value
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(i32::from(digit)))
                {
                    Some(next_value) => value = next_value,
                    None => {
                        value = i32::MAX;
                        if !overflowed {
                            let site = self.capture_hot_diagnostic_site(&next);
                            self.number_too_big_error(Some(site))?;
                            overflowed = true;
                        }
                    }
                }
                continue;
            }
            if !hot_is_space(&next) {
                self.back_input_hot(next)?;
            }
            break;
        }
        let text = if roman {
            super::expand_render::roman_numeral(value)
        } else {
            value.to_string()
        };
        self.observe_integer_value(value);
        self.push_rendered_text(&text, origin);
        Ok(())
    }

    /// Creates one invocation provenance node and atomically exposes its
    /// activation/body ownership pair to the input stack.
    ///
    /// The scalar macro matcher owns argument matching and calls this only
    /// after it has completed every range. Nested invocations use the live
    /// activation chain, not a replay trace, as their provenance parent.
    #[allow(dead_code)] // consumed by the ordered scalar macro matcher issue
    pub(crate) fn push_macro_activation(
        &mut self,
        name: tex_state::interner::Symbol,
        body: tex_state::ResidentMacroBody<G>,
        call_site: OriginId,
        arguments: Option<ArgumentSetId<G>>,
    ) -> InputLevelId {
        let invocation = call_site;
        self.invalidate_delivery_freshness();
        self.command
            .push_macro_activation(name, body, arguments, invocation)
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
    let meaning = command.meaning_ref();
    matches!(meaning, ResolvedMeaning::Macro { .. })
        || matches!(meaning, ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive)) if *primitive != ExpandablePrimitive::EndCsName)
        || (matches!(meaning, ResolvedMeaning::Static(Meaning::Undefined))
            && !matches!(command.spelling().semantic_token(), Token::Param(_)))
}

#[cfg(test)]
mod tests;
