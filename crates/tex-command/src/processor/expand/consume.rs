//! Consumer-side interpretation of the shared input reader.

use super::{ReadSite, ResidentColdOutcome, ResidentWord};
use crate::command::HotCommand;
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningFlags, MeaningWord};
use tex_state::token::{PackedCommandTarget, PackedMeaningResolution};

/// Only the meaning selected by one dense lookup. No spelling, delivery
/// record, recovery geometry, or independent ownership enters this value.
pub(super) struct TokenMeaning<G> {
    pub(super) word: MeaningWord<G>,
    pub(super) control_sequence: Option<Symbol>,
}

impl<G> TokenMeaning<G> {
    pub(super) fn empty() -> Self {
        Self {
            word: MeaningWord::Static(Meaning::Undefined.encode()),
            control_sequence: None,
        }
    }

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

    /// Read and activate ordinary macros without a command record. The same
    /// reader feeds raw callers and collectors; only this consumer expands.
    pub(super) fn fetch_expansion_command<const OBSERVED: bool>(
        &mut self,
        destination: &mut Option<HotCommand<G>>,
        expanded: &mut bool,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let word = match self.read_raw_word(self.create_source_control_sequences)? {
                ResidentColdOutcome::Word(word) => word,
                ResidentColdOutcome::Finished(status) => {
                    destination.take();
                    return Ok(status);
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
            {
                let name = meaning
                    .control_sequence
                    .ok_or_else(CommandError::input_invariant)?;
                self.invalidate_delivery_freshness();
                self.record_macro_expansion();
                match self.macro_call_parts(*flags, *definition, name, word.origin, |processor| {
                    let call = word.materialize(&meaning);
                    processor.report_macro_prefix_mismatch(&call);
                }) {
                    Ok(_)
                    | Err(
                        CommandError::ParagraphInMacroArgument | CommandError::OuterInMacroArgument,
                    ) => {}
                    Err(error) => return Err(error),
                }
                *expanded = true;
                continue;
            }
            let mut command = word.materialize(&meaning);
            self.admit_materialized_read(&word, &command);
            self.settle_hot_delivery_in::<OBSERVED>(&mut command, resolution.literal_catcode())?;
            *destination = Some(command);
            return Ok(DeliveryStatus::Command);
        }
    }
}
