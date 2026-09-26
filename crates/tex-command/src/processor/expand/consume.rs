//! Consumer-side interpretation of the shared input reader.

use super::{
    ExpandedCommandAction, ExpansionDispatch, ReadSite, ResidentColdOutcome, ResidentWord,
    classify_hot_command, resident::ResidentAdmission,
};
use crate::command::HotCommand;
use crate::input::InputLevel;
#[cfg(any(test, feature = "profiling"))]
use crate::input::ResidentTokenStorage;
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use std::ops::ControlFlow;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningFlags, MeaningWord};
use tex_state::token::{PackedCommandTarget, PackedMeaningResolution};

/// Only the meaning selected by one dense lookup. No spelling, delivery
/// record, recovery geometry, or independent ownership enters this value.
pub(super) struct TokenMeaning<G> {
    pub(super) word: MeaningWord<G>,
    pub(super) control_sequence: Option<Symbol>,
}

/// One read from the shared raw input kernel, interpreted for expansion.
/// Preflight and the ordinary expansion loop share this exact admission so
/// neither has to construct a command for an ordinary unobserved macro.
pub(super) enum ExpansionCandidate<G> {
    ExpandedMacro,
    Command(HotCommand<G>),
    Finished(DeliveryStatus),
}

impl<G> TokenMeaning<G> {
    #[inline(always)]
    pub(super) fn empty() -> Self {
        Self {
            word: MeaningWord::Static(Meaning::Undefined.encode()),
            control_sequence: None,
        }
    }

    #[inline(always)]
    pub(super) fn is_outer(&self) -> bool {
        match &self.word {
            MeaningWord::Macro { flags, .. } => flags.contains(MeaningFlags::OUTER),
            MeaningWord::Static(word) => {
                *word
                    == Meaning::ExpandablePrimitive(
                        tex_state::meaning::ExpandablePrimitive::EndTemplate,
                    )
                    .encode()
            }
            MeaningWord::Font(_) => false,
        }
    }

    #[inline(always)]
    fn install(&self, destination: &mut HotCommand<G>) {
        destination.write_control_sequence(self.control_sequence);
        match &self.word {
            MeaningWord::Static(word) => destination.write_static_meaning_word(*word),
            MeaningWord::Macro { flags, definition } => {
                destination.write_macro_meaning(*flags, *definition)
            }
            MeaningWord::Font(font) => destination.write_font_meaning(*font),
        }
    }
}

impl<G> PackedCommandTarget<G> for TokenMeaning<G> {
    #[inline(always)]
    fn write_control_sequence(&mut self, control_sequence: Option<Symbol>) {
        self.control_sequence = control_sequence;
    }
    #[inline(always)]
    fn write_static_meaning_word(&mut self, word: u64) {
        self.word = MeaningWord::Static(word);
    }
    #[inline(always)]
    fn write_font_meaning(&mut self, font: tex_state::ids::FontId) {
        self.word = MeaningWord::Font(font);
    }
    #[inline(always)]
    fn write_macro_meaning(
        &mut self,
        flags: MeaningFlags,
        definition: tex_state::DefinitionRef<G>,
    ) {
        self.word = MeaningWord::Macro { flags, definition };
    }
}

