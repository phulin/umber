//! Direct unexpanded collection from the shared semantic input reader.

use super::ReadSite;
use crate::command::CurrentCommand;
use crate::execution_scratch::{ArgumentRunAdmission, DefinitionReplacementSink};
use crate::token_collector::TokenCollector;
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use std::ops::ControlFlow;
use tex_state::interner::Symbol;
use tex_state::token::{Catcode, TracedTokenWord};

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// TeX82 §365's raw token operand, without a command product. Observation
    /// and recovery retain their demanded record inside the canonical reader.
    pub fn get_token_spelling(&mut self) -> Result<Option<TracedTokenWord>, CommandError> {
        self.consume_token_spelling(&mut |_, spelling, _| Ok(spelling))
    }

    /// Gives a raw-token consumer final spelling, borrowing a command only at a
    /// real semantic settlement or recovery boundary.
    pub(crate) fn consume_token_spelling<R>(
        &mut self,
        consume: &mut impl FnMut(
            &mut Self,
            TracedTokenWord,
            Option<&CurrentCommand<G>>,
        ) -> Result<R, CommandError>,
    ) -> Result<Option<R>, CommandError> {
        debug_assert!(!self.create_source_control_sequences);
        self.create_source_control_sequences = true;
        let result = if self.is_observed() {
            self.consume_raw_spelling::<true, R>(consume)
        } else {
            self.consume_raw_spelling::<false, R>(consume)
        };
        self.create_source_control_sequences = false;
        result
    }

    fn consume_raw_spelling<const OBSERVED: bool, R>(
        &mut self,
        consume: &mut impl FnMut(
            &mut Self,
            TracedTokenWord,
            Option<&CurrentCommand<G>>,
        ) -> Result<R, CommandError>,
    ) -> Result<Option<R>, CommandError> {
        self.create_source_control_sequences = true;
        self.consume_input(
            true,
            &mut |_, _, _, _, _, _, _| Ok(ControlFlow::Continue(false)),
            &mut |processor, word, origin, suppress, site, kind| {
                let literal = word.literal_catcode();
                let mut meaning = None;
                let mut lookup = false;
                let outer = if literal.is_some_and(|cat| cat != Catcode::Active) && !OBSERVED {
                    false
                } else {
                    let (resolved, resolution) = processor.resolve_token_word(word);
                    lookup = resolution.meaning_lookup();
                    let outer = resolved.is_outer();
                    meaning = Some((resolved, resolution));
                    outer
                };
                processor.record_consumed_word(kind, lookup);
                let exceptional = OBSERVED
                    || processor.outer_recovered_while_matching
                    || processor.eof_recovered_while_matching
                    || processor
                        .command
                        .delivery_mode
                        .requires_semantic_settlement(suppress, outer);
                if exceptional {
                    let (meaning, resolution) =
                        meaning.unwrap_or_else(|| processor.resolve_token_word(word));
                    let mut command = CurrentCommand::from_delivered_parts(
                        word,
                        origin,
                        matches!(site, ReadSite::Source(_)),
                        match site {
                            ReadSite::Source(line) => line,
                            _ => None,
                        },
                        suppress,
                        meaning,
                        resolution.control_sequence(),
                    );
                    processor.settle_hot_delivery_in::<OBSERVED>(&mut command, literal)?;
                    if matches!(
                        command.alignment_adjustment(),
                        super::super::AlignmentDeliveryAdjustment::Delimiter(_)
                    ) {
                        processor.begin_scalar_alignment_v_template(&command)?;
                        return Ok(ControlFlow::Continue(()));
                    }
                    #[cfg(test)]
                    {
                        processor
                            .command
                            .token_collector_path_counters
                            .raw_classifications += 1;
                    }
                    processor.create_source_control_sequences = false;
                    return consume(processor, command.spelling(), Some(&command))
                        .map(|value| ControlFlow::Break(Some(value)));
                }
                if matches!(site, ReadSite::Source(_))
                    && let Some(location) = processor.pending_diagnostic_location.take()
                {
                    processor
                        .command
                        .set_last_diagnostic_location(Some(location));
                }
                processor
                    .command
                    .roots
                    .alignment
                    .account_literal_catcode(literal);
                #[cfg(test)]
                {
                    processor
                        .command
                        .token_collector_path_counters
                        .raw_classifications += 1;
                }
                processor.create_source_control_sequences = false;
                consume(processor, TracedTokenWord::from_parts(word, origin), None)
                    .map(|value| ControlFlow::Break(Some(value)))
            },
            |status| match status {
                DeliveryStatus::ReplayCompleted(_) => Ok(ControlFlow::Continue(())),
                DeliveryStatus::End => Ok(ControlFlow::Break(None)),
                _ => unreachable!("raw spelling has no expanded event"),
            },
        )
    }

    pub(crate) fn collect_unexpanded_replacement(
        &mut self,
        parameters: Option<(u8, Option<Symbol>)>,
        collector: &mut TokenCollector<G>,
    ) -> Result<(), CommandError> {
        let result = if self.is_observed() {
            self.collect_unexpanded_words::<true>(parameters, collector)
        } else {
            self.collect_unexpanded_words::<false>(parameters, collector)
        };
        // Collection returns a token-list owner, never a delivered command.
        result
    }

    /// Moves the plain prefix of the current resident span straight into an
    /// open definition's replacement text, as macro arguments do. Every word
    /// the run leaves behind is delivered by the scalar loop below. Returns
    /// whether the run ended at a transition its caller may settle in place.
    fn consume_replacement_run(
        &mut self,
        macro_definition: bool,
        collector: &mut TokenCollector<G>,
    ) -> Result<bool, CommandError> {
        let Some((writer, cursor)) = collector.replacement_run() else {
            return Ok(false);
        };
        // The body's own closing brace (depth one) ends the run.
        let mut admission = ArgumentRunAdmission::with_commands(self.state, None, None, 1)
            .stopping_at_parameters(macro_definition);
        if self.command.delivery_mode.alignment_active() {
            admission.restrict_to_characters();
        }
        let mut sink = DefinitionReplacementSink { writer, cursor };
        let run = self
            .command
            .consume_plain_resident_run(&mut sink, self.fuel, admission)?;
        Ok(run.at_transition)
    }

    fn collect_unexpanded_words<const OBSERVED: bool>(
        &mut self,
        parameters: Option<(u8, Option<Symbol>)>,
        collector: &mut TokenCollector<G>,
    ) -> Result<(), CommandError> {
        loop {
            if !OBSERVED {
                // A run ending at a drained frame or a parameter reference
                // resumes on the frame that transition exposes.
                while self.consume_replacement_run(parameters.is_some(), collector)?
                    && self.settle_resident_run_boundary()?
                {}
            }
            let selected = self.consume_input(
                true,
                &mut |_, _, _, _, _, _, _| Ok(ControlFlow::Continue(false)),
                &mut |processor, word, origin, suppress, site, kind| {
                    let literal = word.literal_catcode();
                    // Literal spelling already supplies all collection facts. Active
                    // characters and control sequences still need live outer validity.
                    let mut meaning = None;
                    let mut lookup = false;
                    let outer = if literal.is_some_and(|cat| cat != Catcode::Active) && !OBSERVED {
                        false
                    } else {
                        let (resolved, resolution) = processor.resolve_token_word(word);
                        lookup = resolution.meaning_lookup();
                        let outer = resolved.is_outer();
                        meaning = Some((resolved, resolution));
                        outer
                    };
                    processor.record_consumed_word(kind, lookup);
                    let exceptional = OBSERVED
                        || processor
                            .command
                            .delivery_mode
                            .requires_semantic_settlement(suppress, outer);
                    if exceptional {
                        let (meaning, resolution) =
                            meaning.unwrap_or_else(|| processor.resolve_token_word(word));
                        let mut command = CurrentCommand::from_delivered_parts(
                            word,
                            origin,
                            matches!(site, ReadSite::Source(_)),
                            match site {
                                ReadSite::Source(line) => line,
                                _ => None,
                            },
                            suppress,
                            meaning,
                            resolution.control_sequence(),
                        );
                        processor.settle_hot_delivery_in::<OBSERVED>(&mut command, literal)?;
                        if matches!(
                            command.alignment_adjustment(),
                            super::super::AlignmentDeliveryAdjustment::Delimiter(_)
                        ) {
                            processor.begin_scalar_alignment_v_template(&command)?;
                            return Ok(ControlFlow::Break(false));
                        }
                        if command.is_outer_recovery_space() {
                            return Ok(ControlFlow::Break(false));
                        }
                        #[cfg(test)]
                        {
                            processor
                                .command
                                .token_collector_path_counters
                                .raw_classifications += 1;
                        }
                        let token = command.spelling();
                        if processor.accept_replacement_word(
                            parameters,
                            collector,
                            token,
                            |processor| processor.back_input(command),
                        )? {
                            return Ok(ControlFlow::Break(true));
                        }
                        return Ok(ControlFlow::Break(false));
                    }
                    let adjustment = processor
                        .command
                        .roots
                        .alignment
                        .account_literal_catcode(literal);
                    let token = TracedTokenWord::from_parts(word, origin);
                    #[cfg(test)]
                    {
                        processor
                            .command
                            .token_collector_path_counters
                            .raw_classifications += 1;
                    }
                    if processor.accept_replacement_word(
                        parameters,
                        collector,
                        token,
                        |processor| {
                            // Only illegal parameter recovery needs a backed-up command.
                            // A control-sequence meaning is retained from its single lookup;
                            // a literal's meaning is decoded lazily on this error branch.
                            let (meaning, resolution) =
                                meaning.unwrap_or_else(|| processor.resolve_token_word(word));
                            let mut command: CurrentCommand<G> =
                                CurrentCommand::from_delivered_parts(
                                    word,
                                    origin,
                                    matches!(site, ReadSite::Source(_)),
                                    match site {
                                        ReadSite::Source(line) => line,
                                        _ => None,
                                    },
                                    suppress,
                                    meaning,
                                    resolution.control_sequence(),
                                );
                            command.set_alignment_adjustment(adjustment);
                            processor.back_input(command)
                        },
                    )? {
                        return Ok(ControlFlow::Break(true));
                    }
                    Ok(ControlFlow::Break(false))
                },
                |status| match status {
                    DeliveryStatus::ReplayCompleted(_) => Ok(ControlFlow::Break(false)),
                    _ => Err(CommandError::input_invariant()),
                },
            )?;
            if selected {
                return Ok(());
            }
        }
    }
}
