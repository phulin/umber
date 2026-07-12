use std::fmt;

use tex_lex::{InputSource, InputStack};
use tex_state::{CheckpointResumeKind, InputRecordId, InputSummary, Snapshot, SourceId, Universe};

use crate::{ExecError, ModeNest, ModeNestSummary};

const MODE_HASH_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const MODE_HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Clone, Debug)]
struct CheckpointData {
    universe: Snapshot,
    input: InputSummary,
    modes: ModeNestSummary,
    state_hash: u64,
}

/// Result of an aggregate engine capture.
///
/// Only the resume-valid variant can be supplied to restoration, making a
/// hash-only observation structurally incapable of becoming a restart point.
#[derive(Clone, Debug)]
pub enum EngineCheckpoint {
    ResumeValid(ResumeValidCheckpoint),
    HashOnly(HashOnlyObservation),
}

#[derive(Clone, Debug)]
pub struct ResumeValidCheckpoint(CheckpointData);

#[derive(Clone, Debug)]
pub struct HashOnlyObservation(CheckpointData);

impl EngineCheckpoint {
    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.data().state_hash
    }

    #[must_use]
    pub const fn resume_kind(&self) -> CheckpointResumeKind {
        match self {
            Self::ResumeValid(_) => CheckpointResumeKind::ResumeValid,
            Self::HashOnly(_) => CheckpointResumeKind::HashOnly,
        }
    }

    #[must_use]
    pub const fn is_resume_valid(&self) -> bool {
        matches!(self, Self::ResumeValid(_))
    }

    #[must_use]
    pub const fn input_summary(&self) -> &InputSummary {
        &self.data().input
    }

    #[must_use]
    pub const fn mode_summary(&self) -> &ModeNestSummary {
        &self.data().modes
    }

    #[must_use]
    pub const fn as_resume_valid(&self) -> Option<&ResumeValidCheckpoint> {
        match self {
            Self::ResumeValid(checkpoint) => Some(checkpoint),
            Self::HashOnly(_) => None,
        }
    }

    const fn data(&self) -> &CheckpointData {
        match self {
            Self::ResumeValid(checkpoint) => &checkpoint.0,
            Self::HashOnly(observation) => &observation.0,
        }
    }
}

impl ResumeValidCheckpoint {
    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.0.state_hash
    }
}

impl HashOnlyObservation {
    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.0.state_hash
    }
}

/// Failure to restore an engine checkpoint.
#[derive(Debug)]
pub enum EngineRestoreError<E> {
    Input(E),
    Mode(ExecError),
}

impl<E: fmt::Display> fmt::Display for EngineRestoreError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => write!(f, "could not reopen checkpoint input: {error}"),
            Self::Mode(error) => write!(f, "could not restore checkpoint mode nest: {error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for EngineRestoreError<E> {}

impl crate::Executor {
    /// Synchronizes pipeline-owned roots and captures one aggregate checkpoint.
    pub fn checkpoint<S: InputSource>(
        &self,
        input: &mut InputStack<S>,
        universe: &mut Universe,
    ) -> EngineCheckpoint {
        let input_summary = input.publication_summary(universe);
        universe.set_input_summary(input_summary.clone());
        let modes = self.nest().summary();
        let universe = universe.snapshot();
        let state_hash = combine_mode_hash(universe.state_hash(), &modes);
        let data = CheckpointData {
            universe,
            input: input_summary,
            modes,
            state_hash,
        };
        match data.universe.resume_kind() {
            CheckpointResumeKind::ResumeValid => {
                EngineCheckpoint::ResumeValid(ResumeValidCheckpoint(data))
            }
            CheckpointResumeKind::HashOnly => EngineCheckpoint::HashOnly(HashOnlyObservation(data)),
        }
    }

    /// Restores all engine-owned components only after input and mode recovery
    /// have both succeeded.
    pub fn restore_checkpoint<S, E, F>(
        &mut self,
        input: &mut InputStack<S>,
        universe: &mut Universe,
        checkpoint: &ResumeValidCheckpoint,
        reopen_source: F,
    ) -> Result<(), EngineRestoreError<E>>
    where
        S: InputSource,
        F: FnMut(SourceId, Option<InputRecordId>, &tex_state::SourceFrameSummary) -> Result<S, E>,
    {
        let checkpoint = &checkpoint.0;
        let restored_input = InputStack::from_summary(&checkpoint.input, reopen_source)
            .map_err(EngineRestoreError::Input)?;
        let restored_modes =
            ModeNest::from_summary(checkpoint.modes.clone()).map_err(EngineRestoreError::Mode)?;

        universe.rollback(&checkpoint.universe);
        *input = restored_input;
        self.nest = restored_modes;
        Ok(())
    }
}

fn combine_mode_hash(universe_hash: u64, modes: &ModeNestSummary) -> u64 {
    let mut hash = MODE_HASH_OFFSET;
    for byte in format!("{modes:?}").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(MODE_HASH_PRIME);
    }
    universe_hash.rotate_left(17) ^ hash
}
