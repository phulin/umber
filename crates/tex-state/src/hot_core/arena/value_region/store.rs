//! Concrete runtime token, macro, glue, and provenance region facade.
//!
//! This module is the semantic seam over the generic six-column arena. Values
//! carry copy-only coordinates; accepted stores and narrower canonical root
//! sets retain one sealed owner per region.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::num::{NonZeroU32, NonZeroUsize};

use crate::glue::GlueSpec;
use crate::ids::{GlueId, MacroDefinitionId, OriginListId, TokenListId};
use crate::macro_store::{MacroMeaning, MacroParameterPattern};
use crate::meaning::MeaningFlags;
use crate::token::{OriginId, RootedTracedTokenWord, Token, TracedTokenWord};
use crate::token_store::TokenSemanticId;

use super::{
    AcceptedRuntimeValueRegions, AdmittedRuntimeValueRegion, ChunkOwner, RegionArenaError,
    RegionCoordinate, RuntimeValueRegionAccounting, RuntimeValueRegionArena,
    RuntimeValueRegionMark, checked_offset,
};

pub(crate) mod registry;
mod storage;

use storage::*;

type ConcreteRegions = AcceptedRuntimeValueRegions<
    Token,
    RuntimeTokenListRow,
    RuntimeMacroRecord,
    RuntimeMacroRootRow,
    GlueSpec,
    RuntimeOriginEntry,
>;

type ConcreteArena = RuntimeValueRegionArena<
    Token,
    RuntimeTokenListRow,
    RuntimeMacroRecord,
    RuntimeMacroRootRow,
    GlueSpec,
    RuntimeOriginEntry,
>;

type ConcreteAdmission<'a> = AdmittedRuntimeValueRegion<
    'a,
    Token,
    RuntimeTokenListRow,
    RuntimeMacroRecord,
    RuntimeMacroRootRow,
    GlueSpec,
    RuntimeOriginEntry,
>;

/// A compact span local to one already-admitted region.
struct LocalSpan<T> {
    start: u32,
    len: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> Copy for LocalSpan<T> {}

impl<T> Clone for LocalSpan<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for LocalSpan<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalSpan")
            .field("start", &self.start)
            .field("len", &self.len)
            .finish()
    }
}

impl<T> PartialEq for LocalSpan<T> {
    fn eq(&self, other: &Self) -> bool {
        self.start == other.start && self.len == other.len
    }
}

impl<T> Eq for LocalSpan<T> {}

impl<T> LocalSpan<T> {
    const fn new(start: u32, len: u32) -> Self {
        Self {
            start,
            len,
            marker: PhantomData,
        }
    }
}

/// One immutable token-list row in a runtime value region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeTokenListRow {
    id: TokenListId,
    semantic_id: TokenSemanticId,
    tokens: LocalSpan<Token>,
    provenance: LocalSpan<RuntimeOriginEntry>,
}

/// Copy-only sparse provenance for one token offset.
///
/// Structural ownership remains solely in `ProvenanceStore`'s archive. This
/// row is a compact lookup coordinate and never clones an `OriginRef`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RuntimeOriginEntry {
    token_offset: u32,
    origin: OriginId,
}

impl RuntimeOriginEntry {
    #[must_use]
    pub(crate) const fn new(token_offset: u32, origin: OriginId) -> Self {
        Self {
            token_offset,
            origin,
        }
    }

    pub(crate) const fn token_offset(&self) -> u32 {
        self.token_offset
    }

    pub(crate) const fn origin(self) -> OriginId {
        self.origin
    }
}

/// Copy-only identity and physical span for one exact origin sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeOriginListCoordinate {
    owner: ChunkOwner,
    origins: LocalSpan<RuntimeOriginEntry>,
    id: OriginListId,
}

impl RuntimeOriginListCoordinate {
    pub(crate) const fn id(self) -> OriginListId {
        self.id
    }

    pub(crate) const fn owner(self) -> ChunkOwner {
        self.owner
    }
}

