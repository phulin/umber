//! Ephemeral command-processor orchestration.

mod alignment;
mod expand;
mod next;
mod status;

use tex_state::CommandContext;

use crate::{CommandHostContext, CommandRuntime, CommandState, DeliveryStamp};

pub(crate) use alignment::AlignmentDeliveryState;
#[cfg(test)]
pub(crate) use alignment::{ActiveCellDelivery, SuspendedAlignment};
pub(crate) use expand::ExpansionState;
#[cfg(test)]
pub(crate) use status::{
    AbsorbingContext, AlignmentId, AlignmentScanContext, ArgumentBuilderId, ConditionId,
    DefinitionContext, MatchingContext, ScannerWarning, SkippingContext, TokenBuilderId,
};
pub(crate) use status::{ScannerState, ScannerStatus};

/// Optional observation sink for semantic command deliveries.
pub(crate) trait CommandObserver {}

/// Borrow-only capability facade for one bounded executor operation.
///
/// The processor owns no semantic or host state and therefore cannot outlive
/// the borrows that construct it. All future raw delivery, expansion,
/// scanners, conditionals, and primitives operate through this single
/// aggregate facade.
#[allow(dead_code)] // later canonical command operations consume every capability
pub struct CommandProcessor<'a> {
    command: &'a mut CommandState,
    runtime: &'a mut CommandRuntime,
    state: CommandContext<'a>,
    host: CommandHostContext<'a>,
    observer: Option<&'a mut dyn CommandObserver>,
    /// Only the immediately preceding raw delivery may be backed up. This is
    /// processor-local so stamps cannot survive a snapshot or a new episode.
    last_delivery: Option<DeliveryStamp>,
    next_delivery_sequence: u64,
}

impl<'a> CommandProcessor<'a> {
    /// Borrows every ownership domain needed by one command operation.
    #[must_use]
    pub fn new(
        command: &'a mut CommandState,
        runtime: &'a mut CommandRuntime,
        state: CommandContext<'a>,
        host: CommandHostContext<'a>,
    ) -> Self {
        Self {
            command,
            runtime,
            state,
            host,
            observer: None,
            last_delivery: None,
            next_delivery_sequence: 0,
        }
    }
}
