//! Command snapshot and durable-summary ownership.

use std::fmt;
use std::sync::Arc;

use crate::CommandState;
use crate::conditionals::ConditionStack;
use crate::input::InputLevel;
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
    pub(crate) input: InputState,
    pub(crate) parameters: ParameterState,
    pub(crate) conditions: ConditionStack,
    pub(crate) align_state: i32,
    pub(crate) expansion: ExpansionState,
    pub(crate) next_builder_identity: u64,
}

impl CommandStateSnapshot {
    /// Returns the immutable profile identity captured by this snapshot.
    #[must_use]
    pub fn profile_fingerprint(&self) -> CommandProfileFingerprint {
        self.state.profile().fingerprint()
    }
}

impl CommandSummary {
    /// Returns the root source coordinate capability retained by this
    /// continuation, when the root input is still live.
    #[doc(hidden)]
    #[must_use]
    pub fn root_source_id(&self) -> Option<tex_state::SourceId> {
        self.input.levels.iter().find_map(|level| {
            let InputLevel::Source(source) = level else {
                return None;
            };
            Some(source.cursor.backing.id)
        })
    }

    /// Compares future command semantics while normalizing allocation-local
    /// provenance handles on backed-up tokens through their captured source
    /// provenance.
    #[must_use]
    pub fn exact_future_state_matches(
        &self,
        other: &Self,
        self_root_anchor: usize,
        other_root_anchor: usize,
    ) -> bool {
        let mut normalized = self.clone();
        let Some((old_backing, expected_backing)) =
            normalized
                .input
                .levels
                .iter()
                .find_map(|level| match level {
                    InputLevel::Source(source) => Some((
                        source.cursor.backing.bytes.to_vec(),
                        other.input.levels.iter().find_map(|level| match level {
                            InputLevel::Source(source) => Some(source.cursor.backing.bytes.clone()),
                            InputLevel::Tokens(_) => None,
                        })?,
                    )),
                    InputLevel::Tokens(_) => None,
                })
        else {
            return normalized == *other;
        };
        if !normalized.rebind_root_source_at(
            &old_backing,
            expected_backing,
            self_root_anchor,
            other_root_anchor,
        ) {
            return false;
        }
        if normalized.input.levels.len() != other.input.levels.len() {
            return false;
        }
        for (left, right) in normalized.input.levels.iter_mut().zip(&other.input.levels) {
            match (left, right) {
                (InputLevel::Tokens(left), InputLevel::Tokens(right)) => {
                    if left.payload.backed_up_words().is_some()
                        && right.payload.backed_up_words().is_some()
                        && left
                            .payload
                            .adopt_matching_origins(&right.payload)
                            .is_none()
                    {
                        return false;
                    }
                }
                (InputLevel::Source(_), InputLevel::Source(_)) => {}
                _ => return false,
            }
        }
        normalized == *other
    }

    /// Returns the immutable profile identity captured by this durable summary.
    #[must_use]
    pub fn profile_fingerprint(&self) -> CommandProfileFingerprint {
        self.expansion.profile.fingerprint()
    }

    /// Conservative physical cursor of the bottom registered source.
    #[must_use]
    pub fn root_source_anchor(&self) -> Option<usize> {
        self.input.levels.iter().find_map(|level| {
            let crate::input::InputLevel::Source(source) = level else {
                return None;
            };
            // A loaded physical line owns its complete normalized image,
            // including unread bytes after the command that published this
            // checkpoint. The next refill offset is therefore the earliest
            // safe edit boundary both while a line is active and between
            // lines; the token cursor would admit a stale line suffix.
            usize::try_from(source.cursor.next_physical_offset).ok()
        })
    }

    /// Rebinds the bottom generated source to edited bytes while retaining
    /// its exact command-owned identity and lexer cursor.
    pub fn rebind_root_source(&mut self, old: &[u8], new: Arc<[u8]>) -> bool {
        self.rebind_root_source_at(old, new, 0, 0)
    }