/// Borrowed exact origin sequence admitted through the live registry.
pub(crate) struct RuntimeOriginListView<'a> {
    coordinate: RuntimeOriginListCoordinate,
    origins: &'a [RuntimeOriginEntry],
}

impl RuntimeOriginListView<'_> {
    pub(crate) const fn coordinate(&self) -> RuntimeOriginListCoordinate {
        self.coordinate
    }

    pub(crate) const fn len(&self) -> usize {
        self.origins.len()
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    pub(crate) fn origin(&self, index: usize) -> Option<OriginId> {
        self.origins
            .get(index)
            .copied()
            .map(RuntimeOriginEntry::origin)
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = OriginId> + '_ {
        self.origins.iter().copied().map(RuntimeOriginEntry::origin)
    }
}

/// Copy-only token-list identity and physical row coordinate.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeTokenListCoordinate {
    row: RegionCoordinate<RuntimeTokenListRow>,
    id: TokenListId,
}

impl RuntimeTokenListCoordinate {
    pub(crate) const fn id(self) -> TokenListId {
        self.id
    }

    pub(crate) const fn owner(self) -> ChunkOwner {
        self.row.owner()
    }
}

impl PartialEq for RuntimeTokenListCoordinate {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RuntimeTokenListCoordinate {}

impl Hash for RuntimeTokenListCoordinate {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Borrowed token words and their sparse structural provenance roots.
pub(crate) struct RuntimeTokenListView<'a> {
    coordinate: RuntimeTokenListCoordinate,
    semantic_id: TokenSemanticId,
    tokens: &'a [Token],
    provenance: &'a [RuntimeOriginEntry],
}

impl<'a> RuntimeTokenListView<'a> {
    pub(crate) const fn coordinate(&self) -> RuntimeTokenListCoordinate {
        self.coordinate
    }

    pub(crate) const fn semantic_id(&self) -> TokenSemanticId {
        self.semantic_id
    }

    pub(crate) const fn tokens(&self) -> &'a [Token] {
        self.tokens
    }

    pub(crate) const fn provenance(&self) -> &'a [RuntimeOriginEntry] {
        self.provenance
    }

    /// Reconstructs the packed traced word at a cold provenance boundary.
    pub(crate) fn traced_word(&self, index: usize) -> Option<TracedTokenWord> {
        let token = *self.tokens.get(index)?;
        let origin = self
            .provenance
            .binary_search_by_key(&(index as u32), RuntimeOriginEntry::token_offset)
            .map_or(crate::token::OriginId::UNKNOWN, |root| {
                self.provenance[root].origin()
            });
        Some(TracedTokenWord::pack(token, origin))
    }

    /// Reconstructs a structurally rooted word only for cold consumers.
    pub(crate) fn rooted_word(
        &self,
        index: usize,
        mut resolve: impl FnMut(OriginId) -> Option<crate::provenance::OriginRef>,
    ) -> Option<RootedTracedTokenWord> {
        let word = self.traced_word(index)?;
        let origin = resolve(word.origin())?;
        Some(RootedTracedTokenWord::from_word(word, origin))
    }
}

impl AsRef<[Token]> for RuntimeTokenListView<'_> {
    fn as_ref(&self) -> &[Token] {
        self.tokens
    }
}

impl core::ops::Deref for RuntimeTokenListView<'_> {
    type Target = [Token];

    fn deref(&self) -> &Self::Target {
        self.tokens
    }
}

/// Fixed macro meaning and replay metadata aligned with one root row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeMacroRecord {
    definition: MacroDefinitionId,
    flags: MeaningFlags,
    parameter_pattern: MacroParameterPattern,
    parameter_len: u32,
    replacement_len: u32,
    observation_operand: i64,
    allocation_serial: u64,
}

/// Region-owned roots aligned one-for-one with [`RuntimeMacroRecord`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeMacroRootRow {
    definition: MacroDefinitionId,
    parameter_text: RuntimeTokenListCoordinate,
    replacement_text: RuntimeTokenListCoordinate,
    definition_origin: OriginId,
    parameter_origins: LocalSpan<RuntimeOriginEntry>,
    replacement_origins: LocalSpan<RuntimeOriginEntry>,
}

