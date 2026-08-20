//! Heterogeneous rollback-owned runtime value regions.
//!
//! The six columns share one region key and one rollback lifecycle. Mutable
//! candidate storage is bump-appended, immutable suffix regions are sealed by
//! moving their vectors behind one region owner, and rollback recovers or drops
//! whole suffix regions. Coordinates never own their payload.

use core::cmp::Ordering;
use core::mem::size_of;
use core::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Arc;

use super::{ChunkOwner, FIRST_GENERATION, RegionArenaError, RegionCoordinate, fresh_namespace};

pub(crate) mod store;

/// The six physical columns that make up one logical runtime value region.
struct RuntimeValueColumns<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance> {
    token_words: Vec<TokenWord>,
    token_lists: Vec<TokenList>,
    macro_records: Vec<MacroRecord>,
    macro_roots: Vec<MacroRoot>,
    glue_specs: Vec<Glue>,
    provenance_roots: Vec<Provenance>,
}

impl<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance> Default
    for RuntimeValueColumns<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>
{
    fn default() -> Self {
        Self {
            token_words: Vec::new(),
            token_lists: Vec::new(),
            macro_records: Vec::new(),
            macro_roots: Vec::new(),
            glue_specs: Vec::new(),
            provenance_roots: Vec::new(),
        }
    }
}

impl<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>
    RuntimeValueColumns<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>
{
    fn logical_values(&self) -> usize {
        self.token_words
            .len()
            .saturating_add(self.token_lists.len())
            .saturating_add(self.macro_records.len())
            .saturating_add(self.macro_roots.len())
            .saturating_add(self.glue_specs.len())
            .saturating_add(self.provenance_roots.len())
    }

    fn logical_bytes(&self) -> usize {
        self.token_words
            .len()
            .saturating_mul(size_of::<TokenWord>())
            .saturating_add(
                self.token_lists
                    .len()
                    .saturating_mul(size_of::<TokenList>()),
            )
            .saturating_add(
                self.macro_records
                    .len()
                    .saturating_mul(size_of::<MacroRecord>()),
            )
            .saturating_add(
                self.macro_roots
                    .len()
                    .saturating_mul(size_of::<MacroRoot>()),
            )
            .saturating_add(self.glue_specs.len().saturating_mul(size_of::<Glue>()))
            .saturating_add(
                self.provenance_roots
                    .len()
                    .saturating_mul(size_of::<Provenance>()),
            )
    }

    fn retained_values(&self) -> usize {
        self.token_words
            .capacity()
            .saturating_add(self.token_lists.capacity())
            .saturating_add(self.macro_records.capacity())
            .saturating_add(self.macro_roots.capacity())
            .saturating_add(self.glue_specs.capacity())
            .saturating_add(self.provenance_roots.capacity())
    }

    fn retained_bytes(&self) -> usize {
        self.token_words
            .capacity()
            .saturating_mul(size_of::<TokenWord>())
            .saturating_add(
                self.token_lists
                    .capacity()
                    .saturating_mul(size_of::<TokenList>()),
            )
            .saturating_add(
                self.macro_records
                    .capacity()
                    .saturating_mul(size_of::<MacroRecord>()),
            )
            .saturating_add(
                self.macro_roots
                    .capacity()
                    .saturating_mul(size_of::<MacroRoot>()),
            )
            .saturating_add(self.glue_specs.capacity().saturating_mul(size_of::<Glue>()))
            .saturating_add(
                self.provenance_roots
                    .capacity()
                    .saturating_mul(size_of::<Provenance>()),
            )
    }

    fn provenance_values(&self) -> usize {
        self.provenance_roots.len()
    }

    fn retained_provenance_values(&self) -> usize {
        self.provenance_roots.capacity()
    }

    fn retained_provenance_bytes(&self) -> usize {
        self.provenance_roots
            .capacity()
            .saturating_mul(size_of::<Provenance>())
    }

    fn lengths(&self) -> RuntimeValueColumnLengths {
        RuntimeValueColumnLengths {
            token_words: self.token_words.len() as u32,
            token_lists: self.token_lists.len() as u32,
            macro_records: self.macro_records.len() as u32,
            macro_roots: self.macro_roots.len() as u32,
            glue_specs: self.glue_specs.len() as u32,
            provenance_roots: self.provenance_roots.len() as u32,
        }
    }

    fn validate_lengths(&self, lengths: RuntimeValueColumnLengths) -> bool {
        self.token_words.len() >= lengths.token_words as usize
            && self.token_lists.len() >= lengths.token_lists as usize
            && self.macro_records.len() >= lengths.macro_records as usize
            && self.macro_roots.len() >= lengths.macro_roots as usize
            && self.glue_specs.len() >= lengths.glue_specs as usize
            && self.provenance_roots.len() >= lengths.provenance_roots as usize
    }

    fn truncate(&mut self, lengths: RuntimeValueColumnLengths) {
        self.token_words.truncate(lengths.token_words as usize);
        self.token_lists.truncate(lengths.token_lists as usize);
        self.macro_records.truncate(lengths.macro_records as usize);
        self.macro_roots.truncate(lengths.macro_roots as usize);
        self.glue_specs.truncate(lengths.glue_specs as usize);
        self.provenance_roots
            .truncate(lengths.provenance_roots as usize);
    }

    fn clear(&mut self) {
        self.truncate(RuntimeValueColumnLengths::default());
    }
}

