//! The one live definition build and its direct scan-episode writer.

use core::marker::PhantomData;
use core::num::NonZeroU32;

use crate::macro_definition::MacroParameterPatternBuilder;
use crate::token::TokenWord;

use super::body::DefinitionEntry;
use super::{
    DefinitionAllocationError, DefinitionArena, DefinitionBuildError, DefinitionBuildPhase,
    PublishedDefinition,
};

pub struct DefinitionBuildKey<G> {
    serial: NonZeroU32,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for DefinitionBuildKey<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for DefinitionBuildKey<G> {}

impl<G> core::fmt::Debug for DefinitionBuildKey<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionBuildKey(..)")
    }
}

impl<G> PartialEq for DefinitionBuildKey<G> {
    fn eq(&self, other: &Self) -> bool {
        self.serial == other.serial
    }
}

impl<G> Eq for DefinitionBuildKey<G> {}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ActiveDefinitionBuild {
    serial: NonZeroU32,
    parameter_len: u32,
    pattern: MacroParameterPatternBuilder,
    origin: crate::token::OriginId,
    phase: DefinitionBuildPhase,
}

impl ActiveDefinitionBuild {
    fn push_parameter(
        &mut self,
        words: &mut Vec<TokenWord>,
        word: TokenWord,
    ) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        let mut pattern = self.pattern;
        pattern.push_parameter(word)?;
        let parameter_len = self
            .parameter_len
            .checked_add(1)
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        push_word(words, word)?;
        self.parameter_len = parameter_len;
        self.pattern = pattern;
        Ok(())
    }

    fn finish_parameters(&mut self) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        self.phase = DefinitionBuildPhase::OpenReplacement;
        Ok(())
    }

    #[inline(always)]
    fn push_replacement(
        &mut self,
        words: &mut Vec<TokenWord>,
        word: TokenWord,
    ) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        self.pattern.validate_replacement(word)?;
        push_word(words, word)
    }
}

#[inline(always)]
fn push_word(words: &mut Vec<TokenWord>, word: TokenWord) -> Result<(), DefinitionBuildError> {
    if words.len() >= u32::MAX as usize {
        return Err(DefinitionBuildError::CapacityOverflow);
    }
    words.push(word);
    Ok(())
}

/// Direct writer for one synchronous definition-scan episode.
///
/// The semantic build and its word lane move here for the episode, so the
/// writer is the one authoritative build, not a second representation. It is
/// returned to the arena before a scanner can suspend or report an error.
#[doc(hidden)]
#[derive(Debug, Eq, PartialEq)]
pub struct DefinitionBuildWriter<G> {
    build: ActiveDefinitionBuild,
    words: Vec<TokenWord>,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> DefinitionBuildWriter<G> {
    #[must_use]
    pub const fn phase(&self) -> DefinitionBuildPhase {
        self.build.phase
    }

    #[inline(always)]
    pub fn push_parameter(&mut self, word: TokenWord) -> Result<(), DefinitionBuildError> {
        self.build.push_parameter(&mut self.words, word)
    }

    #[inline(always)]
    pub fn finish_parameters(&mut self) -> Result<(), DefinitionBuildError> {
        self.build.finish_parameters()
    }

    #[inline(always)]
    pub fn push_replacement(&mut self, word: TokenWord) -> Result<(), DefinitionBuildError> {
        self.build.push_replacement(&mut self.words, word)
    }

    /// Borrows the incomplete replacement for a scanner runaway diagnostic.
    pub fn replacement_words(&self) -> &[TokenWord] {
        &self.words[self.build.parameter_len as usize..]
    }

    #[inline(always)]
    pub fn set_origin(&mut self, origin: crate::token::OriginId) {
        self.build.origin = origin;
    }
}

impl<G> DefinitionArena<G> {
    pub(crate) fn begin_build(
        &mut self,
        origin: crate::token::OriginId,
    ) -> Result<DefinitionBuildKey<G>, DefinitionAllocationError> {
        if self.active_build.is_some() || self.active_writer.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        let serial = NonZeroU32::new(self.next_build_serial)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.next_build_serial = self
            .next_build_serial
            .checked_add(1)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        debug_assert!(self.build_words.is_empty());
        self.build_words.clear();
        self.active_build = Some(ActiveDefinitionBuild {
            serial,
            parameter_len: 0,
            pattern: MacroParameterPatternBuilder::new(),
            origin,
            phase: DefinitionBuildPhase::OpenParameters,
        });
        Ok(DefinitionBuildKey {
            serial,
            _brand: PhantomData,
        })
    }

