//! Persistent live token, macro, glue, and origin-list registry over one arena candidate.

use core::fmt;
use core::mem::size_of;
use core::num::{NonZeroU32, NonZeroUsize};

use crate::glue::GlueSpec;
use crate::identity::{HandleIdentity, IdentityAllocator, IdentityError};
use crate::ids::{GlueId, MacroDefinitionId, OriginListId, TokenListId};
use crate::macro_store::MacroParameterPattern;
use crate::meaning::MeaningFlags;
use crate::token::TracedTokenWord;
use crate::token::{OriginId, Token};
use crate::token_store::TokenSemanticId;

use super::{
    RegionArenaError, RuntimeGlueCoordinate, RuntimeGlueView, RuntimeMacroCoordinate,
    RuntimeMacroInput, RuntimeMacroView, RuntimeOriginEntry, RuntimeOriginListCoordinate,
    RuntimeOriginListView, RuntimeTokenListCoordinate, RuntimeTokenListInput, RuntimeTokenListView,
    RuntimeTracedTokenListInput, RuntimeValueCandidate, RuntimeValueProvider,
    RuntimeValueRegionAccounting, RuntimeValueRegionMark, RuntimeValueRootSet,
};

/// A rejected live-registry operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeValueRegistryError {
    Region(RegionArenaError),
    Identity(IdentityError),
    UnknownTokenList,
    UnknownMacroDefinition,
    UnknownGlue,
    UnknownOriginList,
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
    token_locations: u32,
    macro_locations: u32,
    glue_locations: u32,
    origin_list_locations: u32,
    next_macro_observation_operand: i64,
    next_macro_allocation_serial: u64,
}

/// Token-list payload allocated under a fresh registry identity.
pub(crate) struct RuntimeTokenValueInput<'a> {
    pub(crate) semantic_id: TokenSemanticId,
    pub(crate) tokens: &'a [Token],
    pub(crate) provenance: &'a [RuntimeOriginEntry],
}

/// Traced scanner payload allocated without an intermediate semantic buffer.
pub(crate) struct RuntimeTracedTokenValueInput<'a> {
    pub(crate) semantic_id: TokenSemanticId,
    pub(crate) words: &'a [TracedTokenWord],
}

/// Exact provenance sequence allocated under a fresh registry identity.
pub(crate) struct RuntimeOriginListValueInput<'a> {
    pub(crate) origins: &'a [OriginId],
}

/// Macro payload allocated under a fresh registry identity.
pub(crate) struct RuntimeMacroValueInput<'a> {
    pub(crate) flags: MeaningFlags,
    pub(crate) parameter_pattern: MacroParameterPattern,
    pub(crate) parameter_text: TokenListId,
    pub(crate) replacement_text: TokenListId,
    pub(crate) definition_origin: OriginId,
    pub(crate) parameter_origins: &'a [RuntimeOriginEntry],
    pub(crate) replacement_origins: &'a [RuntimeOriginEntry],
    pub(crate) observation_width: u32,
}

/// Logical and retained storage evidence for focused registry controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeValueRegistryAccounting {
    pub(crate) regions: RuntimeValueRegionAccounting,
    pub(crate) token_locations: usize,
    pub(crate) macro_locations: usize,
    pub(crate) glue_locations: usize,
    pub(crate) origin_list_locations: usize,
    pub(crate) retained_location_slots: usize,
    pub(crate) identity_slots: usize,
    pub(crate) retained_identity_slots: usize,
}

/// Origin-list-specific accounting within the aggregate runtime registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeOriginListAccounting {
    pub(crate) lists: usize,
    pub(crate) entries: usize,
    pub(crate) retained_list_slots: usize,
    pub(crate) retained_entry_slots: usize,
    pub(crate) retained_bytes: usize,
}

/// One persistent mutable candidate plus dense, generation-safe family maps.
pub(crate) struct RuntimeValueRegistry {
    candidate: RuntimeValueCandidate,
    token_identities: IdentityAllocator,
    macro_identities: IdentityAllocator,
    glue_identities: IdentityAllocator,
    origin_list_identities: IdentityAllocator,
    token_locations: Vec<RuntimeTokenListCoordinate>,
    macro_locations: Vec<RuntimeMacroCoordinate>,
    glue_locations: Vec<RuntimeGlueCoordinate>,
    origin_list_locations: Vec<RuntimeOriginListCoordinate>,
    origin_list_limit: usize,
    origin_list_entry_limit: usize,
    next_macro_observation_operand: i64,
    next_macro_allocation_serial: u64,
}

