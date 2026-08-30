//! Canonical raw command delivery.
//!
//! This is the sole scalar path from input levels to `CurrentCommand<G>`, after
//! TeX.web §341 (`get_next`).  Later scanner and alignment milestones extend
//! the two explicit entry points below; they do not add another lexical path.

use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use crate::command::{CurrentCommand, DeliveryStamp, ResolvedCommand};
use crate::error::CommandError;
use crate::input::{InputLevel, InputTopTransition};
// tex.web §303's `name` classification only reaches an observation payload.
use crate::CommandReplayDelivery;
use crate::input::SourceNameClass;

use super::CommandProcessor;
use super::{
    AlignmentInterceptionPolicy, DeliveryErrorSlot, DeliveryFailed, DeliveryMode, DeliveryPolicy,
    ReplayCompletionPolicy,
};

/// TeX82 §345's invalid source-character report.
///
/// The tokenizer has already consumed the character when this is recorded;
/// raw delivery reports it with deletions disabled and then restarts at the
/// following character instead of producing a token for it.
const INVALID_SOURCE_CHARACTER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0345;

use super::end_input::{RetirementHandoff, SourceExhaustionStatus};

use crate::observation::{
    AlignmentRecord, CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation,
    CommandProvenance, InputReason, InputRecord, InputTransition, observed_token,
};