    /// Admits one synchronous definition scan. Validation is paid once per
    /// scan episode; the returned writer owns the build until released.
    pub(crate) fn admit_build_writer(
        &mut self,
        key: DefinitionBuildKey<G>,
        phase: DefinitionBuildPhase,
    ) -> Result<DefinitionBuildWriter<G>, DefinitionBuildError> {
        if self.active_writer.is_some() {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        let valid = self
            .active_build
            .as_ref()
            .is_some_and(|build| build.serial == key.serial && build.phase == phase);
        if !valid {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        let build = self.active_build.take().expect("validated active build");
        self.active_writer = Some(build.serial);
        Ok(DefinitionBuildWriter {
            build,
            words: core::mem::take(&mut self.build_words),
            _brand: PhantomData,
        })
    }

    /// Returns an admitted writer's build and word lane to the arena.
    pub(crate) fn release_build_writer(
        &mut self,
        writer: DefinitionBuildWriter<G>,
    ) -> Result<DefinitionBuildKey<G>, DefinitionBuildError> {
        let DefinitionBuildWriter { build, words, .. } = writer;
        let matches = self.active_writer == Some(build.serial) && self.active_build.is_none();
        self.active_writer = None;
        let serial = build.serial;
        self.active_build = Some(build);
        self.build_words = words;
        if !matches {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        Ok(DefinitionBuildKey {
            serial,
            _brand: PhantomData,
        })
    }

    fn active_build_mut(
        &mut self,
        key: DefinitionBuildKey<G>,
    ) -> Result<(&mut ActiveDefinitionBuild, &mut Vec<TokenWord>), DefinitionBuildError> {
        let build = self
            .active_build
            .as_mut()
            .filter(|build| build.serial == key.serial)
            .ok_or(DefinitionBuildError::InvalidPhase)?;
        Ok((build, &mut self.build_words))
    }

    pub(crate) fn push_parameter(
        &mut self,
        key: DefinitionBuildKey<G>,
        word: TokenWord,
    ) -> Result<(), DefinitionBuildError> {
        let (build, words) = self.active_build_mut(key)?;
        build.push_parameter(words, word)
    }

    pub(crate) fn finish_parameters(
        &mut self,
        key: DefinitionBuildKey<G>,
    ) -> Result<(), DefinitionBuildError> {
        self.active_build_mut(key)?.0.finish_parameters()
    }

    pub(crate) fn set_build_origin(
        &mut self,
        key: DefinitionBuildKey<G>,
        origin: crate::token::OriginId,
    ) -> Result<(), DefinitionBuildError> {
        self.active_build_mut(key)?.0.origin = origin;
        Ok(())
    }

    pub(crate) fn push_replacement(
        &mut self,
        key: DefinitionBuildKey<G>,
        word: TokenWord,
    ) -> Result<(), DefinitionBuildError> {
        let (build, words) = self.active_build_mut(key)?;
        build.push_replacement(words, word)
    }

    /// Publishes the active build as one exactly sized immutable row.
    pub(crate) fn seal_build(
        &mut self,
        key: DefinitionBuildKey<G>,
    ) -> Result<PublishedDefinition<G>, DefinitionBuildError> {
        let (build, _) = self.active_build_mut(key)?;
        if build.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        self.reserve_batch(1).map_err(|error| match error {
            DefinitionAllocationError::AllocationFailed => DefinitionBuildError::AllocationFailed,
            _ => DefinitionBuildError::CapacityOverflow,
        })?;
        let build = self.active_build.take().expect("validated active build");
        let entry = DefinitionEntry::new(
            self.build_words.as_slice().into(),
            build.parameter_len,
            build.origin,
            build.pattern,
            self.accounting.clone(),
        );
        self.build_words.clear();
        Ok(self.push_dynamic(entry))
    }

    pub(crate) fn abort_build(&mut self, key: DefinitionBuildKey<G>) {
        if self
            .active_build
            .as_ref()
            .is_some_and(|build| build.serial == key.serial)
        {
            self.abort_active_build();
        }
    }

    pub(super) fn abort_active_build(&mut self) {
        assert!(
            self.active_writer.is_none(),
            "definition writer is still admitted"
        );
        if self.active_build.take().is_some() {
            self.build_words.clear();
        }
    }
}
