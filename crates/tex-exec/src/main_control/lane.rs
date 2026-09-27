//! The command lane: main control's minimal inner loop for its most frequent
//! commands. See `docs/command_lane.md`.
//!
//! The lane is not a second executor. It borrows the admitted run's command
//! processor and settles, in place, only commands whose complete TeX82
//! behavior in an eligible context is already known at delivery. Every other
//! command is left in the episode's command slot exactly as ordinary
//! preflight delivery would leave it, so the admitted run continues with no
//! backup and no second delivery.

use super::*;

/// The admitted-run facts that make a context lane-eligible. They are
/// sampled once per admitted-loop iteration, and no command the lane settles
/// can change any of them.
pub(super) struct LaneContext {
    pub(super) mode: Mode,
    pub(super) raw_main_loop_delivery: bool,
    pub(super) observing: bool,
    pub(super) tracked_region_is_active: bool,
    pub(super) host_boundary_pending: bool,
    pub(super) leader_pending: bool,
    pub(super) characters_pending: bool,
}

impl LaneContext {
    /// Whether the lane may settle commands before the admitted run's
    /// generic dispatch. Math modes keep their mode-specific scan prelude,
    /// and the character main loop owns its own raw lookahead.
    pub(super) const fn is_eligible(&self) -> bool {
        matches!(
            self.mode,
            Mode::Vertical | Mode::InternalVertical | Mode::Horizontal | Mode::RestrictedHorizontal
        ) && !self.raw_main_loop_delivery
            && !self.observing
            && !self.tracked_region_is_active
            && !self.host_boundary_pending
            && !self.leader_pending
            && !self.characters_pending
    }
}

/// Performs ordinary preflight delivery and settles every lane-owned command
/// it delivers before the next one.
///
/// `\relax` is TeX82 §1045's `any_mode(relax): do_nothing`. In an eligible
/// context no character run is pending, so its §1030 word boundary flushes
/// nothing, and it reaches no mode, group, or box arm. Its only effect is
/// consumed input, so it joins the rollback unit of the command that
/// follows it: discarding that unit re-reads a command that does nothing.
/// It still counts as one operation, so the slice limit is unchanged.
///
/// The first command the lane does not own, and every non-command status,
/// is returned in `destination` exactly as ordinary preflight returns it.
pub(super) fn lane_fetch<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    destination: &mut Option<tex_command::CurrentCommand<G>>,
    operations: &mut usize,
    max_operations: usize,
) -> Result<tex_command::DeliveryStatus, tex_command::CommandError> {
    if processor.int_param(IntParam::TRACING_COMMANDS) > 0 {
        return processor.preflight_command_into(destination);
    }
    loop {
        let status = processor.preflight_command_into(destination)?;
        if status != tex_command::DeliveryStatus::Command
            || *operations + 1 >= max_operations
            || processor.has_pending_reports()
        {
            return Ok(status);
        }
        let settles = destination.as_ref().is_some_and(|command| {
            matches!(command.meaning(), ResolvedMeaning::Static(Meaning::Relax))
        });
        if !settles {
            return Ok(status);
        }
        destination.take();
        *operations += 1;
    }
}
