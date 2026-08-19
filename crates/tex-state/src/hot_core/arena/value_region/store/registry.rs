//! Persistent live token, macro, and glue registry over one arena candidate.

use core::num::NonZeroU32;

use crate::glue::GlueSpec;
use crate::identity::{IdentityAllocator, IdentityError, IdentityMark};
use crate::ids::{GlueId, MacroDefinitionId, TokenListId};
use crate::macro_store::MacroParameterPattern;
use crate::meaning::MeaningFlags;
use crate::provenance::OriginRef;
use crate::token::Token;
use crate::token_store::TokenSemanticId;

use super::{
    RegionArenaError, RuntimeGlueCoordinate, RuntimeGlueView, RuntimeMacroCoordinate,
    RuntimeMacroInput, RuntimeMacroView, RuntimeOriginRoot, RuntimeTokenListCoordinate,
    RuntimeTokenListInput, RuntimeTokenListView, RuntimeValueCandidate,
    RuntimeValueRegionAccounting, RuntimeValueRegionMark, RuntimeValueStore,
};

/// A rejected live-registry operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeValueRegistryError {
    Region(RegionArenaError),
    Identity(IdentityError),
    UnknownTokenList,
    UnknownMacroDefinition,
    UnknownGlue,
    LocationCapacityExhausted,
    InvalidMark,
}

impl From<RegionArenaError> for RuntimeValueRegistryError {
    fn from(error: RegionArenaError) -> Self {
        Self::Region(error)
    }
}

impl From<IdentityError> for RuntimeValueRegistryError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}

/// Fixed-size rollback mark for all live runtime-value families.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeValueRegistryMark {
    arena: RuntimeValueRegionMark,
    token_identities: IdentityMark,
    macro_identities: IdentityMark,
    glue_identities: IdentityMark,
    token_locations: u32,
    macro_locations: u32,
    glue_locations: u32,
}

/// Token-list payload allocated under a fresh registry identity.
pub(crate) struct RuntimeTokenValueInput<'a> {
    pub(crate) semantic_id: TokenSemanticId,
    pub(crate) tokens: &'a [Token],
    pub(crate) provenance_roots: &'a [RuntimeOriginRoot],
}

/// Macro payload allocated under a fresh registry identity.
pub(crate) struct RuntimeMacroValueInput<'a> {
    pub(crate) flags: MeaningFlags,
    pub(crate) parameter_pattern: MacroParameterPattern,
    pub(crate) parameter_text: TokenListId,
    pub(crate) replacement_text: TokenListId,
    pub(crate) definition_origin: &'a OriginRef,
    pub(crate) parameter_origins: &'a [RuntimeOriginRoot],
    pub(crate) replacement_origins: &'a [RuntimeOriginRoot],
    pub(crate) observation_operand: i64,
    pub(crate) allocation_serial: u64,
}

/// Logical and retained storage evidence for focused registry controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeValueRegistryAccounting {
    pub(crate) regions: RuntimeValueRegionAccounting,
    pub(crate) token_locations: usize,
    pub(crate) macro_locations: usize,
    pub(crate) glue_locations: usize,
    pub(crate) retained_location_slots: usize,
    pub(crate) identity_slots: usize,
    pub(crate) retained_identity_slots: usize,
}

/// One persistent mutable candidate plus dense, generation-safe family maps.
pub(crate) struct RuntimeValueRegistry {
    candidate: RuntimeValueCandidate,
    token_identities: IdentityAllocator,
    macro_identities: IdentityAllocator,
    glue_identities: IdentityAllocator,
    token_locations: Vec<RuntimeTokenListCoordinate>,
    macro_locations: Vec<RuntimeMacroCoordinate>,
    glue_locations: Vec<RuntimeGlueCoordinate>,
}

impl RuntimeValueRegistry {
    pub(crate) fn new(
        initial_region_capacity: NonZeroU32,
        empty_token_semantic_id: TokenSemanticId,
    ) -> Result<Self, RuntimeValueRegistryError> {
        let mut candidate = RuntimeValueStore::new(initial_region_capacity).candidate()?;
        let empty = candidate.append_token_list(RuntimeTokenListInput {
            id: TokenListId::EMPTY,
            semantic_id: empty_token_semantic_id,
            tokens: &[],
            provenance_roots: &[],
        })?;
        let zero = candidate.append_glue(GlueId::ZERO, GlueSpec::ZERO)?;
        Ok(Self {
            candidate,
            token_identities: IdentityAllocator::new(1),
            macro_identities: IdentityAllocator::new(0),
            glue_identities: IdentityAllocator::new(1),
            token_locations: vec![empty],
            macro_locations: Vec::new(),
            glue_locations: vec![zero],
        })
    }