impl ResidentWord {
    /// Materialize only when a consumer needs a full command. This operation
    /// is pure: diagnostic construction must not replace backup authority.
    #[inline(always)]
    pub(super) fn materialize<G>(&self, meaning: &TokenMeaning<G>) -> HotCommand<G> {
        let line = match self.site {
            ReadSite::Source(line) => line,
            _ => None,
        };
        let mut command = HotCommand::delivery_storage(
            self.word,
            self.origin,
            self.identity,
            self.position,
            self.active_source,
            matches!(self.site, ReadSite::Source(_)),
            line,
            self.suppress_expandable,
        );
        meaning.install(&mut command);
        command
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    /// TeX.web §494's raw skip consumer. Literal resident characters that
    /// cannot affect skipping share one physical-frame admission; the first
    /// exceptional word is returned by that same reader for normal settlement.
    pub(crate) fn get_next_skipping_into(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        if self.is_observed()
            || self.command.delivery_mode.tracing()
            || self.command.delivery_mode.requires_persistent_settlement()
        {
            return self.get_next_hot_into(destination);
        }
        let Some(index) = self.command.roots.input.levels.top.checked_sub(1) else {
            return self.get_next_hot_into(destination);
        };
        let Some(InputLevel::Resident(row)) = self.command.roots.input.levels.rows.get(index)
        else {
            return self.get_next_hot_into(destination);
        };
        #[cfg(any(test, feature = "profiling"))]
        let argument = matches!(row.storage, ResidentTokenStorage::MacroArgument(_));
        // A `\noexpand`-marked frame settles its control sequence through
        // the ordinary reader; every other resident control sequence is
        // classified by one meaning lookup below.
        let classify_meanings = !row.header.frame.flags().contains(
            tex_state::packed_input::InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
        );
        if self.fuel.remaining() == 0 {
            return self.get_next_hot_into(destination);
        }

        self.pending_diagnostic_location = None;
        let mut consumed = 0_u32;
        let fuel = &mut *self.fuel;
        let state = &*self.state;
        let selected = Self::read_resident_run(self.command, |word, _| {
            let transparent = match word.literal_catcode() {
                Some(
                    tex_state::token::Catcode::BeginGroup | tex_state::token::Catcode::EndGroup,
                ) => false,
                Some(tex_state::token::Catcode::Active) => {
                    classify_meanings && Self::skipped_meaning_is_transparent(state, word)
                }
                Some(_) => true,
                // Parameters and frozen tokens keep their input transitions.
                None => {
                    word.is_control_sequence()
                        && classify_meanings
                        && Self::skipped_meaning_is_transparent(state, word)
                }
            };
            if !transparent {
                return Ok(ResidentAdmission::Boundary);
            }
            fuel.charge()?;
            consumed += 1;
            if fuel.remaining() == 0 || consumed == u32::MAX {
                Ok(ResidentAdmission::Stop)
            } else {
                Ok(ResidentAdmission::Continue)
            }
        })?;
        if consumed != 0 {
            self.enter_resident_delivery();
            #[cfg(test)]
            if argument {
                self.command
                    .raw_delivery_path_counters
                    .macro_argument_direct += u64::from(consumed);
            } else {
                self.command.raw_delivery_path_counters.stored_direct += u64::from(consumed);
            }
            #[cfg(feature = "profiling")]
            self.fuel.record_raw_run(
                self.command.delivery_mode.scanner_active(),
                if argument {
                    crate::fuel::RawDeliveryKind::MacroArgument
                } else {
                    crate::fuel::RawDeliveryKind::StoredToken
                },
                consumed,
            );
        }
        match selected {
            ControlFlow::Continue(selected) => {
                self.pending_diagnostic_location = None;
                let result = (|| {
                    self.charge_command_action()?;
                    match self
                        .finish_charged_raw_read(selected, self.create_source_control_sequences)?
                    {
                        ResidentColdOutcome::Word(word) => {
                            self.finish_selected_hot_word::<false>(word, destination)
                        }
                        ResidentColdOutcome::Finished(status) => {
                            destination.take();
                            Ok(status)
                        }
                        ResidentColdOutcome::Retry => {
                            unreachable!("charged raw reader settles transitions")
                        }
                    }
                })();
                match result {
                    Ok(status) => match self.settle_hot_raw_step(status, destination)? {
                        Some(status) => Ok(status),
                        None => self.get_next_hot_into(destination),
                    },
                    Err(failure) => self.fail_hot_expanded_delivery(
                        destination,
                        self.command.transient.active_expansion_depth,
                        failure,
                    ),
                }
            }
            ControlFlow::Break(()) => self.get_next_hot_into(destination),
        }
    }

    /// TeX82 §494 `pass_text` inspects only `cur_cmd`: a conditional
    /// (`if_test`) or delimiter (`fi_or_else`) ends or nests the skip, and
    /// §336's `check_outer_validity` owns an outer macro or `\endtemplate`.
    /// Every other skipped command is discarded without settlement, so its
    /// word needs one dense meaning lookup and no delivery record.
    #[inline(always)]
    fn skipped_meaning_is_transparent(
        state: &tex_state::CommandContext<'_, G>,
        word: tex_state::token::TokenWord,
    ) -> bool {
        let mut meaning = TokenMeaning::empty();
        state.write_packed_token_command_into(word, &mut meaning);
        if meaning.is_outer() {
            return false;
        }
        match meaning.word {
            MeaningWord::Static(word) => {
                !matches!(
                    Meaning::runtime_word_class(word),
                    tex_state::meaning::StaticCommandClass::Expandable
                ) || !matches!(
                    tex_state::meaning::ExpandablePrimitive::from_operand(
                        Meaning::runtime_word_operand(word)
                    ),
                    Some(primitive)
                        if crate::conditionals::ConditionalKind::from_primitive(primitive)
                            .is_some()
                            || matches!(
                                primitive,
                                tex_state::meaning::ExpandablePrimitive::Fi
                                    | tex_state::meaning::ExpandablePrimitive::Else
                                    | tex_state::meaning::ExpandablePrimitive::Or
                            )
                )
            }
            MeaningWord::Macro { .. } | MeaningWord::Font(_) => true,
        }
    }

    #[inline(always)]
    pub(super) fn read_expansion_candidate<
        const OBSERVED: bool,
        const STOP_PROTECTED: bool,
        const PREFLIGHT_FIRST: bool,
    >(
        &mut self,
    ) -> Result<ExpansionCandidate<G>, CommandError> {
        let word = match self.read_raw_word(self.create_source_control_sequences)? {
            ResidentColdOutcome::Word(word) => word,
            ResidentColdOutcome::Finished(status) => {
                return Ok(ExpansionCandidate::Finished(status));
            }
            ResidentColdOutcome::Retry => unreachable!("reader settles transitions"),
        };
        let (meaning, resolution) = self.resolve_read(&word);
        self.record_consumed_read(&word, resolution.meaning_lookup());
        if !OBSERVED
            && !self
                .command
                .delivery_mode
                .requires_semantic_settlement(word.suppress_expandable, meaning.is_outer())
            && let MeaningWord::Macro { flags, definition } = &meaning.word
            && !(STOP_PROTECTED && flags.contains(MeaningFlags::PROTECTED))
        {
            let name = meaning
                .control_sequence
                .ok_or_else(CommandError::input_invariant)?;
            let spelling = tex_state::token::TracedTokenWord::from_parts(word.word, word.origin);
            if PREFLIGHT_FIRST {
                // The supplied-command path activated this first macro
                // inside expanded_next_hot's transient quiescence guard.
                let depth = self.command.transient.active_expansion_depth;
                let active_depth = depth
                    .checked_add(1)
                    .ok_or_else(CommandError::input_invariant)?;
                self.command.transient.active_expansion_depth = active_depth;
                let result = self.activate_read_macro(spelling, *flags, *definition, name);
                self.command.transient.active_expansion_depth = depth;
                result?;
            } else {
                self.activate_read_macro(spelling, *flags, *definition, name)?;
            }
            return Ok(ExpansionCandidate::ExpandedMacro);
        }
        let mut command = word.materialize(&meaning);
        self.admit_materialized_read(&word, &command);
        self.settle_hot_delivery_in::<OBSERVED>(&mut command, resolution.literal_catcode())?;
        Ok(ExpansionCandidate::Command(command))
    }

    #[inline(always)]
    pub(super) fn resolve_read(
        &self,
        word: &ResidentWord,
    ) -> (TokenMeaning<G>, PackedMeaningResolution) {
        let mut meaning = TokenMeaning::empty();
        let resolution = self
            .state
            .write_packed_token_command_into(word.word, &mut meaning);
        (meaning, resolution)
    }

    #[inline(always)]
    pub(super) fn record_consumed_read(&mut self, word: &ResidentWord, lookup: bool) {
        #[cfg(test)]
        match word.storage_kind {
            super::ResidentStorageKind::Stored => {
                self.command.raw_delivery_path_counters.stored_direct += 1;
                self.command.stored_token_advance_counters.meaning_lookups += u64::from(lookup);
            }
            super::ResidentStorageKind::MacroArgument => {
                self.command
                    .raw_delivery_path_counters
                    .macro_argument_direct += 1;
            }
            _ => {}
        }
        #[cfg(feature = "profiling")]
        self.fuel.record_raw_delivery(
            self.command.delivery_mode.scanner_active(),
            lookup,
            word.raw_kind,
        );
        #[cfg(not(any(test, feature = "profiling")))]
        let _ = (word, lookup);
    }

    #[inline(always)]
    pub(super) fn admit_materialized_read(&mut self, word: &ResidentWord, command: &HotCommand<G>) {
        #[cfg(test)]
        match word.storage_kind {
            super::ResidentStorageKind::Stored => {
                self.command.stored_token_advance_counters.command_writes += 1
            }
            super::ResidentStorageKind::MacroBody => {
                self.command.macro_kernel_counters.body_command_writes += 1
            }
            super::ResidentStorageKind::MacroArgument => {
                self.command.macro_kernel_counters.argument_command_writes += 1
            }
            _ => {}
        }
        match word.site {
            ReadSite::Resident => self.enter_resident_delivery(),
            ReadSite::Source(_) | ReadSite::Synthetic => {
                self.readmit_delivery_stamp(command.delivery_stamp())
            }
        }
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
        destination: &mut Option<HotCommand<G>>,
        initial_action: Option<ExpandedCommandAction>,
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
            let mut command =
                match self.read_expansion_candidate::<OBSERVED, STOP_PROTECTED, false>()? {
                    ExpansionCandidate::ExpandedMacro => {
                        expanded = true;
                        continue;
                    }
                    ExpansionCandidate::Finished(status) => {
                        destination.take();
                        return Ok(status);
                    }
                    ExpansionCandidate::Command(command) => command,
                };
            let action = classify_hot_command(&command);
            if STOP_PROTECTED && Self::protected_terminal(&command, action) {
                *destination = Some(command);
                return Ok(DeliveryStatus::Command);
            }
            if PRESERVE_UNDEFINED
                && matches!(
                    action,
                    ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
                )
            {
                let status =
                    self.finish_terminal_expansion::<OBSERVED>(&mut command, action, expanded);
                *destination = Some(command);
                return Ok(status);
            }
            if let ExpandedCommandAction::Expand(dispatch) = action {
                self.execute_expansion_action(&mut command, dispatch)?;
                expanded = true;
                continue;
            }
            let status = self.finish_terminal_expansion::<OBSERVED>(&mut command, action, expanded);
            *destination = Some(command);
            return Ok(status);
        }
    }

    /// e-TeX's `get_x_or_protected` procedure stops on the first
    /// unexpandable command or protected macro after every expansion restart.
    /// The command has already crossed its raw boundary; returning it here
    /// deliberately skips TeX82's terminal expanded observation.
    #[inline(always)]
    fn protected_terminal(command: &HotCommand<G>, action: ExpandedCommandAction) -> bool {
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
        spelling: tex_state::token::TracedTokenWord,
        flags: MeaningFlags,
        definition: tex_state::DefinitionRef<G>,
        name: Symbol,
    ) -> Result<(), CommandError> {
        self.invalidate_delivery_freshness();
        self.record_macro_expansion();
        match self.macro_call_parts(flags, definition, name, spelling.origin(), |processor| {
            processor.report_unobserved_macro_prefix_mismatch(spelling, flags, definition, name);
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
        destination: &mut Option<HotCommand<G>>,
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
        command: &mut HotCommand<G>,
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
