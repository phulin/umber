//! TeX82 §494's completing consumer of the canonical raw input reader.

use super::ReadSite;
use crate::command::CurrentCommand;
use crate::conditionals::{ConditionalDelimiter, ConditionalKind, PassTextStop};
use crate::{CommandError, CommandProcessor, DeliveryStatus};
use std::cell::Cell;
use std::ops::ControlFlow;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningWord, StaticCommandClass};
use tex_state::token::Catcode;

/// Only an unmatched delimiter completes §494. Other commands have no product.
#[inline(always)]
fn consume_conditional(
    primitive: ExpandablePrimitive,
    nested: &Cell<u32>,
) -> Option<ConditionalDelimiter> {
    if ConditionalKind::from_primitive(primitive).is_some() {
        nested.set(nested.get().saturating_add(1));
        return None;
    }
    let delimiter = match primitive {
        ExpandablePrimitive::Fi => ConditionalDelimiter::Fi,
        ExpandablePrimitive::Else => ConditionalDelimiter::Else,
        ExpandablePrimitive::Or => ConditionalDelimiter::Or,
        _ => return None,
    };
    if nested.get() == 0 {
        return Some(delimiter);
    }
    if delimiter == ConditionalDelimiter::Fi {
        nested.set(nested.get() - 1);
    }
    None
}

#[inline(always)]
fn consume_meaning<G>(
    meaning: &MeaningWord<G>,
    nested: &Cell<u32>,
) -> Option<ConditionalDelimiter> {
    let word = meaning.scalar_word()?;
    if Meaning::runtime_word_class(word) != StaticCommandClass::Expandable {
        return None;
    }
    consume_conditional(
        ExpandablePrimitive::from_operand(Meaning::runtime_word_operand(word))?,
        nested,
    )
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    pub(crate) fn consume_skipped_text(&mut self) -> Result<PassTextStop, CommandError> {
        let result = if self.is_observed() {
            self.consume_skipped_text_in::<true>()
        } else {
            self.consume_skipped_text_in::<false>()
        };
        match result {
            Ok(delimiter) => Ok(PassTextStop {
                delimiter,
                nested_conditions: 0,
            }),
            Err(failure) => self.fail_hot_expanded_delivery(
                &mut None,
                self.command.transient.active_expansion_depth,
                failure,
            ),
        }
    }

    fn consume_skipped_text_in<const OBSERVED: bool>(
        &mut self,
    ) -> Result<ConditionalDelimiter, CommandError> {
        let nested = Cell::new(0_u32);
        self.consume_input::<_, _, _>(
            self.create_source_control_sequences,
            &mut (),
            false,
            &|_, suppress, mode| !OBSERVED && !suppress && !mode.requires_persistent_settlement(),
            &mut |_| {},
            &mut |_, state, fuel, alignment, word, _, _, kind, _mode| {
                let literal = word.literal_catcode();
                let stop = if literal.is_some_and(|cat| cat != Catcode::Active) {
                    None
                } else {
                    let mut meaning = MeaningWord::scalar(Meaning::UNDEFINED_WORD);
                    state.write_packed_token_command_into(word, &mut meaning);
                    if meaning.is_outer() {
                        return ControlFlow::Continue(Err(()));
                    }
                    consume_meaning(&meaning, &nested)
                };
                alignment.account_literal_catcode(literal);
                #[cfg(feature = "profiling")]
                fuel.record_raw_run(_mode.scanner_active(), kind, 1);
                #[cfg(not(feature = "profiling"))]
                let _ = (fuel, kind);
                stop.map_or(ControlFlow::Continue(Ok(())), ControlFlow::Break)
            },
            &mut |_, processor, word, origin, suppress, site, kind, _| {
                let literal = word.literal_catcode();
                let mut meaning = None;
                let mut lookup = false;
                let outer = if !OBSERVED && literal.is_some_and(|cat| cat != Catcode::Active) {
                    false
                } else {
                    let (resolved, control_sequence) = processor.resolve_token_word(word);
                    lookup = word.requires_meaning_lookup();
                    let outer = resolved.is_outer();
                    meaning = Some((resolved, control_sequence));
                    outer
                };
                processor.record_consumed_word(kind, lookup);
                let stop = if OBSERVED
                    || processor
                        .command
                        .delivery_mode
                        .requires_semantic_settlement(suppress, outer)
                {
                    let (meaning, control_sequence) =
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
                        control_sequence,
                    );
                    processor.settle_hot_delivery_in::<OBSERVED>(&mut command, literal)?;
                    if matches!(
                        command.alignment_adjustment(),
                        super::super::AlignmentDeliveryAdjustment::Delimiter(_)
                    ) {
                        processor.begin_scalar_alignment_v_template(&command)?;
                        return Ok(ControlFlow::Continue(()));
                    }
                    match command.static_meaning() {
                        Some(Meaning::ExpandablePrimitive(primitive)) => {
                            consume_conditional(primitive, &nested)
                        }
                        _ => None,
                    }
                } else {
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
                    meaning
                        .as_ref()
                        .and_then(|(meaning, _)| consume_meaning(meaning, &nested))
                };
                Ok(stop.map_or(ControlFlow::Continue(()), ControlFlow::Break))
            },
            |status| match status {
                DeliveryStatus::ReplayCompleted(_) => Ok(ControlFlow::Continue(())),
                DeliveryStatus::End => Err(CommandError::input_invariant()),
                _ => unreachable!("raw input transitions return only EOF or replay completion"),
            },
        )
    }
}