    pub(crate) fn mark(&self) -> Result<RuntimeValueRegistryMark, RuntimeValueRegistryError> {
        Ok(RuntimeValueRegistryMark {
            arena: self.candidate.mark()?,
            token_identities: self.token_identities.watermark(),
            macro_identities: self.macro_identities.watermark(),
            glue_identities: self.glue_identities.watermark(),
            token_locations: checked_location_len(self.token_locations.len())?,
            macro_locations: checked_location_len(self.macro_locations.len())?,
            glue_locations: checked_location_len(self.glue_locations.len())?,
        })
    }

    pub(crate) fn validate_rollback(
        &self,
        mark: RuntimeValueRegistryMark,
    ) -> Result<(), RuntimeValueRegistryError> {
        self.candidate.validate_truncate(mark.arena)?;
        self.token_identities
            .validate_rollback(mark.token_identities)?;
        self.macro_identities
            .validate_rollback(mark.macro_identities)?;
        self.glue_identities
            .validate_rollback(mark.glue_identities)?;
        validate_location_mark(self.token_locations.len(), mark.token_locations)?;
        validate_location_mark(self.macro_locations.len(), mark.macro_locations)?;
        validate_location_mark(self.glue_locations.len(), mark.glue_locations)?;
        Ok(())
    }

    /// Restores locations and identities before reclaiming arena suffixes.
    /// Published destination roots must already be restored by the caller.
    pub(crate) fn rollback(
        &mut self,
        mark: RuntimeValueRegistryMark,
    ) -> Result<(), RuntimeValueRegistryError> {
        self.validate_rollback(mark)?;
        self.token_locations.truncate(mark.token_locations as usize);
        self.macro_locations.truncate(mark.macro_locations as usize);
        self.glue_locations.truncate(mark.glue_locations as usize);
        self.token_identities.rollback(mark.token_identities)?;
        self.macro_identities.rollback(mark.macro_identities)?;
        self.glue_identities.rollback(mark.glue_identities)?;
        self.candidate.truncate(mark.arena)?;
        Ok(())
    }

    pub(crate) fn allocate_token_list(
        &mut self,
        input: RuntimeTokenValueInput<'_>,
    ) -> Result<TokenListId, RuntimeValueRegistryError> {
        reserve_location(&mut self.token_locations)?;
        let identity_mark = self.token_identities.watermark();
        let identity = self.token_identities.allocate()?;
        let id = TokenListId::from_identity(identity);
        let coordinate = match self.candidate.append_token_list(RuntimeTokenListInput {
            id,
            semantic_id: input.semantic_id,
            tokens: input.tokens,
            provenance_roots: input.provenance_roots,
        }) {
            Ok(coordinate) => coordinate,
            Err(error) => {
                self.token_identities.rollback(identity_mark)?;
                return Err(error.into());
            }
        };
        debug_assert_eq!(id.raw() as usize, self.token_locations.len());
        self.token_locations.push(coordinate);
        Ok(id)
    }

    pub(crate) fn allocate_macro(
        &mut self,
        input: RuntimeMacroValueInput<'_>,
    ) -> Result<MacroDefinitionId, RuntimeValueRegistryError> {
        let parameter_text = self.token_coordinate(input.parameter_text)?;
        let replacement_text = self.token_coordinate(input.replacement_text)?;
        reserve_location(&mut self.macro_locations)?;
        let identity_mark = self.macro_identities.watermark();
        let identity = self.macro_identities.allocate()?;
        let definition = MacroDefinitionId::from_identity(identity);
        let coordinate = match self.candidate.append_macro(RuntimeMacroInput {
            definition,
            flags: input.flags,
            parameter_pattern: input.parameter_pattern,
            parameter_text,
            replacement_text,
            definition_origin: input.definition_origin,
            parameter_origins: input.parameter_origins,
            replacement_origins: input.replacement_origins,
            observation_operand: input.observation_operand,
            allocation_serial: input.allocation_serial,
        }) {
            Ok(coordinate) => coordinate,
            Err(error) => {
                self.macro_identities.rollback(identity_mark)?;
                return Err(error.into());
            }
        };
        debug_assert_eq!(definition.raw() as usize, self.macro_locations.len());
        self.macro_locations.push(coordinate);
        Ok(definition)
    }

    pub(crate) fn allocate_glue(
        &mut self,
        spec: GlueSpec,
    ) -> Result<GlueId, RuntimeValueRegistryError> {
        reserve_location(&mut self.glue_locations)?;
        let identity_mark = self.glue_identities.watermark();
        let identity = self.glue_identities.allocate()?;
        let id = GlueId::from_identity(identity);
        let coordinate = match self.candidate.append_glue(id, spec) {
            Ok(coordinate) => coordinate,
            Err(error) => {
                self.glue_identities.rollback(identity_mark)?;
                return Err(error.into());
            }
        };
        debug_assert_eq!(id.raw() as usize, self.glue_locations.len());
        self.glue_locations.push(coordinate);
        Ok(id)
    }

