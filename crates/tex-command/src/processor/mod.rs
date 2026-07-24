//! Ephemeral command-processor orchestration.

mod alignment;
mod expand;
mod next;
pub(crate) mod status;

use tex_state::CommandContext;

use crate::{CommandHostContext, CommandRuntime, CommandState, DeliveryStamp};

#[cfg(any(test, feature = "instrumentation"))]
use crate::observation::CommandObserver;

#[cfg(test)]
pub(crate) use alignment::{ActiveCellDelivery, SuspendedAlignment};
pub use alignment::{
    AlignmentCellTemplates, AlignmentDelivery, AlignmentDeliveryEvent, AlignmentIdentity,
    AlignmentLifecycleError, AlignmentRequest, AlignmentRequestResult,
};
pub(crate) use alignment::{AlignmentDeliveryAdjustment, AlignmentDeliveryState};
pub(crate) use expand::ExpansionState;
#[cfg(test)]
pub(crate) use status::{
    AbsorbingContext, AlignmentId, AlignmentScanContext, ArgumentBuilderId, ConditionId,
    DefinitionContext, MatchingContext, ScannerWarning, SkippingContext, TokenBuilderId,
};
pub(crate) use status::{ScannerState, ScannerStatus};

/// Borrow-only capability facade for one bounded executor operation.
///
/// The processor owns no semantic or host state and therefore cannot outlive
/// the borrows that construct it. All future raw delivery, expansion,
/// scanners, conditionals, and primitives operate through this single
/// aggregate facade.
#[allow(dead_code)] // later canonical command operations consume every capability
pub struct CommandProcessor<'a> {
    pub(crate) command: &'a mut CommandState,
    runtime: &'a mut CommandRuntime,
    pub(crate) state: CommandContext<'a>,
    pub(crate) host: CommandHostContext<'a>,
    #[cfg(any(test, feature = "instrumentation"))]
    observer: Option<&'a mut dyn CommandObserver>,
    /// Only the immediately preceding raw delivery may be backed up. This is
    /// processor-local so stamps cannot survive a snapshot or a new episode.
    last_delivery: Option<DeliveryStamp>,
    next_delivery_sequence: u64,
    /// Set only by canonical outer-validity recovery while a scalar macro
    /// matcher owns `ScannerStatus::Matching`.
    pub(crate) outer_recovered_while_matching: bool,
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
            #[cfg(any(test, feature = "instrumentation"))]
            observer: None,
            last_delivery: None,
            next_delivery_sequence: 0,
            outer_recovered_while_matching: false,
        }
    }

    /// Installs a non-fallible semantic observer for this bounded processor
    /// episode. This exists only in tests and explicit instrumentation builds.
    #[cfg(any(test, feature = "instrumentation"))]
    #[must_use]
    pub fn with_observer(mut self, observer: &'a mut dyn CommandObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    #[cfg(any(test, feature = "instrumentation"))]
    pub(crate) fn observe(&mut self, observation: crate::observation::CommandObservation) {
        if let Some(observer) = self.observer.as_deref_mut() {
            observer.committed(observation);
        }
    }
}