    /// Rebinds the generated root and maps every live source coordinate from
    /// one already-proven unchanged suffix anchor to the other.
    pub fn rebind_root_source_at(
        &mut self,
        old: &[u8],
        new: Arc<[u8]>,
        old_anchor: usize,
        new_anchor: usize,
    ) -> bool {
        let Ok(old_anchor) = u64::try_from(old_anchor) else {
            return false;
        };
        let Ok(new_anchor) = u64::try_from(new_anchor) else {
            return false;
        };
        let Some(byte_delta) = i64::try_from(new_anchor)
            .ok()
            .and_then(|new_anchor| i64::try_from(old_anchor).ok().map(|old| new_anchor - old))
        else {
            return false;
        };
        let old_line = old
            .get(..usize::try_from(old_anchor).unwrap_or(usize::MAX))
            .map(|prefix| prefix.iter().filter(|&&byte| byte == b'\n').count());
        let new_line = new
            .get(..usize::try_from(new_anchor).unwrap_or(usize::MAX))
            .map(|prefix| prefix.iter().filter(|&&byte| byte == b'\n').count());
        let Some(line_delta) = old_line.zip(new_line).and_then(|(old, new)| {
            i64::try_from(new)
                .ok()
                .zip(i64::try_from(old).ok())
                .map(|(new, old)| new - old)
        }) else {
            return false;
        };
        let Some(source) = self.input.levels.iter_mut().find_map(|level| match level {
            crate::input::InputLevel::Source(source) => Some(source),
            crate::input::InputLevel::Tokens(_) => None,
        }) else {
            return false;
        };
        if source.cursor.backing.bytes.as_ref() != old {
            return false;
        }
        let id = source.cursor.backing.id;
        let Ok(backing) = source.cursor.backing.rebind_generated(id, new) else {
            return false;
        };
        source.cursor.backing = backing;
        let Some(next_physical_offset) = source
            .cursor
            .next_physical_offset
            .checked_add_signed(byte_delta)
        else {
            return false;
        };
        let Some(next_line_number) = source
            .cursor
            .next_line_number
            .checked_add_signed(line_delta)
        else {
            return false;
        };
        source.cursor.next_physical_offset = next_physical_offset;
        source.cursor.next_line_number = next_line_number;
        if let Some(line) = &mut source.cursor.line
            && line
                .rehome_edited_backing(
                    id,
                    &source.cursor.backing.bytes,
                    source.cursor.backing.mode,
                    byte_delta,
                    line_delta,
                )
                .is_none()
        {
            return false;
        }
        for level in &mut self.input.levels {
            let InputLevel::Tokens(tokens) = level else {
                continue;
            };
            if tokens.payload.backed_up_words().is_some()
                && tokens
                    .payload
                    .rehome_backed_up_source(id, byte_delta)
                    .is_none()
            {
                return false;
            }
        }
        true
    }

    /// Whether this summary still owns the expected bottom source backing.
    #[must_use]
    pub fn root_source_matches(&self, expected: &[u8]) -> bool {
        self.input.levels.iter().find_map(|level| match level {
            crate::input::InputLevel::Source(source) => Some(source.cursor.backing.bytes.as_ref()),
            crate::input::InputLevel::Tokens(_) => None,
        }) == Some(expected)
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
    /// includes process-local scratch allocations.
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

    /// Restores an isolated nested-input transaction while retaining the
    /// conditional stack produced by expansion inside that transaction.
    ///
    /// TeX82 §1370's deferred `write_out` input is artificial, but expansion
    /// still uses the live global `cond_ptr`. Its `\if` pushes and `\fi` pops
    /// therefore survive after the synthetic input levels are removed. This
    /// boundary restores the surrounding cursor/scanner state without
    /// resurrecting conditional frames that the nested expansion changed.
    pub fn rollback_nested_input_preserving_conditions(
        &mut self,
        snapshot: CommandStateSnapshot,
    ) -> Result<(), CommandProfileMismatch> {
        let conditions = self.conditions.clone();
        self.rollback(snapshot)?;
        self.conditions = conditions;
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
        if self.transient.active_expansion_depth != 0
            || !self.replay_completions.is_empty()
            || !self.pending_replay_completions.is_empty()
        {
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
        let engine_semantics = self.engine_semantics();
        let usage = self.usage.clone();
        *self = Self {
            engine_semantics,
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
            pending_replay_completions: Vec::new(),
            semantic_diagnostics: Vec::new(),
            name_in_progress: false,
            named_token_list_pushes: Vec::new(),
            file_framing_events: Vec::new(),
            usage,
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
