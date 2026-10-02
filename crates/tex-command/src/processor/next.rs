//! Canonical raw-command entry points and observation.
//!
//! TeX.web §341 (`get_next`) enters the fused destination-directed state
//! machine owned by `expand.rs`. Later scanner and alignment milestones extend
//! the explicit cold entry wrappers below; they do not add another lexical
//! path.

use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, ResolvedMeaning};
use tex_state::token::{Token, TracedTokenWord};

use crate::command::CurrentCommand;
use crate::error::CommandError;

use super::CommandProcessor;

use crate::observation::{
    AlignmentRecord, CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation,
    CommandProvenance, observed_token,
};

impl<G> CommandProcessor<'_, '_, G> {
    /// Delivers one raw command directly into caller-provided final storage.
    pub fn get_next_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        loop {
            let status = self.raw_next(destination)?;
            if let Some(status) = self.settle_raw_step(status, destination)? {
                return Ok(status);
            }
        }
    }

    /// Shared continuation after one compact raw read, whether the word came
    /// from scalar delivery or a resident run's already-advanced boundary.
    /// `None` requests another raw read after replay or alignment admission.
    #[inline(always)]
    pub(super) fn settle_raw_step(
        &mut self,
        status: super::DeliveryStatus,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<Option<super::DeliveryStatus>, CommandError> {
        match status {
            super::DeliveryStatus::ReplayCompleted(_) => Ok(None),
            super::DeliveryStatus::Command => {
                let delimiter = destination.as_ref().is_some_and(|command| {
                    matches!(
                        command.alignment_adjustment(),
                        super::AlignmentDeliveryAdjustment::Delimiter(_)
                    )
                });
                if delimiter {
                    let command = destination
                        .take()
                        .ok_or_else(CommandError::input_invariant)?;
                    self.begin_scalar_alignment_v_template(&command)?;
                    Ok(None)
                } else {
                    Ok(Some(super::DeliveryStatus::Command))
                }
            }
            super::DeliveryStatus::End => Ok(Some(super::DeliveryStatus::End)),
            _ => unreachable!("compact raw delivery has no character or expanded event"),
        }
    }
    /// Delivers raw replay-aware input into caller-provided command storage.
    pub fn get_next_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        let delivery = self.raw_next_with_replay_completion(destination)?;
        debug_assert!(matches!(
            delivery,
            super::DeliveryStatus::End
                | super::DeliveryStatus::Command
                | super::DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(delivery)
    }
    /// Delivers one raw token directly into caller-provided final storage.
    pub fn get_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        debug_assert!(!self.create_source_control_sequences);
        self.create_source_control_sequences = true;
        let delivery = self.get_next_into(destination);
        self.create_source_control_sequences = false;
        delivery
    }

    /// Compact resident-command observation.  Delivery
    /// settlement may already own a hot command; observing it in place keeps
    /// the scanner/expansion path free of a rich command projection.
    pub(super) fn observe_resident_hot_command(&mut self, command: &CurrentCommand<G>) {
        let adjustment = command.alignment_adjustment();
        let previous_align_state = match adjustment {
            crate::processor::AlignmentDeliveryAdjustment::BeginGroup => {
                self.command.alignment.align_state - 1
            }
            crate::processor::AlignmentDeliveryAdjustment::EndGroup => {
                self.command.alignment.align_state + 1
            }
            _ => self.command.alignment.align_state,
        };
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
            self.observe_raw_hot_delivery(command);
        }
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
        observed_command_spelling_for(
            self.state,
            command.spelling(),
            command.control_sequence(),
            command.meaning(),
        )
    }
    fn observe_raw_hot_delivery(&mut self, command: &CurrentCommand<G>) {
        observe!(self, {
            #[cfg(test)]
            {}
            let (command_name, command_operand) =
                crate::observation::canonical_delivery_identity_for_profile(
                    self.command.profile(),
                    command.identity(),
                    command.meaning(),
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
                provenance: CommandProvenance::new(
                    self.observed_delivery_coordinate(),
                    self.current_delivery_sequence(),
                    command.origin(),
                    self.direct_source_provenance(command),
                ),
            })
        });
    }
}

/// Projects the canonical spelling shared by rich and compact deliveries.
/// Keeping this at the command boundary projects only the consuming spelling
/// merely to retain a cold diagnostic token, while ensuring both delivery
/// forms apply the same frozen-sentinel rules.
#[inline]
pub(super) fn observed_command_spelling_for<G>(
    state: &tex_state::CommandContext<'_, G>,
    spelling: TracedTokenWord,
    control_sequence: Option<Symbol>,
    meaning: ResolvedMeaning<G>,
) -> crate::observation::ObservedToken {
    if let Some(symbol) = control_sequence {
        // §353's `get_next` resolves an active character through its own
        // `active_base + c` control-sequence cell and records that cell in
        // `cur_cs`, so §365's `cur_tok` is `cs_token_flag + cur_cs`.
        crate::observation::ObservedToken::ControlSequence(state.resolve(symbol).to_owned())
    } else if spelling.semantic_token().is_frozen_end_template()
        || spelling.semantic_token().is_frozen_endv()
    {
        // TeX82 stores both inaccessible template sentinels in distinct
        // frozen control-sequence slots whose texts are `endtemplate`
        // (TeX.web §780). Their canonical observed spelling is shared even
        // though expanded delivery changes the effective command to `endv`.
        crate::observation::ObservedToken::ControlSequence("endtemplate".into())
    } else if matches!(spelling.semantic_token(), Token::Frozen(_))
        && matches!(meaning, ResolvedMeaning::Static(Meaning::Relax))
    {
        // TeX82's observer presents the inaccessible frozen `\relax`
        // inserted by incomplete-conditional recovery as `\relax`.
        crate::observation::ObservedToken::ControlSequence("relax".into())
    } else if matches!(spelling.semantic_token(), Token::Frozen(_))
        && let ResolvedMeaning::Static(meaning) = meaning
        && let Some(name) = state.primitive_name(meaning)
    {
        crate::observation::ObservedToken::ControlSequence(name.into())
    } else {
        observed_token(
            spelling,
            |symbol| state.resolve(symbol).to_owned(),
            |frozen| state.frozen_primitive_name(frozen).map(str::to_owned),
        )
    }
}