impl<G> CommandProcessor<'_, '_, G> {
    /// Delivers one unexpanded raw command through canonical `get_next`.
    pub fn get_next(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.get_next_into(&mut destination)? {
            super::DeliveryStatus::End => Ok(None),
            super::DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary raw delivery returns only commands"),
        }
    }
    /// Delivers one raw command directly into caller-provided final storage.
    pub fn get_next_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        let delivery = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Raw,
                replay_completion: ReplayCompletionPolicy::Consume,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            destination,
        )?;
        debug_assert!(matches!(
            delivery,
            super::DeliveryStatus::End | super::DeliveryStatus::Command
        ));
        Ok(delivery)
    }
    /// Delivers one raw command or an executor-owned stored-episode
    /// completion. This is the raw counterpart of
    /// [`Self::get_x_token_with_replay_completion`].
    pub fn get_next_with_replay_completion(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let delivery = self.get_next_with_replay_completion_into(&mut destination)?;
        Ok(match delivery {
            super::DeliveryStatus::End => None,
            super::DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            super::DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("raw replay-aware delivery has no expanded event"),
        })
    }
    /// Delivers raw replay-aware input into caller-provided command storage.
    pub fn get_next_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        let delivery = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Raw,
                replay_completion: ReplayCompletionPolicy::Surface,
                alignment_interception: AlignmentInterceptionPolicy::None,
            },
            destination,
        )?;
        debug_assert!(matches!(
            delivery,
            super::DeliveryStatus::End
                | super::DeliveryStatus::Command
                | super::DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(delivery)
    }
    /// Delivers the raw token following TeX's backtick character-code
    /// introducer.
    ///
    /// §442 reads it with `get_token`, so the delivery is an ordinary raw
    /// command whose identity is its own category code -- the *scanner's*
    /// later interpretation of `cur_chr` is category-independent, the
    /// delivery is not. This observed nothing of its own until
    /// `umber2-johp.141`: it used to force the observed spelling to
    /// `other_char`, which existed only to feed a spelling-derived command
    /// name in the transport and silently masked whatever category code the
    /// engine actually held.
    ///
    /// It is `get_token`, not `get_next`, for the further reason §365 gives:
    /// `get_token` is one of the two places TeX82 clears
    /// `no_new_control_sequence`, so `` \`\newname `` enters `newname` in the
    /// hash table exactly as any other `get_token` reader would.
    pub(crate) fn get_next_character_code(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.get_token_into(&mut destination)? {
            super::DeliveryStatus::End => return Ok(None),
            super::DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        if let Some(command) = &destination
            && matches!(
                command.spelling().semantic_token(),
                Token::Char {
                    cat: Catcode::BeginGroup | Catcode::EndGroup,
                    ..
                }
            )
        {
            // TeX82 §442 immediately cancels `get_next`'s brace update
            // when a brace token supplies an alphabetic character constant.
            // The token is consumed as a character code, not as grouping
            // material, so a following alignment delimiter must still see
            // the entry's original `align_state`.
            self.command
                .alignment
                .undo_delivery(command.alignment_adjustment());
        }
        Ok(destination)
    }
    /// Delivers one raw token for consumers which canonically permit a new
    /// source control-sequence spelling.
    pub fn get_token(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.get_token_into(&mut destination)? {
            super::DeliveryStatus::End => Ok(None),
            super::DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
    }
    /// Delivers one raw token directly into caller-provided final storage.
    pub fn get_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        debug_assert!(!self.create_source_control_sequences);
        self.create_source_control_sequences = true;
        let delivery = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Raw,
                replay_completion: ReplayCompletionPolicy::Consume,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            destination,
        );
        self.create_source_control_sequences = false;
        let delivery = delivery?;
        debug_assert!(matches!(
            delivery,
            super::DeliveryStatus::End | super::DeliveryStatus::Command
        ));
        Ok(delivery)
    }
    /// Runs the one TeX82 §341 next-command pipeline in the caller's final
    /// slot: authoritative raw input resolves its resident packed word into
    /// that slot, then one delivery-policy settlement runs. Cold input
    /// transitions re-enter this loop only after their slot typestate borrow
    /// has ended.
    pub(super) fn next_command_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        error: &mut DeliveryErrorSlot,
    ) -> Result<super::DeliveryStatus, DeliveryFailed> {
        // Expanded delivery keeps this same caller-owned value across every
        // synchronous expansion. The fused input/meaning write overwrites all
        // delivery facts and the prior meaning, so rebuilding an empty command
        // between tokens would only duplicate state movement.
        if destination.is_none() {
            *destination = Some(CurrentCommand::empty());
        }
        loop {
            if let Some(episode) = self.take_ready_replay_completion() {
                destination.take();
                return Ok(super::DeliveryStatus::ReplayCompleted(episode));
            }
            let transition = self
                .command
                .next_raw_into(
                    self.state,
                    self.create_source_control_sequences,
                    destination
                        .as_mut()
                        .expect("next-command pipeline owns its reusable command slot")
                        .empty_for_raw_delivery(),
                    self.next_delivery_sequence,
                )
                .map_err(|()| CommandError::input_invariant());
            let transition = match transition {
                Ok(transition) => transition,
                Err(failure) => {
                    destination.take();
                    return error.fail(failure);
                }
            };
            let (resolved, meaning_lookup) = match transition {
                InputTopTransition::Empty => {
                    observe!(
                        self,
                        CommandObservation::Input(InputRecord {
                            transition: InputTransition::Stop,
                            reason: InputReason::Source,
                            source_name: Some(SourceNameClass::Terminal),
                            source: None,
                            level: 0,
                            position: 0,
                        }),
                    );
                    let restarts = match self.raw_end_restarts() {
                        Ok(restarts) => restarts,
                        Err(failure) => return error.fail(failure),
                    };
                    if restarts {
                        continue;
                    }
                    destination.take();
                    return Ok(super::DeliveryStatus::End);
                }
                InputTopTransition::Delivered {
                    resolved,
                    meaning_lookup,
                } => (resolved, meaning_lookup),
                InputTopTransition::ParameterPushed(parameter_level) => {
                    observe!(
                        self,
                        CommandObservation::Input(InputRecord {
                            transition: InputTransition::Push,
                            reason: InputReason::Parameter,
                            source_name: None,
                            source: None,
                            level: parameter_level.0,
                            position: 0,
                        }),
                    );
                    continue;
                }
                InputTopTransition::InvalidCharacter => {
                    self.report_recoverable(
                        INVALID_SOURCE_CHARACTER_DIAGNOSTIC,
                        "Text line contains an invalid character".into(),
                        &[
                            "A funny symbol that I can't read has just been input.",
                            "Continue, and I'll forget that it ever happened.",
                        ],
                    );
                    continue;
                }
                InputTopTransition::NeedLine(identity) => {
                    let line = match self.acquire_source_line(true) {
                        Ok(line) => line,
                        Err(failure) => return error.fail(failure),
                    };
                    let exhausted = if line.is_none() {
                        match self.finish_exhausted_source(identity) {
                            Ok(status) => matches!(status, SourceExhaustionStatus::End),
                            Err(failure) => return error.fail(failure),
                        }
                    } else {
                        false
                    };
                    if exhausted {
                        let restarts = match self.raw_end_restarts() {
                            Ok(restarts) => restarts,
                            Err(failure) => return error.fail(failure),
                        };
                        if restarts {
                            continue;
                        }
                        destination.take();
                        return Ok(super::DeliveryStatus::End);
                    }
                    continue;
                }
                InputTopTransition::SourceExhausted(identity) => {
                    let exhausted = match self.finish_exhausted_source(identity) {
                        Ok(status) => matches!(status, SourceExhaustionStatus::End),
                        Err(failure) => return error.fail(failure),
                    };
                    if exhausted {
                        let restarts = match self.raw_end_restarts() {
                            Ok(restarts) => restarts,
                            Err(failure) => return error.fail(failure),
                        };
                        if restarts {
                            continue;
                        }
                        destination.take();
                        return Ok(super::DeliveryStatus::End);
                    }
                    continue;
                }
                InputTopTransition::TokenExhausted(identity) => {
                    let Some((index, active_source)) =
                        self.command
                            .input
                            .levels
                            .last()
                            .and_then(|level| match level {
                                InputLevel::Tokens(cursor) if cursor.identity() == identity => {
                                    Some((cursor.frame.position(), cursor.frame.source_id()))
                                }
                                InputLevel::MacroArgument(cursor)
                                    if cursor.identity() == identity =>
                                {
                                    Some((cursor.frame.position(), cursor.frame.source_id()))
                                }
                                _ => None,
                            })
                    else {
                        return error.fail(CommandError::input_invariant());
                    };
                    let handoff = match self.retire_input_top(identity) {
                        Ok(handoff) => handoff,
                        Err(failure) => return error.fail(failure),
                    };
                    match handoff {
                        RetirementHandoff::Stop => {
                            let restarts = match self.raw_end_restarts() {
                                Ok(restarts) => restarts,
                                Err(failure) => return error.fail(failure),
                            };
                            if restarts {
                                continue;
                            }
                            destination.take();
                            return Ok(super::DeliveryStatus::End);
                        }
                        RetirementHandoff::Completed => continue,
                        RetirementHandoff::Continue => continue,
                        RetirementHandoff::EndV(level) => destination
                            .as_mut()
                            .expect("next-command pipeline owns its reusable command slot")
                            .empty_for_raw_delivery()
                            .write_resolved_delivery(
                                TokenWord::pack(self.state.frozen_end_template_token()),
                                OriginId::UNKNOWN,
                                level.0,
                                u64::from(index),
                                self.next_delivery_sequence,
                                None,
                                active_source,
                                false,
                                None,
                                false,
                                self.state,
                            ),
                    }
                }
            };

            let delivery_stamp = resolved.as_ref().delivery_stamp();
            self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
            let scanner = !matches!(
                self.command.scanner.status(),
                crate::processor::ScannerStatus::Normal
            );
            self.record_raw_delivery(scanner, meaning_lookup);
            if let Err(failure) = self.apply_delivery_rules(resolved, delivery_stamp) {
                destination.take();
                return error.fail(failure);
            }
            return Ok(super::DeliveryStatus::Command);
        }
    }
    /// Applies the remaining §341 delivery rules to one resolved command.
    /// Resolution has ended its dense meaning borrow before this function can
    /// perform recovery, alignment mutation, or observation.
    fn apply_delivery_rules(
        &mut self,
        mut resolved: ResolvedCommand<'_, G>,
        delivery_stamp: DeliveryStamp,
    ) -> Result<(), CommandError> {
        if resolved.as_ref().suppresses_expandable_control_sequence() {
            resolved.as_mut().suppress_expandable();
        }
        // Outer-validity recovery canonically backs up this exact raw
        // delivery before substituting its recovery space.
        self.last_delivery = Some(delivery_stamp);
        self.check_outer_validity_entry(resolved.as_mut())?;
        let previous_align_state = self.command.alignment.align_state;
        self.command.classify_alignment_delivery(resolved.as_mut());
        let command = resolved.as_ref();
        let adjustment = command.alignment_adjustment();
        if self.command.alignment.active_alignment.is_some()
            && !matches!(
                adjustment,
                crate::processor::AlignmentDeliveryAdjustment::None
            )
        {
            self.observe(CommandObservation::Alignment(AlignmentRecord {
                transition: match adjustment {
                    crate::processor::AlignmentDeliveryAdjustment::BeginGroup => "begin_group",
                    crate::processor::AlignmentDeliveryAdjustment::EndGroup => "end_group",
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_) => "delimiter",
                    crate::processor::AlignmentDeliveryAdjustment::None => unreachable!(),
                },
                alignment: self
                    .command
                    .alignment
                    .active_alignment
                    .map(|identity| identity.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: if matches!(
                    adjustment,
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                ) {
                    previous_align_state
                } else {
                    self.command.alignment.align_state
                },
                delimiter: match adjustment {
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(delimiter) => {
                        Some(delimiter.observation_name())
                    }
                    _ => None,
                },
                previous_align_state: matches!(
                    adjustment,
                    crate::processor::AlignmentDeliveryAdjustment::BeginGroup
                        | crate::processor::AlignmentDeliveryAdjustment::EndGroup
                )
                .then_some(previous_align_state),
            }));
        }
        if !matches!(
            adjustment,
            crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
        ) {
            self.observe_raw_delivery(command);
        }
        Ok(())
    }
    pub(crate) fn observed_token(
        &self,
        token: TracedTokenWord,
    ) -> crate::observation::ObservedToken {
        observed_token(
            token,
            |symbol| self.state.resolve(symbol).to_owned(),
            |frozen| self.state.frozen_primitive_name(frozen).map(str::to_owned),
        )
    }
    pub(crate) fn observed_command_spelling(
        &self,
        command: &CurrentCommand<G>,
    ) -> crate::observation::ObservedToken {
        if let Some(symbol) = command.control_sequence() {
            // §353's `get_next` resolves an active character through its own
            // `active_base + c` control-sequence cell and records that cell
            // in `cur_cs`, so §365's `cur_tok` is `cs_token_flag + cur_cs`.
            // Observations expose that identity at the current-command
            // boundary, just as they do for escaped control sequences.  The
            // raw token spelling remains available on `CurrentCommand<G>` for
            // token-sensitive consumers.
            crate::observation::ObservedToken::ControlSequence(
                self.state.resolve(symbol).to_owned(),
            )
        } else if command.spelling().semantic_token().is_frozen_end_template()
            || command.spelling().semantic_token().is_frozen_endv()
        {
            // TeX82 stores both inaccessible template sentinels in distinct
            // frozen control-sequence slots whose texts are `endtemplate`
            // (TeX.web §780). `get_next` therefore exposes that control
            // sequence identity at the raw boundary, while §380's
            // `get_x_token` changes only its effective command to `endv` --
            // and §380's `x_token` does not even do that, reaching §375's
            // separate `frozen_endv` token through §366 `expand` instead.
            crate::observation::ObservedToken::ControlSequence("endtemplate".into())
        } else if matches!(command.spelling().semantic_token(), Token::Frozen(_))
            && matches!(
                command.meaning(),
                tex_state::ResolvedMeaning::Static(Meaning::Relax)
            )
        {
            // TeX82's observer presents the inaccessible frozen `\relax`
            // inserted by incomplete-conditional recovery as `\relax`.
            // A `\noexpand` target has the same effective meaning but retains
            // its original control-sequence spelling.
            crate::observation::ObservedToken::ControlSequence("relax".into())
        } else if matches!(command.spelling().semantic_token(), Token::Frozen(_))
            && let tex_state::ResolvedMeaning::Static(meaning) = command.meaning()
            && let Some(name) = self.state.primitive_name(meaning)
        {
            crate::observation::ObservedToken::ControlSequence(name.into())
        } else {
            self.observed_token(command.spelling())
        }
    }
    fn observe_raw_delivery(&mut self, command: &CurrentCommand<G>) {
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
                boundary: CommandDeliveryBoundary::Raw,
                spelling,
                command: command_name,
                command_operand,
                semantic_operand,
                provenance: CommandProvenance::from_stamp(
                    command.delivery_stamp(),
                    command.origin(),
                    command.direct_source_provenance(),
                ),
            })
        });
    }
}