impl fmt::Debug for RuntimeValueRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeValueRegistry")
            .field("accounting", &self.accounting())
            .finish()
    }
}

impl RuntimeValueRegistry {
    pub(crate) fn new(
        initial_region_capacity: NonZeroU32,
        empty_token_semantic_id: TokenSemanticId,
    ) -> Result<Self, RuntimeValueRegistryError> {
        let mut candidate = RuntimeValueRootSet::new(initial_region_capacity).candidate()?;
        let empty = candidate.append_token_list(RuntimeTokenListInput {
            id: TokenListId::EMPTY,
            semantic_id: empty_token_semantic_id,
            tokens: &[],
            provenance: &[],
        })?;
        let zero = candidate.append_glue(GlueId::ZERO, GlueSpec::ZERO)?;
        let empty_origins = candidate.append_origin_list(OriginListId::EMPTY, &[])?;
        Ok(Self {
            candidate,
            token_identities: IdentityAllocator::new(1),
            macro_identities: IdentityAllocator::new(0),
            glue_identities: IdentityAllocator::new(1),
            origin_list_identities: IdentityAllocator::new(1),
            token_locations: vec![empty],
            macro_locations: Vec::new(),
            glue_locations: vec![zero],
            origin_list_locations: vec![empty_origins],
            origin_list_limit: usize::MAX,
            origin_list_entry_limit: usize::MAX,
            next_macro_observation_operand: 249_985,
            next_macro_allocation_serial: 0,
        })
    }

    pub(crate) fn mark(&self) -> Result<RuntimeValueRegistryMark, RuntimeValueRegistryError> {
        Ok(RuntimeValueRegistryMark {
            arena: self.candidate.mark()?,
            token_locations: checked_location_len(self.token_locations.len())?,
            macro_locations: checked_location_len(self.macro_locations.len())?,
            glue_locations: checked_location_len(self.glue_locations.len())?,
            origin_list_locations: checked_location_len(self.origin_list_locations.len())?,
            next_macro_observation_operand: self.next_macro_observation_operand,
            next_macro_allocation_serial: self.next_macro_allocation_serial,
        })
    }

    pub(crate) fn validate_rollback(
        &self,
        mark: RuntimeValueRegistryMark,
    ) -> Result<(), RuntimeValueRegistryError> {
        match self.candidate.validate_truncate(mark.arena) {
            Ok(()) => {}
            // Publication moves the candidate's sealed owners into the
            // canonical consumer. The old arena shape is intentionally gone,
            // but its namespace still authenticates this registry mark.
            Err(RegionArenaError::InvalidMark)
                if self.candidate.owns_mark_namespace(mark.arena) => {}
            Err(error) => return Err(error.into()),
        }
        let token_identities = self.token_identities.watermark_at(mark.token_locations)?;
        let macro_identities = self.macro_identities.watermark_at(mark.macro_locations)?;
        let glue_identities = self.glue_identities.watermark_at(mark.glue_locations)?;
        let origin_list_identities = self
            .origin_list_identities
            .watermark_at(mark.origin_list_locations)?;
        self.token_identities.validate_rollback(token_identities)?;
        self.macro_identities.validate_rollback(macro_identities)?;
        self.glue_identities.validate_rollback(glue_identities)?;
        self.origin_list_identities
            .validate_rollback(origin_list_identities)?;
        validate_location_mark(self.token_locations.len(), mark.token_locations)?;
        validate_location_mark(self.macro_locations.len(), mark.macro_locations)?;
        validate_location_mark(self.glue_locations.len(), mark.glue_locations)?;
        validate_location_mark(self.origin_list_locations.len(), mark.origin_list_locations)?;
        Ok(())
    }

