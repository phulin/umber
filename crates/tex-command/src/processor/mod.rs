//! Ephemeral command-processor orchestration.

mod alignment;
mod expand;
mod next;
pub(crate) mod status;

use tex_state::CommandContext;

use crate::{
    CommandHostContext, CommandReplayEpisode, CommandRuntime, CommandState, DeliveryStamp,
};

#[cfg(any(test, feature = "instrumentation"))]
use crate::input::InputLevelId;

#[cfg(any(test, feature = "instrumentation"))]
use crate::observation::CommandObserver;

#[cfg(any(test, feature = "instrumentation"))]
pub use alignment::AlignmentCellFinishObservations;
pub(crate) use alignment::CELL_ALIGN_STATE;
#[cfg(test)]
pub(crate) use alignment::{ActiveCellDelivery, SuspendedAlignment};
pub use alignment::{
    AlignmentCellCompletion, AlignmentCellDelimiter, AlignmentCellTemplates, AlignmentDelivery,
    AlignmentDeliveryEvent, AlignmentIdentity, AlignmentLifecycleError, AlignmentPreamble,
    AlignmentRequest, AlignmentRequestResult, FinishedAlignmentCell,
};
pub(crate) use alignment::{AlignmentDeliveryAdjustment, AlignmentDeliveryState};
pub(crate) use expand::ExpansionState;
pub(crate) use expand::{meaning_text, render_the_value, string_text};
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
    #[cfg(any(test, feature = "instrumentation"))]
    observe_next_raw_as_character_code: bool,
    /// The §53 write scanner registers its replay level here solely to name
    /// that level in detached observation. This is processor-local observer
    /// metadata: raw delivery neither reads replay provenance nor lets this
    /// value affect input semantics.
    #[cfg(any(test, feature = "instrumentation"))]
    immediate_write_retirement: Option<InputLevelId>,
    /// Only the immediately preceding raw delivery may be backed up. This is
    /// processor-local so stamps cannot survive a snapshot or a new episode.
    last_delivery: Option<DeliveryStamp>,
    /// Completion published by raw retirement to the episode-aware expanded
    /// delivery boundary. It is processor-local because retirement itself is
    /// already represented by command state.
    replay_completion: Option<CommandReplayEpisode>,
    /// The non-numeric command that completed the most recent integer scan.
    /// It remains backed up in input; dimension scanning uses the semantic
    /// fact to decide whether that replay is a decimal point or a unit.
    pub(crate) last_integer_terminator: Option<crate::CurrentCommand>,
    next_delivery_sequence: u64,
    /// Set only by canonical outer-validity recovery while a scalar macro
    /// matcher owns `ScannerStatus::Matching`.
    pub(crate) outer_recovered_while_matching: bool,
    /// Set only when terminal EOF invokes TeX82's `check_outer_validity`
    /// recovery while a scalar macro matcher is live. The inserted frozen
    /// `\\par` terminates the failed match, but must not become a visible
    /// §394 `back_error` replay token.
    pub(crate) eof_recovered_while_matching: bool,
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
            #[cfg(any(test, feature = "instrumentation"))]
            observe_next_raw_as_character_code: false,
            #[cfg(any(test, feature = "instrumentation"))]
            immediate_write_retirement: None,
            last_delivery: None,
            replay_completion: None,
            last_integer_terminator: None,
            next_delivery_sequence: 0,
            outer_recovered_while_matching: false,
            eof_recovered_while_matching: false,
        }
    }

    /// Reads a live integer parameter while main control selects an
    /// assignment policy for a completed command operation.
    #[must_use]
    pub fn int_param(&self, parameter: tex_state::env::banks::IntParam) -> i32 {
        self.state.int_param(parameter)
    }

    /// Installs a non-fallible semantic observer for this bounded processor
    /// episode. This exists only in tests and explicit instrumentation builds.
    #[cfg(any(test, feature = "instrumentation"))]
    #[must_use]
    pub fn with_observer(mut self, observer: &'a mut dyn CommandObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Records a completed typed mutation selected by the replay consumer.
    ///
    /// The command processor remains the sole owner of the observer stream;
    /// replay supplies only a value it has already scanned through this
    /// processor and will apply after the processor borrow ends.
    #[cfg(any(test, feature = "instrumentation"))]
    pub fn observe_typed_mutation(&mut self, target: &'static str, value: impl Into<String>) {
        self.observe(crate::observation::CommandObservation::Mutation(
            crate::observation::MutationRecord {
                target,
                value: value.into(),
                key: None,
                tokens: None,
                global: false,
            },
        ));
    }

    #[cfg(any(test, feature = "instrumentation"))]
    pub(crate) fn observe(&mut self, observation: crate::observation::CommandObservation) {
        if let Some(observer) = self.observer.as_deref_mut() {
            observer.committed(observation);
        }
    }

    /// Registers the write-list lifetime established by TeX82 §53's
    /// `write_out`. The scanner owns this classification; raw delivery only
    /// consumes the already-registered observer identity when the level ends.
    #[cfg(any(test, feature = "instrumentation"))]
    pub(crate) fn observe_immediate_write_retirement(&mut self, level: InputLevelId) {
        debug_assert!(self.immediate_write_retirement.is_none());
        self.immediate_write_retirement = Some(level);
    }

    /// Returns whether the just-retired raw level is the §53 write-list level.
    /// This deliberately consumes identity rather than consulting `ReplayTrace`:
    /// trace/provenance explains an input frame but cannot select delivery
    /// observation semantics.
    #[cfg(any(test, feature = "instrumentation"))]
    pub(crate) fn take_immediate_write_retirement(&mut self, level: InputLevelId) -> bool {
        if self.immediate_write_retirement == Some(level) {
            self.immediate_write_retirement = None;
            true
        } else {
            false
        }
    }
}
