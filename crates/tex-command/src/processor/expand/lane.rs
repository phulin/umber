//! The command lane's expanded delivery (`docs/command_lane.md`).
//!
//! This is main-control preflight without its final materialization: the
//! delivered command stays in the compact hot form the delivery loops
//! already own, and becomes a [`CurrentCommand`] only when the lane hands it
//! to the admitted run.

use super::*;

/// Caller-owned slot for one command delivered to the command lane.
///
/// The command stays compact while the lane classifies and settles it.
/// [`Self::take_current`] materializes the ordinary command the admitted
/// run's own delivery would have produced.
#[derive(Debug)]
pub struct LaneCommandSlot<G> {
    command: Option<HotCommand<G>>,
}

impl<G> Default for LaneCommandSlot<G> {
    fn default() -> Self {
        Self { command: None }
    }
}

impl<G> LaneCommandSlot<G> {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.command.is_none()
    }

    /// The delivered command's meaning.
    ///
    /// # Panics
    ///
    /// Panics when the slot is empty.
    #[must_use]
    pub fn meaning(&self) -> ResolvedMeaning<G> {
        self.command
            .as_ref()
            .expect("lane slot holds a delivered command")
            .resolved_meaning()
    }

    /// The delivered command's token origin.
    ///
    /// # Panics
    ///
    /// Panics when the slot is empty.
    #[must_use]
    pub fn origin(&self) -> OriginId {
        self.command
            .as_ref()
            .expect("lane slot holds a delivered command")
            .origin()
    }

    /// Drops the delivered command after the lane settled it.
    pub fn clear(&mut self) {
        self.command = None;
    }

    /// Materializes the delivered command, leaving the slot empty.
    pub fn take_current(&mut self) -> Option<CurrentCommand<G>> {
        self.command.take().map(|command| command.materialize())
    }
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Delivers one command exactly as [`Self::preflight_command_into`]
    /// does, but leaves an ordinary command compact in `slot`.
    ///
    /// The lane runs unobserved, so the expanded delivery observation that
    /// preflight would publish is empty. The slot holds a command exactly
    /// when preflight's destination would.
    pub fn lane_command_into(
        &mut self,
        slot: &mut LaneCommandSlot<G>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(!self.is_observed(), "the command lane runs unobserved");
        debug_assert!(slot.command.is_none());
        let hot = &mut slot.command;
        loop {
            let status = match self.read_expansion_candidate::<false, false, true>(hot) {
                Ok(ExpansionCandidate::ExpandedMacro) => self.expanded_next_hot(hot, None),
                Ok(ExpansionCandidate::Command) => {
                    let command = hot.as_ref().ok_or_else(CommandError::input_invariant)?;
                    match classify_hot_command(command) {
                        ExpandedCommandAction::Return => return Ok(DeliveryStatus::Command),
                        ExpandedCommandAction::EndTemplate => {
                            if matches!(
                                command.alignment_adjustment(),
                                crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                            ) {
                                Ok(DeliveryStatus::AlignmentEndTemplate)
                            } else {
                                hot.take();
                                self.insert_frozen_endv()?;
                                self.expanded_next_hot(hot, None)
                            }
                        }
                        _ if command.command_word().is_main_loop_character() => {
                            return Ok(DeliveryStatus::Command);
                        }
                        action => self.expanded_next_hot(hot, Some(action)),
                    }
                }
                Ok(ExpansionCandidate::Finished(status)) => {
                    hot.take();
                    return Ok(status);
                }
                Err(failure) => {
                    return self.fail_hot_expanded_delivery(
                        hot,
                        self.command.transient.active_expansion_depth,
                        failure,
                    );
                }
            };
            match status {
                Ok(DeliveryStatus::AlignmentEndTemplate) => {
                    let command = hot
                        .take()
                        .ok_or_else(CommandError::input_invariant)?
                        .materialize();
                    self.begin_scalar_alignment_v_template(&command)?;
                }
                Ok(
                    DeliveryStatus::Command
                    | DeliveryStatus::PendingExpanded
                    | DeliveryStatus::AlignmentClosingBrace,
                ) => return Ok(DeliveryStatus::Command),
                Ok(
                    status @ (DeliveryStatus::End
                    | DeliveryStatus::ReplayCompleted(_)
                    | DeliveryStatus::CharacterRun),
                ) => {
                    hot.take();
                    return Ok(status);
                }
                Ok(status) => return Ok(status),
                Err(error) => {
                    hot.take();
                    return Err(error);
                }
            }
        }
    }
}