/// Copy-only macro-definition identity and aligned record/root coordinate.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeMacroCoordinate {
    row: RegionCoordinate<RuntimeMacroRecord>,
    id: MacroDefinitionId,
}

impl PartialEq for RuntimeMacroCoordinate {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RuntimeMacroCoordinate {}

impl Hash for RuntimeMacroCoordinate {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl RuntimeMacroCoordinate {
    pub(crate) const fn id(self) -> MacroDefinitionId {
        self.id
    }

    pub(crate) const fn owner(self) -> ChunkOwner {
        self.row.owner()
    }
}

impl RuntimeMacroRecord {
    pub(crate) const fn definition(self) -> MacroDefinitionId {
        self.definition
    }

    pub(crate) const fn flags(self) -> MeaningFlags {
        self.flags
    }

    pub(crate) const fn parameter_len(self) -> u32 {
        self.parameter_len
    }

    pub(crate) const fn replacement_len(self) -> u32 {
        self.replacement_len
    }
}

/// Borrowed macro record, token lists, and compact provenance closure.
pub(crate) struct RuntimeMacroView<'a> {
    coordinate: RuntimeMacroCoordinate,
    record: &'a RuntimeMacroRecord,
    parameter_text: RuntimeTokenListView<'a>,
    replacement_text: RuntimeTokenListView<'a>,
    definition_origin: OriginId,
    parameter_origins: &'a [RuntimeOriginEntry],
    replacement_origins: &'a [RuntimeOriginEntry],
}

impl<'a> RuntimeMacroView<'a> {
    pub(crate) const fn coordinate(&self) -> RuntimeMacroCoordinate {
        self.coordinate
    }

    pub(crate) const fn record(&self) -> &'a RuntimeMacroRecord {
        self.record
    }

    pub(crate) const fn meaning(&self) -> MacroMeaning {
        MacroMeaning::new(
            self.record.flags,
            self.parameter_text.coordinate.id,
            self.replacement_text.coordinate.id,
        )
    }

    pub(crate) const fn parameter_pattern(&self) -> MacroParameterPattern {
        self.record.parameter_pattern
    }

    pub(crate) const fn parameter_text(&self) -> &RuntimeTokenListView<'a> {
        &self.parameter_text
    }

    pub(crate) const fn replacement_text(&self) -> &RuntimeTokenListView<'a> {
        &self.replacement_text
    }

    pub(crate) const fn definition_origin(&self) -> OriginId {
        self.definition_origin
    }

    pub(crate) const fn parameter_origins(&self) -> &'a [RuntimeOriginEntry] {
        self.parameter_origins
    }

    pub(crate) const fn replacement_origins(&self) -> &'a [RuntimeOriginEntry] {
        self.replacement_origins
    }

    pub(crate) fn has_provenance(&self) -> bool {
        self.definition_origin != OriginId::UNKNOWN
            || !self.parameter_origins.is_empty()
            || !self.replacement_origins.is_empty()
            || !self.parameter_text.provenance().is_empty()
            || !self.replacement_text.provenance().is_empty()
    }

    pub(crate) fn parameter_traced_word(&self, index: usize) -> Option<TracedTokenWord> {
        traced_word_with_overrides(&self.parameter_text, self.parameter_origins, index)
    }

    pub(crate) fn replacement_traced_word(&self, index: usize) -> Option<TracedTokenWord> {
        traced_word_with_overrides(&self.replacement_text, self.replacement_origins, index)
    }

    pub(crate) const fn observation_operand(&self) -> i64 {
        self.record.observation_operand
    }

    pub(crate) const fn allocation_serial(&self) -> u64 {
        self.record.allocation_serial
    }
}