    pub(crate) fn token_list(
        &self,
        id: TokenListId,
    ) -> Result<RuntimeTokenListView<'_>, RuntimeValueRegistryError> {
        Ok(self
            .candidate
            .admit_token_list(self.token_coordinate(id)?)?)
    }

    pub(crate) fn macro_definition(
        &self,
        id: MacroDefinitionId,
    ) -> Result<RuntimeMacroView<'_>, RuntimeValueRegistryError> {
        Ok(self.candidate.admit_macro(self.macro_coordinate(id)?)?)
    }

    pub(crate) fn glue(
        &self,
        id: GlueId,
    ) -> Result<RuntimeGlueView<'_>, RuntimeValueRegistryError> {
        Ok(self.candidate.admit_glue(self.glue_coordinate(id)?)?)
    }

    /// Adds only region owners absent from the current canonical root set.
    pub(crate) fn publish_into(
        &mut self,
        destination: &mut RuntimeValueStore,
    ) -> Result<(), RuntimeValueRegistryError> {
        self.candidate.publish_into(destination)?;
        Ok(())
    }

    /// Cold generation fork: seals once, shares region owners, and forks ids.
    pub(crate) fn fork(&mut self) -> Result<Self, RuntimeValueRegistryError> {
        let published = self.candidate.published_store()?;
        Ok(Self {
            candidate: RuntimeValueCandidate::from_store(published)?,
            token_identities: self.token_identities.fork(),
            macro_identities: self.macro_identities.fork(),
            glue_identities: self.glue_identities.fork(),
            token_locations: self.token_locations.clone(),
            macro_locations: self.macro_locations.clone(),
            glue_locations: self.glue_locations.clone(),
        })
    }

    pub(crate) fn empty_published_store(&self) -> RuntimeValueStore {
        RuntimeValueStore {
            regions: self.candidate.arena.base.retain_regions(&[]),
        }
    }

    pub(crate) fn accounting(&self) -> RuntimeValueRegistryAccounting {
        let token_shape = self.token_identities.storage_shape();
        let macro_shape = self.macro_identities.storage_shape();
        let glue_shape = self.glue_identities.storage_shape();
        RuntimeValueRegistryAccounting {
            regions: self.candidate.accounting(),
            token_locations: self.token_locations.len(),
            macro_locations: self.macro_locations.len(),
            glue_locations: self.glue_locations.len(),
            retained_location_slots: self
                .token_locations
                .capacity()
                .saturating_add(self.macro_locations.capacity())
                .saturating_add(self.glue_locations.capacity()),
            identity_slots: token_shape
                .0
                .saturating_add(macro_shape.0)
                .saturating_add(glue_shape.0),
            retained_identity_slots: token_shape
                .1
                .saturating_add(macro_shape.1)
                .saturating_add(glue_shape.1),
        }
    }

    fn token_coordinate(
        &self,
        id: TokenListId,
    ) -> Result<RuntimeTokenListCoordinate, RuntimeValueRegistryError> {
        if !self.token_identities.contains(id.identity()) {
            return Err(RuntimeValueRegistryError::UnknownTokenList);
        }
        self.token_locations
            .get(id.raw() as usize)
            .copied()
            .filter(|coordinate| coordinate.id() == id)
            .ok_or(RuntimeValueRegistryError::UnknownTokenList)
    }

    fn macro_coordinate(
        &self,
        id: MacroDefinitionId,
    ) -> Result<RuntimeMacroCoordinate, RuntimeValueRegistryError> {
        if !self.macro_identities.contains(id.identity()) {
            return Err(RuntimeValueRegistryError::UnknownMacroDefinition);
        }
        self.macro_locations
            .get(id.raw() as usize)
            .copied()
            .filter(|coordinate| coordinate.id() == id)
            .ok_or(RuntimeValueRegistryError::UnknownMacroDefinition)
    }

    fn glue_coordinate(
        &self,
        id: GlueId,
    ) -> Result<RuntimeGlueCoordinate, RuntimeValueRegistryError> {
        if !self.glue_identities.contains(id.identity()) {
            return Err(RuntimeValueRegistryError::UnknownGlue);
        }
        self.glue_locations
            .get(id.raw() as usize)
            .copied()
            .filter(|coordinate| coordinate.id() == id)
            .ok_or(RuntimeValueRegistryError::UnknownGlue)
    }
}

fn reserve_location<T>(locations: &mut Vec<T>) -> Result<(), RuntimeValueRegistryError> {
    locations
        .try_reserve(1)
        .map_err(|_| RuntimeValueRegistryError::LocationCapacityExhausted)
}

fn checked_location_len(len: usize) -> Result<u32, RuntimeValueRegistryError> {
    u32::try_from(len).map_err(|_| RuntimeValueRegistryError::LocationCapacityExhausted)
}

fn validate_location_mark(current: usize, mark: u32) -> Result<(), RuntimeValueRegistryError> {
    (mark as usize <= current)
        .then_some(())
        .ok_or(RuntimeValueRegistryError::InvalidMark)
}

#[cfg(test)]
mod tests;
