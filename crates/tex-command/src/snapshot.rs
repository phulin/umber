//! Command snapshot and durable-summary ownership.

use std::fmt;

use crate::CommandState;
use crate::conditionals::ConditionStack;
use crate::input::InputState;
use crate::macro_call::ParameterState;
use crate::processor::{AlignmentDeliveryState, ExpansionState, ScannerState, ScannerStatus};
use crate::profile::{CommandProfileBoundary, CommandProfileFingerprint, CommandProfileMismatch};
use crate::state::TransientState;

/// Exact owned command-machine state for one executor-step rollback.
///
/// The value contains only [`CommandState`]. Runtime caches, processor
/// borrows, host capabilities, and an ephemeral current command have no field
/// through which they could enter this snapshot.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CommandStateSnapshot {
    state: CommandState,
}

/// Restartable command state published at a named incremental boundary.
///
/// Construction is restricted to [`CommandState::publish_summary`], which
/// proves that every resumable command episode is quiescent. Consequently the
/// summary does not store scanner status, live builders, rollback roots,
/// expansion episodes, or alignment-template identities.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CommandSummary {
    input: InputState,
    parameters: ParameterState,
    conditions: ConditionStack,
    align_state: i32,
    expansion: ExpansionState,
    next_builder_identity: u64,
}

impl CommandStateSnapshot {
    /// Returns the immutable profile identity captured by this snapshot.
    #[must_use]
    pub fn profile_fingerprint(&self) -> CommandProfileFingerprint {
        self.state.profile().fingerprint()
    }
}

impl CommandSummary {
    /// Returns the immutable profile identity captured by this durable summary.
    #[must_use]
    pub fn profile_fingerprint(&self) -> CommandProfileFingerprint {
        self.expansion.profile.fingerprint()
    }
}

/// The first nonquiescent command-state class preventing summary publication.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandSummaryError {
    /// Conditional text is being skipped by `pass_text`.
    ConditionalSkip,
    /// A macro argument matcher is consuming raw input.
    MacroMatch,
    /// A definition token-list scan is incomplete.
    DefinitionScan,
    /// An alignment preamble token-list scan is incomplete.
    AlignmentScan,
    /// Another balanced token-list absorption is incomplete.
    AbsorbingScan,
    /// An expanded-command request still owns the command machine.
    ExpansionActive,
    /// A u- or v-template is still associated with the active cell.
    AlignmentTemplateActive,
    /// An outer alignment delivery context is suspended.
    SuspendedAlignment,
    /// A semantic token builder remains live.
    LiveTokenBuilder,
    /// A temporary rollback root remains live.
    LiveRollbackRoot,
    /// Scanner warning context remains installed despite normal status.
    ScannerWarningContext,
    /// A command diagnostic has not yet crossed the executor boundary.
    PendingSemanticDiagnostic,
}

impl fmt::Display for CommandSummaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ConditionalSkip => "conditional skipping is active",
            Self::MacroMatch => "macro argument matching is active",
            Self::DefinitionScan => "definition scanning is active",
            Self::AlignmentScan => "alignment scanning is active",
            Self::AbsorbingScan => "balanced token absorption is active",
            Self::ExpansionActive => "command expansion is active",
            Self::AlignmentTemplateActive => "alignment template delivery is active",
            Self::SuspendedAlignment => "an alignment delivery context is suspended",
            Self::LiveTokenBuilder => "a semantic token builder is live",
            Self::LiveRollbackRoot => "a temporary rollback root is live",
            Self::ScannerWarningContext => "scanner warning context remains installed",
            Self::PendingSemanticDiagnostic => {
                "a command semantic diagnostic is awaiting executor delivery"
            }
        })
    }
}

impl std::error::Error for CommandSummaryError {}

impl CommandState {
    /// Captures every future-relevant command field for executor-step retry.
    ///
    /// Capture clones retained source and token backing through their owned
    /// command-state representations. It neither consults host policy nor
    /// includes discardable [`crate::CommandRuntime`] data.
    #[must_use]
    pub fn snapshot(&self) -> CommandStateSnapshot {
        CommandStateSnapshot {
            state: self.clone(),
        }
    }

    /// Restores an exact executor-step snapshot without host access.
    pub fn rollback(
        &mut self,
        snapshot: CommandStateSnapshot,
    ) -> Result<(), CommandProfileMismatch> {
        self.profile().validate_fingerprint(
            CommandProfileBoundary::Snapshot,
            snapshot.profile_fingerprint(),
        )?;
        *self = snapshot.state;
        Ok(())
    }

    /// Validates and publishes restartable state for a named boundary.
    pub fn publish_summary(&self) -> Result<CommandSummary, CommandSummaryError> {
        match self.scanner.status() {
            ScannerStatus::Normal => {}
            ScannerStatus::Skipping { .. } => {
                return Err(CommandSummaryError::ConditionalSkip);
            }
            ScannerStatus::Defining { .. } => {
                return Err(CommandSummaryError::DefinitionScan);
            }
            ScannerStatus::Matching { .. } => {
                return Err(CommandSummaryError::MacroMatch);
            }
            ScannerStatus::Aligning { .. } => {
                return Err(CommandSummaryError::AlignmentScan);
            }
            ScannerStatus::Absorbing { .. } => {
                return Err(CommandSummaryError::AbsorbingScan);
            }
        }
        if self.scanner.warning().is_some() {
            return Err(CommandSummaryError::ScannerWarningContext);
        }
        if !self.semantic_diagnostics.is_empty() {
            return Err(CommandSummaryError::PendingSemanticDiagnostic);
        }
        if self.transient.active_expansion_depth != 0 {
            return Err(CommandSummaryError::ExpansionActive);
        }
        if self.alignment.active_alignment.is_some() || self.alignment.active_cell.is_some() {
            return Err(CommandSummaryError::AlignmentTemplateActive);
        }
        if !self.alignment.suspended.is_empty() || !self.alignment.align_stack.is_empty() {
            return Err(CommandSummaryError::SuspendedAlignment);
        }
        if !self.transient.builders.is_empty() {
            return Err(CommandSummaryError::LiveTokenBuilder);
        }
        if !self.transient.rollback_roots.is_empty() {
            return Err(CommandSummaryError::LiveRollbackRoot);
        }

        Ok(CommandSummary {
            input: self.input.clone(),
            parameters: self.parameters.clone(),
            conditions: self.conditions.clone(),
            align_state: self.alignment.align_state,
            expansion: self.expansion.clone(),
            next_builder_identity: self.transient.next_builder_identity,
        })
    }

    /// Replaces the command machine with a validated named-boundary summary.
    ///
    /// Omitted transient domains are reconstructed in their unique quiescent
    /// forms. All source/token backing is already owned by the summary, so
    /// restoration cannot perform host acquisition.
    pub fn restore_summary(
        &mut self,
        summary: CommandSummary,
    ) -> Result<(), CommandProfileMismatch> {
        self.profile().validate_fingerprint(
            CommandProfileBoundary::Summary,
            summary.profile_fingerprint(),
        )?;
        *self = Self {
            input: summary.input,
            parameters: summary.parameters,
            scanner: ScannerState::default(),
            conditions: summary.conditions,
            alignment: AlignmentDeliveryState {
                align_state: summary.align_state,
                ..AlignmentDeliveryState::default()
            },
            expansion: summary.expansion,
            replay_completions: Vec::new(),
            semantic_diagnostics: Vec::new(),
            named_token_list_pushes: Vec::new(),
            transient: TransientState {
                next_builder_identity: summary.next_builder_identity,
                ..TransientState::default()
            },
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests;