fn traced_word_with_overrides(
    tokens: &RuntimeTokenListView<'_>,
    provenance: &[RuntimeOriginEntry],
    index: usize,
) -> Option<TracedTokenWord> {
    let token = *tokens.tokens().get(index)?;
    let origin = provenance
        .binary_search_by_key(&(index as u32), RuntimeOriginEntry::token_offset)
        .map_or_else(
            |_| {
                tokens
                    .traced_word(index)
                    .expect("token index was already bounded")
                    .origin()
            },
            |entry| provenance[entry].origin(),
        );
    Some(TracedTokenWord::pack(token, origin))
}

/// Copy-only glue identity and physical row coordinate.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeGlueCoordinate {
    row: RegionCoordinate<GlueSpec>,
    id: GlueId,
}

impl PartialEq for RuntimeGlueCoordinate {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RuntimeGlueCoordinate {}

impl Hash for RuntimeGlueCoordinate {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl RuntimeGlueCoordinate {
    pub(crate) const fn id(self) -> GlueId {
        self.id
    }

    pub(crate) const fn owner(self) -> ChunkOwner {
        self.row.owner()
    }
}

/// Borrowed glue value after one region admission.
pub(crate) struct RuntimeGlueView<'a> {
    coordinate: RuntimeGlueCoordinate,
    spec: &'a GlueSpec,
}

impl<'a> RuntimeGlueView<'a> {
    pub(crate) const fn coordinate(&self) -> RuntimeGlueCoordinate {
        self.coordinate
    }

    pub(crate) const fn spec(&self) -> &'a GlueSpec {
        self.spec
    }
}

/// Input used to append one complete token-list row atomically.
pub(crate) struct RuntimeTokenListInput<'a> {
    pub(crate) id: TokenListId,
    pub(crate) semantic_id: TokenSemanticId,
    pub(crate) tokens: &'a [Token],
    pub(crate) provenance: &'a [RuntimeOriginEntry],
}

/// Scanner-owned traced words appended without an intermediate token/root heap.
pub(crate) struct RuntimeTracedTokenListInput<'a> {
    pub(crate) id: TokenListId,
    pub(crate) semantic_id: TokenSemanticId,
    pub(crate) words: &'a [TracedTokenWord],
}

/// Input used to append one record/root/provenance macro composite atomically.
pub(crate) struct RuntimeMacroInput<'a> {
    pub(crate) definition: MacroDefinitionId,
    pub(crate) flags: MeaningFlags,
    pub(crate) parameter_pattern: MacroParameterPattern,
    pub(crate) parameter_text: RuntimeTokenListCoordinate,
    pub(crate) replacement_text: RuntimeTokenListCoordinate,
    pub(crate) definition_origin: OriginId,
    pub(crate) parameter_origins: &'a [RuntimeOriginEntry],
    pub(crate) replacement_origins: &'a [RuntimeOriginEntry],
    pub(crate) observation_operand: i64,
    pub(crate) allocation_serial: u64,
}

/// Accepted region owners and their local use counts.
///
/// A full store, Env roots, a journal frame, or an input continuation can all
/// use this same root-set representation. Cloning retains once per region.
#[derive(Clone)]
pub(crate) struct RuntimeValueStore {
    regions: ConcreteRegions,
}

impl fmt::Debug for RuntimeValueStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeValueStore")
            .field("accounting", &self.accounting())
            .finish()
    }
}

/// Fixed-size restoration point for a canonical published-region root set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeValueStorePublicationMark {
    regions: u32,
}

impl RuntimeValueStore {
    pub(crate) const fn new(initial_region_capacity: NonZeroU32) -> Self {
        Self {
            regions: ConcreteRegions::new(initial_region_capacity),
        }
    }

    pub(crate) fn candidate(&self) -> Result<RuntimeValueCandidate, RegionArenaError> {
        Ok(RuntimeValueCandidate {
            arena: self.regions.candidate()?,
        })
    }

    /// Makes an empty canonical root set with the same allocation policy.
    pub(crate) fn empty_root_set(&self) -> Self {
        Self {
            regions: self.regions.retain_regions(&[]),
        }
    }