struct MutableRuntimeValueRegion<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance> {
    key: ChunkOwner,
    columns: RuntimeValueColumns<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
    appendable: bool,
}

struct SealedRuntimeValueRegion<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance> {
    key: ChunkOwner,
    columns: RuntimeValueColumns<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
}

type SealedOwner<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance> =
    Arc<SealedRuntimeValueRegion<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>>;

struct RuntimeValueRegionRoot<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance> {
    owner: SealedOwner<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
    uses: NonZeroUsize,
}

/// One region admitted once through namespace, slot, and generation.
///
/// Reads through this borrow need only validate the typed offset. The borrow
/// cannot outlive the canonical root set that owns the sealed region.
pub(crate) struct AdmittedRuntimeValueRegion<
    'a,
    TokenWord,
    TokenList,
    MacroRecord,
    MacroRoot,
    Glue,
    Provenance,
> {
    region: &'a SealedRuntimeValueRegion<
        TokenWord,
        TokenList,
        MacroRecord,
        MacroRoot,
        Glue,
        Provenance,
    >,
}

/// Explicit canonical region-root set for immutable runtime values.
///
/// Cloning this set is reserved for a generation fork or named checkpoint. It
/// clones one owner per region, never one owner per value.
pub(crate) struct AcceptedRuntimeValueRegions<
    TokenWord,
    TokenList,
    MacroRecord,
    MacroRoot,
    Glue,
    Provenance,
> {
    regions:
        Vec<RuntimeValueRegionRoot<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>>,
    initial_region_capacity: NonZeroU32,
}

impl<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance> Clone
    for AcceptedRuntimeValueRegions<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>
{
    fn clone(&self) -> Self {
        Self {
            regions: self
                .regions
                .iter()
                .map(|root| RuntimeValueRegionRoot {
                    owner: Arc::clone(&root.owner),
                    uses: root.uses,
                })
                .collect(),
            initial_region_capacity: self.initial_region_capacity,
        }
    }
}

