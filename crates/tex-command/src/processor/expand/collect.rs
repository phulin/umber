//! Direct unexpanded collection from the shared semantic input reader.

use super::{ResidentColdOutcome, consume::TokenMeaning};
use crate::command::HotCommand;
use crate::token_collector::{ClassifiedToken, TokenCollector};
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use tex_state::interner::Symbol;
use tex_state::token::{Catcode, TracedTokenWord};

impl<G> CommandProcessor<'_, '_, G> {
    pub(crate) fn collect_unexpanded_replacement(
        &mut self,
        parameters: Option<(u8, Option<Symbol>)>,
        collector: &mut TokenCollector<G>,
    ) -> Result<(), CommandError> {
        self.invalidate_delivery_freshness();
        let result = if self.is_observed() {
            self.collect_unexpanded_words::<true>(parameters, collector)
        } else {
            self.collect_unexpanded_words::<false>(parameters, collector)
        };
        // Collection returns a token-list owner, never a delivered command.
        self.invalidate_delivery_freshness();
        result
    }

    fn collect_unexpanded_words<const OBSERVED: bool>(
        &mut self,
        parameters: Option<(u8, Option<Symbol>)>,
        collector: &mut TokenCollector<G>,
    ) -> Result<(), CommandError> {
        loop {
            // Source creation is the reader's policy for this consumer. No
            // mutable processor flag needs setting and clearing per body word.
            let word = match self.read_raw_word(true)? {
                ResidentColdOutcome::Word(word) => word,
                ResidentColdOutcome::Finished(DeliveryStatus::ReplayCompleted(_)) => continue,
                ResidentColdOutcome::Finished(_) => return Err(CommandError::input_invariant()),
                ResidentColdOutcome::Retry => unreachable!("reader settles transitions"),
            };
            let literal = word.word.literal_catcode();
            // Literal spelling already supplies all collection facts. Active
            // characters and control sequences still need live outer validity.
            let mut meaning = None;
            let mut lookup = false;
            let outer = if literal.is_some_and(|cat| cat != Catcode::Active) && !OBSERVED {
                false
            } else {
                let (resolved, resolution) = self.resolve_read(&word);
                lookup = resolution.meaning_lookup();
                let outer = resolved.is_outer();
                meaning = Some(resolved);
                outer
            };
            self.record_consumed_read(&word, lookup);
            let exceptional = OBSERVED
                || self
                    .command
                    .delivery_mode
                    .requires_semantic_settlement(word.suppress_expandable, outer);
            if exceptional {
                let meaning = meaning.unwrap_or_else(|| self.resolve_read(&word).0);
                let mut command = word.materialize(&meaning);
                self.admit_materialized_read(&word, &command);
                self.settle_hot_delivery_in::<OBSERVED>(&mut command, literal)?;
                if matches!(
                    command.alignment_adjustment(),
                    super::super::AlignmentDeliveryAdjustment::Delimiter(_)
                ) {
                    self.begin_scalar_alignment_v_template_hot(&command)?;
                    continue;
                }
                if command.is_outer_recovery_space() {
                    continue;
                }
                let token = self.classify_collector_hot_token(&command, None);
                if self.accept_replacement_word(parameters, collector, token, |processor| {
                    processor.back_input_hot(command)
                })? {
                    return Ok(());
                }
                continue;
            }
            let adjustment = self
                .command
                .roots
                .alignment
                .account_literal_catcode(&mut self.command.timeline, literal);
            let token = ClassifiedToken::from_word(
                TracedTokenWord::from_parts(word.word, word.origin),
                None,
            );
            #[cfg(test)]
            {
                self.command
                    .token_collector_path_counters
                    .raw_classifications += 1;
            }
            if self.accept_replacement_word(parameters, collector, token, |processor| {
                // Only illegal parameter recovery needs a backed-up command.
                // A control-sequence meaning is retained from its single lookup;
                // a literal's meaning is decoded lazily on this error branch.
                let meaning: TokenMeaning<G> =
                    meaning.unwrap_or_else(|| processor.resolve_read(&word).0);
                let mut command: HotCommand<G> = word.materialize(&meaning);
                command.set_alignment_adjustment(adjustment);
                processor.admit_materialized_read(&word, &command);
                processor.back_input_hot(command)
            })? {
                return Ok(());
            }
        }
    }
}