    pub(crate) fn publication_mark(
        &self,
    ) -> Result<RuntimeValueStorePublicationMark, RegionArenaError> {
        Ok(RuntimeValueStorePublicationMark {
            regions: u32::try_from(self.regions.regions.len())
                .map_err(|_| RegionArenaError::SlotCapacityExhausted)?,
        })
    }

    /// Restores roots before the owning candidate rolls back its sealed suffix.
    pub(crate) fn restore_publication(
        &mut self,
        mark: RuntimeValueStorePublicationMark,
    ) -> Result<(), RegionArenaError> {
        if mark.regions as usize > self.regions.regions.len() {
            return Err(RegionArenaError::InvalidMark);
        }
        self.regions.regions.truncate(mark.regions as usize);
        Ok(())
    }

    pub(crate) fn admit_token_list(
        &self,
        coordinate: RuntimeTokenListCoordinate,
    ) -> Result<RuntimeTokenListView<'_>, RegionArenaError> {
        token_list_view(&self.regions, coordinate)
    }

    pub(crate) fn admit_macro(
        &self,
        coordinate: RuntimeMacroCoordinate,
    ) -> Result<RuntimeMacroView<'_>, RegionArenaError> {
        let admitted = self.regions.admit(coordinate.owner())?;
        let record = admitted.macro_record(coordinate.row)?;
        let root_coordinate = coordinate
            .owner()
            .coordinate::<RuntimeMacroRootRow>(coordinate.row.offset());
        let roots = admitted.macro_root(root_coordinate)?;
        validate_macro_identity(coordinate, record, roots)?;
        let parameter_text =
            token_list_view_from_admission(&self.regions, &admitted, roots.parameter_text)?;
        let replacement_text =
            token_list_view_from_admission(&self.regions, &admitted, roots.replacement_text)?;
        let definition_origin = roots.definition_origin;
        let parameter_origins = provenance_span(&admitted, roots.parameter_origins)?;
        let replacement_origins = provenance_span(&admitted, roots.replacement_origins)?;
        Ok(RuntimeMacroView {
            coordinate,
            record,
            parameter_text,
            replacement_text,
            definition_origin,
            parameter_origins,
            replacement_origins,
        })
    }

    pub(crate) fn admit_glue(
        &self,
        coordinate: RuntimeGlueCoordinate,
    ) -> Result<RuntimeGlueView<'_>, RegionArenaError> {
        let admitted = self.regions.admit(coordinate.owner())?;
        Ok(RuntimeGlueView {
            coordinate,
            spec: admitted.glue(coordinate.row)?,
        })
    }

    pub(crate) fn admit_origin_list(
        &self,
        coordinate: RuntimeOriginListCoordinate,
    ) -> Result<RuntimeOriginListView<'_>, RegionArenaError> {
        let admitted = self.regions.admit(coordinate.owner())?;
        Ok(RuntimeOriginListView {
            coordinate,
            origins: provenance_span(&admitted, coordinate.origins)?,
        })
    }

    pub(crate) fn retain_token_list_from(
        &mut self,
        source: &Self,
        coordinate: RuntimeTokenListCoordinate,
        uses: NonZeroUsize,
    ) -> Result<(), RegionArenaError> {
        source.admit_token_list(coordinate)?;
        self.regions
            .retain_from(&source.regions, coordinate.owner(), uses)
    }

    pub(crate) fn release_token_list(
        &mut self,
        coordinate: RuntimeTokenListCoordinate,
        uses: NonZeroUsize,
    ) -> Result<(), RegionArenaError> {
        self.regions.release(coordinate.owner(), uses)
    }

    /// Retains the macro region and both token-list regions as one closure.
    pub(crate) fn retain_macro_from(
        &mut self,
        source: &Self,
        coordinate: RuntimeMacroCoordinate,
        uses: NonZeroUsize,
    ) -> Result<(), RegionArenaError> {
        let view = source.admit_macro(coordinate)?;
        let owners = [
            coordinate.owner(),
            view.parameter_text.coordinate.owner(),
            view.replacement_text.coordinate.owner(),
        ];
        for owner in owners {
            self.regions.retain_from(&source.regions, owner, uses)?;
        }
        Ok(())
    }

    pub(crate) fn release_macro(
        &mut self,
        coordinate: RuntimeMacroCoordinate,
        uses: NonZeroUsize,
    ) -> Result<(), RegionArenaError> {
        let view = self.admit_macro(coordinate)?;
        let owners = [
            coordinate.owner(),
            view.parameter_text.coordinate.owner(),
            view.replacement_text.coordinate.owner(),
        ];
        for owner in owners.into_iter().rev() {
            self.regions.release(owner, uses)?;
        }
        Ok(())
    }

    pub(crate) fn retain_glue_from(
        &mut self,
        source: &Self,
        coordinate: RuntimeGlueCoordinate,
        uses: NonZeroUsize,
    ) -> Result<(), RegionArenaError> {
        source.admit_glue(coordinate)?;
        self.regions
            .retain_from(&source.regions, coordinate.owner(), uses)
    }

    pub(crate) fn release_glue(
        &mut self,
        coordinate: RuntimeGlueCoordinate,
        uses: NonZeroUsize,
    ) -> Result<(), RegionArenaError> {
        self.regions.release(coordinate.owner(), uses)
    }

    pub(crate) fn accounting(&self) -> RuntimeValueRegionAccounting {
        self.regions.accounting()
    }

    #[cfg(test)]
    fn testing_uses(&self, owner: ChunkOwner) -> usize {
        self.regions.testing_uses(owner)
    }
}