impl<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>
    AcceptedRuntimeValueRegions<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>
{
    pub(crate) const fn new(initial_region_capacity: NonZeroU32) -> Self {
        Self {
            regions: Vec::new(),
            initial_region_capacity,
        }
    }

    pub(crate) fn candidate(
        &self,
    ) -> Result<
        RuntimeValueRegionArena<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
        RegionArenaError,
    > {
        RuntimeValueRegionArena::new(self.clone())
    }

    pub(crate) fn resolve_token_word(
        &self,
        coordinate: RegionCoordinate<TokenWord>,
    ) -> Result<&TokenWord, RegionArenaError> {
        self.resolve_region(coordinate.key)?
            .columns
            .token_words
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn admit(
        &self,
        key: ChunkOwner,
    ) -> Result<
        AdmittedRuntimeValueRegion<
            '_,
            TokenWord,
            TokenList,
            MacroRecord,
            MacroRoot,
            Glue,
            Provenance,
        >,
        RegionArenaError,
    > {
        Ok(AdmittedRuntimeValueRegion {
            region: self.resolve_region(key)?,
        })
    }

    pub(crate) fn resolve_token_list(
        &self,
        coordinate: RegionCoordinate<TokenList>,
    ) -> Result<&TokenList, RegionArenaError> {
        self.resolve_region(coordinate.key)?
            .columns
            .token_lists
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_macro_record(
        &self,
        coordinate: RegionCoordinate<MacroRecord>,
    ) -> Result<&MacroRecord, RegionArenaError> {
        self.resolve_region(coordinate.key)?
            .columns
            .macro_records
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_macro_root(
        &self,
        coordinate: RegionCoordinate<MacroRoot>,
    ) -> Result<&MacroRoot, RegionArenaError> {
        self.resolve_region(coordinate.key)?
            .columns
            .macro_roots
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_glue(
        &self,
        coordinate: RegionCoordinate<Glue>,
    ) -> Result<&Glue, RegionArenaError> {
        self.resolve_region(coordinate.key)?
            .columns
            .glue_specs
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_provenance(
        &self,
        coordinate: RegionCoordinate<Provenance>,
    ) -> Result<&Provenance, RegionArenaError> {
        self.resolve_region(coordinate.key)?
            .columns
            .provenance_roots
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn accounting(&self) -> RuntimeValueRegionAccounting {
        accounting_for_roots(&self.regions, 0, 0, self.regions.capacity())
    }

    /// Builds an explicit narrower canonical root set at a cold owner barrier.
    pub(crate) fn retain_regions(&self, keys: &[ChunkOwner]) -> Self {
        let regions = self
            .regions
            .iter()
            .filter_map(|root| {
                let uses = keys.iter().filter(|key| **key == root.owner.key).count();
                NonZeroUsize::new(uses).map(|uses| RuntimeValueRegionRoot {
                    owner: Arc::clone(&root.owner),
                    uses,
                })
            })
            .collect();
        Self {
            regions,
            initial_region_capacity: self.initial_region_capacity,
        }
    }

    /// Retains `uses` coordinates from one admitted source region.
    ///
    /// This operation clones the owner only when the destination did not
    /// already name the region. Multiplicity remains local bookkeeping.
    pub(crate) fn retain_from(
        &mut self,
        source: &Self,
        key: ChunkOwner,
        uses: NonZeroUsize,
    ) -> Result<(), RegionArenaError> {
        let source_root = source.root(key)?;
        match self.root_position(key) {
            Ok(index) => {
                self.regions[index].uses = self.regions[index]
                    .uses
                    .checked_add(uses.get())
                    .ok_or(RegionArenaError::SlotCapacityExhausted)?;
            }
            Err(index) => self.regions.insert(
                index,
                RuntimeValueRegionRoot {
                    owner: Arc::clone(&source_root.owner),
                    uses,
                },
            ),
        }
        Ok(())
    }

    /// Releases local coordinate uses and drops the sole region owner when
    /// the last use leaves this canonical set.
    pub(crate) fn release(
        &mut self,
        key: ChunkOwner,
        uses: NonZeroUsize,
    ) -> Result<(), RegionArenaError> {
        let index = self
            .root_position(key)
            .map_err(|_| RegionArenaError::UnknownChunk)?;
        let retained = self.regions[index]
            .uses
            .get()
            .checked_sub(uses.get())
            .ok_or(RegionArenaError::UnknownChunk)?;
        if let Some(retained) = NonZeroUsize::new(retained) {
            self.regions[index].uses = retained;
        } else {
            self.regions.remove(index);
        }
        Ok(())
    }

    /// Transfers uses without exposing an ownerless interval.
    pub(crate) fn transfer(
        source: &mut Self,
        destination: &mut Self,
        key: ChunkOwner,
        uses: NonZeroUsize,
    ) -> Result<(), RegionArenaError> {
        destination.retain_from(source, key, uses)?;
        source.release(key, uses)
    }

    fn root(
        &self,
        key: ChunkOwner,
    ) -> Result<
        &RuntimeValueRegionRoot<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
        RegionArenaError,
    > {
        self.root_position(key)
            .map(|index| &self.regions[index])
            .map_err(|_| RegionArenaError::UnknownChunk)
    }

    fn root_position(&self, key: ChunkOwner) -> Result<usize, usize> {
        self.regions
            .binary_search_by(|root| compare_region_keys(root.owner.key, key))
    }

    #[cfg(test)]
    fn testing_uses(&self, key: ChunkOwner) -> usize {
        self.root(key).map_or(0, |root| root.uses.get())
    }

    fn resolve_region(
        &self,
        key: ChunkOwner,
    ) -> Result<
        &SealedRuntimeValueRegion<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
        RegionArenaError,
    > {
        resolve_rooted_region(&self.regions, key)
    }
}

impl<'a, TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>
    AdmittedRuntimeValueRegion<'a, TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>
{
    fn validate<T>(&self, coordinate: RegionCoordinate<T>) -> Result<usize, RegionArenaError> {
        if coordinate.key != self.region.key {
            return Err(if coordinate.key.namespace != self.region.key.namespace {
                RegionArenaError::ForeignNamespace
            } else if coordinate.key.slot != self.region.key.slot {
                RegionArenaError::UnknownChunk
            } else {
                RegionArenaError::StaleGeneration
            });
        }
        Ok(coordinate.offset as usize)
    }

    pub(crate) fn token_word(
        &self,
        coordinate: RegionCoordinate<TokenWord>,
    ) -> Result<&'a TokenWord, RegionArenaError> {
        self.region
            .columns
            .token_words
            .get(self.validate(coordinate)?)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn token_list(
        &self,
        coordinate: RegionCoordinate<TokenList>,
    ) -> Result<&'a TokenList, RegionArenaError> {
        self.region
            .columns
            .token_lists
            .get(self.validate(coordinate)?)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn macro_record(
        &self,
        coordinate: RegionCoordinate<MacroRecord>,
    ) -> Result<&'a MacroRecord, RegionArenaError> {
        self.region
            .columns
            .macro_records
            .get(self.validate(coordinate)?)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn macro_root(
        &self,
        coordinate: RegionCoordinate<MacroRoot>,
    ) -> Result<&'a MacroRoot, RegionArenaError> {
        self.region
            .columns
            .macro_roots
            .get(self.validate(coordinate)?)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn glue(
        &self,
        coordinate: RegionCoordinate<Glue>,
    ) -> Result<&'a Glue, RegionArenaError> {
        self.region
            .columns
            .glue_specs
            .get(self.validate(coordinate)?)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn provenance(
        &self,
        coordinate: RegionCoordinate<Provenance>,
    ) -> Result<&'a Provenance, RegionArenaError> {
        self.region
            .columns
            .provenance_roots
            .get(self.validate(coordinate)?)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RuntimeValueColumnLengths {
    token_words: u32,
    token_lists: u32,
    macro_records: u32,
    macro_roots: u32,
    glue_specs: u32,
    provenance_roots: u32,
}

/// Fixed-size rollback watermark for one heterogeneous value-region candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeValueRegionMark {
    namespace: NonZeroU64,
    allocation_sequence: u64,
    sealed_regions: u32,
    active_slot: u32,
    active_generation: u32,
    lengths: RuntimeValueColumnLengths,
}

/// Mutable bump suffix over an explicit accepted region-root set.
pub(crate) struct RuntimeValueRegionArena<
    TokenWord,
    TokenList,
    MacroRecord,
    MacroRoot,
    Glue,
    Provenance,
> {
    base:
        AcceptedRuntimeValueRegions<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
    namespace: NonZeroU64,
    sealed_suffix: Vec<SealedOwner<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>>,
    active: Option<
        MutableRuntimeValueRegion<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
    >,
    reusable: Vec<
        MutableRuntimeValueRegion<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
    >,
    next_slot: u32,
    allocation_sequence: u64,
    region_capacity: u32,
    storage_growth_events: usize,
}

impl<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>
    RuntimeValueRegionArena<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>
{
    fn new(
        base: AcceptedRuntimeValueRegions<
            TokenWord,
            TokenList,
            MacroRecord,
            MacroRoot,
            Glue,
            Provenance,
        >,
    ) -> Result<Self, RegionArenaError> {
        let namespace = fresh_namespace()?;
        Ok(Self {
            region_capacity: base.initial_region_capacity.get(),
            base,
            namespace,
            sealed_suffix: Vec::new(),
            active: None,
            reusable: Vec::new(),
            next_slot: 0,
            allocation_sequence: 0,
            storage_growth_events: 0,
        })
    }

    pub(crate) fn mark(&self) -> Result<RuntimeValueRegionMark, RegionArenaError> {
        let sealed_regions = u32::try_from(self.sealed_suffix.len())
            .map_err(|_| RegionArenaError::SlotCapacityExhausted)?;
        let (active_slot, active_generation, lengths) =
            self.active
                .as_ref()
                .map_or((0, 0, RuntimeValueColumnLengths::default()), |active| {
                    (
                        active.key.slot,
                        active.key.generation.get(),
                        active.columns.lengths(),
                    )
                });
        Ok(RuntimeValueRegionMark {
            namespace: self.namespace,
            allocation_sequence: self.allocation_sequence,
            sealed_regions,
            active_slot,
            active_generation,
            lengths,
        })
    }

    pub(crate) fn validate_mark(
        &self,
        mark: RuntimeValueRegionMark,
    ) -> Result<(), RegionArenaError> {
        if mark.namespace != self.namespace
            || mark.allocation_sequence > self.allocation_sequence
            || mark.sealed_regions as usize > self.sealed_suffix.len()
        {
            return Err(RegionArenaError::InvalidMark);
        }
        if mark.active_generation == 0 {
            return (mark.lengths == RuntimeValueColumnLengths::default())
                .then_some(())
                .ok_or(RegionArenaError::InvalidMark);
        }
        let key = ChunkOwner {
            namespace: mark.namespace,
            slot: mark.active_slot,
            generation: NonZeroU32::new(mark.active_generation)
                .ok_or(RegionArenaError::InvalidMark)?,
        };
        let columns = if self.active.as_ref().is_some_and(|active| active.key == key) {
            &self
                .active
                .as_ref()
                .expect("active key was checked")
                .columns
        } else {
            &self
                .sealed_suffix
                .iter()
                .skip(mark.sealed_regions as usize)
                .find(|region| region.key == key)
                .ok_or(RegionArenaError::InvalidMark)?
                .columns
        };
        columns
            .validate_lengths(mark.lengths)
            .then_some(())
            .ok_or(RegionArenaError::InvalidMark)
    }

    pub(crate) fn truncate(
        &mut self,
        mark: RuntimeValueRegionMark,
    ) -> Result<(), RegionArenaError> {
        self.validate_mark(mark)?;
        let target_key = NonZeroU32::new(mark.active_generation).map(|generation| ChunkOwner {
            namespace: mark.namespace,
            slot: mark.active_slot,
            generation,
        });
        let mut target = None;
        if let Some(active) = self.active.take() {
            if Some(active.key) == target_key {
                target = Some(active);
            } else {
                self.recycle_mutable(active);
            }
        }
        while self.sealed_suffix.len() > mark.sealed_regions as usize {
            let sealed = self.sealed_suffix.pop().expect("suffix length was checked");
            let key = sealed.key;
            match unseal_private(sealed) {
                Some(mutable) if Some(mutable.key) == target_key => target = Some(mutable),
                Some(mutable) => self.recycle_mutable(mutable),
                None if Some(key) == target_key => return Err(RegionArenaError::InvalidMark),
                // A newer checkpoint may still retain this discarded whole
                // region. Drop the candidate's owner; the checkpoint releases
                // the storage when it retires. Shared storage is never reused.
                None => {}
            }
        }
        if let Some(mut active) = target {
            active.columns.truncate(mark.lengths);
            if active.columns.logical_values() == 0 {
                self.recycle_mutable(active);
            } else {
                // A truncated offset must never become live again in the same
                // generation. The retained prefix seals before the next bump.
                active.appendable = false;
                self.active = Some(active);
            }
        }
        Ok(())
    }

    pub(crate) fn validate_accept(&self) -> Result<(), RegionArenaError> {
        Ok(())
    }

    pub(crate) fn accept(
        mut self,
    ) -> Result<
        AcceptedRuntimeValueRegions<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
        RegionArenaError,
    > {
        self.seal_active()?;
        self.base
            .regions
            .extend(
                self.sealed_suffix
                    .drain(..)
                    .map(|owner| RuntimeValueRegionRoot {
                        owner,
                        uses: NonZeroUsize::MIN,
                    }),
            );
        Ok(self.base)
    }

    pub(crate) fn append_token_word(
        &mut self,
        value: TokenWord,
    ) -> Result<RegionCoordinate<TokenWord>, RegionArenaError> {
        self.ensure_active()?;
        let active = self.active.as_mut().expect("active region was ensured");
        reserve_column(
            &mut active.columns.token_words,
            self.region_capacity,
            &mut self.storage_growth_events,
        )?;
        let offset = checked_offset(active.columns.token_words.len())?;
        active.columns.token_words.push(value);
        Ok(active.key.coordinate(offset))
    }

    pub(crate) fn append_token_list(
        &mut self,
        value: TokenList,
    ) -> Result<RegionCoordinate<TokenList>, RegionArenaError> {
        self.ensure_active()?;
        let active = self.active.as_mut().expect("active region was ensured");
        reserve_column(
            &mut active.columns.token_lists,
            self.region_capacity,
            &mut self.storage_growth_events,
        )?;
        let offset = checked_offset(active.columns.token_lists.len())?;
        active.columns.token_lists.push(value);
        Ok(active.key.coordinate(offset))
    }

    pub(crate) fn append_macro_record(
        &mut self,
        value: MacroRecord,
    ) -> Result<RegionCoordinate<MacroRecord>, RegionArenaError> {
        self.ensure_active()?;
        let active = self.active.as_mut().expect("active region was ensured");
        reserve_column(
            &mut active.columns.macro_records,
            self.region_capacity,
            &mut self.storage_growth_events,
        )?;
        let offset = checked_offset(active.columns.macro_records.len())?;
        active.columns.macro_records.push(value);
        Ok(active.key.coordinate(offset))
    }

    pub(crate) fn append_macro_root(
        &mut self,
        value: MacroRoot,
    ) -> Result<RegionCoordinate<MacroRoot>, RegionArenaError> {
        self.ensure_active()?;
        let active = self.active.as_mut().expect("active region was ensured");
        reserve_column(
            &mut active.columns.macro_roots,
            self.region_capacity,
            &mut self.storage_growth_events,
        )?;
        let offset = checked_offset(active.columns.macro_roots.len())?;
        active.columns.macro_roots.push(value);
        Ok(active.key.coordinate(offset))
    }

    pub(crate) fn append_glue(
        &mut self,
        value: Glue,
    ) -> Result<RegionCoordinate<Glue>, RegionArenaError> {
        self.ensure_active()?;
        let active = self.active.as_mut().expect("active region was ensured");
        reserve_column(
            &mut active.columns.glue_specs,
            self.region_capacity,
            &mut self.storage_growth_events,
        )?;
        let offset = checked_offset(active.columns.glue_specs.len())?;
        active.columns.glue_specs.push(value);
        Ok(active.key.coordinate(offset))
    }

    pub(crate) fn append_provenance(
        &mut self,
        value: Provenance,
    ) -> Result<RegionCoordinate<Provenance>, RegionArenaError> {
        self.ensure_active()?;
        let active = self.active.as_mut().expect("active region was ensured");
        reserve_column(
            &mut active.columns.provenance_roots,
            self.region_capacity,
            &mut self.storage_growth_events,
        )?;
        let offset = checked_offset(active.columns.provenance_roots.len())?;
        active.columns.provenance_roots.push(value);
        Ok(active.key.coordinate(offset))
    }

    pub(crate) fn resolve_token_word(
        &self,
        coordinate: RegionCoordinate<TokenWord>,
    ) -> Result<&TokenWord, RegionArenaError> {
        self.resolve_columns(coordinate.key)?
            .token_words
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_token_list(
        &self,
        coordinate: RegionCoordinate<TokenList>,
    ) -> Result<&TokenList, RegionArenaError> {
        self.resolve_columns(coordinate.key)?
            .token_lists
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_macro_record(
        &self,
        coordinate: RegionCoordinate<MacroRecord>,
    ) -> Result<&MacroRecord, RegionArenaError> {
        self.resolve_columns(coordinate.key)?
            .macro_records
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_macro_root(
        &self,
        coordinate: RegionCoordinate<MacroRoot>,
    ) -> Result<&MacroRoot, RegionArenaError> {
        self.resolve_columns(coordinate.key)?
            .macro_roots
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_glue(
        &self,
        coordinate: RegionCoordinate<Glue>,
    ) -> Result<&Glue, RegionArenaError> {
        self.resolve_columns(coordinate.key)?
            .glue_specs
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_provenance(
        &self,
        coordinate: RegionCoordinate<Provenance>,
    ) -> Result<&Provenance, RegionArenaError> {
        self.resolve_columns(coordinate.key)?
            .provenance_roots
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn accounting(&self) -> RuntimeValueRegionAccounting {
        let mut accounting = self.base.accounting().plus(accounting_for_owners(
            &self.sealed_suffix,
            0,
            0,
            self.sealed_suffix.capacity(),
        ));
        if let Some(active) = &self.active {
            accounting = accounting.plus(accounting_for_columns(&active.columns, 1, 0));
        }
        for reusable in &self.reusable {
            accounting = accounting.plus(accounting_for_columns(&reusable.columns, 0, 1));
        }
        accounting.registry_slots = self.next_slot as usize;
        accounting.registry_capacity = self
            .sealed_suffix
            .capacity()
            .saturating_add(self.reusable.capacity())
            .saturating_add(usize::from(self.active.is_some()));
        accounting
    }

    #[cfg(test)]
    pub(crate) fn testing_storage_growth_events(&self) -> usize {
        self.storage_growth_events
    }

    fn ensure_active(&mut self) -> Result<(), RegionArenaError> {
        let needs_new = self.active.as_ref().is_none_or(|active| {
            !active.appendable || active.columns.logical_values() >= self.region_capacity as usize
        });
        if !needs_new {
            return Ok(());
        }
        self.seal_active()?;
        let mut active = if let Some(mut reusable) = self.reusable.pop() {
            let generation = reusable
                .key
                .generation
                .get()
                .checked_add(1)
                .and_then(NonZeroU32::new)
                .ok_or(RegionArenaError::GenerationExhausted)?;
            reusable.key.generation = generation;
            reusable.appendable = true;
            reusable
        } else {
            let slot = self.next_slot;
            self.next_slot = self
                .next_slot
                .checked_add(1)
                .ok_or(RegionArenaError::SlotCapacityExhausted)?;
            self.storage_growth_events = self.storage_growth_events.saturating_add(1);
            MutableRuntimeValueRegion {
                key: ChunkOwner {
                    namespace: self.namespace,
                    slot,
                    generation: FIRST_GENERATION,
                },
                columns: RuntimeValueColumns::default(),
                appendable: true,
            }
        };
        active.columns.clear();
        self.allocation_sequence = self
            .allocation_sequence
            .checked_add(1)
            .ok_or(RegionArenaError::SlotCapacityExhausted)?;
        self.active = Some(active);
        Ok(())
    }

    fn seal_active(&mut self) -> Result<(), RegionArenaError> {
        let Some(active) = self.active.take() else {
            return Ok(());
        };
        if active.columns.logical_values() == 0 {
            self.recycle_mutable(active);
            return Ok(());
        }
        let old_capacity = self.sealed_suffix.capacity();
        self.sealed_suffix
            .try_reserve(1)
            .map_err(|_| RegionArenaError::AllocationFailed)?;
        if self.sealed_suffix.capacity() != old_capacity {
            self.storage_growth_events = self.storage_growth_events.saturating_add(1);
        }
        self.sealed_suffix.push(Arc::new(SealedRuntimeValueRegion {
            key: active.key,
            columns: active.columns,
        }));
        Ok(())
    }

    fn recycle_mutable(
        &mut self,
        mut region: MutableRuntimeValueRegion<
            TokenWord,
            TokenList,
            MacroRecord,
            MacroRoot,
            Glue,
            Provenance,
        >,
    ) {
        region.columns.clear();
        region.appendable = false;
        self.reusable.push(region);
    }

    fn resolve_columns(
        &self,
        key: ChunkOwner,
    ) -> Result<
        &RuntimeValueColumns<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
        RegionArenaError,
    > {
        if let Some(active) = &self.active
            && active.key == key
        {
            return Ok(&active.columns);
        }
        match resolve_sealed_region(&self.sealed_suffix, key) {
            Ok(region) => return Ok(&region.columns),
            Err(RegionArenaError::ForeignNamespace) => {}
            Err(error) => return Err(error),
        }
        Ok(&self.base.resolve_region(key)?.columns)
    }
}

/// Logical and retained region-level accounting for exact controls.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RuntimeValueRegionAccounting {
    pub(crate) logical_values: usize,
    pub(crate) logical_bytes: usize,
    pub(crate) live_regions: usize,
    pub(crate) reusable_regions: usize,
    pub(crate) region_owners: usize,
    pub(crate) retained_payload_values: usize,
    pub(crate) retained_payload_bytes: usize,
    pub(crate) provenance_values: usize,
    pub(crate) retained_provenance_values: usize,
    pub(crate) retained_provenance_bytes: usize,
    pub(crate) registry_slots: usize,
    pub(crate) registry_capacity: usize,
}

impl RuntimeValueRegionAccounting {
    fn plus(self, other: Self) -> Self {
        Self {
            logical_values: self.logical_values.saturating_add(other.logical_values),
            logical_bytes: self.logical_bytes.saturating_add(other.logical_bytes),
            live_regions: self.live_regions.saturating_add(other.live_regions),
            reusable_regions: self.reusable_regions.saturating_add(other.reusable_regions),
            region_owners: self.region_owners.saturating_add(other.region_owners),
            retained_payload_values: self
                .retained_payload_values
                .saturating_add(other.retained_payload_values),
            retained_payload_bytes: self
                .retained_payload_bytes
                .saturating_add(other.retained_payload_bytes),
            provenance_values: self
                .provenance_values
                .saturating_add(other.provenance_values),
            retained_provenance_values: self
                .retained_provenance_values
                .saturating_add(other.retained_provenance_values),
            retained_provenance_bytes: self
                .retained_provenance_bytes
                .saturating_add(other.retained_provenance_bytes),
            registry_slots: self.registry_slots.saturating_add(other.registry_slots),
            registry_capacity: self
                .registry_capacity
                .saturating_add(other.registry_capacity),
        }
    }
}

fn reserve_column<T>(
    column: &mut Vec<T>,
    capacity: u32,
    storage_growth_events: &mut usize,
) -> Result<(), RegionArenaError> {
    if column.capacity() == 0 {
        column
            .try_reserve_exact(capacity as usize)
            .map_err(|_| RegionArenaError::AllocationFailed)?;
        *storage_growth_events = storage_growth_events.saturating_add(1);
    }
    Ok(())
}

fn checked_offset(len: usize) -> Result<u32, RegionArenaError> {
    u32::try_from(len).map_err(|_| RegionArenaError::OffsetCapacityExhausted)
}

fn unseal_private<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>(
    sealed: SealedOwner<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
) -> Option<
    MutableRuntimeValueRegion<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
> {
    let sealed = Arc::try_unwrap(sealed).ok()?;
    Some(MutableRuntimeValueRegion {
        key: sealed.key,
        columns: sealed.columns,
        appendable: false,
    })
}

fn resolve_rooted_region<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>(
    regions: &[RuntimeValueRegionRoot<
        TokenWord,
        TokenList,
        MacroRecord,
        MacroRoot,
        Glue,
        Provenance,
    >],
    key: ChunkOwner,
) -> Result<
    &SealedRuntimeValueRegion<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
    RegionArenaError,
> {
    match regions.binary_search_by(|root| compare_region_keys(root.owner.key, key)) {
        Ok(index) => Ok(&regions[index].owner),
        Err(_)
            if regions
                .iter()
                .any(|root| root.owner.key.namespace == key.namespace) =>
        {
            if regions.iter().any(|root| {
                root.owner.key.namespace == key.namespace && root.owner.key.slot == key.slot
            }) {
                Err(RegionArenaError::StaleGeneration)
            } else {
                Err(RegionArenaError::UnknownChunk)
            }
        }
        Err(_) => Err(RegionArenaError::ForeignNamespace),
    }
}

fn compare_region_keys(left: ChunkOwner, right: ChunkOwner) -> Ordering {
    (left.namespace, left.slot, left.generation).cmp(&(
        right.namespace,
        right.slot,
        right.generation,
    ))
}

fn resolve_sealed_region<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>(
    regions: &[SealedOwner<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>],
    key: ChunkOwner,
) -> Result<
    &SealedRuntimeValueRegion<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
    RegionArenaError,
> {
    let mut namespace_seen = false;
    let mut slot_seen = false;
    for region in regions.iter().rev() {
        if region.key.namespace != key.namespace {
            continue;
        }
        namespace_seen = true;
        if region.key.slot != key.slot {
            continue;
        }
        slot_seen = true;
        if region.key.generation != key.generation {
            continue;
        }
        return Ok(region);
    }
    if slot_seen {
        Err(RegionArenaError::StaleGeneration)
    } else if namespace_seen {
        Err(RegionArenaError::UnknownChunk)
    } else {
        Err(RegionArenaError::ForeignNamespace)
    }
}

fn accounting_for_roots<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>(
    regions: &[RuntimeValueRegionRoot<
        TokenWord,
        TokenList,
        MacroRecord,
        MacroRoot,
        Glue,
        Provenance,
    >],
    reusable_regions: usize,
    registry_slots: usize,
    registry_capacity: usize,
) -> RuntimeValueRegionAccounting {
    regions.iter().fold(
        RuntimeValueRegionAccounting {
            reusable_regions,
            registry_slots,
            registry_capacity,
            ..RuntimeValueRegionAccounting::default()
        },
        |accounting, root| accounting.plus(accounting_for_columns(&root.owner.columns, 1, 0)),
    )
}

fn accounting_for_owners<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>(
    regions: &[SealedOwner<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>],
    reusable_regions: usize,
    registry_slots: usize,
    registry_capacity: usize,
) -> RuntimeValueRegionAccounting {
    regions.iter().fold(
        RuntimeValueRegionAccounting {
            reusable_regions,
            registry_slots,
            registry_capacity,
            ..RuntimeValueRegionAccounting::default()
        },
        |accounting, region| accounting.plus(accounting_for_columns(&region.columns, 1, 0)),
    )
}

fn accounting_for_columns<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>(
    columns: &RuntimeValueColumns<TokenWord, TokenList, MacroRecord, MacroRoot, Glue, Provenance>,
    live_regions: usize,
    reusable_regions: usize,
) -> RuntimeValueRegionAccounting {
    RuntimeValueRegionAccounting {
        logical_values: columns.logical_values(),
        logical_bytes: columns.logical_bytes(),
        live_regions,
        reusable_regions,
        region_owners: live_regions,
        retained_payload_values: columns.retained_values(),
        retained_payload_bytes: columns.retained_bytes(),
        provenance_values: columns.provenance_values(),
        retained_provenance_values: columns.retained_provenance_values(),
        retained_provenance_bytes: columns.retained_provenance_bytes(),
        registry_slots: 0,
        registry_capacity: 0,
    }
}

#[cfg(test)]
mod tests;
