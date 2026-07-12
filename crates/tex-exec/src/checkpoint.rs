use std::fmt;

use tex_lex::{InputSource, InputStack};
use tex_state::{CheckpointResumeKind, InputRecordId, InputSummary, Snapshot, SourceId, Universe};

use crate::{ExecError, ModeNest, ModeNestSummary};

const MODE_HASH_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const MODE_HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Atomic engine-level checkpoint over state, live input, and executor modes.
#[derive(Clone, Debug)]
pub struct EngineCheckpoint {
    universe: Snapshot,
    input: InputSummary,
    modes: ModeNestSummary,
    state_hash: u64,
}

impl EngineCheckpoint {
    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.state_hash
    }

    #[must_use]
    pub const fn resume_kind(&self) -> CheckpointResumeKind {
        self.universe.resume_kind()
    }

    #[must_use]
    pub const fn is_resume_valid(&self) -> bool {
        self.universe.is_resume_valid()
    }

    #[must_use]
    pub const fn input_summary(&self) -> &InputSummary {
        &self.input
    }

    #[must_use]
    pub const fn mode_summary(&self) -> &ModeNestSummary {
        &self.modes
    }
}

/// Failure to restore an engine checkpoint.
#[derive(Debug)]
pub enum EngineRestoreError<E> {
    HashOnly,
    Input(E),
    Mode(ExecError),
}

impl<E: fmt::Display> fmt::Display for EngineRestoreError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HashOnly => f.write_str("hash-only checkpoint is not a restart point"),
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
        EngineCheckpoint {
            universe,
            input: input_summary,
            modes,
            state_hash,
        }
    }

    /// Restores all engine-owned components only after input and mode recovery
    /// have both succeeded.
    pub fn restore_checkpoint<S, E, F>(
        &mut self,
        input: &mut InputStack<S>,
        universe: &mut Universe,
        checkpoint: &EngineCheckpoint,
        reopen_source: F,
    ) -> Result<(), EngineRestoreError<E>>
    where
        S: InputSource,
        F: FnMut(SourceId, Option<InputRecordId>, &tex_state::SourceFrameSummary) -> Result<S, E>,
    {
        if !checkpoint.is_resume_valid() {
            return Err(EngineRestoreError::HashOnly);
        }
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