/// Mutable rollback-owned suffix over an accepted [`RuntimeValueStore`].
pub(crate) struct RuntimeValueCandidate {
    arena: ConcreteArena,
}

impl RuntimeValueCandidate {
    fn from_store(store: RuntimeValueStore) -> Result<Self, RegionArenaError> {
        Ok(Self {
            arena: RuntimeValueRegionArena::new(store.regions)?,
        })
    }

    /// Shares every immutable region while leaving the private active suffix
    /// for the cold fork path to copy into its own namespace.
    fn sealed_store(&self) -> RuntimeValueStore {
        RuntimeValueStore {
            regions: clone_sealed_regions(&self.arena),
        }
    }

    fn active_owner(&self) -> Option<ChunkOwner> {
        self.arena.active.as_ref().map(|active| active.key)
    }

    pub(crate) fn mark(&self) -> Result<RuntimeValueRegionMark, RegionArenaError> {
        self.arena.mark()
    }

    pub(crate) fn validate_mark(
        &self,
        mark: RuntimeValueRegionMark,
    ) -> Result<(), RegionArenaError> {
        self.arena.validate_mark(mark)
    }

    pub(crate) fn truncate(
        &mut self,
        mark: RuntimeValueRegionMark,
    ) -> Result<(), RegionArenaError> {
        self.arena.truncate(mark)
    }

    pub(crate) fn validate_truncate(
        &self,
        mark: RuntimeValueRegionMark,
    ) -> Result<(), RegionArenaError> {
        validate_private_truncate(&self.arena, mark)
    }