    /// Restores locations and identities before reclaiming arena suffixes.
    /// Published destination roots must already be restored by the caller.
    pub(crate) fn rollback(
        &mut self,
        mark: RuntimeValueRegistryMark,
    ) -> Result<(), RuntimeValueRegistryError> {
        self.validate_rollback(mark)?;
        let token_identities = self.token_identities.watermark_at(mark.token_locations)?;
        let macro_identities = self.macro_identities.watermark_at(mark.macro_locations)?;
        let glue_identities = self.glue_identities.watermark_at(mark.glue_locations)?;
        let origin_list_identities = self
            .origin_list_identities
            .watermark_at(mark.origin_list_locations)?;
        self.token_locations.truncate(mark.token_locations as usize);
        self.macro_locations.truncate(mark.macro_locations as usize);
        self.glue_locations.truncate(mark.glue_locations as usize);
        self.origin_list_locations
            .truncate(mark.origin_list_locations as usize);
        self.token_identities.rollback(token_identities)?;
        self.macro_identities.rollback(macro_identities)?;
        self.glue_identities.rollback(glue_identities)?;
        self.origin_list_identities
            .rollback(origin_list_identities)?;
        self.next_macro_observation_operand = mark.next_macro_observation_operand;
        self.next_macro_allocation_serial = mark.next_macro_allocation_serial;
        if let Err(error) = self.candidate.truncate(mark.arena) {
            if error != RegionArenaError::InvalidMark
                || !self.candidate.owns_mark_namespace(mark.arena)
            {
                return Err(error.into());
            }
            self.candidate = RuntimeValueCandidate::from_store(self.candidate.empty_root_set())?;
        }
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
            provenance: input.provenance,
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

    /// Cold collision-safe exact lookup. Ordinary scanner publication uses
    /// `allocate_traced_token_list` and never enters this linear cold path.
    pub(crate) fn intern_token_list(
        &mut self,
        roots: &[&RuntimeValueRootSet],
        input: RuntimeTokenValueInput<'_>,
    ) -> Result<TokenListId, RuntimeValueRegistryError> {
        let provider = RuntimeValueProvider::new(&self.candidate, roots);
        for coordinate in self.token_locations.iter().copied() {
            let view = provider.admit_token_list(coordinate)?;
            if view.semantic_id() == input.semantic_id
                && view.tokens() == input.tokens
                && view.provenance() == input.provenance
            {
                return Ok(coordinate.id());
            }
        }
        self.allocate_token_list(input)
    }

    pub(crate) fn allocate_traced_token_list(
        &mut self,
        input: RuntimeTracedTokenValueInput<'_>,
    ) -> Result<TokenListId, RuntimeValueRegistryError> {
        reserve_location(&mut self.token_locations)?;
        let identity_mark = self.token_identities.watermark();
        let identity = self.token_identities.allocate()?;
        let id = TokenListId::from_identity(identity);
        let coordinate =
            match self
                .candidate
                .append_traced_token_list(RuntimeTracedTokenListInput {
                    id,
                    semantic_id: input.semantic_id,
                    words: input.words,
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
        roots: &[&RuntimeValueRootSet],
        input: RuntimeMacroValueInput<'_>,
    ) -> Result<MacroDefinitionId, RuntimeValueRegistryError> {
        let parameter_text = self.token_coordinate(input.parameter_text)?;
        let replacement_text = self.token_coordinate(input.replacement_text)?;
        let provider = RuntimeValueProvider::new(&self.candidate, roots);
        let parameter_len = u32::try_from(provider.admit_token_list(parameter_text)?.len())
            .map_err(|_| RuntimeValueRegistryError::LocationCapacityExhausted)?;
        let replacement_len = u32::try_from(provider.admit_token_list(replacement_text)?.len())
            .map_err(|_| RuntimeValueRegistryError::LocationCapacityExhausted)?;
        reserve_location(&mut self.macro_locations)?;
        let observation_operand = self.next_macro_observation_operand;
        let allocation_serial = self.next_macro_allocation_serial;
        let next_macro_observation_operand = self
            .next_macro_observation_operand
            .checked_sub(i64::from(input.observation_width))
            .ok_or(RuntimeValueRegistryError::LocationCapacityExhausted)?;
        let next_macro_allocation_serial = self
            .next_macro_allocation_serial
            .checked_add(1)
            .ok_or(RuntimeValueRegistryError::LocationCapacityExhausted)?;
        let identity_mark = self.macro_identities.watermark();
        let identity = self.macro_identities.allocate()?;
        let definition = MacroDefinitionId::from_identity(identity);
        self.next_macro_observation_operand = next_macro_observation_operand;
        self.next_macro_allocation_serial = next_macro_allocation_serial;
        let coordinate = match self.candidate.append_macro(RuntimeMacroInput {
            definition,
            flags: input.flags,
            parameter_pattern: input.parameter_pattern,
            parameter_text,
            replacement_text,
            parameter_len,
            replacement_len,
            definition_origin: input.definition_origin,
            parameter_origins: input.parameter_origins,
            replacement_origins: input.replacement_origins,
            observation_operand,
            allocation_serial,
        }) {
            Ok(coordinate) => coordinate,
            Err(error) => {
                self.macro_identities.rollback(identity_mark)?;
                self.next_macro_observation_operand = observation_operand;
                self.next_macro_allocation_serial = allocation_serial;
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

    /// Cold collision-safe exact glue lookup.
    pub(crate) fn intern_glue(
        &mut self,
        roots: &[&RuntimeValueRootSet],
        spec: GlueSpec,
    ) -> Result<GlueId, RuntimeValueRegistryError> {
        let provider = RuntimeValueProvider::new(&self.candidate, roots);
        for coordinate in self.glue_locations.iter().copied() {
            if provider.admit_glue(coordinate)?.spec() == &spec {
                return Ok(coordinate.id());
            }
        }
        self.allocate_glue(spec)
    }

    /// Cold collision-safe exact provenance-list lookup.
    pub(crate) fn intern_origin_list(
        &mut self,
        roots: &[&RuntimeValueRootSet],
        input: RuntimeOriginListValueInput<'_>,
    ) -> Result<OriginListId, RuntimeValueRegistryError> {
        let provider = RuntimeValueProvider::new(&self.candidate, roots);
        #[cfg(feature = "profiling")]
        let mut comparisons = 0;
        for coordinate in self.origin_list_locations.iter().copied() {
            #[cfg(feature = "profiling")]
            {
                comparisons += 1;
            }
            let view = provider.admit_origin_list(coordinate)?;
            if view.len() == input.origins.len() && view.iter().eq(input.origins.iter().copied()) {
                #[cfg(feature = "profiling")]
                crate::measurement::record_provenance_list_resolution(comparisons);
                #[cfg(feature = "profiling")]
                crate::measurement::record_provenance_list_intern(true, false);
                return Ok(coordinate.id());
            }
        }
        #[cfg(feature = "profiling")]
        crate::measurement::record_provenance_list_resolution(comparisons);
        let entries = self.origin_list_entry_count(roots)?;
        if self.origin_list_locations.len().saturating_sub(1) >= self.origin_list_limit
            || entries
                .checked_add(input.origins.len())
                .is_none_or(|entries| entries > self.origin_list_entry_limit)
        {
            #[cfg(feature = "profiling")]
            crate::measurement::record_provenance_list_intern(false, false);
            return Ok(OriginListId::EMPTY);
        }
        reserve_location(&mut self.origin_list_locations)?;
        let identity_mark = self.origin_list_identities.watermark();
        let identity = self.origin_list_identities.allocate()?;
        let id = OriginListId::from_identity(identity);
        let coordinate = match self.candidate.append_origin_list(id, input.origins) {
            Ok(coordinate) => coordinate,
            Err(error) => {
                self.origin_list_identities.rollback(identity_mark)?;
                return Err(error.into());
            }
        };
        debug_assert_eq!(id.raw() as usize, self.origin_list_locations.len());
        self.origin_list_locations.push(coordinate);
        #[cfg(feature = "profiling")]
        crate::measurement::record_provenance_list_intern(false, true);
        Ok(id)
    }

    pub(crate) fn token_list<'a>(
        &'a self,
        roots: &[&'a RuntimeValueRootSet],
        id: TokenListId,
    ) -> Result<RuntimeTokenListView<'a>, RuntimeValueRegistryError> {
        Ok(RuntimeValueProvider::new(&self.candidate, roots)
            .admit_token_list(self.token_coordinate(id)?)?)
    }

    pub(crate) fn macro_definition<'a>(
        &'a self,
        roots: &[&'a RuntimeValueRootSet],
        id: MacroDefinitionId,
    ) -> Result<RuntimeMacroView<'a>, RuntimeValueRegistryError> {
        Ok(RuntimeValueProvider::new(&self.candidate, roots)
            .admit_macro(self.macro_coordinate(id)?)?)
    }

    pub(crate) fn glue<'a>(
        &'a self,
        roots: &[&'a RuntimeValueRootSet],
        id: GlueId,
    ) -> Result<RuntimeGlueView<'a>, RuntimeValueRegistryError> {
        Ok(RuntimeValueProvider::new(&self.candidate, roots)
            .admit_glue(self.glue_coordinate(id)?)?)
    }

    pub(crate) fn origin_list<'a>(
        &'a self,
        roots: &[&'a RuntimeValueRootSet],
        id: OriginListId,
    ) -> Result<RuntimeOriginListView<'a>, RuntimeValueRegistryError> {
        Ok(RuntimeValueProvider::new(&self.candidate, roots)
            .admit_origin_list(self.origin_list_coordinate(id)?)?)
    }

    pub(crate) fn token_len(&self) -> u32 {
        u32::try_from(self.token_locations.len()).expect("token location count exceeds u32")
    }

    pub(crate) fn macro_len(&self) -> u32 {
        u32::try_from(self.macro_locations.len()).expect("macro location count exceeds u32")
    }

    pub(crate) fn glue_len(&self) -> u32 {
        u32::try_from(self.glue_locations.len()).expect("glue location count exceeds u32")
    }

    pub(crate) fn origin_list_len(&self) -> u32 {
        u32::try_from(self.origin_list_locations.len())
            .expect("origin-list location count exceeds u32")
    }

    pub(crate) fn token_id_at(&self, raw: u32) -> Option<TokenListId> {
        self.token_locations
            .get(raw as usize)
            .map(|value| value.id())
    }

    pub(crate) fn macro_id_at(&self, raw: u32) -> Option<MacroDefinitionId> {
        self.macro_locations
            .get(raw as usize)
            .map(|value| value.id())
    }

    pub(crate) fn glue_id_at(&self, raw: u32) -> Option<GlueId> {
        self.glue_locations
            .get(raw as usize)
            .map(|value| value.id())
    }

    pub(crate) fn origin_list_id_at(&self, raw: u32) -> Option<OriginListId> {
        self.origin_list_locations
            .get(raw as usize)
            .map(|value| value.id())
    }

    pub(crate) fn contains_token(&self, id: TokenListId) -> bool {
        self.token_coordinate(id).is_ok()
    }

    pub(crate) fn contains_macro(&self, id: MacroDefinitionId) -> bool {
        self.macro_coordinate(id).is_ok()
    }

    pub(crate) fn contains_glue(&self, id: GlueId) -> bool {
        self.glue_coordinate(id).is_ok()
    }

    pub(crate) fn contains_origin_list(&self, id: OriginListId) -> bool {
        self.origin_list_coordinate(id).is_ok()
    }

    pub(crate) fn install_frozen_token_list(
        &mut self,
        roots: &[&RuntimeValueRootSet],
        expected_raw: u32,
        input: RuntimeTokenValueInput<'_>,
    ) -> Result<TokenListId, RuntimeValueRegistryError> {
        if expected_raw == TokenListId::EMPTY.raw() {
            let empty = self.token_list(roots, TokenListId::EMPTY)?;
            return (empty.tokens() == input.tokens && empty.semantic_id() == input.semantic_id)
                .then_some(TokenListId::EMPTY)
                .ok_or(RuntimeValueRegistryError::UnknownTokenList);
        }
        let mark = self.mark()?;
        let id = self.allocate_token_list(input)?;
        if id.raw() == expected_raw {
            Ok(id)
        } else {
            self.rollback(mark)?;
            Err(RuntimeValueRegistryError::InvalidMark)
        }
    }

    pub(crate) fn install_frozen_macro(
        &mut self,
        roots: &[&RuntimeValueRootSet],
        expected_raw: u32,
        input: RuntimeMacroValueInput<'_>,
    ) -> Result<MacroDefinitionId, RuntimeValueRegistryError> {
        let mark = self.mark()?;
        let id = self.allocate_macro(roots, input)?;
        if id.raw() == expected_raw {
            Ok(id)
        } else {
            self.rollback(mark)?;
            Err(RuntimeValueRegistryError::InvalidMark)
        }
    }

    pub(crate) fn install_frozen_glue(
        &mut self,
        roots: &[&RuntimeValueRootSet],
        expected_raw: u32,
        spec: GlueSpec,
    ) -> Result<GlueId, RuntimeValueRegistryError> {
        if expected_raw == GlueId::ZERO.raw() {
            return (self.glue(roots, GlueId::ZERO)?.spec() == &spec)
                .then_some(GlueId::ZERO)
                .ok_or(RuntimeValueRegistryError::UnknownGlue);
        }
        let mark = self.mark()?;
        let id = self.allocate_glue(spec)?;
        if id.raw() == expected_raw {
            Ok(id)
        } else {
            self.rollback(mark)?;
            Err(RuntimeValueRegistryError::InvalidMark)
        }
    }

    /// Adds only region owners absent from the current canonical root set.
    pub(crate) fn publish_into(
        &mut self,
        destination: &mut RuntimeValueRootSet,
    ) -> Result<(), RuntimeValueRegistryError> {
        self.candidate.publish_into(destination)?;
        Ok(())
    }

    pub(crate) fn retain_token_list_into(
        &self,
        source: &RuntimeValueRootSet,
        destination: &mut RuntimeValueRootSet,
        id: TokenListId,
    ) -> Result<(), RuntimeValueRegistryError> {
        destination.retain_token_list_from(
            source,
            self.token_coordinate(id)?,
            NonZeroUsize::MIN,
        )?;
        Ok(())
    }

    pub(crate) fn retain_glue_into(
        &self,
        source: &RuntimeValueRootSet,
        destination: &mut RuntimeValueRootSet,
        id: GlueId,
    ) -> Result<(), RuntimeValueRegistryError> {
        destination.retain_glue_from(source, self.glue_coordinate(id)?, NonZeroUsize::MIN)?;
        Ok(())
    }

    /// Rebuilds a generation fork at an inherited published checkpoint.
    ///
    /// The parent arena mark names the source candidate namespace and cannot
    /// be replayed in the child's fresh suffix namespace. The retained
    /// location and identity prefixes remain exact, while `published` owns
    /// precisely the sealed regions accepted at that checkpoint.
    pub(crate) fn rollback_inherited(
        &mut self,
        mark: RuntimeValueRegistryMark,
        published: &RuntimeValueRootSet,
    ) -> Result<(), RuntimeValueRegistryError> {
        self.restore_inherited_prefix(mark, published)
    }

    /// Builds a generation fork directly from a checkpoint's sealed roots.
    ///
    /// Dense family coordinates and identity metadata are copy-only. The
    /// mutable source suffix is neither copied nor admitted into the child.
    pub(crate) fn fork_at(
        &self,
        mark: RuntimeValueRegistryMark,
        published: &RuntimeValueRootSet,
    ) -> Result<Self, RuntimeValueRegistryError> {
        let mut child = Self {
            candidate: RuntimeValueCandidate::from_store(published.clone())?,
            token_identities: self.token_identities.fork(),
            macro_identities: self.macro_identities.fork(),
            glue_identities: self.glue_identities.fork(),
            origin_list_identities: self.origin_list_identities.fork(),
            token_locations: self.token_locations.clone(),
            macro_locations: self.macro_locations.clone(),
            glue_locations: self.glue_locations.clone(),
            origin_list_locations: self.origin_list_locations.clone(),
            origin_list_limit: self.origin_list_limit,
            origin_list_entry_limit: self.origin_list_entry_limit,
            next_macro_observation_operand: self.next_macro_observation_operand,
            next_macro_allocation_serial: self.next_macro_allocation_serial,
        };
        child.restore_inherited_prefix(mark, published)?;
        Ok(child)
    }

    fn restore_inherited_prefix(
        &mut self,
        mark: RuntimeValueRegistryMark,
        published: &RuntimeValueRootSet,
    ) -> Result<(), RuntimeValueRegistryError> {
        let token_identities = self.token_identities.watermark_at(mark.token_locations)?;
        let macro_identities = self.macro_identities.watermark_at(mark.macro_locations)?;
        let glue_identities = self.glue_identities.watermark_at(mark.glue_locations)?;
        let origin_list_identities = self
            .origin_list_identities
            .watermark_at(mark.origin_list_locations)?;
        self.token_identities.validate_rollback(token_identities)?;
        self.macro_identities.validate_rollback(macro_identities)?;
        self.glue_identities.validate_rollback(glue_identities)?;
        self.origin_list_identities
            .validate_rollback(origin_list_identities)?;
        validate_location_mark(self.token_locations.len(), mark.token_locations)?;
        validate_location_mark(self.macro_locations.len(), mark.macro_locations)?;
        validate_location_mark(self.glue_locations.len(), mark.glue_locations)?;
        validate_location_mark(self.origin_list_locations.len(), mark.origin_list_locations)?;

        self.token_locations.truncate(mark.token_locations as usize);
        self.macro_locations.truncate(mark.macro_locations as usize);
        self.glue_locations.truncate(mark.glue_locations as usize);
        self.origin_list_locations
            .truncate(mark.origin_list_locations as usize);
        self.token_identities.rollback(token_identities)?;
        self.macro_identities.rollback(macro_identities)?;
        self.glue_identities.rollback(glue_identities)?;
        self.origin_list_identities
            .rollback(origin_list_identities)?;
        self.next_macro_observation_operand = mark.next_macro_observation_operand;
        self.next_macro_allocation_serial = mark.next_macro_allocation_serial;
        self.candidate = RuntimeValueCandidate::from_store(published.empty_root_set())?;

        let roots = [published];
        let provider = RuntimeValueProvider::new(&self.candidate, &roots);
        for coordinate in self.token_locations.iter().copied() {
            provider.admit_token_list(coordinate)?;
        }
        for coordinate in self.macro_locations.iter().copied() {
            provider.admit_macro(coordinate)?;
        }
        for coordinate in self.glue_locations.iter().copied() {
            provider.admit_glue(coordinate)?;
        }
        for coordinate in self.origin_list_locations.iter().copied() {
            provider.admit_origin_list(coordinate)?;
        }
        Ok(())
    }

    /// Cold generation fork. Immutable regions share one owner each; values
    /// in the private active region alone are copied into a fresh namespace.
    pub(crate) fn fork(
        &self,
        roots: &[&RuntimeValueRootSet],
    ) -> Result<Self, RuntimeValueRegistryError> {
        let active_owner = self.candidate.active_owner();
        let mut child = Self {
            candidate: RuntimeValueCandidate::from_store(self.candidate.empty_root_set())?,
            token_identities: self.token_identities.fork(),
            macro_identities: self.macro_identities.fork(),
            glue_identities: self.glue_identities.fork(),
            origin_list_identities: self.origin_list_identities.fork(),
            token_locations: self.token_locations.clone(),
            macro_locations: self.macro_locations.clone(),
            glue_locations: self.glue_locations.clone(),
            origin_list_locations: self.origin_list_locations.clone(),
            origin_list_limit: self.origin_list_limit,
            origin_list_entry_limit: self.origin_list_entry_limit,
            next_macro_observation_operand: self.next_macro_observation_operand,
            next_macro_allocation_serial: self.next_macro_allocation_serial,
        };
        let Some(active_owner) = active_owner else {
            return Ok(child);
        };

        let provider = RuntimeValueProvider::new(&self.candidate, roots);
        for (slot, coordinate) in self.token_locations.iter().copied().enumerate() {
            if coordinate.owner() != active_owner {
                continue;
            }
            let view = self.candidate.admit_token_list(coordinate)?;
            let copied = child.candidate.append_token_list(RuntimeTokenListInput {
                id: coordinate.id(),
                semantic_id: view.semantic_id(),
                tokens: view.tokens(),
                provenance: view.provenance(),
            })?;
            child.token_locations[slot] = copied;
        }
        for (slot, coordinate) in self.macro_locations.iter().copied().enumerate() {
            if coordinate.owner() != active_owner {
                continue;
            }
            let view = provider.admit_macro(coordinate)?;
            let meaning = view.meaning();
            let parameter_text = child.token_coordinate(meaning.parameter_text())?;
            let replacement_text = child.token_coordinate(meaning.replacement_text())?;
            let copied = child.candidate.append_macro(RuntimeMacroInput {
                definition: coordinate.id(),
                flags: meaning.flags(),
                parameter_pattern: view.parameter_pattern(),
                parameter_text,
                replacement_text,
                parameter_len: u32::try_from(view.parameter_text().len())
                    .map_err(|_| RuntimeValueRegistryError::LocationCapacityExhausted)?,
                replacement_len: u32::try_from(view.replacement_text().len())
                    .map_err(|_| RuntimeValueRegistryError::LocationCapacityExhausted)?,
                definition_origin: view.definition_origin(),
                parameter_origins: view.parameter_origins(),
                replacement_origins: view.replacement_origins(),
                observation_operand: view.observation_operand(),
                allocation_serial: view.allocation_serial(),
            })?;
            child.macro_locations[slot] = copied;
        }
        for (slot, coordinate) in self.glue_locations.iter().copied().enumerate() {
            if coordinate.owner() != active_owner {
                continue;
            }
            let view = self.candidate.admit_glue(coordinate)?;
            let copied = child.candidate.append_glue(coordinate.id(), *view.spec())?;
            child.glue_locations[slot] = copied;
        }
        for (slot, coordinate) in self.origin_list_locations.iter().copied().enumerate() {
            if coordinate.owner() != active_owner {
                continue;
            }
            let view = self.candidate.admit_origin_list(coordinate)?;
            let origins = view.iter().collect::<Vec<_>>();
            let copied = child
                .candidate
                .append_origin_list(coordinate.id(), &origins)?;
            child.origin_list_locations[slot] = copied;
        }
        Ok(child)
    }

    pub(crate) fn empty_root_set(&self) -> RuntimeValueRootSet {
        self.candidate.empty_root_set()
    }

    pub(crate) fn accounting(&self) -> RuntimeValueRegistryAccounting {
        let token_shape = self.token_identities.storage_shape();
        let macro_shape = self.macro_identities.storage_shape();
        let glue_shape = self.glue_identities.storage_shape();
        let origin_list_shape = self.origin_list_identities.storage_shape();
        RuntimeValueRegistryAccounting {
            regions: self.candidate.accounting(),
            token_locations: self.token_locations.len(),
            macro_locations: self.macro_locations.len(),
            glue_locations: self.glue_locations.len(),
            origin_list_locations: self.origin_list_locations.len(),
            retained_location_slots: self
                .token_locations
                .capacity()
                .saturating_add(self.macro_locations.capacity())
                .saturating_add(self.glue_locations.capacity())
                .saturating_add(self.origin_list_locations.capacity()),
            identity_slots: token_shape
                .0
                .saturating_add(macro_shape.0)
                .saturating_add(glue_shape.0)
                .saturating_add(origin_list_shape.0),
            retained_identity_slots: token_shape
                .1
                .saturating_add(macro_shape.1)
                .saturating_add(glue_shape.1)
                .saturating_add(origin_list_shape.1),
        }
    }

    pub(crate) fn configure_origin_list_budgets(&mut self, lists: usize, entries: usize) {
        self.origin_list_limit = lists;
        self.origin_list_entry_limit = entries;
    }

    pub(crate) fn origin_list_accounting(
        &self,
        roots: &[&RuntimeValueRootSet],
    ) -> Result<RuntimeOriginListAccounting, RuntimeValueRegistryError> {
        let entries = self.origin_list_entry_count(roots)?;
        let identity_shape = self.origin_list_identities.storage_shape();
        let regions = self.candidate.accounting();
        let retained_list_bytes = self
            .origin_list_locations
            .capacity()
            .saturating_mul(size_of::<RuntimeOriginListCoordinate>());
        let retained_identity_bytes = identity_shape.1.saturating_mul(size_of::<HandleIdentity>());
        Ok(RuntimeOriginListAccounting {
            lists: self.origin_list_locations.len().saturating_sub(1),
            entries,
            retained_list_slots: self.origin_list_locations.capacity(),
            retained_entry_slots: regions.retained_provenance_values,
            retained_bytes: retained_list_bytes
                .saturating_add(retained_identity_bytes)
                .saturating_add(regions.retained_provenance_bytes),
        })
    }

    fn origin_list_entry_count(
        &self,
        roots: &[&RuntimeValueRootSet],
    ) -> Result<usize, RuntimeValueRegistryError> {
        let provider = RuntimeValueProvider::new(&self.candidate, roots);
        self.origin_list_locations
            .iter()
            .copied()
            .try_fold(0_usize, |entries, coordinate| {
                Ok(entries.saturating_add(provider.admit_origin_list(coordinate)?.len()))
            })
    }

    pub(crate) fn token_coordinate(
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

    fn origin_list_coordinate(
        &self,
        id: OriginListId,
    ) -> Result<RuntimeOriginListCoordinate, RuntimeValueRegistryError> {
        if !self.origin_list_identities.contains(id.identity()) {
            return Err(RuntimeValueRegistryError::UnknownOriginList);
        }
        self.origin_list_locations
            .get(id.raw() as usize)
            .copied()
            .filter(|coordinate| coordinate.id() == id)
            .ok_or(RuntimeValueRegistryError::UnknownOriginList)
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
