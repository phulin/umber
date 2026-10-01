//! Detached cold-path definition staging and the shared build phase contract.

use crate::macro_definition::{MacroParameterPatternBuilder, MacroParameterProgramError};
use crate::token::TokenWord;

use super::DefinitionAllocationError;

/// Monotonic mutable phase of an unpublished definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionBuildPhase {
    OpenParameters,
    OpenReplacement,
    Sealed,
    Published,
}

/// Failure while constructing mutable definition attempt data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionBuildError {
    AllocationFailed,
    CapacityOverflow,
    InvalidPhase,
    InvalidProgram(MacroParameterProgramError),
}

impl From<MacroParameterProgramError> for DefinitionBuildError {
    fn from(error: MacroParameterProgramError) -> Self {
        match error {
            MacroParameterProgramError::CapacityOverflow => Self::CapacityOverflow,
            error => Self::InvalidProgram(error),
        }
    }
}

/// Detached cold-path staging for imported and replayed definitions.
///
/// Ordinary `def`-family scanning writes into the arena's build lane
/// directly. This builder serves batches whose source is not the live
/// scanner, such as memo replay; publication copies its already-detached
/// words into one immutable row.
#[derive(Debug)]
pub struct DefinitionBuilder {
    pub(super) words: Vec<TokenWord>,
    pub(super) parameter_len: u32,
    pub(super) pattern: MacroParameterPatternBuilder,
    pub(super) phase: DefinitionBuildPhase,
    #[cfg(test)]
    fail_next_reserve: bool,
}

impl Default for DefinitionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DefinitionBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            words: Vec::new(),
            parameter_len: 0,
            pattern: MacroParameterPatternBuilder::new(),
            phase: DefinitionBuildPhase::OpenParameters,
            #[cfg(test)]
            fail_next_reserve: false,
        }
    }

    /// Clears one recycled cold-path staging row.
    pub fn reset(&mut self) {
        self.words.clear();
        self.parameter_len = 0;
        self.pattern = MacroParameterPatternBuilder::new();
        self.phase = DefinitionBuildPhase::OpenParameters;
        #[cfg(test)]
        {
            self.fail_next_reserve = false;
        }
    }

    pub fn push_parameter(&mut self, word: TokenWord) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        let mut pattern = self.pattern;
        pattern.push_parameter(word)?;
        let parameter_len = self
            .parameter_len
            .checked_add(1)
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        self.reserve_word()?;
        self.parameter_len = parameter_len;
        self.pattern = pattern;
        self.words.push(word);
        Ok(())
    }

    pub fn finish_parameters(&mut self) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        self.phase = DefinitionBuildPhase::OpenReplacement;
        Ok(())
    }

    pub fn push_replacement(&mut self, word: TokenWord) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        self.pattern.validate_replacement(word)?;
        u32::try_from(self.words.len() + 1).map_err(|_| DefinitionBuildError::CapacityOverflow)?;
        self.reserve_word()?;
        self.words.push(word);
        Ok(())
    }

    pub fn seal(&mut self) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        self.phase = DefinitionBuildPhase::Sealed;
        Ok(())
    }

    #[must_use]
    pub fn phase(&self) -> DefinitionBuildPhase {
        self.phase
    }

    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        &self.words[..self.parameter_len as usize]
    }

    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        &self.words[self.parameter_len as usize..]
    }

    #[must_use]
    pub fn words(&self) -> &[TokenWord] {
        &self.words
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.words.capacity()
    }

    fn reserve_word(&mut self) -> Result<(), DefinitionBuildError> {
        #[cfg(test)]
        if core::mem::take(&mut self.fail_next_reserve) {
            return Err(DefinitionBuildError::AllocationFailed);
        }
        self.words
            .try_reserve(1)
            .map_err(|_| DefinitionBuildError::AllocationFailed)
    }

    #[cfg(test)]
    pub(super) fn force_next_reserve_failure(&mut self) {
        self.fail_next_reserve = true;
    }

    pub(super) fn validate_completed(&self) -> Result<(), DefinitionAllocationError> {
        if self.phase != DefinitionBuildPhase::Sealed {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        u32::try_from(self.words.len()).map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        Ok(())
    }
}