    pub(crate) fn admit_token_list(
        &self,
        coordinate: RuntimeTokenListCoordinate,
    ) -> Result<RuntimeTokenListView<'_>, RegionArenaError> {
        candidate_token_list_view(&self.arena, coordinate)
    }

    pub(crate) fn admit_macro(
        &self,
        coordinate: RuntimeMacroCoordinate,
    ) -> Result<RuntimeMacroView<'_>, RegionArenaError> {
        candidate_macro_view(&self.arena, coordinate)
    }

    pub(crate) fn admit_glue(
        &self,
        coordinate: RuntimeGlueCoordinate,
    ) -> Result<RuntimeGlueView<'_>, RegionArenaError> {
        candidate_glue_view(&self.arena, coordinate)
    }

    pub(crate) fn admit_origin_list(
        &self,
        coordinate: RuntimeOriginListCoordinate,
    ) -> Result<RuntimeOriginListView<'_>, RegionArenaError> {
        let columns = self.arena.resolve_columns(coordinate.owner())?;
        Ok(RuntimeOriginListView {
            coordinate,
            origins: resolve_local_span(&columns.provenance_roots, coordinate.origins)?,
        })
    }

    pub(crate) fn append_token_list(
        &mut self,
        input: RuntimeTokenListInput<'_>,
    ) -> Result<RuntimeTokenListCoordinate, RegionArenaError> {
        let additions = BundleAdditions {
            tokens: input.tokens.len(),
            token_lists: 1,
            provenance_roots: input.provenance.len(),
            ..BundleAdditions::default()
        };
        validate_sparse_origins(input.provenance, input.tokens.len())?;
        let active = prepare_bundle(&mut self.arena, additions)?;
        let word_start = checked_offset(active.columns.token_words.len())?;
        active.columns.token_words.extend_from_slice(input.tokens);
        let provenance_start = checked_offset(active.columns.provenance_roots.len())?;
        active
            .columns
            .provenance_roots
            .extend_from_slice(input.provenance);
        let row_offset = checked_offset(active.columns.token_lists.len())?;
        let semantic_id = input.semantic_id;
        active.columns.token_lists.push(RuntimeTokenListRow {
            id: input.id,
            semantic_id,
            tokens: LocalSpan::new(word_start, checked_len(input.tokens.len())?),
            provenance: LocalSpan::new(provenance_start, checked_len(input.provenance.len())?),
        });
        Ok(RuntimeTokenListCoordinate {
            row: active.key.coordinate(row_offset),
            id: input.id,
        })
    }

    pub(crate) fn append_traced_token_list(
        &mut self,
        input: RuntimeTracedTokenListInput<'_>,
    ) -> Result<RuntimeTokenListCoordinate, RegionArenaError> {
        let provenance_len = input
            .words
            .iter()
            .filter(|word| word.origin() != OriginId::UNKNOWN)
            .count();
        let additions = BundleAdditions {
            tokens: input.words.len(),
            token_lists: 1,
            provenance_roots: provenance_len,
            ..BundleAdditions::default()
        };
        let active = prepare_bundle(&mut self.arena, additions)?;
        let word_start = checked_offset(active.columns.token_words.len())?;
        let provenance_start = checked_offset(active.columns.provenance_roots.len())?;
        for (index, word) in input.words.iter().copied().enumerate() {
            active.columns.token_words.push(word.semantic_token());
            if word.origin() != OriginId::UNKNOWN {
                active
                    .columns
                    .provenance_roots
                    .push(RuntimeOriginEntry::new(checked_len(index)?, word.origin()));
            }
        }
        let row_offset = checked_offset(active.columns.token_lists.len())?;
        active.columns.token_lists.push(RuntimeTokenListRow {
            id: input.id,
            semantic_id: input.semantic_id,
            tokens: LocalSpan::new(word_start, checked_len(input.words.len())?),
            provenance: LocalSpan::new(provenance_start, checked_len(provenance_len)?),
        });
        Ok(RuntimeTokenListCoordinate {
            row: active.key.coordinate(row_offset),
            id: input.id,
        })
    }

    pub(crate) fn append_macro(
        &mut self,
        input: RuntimeMacroInput<'_>,
    ) -> Result<RuntimeMacroCoordinate, RegionArenaError> {
        let parameter = self.resolve_token_list(input.parameter_text)?;
        let replacement = self.resolve_token_list(input.replacement_text)?;
        let parameter_len = parameter.tokens.len;
        let replacement_len = replacement.tokens.len;
        validate_sparse_origins(input.parameter_origins, parameter_len as usize)?;
        validate_sparse_origins(input.replacement_origins, replacement_len as usize)?;
        let additions = BundleAdditions {
            macro_records: 1,
            macro_roots: 1,
            provenance_roots: input
                .parameter_origins
                .len()
                .checked_add(input.replacement_origins.len())
                .ok_or(RegionArenaError::OffsetCapacityExhausted)?,
            ..BundleAdditions::default()
        };
        let active = prepare_bundle(&mut self.arena, additions)?;
        let record_offset = checked_offset(active.columns.macro_records.len())?;
        let root_offset = checked_offset(active.columns.macro_roots.len())?;
        if record_offset != root_offset {
            return Err(RegionArenaError::InvalidMark);
        }
        let parameter_start = checked_offset(active.columns.provenance_roots.len())?;
        active
            .columns
            .provenance_roots
            .extend_from_slice(input.parameter_origins);
        let replacement_start = checked_offset(active.columns.provenance_roots.len())?;
        active
            .columns
            .provenance_roots
            .extend_from_slice(input.replacement_origins);
        active.columns.macro_records.push(RuntimeMacroRecord {
            definition: input.definition,
            flags: input.flags,
            parameter_pattern: input.parameter_pattern,
            parameter_len,
            replacement_len,
            observation_operand: input.observation_operand,
            allocation_serial: input.allocation_serial,
        });
        active.columns.macro_roots.push(RuntimeMacroRootRow {
            definition: input.definition,
            parameter_text: input.parameter_text,
            replacement_text: input.replacement_text,
            definition_origin: input.definition_origin,
            parameter_origins: LocalSpan::new(
                parameter_start,
                checked_len(input.parameter_origins.len())?,
            ),
            replacement_origins: LocalSpan::new(
                replacement_start,
                checked_len(input.replacement_origins.len())?,
            ),
        });
        Ok(RuntimeMacroCoordinate {
            row: active.key.coordinate(record_offset),
            id: input.definition,
        })
    }

    pub(crate) fn append_glue(
        &mut self,
        id: GlueId,
        spec: GlueSpec,
    ) -> Result<RuntimeGlueCoordinate, RegionArenaError> {
        Ok(RuntimeGlueCoordinate {
            row: self.arena.append_glue(spec)?,
            id,
        })
    }

    pub(crate) fn append_origin_list(
        &mut self,
        id: OriginListId,
        origins: &[OriginId],
    ) -> Result<RuntimeOriginListCoordinate, RegionArenaError> {
        let additions = BundleAdditions {
            provenance_roots: origins.len(),
            ..BundleAdditions::default()
        };
        let active = prepare_bundle(&mut self.arena, additions)?;
        let start = checked_offset(active.columns.provenance_roots.len())?;
        for (index, origin) in origins.iter().copied().enumerate() {
            active
                .columns
                .provenance_roots
                .push(RuntimeOriginEntry::new(checked_len(index)?, origin));
        }
        Ok(RuntimeOriginListCoordinate {
            owner: active.key,
            origins: LocalSpan::new(start, checked_len(origins.len())?),
            id,
        })
    }

    pub(crate) fn validate_accept(&self) -> Result<(), RegionArenaError> {
        self.arena.validate_accept()
    }

    /// Publishes only region owners absent from `destination` and stays live.
    pub(crate) fn publish_into(
        &mut self,
        destination: &mut RuntimeValueStore,
    ) -> Result<(), RegionArenaError> {
        let mark = destination.publication_mark()?;
        if let Err(error) = publish_candidate_regions(&mut self.arena, &mut destination.regions) {
            destination.restore_publication(mark)?;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn accept(self) -> Result<RuntimeValueStore, RegionArenaError> {
        Ok(RuntimeValueStore {
            regions: self.arena.accept()?,
        })
    }

    pub(crate) fn accounting(&self) -> RuntimeValueRegionAccounting {
        self.arena.accounting()
    }

    fn resolve_token_list(
        &self,
        coordinate: RuntimeTokenListCoordinate,
    ) -> Result<&RuntimeTokenListRow, RegionArenaError> {
        let row = self.arena.resolve_token_list(coordinate.row)?;
        validate_token_identity(coordinate, row)?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests;
